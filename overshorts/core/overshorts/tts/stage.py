"""음성 단계: 대본이 있는 작업(jobs.status = scripted) → voice.wav + timing.json (M3, M6).

Edge-TTS 는 단어 경계 시각을 주므로 자막 타이밍(aligned)까지 한 번에 끝난다.
클로닝(GPT-SoVITS)이 실패하면 그 작업 전체를 기성 보이스(Edge-TTS)로 다시 읽고 검수 경고를 남긴다
(문장마다 목소리가 바뀌지 않게 작업 단위로 바꾼다).
"""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

from ..db import Store
from .assemble import build_voice
from .engines import EdgeTts, Engine, TtsError, parse_voice

DEFAULT_NARRATOR = "edge:ko-KR-SunHiNeural"
DEFAULT_COMMENT = "edge:ko-KR-InJoonNeural"


def _uses_clone(*specs: str | None) -> bool:
    return any(s and parse_voice(s)[0] == "clone" for s in specs)


def _add_warning(job_dir: Path, message: str) -> None:
    path = job_dir / "timing.json"
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        data["warnings"] = [*(data.get("warnings") or []), message]
        path.write_text(json.dumps(data, ensure_ascii=False, indent=1), encoding="utf-8")
    except (OSError, json.JSONDecodeError, AttributeError):
        pass


def voice_pending(store: Store, jobs_dir: Path, narrator: str, comment_voice: str | None,
                  engines: dict[str, Engine] | None = None, decode=None,
                  limit: int = 20, fallback: tuple[str, str | None] | None = None,
                  override: Callable[[int], tuple[str, str | None]] | None = None,
                  ) -> dict[str, int]:
    """fallback = 클로닝이 실패했을 때 읽을 (나레이션, 댓글) 기성 보이스.
    override(job_id) = 그 작업만 쓸 (나레이션, 댓글)."""
    rows = store.conn.execute(
        "SELECT j.id AS job_id, s.lines FROM jobs j JOIN scripts s ON s.id = j.script_id "
        "WHERE j.status = 'scripted' ORDER BY j.id LIMIT ?", (max(1, min(limit, 1000)),),
    ).fetchall()
    counts = {"voiced": 0, "failed": 0, "fallback": 0}
    engines = engines if engines is not None else {}
    for row in rows:
        job_id = int(row["job_id"])
        job_dir = jobs_dir / str(job_id)
        narr, cmt = override(job_id) if override else (narrator, comment_voice)
        kwargs = {"decode": decode} if decode else {}
        try:
            payload = json.loads(row["lines"] or "{}")
            lines = payload.get("lines") if isinstance(payload, dict) else None
            if not isinstance(lines, list):
                raise TtsError("대본 줄이 없습니다")
            try:
                build_voice(lines, job_dir, narr, cmt, engines, **kwargs)
            except TtsError as err:
                if not (fallback and _uses_clone(narr, cmt)):
                    raise
                fb_n = fallback[0] if _uses_clone(narr) else narr
                fb_c = (fallback[1] if _uses_clone(cmt) else cmt) if cmt else None
                build_voice(lines, job_dir, fb_n, fb_c, engines, **kwargs)
                _add_warning(job_dir, f"목소리 클로닝 실패로 기성 보이스(Edge-TTS)로 읽었습니다: {err}"[:300])
                counts["fallback"] += 1
        except (TtsError, json.JSONDecodeError, OSError) as err:
            store.set_job_status(job_id, "scripted", f"음성: {err}")
            counts["failed"] += 1
            continue
        store.set_job_status(job_id, "aligned", None)
        counts["voiced"] += 1
    return counts


def fallback_voices(cfg_voice: dict[str, Any]) -> tuple[str, str | None]:
    """config.yaml 의 목소리가 기성 보이스면 그것, 아니면 기본 Edge 목소리."""
    narr = str(cfg_voice.get("narrator") or "")
    cmt = str(cfg_voice.get("comment") or "")
    return (narr if parse_voice(narr)[0] == "edge" and narr else DEFAULT_NARRATOR,
            cmt if parse_voice(cmt)[0] == "edge" and cmt else DEFAULT_COMMENT)


def cpu_warning() -> str | None:
    """NVIDIA 가 아니면 GPT-SoVITS 가 CPU 로 돈다 (사용자 결정: 느려도 허용, 경고)."""
    try:
        from ..render.runner import detect_gpu

        vendor = detect_gpu().vendor
    except Exception:  # noqa: BLE001 - 감지 실패면 경고 안 함
        return None
    if vendor == "nvidia":
        return None
    why = {"amd": "라데온이라", "intel": "인텔 그래픽이라", "none": "GPU 가 없어"}.get(vendor, "NVIDIA 가 아니라")
    return f"{why} 목소리 클로닝은 CPU 로 합성합니다 (느림, 문장마다 수 초~수십 초)"


_cpu_warning_cache: list[str | None] = []


def voice_stage(store: Store, data: Path, cfg: dict[str, Any], limit: int = 20,
                say: Callable[[str], None] | None = None) -> dict[str, int]:
    """패널·CLI 공용 음성 단계. 목소리: 작업별(검수) > 보이스 화면 선택 > config.yaml."""
    from .align import make_aligner
    from .gpt_sovits import CloneEngine, CloneSettings, GptSovitsServer
    from .voices import VoiceLibrary, job_override, resolve

    cfg_voice = cfg.get("voice") or {}
    lib = VoiceLibrary(store, data / "voices")
    selection = lib.selection()
    narrator, comment = resolve(cfg_voice, selection)
    jobs_dir = data / "jobs"

    def per_job(job_id: int) -> tuple[str, str | None]:
        return resolve(cfg_voice, selection, job_override(jobs_dir, job_id))

    settings = CloneSettings.from_cfg(cfg)
    server = GptSovitsServer.shared(settings, data / "voices" / "gpt-sovits.log")
    aligner = make_aligner(settings.align, settings.align_model)
    engines: dict[str, Engine] = {"edge": EdgeTts(),
                                  "clone": CloneEngine(lib, server, aligner)}
    if say and _uses_clone(narrator, comment):
        if not _cpu_warning_cache:
            _cpu_warning_cache.append(cpu_warning())
        if _cpu_warning_cache[0]:
            say(_cpu_warning_cache[0])
    try:
        return voice_pending(store, jobs_dir, narrator, comment, engines, limit=limit,
                             fallback=fallback_voices(cfg_voice), override=per_job)
    finally:
        # VRAM: 음성 단계가 끝나면 우리가 띄운 TTS 서버·정렬 모델을 내린다
        if settings.unload_after:
            server.stop()
        if aligner is not None:
            aligner.unload()
