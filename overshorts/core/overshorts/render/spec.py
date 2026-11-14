"""render_spec.json 만들기: Python 과 Remotion 이 대화하는 유일한 파일 (기획서 4).

aligned 작업 → data/jobs/<id>/public/ 에 voice.wav·배경 영상을 두고 spec.json 을 쓴다.
검수 화면의 @remotion/player 미리보기와 최종 렌더가 같은 spec 을 쓴다.
"""

from __future__ import annotations

import json
import random
import re
import shutil
from pathlib import Path
from typing import Any

from ..db import Store

FPS = 60
TAIL_SEC = 0.6
_DIGIT = re.compile(r"\d")
_NOUNISH = re.compile(r"[가-힣A-Za-z0-9]{2,}")
VIDEO_EXT = {".mp4", ".mov", ".webm", ".mkv"}


_PARTICLE = re.compile(r"(?:은|는|이|가|을|를|에|에서|의|도|만|과|와|으로|로|이랑|랑)$")


def emphasis_words(title: str) -> set[str]:
    """핵심 단어 후보: 제목의 두 글자 이상 낱말에서 조사를 뗀 것."""
    out = set()
    for w in _NOUNISH.findall(title or ""):
        stem = _PARTICLE.sub("", w)
        if len(stem) >= 2:
            out.add(stem)
    return out


def _stem(word: str) -> str:
    return _PARTICLE.sub("", re.sub(r"[^가-힣A-Za-z0-9]", "", word))


MAX_EMPHASIS = 6  # 숫자 말고 색 강조할 낱말 수 (많으면 강조가 안 된다)


def pick_background(bg_dir: Path | None, seed: int) -> Path | None:
    if bg_dir is None or not bg_dir.is_dir():
        return None
    try:
        clips = sorted(p for p in bg_dir.iterdir() if p.suffix.lower() in VIDEO_EXT)
    except OSError:
        return None
    return random.Random(seed).choice(clips) if clips else None


def build_spec(timing: dict[str, Any], title: str, subject: str, fact: str, source: str,
               background: str | None, dark: bool = True) -> dict[str, Any]:
    keys = emphasis_words(title)
    used = 0
    scenes = []
    for s in timing.get("sentences") or []:
        words = []
        for w in s.get("words") or []:
            text = str(w.get("text", ""))
            emph = bool(_DIGIT.search(text))
            if not emph and used < MAX_EMPHASIS and _stem(text) in keys:
                emph = True
                used += 1
            words.append({"text": text, "start": w["start"], "end": w["end"], "emphasis": emph})
        scenes.append({"line": s.get("line", 0), "part": s.get("part", "body"), "speaker": s.get("speaker", "narrator"),
                       "text": s.get("text", ""), "start": s.get("start", 0),
                       "end": s.get("end", 0), "words": words})
    duration = float(timing.get("duration") or 0) + TAIL_SEC
    return {
        "version": 1, "fps": FPS, "width": 1080, "height": 1920,
        "duration": round(min(max(duration, 1.0), 180.0), 3),
        "title": title, "subject": subject, "source": source,
        "audio": "voice.wav", "background": background,
        "theme": {"dark": dark, "accent": "#FFD43B"},
        "scenes": scenes,
    }


def spec_pending(store: Store, jobs_dir: Path, bg_dir: Path | None,
                 limit: int = 50) -> dict[str, int]:
    rows = store.conn.execute(
        "SELECT j.id AS job_id, p.gall_id, p.gall_name, p.subject, s.lines FROM jobs j "
        "JOIN posts p ON p.id = j.post_id JOIN scripts s ON s.id = j.script_id "
        "WHERE j.status = 'aligned' ORDER BY j.id LIMIT ?", (max(1, min(limit, 1000)),),
    ).fetchall()
    counts = {"ready": 0, "failed": 0}
    for row in rows:
        job_dir = jobs_dir / str(row["job_id"])
        try:
            timing = json.loads((job_dir / "timing.json").read_text(encoding="utf-8"))
            script = json.loads(row["lines"] or "{}")
            public = job_dir / "public"
            public.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(job_dir / "voice.wav", public / "voice.wav")
            bg = pick_background(bg_dir, int(row["job_id"]))
            bg_name = None
            if bg is not None:
                bg_name = "bg" + bg.suffix.lower()
                shutil.copyfile(bg, public / bg_name)
            gall = row["gall_name"] or f"{row['gall_id']} 갤러리"
            spec = build_spec(timing, str(script.get("title") or ""), str(row["subject"] or ""),
                              str(script.get("fact") or ""), f"출처: DC인사이드 {gall}", bg_name)
            spec_path = job_dir / "spec.json"
            spec_path.write_text(json.dumps(spec, ensure_ascii=False), encoding="utf-8")
        except (OSError, json.JSONDecodeError, KeyError, TypeError) as err:
            store.set_job_status(row["job_id"], "aligned", f"미리보기 준비: {err}")
            counts["failed"] += 1
            continue
        store.conn.execute("UPDATE jobs SET spec_path=? WHERE id=?", (str(spec_path), row["job_id"]))
        store.set_job_status(row["job_id"], "preview_ready", None)
        counts["ready"] += 1
    return counts
