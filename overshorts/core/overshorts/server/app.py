"""패널 백엔드 (기획서 3.7, M5). `overshorts serve` 로 띄우면 브라우저가 열린다.

- 긴 작업(수집→검수 대기, 최종 렌더)은 작업 스레드 하나가 차례로 돈다. 크롤링이 한 줄로만
  나가야 하고, GPU 모델도 한 번에 하나만 올려야 하기 때문이다.
- 진행 상황은 WebSocket(/ws)으로 실시간 전송.
- 미리보기는 패널이 @remotion/player 로 spec 을 그린다. 오디오·배경은 /files/jobs/<id>/public/.
"""

from __future__ import annotations

import asyncio
import json
import queue
import threading
import traceback
from collections.abc import Callable
from contextlib import asynccontextmanager
from datetime import datetime
from pathlib import Path
from typing import Any

from fastapi import FastAPI, HTTPException, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel, Field

from ..config import crawl_limits, load_config
from ..crawler.budget import CrawlGuard
from ..crawler.collector import GalleryOptions
from ..db import Store
from ..pipeline import Pipeline
from ..urls import GALL_TYPES, DcUrl, UrlError, parse_url

PANEL_DIST = Path(__file__).resolve().parents[3] / "panel" / "dist"


class Hub:
    """작업 스레드 → WebSocket 클라이언트로 이벤트를 나른다."""

    def __init__(self) -> None:
        self.loop: asyncio.AbstractEventLoop | None = None
        self.clients: set[asyncio.Queue] = set()
        self.recent: list[dict[str, Any]] = []
        self.lock = threading.Lock()

    def emit(self, event: dict[str, Any]) -> None:
        event = {"time": datetime.now().isoformat(timespec="seconds"), **event}
        with self.lock:
            self.recent = (self.recent + [event])[-200:]
        loop = self.loop
        if loop is None:
            return
        for q in list(self.clients):
            try:
                loop.call_soon_threadsafe(q.put_nowait, event)
            except RuntimeError:
                pass


class Worker:
    def __init__(self, cfg: dict[str, Any], db_path: Path, data: Path, hub: Hub,
                 pipeline_factory: Callable[[Store], Pipeline] | None = None):
        self.cfg, self.db_path, self.data, self.hub = cfg, db_path, data, hub
        self.tasks: queue.Queue[tuple[str, Callable[[Pipeline], None]]] = queue.Queue()
        self.current: str | None = None
        self.paused = threading.Event()
        self.factory = pipeline_factory
        self.thread = threading.Thread(target=self._run, name="overshorts-worker", daemon=True)

    def start(self) -> None:
        self.thread.start()

    def submit(self, label: str, fn: Callable[[Pipeline], None]) -> None:
        self.tasks.put((label, fn))
        self.hub.emit({"type": "queue", "pending": self.tasks.qsize(), "current": self.current})

    def _run(self) -> None:
        store = Store(self.db_path)
        pipe = (self.factory(store) if self.factory
                else Pipeline(self.cfg, store, self.data, emit=self.hub.emit))
        while True:
            label, fn = self.tasks.get()
            while self.paused.is_set():  # 일시정지: 다음 작업을 꺼내지 않고 기다린다
                threading.Event().wait(1.0)
            self.current = label
            self.hub.emit({"type": "task", "state": "start", "label": label})
            try:
                fn(pipe)
                self.hub.emit({"type": "task", "state": "done", "label": label})
            except Exception as err:  # noqa: BLE001 - 작업 하나가 실패해도 서버는 계속 돈다
                self.hub.emit({"type": "task", "state": "failed", "label": label,
                               "message": str(err)[:500],
                               "detail": traceback.format_exc(limit=3)[-1500:]})
            finally:
                self.current = None
                self.hub.emit({"type": "jobs_changed"})


# --- 요청 모델 ---
class ParseIn(BaseModel):
    url: str = Field(max_length=2048)


class StartIn(BaseModel):
    url: str = Field(max_length=2048)
    recommend_only: bool = False
    min_upvotes: int = Field(default=0, ge=0, le=100_000)
    within_hours: float | None = Field(default=24, ge=0, le=24 * 365)
    max_posts: int = Field(default=5, ge=1, le=50)
    tone: str | None = None


class EditIn(BaseModel):
    lines: list[dict[str, Any]] = Field(max_length=40)


class RegenIn(BaseModel):
    tone: str | None = None
    preset: str | None = None


class CringeIn(BaseModel):
    line: int = Field(ge=0, le=100)
    text: str = Field(max_length=300)
    reason: str = Field(default="", max_length=300)


def create_app(config_path: str | None = None, cfg: dict[str, Any] | None = None,
               start_worker: bool = True,
               pipeline_factory: Callable[[Store], Pipeline] | None = None) -> FastAPI:
    cfg = cfg or load_config(config_path)
    data = Path(str(cfg.get("data_dir") or "./data"))
    db_path = data / "overshorts.db"
    Store(db_path).close()  # 스키마 만들기
    hub = Hub()
    worker = Worker(cfg, db_path, data, hub, pipeline_factory)
    @asynccontextmanager
    async def lifespan(_app: FastAPI):
        hub.loop = asyncio.get_running_loop()
        if start_worker:
            worker.start()
        yield

    app = FastAPI(title="OverShorts 패널", lifespan=lifespan)
    app.state.worker = worker
    app.state.hub = hub
    (data / "jobs").mkdir(parents=True, exist_ok=True)

    def db() -> Store:
        return Store(db_path)

    # --- 홈 ---
    @app.get("/api/status")
    def status() -> dict[str, Any]:
        s = db()
        try:
            today = datetime.now().date().isoformat()

            def count(sql: str) -> int:
                return int(s.conn.execute(sql, (today,)).fetchone()[0])

            from ..render.runner import detect_gpu

            gpu = app.state.__dict__.setdefault("gpu", detect_gpu())
            last_limit = next((e for e in reversed(hub.recent)
                               if "한도" in str(e.get("message", ""))), None)
            return {
                "today": {
                    "collected": count("SELECT COUNT(*) FROM posts WHERE fetched_at >= ?"),
                    "generated": count("SELECT COUNT(*) FROM jobs WHERE status IN ('preview_ready',"
                                       "'approved','rendered') AND updated_at >= ?"),
                    "approved": count("SELECT COUNT(*) FROM jobs WHERE status IN ('approved',"
                                      "'rendered') AND updated_at >= ?"),
                },
                "review_waiting": int(s.conn.execute(
                    "SELECT COUNT(*) FROM jobs WHERE status='preview_ready'").fetchone()[0]),
                "budget": CrawlGuard(s, crawl_limits(cfg)).status(),
                "budget_warnings": crawl_limits(cfg).warnings(),
                "claude": {"limited": last_limit is not None,
                           "message": last_limit.get("message") if last_limit else None},
                "gpu": {"vendor": gpu.vendor, "name": gpu.name},
                "worker": {"current": worker.current, "pending": worker.tasks.qsize(),
                           "paused": worker.paused.is_set()},
            }
        finally:
            s.close()

    # --- 새로 만들기 ---
    @app.post("/api/parse")
    def parse(body: ParseIn) -> dict[str, Any]:
        try:
            u = parse_url(body.url)
        except UrlError as err:
            return {"ok": False, "error": str(err)}
        return {"ok": True, "kind": u.kind, "label": u.label, "gall_id": u.gall_id,
                "recommend_only": u.recommend_only}

    @app.post("/api/start")
    def start(body: StartIn) -> dict[str, Any]:
        try:
            u = parse_url(body.url)
        except UrlError as err:
            raise HTTPException(400, str(err)) from err
        opts = GalleryOptions(recommend_only=body.recommend_only or u.recommend_only,
                              min_upvotes=body.min_upvotes,
                              within_hours=body.within_hours or None, max_posts=body.max_posts)
        tone = body.tone

        def run(p: Pipeline) -> None:
            p.ingest(body.url, opts)
            p.inspect_images()
            p.filter()
            p.script(tone=tone)
            p.voice()
            p.preview()

        worker.submit(f"{u.label} 수집·생성", run)
        return {"ok": True, "label": u.label}

    # --- 작업 큐 ---
    @app.get("/api/jobs")
    def jobs(limit: int = 100) -> list[dict[str, Any]]:
        s = db()
        try:
            rows = s.conn.execute(
                "SELECT j.id, j.status, j.error, j.updated_at, j.preset, p.id AS post_id, p.title, "
                "p.gall_id, p.post_no FROM jobs j JOIN posts p ON p.id=j.post_id "
                "ORDER BY j.id DESC LIMIT ?", (max(1, min(limit, 500)),)).fetchall()
            return [dict(r) for r in rows]
        finally:
            s.close()

    @app.post("/api/jobs/{job_id}/retry")
    def retry(job_id: int) -> dict[str, Any]:
        def run(p: Pipeline) -> None:
            p.store.conn.execute("UPDATE jobs SET error=NULL WHERE id=?", (job_id,))
            p.store.conn.commit()
            p.run_to_review()

        worker.submit(f"작업 {job_id} 이 단계부터 다시", run)
        return {"ok": True}

    @app.post("/api/worker/pause")
    def pause(paused: bool = True) -> dict[str, Any]:
        (worker.paused.set if paused else worker.paused.clear)()
        return {"paused": worker.paused.is_set()}

    @app.get("/api/log")
    def log() -> list[dict[str, Any]]:
        return hub.recent[-200:]

    # --- 검수 ---
    @app.get("/api/review")
    def review() -> list[dict[str, Any]]:
        s = db()
        try:
            rows = s.conn.execute(
                "SELECT j.id, j.status, j.spec_path, j.preset, p.id AS post_id, p.title, p.body, "
                "p.comments, p.gall_id, p.gall_type, p.post_no, p.gall_name, s.lines, s.overlap_ratio, s.tone, "
                "s.version FROM jobs j JOIN posts p ON p.id=j.post_id "
                "JOIN scripts s ON s.id=j.script_id WHERE j.status='preview_ready' "
                "ORDER BY j.id").fetchall()
            out = []
            for r in rows:
                filters = [dict(f) for f in s.conn.execute(
                    "SELECT stage, verdict, reasons, fit_score, model FROM filter_results "
                    "WHERE post_id=? ORDER BY id", (r["post_id"],)).fetchall()]
                for f in filters:
                    f["reasons"] = json.loads(f["reasons"] or "[]")
                spec = None
                try:
                    spec = json.loads(Path(r["spec_path"]).read_text(encoding="utf-8"))
                except (OSError, TypeError, json.JSONDecodeError):
                    pass
                script = json.loads(r["lines"] or "{}")
                out.append({
                    "job_id": r["id"], "post_id": r["post_id"], "title": r["title"],
                    "body": r["body"], "comments": json.loads(r["comments"] or "[]"),
                    "gall_name": r["gall_name"],
                    "post_url": DcUrl("post", r["gall_type"] if r["gall_type"] in GALL_TYPES
                                      else "board", r["gall_id"], int(r["post_no"])).view_url(),
                    "script": script, "overlap_ratio": r["overlap_ratio"], "tone": r["tone"],
                    "version": r["version"], "preset": r["preset"], "filters": filters,
                    "spec": spec, "files": f"/files/jobs/{r['id']}/public/",
                })
            return out
        finally:
            s.close()

    def _job_status(job_id: int) -> str:
        s = db()
        try:
            row = s.conn.execute("SELECT status FROM jobs WHERE id=?", (job_id,)).fetchone()
        finally:
            s.close()
        if row is None:
            raise HTTPException(404, "작업이 없습니다")
        return str(row["status"])

    @app.post("/api/review/{job_id}/approve")
    def approve(job_id: int) -> dict[str, Any]:
        if _job_status(job_id) != "preview_ready":
            raise HTTPException(409, "검수 대기 중인 작업이 아닙니다")
        s = db()
        try:
            s.set_job_status(job_id, "approved", None)
        finally:
            s.close()
        worker.submit(f"작업 {job_id} 최종 렌더", lambda p: p.render())
        hub.emit({"type": "jobs_changed"})
        return {"ok": True}

    @app.post("/api/review/{job_id}/edit")
    def edit(job_id: int, body: EditIn) -> dict[str, Any]:
        _job_status(job_id)

        def run(p: Pipeline) -> None:
            p.edit_script(job_id, body.lines)
            p.voice()
            p.preview()

        worker.submit(f"작업 {job_id} 수정 반영", run)
        return {"ok": True}

    @app.post("/api/review/{job_id}/regenerate")
    def regenerate(job_id: int, body: RegenIn) -> dict[str, Any]:
        _job_status(job_id)
        if not body.tone and not body.preset:
            raise HTTPException(400, "톤이나 프리셋을 골라 주세요")
        worker.submit(f"작업 {job_id} 재생성",
                      lambda p: p.regenerate(job_id, body.tone, body.preset))
        return {"ok": True}

    @app.post("/api/review/{job_id}/discard")
    def discard(job_id: int) -> dict[str, Any]:
        _job_status(job_id)
        s = db()
        try:
            Pipeline(cfg, s, data, emit=hub.emit, router=object()).discard(job_id)
        finally:
            s.close()
        return {"ok": True}

    @app.post("/api/review/{job_id}/cringe")
    def cringe(job_id: int, body: CringeIn) -> dict[str, Any]:
        """'오글' 버튼: 문장과 사유를 저장 → 스타일 화면에서 금지 목록·나쁜 예 반영을 승인 (M8)."""
        style_dir = Path(str((cfg.get("style") or {}).get("dir") or "../style"))
        rec = {"time": datetime.now().isoformat(timespec="seconds"), "job_id": job_id,
               "line": body.line, "text": body.text, "reason": body.reason}
        try:
            style_dir.mkdir(parents=True, exist_ok=True)
            with open(style_dir / "reports.jsonl", "a", encoding="utf-8") as f:
                f.write(json.dumps(rec, ensure_ascii=False) + "\n")
        except OSError as err:
            raise HTTPException(500, f"저장 실패: {err}") from err
        return {"ok": True}

    # --- 실시간 ---
    @app.websocket("/ws")
    async def ws(sock: WebSocket) -> None:
        await sock.accept()
        q: asyncio.Queue = asyncio.Queue(maxsize=500)
        hub.clients.add(q)
        try:
            while True:
                event = await q.get()
                await sock.send_text(json.dumps(event, ensure_ascii=False))
        except (WebSocketDisconnect, RuntimeError):
            pass
        finally:
            hub.clients.discard(q)

    app.mount("/files/jobs", StaticFiles(directory=str(data / "jobs")), name="jobs")
    if PANEL_DIST.is_dir():
        app.mount("/assets", StaticFiles(directory=str(PANEL_DIST / "assets")), name="assets")

        @app.get("/{path:path}", include_in_schema=False)
        def panel(path: str):
            if path.startswith("api/"):
                return JSONResponse({"detail": "없음"}, status_code=404)
            return FileResponse(PANEL_DIST / "index.html")

    return app
