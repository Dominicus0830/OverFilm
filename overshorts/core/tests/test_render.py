import json

from overshorts.db import Store
from overshorts.render.runner import GpuInfo, choose_encoder, render_args
from overshorts.render.spec import build_spec, spec_pending

TIMING = {"duration": 3.0, "sentences": [
    {"line": 0, "part": "hook", "speaker": "narrator", "text": "힐량이 500이었다.", "start": 0, "end": 1.5,
     "words": [{"text": "힐량이", "start": 0.1, "end": 0.6}, {"text": "500이었다", "start": 0.6, "end": 1.4}]},
    {"line": 1, "part": "comment", "speaker": "comment", "text": "필요했음", "start": 1.65, "end": 3.0,
     "words": [{"text": "필요했음", "start": 1.7, "end": 2.9}]},
]}


def test_build_spec_emphasis_and_duration():
    spec = build_spec(TIMING, "받은 힐량 표시", "일반", "", "출처", None)
    words = [w for s in spec["scenes"] for w in s["words"]]
    assert [w["emphasis"] for w in words] == [True, True, False]  # 제목 낱말 '힐량', 숫자
    assert spec["duration"] == 3.6 and spec["audio"] == "voice.wav"
    assert spec["scenes"][1]["speaker"] == "comment"


def test_encoder_choice_supports_radeon():
    assert choose_encoder("auto", GpuInfo("amd", "Radeon RX 9060 XT")) == "cpu"
    assert choose_encoder("auto", GpuInfo("nvidia", "RTX 4060")) == "nvenc"
    args = render_args(__import__("pathlib").Path("s.json"), __import__("pathlib").Path("p"),
                       __import__("pathlib").Path("o.mp4"), "nvenc", gl="angle")
    assert "--hardware-acceleration=if-possible" in args and "--gl=angle" in args


def test_spec_stage(tmp_path):
    store = Store(":memory:")
    pid = store.upsert_post({"gall_id": "g", "post_no": 1, "title": "t", "body": "b",
                             "gall_name": "예시 마이너 갤러리", "subject": "일반"})
    job = store.add_job(pid, "scripted")
    sid = store.add_script(pid, 1, "담백", {"title": "받은 힐량", "lines": []}, 0.1, "x")
    store.set_job_status(job, "aligned", None, script_id=sid)
    d = tmp_path / "jobs" / str(job)
    d.mkdir(parents=True)
    (d / "timing.json").write_text(json.dumps(TIMING), encoding="utf-8")
    (d / "voice.wav").write_bytes(b"RIFF")
    bg = tmp_path / "bg"
    bg.mkdir()
    (bg / "a.mp4").write_bytes(b"x")
    assert spec_pending(store, tmp_path / "jobs", bg) == {"ready": 1, "failed": 0}
    spec = json.loads((d / "spec.json").read_text(encoding="utf-8"))
    assert spec["source"] == "출처: DC인사이드 예시 마이너 갤러리" and spec["background"] == "bg.mp4"
    assert (d / "public" / "voice.wav").exists() and (d / "public" / "bg.mp4").exists()
    assert store.conn.execute("SELECT status FROM jobs").fetchone()["status"] == "preview_ready"


def test_caption_chunks_two_to_four_words():
    from overshorts.render.spec import caption_chunks
    words = [{"text": t, "start": i * 0.3, "end": i * 0.3 + 0.25} for i, t in
             enumerate("햄스터가 먼저 들이받은 건 다리 주탑이었다".split())]
    chunks = caption_chunks([{"words": words, "seg": 1}])
    assert [len(c["words"]) for c in chunks] == [4, 2]
    assert all(c["seg"] == 1 for c in chunks) and chunks[0]["end"] == chunks[1]["start"]


def test_frame_spec_media_and_comments(tmp_path):
    from overshorts.render.spec import SpecOptions, build_frame_spec
    img = tmp_path / "a.img"
    img.write_bytes(b"\x89PNG\r\n\x1a\n" + b"\x00" * 8 + (16).to_bytes(4, "big") + (9).to_bytes(4, "big"))
    media = [{"name": "사진 1", "kind": "image", "path": str(img), "width": 16, "height": 9}]
    comments = [{"no": 1, "text": "첫 댓글", "nick": "옵붕이1", "ip": "1.2"},
                {"no": 2, "text": "답글", "depth": 1, "parent": 1, "nick": "고닉", "icon": "fix"}]
    script = {"title": "제목", "segments": [
        {"kind": "title", "tts": False},
        {"kind": "media", "tts": True, "media": [{"name": "사진 1", "display": "fill", "line": 0}]},
        {"kind": "comments", "tts": True, "comments": [0, 1], "read": [1]}],
        "lines": [{"part": "pause", "seg": 0}, {"part": "body", "text": "사진 설명", "seg": 1},
                  {"part": "pause", "seg": 1}, {"part": "comment", "text": "답글", "comment_no": 1, "seg": 2},
                  {"part": "pause", "seg": 2}]}
    timing = {"duration": 9.0, "sentences": [
        {"line": 1, "part": "body", "text": "사진 설명", "start": 2.1, "end": 3.0, "words": []},
        {"line": 3, "part": "comment", "speaker": "narrator", "text": "답글", "start": 6.0, "end": 6.5,
         "words": []}],
        "segments": [{"seg": 0, "start": 0, "end": 2.0}, {"seg": 1, "start": 2.0, "end": 4.5},
                     {"seg": 2, "start": 4.5, "end": 9.0}]}
    pub = tmp_path / "public"
    pub.mkdir()
    spec = build_frame_spec(timing, script, {"title": "원문"}, pub, "출처", None,
                            SpecOptions(nickname="replace"), media, comments)
    assert spec["preset"] == "frame" and spec["version"] == 2 and spec["post_title"] == "원문"
    m = spec["segments"][1]["media"][0]
    assert m["src"] == "m0.png" and (pub / "m0.png").exists() and m["start"] == 2.0
    cs = spec["segments"][2]["comments"]
    assert [c["nick"] for c in cs] == ["ㅇㅇ", "ㅇㅇ"] and cs[1]["read"] == [6.0, 6.5]
    assert cs[0]["appear"] <= cs[1]["appear"] <= 5.8
    assert spec["captions"] == []  # 단어 시각이 없으면 자막 없음, 읽는 댓글은 자막으로 안 띄움
