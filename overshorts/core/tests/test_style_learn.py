"""스타일 복사·학습 (Premiere 파일·통과된 쇼츠·유튜브 링크)."""

import gzip
import json

import pytest

from overshorts.db import Store
from overshorts.llm import Router
from overshorts.presets import load_presets
from overshorts.presets_learn import (
    clean_links,
    copy_preset,
    learn,
    passed_shorts,
    premiere_facts,
    short_facts,
)


class Fake:
    name = "fake"

    def __init__(self, answer):
        self.answer, self.tasks = answer, []

    def complete(self, task, schema):
        self.tasks.append(task)
        return self.answer


def test_copy_builtin_and_custom(tmp_path):
    a = copy_preset(tmp_path, "ogal")
    assert a.id == "ogal-copy" and a.label.endswith("복사") and a.origin == "custom"
    b = copy_preset(tmp_path, "ogal", "내 옵갤")
    assert b.id == "ogal-copy2" and b.label == "내 옵갤"
    table = load_presets(tmp_path)
    assert table["ogal"].origin == "builtin" and table["ogal-copy2"].guide == table["ogal"].guide
    with pytest.raises(ValueError):
        copy_preset(tmp_path, "nope")


def _prproj(durs_sec):
    t = 254_016_000_000
    items, start = [], 0
    for d in durs_sec:
        end = start + int(d * t)
        items.append(f"<VideoClipTrackItem><ClipTrackItem><TrackItem><Start>{start}</Start><End>{end}</End>"
                     f"</TrackItem></ClipTrackItem></VideoClipTrackItem>")
        start = end
    xml = ("<?xml version='1.0'?><PremiereData Version='3'><Sequence><Name>쇼츠1</Name></Sequence>"
           "<FrameRect>0,0,1080,1920</FrameRect>" + "".join(items)
           + "<AudioClipTrackItem><Start>0</Start><End>254016000000</End></AudioClipTrackItem>"
           "<ActualMediaFilePath>C:\\sfx\\whoosh.wav</ActualMediaFilePath></PremiereData>")
    return gzip.compress(xml.encode("utf-8"))


def test_premiere_facts_prproj_and_fcp_xml():
    f = premiere_facts(_prproj([0.8, 1.5, 2.0, 6.0]), "C:\\a\\쇼츠.prproj")
    assert f["format"] == "prproj" and f["frame_size"] == "1080x1920" and f["file"] == "쇼츠.prproj"
    assert f["video_clips"]["count"] == 4 and f["video_clips"]["under_1s"] == 1 and f["video_clips"]["over_5s"] == 1
    assert "whoosh.wav" in f["sound_files"] and f["sequences"] == ["쇼츠1"]
    xml = ("<xmeml><sequence><name>S</name><rate><timebase>30</timebase></rate><media><video><track>"
           "<clipitem><start>0</start><end>45</end></clipitem><clipitem><start>45</start><end>105</end></clipitem>"
           "</track></video><audio></audio></media></sequence></xmeml>")
    g = premiere_facts(xml.encode(), "s.xml")
    assert g["format"] == "fcp-xml" and g["video_clips"]["count"] == 2 and g["video_clips"]["median_sec"] == 1.75
    with pytest.raises(ValueError):
        premiere_facts(b"not premiere", "x.xml")
    with pytest.raises(ValueError):
        premiere_facts(b"\x1f\x8b broken", "x.prproj")


def test_clean_links_only_youtube():
    assert clean_links(["https://youtube.com/shorts/lKP7Ixw1ixc", "https://example.com/x", 3,
                        "https://youtu.be/abc"]) == ["https://youtube.com/shorts/lKP7Ixw1ixc", "https://youtu.be/abc"]


def _seed(tmp_path):
    st = Store(tmp_path / "db.sqlite")
    pid = st.upsert_post({"gall_id": "ex", "post_no": 1, "title": "통과한 글", "body": "b", "comments": []})
    ok = st.add_job(pid, "rendered")
    sid = st.add_script(pid, 1, "담백", {"title": "제목", "composition": "A", "segments": [
        {"kind": "title", "tts": False, "lines": ["훅 한 줄"]},
        {"kind": "comments", "tts": False, "comments": [1, 2], "screens": [[1, 2]]}]}, 0.1, "x")
    spec = tmp_path / "spec.json"
    spec.write_text(json.dumps({"duration": 31.5, "segments": [{"kind": "title", "start": 0, "end": 2.5}]}),
                    encoding="utf-8")
    st.conn.execute("UPDATE jobs SET script_id=?, spec_path=? WHERE id=?", (sid, str(spec), ok))
    pid2 = st.upsert_post({"gall_id": "ex", "post_no": 2, "title": "검수 중", "body": "b", "comments": []})
    wait = st.add_job(pid2, "preview_ready")
    st.conn.commit()
    return st, ok, wait


def test_passed_shorts_and_facts(tmp_path):
    st, ok, wait = _seed(tmp_path)
    assert [r["id"] for r in passed_shorts(st)] == [ok]
    facts = short_facts(st, [ok, wait, 999])
    assert len(facts) == 1 and facts[0]["duration_sec"] == 31.5
    assert facts[0]["segments"][1]["screens"] == 1 and facts[0]["segment_seconds"][0]["sec"] == 2.5


def test_learn_supplement_keeps_id_and_new_avoids_clash(tmp_path):
    st, ok, _ = _seed(tmp_path)
    ans = {"reply": "컷이 짧습니다", "findings": ["[쇼츠] 댓글 비중 큼"],
           "preset": {"id": "whatever", "label": "학습", "guide": "컷은 1초 안팎"}}
    f = Fake(ans)
    r = Router({"preset": f}, f)
    out = learn(r, st, tmp_path, "ogal", None, [ok], ["https://youtu.be/abcdef"], "짧게")
    assert out["preset"]["id"] == "ogal" and out["base"] == "ogal"
    assert f.tasks[0].tools == "WebSearch,WebFetch" and "통과한 글" in f.tasks[0].prompt
    assert "https://youtu.be/abcdef" in out["preset"]["reference"]["videos"]
    f2 = Fake({**ans, "preset": {"id": "ogal", "label": "새것"}})
    out2 = learn(Router({"preset": f2}, f2), st, tmp_path, None, {"format": "prproj"}, [], [], "")
    assert out2["preset"]["id"] == "ogal-new" and f2.tasks[0].tools == ""
    with pytest.raises(ValueError):
        learn(r, st, tmp_path, None, None, [], ["https://example.com"], "")


def test_server_copy_passed_learn(tmp_path):
    pytest.importorskip("fastapi")
    from fastapi.testclient import TestClient

    from overshorts.server.app import create_app

    data = tmp_path / "data"
    st = Store(data / "overshorts.db")
    pid = st.upsert_post({"gall_id": "ex", "post_no": 1, "title": "통과", "body": "b", "comments": []})
    st.add_job(pid, "approved")
    st.close()
    c = TestClient(create_app(cfg={"data_dir": str(data), "style": {"dir": str(tmp_path / "style")}},
                              start_worker=False))
    r = c.post("/api/presets/ogal/copy", json={"label": "복사본"}).json()
    assert r["ok"] and r["id"] == "ogal-copy"
    assert any(p["id"] == "ogal-copy" for p in c.get("/api/presets").json())
    assert c.post("/api/presets/nope/copy", json={}).status_code == 400
    assert [p["title"] for p in c.get("/api/presets/passed").json()] == ["통과"]
    bad = c.post("/api/presets/learn", json={"premiere": {"name": "a.prproj", "data": "bm90IHByZW1pZXJl"}}).json()
    assert not bad["ok"] and "Premiere" in bad["error"]
