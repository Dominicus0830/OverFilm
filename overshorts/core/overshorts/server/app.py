"""패널 백엔드 (기획서 3.7, M5). `overshorts serve` 로 띄우면 브라우저가 열린다.

- 긴 작업(수집→검수 대기, 최종 렌더)은 작업 스레드 하나가 차례로 돈다. 크롤링이 한 줄로만
  나가야 하고, GPU 모델도 한 번에 하나만 올려야 하기 때문이다.
- 진행 상황은 WebSocket(/ws)으로 실시간 전송.
- 미리보기는 패널이 @remotion/player 로 spec 을 그린다. 오디오·배경은 /files/jobs/<id>/public/.
"""

from __future__ import annotations

import asyncio
import json
import os
import secrets
import shutil
import threading
import traceback
from collections.abc import Callable
from contextlib import asynccontextmanager
from datetime import datetime
from pathlib import Path
from typing import Any

from fastapi import FastAPI, Header, HTTPException, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel, Field

from .. import __version__
from ..config import crawl_limits, load_config
from ..crawler.budget import CrawlGuard
from ..crawler.collector import GalleryOptions
from ..db import Store
from ..pipeline import Pipeline
from ..proc import Cancelled
from ..urls import GALL_TYPES, DcUrl, UrlError, parse_url
from .settings import settings_router

PANEL_DIST = Path(__file__).resolve().parents[3] / "panel" / "dist"


def _quiet_connection_reset(loop: asyncio.AbstractEventLoop, context: dict[str, Any]) -> None:
    """Windows 의 asyncio(Proactor)는 브라우저가 연결을 먼저 끊으면(창을 열 때 미리 열어 둔 연결을
    닫거나 새로 고칠 때) 이미 끝난 연결을 정리하다 ConnectionResetError(WinError 10054)를
    'Exception in callback _ProactorBasePipeTransport._call_connection_lost' 로 남긴다.
    요청 처리와 상관없는 정리 단계라 기능에는 영향이 없다. 서버 기록에서 오류로 오해하지 않게
    그 경우만 조용히 넘기고, 다른 예외는 그대로 기록한다."""
    err = context.get("exception")
    if isinstance(err, ConnectionResetError) and "_call_connection_lost" in repr(context.get("handle")):
        return
    loop.default_exception_handler(context)


def _stop(app: FastAPI, worker: Worker) -> None:
    """서버를 끈다. 도는 작업이 있으면 먼저 취소해 렌더·FFmpeg 같은 하위 프로그램이 남지 않게 한다."""
    if worker.current is not None:
        worker.cancel_current()
        for _ in range(50):  # 최대 약 10초 기다린다
            if worker.current is None:
                break
            threading.Event().wait(0.2)
    request_exit = getattr(app.state, "request_exit", None)
    if callable(request_exit):
        request_exit()


async def _exit_when_no_window(app: FastAPI, hub: Hub, worker: Worker,
                               grace: float = 12.0, first_wait: float = 180.0) -> None:
    """창 모드: 패널 창이 모두 닫히고 grace 초 동안 다시 붙지 않으면 서버를 끈다.
    새로 고침·잠깐 끊김은 패널이 3초 안에 다시 붙으므로 꺼지지 않는다.
    창이 한 번도 붙지 않으면(창을 못 열었으면) first_wait 초 뒤에 끈다. 숨은 서버가 남지 않게."""
    loop = asyncio.get_running_loop()
    started = last_seen = loop.time()
    seen = False
    while True:
        await asyncio.sleep(2.0)
        now = loop.time()
        if hub.clients:
            seen, last_seen = True, now
            continue
        if (seen and now - last_seen >= grace) or (not seen and now - started >= first_wait):
            await asyncio.to_thread(_stop, app, worker)
            return


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


class TaskList:
    """작업 스레드가 꺼내 쓰는 대기열. queue.Queue 와 달리 대기 중인 것을 보여 주고 뺄 수 있다
    (작업 큐 화면의 '빼기', 2026-11-21)."""

    def __init__(self) -> None:
        self._items: list[tuple[int, str, Callable[[Pipeline], None]]] = []
        self._cond = threading.Condition()
        self._next_id = 1

    def put(self, label: str, fn: Callable[[Pipeline], None]) -> int:
        with self._cond:
            task_id = self._next_id
            self._next_id += 1
            self._items.append((task_id, label, fn))
            self._cond.notify()
            return task_id

    def get(self) -> tuple[int, str, Callable[[Pipeline], None]]:
        with self._cond:
            while not self._items:
                self._cond.wait()
            return self._items.pop(0)

    def remove(self, task_id: int) -> bool:
        with self._cond:
            before = len(self._items)
            self._items = [t for t in self._items if t[0] != task_id]
            return len(self._items) != before

    def clear(self) -> int:
        with self._cond:
            n = len(self._items)
            self._items = []
            return n

    def snapshot(self) -> list[dict[str, Any]]:
        with self._cond:
            return [{"id": t[0], "label": t[1]} for t in self._items]

    def qsize(self) -> int:
        with self._cond:
            return len(self._items)


class Worker:
    def __init__(self, cfg: dict[str, Any], db_path: Path, data: Path, hub: Hub,
                 pipeline_factory: Callable[[Store], Pipeline] | None = None):
        self.cfg, self.db_path, self.data, self.hub = cfg, db_path, data, hub
        self.tasks = TaskList()
        self.current: str | None = None
        self.current_id: int | None = None
        self.paused = threading.Event()
        self.cancel = threading.Event()  # 지금 도는 작업 취소 (proc.bind_cancel)
        self.factory = pipeline_factory
        self.thread = threading.Thread(target=self._run, name="overshorts-worker", daemon=True)

    def start(self) -> None:
        self.thread.start()

    def _queue_event(self) -> None:
        self.hub.emit({"type": "queue", "pending": self.tasks.qsize(), "current": self.current})

    def submit(self, label: str, fn: Callable[[Pipeline], None]) -> int:
        task_id = self.tasks.put(label, fn)
        self._queue_event()
        return task_id

    def state(self) -> dict[str, Any]:
        return {"current": ({"id": self.current_id, "label": self.current}
                            if self.current is not None else None),
                "pending": self.tasks.snapshot(), "paused": self.paused.is_set(),
                "cancelling": self.cancel.is_set() and self.current is not None}

    def remove(self, task_id: int) -> bool:
        """아직 시작하지 않은 작업을 대기열에서 뺀다."""
        ok = self.tasks.remove(task_id)
        if ok:
            self._queue_event()
        return ok

    def cancel_current(self, task_id: int | None = None) -> bool:
        """지금 도는 작업을 멈춘다. 하위 프로그램(렌더·Claude Code·FFmpeg)은 자식까지 끄고,
        단계는 다음 확인 지점에서 멈춘다. 이미 끝난 단계의 결과는 남으므로 '이 단계부터 다시'로 이어 간다."""
        if self.current is None or (task_id is not None and task_id != self.current_id):
            return False
        self.cancel.set()
        self.hub.emit({"type": "task", "state": "cancelling", "label": self.current})
        return True

    def _run(self) -> None:
        from ..proc import bind_cancel

        bind_cancel(self.cancel)
        store = Store(self.db_path)
        pipe = (self.factory(store) if self.factory
                else Pipeline(self.cfg, store, self.data, emit=self.hub.emit))
        while True:
            task_id, label, fn = self.tasks.get()
            while self.paused.is_set():  # 일시정지: 다음 작업을 꺼내지 않고 기다린다
                threading.Event().wait(1.0)
            self.cancel.clear()
            self.current, self.current_id = label, task_id
            self.hub.emit({"type": "task", "state": "start", "label": label})
            try:
                fn(pipe)
                self.hub.emit({"type": "task", "state": "done", "label": label})
            except Cancelled:
                self.hub.emit({"type": "task", "state": "cancelled", "label": label,
                               "message": "취소했습니다"})
            except Exception as err:  # noqa: BLE001 - 작업 하나가 실패해도 서버는 계속 돈다
                self.hub.emit({"type": "task", "state": "failed", "label": label,
                               "message": str(err)[:500],
                               "detail": traceback.format_exc(limit=3)[-1500:]})
            finally:
                try:  # 취소가 DB 쓰기 도중이었으면 반쯤 쓴 것을 버린다
                    store.conn.rollback()
                except Exception:  # noqa: BLE001
                    pass
                self.cancel.clear()
                self.current, self.current_id = None, None
                self.hub.emit({"type": "jobs_changed"})
                self._queue_event()


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
    confirm: bool = False  # 갤러리 크롤링 확인창에서 '계속'
    remember: bool = False  # '다시 묻지 않기'


class LimitIn(BaseModel):
    action: str = Field(pattern="^(switch|wait)$")
    remember: bool = False


class SegTtsIn(BaseModel):
    seg: int = Field(ge=0, le=20)
    tts: bool


class SourceIn(BaseModel):
    show: bool


class ReactionIn(BaseModel):
    after: int = Field(ge=-1, le=20, description="이 세그먼트 뒤에 넣는다 (-1 = 맨 앞)")
    image: str = Field(default="", max_length=70, description="'영웅:이름' / '짤:이름' / '검색:찾을 짤'")
    say: str = Field(default="", max_length=40)
    say_style: str = Field(default="calm", pattern="^(calm|shout|question|sad|smug)$")


class MemeIn(BaseModel):
    name: str = Field(min_length=1, max_length=40)
    use: str = Field(default="", max_length=80, description="언제 쓰는 짤인지 (AI 가 참고)")
    data: str = Field(max_length=12_000_000, description="그림 파일 base64")


class EditIn(BaseModel):
    lines: list[dict[str, Any]] = Field(max_length=80)


class RegenIn(BaseModel):
    tone: str | None = None
    preset: str | None = None
    feedback: str | None = Field(default=None, max_length=1000)  # 피드백을 넣고 대본 다시 쓰기 (S5)
    ab: bool = False  # A안·B안 두 버전


class VariantIn(BaseModel):
    script_id: int = Field(ge=1)


class PresetChatIn(BaseModel):
    messages: list[dict[str, Any]] = Field(max_length=30)
    references: list[str] = Field(default_factory=list, max_length=5)  # 예시 영상 링크·파일 이름 (선택)
    current: dict[str, Any] | None = None


class PresetSaveIn(BaseModel):
    preset: dict[str, Any]
    references: list[str] = Field(default_factory=list, max_length=5)


class PresetCopyIn(BaseModel):
    label: str = Field(default="", max_length=20)


class PremiereFileIn(BaseModel):
    name: str = Field(max_length=200)
    data: str = Field(max_length=90_000_000, description="파일 base64 (64MB 까지)")


class PresetLearnIn(BaseModel):
    base: str | None = Field(default=None, max_length=32)  # 보충할 유형 id (없으면 새 유형)
    premiere: PremiereFileIn | None = None
    jobs: list[int] = Field(default_factory=list, max_length=10)  # 통과된 쇼츠 작업 번호
    links: list[str] = Field(default_factory=list, max_length=5)  # 유튜브 링크
    note: str = Field(default="", max_length=1000)


class CringeIn(BaseModel):
    line: int = Field(ge=0, le=100)
    text: str = Field(max_length=300)
    reason: str = Field(default="", max_length=300)


def create_app(config_path: str | None = None, cfg: dict[str, Any] | None = None,
               start_worker: bool = True,
               pipeline_factory: Callable[[Store], Pipeline] | None = None,
               idle_exit: bool = False, shutdown_token: str | None = None) -> FastAPI:
    """idle_exit: 창 모드(`serve --window`). 패널 창이 모두 닫히면(WebSocket 이 다 끊기면) 서버를 끈다.
    shutdown_token: 새 버전 실행기가 이 서버를 끌 때 쓰는 토큰 (/api/shutdown)."""
    cfg = cfg or load_config(config_path)
    data = Path(str(cfg.get("data_dir") or "./data"))
    db_path = data / "overshorts.db"
    Store(db_path).close()  # 스키마 만들기
    hub = Hub()
    worker = Worker(cfg, db_path, data, hub, pipeline_factory)
    @asynccontextmanager
    async def lifespan(_app: FastAPI):
        hub.loop = asyncio.get_running_loop()
        hub.loop.set_exception_handler(_quiet_connection_reset)
        if start_worker:
            worker.start()
        watch = asyncio.create_task(_exit_when_no_window(_app, hub, worker)) if idle_exit else None
        yield
        if watch is not None:
            watch.cancel()

    app = FastAPI(title="OverShorts 패널", lifespan=lifespan)
    app.state.worker = worker
    app.state.hub = hub
    (data / "jobs").mkdir(parents=True, exist_ok=True)
    from .voices import voices_router  # 보이스 화면 (M6 클로닝)

    app.include_router(voices_router(cfg, data, db_path, worker))

    def db() -> Store:
        return Store(db_path)

    app.include_router(settings_router(cfg, config_path, data, worker.submit))

    # --- 창 모드 (window.py) ---
    @app.get("/api/hello")
    def hello() -> dict[str, Any]:
        """실행기가 '이미 켜진 OverShorts 인지, 같은 빌드인지' 알아보는 가벼운 응답."""
        from ..window import build_id

        build = app.state.__dict__.setdefault("build", build_id())
        return {"app": "overshorts", "version": __version__, "build": build,
                "window": idle_exit, "busy": worker.current is not None or worker.tasks.qsize() > 0}

    @app.post("/api/shutdown")
    def shutdown(x_overshorts_token: str = Header(default="")) -> dict[str, Any]:
        """새 버전 실행기가 예전 서버를 끈다. 토큰은 데이터 폴더의 .server.json 에만 있어
        다른 웹 페이지는 부를 수 없다 (사용자 정의 헤더라 브라우저가 다른 사이트의 요청을 막는다)."""
        if not shutdown_token or not secrets.compare_digest(x_overshorts_token, shutdown_token):
            raise HTTPException(403, "끌 수 없습니다")
        threading.Thread(target=_stop, args=(app, worker), name="overshorts-stop", daemon=True).start()
        return {"ok": True}

    @app.get("/api/server-log")
    def server_log() -> dict[str, Any]:
        """창 모드는 콘솔이 없어서 서버 출력이 파일로 간다 (launch.py 가 OVERSHORTS_LOG 로 알려 준다)."""
        path = os.environ.get("OVERSHORTS_LOG") or ""
        if not path or not Path(path).is_file():
            return {"path": path or None, "lines": []}
        try:
            with open(path, "rb") as f:
                f.seek(0, os.SEEK_END)
                f.seek(max(0, f.tell() - 64_000))
                text = f.read().decode("utf-8", "replace")
        except OSError as err:
            raise HTTPException(500, f"로그를 읽지 못했습니다: {err}") from err
        return {"path": path, "lines": text.splitlines()[-300:]}

    @app.post("/api/server-log/open")
    def open_server_log_folder() -> dict[str, Any]:
        path = os.environ.get("OVERSHORTS_LOG") or ""
        folder = Path(path).parent if path else None
        if folder is None or not folder.is_dir() or not hasattr(os, "startfile"):
            raise HTTPException(404, "로그 폴더가 없습니다")
        try:
            os.startfile(str(folder))  # type: ignore[attr-defined]  # noqa: S606 - 윈도우 탐색기로 연다
        except OSError as err:
            raise HTTPException(500, f"폴더를 열지 못했습니다: {err}") from err
        return {"ok": True}

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
        if u.kind != "post":
            # 기획서 3.7-8: 갤러리 전체 크롤링은 예상 요청 수와 남은 예산을 보여 주고 '계속'을 받는다
            prefs = _prefs()
            if body.remember:
                _save_pref("skip_crawl_confirm", True)
            elif not body.confirm and not prefs.get("skip_crawl_confirm"):
                s = db()
                try:
                    st = CrawlGuard(s, crawl_limits(cfg)).status()
                finally:
                    s.close()
                crawler = cfg.get("crawler") or {}
                per_post = 2 + int(crawler.get("max_images") or 6)
                return {"ok": False, "confirm": {
                    "requests_min": 1 + body.max_posts * 2,
                    "requests_max": 3 + body.max_posts * per_post,
                    "hour_left": max(0, st["hour_limit"] - st["hour_used"]),
                    "day_left": max(0, st["day_limit"] - st["day_used"]),
                    "cooldown_until": st.get("cooldown_until"),
                }}

        def run(p: Pipeline) -> None:
            p.ingest(body.url, opts)
            p.inspect_images()
            p.filter()
            p.script(tone=tone)
            p.voice()
            p.preview()

        worker.submit(f"{u.label} 수집·생성", run)
        return {"ok": True, "label": u.label}

    def _prefs() -> dict[str, Any]:
        try:
            got = json.loads((data / "panel_prefs.json").read_text(encoding="utf-8"))
            return got if isinstance(got, dict) else {}
        except (OSError, json.JSONDecodeError):
            return {}

    def _save_pref(key: str, value: Any) -> None:
        prefs = _prefs()
        prefs[key] = value
        (data / "panel_prefs.json").write_text(json.dumps(prefs, ensure_ascii=False), encoding="utf-8")

    @app.get("/api/prefs")
    def prefs() -> dict[str, Any]:
        return _prefs()

    @app.post("/api/prefs/source")
    def prefs_source(body: SourceIn) -> dict[str, Any]:
        """영상 아래 '출처: DC인사이드 …' 켜기/끄기. 이미 만든 미리보기도 바로 바뀐다."""
        _save_pref("show_source", bool(body.show))
        worker.submit("출처 표시 " + ("켬" if body.show else "끔"), lambda p: p.set_show_source(body.show))
        return {"ok": True}

    @app.post("/api/prefs/reset")
    def prefs_reset() -> dict[str, Any]:
        """'다시 묻지 않기'를 되돌린다 (설정 화면)."""
        try:
            (data / "panel_prefs.json").unlink()
        except OSError:
            pass
        return {"ok": True}

    @app.post("/api/llm/limit")
    def llm_limit(body: LimitIn) -> dict[str, Any]:
        """Claude 한도 확인창: 전환 = 로컬 LLM 으로 대본을 이어서 씀, 대기 = 대기열에 그대로."""
        mode = "fallback" if body.action == "switch" else "wait"
        if body.remember:
            _save_pref("on_limit", mode)

        def run(p: Pipeline) -> None:
            router = p.router()
            before = router.on_limit
            router.on_limit = mode
            try:
                if mode == "fallback":
                    p.script()
                    p.voice()
                    p.preview()
            finally:
                if not body.remember:
                    router.on_limit = before

        if mode == "fallback" or body.remember:
            worker.submit("로컬 LLM 으로 대본 이어 쓰기" if mode == "fallback" else "한도 대기 설정", run)
        return {"ok": True, "mode": mode}

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

    @app.post("/api/jobs/{job_id}/force")
    def force(job_id: int) -> dict[str, Any]:
        """'막힘' 작업을 그래도 만든다. 규칙에서 막혔으면 LLM 필터부터, LLM 필터에서 막혔으면 대본부터."""
        s = db()
        try:
            row = s.conn.execute("SELECT status, error, post_id FROM jobs WHERE id=?", (job_id,)).fetchone()
            if row is None:
                raise HTTPException(404, "작업을 찾지 못했습니다")
            if row["status"] != "blocked":
                raise HTTPException(409, "막힌 작업이 아닙니다")
            to = "crawled" if str(row["error"] or "").startswith("규칙:") else "filtered"
            s.conn.execute("UPDATE posts SET discarded=0 WHERE id=?", (row["post_id"],))
            s.set_job_status(job_id, to, None)
            s.add_filter_result(int(row["post_id"]), "human", "pass", ["사람이 그래도 만들기"])
        finally:
            s.close()
        worker.submit(f"작업 {job_id} 그래도 만들기", lambda p: p.run_to_review())
        return {"ok": True}

    @app.delete("/api/jobs/{job_id}")
    def delete_job(job_id: int) -> dict[str, Any]:
        """작업 큐에서 지운다 (작업 폴더의 중간 파일도). 완성된 mp4 는 남긴다.
        같은 글은 나중에 다시 넣으면 새 작업으로 만들어진다 ('폐기'와 다름)."""
        s = db()
        try:
            row = s.conn.execute("SELECT id FROM jobs WHERE id=?", (job_id,)).fetchone()
            if row is None:
                raise HTTPException(404, "작업을 찾지 못했습니다")
            s.conn.execute("DELETE FROM jobs WHERE id=?", (job_id,))
            s.conn.commit()
        finally:
            s.close()
        # 지금 도는 작업이 이 글을 만지고 있으면 멈춘다 (단계는 여러 글을 한꺼번에 돌기도 한다)
        cancelled = bool(worker.current and f"작업 {job_id} " in f"{worker.current} " and
                         worker.cancel_current())
        job_dir = (data / "jobs" / str(int(job_id))).resolve()
        if job_dir.parent == (data / "jobs").resolve():
            shutil.rmtree(job_dir, ignore_errors=True)
        hub.emit({"type": "jobs_changed"})
        return {"ok": True, "cancelled": cancelled}

    @app.get("/api/worker")
    def worker_state() -> dict[str, Any]:
        return worker.state()

    @app.post("/api/worker/cancel")
    def worker_cancel(task_id: int | None = None) -> dict[str, Any]:
        """지금 도는 작업 취소."""
        return {"ok": worker.cancel_current(task_id), **worker.state()}

    @app.delete("/api/worker/tasks/{task_id}")
    def worker_remove(task_id: int) -> dict[str, Any]:
        """아직 시작하지 않은 작업을 대기열에서 뺀다."""
        if not worker.remove(task_id):
            raise HTTPException(404, "대기열에 없는 작업입니다 (이미 시작했거나 끝났습니다)")
        return {"ok": True, **worker.state()}

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

    def _style_dir() -> Path:
        return Path(str((cfg.get("style") or {}).get("dir") or "../style"))

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

    @app.post("/api/review/{job_id}/overfilm")
    def to_overfilm(job_id: int) -> dict[str, Any]:
        """OverFilm 편집기로 넘기기: 장면 파일을 만들고 편집기를 띄운다 (타임라인에서 사람이 더 고침)."""
        if _job_status(job_id) not in ("preview_ready", "approved", "rendered"):
            raise HTTPException(409, "미리보기가 준비된 작업만 넘길 수 있습니다")

        def run(p: Pipeline) -> None:
            out = p.export_overfilm(job_id)
            hub.emit({"type": "overfilm", "job_id": job_id, **out})

        worker.submit(f"작업 {job_id} OverFilm 편집기로", run)
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

    @app.post("/api/review/{job_id}/segment-tts")
    def segment_tts(job_id: int, body: SegTtsIn) -> dict[str, Any]:
        """세그먼트별 TTS 켜기/끄기 (기획서 3.3 개정 6) → 음성·미리보기 다시."""
        _job_status(job_id)
        worker.submit(f"작업 {job_id} 세그먼트 {body.seg + 1} TTS "
                      + ("켬" if body.tts else "끔"),
                      lambda p: p.set_segment_tts(job_id, body.seg, body.tts))
        return {"ok": True}

    @app.post("/api/review/{job_id}/reaction")
    def add_reaction(job_id: int, body: ReactionIn) -> dict[str, Any]:
        """짤 장면 넣기 (2026-11-15 사용자 편집 규칙) → 음성·미리보기 다시."""
        _job_status(job_id)
        worker.submit(f"작업 {job_id} 짤 넣기", lambda p: p.add_reaction(job_id, body.after, body.image, body.say,
                                                                     body.say_style))
        return {"ok": True}

    @app.delete("/api/review/{job_id}/segment/{seg}")
    def remove_segment(job_id: int, seg: int) -> dict[str, Any]:
        _job_status(job_id)
        worker.submit(f"작업 {job_id} 짤 빼기", lambda p: p.remove_segment(job_id, seg))
        return {"ok": True}

    @app.get("/api/memes")
    def memes() -> dict[str, Any]:
        from ..script.game_check import load_heroes
        from ..script.reactions import list_memes

        return {"memes": [{"name": m.name, "use": m.use} for m in list_memes(_style_dir())],
                "heroes": [h.ko for h in load_heroes()]}

    @app.post("/api/memes")
    def add_meme(body: MemeIn) -> dict[str, Any]:
        """짤 모음에 그림 추가 (style/memes/). 이름이 같으면 덮어쓴다."""
        import base64
        import binascii

        from ..script.reactions import MAX_IMAGE_BYTES, _safe, image_ext, memes_dir

        try:
            data = base64.b64decode(body.data.split(",")[-1], validate=True)
        except (binascii.Error, ValueError) as err:
            raise HTTPException(400, "그림을 읽지 못했습니다") from err
        ext = image_ext(data)
        if ext is None or len(data) > MAX_IMAGE_BYTES:
            raise HTTPException(400, "png·jpg·webp·gif 그림만 (8MB 까지)")
        d = memes_dir(_style_dir())
        if d is None:
            raise HTTPException(500, "스타일 폴더가 없습니다")
        d.mkdir(parents=True, exist_ok=True)
        name = _safe(body.name)
        for old in d.glob(f"{name}.*"):
            if old.suffix.lower() in (".png", ".jpg", ".jpeg", ".webp", ".gif"):
                old.unlink(missing_ok=True)
        (d / f"{name}{ext}").write_bytes(data)
        if body.use.strip():
            with open(d / "memes.tsv", "a", encoding="utf-8") as f:
                f.write(f"{name}\t{body.use.strip()}\n")
        return {"ok": True, "name": name}

    @app.post("/api/review/{job_id}/regenerate")
    def regenerate(job_id: int, body: RegenIn) -> dict[str, Any]:
        _job_status(job_id)
        feedback = (body.feedback or "").strip() or None
        if not body.tone and not body.preset and not feedback and not body.ab:
            raise HTTPException(400, "톤·프리셋을 고르거나 피드백을 적어 주세요")
        worker.submit(f"작업 {job_id} 재생성",
                      lambda p: p.regenerate(job_id, body.tone, body.preset, feedback, body.ab))
        return {"ok": True}

    @app.post("/api/review/{job_id}/variant")
    def variant(job_id: int, body: VariantIn) -> dict[str, Any]:
        """A안·B안 중 다른 안으로 바꾼다 (음성·미리보기를 다시 만든다)."""
        _job_status(job_id)
        worker.submit(f"작업 {job_id} 대본 안 바꾸기", lambda p: p.pick_variant(job_id, body.script_id))
        return {"ok": True}

    # --- 편집 유형 (S17·S18) ---
    @app.get("/api/presets")
    def presets_list() -> list[dict[str, Any]]:
        from ..presets import load_presets
        from ..presets_custom import summarize

        return [{"id": p.id, "label": p.label, "origin": p.origin, "summary": summarize(p)}
                for p in load_presets(_style_dir()).values()]

    @app.post("/api/presets/chat")
    def presets_chat(body: PresetChatIn) -> dict[str, Any]:
        """Claude Code 와 대화로 커스텀 유형 만들기. 저장은 하지 않고 요약만 돌려준다."""
        from ..llm import LlmError
        from ..presets_custom import chat

        s = db()
        try:
            router = (pipeline_factory(s) if pipeline_factory else Pipeline(cfg, s, data)).router()
            try:
                return {"ok": True, **chat(router, body.messages, body.references, body.current)}
            except (LlmError, ValueError) as err:
                return {"ok": False, "error": str(err)[:300]}
        finally:
            s.close()

    @app.post("/api/presets/save")
    def presets_save(body: PresetSaveIn) -> dict[str, Any]:
        from ..presets_custom import save_custom_preset, summarize

        try:
            p = save_custom_preset(_style_dir(), body.preset, body.references)
        except (ValueError, OSError) as err:
            raise HTTPException(400, str(err)[:300]) from err
        return {"ok": True, "id": p.id, "summary": summarize(p)}

    @app.delete("/api/presets/{preset_id}")
    def presets_delete(preset_id: str) -> dict[str, Any]:
        from ..presets_custom import delete_custom_preset

        if not delete_custom_preset(_style_dir(), preset_id):
            raise HTTPException(404, "내가 만든 유형이 아닙니다")
        return {"ok": True}

    @app.post("/api/presets/{preset_id}/copy")
    def presets_copy(preset_id: str, body: PresetCopyIn) -> dict[str, Any]:
        """스타일 복사: 새 이름의 내 유형으로 복제해 바로 저장한다."""
        from ..presets_custom import summarize
        from ..presets_learn import copy_preset

        try:
            p = copy_preset(_style_dir(), preset_id, body.label)
        except (ValueError, OSError) as err:
            raise HTTPException(400, str(err)[:300]) from err
        return {"ok": True, "id": p.id, "label": p.label, "summary": summarize(p)}

    @app.get("/api/presets/passed")
    def presets_passed() -> list[dict[str, Any]]:
        """학습에 쓸 수 있는 통과된 쇼츠 목록."""
        from ..presets_learn import passed_shorts

        s = db()
        try:
            return passed_shorts(s)
        finally:
            s.close()

    @app.post("/api/presets/learn")
    def presets_learn(body: PresetLearnIn) -> dict[str, Any]:
        """Premiere 파일·통과된 쇼츠·유튜브 링크로 스타일 학습 (보충 또는 새 유형). 저장은 하지 않는다."""
        import base64
        import binascii

        from ..llm import LlmError
        from ..presets_learn import learn, premiere_facts

        facts = None
        if body.premiere is not None:
            try:
                raw = base64.b64decode(body.premiere.data.split(",", 1)[-1], validate=False)
                facts = premiere_facts(raw, body.premiere.name)
            except (ValueError, binascii.Error) as err:
                return {"ok": False, "error": str(err)[:300]}
        s = db()
        try:
            router = (pipeline_factory(s) if pipeline_factory else Pipeline(cfg, s, data)).router()
            try:
                return {"ok": True, "premiere": facts,
                        **learn(router, s, _style_dir(), body.base, facts, body.jobs, body.links, body.note)}
            except (LlmError, ValueError) as err:
                return {"ok": False, "error": str(err)[:300]}
        finally:
            s.close()

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

        async def pump() -> None:
            while True:
                event = await q.get()
                await sock.send_text(json.dumps(event, ensure_ascii=False))

        sender = asyncio.create_task(pump())
        try:
            # 받는 쪽도 돌려야 창이 닫힌 것(연결 끊김)을 바로 안다. 보내기만 하면 다음 이벤트까지 모른다.
            while not sender.done():
                await sock.receive_text()
        except (WebSocketDisconnect, RuntimeError):
            pass
        finally:
            sender.cancel()
            hub.clients.discard(q)

    app.mount("/files/jobs", StaticFiles(directory=str(data / "jobs")), name="jobs")
    if PANEL_DIST.is_dir():
        app.mount("/assets", StaticFiles(directory=str(PANEL_DIST / "assets")), name="assets")

        @app.get("/{path:path}", include_in_schema=False)
        def panel(path: str):
            if path.startswith("api/"):
                return JSONResponse({"detail": "없음"}, status_code=404)
            # 창 아이콘·manifest 같은 dist 맨 위 파일 (panel/public). 하위 폴더나 '..' 는 받지 않는다.
            if path and "/" not in path and "\\" not in path and not path.startswith("."):
                top = PANEL_DIST / path
                if top.is_file() and top.parent == PANEL_DIST:
                    return FileResponse(top)
            # 새 버전을 설치하면 바로 새 화면이 뜨게 (브라우저가 예전 index.html 을 쓰지 않게)
            return FileResponse(PANEL_DIST / "index.html", headers={"Cache-Control": "no-cache"})

    return app
