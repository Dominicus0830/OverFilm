"""GPT-SoVITS 클로닝 엔진 (M6, 기획서 5.3).

GPT-SoVITS(MIT)는 우리 프로그램에 넣지 않고, 사용자가 받은 GPT-SoVITS 폴더의 `api_v2.py` 를
로컬 API 서버로 띄워 HTTP 로 부른다 (`POST /tts`).

- 서버 주소(`voice.clone.server`)에 이미 떠 있으면 그대로 쓴다.
- 없고 `voice.clone.dir`(GPT-SoVITS 폴더)를 정했으면 우리가 띄우고, 음성 단계가 끝나면 내린다
  (GPU 모델은 한 번에 하나만: Ollama 는 필터 뒤에 내리고, TTS 서버는 음성 단계 뒤에 내린다).
- 라데온·인텔·GPU 없음: GPT-SoVITS 가 CUDA 를 못 찾으면 스스로 CPU 로 돈다 (사용자 결정
  2026-11-14: 느려도 허용). 화면에 경고를 띄운다.
- 단어 시각은 엔진이 안 주므로 align.py (stable-ts 가 있으면 강제 정렬, 없으면 음절 비율).
"""

from __future__ import annotations

import json
import os
import platform
import subprocess
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, ClassVar

from .engines import Synth, TtsError
from .voices import VoiceLibrary


@dataclass
class CloneSettings:
    server: str = "http://127.0.0.1:9880"
    dir: str | None = None  # GPT-SoVITS 폴더 (api_v2.py 가 있는 곳). 정하면 필요할 때 띄운다
    python: str | None = None  # 비우면 <dir>/runtime/python.exe (통합 패키지) → python
    config: str | None = None  # tts_infer.yaml (비우면 GPT-SoVITS 기본값)
    speed: float = 1.0
    top_k: int = 15
    temperature: float = 1.0
    start_timeout: float = 300.0  # CPU 에서 모델 올리는 데 1~3분 걸릴 수 있다
    sentence_timeout: float = 300.0
    unload_after: bool = True
    align: str = "auto"  # auto | stable_ts | syllable
    align_model: str = "base"

    @classmethod
    def from_cfg(cls, cfg: dict[str, Any]) -> CloneSettings:
        c = ((cfg.get("voice") or {}).get("clone") or {})
        if not isinstance(c, dict):
            c = {}

        def num(key: str, lo: float, hi: float, default: float) -> float:
            try:
                return min(hi, max(lo, float(c.get(key, default))))
            except (TypeError, ValueError):
                return default

        server = str(c.get("server") or cls.server).rstrip("/")
        if not server.startswith(("http://", "https://")):
            server = "http://" + server
        return cls(
            server=server,
            dir=str(c["dir"]) if c.get("dir") else None,
            python=str(c["python"]) if c.get("python") else None,
            config=str(c["config"]) if c.get("config") else None,
            speed=num("speed", 0.6, 1.6, 1.0),
            top_k=int(num("top_k", 1, 100, 15)),
            temperature=num("temperature", 0.1, 2.0, 1.0),
            start_timeout=num("start_timeout", 10, 1800, 300),
            sentence_timeout=num("sentence_timeout", 10, 1800, 300),
            unload_after=bool(c.get("unload_after", True)),
            align=str(c.get("align") or "auto"),
            align_model=str(c.get("align_model") or "base"),
        )


def _http(url: str, data: bytes | None = None, timeout: float = 10.0) -> tuple[int, bytes, str]:
    req = urllib.request.Request(url, data=data, method="POST" if data is not None else "GET")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    # 로컬 서버라 프록시를 타지 않는다
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(req, timeout=timeout) as res:
            return res.status, res.read(80 * 1024 * 1024), res.headers.get("Content-Type", "")
    except urllib.error.HTTPError as err:
        return err.code, err.read(64 * 1024), err.headers.get("Content-Type", "") if err.headers else ""


class GptSovitsServer:
    """서버 하나를 프로세스 안에서 같이 쓴다 (패널 미리듣기 + 작업 스레드)."""

    _shared: ClassVar[dict[tuple[str, str | None], GptSovitsServer]] = {}
    _shared_lock = threading.Lock()

    def __init__(self, settings: CloneSettings, log_path: Path | None = None):
        self.s = settings
        self.log_path = log_path
        self.proc: subprocess.Popen | None = None
        self.lock = threading.RLock()
        self.weights: tuple[str, str] = ("", "")  # 지금 올린 (gpt, sovits) 학습 모델

    @classmethod
    def shared(cls, settings: CloneSettings, log_path: Path | None = None) -> GptSovitsServer:
        key = (settings.server, settings.dir)
        with cls._shared_lock:
            got = cls._shared.get(key)
            if got is None:
                got = cls._shared[key] = cls(settings, log_path)
            else:
                got.s = settings
                got.log_path = log_path or got.log_path
            return got

    # --- 상태 ---
    def alive(self, timeout: float = 2.0) -> bool:
        """어떤 HTTP 응답이든 오면 떠 있는 것 (api_v2 는 / 에 404 를 준다)."""
        try:
            _http(self.s.server + "/", timeout=timeout)
        except (OSError, ValueError):
            return False
        return True

    @property
    def managed(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def launch_command(self) -> list[str]:
        if not self.s.dir:
            raise TtsError("GPT-SoVITS 서버가 꺼져 있습니다. GPT-SoVITS 폴더(voice.clone.dir)를 정하거나 "
                           f"직접 띄워 주세요: python api_v2.py -a 127.0.0.1 -p {self._port()}")
        folder = Path(self.s.dir)
        if not (folder / "api_v2.py").is_file():
            raise TtsError(f"GPT-SoVITS 폴더에 api_v2.py 가 없습니다: {folder}")
        python = self.s.python
        if not python:
            for cand in (folder / "runtime" / "python.exe", folder / "runtime" / "bin" / "python"):
                if cand.is_file():
                    python = str(cand)
                    break
        args = [python or "python", "api_v2.py", "-a", self._host(), "-p", str(self._port())]
        if self.s.config:
            args += ["-c", self.s.config]
        return args

    def _host(self) -> str:
        return urllib.parse.urlparse(self.s.server).hostname or "127.0.0.1"

    def _port(self) -> int:
        return urllib.parse.urlparse(self.s.server).port or 9880

    def ensure_running(self, wait: bool = True) -> None:
        with self.lock:
            if self.alive():
                return
            if self.proc is not None and self.proc.poll() is None:
                pass  # 띄우는 중
            else:
                args = self.launch_command()
                log = None
                try:
                    if self.log_path:
                        self.log_path.parent.mkdir(parents=True, exist_ok=True)
                        log = open(self.log_path, "ab")  # noqa: SIM115 - 프로세스가 끝날 때까지 쓴다
                    flags = 0x0800_0000 if platform.system() == "Windows" else 0  # 창 안 띄움
                    env = {**os.environ, "PYTHONIOENCODING": "utf-8", "PYTHONUTF8": "1"}
                    self.proc = subprocess.Popen(args, cwd=self.s.dir, stdout=log or subprocess.DEVNULL,
                                                 stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                                                 env=env, creationflags=flags)
                except OSError as err:
                    raise TtsError(f"GPT-SoVITS 서버를 띄우지 못했습니다 ({args[0]}): {err}") from err
                finally:
                    if log is not None:
                        log.close()
                self.weights = ("", "")
            if not wait:
                return
            deadline = time.monotonic() + self.s.start_timeout
            while time.monotonic() < deadline:
                if self.proc is not None and self.proc.poll() is not None:
                    code = self.proc.returncode
                    self.proc = None
                    raise TtsError(f"GPT-SoVITS 서버가 바로 꺼졌습니다 (코드 {code}). "
                                   f"로그: {self.log_path or '없음'}")
                if self.alive():
                    return
                time.sleep(1.0)
            raise TtsError(f"GPT-SoVITS 서버가 {int(self.s.start_timeout)}초 안에 뜨지 않았습니다 "
                           f"(CPU 면 더 걸릴 수 있음, 로그: {self.log_path or '없음'})")

    def stop(self) -> bool:
        """우리가 띄운 서버만 내린다. 사용자가 직접 띄운 서버는 그대로 둔다."""
        with self.lock:
            proc, self.proc = self.proc, None
            self.weights = ("", "")
            if proc is None or proc.poll() is not None:
                return False
            try:
                _http(self.s.server + "/control?command=exit", timeout=5)
            except OSError:
                pass
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.terminate()
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proc.kill()
            return True

    # --- 합성 ---
    def set_weights(self, gpt: str, sovits: str) -> None:
        """목소리마다 학습한 모델(.ckpt/.pth)이 있으면 바꿔 올린다. 비면 서버 기본 모델."""
        if not gpt and not sovits and self.weights == ("", ""):
            return  # 기본 모델 그대로
        default = self.default_weights()
        want = (gpt or default[0], sovits or default[1])
        if want == self.weights:
            return
        if not all(want):
            # 기본 모델로 돌아가야 하는데 경로를 모른다 → 우리가 띄운 서버면 다시 띄운다
            if not self.managed:
                raise TtsError("학습 모델을 쓴 목소리 다음에 기본 모델로 돌아갈 수 없습니다. "
                               "GPT-SoVITS 서버를 다시 켜 주세요")
            self.stop()
            self.ensure_running()
            if not gpt and not sovits:
                return
        for key, path in (("set_gpt_weights", want[0]), ("set_sovits_weights", want[1])):
            if not path:
                continue
            code, body, _ = _http(f"{self.s.server}/{key}?"
                                  + urllib.parse.urlencode({"weights_path": path}), timeout=600)
            if code != 200:
                raise TtsError(f"GPT-SoVITS 모델을 바꾸지 못했습니다 ({path}): {_message(body)}")
        self.weights = want

    def default_weights(self) -> tuple[str, str]:
        """tts_infer.yaml 의 custom 모델 (학습 모델을 쓴 목소리 다음에 기본으로 돌아갈 때)."""
        if not self.s.dir:
            return "", ""
        path = Path(self.s.config) if self.s.config else Path("GPT_SoVITS/configs/tts_infer.yaml")
        if not path.is_absolute():
            path = Path(self.s.dir) / path
        try:
            import yaml

            data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
            custom = data.get("custom") or {}
            return str(custom.get("t2s_weights_path") or ""), str(custom.get("vits_weights_path") or "")
        except Exception:  # noqa: BLE001 - 파일 없음·yaml 오류면 모르는 것으로
            return "", ""

    def tts(self, text: str, ref_wav: str, ref_text: str, lang: str = "ko") -> bytes:
        body = {
            "text": text, "text_lang": lang, "ref_audio_path": ref_wav, "prompt_text": ref_text,
            "prompt_lang": lang, "top_k": self.s.top_k, "top_p": 1, "temperature": self.s.temperature,
            "text_split_method": "cut5", "batch_size": 1, "speed_factor": self.s.speed,
            "media_type": "wav", "streaming_mode": False, "seed": 42, "parallel_infer": True,
            "repetition_penalty": 1.35,
        }
        try:
            code, data, ctype = _http(self.s.server + "/tts",
                                      json.dumps(body, ensure_ascii=False).encode("utf-8"),
                                      timeout=self.s.sentence_timeout)
        except TimeoutError as err:
            raise TtsError(f"GPT-SoVITS 가 {int(self.s.sentence_timeout)}초 안에 답하지 않았습니다") from err
        except OSError as err:
            raise TtsError(f"GPT-SoVITS 서버에 연결하지 못했습니다: {err}") from err
        if code != 200 or "json" in ctype or not data.startswith(b"RIFF"):
            raise TtsError(f"GPT-SoVITS 합성 실패: {_message(data)}")
        return data


def _message(body: bytes) -> str:
    try:
        got = json.loads(body.decode("utf-8", "replace"))
        if isinstance(got, dict):
            msg = str(got.get("message") or "") + (f" ({got['Exception']})" if got.get("Exception") else "")
            if "3~10" in msg:
                return "참조 음성이 3~10초가 아닙니다. 목소리를 다시 등록하세요"
            return msg[:300] or str(got)[:300]
    except (ValueError, UnicodeDecodeError):
        pass
    return body[:200].decode("utf-8", "replace")


class CloneEngine:
    """Engine 프로토콜: synthesize(text, voice) → Synth. voice = 등록 번호 ('clone:3' 의 3)."""

    name = "clone"

    def __init__(self, library: VoiceLibrary, server: GptSovitsServer, aligner=None):
        self.library = library
        self.server = server
        self.aligner = aligner

    def synthesize(self, text: str, voice: str) -> Synth:
        v = self.library.get(voice)
        self.server.ensure_running()
        self.server.set_weights(str(v.meta.get("gpt_weights") or ""),
                                str(v.meta.get("sovits_weights") or ""))
        lang = v.lang if v.lang in ("ko", "all_ko", "en", "ja", "zh", "auto") else "ko"
        audio = self.server.tts(text, v.ref_wav, v.ref_text, lang)
        words = None
        if self.aligner is not None:
            words = self.aligner.align(audio, text)
        return Synth(audio, words)
