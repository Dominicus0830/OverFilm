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
                  limit: int = 20) -> dict[str, int]:
    rows = store.conn.execute(
        "SELECT j.id AS job_id, s.lines FROM jobs j JOIN scripts s ON s.id = j.script_id "
        "WHERE j.status = 'scripted' ORDER BY j.id LIMIT ?", (max(1, min(limit, 1000)),),
    ).fetchall()
    counts = {"voiced": 0, "failed": 0}
    for row in rows:
        try:
            payload = json.loads(row["lines"] or "{}")
            lines = payload.get("lines") if isinstance(payload, dict) else None
            if not isinstance(lines, list):
                raise TtsError("대본 줄이 없습니다")
            kwargs = {"decode": decode} if decode else {}
            build_voice(lines, jobs_dir / str(row["job_id"]), narrator, comment_voice,
                                engines, **kwargs)
        except (TtsError, json.JSONDecodeError, OSError) as err:
            store.set_job_status(row["job_id"], "scripted", f"음성: {err}")
            counts["failed"] += 1
            continue
        store.set_job_status(row["job_id"], "aligned", None)
        counts["voiced"] += 1
    return counts
