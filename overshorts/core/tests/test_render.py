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
    assert spec["background_sdr"] is False  # 읽을 수 없는 배경은 색 변환 유지
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


def test_comment_pages_keep_replies_with_parent():
    from overshorts.render.spec import _paginate
    long = "가" * 150
    rows = [{"no": 0, "depth": 0, "text": long, "read": None},
            {"no": 1, "depth": 0, "text": long, "read": None},
            {"no": 2, "depth": 1, "text": "답글", "read": None},
            {"no": 3, "depth": 1, "text": "답글 둘", "read": [9.0, 9.5]},
            {"no": 4, "depth": 0, "text": "짧음", "read": None}]
    _paginate(rows, 0.0, 12.0)
    page = {r["no"]: r["page"] for r in rows}
    assert page[1] == page[2] == page[3]  # 원댓글과 답글은 같은 장
    assert page[4] == page[3]  # 짧으면 다른 댓글도 같은 장에
    assert all(r["appear"] <= 8.7 for r in rows if r["page"] == page[3])


def test_silent_captions_spread_over_segment():
    from overshorts.render.spec import silent_captions

    caps = silent_captions(["우리탱 왜던짐?"], 2.0, 5.0, 1)
    assert caps and caps[0]["start"] >= 2.0 and caps[-1]["end"] == 5.0
    assert " ".join(w["text"] for c in caps for w in c["words"]) == "우리탱 왜던짐?"


def test_rich_lines_follow_writer_styles():
    from overshorts.render.spec import rich_lines
    styles = [{"text": "크아아아아악 씨이바아아알 퀸장연 기습시위!!!",
               "runs": [{"t": "크아아아아악 씨이바아아알 퀸장연 기습시위!!!", "color": "#ff0000", "size": 36}]}]
    exact = rich_lines("크아아아아악 씨이바아아알 퀸장연 기습시위!!!", styles)
    assert exact == [[{"t": "크아아아아악 씨이바아아알 퀸장연 기습시위!!!", "color": "#ff0000", "size": 36}]]
    # 욕을 초성으로 바꾼 대사도 같은 원문 줄의 꾸밈
    assert rich_lines("크아아아아악 ㅅㅂ 퀸장연 기습시위!!!", styles)[0][0]["color"] == "#ff0000"
    assert rich_lines("우리탱 왜던짐?", styles) is None
    assert rich_lines("아무 글", []) is None


def test_decode_output_reads_cp949_errors():
    from overshorts.proc import decode_output
    msg = "'remotion'은(는) 내부 또는 외부 명령, 실행할 수 있는 프로그램, 또는 배치 파일이 아닙니다."
    assert decode_output(msg.encode("cp949")) == msg
    assert decode_output("렌더 완료".encode()) == "렌더 완료"
    assert decode_output(None) == ""


def test_render_calls_remotion_cli_with_node():
    from pathlib import Path

    from overshorts.render import runner
    args = render_args(Path("s.json"), Path("p"), Path("o.mp4"), "cpu")
    if runner.REMOTION_CLI.is_file() and __import__("shutil").which("node"):
        assert args[1].endswith("remotion-cli.js") and args[2] == "render"


def test_background_sdr_only_when_probe_says_so():
    from overshorts.media import is_sdr_probe

    def v(**kw):
        return {"streams": [{"codec_type": "video", **kw}]}

    assert is_sdr_probe(v(color_transfer="bt709")) and is_sdr_probe(v())  # 표시 없는 녹화본도 SDR
    assert not is_sdr_probe(v(color_transfer="smpte2084")) and not is_sdr_probe(v(color_transfer="arib-std-b67"))
    assert not is_sdr_probe({"streams": [{"codec_type": "audio"}]}) and not is_sdr_probe({})


def test_concurrency_auto_is_never_below_remotion_default():
    from overshorts.render.runner import auto_concurrency, choose_concurrency

    assert auto_concurrency(4, None) == 2 and auto_concurrency(4, 8) == 2  # 모르거나 적으면 Remotion 기본(절반)
    assert auto_concurrency(4, 16) == 4  # 측정한 4코어 16GB: 4탭
    assert auto_concurrency(4, 15.6) == 3 and auto_concurrency(4, 14.9) == 2  # 16GB PC 는 15.x 로 보임
    assert auto_concurrency(12, 16) == 6 and auto_concurrency(12, 32) == 8  # 상한 8
    assert auto_concurrency(32, 64) == 16  # 기본이 더 크면 기본 그대로
    assert auto_concurrency(1, 64) == 1 and auto_concurrency(4, float("nan")) == 2
    assert choose_concurrency("3", cores=8) == 3 and choose_concurrency(16, cores=4) == 4
    assert choose_concurrency("auto", cores=4, ram_gb=16) == 4 and choose_concurrency(None, cores=4, ram_gb=4) == 2
    assert choose_concurrency("0", cores=4) == 1


def test_render_args_with_prebuilt_bundle():
    from pathlib import Path

    args = render_args(Path("s.json"), Path("p"), Path("o.mp4"), "cpu", concurrency=4, serve_dir=Path("/x/serve"))
    i = args.index("render")
    assert args[i + 1] == str(Path("/x/serve")) and args[i + 2] == "Story"
    assert "--concurrency=4" in args and not any(a.startswith("--public-dir") for a in args)
    old = render_args(Path("s.json"), Path("p"), Path("o.mp4"), "cpu")
    assert "src/index.ts" in old and "--public-dir=p" in old and not any(a.startswith("--concurrency") for a in old)


def test_serve_dir_links_bundle_and_job_public(tmp_path):
    from overshorts.render.bundle import bundle_key, make_serve_dir

    bundle = tmp_path / "bundle"
    (bundle / "public").mkdir(parents=True)
    (bundle / "index.html").write_text("<html>", encoding="utf-8")
    (bundle / "public" / "stale.txt").write_text("x", encoding="utf-8")
    pub = tmp_path / "public"
    (pub / "sub").mkdir(parents=True)
    (pub / "voice.wav").write_bytes(b"RIFF")
    (pub / "sub" / "a.png").write_bytes(b"png")
    serve = make_serve_dir(bundle, pub, tmp_path / "job" / "serve")
    assert serve is not None
    assert (serve / "index.html").read_text(encoding="utf-8") == "<html>"
    assert (serve / "public" / "voice.wav").read_bytes() == b"RIFF" and (serve / "public" / "sub" / "a.png").is_file()
    assert not (serve / "public" / "stale.txt").exists()

    render = tmp_path / "render"
    (render / "src").mkdir(parents=True)
    (render / "src" / "a.tsx").write_text("1", encoding="utf-8")
    k1 = bundle_key(render)
    assert k1 and bundle_key(render) == k1
    (render / "src" / "a.tsx").write_text("2", encoding="utf-8")
    assert bundle_key(render) != k1
    assert bundle_key(tmp_path / "nothing") is None
