"""파이프라인: 링크 → 검수 대기까지 단계를 차례로 돈다 (기획서 4 도식 1~6단계).

패널(작업 큐)·CLI·나중의 MCP 도구(shorts_ingest)가 모두 이것을 부른다.
같은 단계의 글을 모아 처리하고 단계가 바뀔 때 GPU 모델을 내린다 (VRAM 스케줄러).
각 단계는 결과를 DB·작업 폴더에 남기므로, 실패하면 그 단계부터 다시 돈다.
"""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

from .config import crawl_limits, rule_config
from .crawler.budget import CrawlGuard
from .crawler.collector import Collector, GalleryOptions
from .crawler.fetcher import Fetcher
from .db import Store

Emit = Callable[[dict[str, Any]], None]

# 실패한 작업을 '이 단계부터 다시' 돌릴 때 되돌릴 상태
RETRY_FROM = {"crawled": "crawled", "filtered": "filtered", "scripted": "scripted",
              "aligned": "aligned", "approved": "approved"}


class Pipeline:
    def __init__(self, cfg: dict[str, Any], store: Store, data_dir: Path,
                 emit: Emit | None = None, fetcher: Fetcher | None = None,
                 router=None):
        self.cfg = cfg
        self.store = store
        self.data = data_dir
        self.emit = emit or (lambda e: None)
        self.guard = CrawlGuard(store, crawl_limits(cfg))
        crawler = cfg.get("crawler") or {}
        self.fetcher = fetcher or Fetcher(self.guard, proxy=crawler.get("proxy"))
        self._router = router

    def _say(self, stage: str, msg: str, **extra: Any) -> None:
        self.emit({"type": "progress", "stage": stage, "message": msg, **extra})

    def router(self):
        if self._router is None:
            from .cli import build_router

            self._router, _ = build_router(self.cfg)
        return self._router

    # --- 단계 ---
    def ingest(self, url: str, opts: GalleryOptions) -> dict[str, Any]:
        crawler = self.cfg.get("crawler") or {}
        col = Collector(self.store, self.fetcher, rule_config(self.cfg),
                        cache_dir=self.data / "cache",
                        max_comments=int(crawler.get("max_comments") or 50),
                        image_dir=self.data / "images",
                        max_images=int(crawler.get("max_images") or 5))
        self._say("crawl", "수집 중")
        rep = col.ingest(url, opts)
        if rep.stopped:
            self.emit({"type": "alert", "level": "warn", "message": rep.stopped})
        self._say("crawl", rep.summary(), saved=rep.saved)
        return {"saved": rep.saved, "blocked": len(rep.blocked), "stopped": rep.stopped,
                "errors": rep.errors, "summary": rep.summary()}

    def inspect_images(self) -> None:
        from .images import ImageInspector, inspect_pending

        f = self.cfg.get("filter") or {}
        insp = ImageInspector(self.data / "models", blur_faces=bool(f.get("blur_faces", True)),
                              drop_nsfw=bool(f.get("drop_nsfw_images", True)))
        try:
            counts = inspect_pending(self.store, insp, self.data / "images")
        except ImportError:
            return
        if counts["posts"]:
            self._say("images", f"그림 {counts['images']}장 검사, 얼굴 흐림 {counts['faces']}장")

    def filter(self) -> None:
        from .filters.llm_filter import filter_pending

        f = self.cfg.get("filter") or {}
        self._say("filter", "LLM 필터 중")
        c = filter_pending(self.store, self.router(), list(f.get("block_categories") or []),
                           [str(t) for t in f.get("allow_topics") or []])
        self._unload_ollama()
        self._say("filter", f"통과 {c['pass']}, 검토 필요 {c['review']}, 막힘 {c['block']}")

    def script(self, tone: str | None = None, post_ids: list[int] | None = None) -> None:
        from .script.writer import ScriptSettings, script_pending

        render = self.cfg.get("render") or {}
        style = self.cfg.get("style") or {}
        settings = ScriptSettings(style_dir=style.get("dir") or "../style",
                                  tone=tone or str(render.get("tone") or "담백"),
                                  speech=str(style.get("speech") or "반말"),
                                  min_cringe_score=float(style.get("min_cringe_score") or 7))
        self._say("script", "대본 쓰는 중 (Claude)")
        c = script_pending(self.store, self.router(), settings, post_ids)
        self._say("script", f"대본 {c['scripted']}개, 실패 {c['failed']}")

    def voice(self) -> None:
        from .tts.stage import voice_pending

        v = self.cfg.get("voice") or {}
        self._say("voice", "음성 만드는 중")
        c = voice_pending(self.store, self.data / "jobs",
                          str(v.get("narrator") or "edge:ko-KR-SunHiNeural"),
                          str(v.get("comment") or "") or None)
        self._say("voice", f"음성 {c['voiced']}개, 실패 {c['failed']}")

    def preview(self) -> None:
        from .render.spec import spec_pending

        bg = (self.cfg.get("render") or {}).get("background_dir")
        c = spec_pending(self.store, self.data / "jobs", Path(str(bg)) if bg else None)
        self._say("preview", f"검수 대기 {c['ready']}개")

    def run_to_review(self) -> None:
        """밀린 단계를 모두 돌려 검수 대기까지."""
        self.inspect_images()
        self.filter()
        self.script()
        self.voice()
        self.preview()
        self.emit({"type": "jobs_changed"})

    def render(self) -> None:
        from .render.runner import render_approved

        r = self.cfg.get("render") or {}
        self._say("render", "최종 렌더 중")
        c = render_approved(self.store, Path(str(r.get("out_dir") or self.data / "approved")),
                            str(self.cfg.get("encoder") or "auto"), r.get("browser_executable"))
        self._say("render", f"렌더 {c['rendered']}개, 실패 {c['failed']}")
        self.emit({"type": "jobs_changed"})

    def _unload_ollama(self) -> None:
        for client in {id(c): c for c in getattr(self._router, "clients", {}).values()}.values():
            unload = getattr(client, "unload", None)
            if unload:
                unload()

    # --- 검수 동작 ---
    def edit_script(self, job_id: int, lines: list[dict[str, Any]]) -> None:
        """대본 수정 → 고친 대본으로 TTS 부터 다시."""
        from .script.checks import check_script, load_banned

        row = self.store.conn.execute(
            "SELECT j.post_id, s.lines, s.tone, p.body, p.comments FROM jobs j "
            "JOIN scripts s ON s.id=j.script_id JOIN posts p ON p.id=j.post_id WHERE j.id=?",
            (job_id,)).fetchone()
        if row is None:
            raise ValueError("작업을 찾지 못했습니다")
        clean = [{"part": str(ln.get("part") or "body"), "text": str(ln.get("text") or "")[:200],
                  "comment_no": ln.get("comment_no")}
                 for ln in lines[:40] if str(ln.get("text") or "").strip()]
        if not clean:
            raise ValueError("대본이 비었습니다")
        payload = json.loads(row["lines"] or "{}")
        comments = [str(c.get("text", "")) for c in json.loads(row["comments"] or "[]")
                    if isinstance(c, dict)]
        style = self.cfg.get("style") or {}
        checks = check_script(clean, row["body"], comments, load_banned(style.get("dir") or "../style"))
        payload.update(lines=clean, warnings=checks.reasons(), est_seconds=checks.est_seconds,
                       edited=True)
        version = self.store.next_script_version(row["post_id"])
        sid = self.store.add_script(row["post_id"], version, row["tone"], payload,
                                    checks.overlap_ratio, "사람")
        self.store.set_job_status(job_id, "scripted", None, script_id=sid)

    def regenerate(self, job_id: int, tone: str | None, preset: str | None) -> None:
        """톤을 바꾸면 대본부터, 프리셋만 바꾸면 렌더 설정만 다시 (3.6)."""
        row = self.store.conn.execute("SELECT post_id FROM jobs WHERE id=?", (job_id,)).fetchone()
        if row is None:
            raise ValueError("작업을 찾지 못했습니다")
        if tone:
            self.store.set_job_status(job_id, "filtered", None)
            self.script(tone=tone, post_ids=[int(row["post_id"])])
            self.voice()
            self.preview()
        elif preset:
            self.store.conn.execute("UPDATE jobs SET preset=? WHERE id=?", (preset, job_id))
            self.store.set_job_status(job_id, "aligned", None)
            self.preview()
        self.emit({"type": "jobs_changed"})

    def discard(self, job_id: int) -> None:
        """폐기: 같은 글을 다시 수집하지 않는다."""
        row = self.store.conn.execute("SELECT post_id FROM jobs WHERE id=?", (job_id,)).fetchone()
        if row is None:
            raise ValueError("작업을 찾지 못했습니다")
        self.store.discard_post(int(row["post_id"]))
        self.store.set_job_status(job_id, "rejected", "폐기")
        self.emit({"type": "jobs_changed"})
