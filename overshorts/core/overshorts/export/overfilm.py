"""OverFilm 편집기로 넘기기: 검수한 쇼츠를 편집기 타임라인에서 사람이 더 고칠 수 있게 (2026-11-15).

render_spec(검수 화면 미리보기와 같은 것)을 `.overshorts` 장면 파일로 바꾼다. 편집기
(`overfilm.shorts.import`, crates/engine/src/overfilm_shorts.rs)는 위치를 다시 계산하지 않고 이 파일의
픽셀 값대로 놓기만 한다. 그래서 화면 배치(제목 15~26%, 스크린샷 28~66%, 자막 67~72%, 출처 75.5%)는
여기와 render/src/FrameStory.tsx 두 곳만 맞추면 된다.

| 편집기 트랙 | 무엇 | 편집기에서 |
|---|---|---|
| V1 장면 | 디시 화면처럼 그린 글 머리·사진·본문·댓글창 (구간마다 PNG, 움직이면 그 구간만 H.264), 짤 원본 그림 | 그림 클립 (바꾸기·자르기·옮기기) |
| V2 자막 | 노란 2~4단어 자막, 짤 장면 대사 (색·크기·글꼴) | 글자 클립 (다시 쓰기·스타일) |
| V3 제목 | 위 고정 제목 | 글자 클립 |
| V4 출처 | 출처 표시 (끄면 없음) | 글자 클립 |
| A1 TTS / A2 BGM / A3 원본 소리 | 나레이션, BGM(말할 때 −12dB 덕킹은 볼륨 키프레임), 원문 동영상 소리 | 소리 클립 |

장면 그림은 Remotion 으로 `layers: 'stage'`(제목·자막·출처·소리 없이 투명 바탕)를 그린다
(render/scripts/stage-export.mjs). 렌더 폴더에서 `npm install` 이 먼저 돼 있어야 한다.
"""

from __future__ import annotations

import json
import logging
import math
import os
import platform
import re
import shutil
import subprocess
import tempfile
from collections.abc import Callable
from itertools import pairwise
from pathlib import Path
from typing import Any

from ..proc import decode_output

log = logging.getLogger(__name__)

FORMAT = "overshorts.scenes"
EXTENSION = ".overshorts"
RENDER_DIR = Path(__file__).resolve().parents[3] / "render"

# FrameStory.tsx 의 SAFE (화면 높이 %)
SAFE = {"title": (15, 26), "tall": (28, 66), "caption": (67, 72), "source": (75.5, 77.5)}


def _safe(spec: dict[str, Any]) -> dict[str, tuple[float, float]]:
    """spec.layout (편집 유형) 의 세로 위치. 없으면 옛 프레임형 SAFE."""
    ly = spec.get("layout")
    if not isinstance(ly, dict):
        return SAFE

    def pair(key: str, d: tuple[float, float]) -> tuple[float, float]:
        v = ly.get(key)
        try:
            return (float(v[0]), float(v[1]))
        except (TypeError, ValueError, IndexError, KeyError):
            return d

    top = _num(ly.get("source_top"), SAFE["source"][0])
    return {"title": pair("title", SAFE["title"]), "tall": pair("stage", SAFE["tall"]),
            "caption": pair("caption_box", SAFE["caption"]), "source": (top, top + 2)}
FONT = "Pretendard"
SHOUT = "Black Han Sans"
HAND = "Gaegu"
# FrameStory.tsx SAY_LOOK: (글꼴, 스타일, 크기 px, 색, CSS 테두리 px). CSS 테두리는 글자 안팎 반씩이라
# paint-order: stroke fill 로 보이는 바깥 테두리는 절반이다.
SAY_LOOK = {
    "calm": (FONT, "ExtraBold", 80, "#FFFFFF", 12),
    "question": (SHOUT, "Regular", 100, "#FFFFFF", 14),
    "shout": (SHOUT, "Regular", 124, "#FF3B30", 16),
    "sad": (HAND, "Bold", 100, "#9CC9FF", 12),
    "smug": (SHOUT, "Regular", 108, "#FFD43B", 14),
}
KIND_NAME = {"title": "글 머리", "media": "사진", "body": "본문", "comments": "댓글", "reaction": "짤"}
_COLOR = re.compile(r"^#[0-9a-fA-F]{3,8}$")


def _num(v: Any, default: float = 0.0) -> float:
    try:
        x = float(v)
    except (TypeError, ValueError):
        return default
    return x if math.isfinite(x) else default


def _pct(height: int, p: float) -> float:
    return round(height * p / 100, 1)


def safe_name(name: str) -> str:
    s = re.sub(r'[\\/:*?"<>|\x00-\x1f]', "_", name).strip().strip(".")
    return s[:60] or "OverShorts"


# --- 장면 그림 구간 ---

def stage_spans(spec: dict[str, Any]) -> list[dict[str, Any]]:
    """장면이 바뀌는 시각마다 끊은 구간 (짤 장면은 원본 그림을 따로 놓으니 뺀다).

    프레임형: 세그먼트는 다음 세그먼트가 시작할 때까지 보인다 (FrameStory 와 같음). 그 안에서 사진이
    바뀌는 시각, 댓글이 나타나는 시각·읽는 댓글 강조가 켜지고 꺼지는 시각에 또 끊는다.
    썰 카드형: 글 카드가 단어마다 바뀌므로 통째로 한 구간 (영상이 된다).
    """
    duration = _num(spec.get("duration"), 0.0)
    if duration <= 0:
        return []
    if spec.get("preset") != "frame":
        return [{"id": 1, "start": 0.0, "end": duration, "kind": "card"}]
    segs = [s for s in spec.get("segments") or [] if isinstance(s, dict)]
    out: list[dict[str, Any]] = []
    for i, seg in enumerate(segs):
        start = _num(seg.get("start"))
        end = _num(segs[i + 1].get("start"), duration) if i + 1 < len(segs) else duration
        end = min(end, duration)
        if end - start < 0.02 or seg.get("kind") == "reaction":
            continue
        cuts = {start, end}
        for m in seg.get("media") or []:
            cuts.add(_num(m.get("start"), start))
        for c in seg.get("comments") or []:
            cuts.add(_num(c.get("appear"), start))
            read = c.get("read")
            if isinstance(read, list) and len(read) == 2:
                cuts.update((_num(read[0], start), _num(read[1], start)))
        points = sorted(t for t in cuts if start <= t <= end)
        for a, b in pairwise(points):
            if b - a >= 0.02:
                out.append({"id": len(out) + 1, "start": round(a, 3), "end": round(b, 3),
                            "kind": str(seg.get("kind") or "body"), "seg": i})
    return out


# --- 장면 파일 ---

def _caption_text(c: dict[str, Any]) -> str:
    runs = c.get("runs")
    if isinstance(runs, list) and runs:
        return "".join(str(r.get("t") or "") for r in runs if isinstance(r, dict)).strip()
    return " ".join(str(w.get("text") or "") for w in c.get("words") or [] if isinstance(w, dict)).strip()


def _run_color(c: dict[str, Any]) -> str | None:
    """원문 꾸밈 대사: 글자가 가장 많은 조각의 색 (편집기 글자 클립은 한 색)."""
    best, n = None, 0
    for r in c.get("runs") or []:
        if not isinstance(r, dict):
            continue
        color = str(r.get("color") or "")
        k = len(str(r.get("t") or "").strip())
        if _COLOR.match(color) and k > n:
            best, n = color, k
    return best


def _frame_captions(spec: dict[str, Any], w: int, h: int) -> list[dict[str, Any]]:
    safe = _safe(spec)
    title_bar = (spec.get("layout") or {}).get("title_bar", True) is not False
    out = []
    for c in spec.get("captions") or []:
        if not isinstance(c, dict):
            continue
        text = _caption_text(c)
        if not text:
            continue
        start, end = _num(c.get("start")), _num(c.get("end"))
        style = c.get("style")
        if style in SAY_LOOK or c.get("runs"):
            font, fstyle, size, color, stroke = SAY_LOOK.get(style or "calm", SAY_LOOK["calm"])
            if c.get("runs"):
                font, fstyle, size, color, stroke = SAY_LOOK["calm"]
                color = _run_color(c) or color
            # 긴 줄은 두 줄 안에 들어가게 줄인다 (FrameStory 와 같은 식)
            chars = len(re.sub(r"\s", "", text))
            size = max(58, round(size * min(1.0, 12 / max(1, chars)) ** 0.75))
            if title_bar:  # 스크린샷 아래 (옛 프레임형)
                top = safe["tall"][1] + 0.5
                mid = top + (safe["source"][0] - safe["tall"][1] - 1) / 2
            else:  # 제목 바 없는 유형: 짤 아래쪽에 겹침
                mid = (safe["caption"][0] + safe["caption"][1]) / 2
            out.append({"track": "자막", "text": text, "start": start, "end": end, "size": size,
                        "color": color, "stroke": "#000000", "strokeWidth": stroke / 2, "font": font,
                        "fontStyle": fstyle, "center": [w / 2, _pct(h, mid)], "maxWidth": w - 60,
                        "maxLines": 2, "lineSpacing": 1.1})
        elif style == "band":  # 흰 띠 한 줄: 편집기 글자 클립은 박스가 없어 흰 글자 + 검은 테두리로 (띠는 편집기에서)
            mid = (safe["caption"][0] + safe["caption"][1]) / 2
            size = max(34, min(60, round(1000 / max(8, len(text)))))
            out.append({"track": "자막", "text": text, "start": start, "end": end, "size": size,
                        "color": "#FFFFFF", "stroke": "#000000", "strokeWidth": 5, "font": FONT,
                        "fontStyle": "ExtraBold", "center": [w / 2, _pct(h, mid)], "maxWidth": w - 80,
                        "maxLines": 1, "lineSpacing": 1.1})
        else:
            mid = (safe["caption"][0] + safe["caption"][1]) / 2
            out.append({"track": "자막", "text": text, "start": start, "end": end, "size": 74,
                        "color": "#FFD43B", "stroke": "#000000", "strokeWidth": 6, "font": FONT,
                        "fontStyle": "Black", "center": [w / 2, _pct(h, mid)], "maxWidth": w - 80,
                        "maxLines": 2, "lineSpacing": 1.1})
    return out


def _card_captions(spec: dict[str, Any], w: int, h: int) -> list[dict[str, Any]]:
    """썰 카드형: 읽는 단어 하나씩 크게 (CardStory 의 Captions)."""
    accent = str((spec.get("theme") or {}).get("accent") or "#FFD43B")
    out = []
    for sc in spec.get("scenes") or []:
        for wd in sc.get("words") or []:
            text = str(wd.get("text") or "").strip()
            if not text:
                continue
            start = _num(wd.get("start"))
            end = max(_num(wd.get("end")), start + 0.12)
            out.append({"track": "자막", "text": text, "start": start, "end": end, "size": 104,
                        "color": accent if wd.get("emphasis") and _COLOR.match(accent) else "#FFFFFF",
                        "stroke": "#000000", "strokeWidth": 5, "font": FONT, "fontStyle": "Black",
                        "center": [w / 2, round((1380 + 60) * h / 1920, 1)], "maxWidth": w - 40,
                        "maxLines": 1})
    return out


def _ducking(spec: dict[str, Any]) -> list[list[float]]:
    """BGM 볼륨: 말할 때 0.07(−23dB), 쉴 때 0.28(−11dB) (FrameStory 의 덕킹). 0.05초에 걸쳐 바꾼다."""
    loud, quiet, ramp = -11.06, -23.1, 0.05
    spans = sorted((_num(s.get("start")), _num(s.get("end"))) for s in spec.get("scenes") or []
                   if isinstance(s, dict))
    merged: list[list[float]] = []
    for a, b in spans:
        if merged and a - merged[-1][1] < 2 * ramp:
            merged[-1][1] = max(merged[-1][1], b)
        elif b > a:
            merged.append([a, b])
    pts: list[list[float]] = [[0.0, quiet if merged and merged[0][0] <= ramp else loud]]
    for a, b in merged:
        if a - ramp > pts[-1][0]:
            pts.append([round(a - ramp, 3), loud])
            pts.append([round(a, 3), quiet])
        pts.append([round(max(b, pts[-1][0] + 0.001), 3), quiet])
        pts.append([round(b + ramp, 3), loud])
    return pts


def build_scenes(spec: dict[str, Any], stage: list[dict[str, Any]], name: str,
                 files: dict[str, str]) -> dict[str, Any]:
    """render_spec + 그린 장면 구간 → `.overshorts` 장면 파일 내용.

    stage: stage-export.mjs 결과 ({id, file, kind, start, end}).
    files: spec 속 public 경로 → 장면 파일 옆으로 복사한 상대 경로 (없으면 그 항목을 뺀다).
    """
    w = int(_num(spec.get("width"), 1080)) or 1080
    h = int(_num(spec.get("height"), 1920)) or 1920
    duration = _num(spec.get("duration"), 1.0)
    frame = spec.get("preset") == "frame"
    pictures: list[dict[str, Any]] = []
    for s in stage:
        pictures.append({"track": "장면", "file": f"stage/{s['file']}", "start": s["start"], "end": s["end"],
                         "label": KIND_NAME.get(str(s.get("kind")), "장면")})
    segs = [s for s in spec.get("segments") or [] if isinstance(s, dict)]
    safe = _safe(spec)
    box_top, box_bottom = _pct(h, safe["tall"][0]), _pct(h, safe["tall"][1])
    sounds: list[dict[str, Any]] = []
    for i, seg in enumerate(segs):
        end = _num(segs[i + 1].get("start"), duration) if i + 1 < len(segs) else duration
        if seg.get("kind") == "reaction" and seg.get("image") in files:
            # 짤: 검은 바탕 가운데에 통째로 (FrameStory 와 같음). 편집기에서 다른 그림으로 바꾸기 쉽게 원본 그대로.
            pictures.append({"track": "장면", "file": files[seg["image"]], "start": _num(seg.get("start")),
                             "end": end, "rect": [0, box_top, w, round(box_bottom - box_top, 1)],
                             "fit": "contain", "label": "짤"})
        for m in seg.get("media") or []:
            if m.get("kind") == "video" and not m.get("muted", True) and m.get("src") in files:
                sounds.append({"track": "원본 소리", "file": files[m["src"]], "start": _num(m.get("start")),
                               "end": _num(m.get("end")), "label": "원문 동영상 소리"})
    pictures.sort(key=lambda p: p["start"])

    texts: list[dict[str, Any]] = []
    title = str(spec.get("title") or "").strip()
    if title and frame and (spec.get("layout") or {}).get("title_bar", True) is False:
        pass  # 제목 바 없는 유형: 유튜브 제목이 제목 역할 (화면에 얹지 않음)
    elif title and frame:
        mid = (safe["title"][0] + safe["title"][1]) / 2
        texts.append({"track": "제목", "text": title, "start": 0, "end": duration,
                      "size": 72 if len(title) > 16 else 84, "color": "#FFFFFF", "font": FONT,
                      "fontStyle": "Black", "center": [w / 2, _pct(h, mid)], "maxWidth": w - 120,
                      "maxLines": 2, "lineSpacing": 1.2})
    elif title:
        dark = (spec.get("theme") or {}).get("dark", True) is not False
        texts.append({"track": "제목", "text": title, "start": 0, "end": duration, "size": 64,
                      "color": "#FFFFFF" if dark else "#111111", "font": FONT, "fontStyle": "ExtraBold",
                      "center": [w / 2, round(147 * h / 1920, 1)], "maxWidth": w - 128, "maxLines": 1})
    texts += _frame_captions(spec, w, h) if frame else _card_captions(spec, w, h)
    source = str(spec.get("source") or "").strip()
    if source:
        y = _pct(h, safe["source"][0]) + 17 if frame else h - 70 - 18
        texts.append({"track": "출처", "text": source, "start": 0, "end": duration,
                      "size": 28 if frame else 30, "color": "#FFFFFF99" if frame else "#FFFFFFBF",
                      "font": FONT, "fontStyle": "Regular", "center": [w / 2, y], "maxWidth": w - 80,
                      "maxLines": 1})

    if spec.get("audio") in files:
        sounds.insert(0, {"track": "TTS", "file": files[spec["audio"]], "start": 0, "label": "나레이션"})
    if spec.get("bgm") in files:
        sounds.append({"track": "BGM", "file": files[spec["bgm"]], "start": 0, "end": duration, "loop": True,
                       "levels": _ducking(spec), "label": "BGM"})

    markers = []
    for i, seg in enumerate(segs):
        end = _num(segs[i + 1].get("start"), duration) if i + 1 < len(segs) else duration
        label = KIND_NAME.get(str(seg.get("kind")), "장면")
        sub = str(seg.get("subtitle") or seg.get("say") or "").strip()
        markers.append({"name": f"{i + 1}. {label}" + (f" · {sub}" if sub else ""),
                        "start": _num(seg.get("start")), "end": end,
                        "comment": "TTS 켬" if seg.get("tts") else "TTS 끔"})
    return {
        "format": FORMAT, "version": 1, "name": name, "width": w, "height": h,
        "fps": _num(spec.get("fps"), 60), "duration": duration,
        "tracks": {"video": ["장면", "자막", "제목", "출처"], "audio": ["TTS", "BGM", "원본 소리"]},
        "texts": texts, "pictures": pictures, "sounds": sounds, "markers": markers,
        "made_by": "OverShorts", "preset": spec.get("preset"),
    }


# --- 내보내기 ---

StageRunner = Callable[[dict[str, Any], Path, Path, list[dict[str, Any]]], list[dict[str, Any]]]


def run_stage_export(spec: dict[str, Any], public: Path, out: Path, spans: list[dict[str, Any]],
                     browser: str | None = None, timeout: float = 1800) -> list[dict[str, Any]]:
    """stage-export.mjs 로 장면 그림을 그린다."""
    if not spans:
        return []
    if not (RENDER_DIR / "node_modules").exists():
        raise RuntimeError(f"render 폴더에서 npm install 을 먼저 하세요 ({RENDER_DIR})")
    node = shutil.which("node") or "node"
    # node 는 render 폴더에서 돈다: 데이터 폴더가 상대 경로(설치본 ./data)여도 맞게 절대 경로로 넘긴다
    # (2026-11-15 설치본: 상대 경로라 사진을 못 찾아 'Error loading image' 로 실패)
    job = {"spec": spec, "publicDir": str(public.resolve()), "outDir": str(out.resolve()), "spans": spans,
           "browser": browser, "ignoreCertificateErrors": bool(os.environ.get("OVERSHORTS_IGNORE_CERT")),
           "gl": "angle" if platform.system() == "Windows" else None}
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8") as f:
        json.dump(job, f, ensure_ascii=False)
        job_path = f.name
    log_path = out.resolve().parent / "stage-export.log"
    try:
        proc = subprocess.run([node, "scripts/stage-export.mjs", job_path], cwd=RENDER_DIR,
                              capture_output=True, check=False, timeout=timeout,
                              creationflags=0x0800_0000 if os.name == "nt" else 0)
    except (OSError, subprocess.TimeoutExpired) as err:
        raise RuntimeError(f"장면 그림 그리기 실패: {err}") from err
    finally:
        Path(job_path).unlink(missing_ok=True)
    stdout, stderr = decode_output(proc.stdout), decode_output(proc.stderr)
    try:  # 전체 기록(호출 스택 포함)은 파일로, 화면에는 원인 한 줄만
        log_path.write_text(stdout + "\n" + stderr, encoding="utf-8")
    except OSError:
        pass
    result = _last_json(stdout)
    if result is None or proc.returncode != 0 or "error" in result:
        cause = str((result or {}).get("error") or _first_error_line(stderr + "\n" + stdout))
        raise RuntimeError(f"장면 그림 그리기 실패: {cause} (자세한 기록: {log_path})")
    errors = [e for e in result.get("errors") or [] if isinstance(e, dict)]
    for e in errors:
        log.warning("장면 %s 못 그림: %s", e.get("id"), e.get("error"))
    # 못 그린 장면은 {id, error} 로 같이 돌려준다 (export_job 이 경고로 남김)
    return list(result.get("spans") or []) + [{"id": e.get("id"), "error": str(e.get("error"))} for e in errors]


def _last_json(stdout: str) -> dict[str, Any] | None:
    for line in reversed(stdout.strip().splitlines()):
        if line.startswith("{"):
            try:
                v = json.loads(line)
            except ValueError:
                continue
            return v if isinstance(v, dict) else None
    return None


def _first_error_line(text: str) -> str:
    """Node·Remotion 출력에서 원인 한 줄 ('Error: …'), 없으면 첫 줄."""
    lines = [ln.strip() for ln in text.splitlines() if ln.strip()]
    for ln in lines:
        if re.search(r"(Error|error|오류)[:\]]", ln) and "at " not in ln[:4]:
            return ln[:300]
    return (lines[0] if lines else "알 수 없는 오류")[:300]


def export_job(job_dir: Path, out_dir: Path | None = None, browser: str | None = None,
               stage_runner: StageRunner | None = None) -> Path:
    """작업 폴더(spec.json, public/) → `<out_dir>/<제목>.overshorts` 와 옆의 파일들. 장면 파일 경로를 돌려준다."""
    job_dir = job_dir.resolve()
    spec = json.loads((job_dir / "spec.json").read_text(encoding="utf-8"))
    public = job_dir / "public"
    out = (out_dir or job_dir / "overfilm").resolve()
    if out.exists():
        shutil.rmtree(out)  # 예전 내보내기 (편집기는 프로젝트를 따로 저장하므로 여기 것은 다시 만들어도 된다)
    (out / "stage").mkdir(parents=True)
    (out / "media").mkdir()
    files: dict[str, str] = {}
    wanted = [spec.get("audio"), spec.get("bgm")]
    for seg in spec.get("segments") or []:
        wanted.append(seg.get("image"))
        wanted += [m.get("src") for m in seg.get("media") or [] if m.get("kind") == "video"]
    for rel in wanted:
        if not isinstance(rel, str) or not rel or rel in files:
            continue
        src = (public / rel).resolve()
        if not src.is_file() or public.resolve() not in src.parents:
            continue
        dst = out / "media" / Path(rel).name
        shutil.copyfile(src, dst)
        files[rel] = f"media/{dst.name}"
    spans = stage_spans(spec)
    runner = stage_runner or (lambda sp, pub, o, s: run_stage_export(sp, pub, o, s, browser))
    drawn = runner(spec, public, out / "stage", [{k: s[k] for k in ("id", "start", "end")} for s in spans])
    kinds = {s["id"]: s["kind"] for s in spans}
    failed = [d for d in drawn if "file" not in d]
    drawn = [d for d in drawn if "file" in d]
    for d in drawn:
        d["kind"] = kinds.get(d.get("id"), "장면")
    name = str(spec.get("title") or spec.get("post_title") or "OverShorts").strip() or "OverShorts"
    scenes = build_scenes(spec, drawn, name, files)
    if failed:
        scenes["warnings"] = [f"장면 {d.get('id')} ({KIND_NAME.get(kinds.get(d.get('id'), ''), '장면')}) 못 그림: "
                              f"{d.get('error')}" for d in failed]
    path = out / f"{safe_name(name)}{EXTENSION}"
    path.write_text(json.dumps(scenes, ensure_ascii=False, indent=1), encoding="utf-8")
    return path


# --- 편집기 열기 ---

def find_editor(setting: str | None = None) -> Path | None:
    """설정한 경로 → 설치본(%LOCALAPPDATA%\\Programs\\OverFilm\\OverFilm.exe) → PATH 의 OverFilm/filmcraft."""
    cands: list[Path] = []
    if setting:
        cands.append(Path(setting).expanduser())
    local = os.environ.get("LOCALAPPDATA")
    if local:
        cands.append(Path(local) / "Programs" / "OverFilm" / "OverFilm.exe")
    for exe in ("OverFilm", "overfilm", "filmcraft"):
        found = shutil.which(exe)
        if found:
            cands.append(Path(found))
    return next((c for c in cands if c.is_file()), None)


def open_in_editor(scene_file: Path, editor: Path) -> None:
    """편집기를 새로 띄워 장면 파일을 연다 (편집기의 파일 ▸ 가져오기와 같은 길)."""
    flags = 0
    if os.name == "nt":
        flags = 0x0000_0008 | 0x0000_0200  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    subprocess.Popen([str(editor), str(scene_file)], cwd=str(scene_file.parent), creationflags=flags,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     close_fds=True)
