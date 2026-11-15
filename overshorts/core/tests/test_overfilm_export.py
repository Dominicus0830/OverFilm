"""OverFilm 편집기로 넘기기: 장면 구간·장면 파일 (Remotion 없이 가짜 그림으로)."""

import json
from pathlib import Path

from overshorts.export.overfilm import FORMAT, build_scenes, export_job, stage_spans


def _spec() -> dict:
    def w(t: str, s: float, e: float) -> dict:
        return {"text": t, "start": s, "end": e}

    return {
        "version": 2, "preset": "frame", "fps": 60, "width": 1080, "height": 1920, "duration": 10,
        "title": "햄스터 예고편 보고 우는 사람 손", "source": "출처: 디시인사이드", "audio": "voice.wav",
        "bgm": "bgm.mp3",
        "scenes": [{"start": 0.2, "end": 1.8, "words": [w("햄스터", 0.2, 1.8)]},
                   {"start": 6.0, "end": 7.0, "words": [w("본문", 6.0, 7.0)]}],
        "segments": [
            {"kind": "title", "start": 0, "end": 2, "media": [], "comments": []},
            {"kind": "comments", "start": 2, "end": 5, "media": [], "comments": [
                {"appear": 2, "read": [2.5, 3.0]}, {"appear": 4, "read": None}]},
            {"kind": "reaction", "start": 5, "end": 6, "image": "r2.png", "say": "이게 맞냐", "media": [],
             "comments": []},
            {"kind": "media", "start": 6, "end": 10, "media": [
                {"src": "m0.mp4", "kind": "video", "start": 6, "end": 9, "muted": False}], "comments": []},
        ],
        "captions": [{"start": 0.2, "end": 1.8, "words": [w("햄스터", 0.2, 1.8)], "style": None},
                     {"start": 5, "end": 6, "words": [w("이게 맞냐", 5, 6)], "style": "shout"}],
    }


def test_stage_spans_cut_where_the_screen_changes_and_skip_reaction_pictures():
    spans = stage_spans(_spec())
    assert [(s["start"], s["end"]) for s in spans] == [
        (0, 2), (2, 2.5), (2.5, 3.0), (3.0, 4), (4, 5), (6, 10)]
    assert all(s["kind"] != "reaction" for s in spans)
    card = stage_spans({**_spec(), "preset": "card"})
    assert len(card) == 1 and card[0]["end"] == 10


def test_scene_file_has_editable_texts_pictures_and_sounds():
    files = {"voice.wav": "media/voice.wav", "bgm.mp3": "media/bgm.mp3", "r2.png": "media/r2.png",
             "m0.mp4": "media/m0.mp4"}
    stage = [{"id": 1, "file": "stage-001.png", "kind": "title", "start": 0, "end": 2}]
    sc = build_scenes(_spec(), stage, "햄스터", files)
    assert sc["format"] == FORMAT and sc["version"] == 1
    tracks = {t["track"] for t in sc["texts"]}
    assert tracks == {"제목", "자막", "출처"}
    shout = next(t for t in sc["texts"] if t["text"] == "이게 맞냐")
    assert shout["color"] == "#FF3B30" and shout["font"] == "Black Han Sans"
    reaction = next(p for p in sc["pictures"] if p["label"] == "짤")
    assert reaction["file"] == "media/r2.png" and reaction["fit"] == "contain"
    assert reaction["rect"][1] == 537.6  # 화면 높이 28%
    sounds = {s["track"]: s for s in sc["sounds"]}
    assert sounds["TTS"]["file"] == "media/voice.wav"
    assert sounds["BGM"]["loop"] and sounds["BGM"]["levels"][0] == [0.0, -11.06]
    assert min(v for _, v in sounds["BGM"]["levels"]) == -23.1  # 말할 때 덕킹
    assert sounds["원본 소리"]["start"] == 6
    assert [m["name"] for m in sc["markers"]][2].startswith("3. 짤")


def test_export_job_copies_only_what_the_spec_uses(tmp_path: Path):
    job = tmp_path / "7"
    (job / "public").mkdir(parents=True)
    for name in ("voice.wav", "bgm.mp3", "r2.png", "m0.mp4", "unused.png"):
        (job / "public" / name).write_bytes(b"x")
    spec = _spec()
    spec["audio"] = "../../secret.wav"  # 작업 폴더 밖은 복사하지 않는다
    (job / "spec.json").write_text(json.dumps(spec), encoding="utf-8")
    calls = []

    def fake(sp, pub, out, spans):
        calls.append(spans)
        (out / "stage-001.png").write_bytes(b"png")
        return [{"id": 1, "file": "stage-001.png", "kind": "image", "start": 0, "end": 2}]

    path = export_job(job, stage_runner=fake)
    assert path.suffix == ".overshorts" and path.parent == job / "overfilm"
    assert sorted(p.name for p in (job / "overfilm" / "media").iterdir()) == ["bgm.mp3", "m0.mp4", "r2.png"]
    sc = json.loads(path.read_text(encoding="utf-8"))
    assert "TTS" not in {s["track"] for s in sc["sounds"]}
    assert sc["pictures"][0]["label"] == "글 머리" and len(calls[0]) == 6


def test_relative_job_dir_and_failed_scenes(tmp_path: Path, monkeypatch):
    """설치본은 데이터 폴더가 상대 경로(./data): 그림 그리는 쪽에는 절대 경로로 넘기고, 못 그린 장면은 경고로."""
    job = tmp_path / "data" / "jobs" / "3"
    (job / "public").mkdir(parents=True)
    (job / "spec.json").write_text(json.dumps(_spec()), encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    seen = {}

    def fake(sp, pub, out, spans):
        seen["pub"], seen["out"] = pub, out
        return [{"id": 1, "error": "Error: Error loading image with src: m0.png"}]

    path = export_job(Path("data/jobs/3"), stage_runner=fake)
    assert seen["pub"].is_absolute() and seen["out"].is_absolute() and path.is_absolute()
    sc = json.loads(path.read_text(encoding="utf-8"))
    assert "Error loading image" in sc["warnings"][0] and sc["pictures"] == []


def test_first_error_line_skips_the_stack():
    from overshorts.export.overfilm import _first_error_line, _last_json

    out = "bundling...\n{\"error\": \"Error: Error loading image with src: x.png\"}\n"
    assert _last_json(out) == {"error": "Error: Error loading image with src: x.png"}
    err = "    at processDispatchQueue (http://localhost:3000/bundle.js:1:2)\nError: boom\n  at x"
    assert _first_error_line(err) == "Error: boom"
