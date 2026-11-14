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

from .config import crawl_limits, rule_config, script_settings, spec_options
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

            self._router, _ = build_router(self.cfg, ask=True)
            pref = self.prefs().get("on_limit")
            if pref in ("fallback", "wait"):  # 확인창에서 '다시 묻지 않기'
                self._router.on_limit = pref
        return self._router

    def prefs(self) -> dict[str, Any]:
        try:
            data = json.loads((self.data / "panel_prefs.json").read_text(encoding="utf-8"))
            return data if isinstance(data, dict) else {}
        except (OSError, json.JSONDecodeError):
            return {}

    def save_pref(self, key: str, value: Any) -> None:
        prefs = self.prefs()
        prefs[key] = value
        self.data.mkdir(parents=True, exist_ok=True)
        (self.data / "panel_prefs.json").write_text(json.dumps(prefs, ensure_ascii=False),
                                                     encoding="utf-8")

    # --- 단계 ---
    def ingest(self, url: str, opts: GalleryOptions) -> dict[str, Any]:
        crawler = self.cfg.get("crawler") or {}
        col = Collector(self.store, self.fetcher, rule_config(self.cfg),
                        cache_dir=self.data / "cache",
                        max_comments=int(crawler.get("max_comments") or 50),
                        image_dir=self.data / "images",
                        max_images=int(crawler.get("max_images") or 6))
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
        from .script.writer import script_pending

        settings = script_settings(self.cfg, tone)
        self._say("script", "대본 쓰는 중 (Claude)")
        c = script_pending(self.store, self.router(), settings, post_ids)
        self._say("script", f"대본 {c['scripted']}개, 실패 {c['failed']}")
        if c.get("limited"):
            # 기획서 3.7-8: Claude 한도 → 로컬 LLM 으로 바꿀지 묻는다 (품질이 낮아짐)
            self.emit({"type": "confirm", "kind": "llm_limit",
                       "message": "Claude 사용 한도에 걸렸습니다. 로컬 LLM 으로 바꾸면 대본 품질이 "
                                  "낮아집니다. 바꿀까요, 한도가 풀릴 때까지 기다릴까요?"})

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
        c = spec_pending(self.store, self.data / "jobs", Path(str(bg)) if bg else None,
                         opts=spec_options(self.cfg))
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
        """대본 수정 → 고친 대본으로 TTS 부터 다시. 세그먼트(pause 줄·TTS 켜기/끄기)는 그대로 둔다."""
        from .script.checks import check_script, load_banned
        from .script.segments import with_pauses

        row = self.store.conn.execute(
            "SELECT j.post_id, s.lines, s.tone, p.body, p.comments FROM jobs j "
            "JOIN scripts s ON s.id=j.script_id JOIN posts p ON p.id=j.post_id WHERE j.id=?",
            (job_id,)).fetchone()
        if row is None:
            raise ValueError("작업을 찾지 못했습니다")
        clean = [{"part": str(ln.get("part") or "body"), "text": str(ln.get("text") or "")[:200],
                  "comment_no": ln.get("comment_no"), "seg": ln.get("seg")}
                 for ln in lines[:60]
                 if str(ln.get("text") or "").strip() and ln.get("part") != "pause"]
        if not clean:
            raise ValueError("대본이 비었습니다")
        payload = json.loads(row["lines"] or "{}")
        comments = [str(c.get("text", "")) for c in json.loads(row["comments"] or "[]")
                    if isinstance(c, dict)]
        style = self.cfg.get("style") or {}
        checks = check_script(clean, row["body"], comments, load_banned(style.get("dir") or "../style"))
        merged = with_pauses(clean, payload.get("lines") or [])
        segs = payload.get("segments") or []
        for i, seg in enumerate(segs):
            if seg.get("kind") in ("media", "body", "title"):
                seg["lines"] = [ln["text"] for ln in merged
                                if ln.get("seg") == i and ln.get("part") not in ("pause", "comment")]
        payload.update(lines=merged, warnings=checks.reasons(), est_seconds=checks.est_seconds,
                       edited=True)
        self._save_script(job_id, row, payload, checks.overlap_ratio)

    def set_segment_tts(self, job_id: int, seg: int, tts: bool) -> None:
        """검수 화면에서 세그먼트 TTS 켜기/끄기 → 줄을 다시 만들고 TTS 부터 다시."""
        from .script.segments import flatten, media_items

        row = self.store.conn.execute(
            "SELECT j.post_id, s.lines, s.tone, s.overlap_ratio, p.images, p.comments FROM jobs j "
            "JOIN scripts s ON s.id=j.script_id JOIN posts p ON p.id=j.post_id WHERE j.id=?",
            (job_id,)).fetchone()
        if row is None:
            raise ValueError("작업을 찾지 못했습니다")
        payload = json.loads(row["lines"] or "{}")
        segs = payload.get("segments") or []
        if not 0 <= seg < len(segs):
            raise ValueError("세그먼트를 찾지 못했습니다")
        segs[seg]["tts"] = bool(tts)
        segs[seg]["tts_reason"] = "검수에서 " + ("켬" if tts else "끔")
        comments = [c for c in json.loads(row["comments"] or "[]") if isinstance(c, dict)]
        payload["lines"] = flatten(segs, comments, media_items(json.loads(row["images"] or "[]")))
        self._save_script(job_id, row, payload, float(row["overlap_ratio"] or 0))
        self.voice()
        self.preview()
        self.emit({"type": "jobs_changed"})

    def _save_script(self, job_id: int, row, payload: dict[str, Any], overlap: float) -> None:
        version = self.store.next_script_version(row["post_id"])
        sid = self.store.add_script(row["post_id"], version, row["tone"], payload, overlap, "사람")
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
