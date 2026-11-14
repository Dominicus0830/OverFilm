import json

import pytest

fastapi = pytest.importorskip("fastapi")
from fastapi.testclient import TestClient  # noqa: E402

from overshorts.db import Store  # noqa: E402
from overshorts.server.app import create_app  # noqa: E402


def seed(tmp_path):
    data = tmp_path / "data"
    st = Store(data / "overshorts.db")
    pid = st.upsert_post({"gall_id": "ex", "post_no": 9, "gall_type": "mgallery", "title": "제목",
                          "body": "본문", "comments": [{"text": "댓글"}]})
    job = st.add_job(pid, "preview_ready")
    sid = st.add_script(pid, 1, "담백", {"title": "t", "lines": [{"part": "hook", "text": "훅"}],
                                        "warnings": [], "cringe": None, "est_seconds": 3}, 0.1, "x")
    spec = data / "jobs" / str(job) / "spec.json"
    spec.parent.mkdir(parents=True)
    spec.write_text(json.dumps({"duration": 3, "scenes": [], "audio": "voice.wav"}), encoding="utf-8")
    st.conn.execute("UPDATE jobs SET script_id=?, spec_path=? WHERE id=?", (sid, str(spec), job))
    st.conn.commit()
    return data, job


def test_review_flow(tmp_path):
    data, job = seed(tmp_path)
    app = create_app(cfg={"data_dir": str(data), "style": {"dir": str(tmp_path / "style")}},
                     start_worker=False)
    c = TestClient(app)
    assert c.post("/api/parse", json={"url": "https://m.dcinside.com/board/ex/9"}).json()["ok"]
    assert not c.post("/api/parse", json={"url": "https://example.com"}).json()["ok"]
    items = c.get("/api/review").json()
    assert len(items) == 1 and items[0]["spec"]["audio"] == "voice.wav"
    assert items[0]["post_url"] == "https://gall.dcinside.com/mgallery/board/view/?id=ex&no=9"
    r = c.post(f"/api/review/{job}/cringe", json={"line": 0, "text": "훅", "reason": "오글"})
    assert r.status_code == 200 and (tmp_path / "style" / "reports.jsonl").exists()
    assert c.post(f"/api/review/{job}/approve").status_code == 200
    assert c.post(f"/api/review/{job}/approve").status_code == 409  # 두 번 승인 안 됨
    assert app.state.worker.tasks.qsize() == 1  # 최종 렌더가 대기열에
    assert c.get("/api/status").json()["today"]["approved"] == 1
    assert c.post("/api/review/999/discard").status_code == 404
    assert c.post("/api/start", json={"url": "nope"}).status_code == 400
