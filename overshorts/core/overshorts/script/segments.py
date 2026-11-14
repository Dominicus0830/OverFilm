"""세그먼트 대본 (기획서 3.3 개정 6, 3.9, 3.10).

AI 는 구성(A: 제목→본문→댓글 / B: 제목→본문)과 세그먼트마다 TTS 켜기/끄기, 보여줄 원문 미디어,
실제 댓글 번호를 고른다. 여기서는 그 답을 믿지 않고 검증한 뒤

- 검수·음성 단계가 쓰는 평평한 `lines` (나레이션 + 'pause' 줄) 를 만들고
- 댓글은 번호로만 고르게 해서 문구가 원문(오타까지) 그대로 나오게 한다.

'pause' 줄은 TTS 를 끈 세그먼트나 화면을 더 보여줄 시간을 무음으로 채운다
(`min_total` = 그 세그먼트가 적어도 이만큼은 화면에 있어야 함).
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from ..media import file_image_size, video_info
from .models import ScriptAnswer

PART = {"title": "hook", "media": "body", "body": "body", "comments": "body"}
MAX_CLIP_SEC = 8.0
_HOSTILE = re.compile(r"조센|쥐센|짱깨|틀딱|한남|김치녀|좆간|안락사|병신|씨발|시발|개새|좆|ㅅㅂ|ㅂㅅ")


def comment_label(c: dict[str, Any]) -> str:
    text = str(c.get("text") or "").replace("\n", " / ")
    con = c.get("dccon") or None
    if con:
        text = (text + " " if text else "") + f"(디시콘: {con.get('name') or '이름 없음'})"
    return text


def numbered_comments(comments: list[dict[str, Any]], limit: int = 50) -> str:
    """[번호] 닉 · 답글 관계 · 내용. 번호는 목록 순서 (0부터)."""
    index = {int(c.get("no") or 0): i for i, c in enumerate(comments)}
    out = []
    for i, c in enumerate(comments[:limit]):
        reply = ""
        if int(c.get("depth") or 0) > 0:
            p = index.get(int(c.get("parent") or 0))
            reply = f"(└ [{p}]의 답글) " if p is not None else "(답글) "
        out.append(f"[{i}] {reply}{comment_label(c)}")
    return "\n".join(out)


def media_items(images: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """쓸 수 있는 원문 미디어 (받았고, 선정성으로 빠지지 않은 것). name = '사진 N' / '영상 N'."""
    out = []
    counters = {"image": 0, "video": 0}
    for item in images:
        kind = "video" if item.get("kind") == "video" else "image"
        counters[kind] += 1
        name = f"{'사진' if kind == 'image' else '영상'} {counters[kind]}"
        local = item.get("blurred") or item.get("local")
        if not local or item.get("dropped") or not exists(local):
            continue
        w, h, secs, audio = item.get("width"), item.get("height"), item.get("seconds"), False
        if not w or not h:
            if kind == "image":
                size = file_image_size(local)
                w, h = size if size else (None, None)
            else:
                info = video_info(local) or {}
                w, h = info.get("width"), info.get("height")
                secs, audio = info.get("seconds"), bool(info.get("audio"))
        out.append({"name": name, "kind": kind, "path": str(local), "width": w, "height": h,
                    "seconds": secs, "audio": audio})
    return out


def media_block(media: list[dict[str, Any]]) -> str:
    if not media:
        return "(쓸 수 있는 원문 사진·영상 없음)"
    rows = []
    for m in media:
        w, h = m.get("width"), m.get("height")
        shape = ""
        if w and h:
            r = w / h
            shape = "가로" if r >= 1.3 else ("세로" if r <= 0.8 else "정사각형 비슷")
            shape = f"{w}x{h} {shape}"
        extra = f", {m['seconds']:.1f}초" if m.get("seconds") else ""
        rows.append(f"- {m['name']}: {shape}{extra}")
    return "\n".join(rows)


def _est_read(text: str) -> float:
    return len(re.sub(r"\s", "", text)) / 10.0 + 0.8


def validate(ans: ScriptAnswer, comments: list[dict[str, Any]], media: list[dict[str, Any]],
             composition: str = "auto", max_comments: int = 6,
             max_media: int = 6) -> tuple[list[dict[str, Any]], list[str]]:
    """AI 답 → 검증한 세그먼트 목록 + 경고. 없는 댓글·미디어 번호는 버린다."""
    warnings: list[str] = []
    by_name = {m["name"]: m for m in media}
    index = {int(c.get("no") or 0): i for i, c in enumerate(comments)}
    used_media = 0
    segs: list[dict[str, Any]] = []
    for seg in ans.segments:
        s: dict[str, Any] = {
            "kind": seg.kind, "subtitle": seg.subtitle.strip(), "tts": bool(seg.tts),
            "tts_reason": seg.tts_reason.strip(),
            "lines": [ln.strip()[:200] for ln in seg.lines if ln.strip()][:12],
            "media": [], "comments": [], "read": [],
            "quote": seg.quote.strip()[:600] if seg.kind == "body" else "",
        }
        if seg.kind == "media":
            for pick in seg.media:
                m = by_name.get(pick.n.strip())
                if m is None or used_media >= max_media:
                    if m is None:
                        warnings.append(f"없는 미디어 '{pick.n}' 는 뺐습니다")
                    continue
                used_media += 1
                s["media"].append({"name": m["name"], "display": pick.display,
                                   "line": min(pick.line, max(0, len(s["lines"]) - 1))})
            if not s["media"]:
                continue  # 원문 미디어 없으면 생략 (기획서 표)
        if seg.kind == "comments":
            if composition == "B":
                continue
            picked: list[int] = []
            for n in seg.comments:
                if 0 <= n < len(comments) and n not in picked:
                    c = comments[n]
                    parent = index.get(int(c.get("parent") or 0)) if int(c.get("depth") or 0) else None
                    if parent is not None and parent not in picked:
                        picked.append(parent)  # 답글은 부모와 같이 (맥락)
                    picked.append(n)
            picked = sorted(set(picked))[:max_comments]
            if not picked:
                continue
            for n in picked:
                if _HOSTILE.search(str(comments[n].get("text") or "")):
                    warnings.append(f"댓글 [{n}] 에 욕설·혐오 표현이 있습니다 (검수에서 확인)")
            s["comments"] = picked
            s["read"] = [n for n in seg.read if n in picked
                         and str(comments[n].get("text") or "").strip()][:2]
            s["lines"] = []
        if seg.kind == "body" and not s["lines"]:
            continue
        segs.append(s)
    kinds = [s["kind"] for s in segs]
    if not kinds or kinds[0] != "title":
        segs.insert(0, {"kind": "title", "subtitle": "", "tts": False, "tts_reason": "제목만 보여줌",
                        "lines": [], "media": [], "comments": [], "read": [], "quote": ""})
    if composition == "A" and "comments" not in kinds:
        warnings.append("구성 A 로 정했지만 고를 만한 댓글이 없어 B 로 만들었습니다")
    return segs, warnings


def display_seconds(seg: dict[str, Any], comments: list[dict[str, Any]],
                    media: list[dict[str, Any]]) -> float:
    """나레이션 없이도 이 세그먼트가 화면에 있어야 할 최소 시간."""
    kind = seg.get("kind")
    if kind == "title":
        return 2.0
    if kind == "media":
        by_name = {m["name"]: m for m in media}
        total = 0.0
        for pick in seg.get("media") or []:
            m = by_name.get(pick.get("name"), {})
            if m.get("kind") == "video":
                total += min(MAX_CLIP_SEC, float(m.get("seconds") or MAX_CLIP_SEC))
            else:
                total += 2.5
        return min(total, 24.0)
    if kind == "body":
        return min(8.0, sum(_est_read(t) for t in seg.get("lines") or []) or 3.0)
    if kind == "comments":
        total = sum(_est_read(comment_label(comments[n])) for n in seg.get("comments") or []
                    if 0 <= n < len(comments))
        return min(14.0, max(2.5, total))
    return 1.0


def flatten(segs: list[dict[str, Any]], comments: list[dict[str, Any]],
            media: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """세그먼트 → 음성 단계가 읽을 줄. TTS 를 끈 세그먼트는 pause 줄만 남는다."""
    lines: list[dict[str, Any]] = []
    for i, seg in enumerate(segs):
        if seg.get("tts"):
            for t in seg.get("lines") or []:
                lines.append({"part": PART.get(seg["kind"], "body"), "text": t, "comment_no": None,
                              "seg": i})
            for n in seg.get("read") or []:
                if 0 <= n < len(comments):
                    lines.append({"part": "comment", "text": str(comments[n].get("text") or ""),
                                  "comment_no": n, "seg": i})
        lines.append({"part": "pause", "text": "", "comment_no": None, "seg": i,
                      "min_total": round(display_seconds(seg, comments, media), 2)})
    return lines


def with_pauses(edited: list[dict[str, Any]], old: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """검수 화면에서 고친 나레이션 줄에 원래 pause 줄을 세그먼트 자리대로 다시 끼운다."""
    pauses = {int(ln.get("seg") or 0): ln for ln in old if ln.get("part") == "pause"}
    groups: dict[int, list[dict[str, Any]]] = {}
    last = 0
    for ln in edited:
        seg = ln.get("seg")
        seg = int(seg) if isinstance(seg, int) and seg >= 0 else last
        last = seg
        groups.setdefault(seg, []).append({**ln, "seg": seg})
    out: list[dict[str, Any]] = []
    for seg in sorted(set(groups) | set(pauses)):
        out += groups.get(seg, [])
        if seg in pauses:
            out.append(pauses[seg])
    return out


def strip_marks(body: str) -> str:
    return "\n".join(ln for ln in (body or "").splitlines()
                     if not re.fullmatch(r"\s*\[(?:사진|영상) \d+\]\s*", ln))


def exists(path: str | Path) -> bool:
    try:
        return Path(path).is_file()
    except OSError:
        return False
