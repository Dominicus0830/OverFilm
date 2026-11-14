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
