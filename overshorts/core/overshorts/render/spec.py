"""render_spec.json 만들기: Python 과 Remotion 이 대화하는 유일한 파일 (기획서 4).

aligned 작업 → data/jobs/<id>/public/ 에 voice.wav·배경 영상을 두고 spec.json 을 쓴다.
검수 화면의 @remotion/player 미리보기와 최종 렌더가 같은 spec 을 쓴다.
"""

from __future__ import annotations

import json
import random
import re
import shutil
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any

from ..db import Store
from ..script.checks import norm
from ..script.segments import media_items, strip_marks

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


# --- 프레임형 (기획서 3.5 개정 5·6) -------------------------------------------

MAX_CAPTION_WORDS = 4
MAX_CAPTION_CHARS = 13  # 74px 글씨로 한 줄
_END_PUNCT = re.compile(r"[.?!,…~]$")
SILENCE_LIMIT = 5.0
AUDIO_EXT = {".mp3", ".wav", ".m4a", ".ogg", ".aac"}


@dataclass
class SpecOptions:
    preset: str = "auto"  # auto | frame | card
    nickname: str = "original"  # original | blur | replace
    bgm_dir: Path | None = None
    body: str = ""
    show_source: bool = True  # 아래 '출처: …' 표시 (검수 화면에서 켜기/끄기)


def caption_chunks(scenes: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """단어 시각 → 한 번에 2~4단어 자막 묶음. 문장 부호에서 끊고, 한 단어만 남지 않게 한다."""
    out: list[dict[str, Any]] = []
    for sc in scenes:
        words = [w for w in sc.get("words") or [] if str(w.get("text", "")).strip()]
        groups: list[list[dict[str, Any]]] = []
        cur: list[dict[str, Any]] = []
        for w in words:
            # 한 줄에 들어가게: 글자가 많으면 4단어 전에 끊는다
            if len(cur) >= 2 and sum(len(str(x["text"])) for x in cur) + len(str(w["text"])) > MAX_CAPTION_CHARS:
                groups.append(cur)
                cur = []
            cur.append(w)
            if len(cur) >= MAX_CAPTION_WORDS or (_END_PUNCT.search(str(w["text"])) and len(cur) >= 2):
                groups.append(cur)
                cur = []
        if cur:
            if (len(cur) == 1 and groups and len(groups[-1]) < MAX_CAPTION_WORDS
                    and sum(len(str(x["text"])) for x in groups[-1] + cur) <= MAX_CAPTION_CHARS + 3):
                groups[-1].append(cur[0])
            else:
                groups.append(cur)
        for k, g in enumerate(groups):
            nxt = groups[k + 1][0]["start"] if k + 1 < len(groups) else g[-1]["end"] + 0.15
            out.append({"start": g[0]["start"], "end": round(max(nxt, g[-1]["end"]), 3),
                        "seg": sc.get("seg", 0), "words": g})
    return out


def _ext(path: Path) -> str:
    try:
        head = path.read_bytes()[:12]
    except OSError:
        return ".jpg"
    if head[:8] == b"\x89PNG\r\n\x1a\n":
        return ".png"
    if head[:4] == b"GIF8":
        return ".gif"
    if head[:4] == b"RIFF" and head[8:12] == b"WEBP":
        return ".webp"
    if head[4:8] == b"ftyp":
        return ".mp4"
    return ".jpg"


def _copy(src: str | Path, public: Path, name: str) -> str | None:
    p = Path(src)
    if not p.is_file():
        return None
    out = name + _ext(p)
    try:
        shutil.copyfile(p, public / out)
    except OSError:
        return None
    return out


def _silences(spans: list[tuple[float, float]], duration: float) -> float:
    """가장 긴 무음 (말소리·원본 소리 없는 구간) 길이."""
    longest, t = 0.0, 0.0
    for a, b in sorted(spans):
        longest = max(longest, a - t)
        t = max(t, b)
    return max(longest, duration - t)


SAY_STYLES = ("calm", "shout", "question", "sad", "smug")


def silent_captions(texts: list[str], start: float, end: float, seg: int) -> list[dict[str, Any]]:
    """TTS 를 끈 장면의 자막: 글자 수에 비례해 시간을 나눠 같은 2~4단어 묶음으로 보여 준다."""
    words = [w for t in texts for w in t.split()]
    total = sum(len(w) + 1 for w in words)
    if not words or total <= 0 or end <= start:
        return []
    k = (end - start - 0.2) / total
    t = start + 0.1
    timed = []
    for w in words:
        d = (len(w) + 1) * k
        timed.append({"text": w, "start": round(t, 3), "end": round(t + d, 3)})
        t += d
    out = caption_chunks([{"words": timed, "seg": seg}])
    if out:
        out[-1]["end"] = round(end, 3)
    return out


def build_frame_spec(timing: dict[str, Any], script: dict[str, Any], post: dict[str, Any],
                     public: Path, source: str, background: str | None, opts: SpecOptions,
                     media: list[dict[str, Any]], comments: list[dict[str, Any]]) -> dict[str, Any]:
    """세그먼트 대본 + 음성 시각 → render_spec v2 (프레임형·썰 카드형 공용)."""
    base = build_spec(timing, str(script.get("title") or ""), str(post.get("subject") or ""),
                      str(script.get("fact") or ""), source, background)
    lines = script.get("lines") or []
    segs_in = script.get("segments") or []
    spans = {int(s.get("seg", 0)): (float(s["start"]), float(s["end"]))
             for s in timing.get("segments") or []}
    for sc in base["scenes"]:
        ln = sc.get("line", 0)
        sc["seg"] = int(lines[ln].get("seg") or 0) if 0 <= ln < len(lines) else 0
        sc["comment_no"] = lines[ln].get("comment_no") if 0 <= ln < len(lines) else None
    by_name = {m["name"]: m for m in media}
    copied: dict[str, str | None] = {}
    sound: list[tuple[float, float]] = [(sc["start"], sc["end"]) for sc in base["scenes"]]
    segments: list[dict[str, Any]] = []
    warnings: list[str] = []
    silent: list[dict[str, Any]] = []
    for i, seg in enumerate(segs_in):
        start, end = spans.get(i, (0.0, 0.0))
        out: dict[str, Any] = {"kind": seg.get("kind", "body"), "start": start, "end": end,
                               "subtitle": str(seg.get("subtitle") or ""), "tts": bool(seg.get("tts")),
                               "media": [], "comments": [], "quote": ""}
        seg_scenes = [sc for sc in base["scenes"] if sc["seg"] == i and sc["part"] != "comment"]
        if out["kind"] == "media":
            picks = seg.get("media") or []
            # 대본 줄(세그먼트 안 번호) → 그 줄을 읽기 시작하는 시각
            narr_lines = sorted({sc["line"] for sc in seg_scenes})
            line_start = {k: min(sc["start"] for sc in seg_scenes if sc["line"] == ln)
                          for k, ln in enumerate(narr_lines)}
            starts = []
            for k, pick in enumerate(picks):
                even = start + (end - start) * k / max(1, len(picks))
                t = line_start.get(int(pick.get("line") or 0), even) if seg.get("tts") else even
                starts.append(max(t, starts[-1] + 0.8 if starts else start))
            if starts:
                starts[0] = start
            for k, pick in enumerate(picks):
                m = by_name.get(pick.get("name"))
                if m is None:
                    continue
                if m["name"] not in copied:
                    copied[m["name"]] = _copy(m["path"], public, f"m{len(copied)}")
                srcf = copied[m["name"]]
                if srcf is None:
                    continue
                m_end = starts[k + 1] if k + 1 < len(starts) else end
                number = int(m["name"].split()[-1]) if m["name"].split()[-1].isdigit() else 0
                item = {"src": srcf, "kind": m["kind"], "display": pick.get("display", "fill"),
                        "number": number,
                        "caption": media_caption(opts.body, number) if m["kind"] == "image" else "",
                        "start": round(starts[k], 3), "end": round(max(m_end, starts[k] + 0.5), 3),
                        "width": m.get("width") or 0, "height": m.get("height") or 0,
                        "muted": True}
                if m["kind"] == "video":
                    # 원본 소리가 중요하면(나레이션을 끈 세그먼트) 원본을 튼다
                    item["muted"] = bool(seg.get("tts")) or not m.get("audio")
                    item["end"] = round(min(item["end"], item["start"] + 8.0), 3)
                    if not item["muted"]:
                        sound.append((item["start"], item["end"]))
                out["media"].append(item)
        elif out["kind"] == "comments":
            reads = {sc.get("comment_no"): (sc["start"], sc["end"]) for sc in base["scenes"]
                     if sc["seg"] == i and sc["part"] == "comment"}
            idx = [n for n in seg.get("comments") or [] if 0 <= n < len(comments)]
            for n in idx:
                c = comments[n]
                con = c.get("dccon") or None
                con_out = None
                if con:
                    local = con.get("local")
                    key = f"con:{local}"
                    if local and key not in copied:
                        copied[key] = _copy(local, public, f"c{len(copied)}")
                    con_out = {"src": copied.get(key) if local else None,
                               "name": str(con.get("name") or "")}
                nick, ip = str(c.get("nick") or "ㅇㅇ"), str(c.get("ip") or "")
                if opts.nickname == "replace":
                    nick, ip = "ㅇㅇ", ""
                out["comments"].append({
                    "no": n, "nick": nick, "ip": ip, "icon": str(c.get("icon") or ""),
                    "text": str(c.get("text") or ""), "depth": int(c.get("depth") or 0),
                    "dccon": con_out, "appear": start, "page": 0,
                    "read": list(reads[n]) if n in reads else None,
                })
            _paginate(out["comments"], start, end, seg.get("pages"))
        elif out["kind"] == "body":
            out["quote"] = _quote(str(seg.get("quote") or ""), opts.body, i, segs_in)
        elif out["kind"] == "reaction":
            path = str(seg.get("image_path") or "")
            out["image"] = _copy(path, public, f"r{i}") if path else None
            out["say"] = str(seg.get("say") or "")
            style = str(seg.get("say_style") or "calm")
            out["say_style"] = style if style in SAY_STYLES else "calm"
        if out["kind"] == "reaction" and not seg.get("tts") and out["say"].strip():
            # 짤 장면 대사는 소리 없이 자막으로 (다른 장면의 나레이션 줄은 TTS 를 켤 때만 자막이 된다)
            # 대사는 한 덩어리로, 짤과 같은 순간에 떴다가 같은 순간에 사라진다 (스타일 자막)
            silent.append({"start": round(start, 3), "end": round(end, 3), "seg": i,
                           "style": out["say_style"],
                           "words": [{"text": out["say"], "start": round(start, 3), "end": round(end, 3)}]})
        segments.append(out)
    has_media = any(sg["media"] or sg.get("image") for sg in segments)
    preset = opts.preset if opts.preset in ("frame", "card") else ("frame" if has_media else "card")
    bgm = None
    if opts.bgm_dir is not None and opts.bgm_dir.is_dir():
        try:
            tracks = sorted(p for p in opts.bgm_dir.iterdir() if p.suffix.lower() in AUDIO_EXT)
        except OSError:
            tracks = []
        if tracks:
            pick = random.Random(str(script.get("title"))).choice(tracks)
            bgm = "bgm" + pick.suffix.lower()
            shutil.copyfile(pick, public / bgm)
    silence = _silences(sound, base["duration"])
    if bgm is None and silence > SILENCE_LIMIT:
        warnings.append(f"무음이 {silence:.1f}초 이어집니다 (BGM 폴더를 정하면 깔립니다)")
    author = post.get("author") or {}
    if opts.nickname == "replace":
        author = {"nick": "ㅇㅇ", "ip": "", "icon": ""}
    base.update(post={"title": str(post.get("title") or ""), "subject": str(post.get("subject") or ""),
                      "nick": str(author.get("nick") or "ㅇㅇ"), "ip": str(author.get("ip") or ""),
                      "icon": str(author.get("icon") or ""), "date": _dc_date(post.get("posted_at"))})
    base.update(version=2, preset=preset, post_title=str(post.get("title") or ""), segments=segments, 
                captions=sorted(caption_chunks([sc for sc in base["scenes"] if sc["part"] != "comment"])
                                + silent, key=lambda c: c["start"]),
                bgm=bgm, nickname=opts.nickname if opts.nickname in ("original", "blur", "replace")
                else "original", warnings=warnings)
    return base


# 디시 스킨 크기·댓글 높이 어림은 대본 단계와 같은 것을 쓴다 (장 나누기를 대본에서 정함)
from ..script.segments import PAGE_H, SKIN_SCALE, SKIN_W, comment_height  # noqa: E402,F401


def _paginate(rows: list[dict[str, Any]], start: float, end: float,
              given: list[list[int]] | None = None) -> None:
    """댓글을 스크린샷 한 장씩 나눈다. 대본 단계에서 장을 정했으면(이어지는 댓글 묶음) 그대로 쓰고,
    아니면 원댓글과 그 답글은 되도록 같은 장에, 짧으면 여러 묶음을 한 장에.
    장마다 보여 줄 시간은 글 양에 비례하고, TTS 가 읽는 댓글이 있는 장은 읽기 전에 나온다."""
    if given:
        by_no = {r["no"]: r for r in rows}
        pages = [[by_no[n] for n in pg if n in by_no] for pg in given]
        pages = [pg for pg in pages if pg]
        _time_pages(pages, start, end)
        return
    threads: list[list[dict[str, Any]]] = []
    for r in rows:
        if r["depth"] and threads:
            threads[-1].append(r)
        else:
            threads.append([r])
    pages: list[list[dict[str, Any]]] = []
    h = PAGE_H + 1
    for th in threads:
        th_h = sum(comment_height(r) for r in th) + (8 if len(th) > 1 else 0)
        if pages and h + th_h <= PAGE_H:
            pages[-1] += th
            h += th_h
        else:
            pages.append(list(th))
            h = th_h
    _time_pages(pages, start, end)


def _time_pages(pages: list[list[dict[str, Any]]], start: float, end: float) -> None:
    if not pages:
        return
    weight = [sum(comment_height(r) for r in pg) for pg in pages]
    total = sum(weight) or 1.0
    t = start
    starts = []
    for w in weight:
        starts.append(t)
        t += (end - start) * w / total
    for k, pg in enumerate(pages):
        reads = [r["read"][0] for r in pg if r.get("read")]
        if reads and k > 0:
            starts[k] = min(starts[k], max(starts[k - 1] + 1.0, min(reads) - 0.3))
    for k, pg in enumerate(pages):
        for r in pg:
            r["page"] = k
            r["appear"] = round(starts[k], 3)


def _dc_date(iso: Any) -> str:
    try:
        return datetime.fromisoformat(str(iso)).strftime("%Y.%m.%d %H:%M:%S")
    except (TypeError, ValueError):
        return ""


def media_caption(body: str, number: int) -> str:
    """본문에서 [사진 N] 바로 아래 글 (다음 사진 전까지). 스크린샷처럼 사진 밑에 보인다."""
    out: list[str] = []
    on = False
    for ln in (body or "").splitlines():
        m = re.fullmatch(r"\s*\[(사진|영상) (\d+)\]\s*", ln)
        if m:
            if on:
                break
            on = m.group(1) == "사진" and int(m.group(2)) == number
            continue
        if on and ln.strip():
            out.append(ln.strip())
    return "\n".join(out)[:300]


def _quote(quote: str, body: str, i: int, segs: list[dict[str, Any]]) -> str:
    """본문 카드에 보일 원문 부분. AI 가 준 인용이 원문에 없으면 본문을 나눠 쓴다."""
    clean = strip_marks(body)
    q = quote.strip()
    if q and norm(q)[:20] in norm(clean):
        return q[:600]
    paras = [p for p in clean.splitlines() if p.strip()]
    bodies = [k for k, s in enumerate(segs) if s.get("kind") == "body"]
    if not paras or i not in bodies:
        return clean[:600]
    n = len(bodies)
    k = bodies.index(i)
    chunk = paras[k * len(paras) // n:(k + 1) * len(paras) // n] or paras[-1:]
    return "\n".join(chunk)[:600]


def spec_pending(store: Store, jobs_dir: Path, bg_dir: Path | None,
                 limit: int = 50, opts: SpecOptions | None = None) -> dict[str, int]:
    opts = opts or SpecOptions()
    rows = store.conn.execute(
        "SELECT j.id AS job_id, p.gall_id, p.gall_name, p.title, p.subject, p.body, p.images, p.comments, "
        "p.author, p.posted_at, "
        "s.lines, j.preset FROM jobs j JOIN posts p ON p.id = j.post_id JOIN scripts s ON s.id = j.script_id "
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
            source_text = f"출처: DC인사이드 {gall}"
            source = source_text if opts.show_source else ""
            # 검수에서 프리셋을 바꿨으면(재생성) 그 작업만 그 프리셋
            preset = row["preset"] if row["preset"] in ("frame", "card") else opts.preset
            if script.get("segments"):
                media = media_items(json.loads(row["images"] or "[]"))
                comments = [c for c in json.loads(row["comments"] or "[]") if isinstance(c, dict)]
                post = {"subject": row["subject"], "title": row["title"],
                        "author": json.loads(row["author"] or "{}"), "posted_at": row["posted_at"]}
                spec = build_frame_spec(timing, script, post, public, source, bg_name,
                                        SpecOptions(preset, opts.nickname, opts.bgm_dir,
                                                    str(row["body"] or ""), opts.show_source),
                                        media, comments)
            else:  # 세그먼트 이전 대본 (썰 카드형)
                spec = build_spec(timing, str(script.get("title") or ""), str(row["subject"] or ""),
                                  str(script.get("fact") or ""), source, bg_name)
            spec["source_text"] = source_text  # 출처 끄기/켜기를 다시 만들지 않고 바꿀 수 있게
            spec_path = job_dir / "spec.json"
            spec_path.write_text(json.dumps(spec, ensure_ascii=False), encoding="utf-8")
        except (OSError, json.JSONDecodeError, KeyError, TypeError, ValueError) as err:
            store.set_job_status(row["job_id"], "aligned", f"미리보기 준비: {err}")
            counts["failed"] += 1
            continue
        store.conn.execute("UPDATE jobs SET spec_path=? WHERE id=?", (str(spec_path), row["job_id"]))
        store.set_job_status(row["job_id"], "preview_ready", None)
        counts["ready"] += 1
    return counts
