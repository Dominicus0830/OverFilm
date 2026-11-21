"""스타일(편집 유형) 복사와 학습: 자료를 모아 Claude Code 에게 주고, 기존 유형을 보충하거나 새 유형을 만든다.

학습 자료는 세 가지이고 하나만 있어도 된다.

- **Premiere Pro 파일** (`.prproj` = gzip XML, Premiere 에서 내보낸 Final Cut XML `.xml`): 시퀀스 크기·길이,
  컷 수와 컷 길이, 트랙 수, 글자(그래픽)·마커 수, 쓴 소리 파일 이름을 읽어 사실 목록으로 만든다.
  편집기(OverFilm)의 'Premiere 스타일 불러오기'는 자막 *모양*(글꼴·색)을 읽는 Rust 코드라 여기서는 쓰지 않는다:
  쇼츠 유형에는 글꼴·색 칸이 없고, 필요한 것은 컷 템포·구성이다.
- **통과된 쇼츠**: 검수에서 통과(approved·rendered)한 작업의 대본 구성과 장면 길이.
- **유튜브 링크**: Claude Code 가 웹 도구(WebSearch·WebFetch)로 직접 특징을 모은다. 영상을 재생해 볼 수는 없어서
  제목·설명·썸네일·자막 같은 페이지 정보로 짐작하고, 못 본 것은 못 봤다고 말한다.

무엇을 넣고 뺄지는 고정 규칙 없이 Claude 가 판단한다. 결과는 저장하지 않고 미리 보기로 돌려주며, 사용자가 대화로
더 고치거나 저장 버튼을 눌러 `presets_custom.save_custom_preset` 으로 저장한다 (커스텀 유형과 같은 흐름).
파일·DB 에서 읽은 값은 모두 믿지 않는다: 크기·개수에 상한을 두고, 읽지 못한 칸은 건너뛴다.
"""

from __future__ import annotations

import gzip
import json
import re
import statistics
import zlib
from pathlib import Path
from typing import Any

from pydantic import BaseModel, Field, ValidationError

from .llm import UNTRUSTED_RULE, Router, Task, untrusted
from .presets import Preset, load_presets, parse_preset
from .presets_custom import save_custom_preset, summarize

MAX_FILE = 64 << 20        # Premiere 파일 최대 크기
MAX_INFLATE = 256 << 20    # 압축 푼 XML 최대 크기
MAX_JOBS = 10
MAX_LINKS = 5
MAX_ITEMS = 20_000         # 클립 최대 개수 (그 뒤는 세지 않는다)
TICKS = 254_016_000_000    # Premiere 시간 단위 (1초)
PASSED = ("approved", "rendered")
_YT = re.compile(r"^https?://(www\.|m\.)?(youtube\.com/(shorts/|watch\?v=)|youtu\.be/)[\w\-?=&%.]{3,200}$", re.I)


# --- 복사 -------------------------------------------------------------------

def _free_id(base: str, taken: set[str], tag: str = "copy") -> str:
    base = re.sub(r"[^a-z0-9_-]", "", base.lower())[:24] or "style"
    if not base[0].isalnum():
        base = "s" + base[:23]
    for n in range(1, 100):
        cand = f"{base}-{tag}" if n == 1 else f"{base}-{tag}{n}"
        if cand not in taken:
            return cand
    raise ValueError("복사본이 너무 많습니다")


def copy_preset(style_dir: str | Path, src_id: str, label: str | None = None) -> Preset:
    """유형 하나를 새 이름의 내 유형으로 복제해 저장한다 (기본 유형도 복사할 수 있다)."""
    table = load_presets(style_dir)
    src = table.get(str(src_id or ""))
    if src is None:
        raise ValueError("복사할 유형을 찾지 못했습니다")
    data = json.loads(src.model_dump_json(exclude={"origin"}))
    data["id"] = _free_id(src.id, set(table))
    name = (label or "").strip() or f"{src.label} 복사"
    data["label"] = name[:20]
    return save_custom_preset(style_dir, data)


# --- Premiere 파일 ------------------------------------------------------------

def _inflate(data: bytes) -> bytes:
    if data[:2] != b"\x1f\x8b":
        return data
    d = zlib.decompressobj(16 + zlib.MAX_WBITS)
    out = d.decompress(data, MAX_INFLATE)
    if d.unconsumed_tail:
        raise ValueError("파일이 너무 큽니다 (풀면 256MB 넘음)")
    return out


def _num(s: str | None) -> int | None:
    try:
        return int(str(s).strip())
    except (TypeError, ValueError):
        return None


def _stats(durs: list[float]) -> dict[str, Any]:
    durs = [d for d in durs if 0 < d < 3600]
    if not durs:
        return {"count": 0}
    durs.sort()
    return {"count": len(durs), "median_sec": round(statistics.median(durs), 2),
            "mean_sec": round(sum(durs) / len(durs), 2), "shortest_sec": round(durs[0], 2),
            "longest_sec": round(durs[-1], 2),
            "under_1s": sum(d < 1 for d in durs), "over_5s": sum(d > 5 for d in durs)}


def _prproj(xml: str) -> dict[str, Any]:
    def clips(tag: str) -> list[float]:
        out = []
        for m in re.finditer(rf"<{tag}\b[^>]*>(.*?)</{tag}>", xml, re.S):
            if len(out) >= MAX_ITEMS:
                break
            body = m.group(1)
            s = _num((re.search(r"<Start>(-?\d+)</Start>", body) or [None, "0"])[1])
            e = _num((re.search(r"<End>(-?\d+)</End>", body) or [None, None])[1])
            if s is not None and e is not None and e > s:
                out.append((e - s) / TICKS)
        return out

    rect = re.search(r"<FrameRect>\s*0,0,(\d+),(\d+)\s*</FrameRect>", xml)
    seqs = re.findall(r"<Sequence\b[^>]*>.*?<Name>([^<]{1,80})</Name>", xml, re.S)[:10]
    return {
        "format": "prproj",
        "sequences": list(dict.fromkeys(seqs)),
        "frame_size": f"{rect[1]}x{rect[2]}" if rect else "",
        "video_clips": _stats(clips("VideoClipTrackItem")),
        "audio_clips": _stats(clips("AudioClipTrackItem")),
        "video_tracks": len(re.findall(r"<VideoClipTrack\b", xml)),
        "audio_tracks": len(re.findall(r"<AudioClipTrack\b", xml)),
        "text_graphics": xml.count("Source Text") + xml.count("SourceText"),
        "captions": len(re.findall(r"<CaptionTrackItem\b|<CaptionDataClipTrackItem\b", xml)),
        "markers": len(re.findall(r"<Marker\b|<DVAMarker\b", xml)),
        "effects": _top(re.findall(r"<MatchName>([^<]{1,80})</MatchName>", xml)),
    }


def _fcpxml(xml: str) -> dict[str, Any]:
    tb = _num((re.search(r"<timebase>(\d+)</timebase>", xml) or [None, None])[1]) or 30
    video, audio = [], []
    for kind, out in (("video", video), ("audio", audio)):
        block = re.search(rf"<{kind}>(.*)</{kind}>", xml, re.S)
        for m in re.finditer(r"<clipitem\b[^>]*>(.*?)</clipitem>", block[1] if block else "", re.S):
            if len(out) >= MAX_ITEMS:
                break
            s = _num((re.search(r"<start>(-?\d+)</start>", m[1]) or [None, None])[1])
            e = _num((re.search(r"<end>(-?\d+)</end>", m[1]) or [None, None])[1])
            if s is not None and e is not None and e > s >= 0:
                out.append((e - s) / max(1, tb))
    w = re.search(r"<width>(\d+)</width>", xml)
    h = re.search(r"<height>(\d+)</height>", xml)
    return {
        "format": "fcp-xml",
        "sequences": re.findall(r"<sequence\b[^>]*>.*?<name>([^<]{1,80})</name>", xml, re.S)[:10],
        "frame_size": f"{w[1]}x{h[1]}" if w and h else "",
        "fps": tb,
        "video_clips": _stats(video), "audio_clips": _stats(audio),
        "video_tracks": len(re.findall(r"<track\b", (re.search(r"<video>(.*)</video>", xml, re.S) or [""])[0])),
        "text_graphics": len(re.findall(r"<effectid>(?:Text|Outline Text|GraphicAndType)[^<]*</effectid>", xml)),
        "markers": len(re.findall(r"<marker>", xml)),
        "effects": _top(re.findall(r"<effectid>([^<]{1,80})</effectid>", xml)),
    }


def _top(names: list[str], n: int = 15) -> list[str]:
    counts: dict[str, int] = {}
    for x in names[:MAX_ITEMS]:
        counts[x] = counts.get(x, 0) + 1
    return [f"{k} ×{v}" for k, v in sorted(counts.items(), key=lambda kv: -kv[1])[:n]]


def premiere_facts(data: bytes, name: str) -> dict[str, Any]:
    """Premiere 프로젝트(.prproj)나 Final Cut XML 에서 편집 사실(컷 템포·구성)을 읽는다. 못 읽으면 ValueError."""
    if len(data) > MAX_FILE:
        raise ValueError("Premiere 파일이 너무 큽니다 (64MB 까지)")
    try:
        raw = _inflate(data)
    except (zlib.error, gzip.BadGzipFile, EOFError) as err:
        raise ValueError(f"Premiere 파일을 풀지 못했습니다: {err}") from err
    xml = raw.decode("utf-8", errors="replace")
    if "<PremiereData" in xml[:4000] or ("<Project" in xml[:4000] and "ClipTrackItem" in xml):
        facts = _prproj(xml)
    elif "<xmeml" in xml[:4000]:
        facts = _fcpxml(xml)
    else:
        raise ValueError("Premiere 프로젝트(.prproj)나 Premiere 에서 내보낸 XML 이 아닙니다")
    sounds = re.findall(r"[^<>\\/\"]{1,80}\.(?:mp3|wav|m4a|aac|ogg|flac)\b", xml, re.I)
    facts["sound_files"] = list(dict.fromkeys(sounds))[:20]
    facts["file"] = Path(str(name or "").replace("\\", "/")).name[:120]
    return facts


# --- 통과된 쇼츠 ---------------------------------------------------------------

def passed_shorts(store: Any, limit: int = 100) -> list[dict[str, Any]]:
    """검수를 통과한 쇼츠 목록 (작은 창에서 고르는 용)."""
    rows = store.conn.execute(
        "SELECT j.id, j.status, j.updated_at, p.title FROM jobs j JOIN posts p ON p.id=j.post_id "
        "WHERE j.status IN ('approved','rendered') ORDER BY j.id DESC LIMIT ?",
        (max(1, min(int(limit), 500)),)).fetchall()
    return [{"id": r["id"], "status": r["status"], "updated_at": r["updated_at"], "title": r["title"]}
            for r in rows]


def _load_json(raw: Any) -> Any:
    try:
        return json.loads(raw or "null")
    except (TypeError, ValueError):
        return None


def short_facts(store: Any, job_ids: list[int]) -> list[dict[str, Any]]:
    """고른 쇼츠마다 대본 구성(세그먼트 종류·읽기·줄 수)과 장면 길이. 통과 안 된 작업은 건너뛴다."""
    out = []
    for jid in list(dict.fromkeys(int(j) for j in job_ids))[:MAX_JOBS]:
        row = store.conn.execute(
            "SELECT j.id, j.status, j.spec_path, p.title, s.lines FROM jobs j JOIN posts p ON p.id=j.post_id "
            "LEFT JOIN scripts s ON s.id=j.script_id WHERE j.id=?", (jid,)).fetchone()
        if row is None or row["status"] not in PASSED:
            continue
        script = _load_json(row["lines"])
        script = script if isinstance(script, dict) else {}
        segs = []
        for seg in (script.get("segments") or [])[:12]:
            if not isinstance(seg, dict):
                continue
            lines = [str(x) for x in (seg.get("lines") or []) if isinstance(x, str)][:12]
            segs.append({"kind": seg.get("kind"), "tts": bool(seg.get("tts")), "lines": len(lines),
                         "chars": sum(len(x) for x in lines), "subtitle": str(seg.get("subtitle") or "")[:20],
                         "media": len(seg.get("media") or []), "comments": len(seg.get("comments") or []),
                         "screens": len(seg.get("screens") or []), "example_line": lines[0][:60] if lines else ""})
        spec: Any = None
        if row["spec_path"]:
            try:
                p = Path(str(row["spec_path"]))
                if p.is_file() and p.stat().st_size < 8 << 20:
                    spec = _load_json(p.read_text(encoding="utf-8"))
            except OSError:
                spec = None
        timing = []
        if isinstance(spec, dict):
            for sg in (spec.get("segments") or [])[:12]:
                if isinstance(sg, dict):
                    try:
                        timing.append({"kind": sg.get("kind"),
                                       "sec": round(float(sg.get("end", 0)) - float(sg.get("start", 0)), 2)})
                    except (TypeError, ValueError):
                        continue
        out.append({"job": row["id"], "title": str(row["title"] or "")[:80],
                    "youtube_title": str(script.get("title") or "")[:40],
                    "composition": script.get("composition"),
                    "duration_sec": spec.get("duration") if isinstance(spec, dict) else None,
                    "render_template": spec.get("preset") if isinstance(spec, dict) else None,
                    "segments": segs, "segment_seconds": timing})
    return out


# --- 유튜브 링크 -----------------------------------------------------------------

def clean_links(links: Any) -> list[str]:
    """유튜브 링크만, 최대 5개."""
    if not isinstance(links, list):
        return []
    out = []
    for x in links:
        t = str(x or "").strip()[:300]
        if _YT.match(t) and t not in out:
            out.append(t)
        if len(out) >= MAX_LINKS:
            break
    return out


# --- 학습 ----------------------------------------------------------------------

class LearnAnswer(BaseModel):
    reply: str = Field(max_length=2000, description="사용자에게: 무엇을 배웠고 무엇을 바꿨는지, 못 본 것은 무엇인지")
    findings: list[str] = Field(default_factory=list, max_length=15,
                                description="자료에서 찾은 특징 한 줄씩 (출처 표시: [Premiere] [쇼츠] [유튜브])")
    preset: Preset = Field(description="학습을 반영한 전체 프리셋")


_SYSTEM = (
    "너는 오버워치 커뮤 글을 세로 쇼츠로 만드는 도구의 '편집 유형(스타일)' 설계 도우미다. "
    "사용자가 준 학습 자료(Premiere 편집 기록에서 읽은 사실, 이미 검수를 통과한 쇼츠의 구성, 유튜브 영상 링크)에서 "
    "편집 스타일의 특징(컷 템포·장면 길이, 구성 순서, 댓글 비중, TTS 여부, 자막 모양, 효과)을 찾아 프리셋 JSON 으로 낸다. "
    "무엇을 넣고 뺄지는 네가 판단한다. 자료에 근거가 없는 칸은 바꾸지 않는다(보충일 때는 기존 값을 그대로 둔다). "
    "guide 에는 대본 작가에게 그대로 줄 편집 규칙 문장을 쓰되, 기존 guide 가 있으면 지우지 말고 이어 쓴다. "
    "사용자가 시키지 않은 내용 바꾸기(욕설 순화, 제목·댓글 고치기, 대본 검사)는 넣지 않는다 (기본은 원문 그대로). "
    "유튜브 링크는 웹 도구로 페이지·oEmbed·썸네일·설명·자막 정보를 찾아본다. 영상을 재생해 볼 수는 없으니 "
    "보지 못한 것은 짐작이라고 말하고, 페이지를 못 열었으면 그렇게 말한다. "
    "preset.id 는 영소문자·숫자·-·_ 1~32자, label 은 20자 이내 한국어 이름. 숫자는 범위 안에서 쓴다. "
    + UNTRUSTED_RULE.replace("<post>, <comments>", "<premiere>, <shorts>, <links>, <note>")
)


def learn_task(base: Preset | None, premiere: dict[str, Any] | None, shorts: list[dict[str, Any]],
               links: list[str], note: str) -> Task:
    parts = []
    if base is not None:
        parts.append("보충할 기존 유형 (자료에 근거가 있는 칸만 바꾸고 나머지는 그대로):\n"
                     + base.model_dump_json(exclude={"origin"}))
    else:
        parts.append("새 유형을 만든다. 자료에서 찾은 특징으로 전체 프리셋을 채운다.")
    if premiere:
        parts.append("Premiere 편집 기록에서 읽은 사실 (컷 길이는 초):\n"
                     + untrusted("premiere", json.dumps(premiere, ensure_ascii=False)))
    if shorts:
        parts.append("검수를 통과한 쇼츠 (이 도구로 만든 것, 사용자가 괜찮다고 한 것):\n"
                     + untrusted("shorts", json.dumps(shorts, ensure_ascii=False)))
    if links:
        parts.append("참고 유튜브 영상 (웹 도구로 직접 특징을 모아라):\n" + untrusted("links", "\n".join(links)))
    if note.strip():
        parts.append("사용자 메모:\n" + untrusted("note", note.strip()[:1000]))
    parts.append("위 자료로 reply·findings·preset 을 낸다.")
    return Task("preset", _SYSTEM, "\n\n".join(parts), "WebSearch,WebFetch" if links else "")


def learn(router: Router, store: Any, style_dir: str | Path, base_id: str | None = None,
          premiere: dict[str, Any] | None = None, job_ids: list[int] | None = None,
          links: list[Any] | None = None, note: str = "") -> dict[str, Any]:
    """학습 자료를 모아 Claude 에게 묻는다. 저장하지 않고 reply·findings·preset·summary 를 돌려준다.

    보충(base_id)이면 id 를 기존 것으로 고정한다: 내 유형은 저장하면 덮어쓰고, 기본 유형은 저장할 때 새 id 가 붙는다.
    새 유형이면 이미 있는 id 와 겹치지 않게 바꾼다 (모르게 덮어쓰지 않도록).
    """
    table = load_presets(style_dir)
    base = table.get(str(base_id)) if base_id else None
    if base_id and base is None:
        raise ValueError("보충할 유형을 찾지 못했습니다")
    shorts = short_facts(store, job_ids or []) if job_ids else []
    yt = clean_links(links)
    if not premiere and not shorts and not yt:
        raise ValueError("학습할 자료가 없습니다: Premiere 파일, 통과된 쇼츠, 유튜브 링크 중 하나 이상을 넣어 주세요")
    ans, _ = router.ask(learn_task(base, premiere, shorts, yt, note), LearnAnswer)
    data = json.loads(ans.preset.model_dump_json(exclude={"origin"}))
    if base is not None:
        data["id"] = base.id
    elif data.get("id") in table:
        data["id"] = _free_id(str(data.get("id")), set(table), "new")
    if yt:
        data.setdefault("reference", {})
        vids = [v for v in (data["reference"].get("videos") or []) if isinstance(v, str)]
        data["reference"]["videos"] = list(dict.fromkeys(vids + yt))[:10]
    try:
        preset = parse_preset(data, "custom")
    except ValidationError as err:
        raise ValueError(f"학습 결과가 형식에 맞지 않습니다: {str(err)[:200]}") from err
    return {"reply": ans.reply, "findings": [f[:200] for f in ans.findings], "preset": preset.model_dump(),
            "summary": summarize(preset), "base": base.id if base else None,
            "used": {"premiere": bool(premiere), "shorts": [s["job"] for s in shorts], "links": yt}}
