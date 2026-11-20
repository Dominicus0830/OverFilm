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
from ..presets import Preset, page_seconds
from .models import ScriptAnswer

PART = {"title": "hook", "media": "body", "body": "body", "comments": "body", "reaction": "body"}
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


# 디시 댓글창 스크린샷 한 장 (가상 480px 화면 기준). 화면 28~66% 를 1040/480 배로 키워 채운다.
SKIN_W = 480
SKIN_SCALE = 1040 / SKIN_W
PAGE_H = (0.66 - 0.28) * 1920 / SKIN_SCALE - 8
MAX_COMMENT_PAGES = 4


def page_height(stage_top: float, stage_bottom: float, height: int = 1920) -> float:
    """댓글 스크린샷 한 장이 들어갈 가상 480px 화면 높이. 무대(%)가 넓을수록 한 장에 더 많이 보인다."""
    return max(120.0, (stage_bottom - stage_top) / 100 * height / SKIN_SCALE - 8)


def comment_height(c: dict[str, Any]) -> float:
    """디시 댓글 한 줄의 높이 어림. 글 칸이 닉 칸 오른쪽이라 한 줄에 20자, 답글은 17자 안팎."""
    per_line = 17 if int(c.get("depth") or 0) else 20
    text = str(c.get("text") or "")
    lines = sum(max(1, -(-len(p) // per_line)) for p in text.split("\n")) if text else 0
    con = 96 if c.get("dccon") else 0
    return max(lines, 0 if con else 1) * 19.5 + con + 17


def comment_windows(picked: list[int], comments: list[dict[str, Any]],
                    page_h: float = PAGE_H, max_pages: int = MAX_COMMENT_PAGES) -> list[list[int]]:
    """고른 댓글이 들어간 '스크린샷' 장들. 고른 댓글이 있는 글타래부터 화면이 꽉 찰 때까지
    이어지는 댓글을 원래 순서 그대로 담는다 (디시콘·답글 포함, 사이를 건너뛰지 않음)."""
    pages: list[list[int]] = []
    covered: set[int] = set()
    for n in sorted(set(picked)):
        if n in covered or not 0 <= n < len(comments) or len(pages) >= max_pages:
            continue
        start = n
        while start > 0 and int(comments[start].get("depth") or 0) and start - 1 not in covered:
            start -= 1  # 답글이면 원댓글부터
        page: list[int] = []
        h = 0.0
        k = start
        while k < len(comments) and k not in covered:
            ch = comment_height(comments[k])
            if page and h + ch > page_h:
                break
            page.append(k)
            h += ch
            k += 1
        # 화면 끝에서 원댓글만 남고 답글이 다음 장으로 넘어가면 그 원댓글은 다음 장으로
        while len(page) > 1 and k < len(comments) and int(comments[k].get("depth") or 0) \
                and not int(comments[page[-1]].get("depth") or 0) and page[-1] not in picked:
            page.pop()
        # 댓글 끝이라 덜 찼으면 앞쪽 댓글로 채운다 (스크린샷은 칸이 꽉 차 있어야 함)
        while page and page[0] > 0 and page[0] - 1 not in covered:
            ch = comment_height(comments[page[0] - 1])
            if h + ch > page_h:
                break
            page.insert(0, page[0] - 1)
            h += ch
        covered.update(page)
        pages.append(page)
    return sorted(pages)


def _thread_units(nos: list[int], comments: list[dict[str, Any]]) -> list[list[int]]:
    """고른 댓글 번호 → 글타래 묶음 (원댓글 + 고른 답글). 답글만 골랐으면 원댓글을 앞에 붙인다."""
    index = {int(c.get("no") or 0): i for i, c in enumerate(comments)}
    units: dict[int, list[int]] = {}
    for n in sorted({n for n in nos if 0 <= n < len(comments)}):
        root = n
        if int(comments[n].get("depth") or 0):
            parent = index.get(int(comments[n].get("parent") or 0))
            if parent is not None and parent < n:
                root = parent
        unit = units.setdefault(root, [root] if root != n else [])
        if n not in unit:
            unit.append(n)
    return [sorted(u) for _, u in sorted(units.items())]


def pack_screens(screens: list[list[int]], picked: list[int], comments: list[dict[str, Any]],
                 page_h: float, max_screens: int, max_total: int,
                 per_screen: tuple[int, int] = (1, 3)) -> list[list[int]]:
    """LLM 이 정한 화면 묶음을 믿지 않고 다듬는다 (S19).

    - 없는 번호·중복은 뺀다. 답글을 고르면 원댓글이 같이 들어가고, 원댓글과 답글은 쪼개지 않는다.
    - 화면 높이(page_h)를 넘으면 글타래 단위로 다음 장으로 넘긴다 (한 글타래가 혼자 넘치면 그대로 한 장).
    - LLM 이 묶음을 안 줬으면 고른 댓글을 per_screen 위쪽 개수와 높이 안에서 차례로 채운다.
    - 장 수·전체 개수 상한을 넘으면 뒤쪽을 버린다. 장은 시간순.
    """
    groups = [g for g in screens if g] or [list(picked)]
    claimed: set[int] = set()
    pages: list[list[int]] = []
    cap = max(1, per_screen[1]) if not screens else 99
    for g in groups:
        cur: list[int] = []
        h = 0.0
        for unit in _thread_units([n for n in g if n not in claimed], comments):
            unit = [n for n in unit if n not in claimed]
            if not unit:
                continue
            uh = sum(comment_height(comments[n]) for n in unit)
            if cur and (h + uh > page_h or len(cur) + len(unit) > cap):
                pages.append(cur)
                cur, h = [], 0.0
            cur += unit
            h += uh
            claimed.update(unit)
        if cur:
            pages.append(cur)
    pages.sort(key=lambda pg: pg[0])
    out: list[list[int]] = []
    total = 0
    for pg in pages[:max(1, max_screens)]:
        if total + len(pg) > max_total:
            pg = pg[:max_total - total]
        if not pg:
            break
        out.append(pg)
        total += len(pg)
    return out


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
             max_media: int = 6, source: str = "",
             memes: list[Any] | None = None,
             post_text: str = "",
             preset: Preset | None = None) -> tuple[list[dict[str, Any]], list[str]]:
    """AI 답 → 검증한 세그먼트 목록 + 경고. 없는 댓글·미디어 번호는 버린다."""
    from .reactions import fix_reaction, main_hero

    n_reaction = 0
    main = main_hero(post_text or source)  # 글 주인공은 제목·본문으로 (댓글은 영웅이 이것저것 나옴)

    warnings: list[str] = []
    by_name = {m["name"]: m for m in media}
    index = {int(c.get("no") or 0): i for i, c in enumerate(comments)}
    used_media = 0
    segs: list[dict[str, Any]] = []
    for seg in ans.segments:
        s: dict[str, Any] = {
            "kind": seg.kind, "subtitle": seg.subtitle.strip(),
            "tts": _tts_for(preset, bool(seg.tts)),
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
            wanted = [n for n in [*seg.comments, *(n for sc in seg.screens for n in sc)]]
            if preset is not None and not preset.comments.enabled:
                continue
            if preset is not None and preset.comments.skip == "time_order":
                # 시간순으로 건너뛴 댓글을 화면 단위로 (LLM 이 정한 묶음, 없으면 높이·개수로 채움)
                lay = preset.layout
                pages = pack_screens([sc for sc in seg.screens], wanted, comments,
                                     page_height(lay.stage_top, lay.stage_bottom),
                                     preset.comments.max_screens, preset.comments.max_total,
                                     preset.comments.per_screen)
                picked = sorted({n for pg in pages for n in pg})
            else:
                picked = []
                for n in wanted:
                    if 0 <= n < len(comments) and n not in picked:
                        c = comments[n]
                        parent = index.get(int(c.get("parent") or 0)) if int(c.get("depth") or 0) else None
                        if parent is not None and parent not in picked:
                            picked.append(parent)  # 답글은 부모와 같이 (맥락)
                        picked.append(n)
                picked = sorted(set(picked))[:max_comments]
                pages = None
            if not picked:
                continue
            for n in picked:
                if _HOSTILE.search(str(comments[n].get("text") or "")):
                    warnings.append(f"댓글 [{n}] 에 욕설·혐오 표현이 있습니다 (검수에서 확인)")
            if pages is None:
                pages = comment_windows(picked, comments)
            s["pages"] = pages
            s["picked"] = picked
            s["comments"] = [n for pg in s["pages"] for n in pg]
            s["read"] = [n for n in seg.read if n in picked
                         and str(comments[n].get("text") or "").strip()][:2]
            s["lines"] = []
        if seg.kind == "body" and not s["lines"]:
            continue
        if seg.kind == "reaction":
            image, warn = fix_reaction(seg.image, seg.say, source, memes or [],
                                       media_names=[m["name"] for m in media], main=main)
            warnings += warn
            # 원본 짤 우선: 글 주인공 영웅이 뚜렷하고 원문 사진이 있는 글이면 원문 사진만 (돌려 가며)
            photos = [m["name"] for m in media if m.get("kind") == "image"]
            if photos and image and not image.startswith("원문:") and main:
                image = f"원문:{photos[n_reaction % len(photos)]}"
            if image.startswith("원문:"):
                n_reaction += 1
            s["image"], s["say"], s["say_style"] = image, seg.say.strip()[:40], seg.say_style
            s["lines"] = [s["say"]] if s["say"] else []
            if not image and not s["say"]:
                continue
        _annotate(s, preset, comments)
        segs.append(s)
    kinds = [s["kind"] for s in segs]
    if not kinds or kinds[0] != "title":
        first = {"kind": "title", "subtitle": "", "tts": False, "tts_reason": "제목만 보여줌",
                 "lines": [], "media": [], "comments": [], "read": [], "quote": ""}
        _annotate(first, preset, comments)
        segs.insert(0, first)
    if composition == "A" and "comments" not in kinds:
        warnings.append("구성 A 로 정했지만 고를 만한 댓글이 없어 B 로 만들었습니다")
    return segs, warnings


def _annotate(s: dict[str, Any], preset: Preset | None, comments: list[dict[str, Any]]) -> None:
    """프리셋의 시간 규칙을 세그먼트에 적어 둔다 (display_seconds·spec 이 읽는다). 프리셋이 없으면 옛 값."""
    if preset is None:
        return
    sec = preset.sections
    if s["kind"] == "title":
        s["hold"] = round((sec.hook_sec[0] + sec.hook_sec[1]) / 2, 2)
    elif s["kind"] == "media":
        s["image_sec"] = round(sec.media_sec[0], 2)
        s["clip_max"] = sec.max_clip_sec
    elif s["kind"] == "comments":
        secs = []
        for pg in s.get("pages") or []:
            rows = [comments[n] for n in pg if 0 <= n < len(comments)]
            chars = sum(len(str(c.get("text") or "")) for c in rows)
            secs.append(page_seconds(preset.comments, chars, sum(1 for c in rows if c.get("dccon"))))
        s["page_secs"] = secs


def _tts_for(preset: Preset | None, asked: bool) -> bool:
    """프리셋이 TTS 를 껐거나 전부 켰으면 LLM 의 세그먼트별 판단보다 우선한다."""
    if preset is None or preset.tts == "auto":
        return asked
    return preset.tts == "on"


def display_seconds(seg: dict[str, Any], comments: list[dict[str, Any]],
                    media: list[dict[str, Any]]) -> float:
    """나레이션 없이도 이 세그먼트가 화면에 있어야 할 최소 시간."""
    kind = seg.get("kind")
    if kind == "title":
        return float(seg.get("hold") or 2.0)
    if kind == "media":
        by_name = {m["name"]: m for m in media}
        total = 0.0
        for pick in seg.get("media") or []:
            m = by_name.get(pick.get("name"), {})
            if m.get("kind") == "video":
                cap = float(seg.get("clip_max") or MAX_CLIP_SEC)
                total += min(cap, float(m.get("seconds") or cap))
            else:
                total += float(seg.get("image_sec") or 2.5)
        return min(total, 60.0 if seg.get("clip_max") else 24.0)
    if kind == "body":
        return min(8.0, sum(_est_read(t) for t in seg.get("lines") or []) or 3.0)
    if kind == "reaction":
        # 짤을 보고 자막을 읽고 웃을 시간 (2026-11-15 사용자: 유지 시간이 짧음)
        n = len(re.sub(r"\s", "", str(seg.get("say") or "")))
        return max(2.8, min(5.0, n / 10 + 2.0))
    if kind == "comments":
        pages = seg.get("pages")
        if seg.get("page_secs"):  # 프리셋: 장당 최소 2.8초 + 읽는 시간
            return min(90.0, float(sum(seg["page_secs"])))
        if pages:  # 스크린샷 한 장마다 고른 댓글을 읽을 시간 (3.5~6초)
            picked = set(seg.get("picked") or [])
            return min(24.0, sum(min(6.0, 3.5 + 0.7 * len(picked & set(pg))) for pg in pages))
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
