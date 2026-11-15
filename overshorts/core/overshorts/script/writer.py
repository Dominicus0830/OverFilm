"""대본 쓰기 단계: 작성 → 규칙 검사 → 오글 검사 → 필요하면 한 번 다시 쓰기 (기획서 3.3).

1. Claude 가 스타일 가이드와 좋은 예·나쁜 예를 보고 쓴다.
2. 규칙 검사 (금지 표현, 문장 길이, 원문 겹침, 지어낸 댓글). 걸리면 사유를 붙여 한 번 다시.
3. 오글 검사: 별도 호출로 0~10 채점. 기준 미만이면 사유를 붙여 한 번 다시.
4. 그래도 남은 문제는 검수 화면 경고로 넘긴다 (자동으로 버리지 않음).
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path

from ..db import Store
from ..llm import LimitReached, LlmError, Router
from .checks import CheckResult, _no_ending, check_script, load_banned
from .game_check import check_script_game
from .reactions import list_memes, meme_block, resolve_segments
from .models import CringeAnswer, ScriptAnswer
from .prompts import cringe_task, load_style, script_task
from .segments import flatten, media_items, validate
from .title_rules import TITLE_BANNED

MAX_REWRITES = 2


@dataclass
class ScriptSettings:
    style_dir: str | Path | None = None
    tone: str = "담백"
    speech: str = "반말"
    min_cringe_score: float = 7.0
    chars_per_sec: float = 7.0
    cringe_check: bool = True
    composition: str = "auto"  # auto | A | B
    max_comments: int = 6
    max_media: int = 6
    title_tail: str = "auto"  # none | ㄷㄷ | ㅋㅋㅋ | auto (제목 꼬리표, 채널 말투)
    assets_dir: str | Path | None = None  # 영웅 프로필 캐시 (없으면 짤 그림을 받지 않음)


@dataclass
class ScriptOutcome:
    script: ScriptAnswer
    segments: list[dict]
    lines: list[dict]
    checks: CheckResult
    cringe: CringeAnswer | None
    model: str
    rewrites: int
    warnings: list[str] = field(default_factory=list)


def write_script(router: Router, title: str, body: str, comments: list[dict],
                 settings: ScriptSettings, media: list[dict] | None = None,
                 posted_at: str | None = None) -> ScriptOutcome:
    """comments: 수집한 댓글 dict 목록 (text, depth, parent, dccon …). media: segments.media_items."""
    media = media or []
    texts = [str(c.get("text") or "") for c in comments]
    source = "\n".join([title, body, *texts])
    style = load_style(settings.style_dir)
    banned = load_banned(settings.style_dir)
    user_words = [p for p, why in banned if why == "사용자 금지 목록"]
    feedback: list[str] | None = None
    rewrites = 0
    memes = list_memes(settings.style_dir)
    while True:
        task = script_task(title, body, comments, media, style, settings.tone, settings.speech,
                           user_words, settings.chars_per_sec, feedback, settings.composition,
                           settings.max_comments, settings.title_tail, meme_block(memes))
        script, model = router.ask(task, ScriptAnswer)
        segs, seg_warn = validate(script, comments, media, settings.composition,
                                  settings.max_comments, settings.max_media, source, memes,
                                  f"{title}\n{body}")
        lines = flatten(segs, comments, media)
        spoken = [ln for ln in lines if ln["part"] != "pause"]
        checks = check_script(spoken, body, texts, banned, settings.chars_per_sec)
        cringe = None
        problems: list[str] = []
        title_issues = check_title(script.title) + check_script_game(
            script.title, [ln["text"] for ln in spoken if ln["part"] != "comment"], source, posted_at)
        title_issues += missing_lines(body, segs)
        if checks.needs_rewrite or title_issues:
            problems += checks.reasons() + title_issues
        elif settings.cringe_check and spoken:
            pairs = [(ln["part"], ln["text"]) for ln in spoken]
            cringe, _ = router.ask(cringe_task(pairs), CringeAnswer)
            if cringe.score < settings.min_cringe_score:
                problems.append(f"오글 점수 {cringe.score:.1f} (기준 {settings.min_cringe_score:g})")
                problems += [f"{i.line + 1}번째 나레이션 줄: {i.reason}" for i in cringe.issues]
        if not problems or rewrites >= MAX_REWRITES:
            warnings = seg_warn + checks.reasons() + title_issues
            if settings.assets_dir is not None:
                warnings += resolve_segments(segs, style_dir=settings.style_dir,
                                             cache=Path(settings.assets_dir), router=router,
                                             context=f"{title}\n{body[:1000]}", media=media)
            if cringe is not None and cringe.score < settings.min_cringe_score:
                warnings.append(f"오글 점수 {cringe.score:.1f} 로 기준 미만")
            return ScriptOutcome(script, segs, lines, checks, cringe, model, rewrites, warnings)
        feedback = problems
        rewrites += 1


def script_pending(store: Store, router: Router, settings: ScriptSettings,
                   post_ids: list[int] | None = None, limit: int = 20) -> dict[str, int]:
    """LLM 필터를 통과한 글(jobs.status = filtered)의 대본을 쓴다."""
    q = ("SELECT j.id AS job_id, p.id AS post_id, p.title, p.body, p.comments, p.images, p.posted_at FROM jobs j "
         "JOIN posts p ON p.id = j.post_id WHERE j.status = 'filtered' AND p.discarded = 0")
    args: list = []
    if post_ids:
        q += f" AND p.id IN ({','.join('?' for _ in post_ids[:500])})"
        args += post_ids[:500]
    q += " ORDER BY j.id LIMIT ?"
    args.append(max(1, min(limit, 1000)))
    counts = {"scripted": 0, "failed": 0, "warned": 0, "limited": 0}
    for row in store.conn.execute(q, args).fetchall():
        comments = [c for c in json.loads(row["comments"] or "[]") if isinstance(c, dict)]
        media = media_items(json.loads(row["images"] or "[]"))
        try:
            out = write_script(router, row["title"], row["body"], comments, settings, media,
                               row["posted_at"])
        except LlmError as err:
            store.set_job_status(row["job_id"], "filtered", f"대본: {err}")
            counts["failed"] += 1
            if isinstance(err, LimitReached):
                counts["limited"] = 1
                break  # 대기열에 그대로 두고 나중에
            continue
        version = store.next_script_version(row["post_id"])
        payload = {
            "title": out.script.title,
            "title_alts": [t for t in out.script.title_alts if t.strip() and t != out.script.title][:2],
            "fact": out.script.fact,
            "composition": "A" if any(sg["kind"] == "comments" for sg in out.segments) else "B",
            "composition_reason": out.script.composition_reason,
            "segments": out.segments,
            "lines": out.lines,
            "est_seconds": out.checks.est_seconds,
            "cringe": out.cringe.model_dump() if out.cringe else None,
            "warnings": out.warnings,
            "rewrites": out.rewrites,
        }
        script_id = store.add_script(row["post_id"], version, settings.tone, payload,
                                     out.checks.overlap_ratio, out.model)
        store.set_job_status(row["job_id"], "scripted", None, script_id=script_id)
        counts["scripted"] += 1
        counts["warned"] += 1 if out.warnings else 0
    return counts


def missing_lines(body: str, segs: list[dict]) -> list[str]:
    """짧은 뻘글이면 본문 줄마다 짤 장면이 있어야 한다 (줄을 빼먹으면 정보 전달이 안 됨)."""
    from difflib import SequenceMatcher

    from .prompts import is_short_post

    if not is_short_post(body):
        return []
    says = [str(sg.get("say") or "") for sg in segs if sg.get("kind") == "reaction"]
    if not says:
        return []  # 짤 장면 구성을 고르지 않았으면 (나레이션 구성) 줄 검사는 하지 않는다
    key = lambda t: re.sub(r"[^가-힣a-zA-Z0-9]", "", t)  # noqa: E731
    out = []
    lines = [ln.strip() for ln in body.splitlines()
             if ln.strip() and not re.fullmatch(r"\[(?:사진|영상) \d+\]", ln.strip())]
    for i, ln in enumerate(lines[:8]):
        if not any(SequenceMatcher(None, key(ln), key(sy)).ratio() >= 0.55 for sy in says):
            out.append(f"본문 {i + 1}번째 줄 '{ln[:20]}' 의 짤 장면이 빠졌습니다 (줄마다 하나씩, 순서대로)")
    return out


def check_title(title: str) -> list[str]:
    """제목 규칙 중 기계로 볼 수 있는 것: 금지어, 길이(12~24자 안팎)."""
    out = [f"제목 금지어 '{w}'" for w in TITLE_BANNED if w in title]
    n = len(title.replace(" ", ""))
    if n > 28:
        out.append(f"제목이 깁니다 ({n}자, 12~24자)")
    if re.search(r"[가-힣]+노[\s?!.~]*$", title) and _no_ending(title):
        out.append("제목에 ~노 어미")
    return out
