"""목소리 목록 (M6, 기획서 3.4·3.7 보이스 화면).

- 기성 보이스: Edge-TTS 한국어 목소리 (등록 없이 바로 씀).
- 클로닝 보이스: 본인·동의받은 목소리만. 샘플(10초~1분) + 참조 구간(3~10초) + 그 구간 대사 + 동의.
  파일은 `data/voices/<id>/` 에 두고, 목록은 SQLite `voices` 표에 둔다.
- 지금 쓰는 목소리(나레이션·댓글)는 `data/voices/selection.json` 에 둔다. 비어 있으면 config.yaml 의
  `voice.narrator`·`voice.comment` 를 쓴다 (클로닝이 실패하면 돌아갈 기성 보이스도 이 값).
- 작업 하나만 다른 목소리로 읽게 하려면 `data/jobs/<id>/voice.json` (검수 화면).

GPT-SoVITS 는 참조 음성이 3~10초가 아니면 합성을 거부하므로 등록할 때 잘라서 맞춘다.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import wave
from array import array
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any

from ..db import Store
from .engines import TtsError, parse_voice

SAMPLE_RATE = 48_000
REF_MIN_SEC = 3.0
REF_MAX_SEC = 10.0
SAMPLE_MAX_SEC = 90.0  # 받는 샘플 길이 상한 (기획서: 10초~1분)
UPLOAD_MAX_BYTES = 15 * 1024 * 1024
NAME_MAX = 40
TEXT_MAX = 200

# Edge-TTS 한국어 목소리 (2026-11 기준 ko-KR 전부)
EDGE_VOICES: list[dict[str, str]] = [
    {"id": "edge:ko-KR-SunHiNeural", "name": "선희 (여, 기본 나레이션)"},
    {"id": "edge:ko-KR-InJoonNeural", "name": "인준 (남, 기본 댓글)"},
    {"id": "edge:ko-KR-HyunsuMultilingualNeural", "name": "현수 (남, 다국어)"},
]

# 녹음할 때 읽을 문장 (5~8초, 받침·된소리·영어 섞임을 고루)
READ_SENTENCES: list[str] = [
    "오늘은 오버워치 갤러리에서 화제가 된 글을 가져왔어. 댓글 반응까지 같이 보자.",
    "솔직히 이번 패치는 좀 이상한데, 탱커 유저들 반응이 진짜 장난 아니더라.",
    "경쟁전 마지막 판에서 궁극기 세 개가 한꺼번에 터졌는데, 결과는 끝까지 봐야 알아.",
]

CONSENT_TEXT = "이 목소리는 제 목소리이거나, 목소리 주인에게 쇼츠 음성으로 써도 된다는 동의를 받았습니다."

_EXTRA_COLUMNS = {"name": "TEXT NOT NULL DEFAULT ''", "lang": "TEXT NOT NULL DEFAULT 'ko'",
                  "sample_wav": "TEXT", "meta": "TEXT NOT NULL DEFAULT '{}'"}


@dataclass
class CloneVoice:
    id: int
    name: str
    ref_wav: str
    ref_text: str
    lang: str = "ko"
    sample_wav: str | None = None
    created_at: str = ""
    meta: dict[str, Any] = field(default_factory=dict)

    @property
    def spec(self) -> str:
        return f"clone:{self.id}"

    def to_json(self) -> dict[str, Any]:
        return {"id": self.spec, "voice_id": self.id, "name": self.name, "kind": "clone",
                "ref_text": self.ref_text, "lang": self.lang, "created_at": self.created_at,
                "ref_sec": self.meta.get("ref_sec"), "sample_sec": self.meta.get("sample_sec"),
                "gpt_weights": self.meta.get("gpt_weights") or "",
                "sovits_weights": self.meta.get("sovits_weights") or ""}


# --- 오디오 ---
def _ffmpeg() -> str:
    exe = shutil.which("ffmpeg")
    if not exe:
        raise TtsError("FFmpeg 가 없어 목소리를 등록할 수 없습니다 (설치 후 PATH 에 추가)")
    return exe


def to_pcm(audio: bytes, max_sec: float = SAMPLE_MAX_SEC) -> bytes:
    """녹음·업로드 파일(webm·mp3·wav·m4a 등) → 48kHz 모노 16비트 PCM, 최대 max_sec 초."""
    if not audio:
        raise TtsError("음성 파일이 비어 있습니다")
    if len(audio) > UPLOAD_MAX_BYTES:
        raise TtsError("음성 파일이 너무 큽니다 (15MB 까지)")
    try:
        proc = subprocess.run(
            [_ffmpeg(), "-hide_banner", "-loglevel", "error", "-i", "pipe:0", "-t", str(max_sec),
             "-f", "s16le", "-ac", "1", "-ar", str(SAMPLE_RATE), "pipe:1"],
            input=audio, capture_output=True, timeout=120, check=False)
    except (OSError, subprocess.TimeoutExpired) as err:
        raise TtsError(f"FFmpeg 실행 실패: {err}") from err
    if proc.returncode != 0 or len(proc.stdout) < 2:
        raise TtsError("음성 파일을 읽지 못했습니다: "
                       + proc.stderr.decode("utf-8", "replace")[:200])
    pcm = proc.stdout
    return pcm[:-1] if len(pcm) % 2 else pcm


def frame_levels(pcm: bytes, frame_sec: float = 0.02) -> list[float]:
    """20ms 마다 RMS (0~32768)."""
    n = max(1, int(SAMPLE_RATE * frame_sec))
    try:
        import numpy as np

        a = np.frombuffer(pcm[: len(pcm) // 2 * 2], dtype="<i2").astype("float32")
        frames = len(a) // n
        if frames == 0:
            return []
        sq = (a[: frames * n].reshape(frames, n) ** 2).mean(axis=1)
        return [float(v) for v in np.sqrt(sq)]
    except ImportError:
        samples = array("h")
        samples.frombytes(pcm[: len(pcm) // 2 * 2])
        out = []
        for i in range(0, len(samples) - n + 1, n):
            chunk = samples[i:i + n]
            out.append((sum(s * s for s in chunk) / n) ** 0.5)
        return out


def speech_span(pcm: bytes, start: float = 0.0, end: float | None = None,
                pad: float = 0.15) -> tuple[float, float]:
    """[start, end] 안에서 앞뒤 무음을 걷어 낸 말소리 구간 (초)."""
    total = len(pcm) / 2 / SAMPLE_RATE
    end = total if end is None else min(end, total)
    start = max(0.0, min(start, end))
    levels = frame_levels(pcm)
    if not levels:
        return start, end
    peak = max(levels)
    # 말소리 = 최대값의 6% 이상이면서 아주 작은 잡음(약 -55dBFS)보다 큰 프레임
    floor = max(60.0, peak * 0.06)
    lo, hi = int(start / 0.02), min(len(levels), int(end / 0.02) + 1)
    voiced = [i for i in range(lo, hi) if levels[i] >= floor]
    if not voiced:
        return start, end
    s = max(start, voiced[0] * 0.02 - pad)
    e = min(end, (voiced[-1] + 1) * 0.02 + pad)
    return round(s, 3), round(e, 3)


def write_wav(path: Path, pcm: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE)
        w.writeframes(pcm)


def cut(pcm: bytes, start: float, end: float) -> bytes:
    a = max(0, int(start * SAMPLE_RATE)) * 2
    b = max(a, int(end * SAMPLE_RATE)) * 2
    return pcm[a:b]


def clean_text(text: str, limit: int = TEXT_MAX) -> str:
    return re.sub(r"\s+", " ", str(text or "")).strip()[:limit]


# --- 목록 ---
class VoiceLibrary:
    def __init__(self, store: Store, root: Path):
        self.store = store
        self.root = Path(root)
        self._ensure_columns()

    def _ensure_columns(self) -> None:
        have = {r[1] for r in self.store.conn.execute("PRAGMA table_info(voices)").fetchall()}
        for col, decl in _EXTRA_COLUMNS.items():
            if col not in have:
                self.store.conn.execute(f"ALTER TABLE voices ADD COLUMN {col} {decl}")
        self.store.conn.commit()

    def _row(self, row: Any) -> CloneVoice:
        try:
            meta = json.loads(row["meta"] or "{}")
        except json.JSONDecodeError:
            meta = {}
        return CloneVoice(int(row["id"]), str(row["name"] or f"목소리 {row['id']}"),
                          str(row["ref_wav"] or ""), str(row["ref_text"] or ""),
                          str(row["lang"] or "ko"), row["sample_wav"], str(row["created_at"] or ""),
                          meta if isinstance(meta, dict) else {})

    def clones(self) -> list[CloneVoice]:
        rows = self.store.conn.execute(
            "SELECT * FROM voices WHERE kind='clone' AND consent=1 ORDER BY id").fetchall()
        return [self._row(r) for r in rows]

    def get(self, voice_id: int | str) -> CloneVoice:
        try:
            vid = int(voice_id)
        except (TypeError, ValueError) as err:
            raise TtsError(f"목소리 번호가 이상합니다: {voice_id}") from err
        row = self.store.conn.execute(
            "SELECT * FROM voices WHERE id=? AND kind='clone' AND consent=1", (vid,)).fetchone()
        if row is None:
            raise TtsError(f"등록된 클로닝 목소리가 없습니다 (번호 {vid}). 보이스 화면에서 확인하세요")
        voice = self._row(row)
        if not Path(voice.ref_wav).is_file():
            raise TtsError(f"'{voice.name}' 참조 음성 파일이 없습니다: {voice.ref_wav}")
        return voice

    def all_voices(self) -> list[dict[str, Any]]:
        out = [{**v, "kind": "preset"} for v in EDGE_VOICES]
        return out + [v.to_json() for v in self.clones()]

    def known(self, spec: str) -> bool:
        engine, voice = parse_voice(spec)
        if engine == "edge":
            return bool(re.fullmatch(r"[A-Za-z]{2,3}-[A-Za-z]{2,4}-[A-Za-z]+Neural", voice))
        if engine == "clone":
            try:
                self.get(voice)
            except TtsError:
                return False
            return True
        return False

    def register(self, name: str, audio: bytes, ref_text: str, consent: bool,
                 start: float | None = None, end: float | None = None, lang: str = "ko",
                 gpt_weights: str = "", sovits_weights: str = "") -> CloneVoice:
        """샘플을 받아 참조 구간(3~10초)을 잘라 등록한다.

        start·end 를 안 주면 앞뒤 무음을 걷어 낸 전체를 쓴다 (10초를 넘으면 구간을 정하라고 알린다).
        """
        if not consent:
            raise TtsError("동의 확인이 필요합니다. 본인 목소리이거나 동의받은 목소리만 등록할 수 있습니다")
        name = clean_text(name, NAME_MAX)
        ref_text = clean_text(ref_text)
        if not name:
            raise TtsError("목소리 이름을 적어 주세요")
        if len(re.findall(r"[가-힣A-Za-z0-9]", ref_text)) < 4:
            raise TtsError("참조 구간에서 말한 내용을 그대로 적어 주세요 (정확할수록 비슷해집니다)")
        if lang not in ("ko", "all_ko", "en", "ja", "zh", "auto"):
            lang = "ko"
        pcm = to_pcm(audio)
        total = len(pcm) / 2 / SAMPLE_RATE
        if total < REF_MIN_SEC:
            raise TtsError(f"샘플이 {total:.1f}초로 너무 짧습니다 (3초 이상)")
        picked = start is not None or end is not None
        s0 = float(start or 0.0)
        e0 = float(end) if end is not None else total
        if not 0 <= s0 < e0:
            raise TtsError("참조 구간의 시작·끝이 이상합니다")
        s, e = speech_span(pcm, s0, e0)
        length = e - s
        if length > REF_MAX_SEC:
            hint = "고른 구간이" if picked else "말소리가"
            raise TtsError(f"{hint} {length:.1f}초입니다. GPT-SoVITS 참조 음성은 3~10초여야 하니 "
                           "시작·끝을 정해 그 부분만 고르고, 그 부분에서 한 말을 적어 주세요")
        if length < REF_MIN_SEC:
            raise TtsError(f"말소리가 {length:.1f}초뿐입니다. 3~10초가 되게 조금 더 길게 말해 주세요")
        row = self.store.conn.execute(
            "INSERT INTO voices (kind, engine, ref_wav, ref_text, consent, created_at) "
            "VALUES ('clone', 'gpt_sovits', '', ?, 1, ?)",
            (ref_text, datetime.now().isoformat(timespec="seconds")))
        vid = int(row.lastrowid or 0)
        folder = self.root / str(vid)
        try:
            write_wav(folder / "sample.wav", pcm)
            write_wav(folder / "ref.wav", cut(pcm, s, e))
            (folder / "ref.txt").write_text(ref_text, encoding="utf-8")
        except OSError as err:
            self.store.conn.rollback()
            raise TtsError(f"목소리 파일을 저장하지 못했습니다: {err}") from err
        meta = {"ref_sec": round(length, 2), "ref_range": [s, e], "sample_sec": round(total, 2),
                "consent": CONSENT_TEXT, "consent_at": datetime.now().isoformat(timespec="seconds"),
                "gpt_weights": clean_text(gpt_weights, 500),
                "sovits_weights": clean_text(sovits_weights, 500)}
        self.store.conn.execute(
            "UPDATE voices SET ref_wav=?, name=?, lang=?, sample_wav=?, meta=? WHERE id=?",
            (str((folder / "ref.wav").resolve()), name, lang,
             str((folder / "sample.wav").resolve()), json.dumps(meta, ensure_ascii=False), vid))
        self.store.conn.commit()
        return self.get(vid)

    def rename(self, voice_id: int, name: str) -> None:
        name = clean_text(name, NAME_MAX)
        if not name:
            raise TtsError("목소리 이름을 적어 주세요")
        self.store.conn.execute("UPDATE voices SET name=? WHERE id=? AND kind='clone'",
                                (name, int(voice_id)))
        self.store.conn.commit()

    def delete(self, voice_id: int) -> None:
        vid = int(voice_id)
        self.store.conn.execute("DELETE FROM voices WHERE id=? AND kind='clone'", (vid,))
        self.store.conn.commit()
        folder = (self.root / str(vid)).resolve()
        if folder.parent == self.root.resolve() and folder.is_dir():
            shutil.rmtree(folder, ignore_errors=True)
        sel = self.selection()
        changed = {k: v for k, v in sel.items() if v == f"clone:{vid}"}
        if changed:
            self.save_selection({k: None for k in changed})

    # --- 지금 쓰는 목소리 ---
    def _selection_path(self) -> Path:
        return self.root / "selection.json"

    def selection(self) -> dict[str, str | None]:
        try:
            got = json.loads(self._selection_path().read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return {}
        if not isinstance(got, dict):
            return {}
        return {k: (str(got[k]) if got.get(k) else None)
                for k in ("narrator", "comment") if k in got}

    def save_selection(self, values: dict[str, str | None]) -> dict[str, str | None]:
        sel = self.selection()
        for key in ("narrator", "comment"):
            if key not in values:
                continue
            v = values[key]
            if v and v != "same" and not self.known(v):
                raise TtsError(f"모르는 목소리입니다: {v}")
            sel[key] = v or None
        self.root.mkdir(parents=True, exist_ok=True)
        self._selection_path().write_text(json.dumps(sel, ensure_ascii=False), encoding="utf-8")
        return sel


def job_override_path(jobs_dir: Path, job_id: int) -> Path:
    return jobs_dir / str(int(job_id)) / "voice.json"


def job_override(jobs_dir: Path, job_id: int) -> dict[str, str | None]:
    try:
        got = json.loads(job_override_path(jobs_dir, job_id).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    if not isinstance(got, dict):
        return {}
    return {k: (str(got[k]) if got.get(k) else None) for k in ("narrator", "comment") if k in got}


def resolve(cfg_voice: dict[str, Any], selection: dict[str, str | None],
            override: dict[str, str | None] | None = None) -> tuple[str, str | None]:
    """(나레이션, 댓글) 목소리. 작업별 > 보이스 화면 선택 > config.yaml.

    댓글 값 'same' = 나레이션 목소리로 댓글도 읽음 (화자 분리 끔).
    """
    narrator = str(cfg_voice.get("narrator") or "edge:ko-KR-SunHiNeural")
    comment = str(cfg_voice.get("comment") or "") or None
    for src in (selection, override or {}):
        if src.get("narrator"):
            narrator = str(src["narrator"])
        if "comment" in src and src.get("comment"):
            comment = str(src["comment"])
    if comment == "same":
        comment = None
    return narrator, comment
