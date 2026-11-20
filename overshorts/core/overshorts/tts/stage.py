"""음성 단계: 대본이 있는 작업(jobs.status = scripted) → voice.wav + timing.json (M3).

Edge-TTS 는 단어 경계 시각을 주므로 자막 타이밍(aligned)까지 한 번에 끝난다.
"""

from __future__ import annotations

import json
from pathlib import Path

from ..db import Store
from .assemble import build_voice
from .engines import Engine, TtsError


def voice_pending(store: Store, jobs_dir: Path, narrator: str, comment_voice: str | None,
                  engines: dict[str, Engine] | None = None, decode=None,
                  limit: int = 20, lint: bool = False, style_dir: str | None = None,
                  verify: bool = False, stt_command: str | None = None) -> dict[str, int]:
    """lint: 읽기 전에 대본을 검사해 걸리면 그 작업을 멈춘다 (S3, 기본 꺼짐).
    verify: 음성을 STT 로 다시 받아 대본과 대조해 verify.json 을 남긴다 (S4, 기본 꺼짐)."""
    rows = store.conn.execute(
        "SELECT j.id AS job_id, s.lines FROM jobs j JOIN scripts s ON s.id = j.script_id "
        "WHERE j.status = 'scripted' ORDER BY j.id LIMIT ?", (max(1, min(limit, 1000)),),
    ).fetchall()
    counts = {"voiced": 0, "failed": 0, "linted": 0, "verify_problems": 0}
    for row in rows:
        try:
            payload = json.loads(row["lines"] or "{}")
            lines = payload.get("lines") if isinstance(payload, dict) else None
            if not isinstance(lines, list):
                raise TtsError("대본 줄이 없습니다")
            if lint:
                from ..script.lint import lint_lines

                found = lint_lines(lines, style_dir)
                if not found.ok:
                    store.set_job_status(row["job_id"], "scripted", "대본 린터: " + "; ".join(found.errors[:3]))
                    counts["linted"] += 1
                    continue
            kwargs = {"decode": decode} if decode else {}
            build_voice(lines, jobs_dir / str(row["job_id"]), narrator, comment_voice,
                                engines, **kwargs)
            if verify:
                from .verify import verify_voice

                counts["verify_problems"] += len(verify_voice(jobs_dir / str(row["job_id"]), lines,
                                                              stt_command).problems)
        except (TtsError, json.JSONDecodeError, OSError) as err:
            store.set_job_status(row["job_id"], "scripted", f"음성: {err}")
            counts["failed"] += 1
            continue
        store.set_job_status(row["job_id"], "aligned", None)
        counts["voiced"] += 1
    return counts
