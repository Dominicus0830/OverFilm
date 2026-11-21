"""작업 큐의 삭제·빼기·취소 (2026-11-21)."""

import sys
import threading
import time

import pytest

from overshorts import proc

fastapi = pytest.importorskip("fastapi")
from fastapi.testclient import TestClient  # noqa: E402

from overshorts.server.app import create_app  # noqa: E402

from test_server import seed  # noqa: E402


def test_delete_job_and_pending_task(tmp_path):
    data, job = seed(tmp_path)
    app = create_app(cfg={"data_dir": str(data)}, start_worker=False)
    c = TestClient(app)
    assert c.delete(f"/api/jobs/{job}").json()["ok"]
    assert c.get("/api/jobs").json() == []
    assert not (data / "jobs" / str(job)).exists()
    assert c.delete(f"/api/jobs/{job}").status_code == 404
    w = app.state.worker
    a = w.submit("하나", lambda p: None)
    w.submit("둘", lambda p: None)
    assert [t["label"] for t in c.get("/api/worker").json()["pending"]] == ["하나", "둘"]
    assert c.delete(f"/api/worker/tasks/{a}").json()["pending"][0]["label"] == "둘"
    assert c.delete(f"/api/worker/tasks/{a}").status_code == 404
    assert c.post("/api/worker/cancel").json()["ok"] is False  # 도는 작업 없음


def test_cancel_kills_running_subprocess():
    ev = threading.Event()
    out: dict = {}

    def work() -> None:
        proc.bind_cancel(ev)
        t0 = time.monotonic()
        try:
            proc.run([sys.executable, "-c", "import time; time.sleep(30)"])
        except proc.Cancelled:
            out["cancelled"] = time.monotonic() - t0

    t = threading.Thread(target=work)
    t.start()
    time.sleep(0.5)
    ev.set()
    t.join(10)
    assert out.get("cancelled", 99) < 5


def test_run_without_cancel_is_plain():
    r = proc.run([sys.executable, "-c", "import sys; sys.stdout.write(sys.stdin.read())"], input=b"hi")
    assert r.returncode == 0 and r.stdout == b"hi"
