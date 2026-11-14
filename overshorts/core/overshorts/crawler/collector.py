"""수집: 링크 하나 → 글 저장 + 1차 규칙 필터 (M1).

요청을 줄이는 순서 (기획서 5.1):
1. 목록 페이지의 추천 수·날짜로 먼저 거른다.
2. 이미 받은 글·폐기한 글은 다시 받지 않는다.
3. 후보 글만 본문을 요청하고, 규칙 필터를 통과한 글만 댓글을 요청한다.
원본 HTML은 작업 폴더 cache/ 에 남긴다.
"""

from __future__ import annotations

import hashlib
import json
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from pathlib import Path

from ..db import Store
from ..filters.rules import RuleConfig, check_post
from ..urls import DcUrl, parse_url
from .budget import CrawlStopped
from .dc_parser import Comment, Post, parse_comments, parse_list, parse_post, redirects_to_mgallery
from .fetcher import FetchError, Fetcher

COMMENT_URL = "https://gall.dcinside.com/board/comment/"
MAX_LIST_PAGES = 5
MAX_COMMENT_PAGES = 3


@dataclass
class GalleryOptions:
    recommend_only: bool = False
    min_upvotes: int = 0
    within_hours: float | None = 24.0
    max_posts: int = 10


@dataclass
class IngestReport:
    url: DcUrl
    saved: list[int] = field(default_factory=list)  # posts.id (규칙 필터 통과)
    blocked: list[tuple[int, list[str]]] = field(default_factory=list)  # (post_no, 사유)
    skipped_known: int = 0
    requests: int = 0
    stopped: str | None = None  # 예산·차단으로 멈췄으면 그 사유
    errors: list[str] = field(default_factory=list)

    def summary(self) -> str:
        parts = [f"{self.url.label}: 저장 {len(self.saved)}개, 규칙에서 막힘 {len(self.blocked)}개"]
        if self.skipped_known:
            parts.append(f"이미 받은 글 {self.skipped_known}개 건너뜀")
        if self.stopped:
            parts.append(f"멈춤: {self.stopped}")
        if self.errors:
            parts.append(f"오류 {len(self.errors)}건")
        return ", ".join(parts)


class Collector:
    def __init__(self, store: Store, fetcher: Fetcher, rules: RuleConfig | None = None,
                 cache_dir: str | Path | None = None, max_comments: int = 50,
                 now: Callable[[], datetime] = datetime.now,
                 image_dir: str | Path | None = None, max_images: int = 5):
        self.store = store
        self.fetcher = fetcher
        self.rules = rules or RuleConfig()
        self.cache_dir = Path(cache_dir) if cache_dir else None
        self.max_comments = max(0, min(int(max_comments), 500))
        self.now = now
        self.image_dir = Path(image_dir) if image_dir else None
        self.max_images = max(0, min(int(max_images), 30))

    # --- 공개 진입점 ---
    def ingest(self, raw_url: str, options: GalleryOptions | None = None) -> IngestReport:
        url = parse_url(raw_url)
        opts = options or GalleryOptions(recommend_only=url.recommend_only)
        if url.recommend_only:
            opts.recommend_only = True
        report = IngestReport(url)
        before = self.fetcher.guard.requests_made
        try:
            url = self._resolve_gall_type(url, opts)
            report.url = url
            if url.kind == "post" and url.post_no is not None:
                self._ingest_post(url, url.post_no, report)
            else:
                self.store.upsert_source(raw_url, url.kind, url.gall_type, url.gall_id,
                                         opts.__dict__)
                self._ingest_gallery(url, opts, report)
        except CrawlStopped as stop:
            report.stopped = stop.reason
        report.requests = self.fetcher.guard.requests_made - before
        return report

    # --- 내부 ---
    def _resolve_gall_type(self, url: DcUrl, opts: GalleryOptions) -> DcUrl:
        """모바일 링크는 정식/마이너를 모른다. 정식 주소를 열어 보고 판단한다."""
        if url.gall_type != "unknown":
            return url
        probe = url.with_gall_type("board")
        page = probe.view_url() if url.kind == "post" else probe.list_url()
        resp = self.fetcher.get(page)
        self._cache(f"{url.gall_id}/probe.html", resp.text)
        if "/mgallery/" in resp.url or redirects_to_mgallery(resp.text):
            return url.with_gall_type("mgallery")
        return probe

    def _cache(self, name: str, text: str) -> None:
        if self.cache_dir is None:
            return
        try:
            path = self.cache_dir / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        except OSError:
            pass  # 캐시는 있으면 좋은 것. 실패해도 수집은 계속.

    def _ingest_gallery(self, url: DcUrl, opts: GalleryOptions, report: IngestReport) -> None:
        listing = DcUrl(url.kind, url.gall_type, url.gall_id, None, opts.recommend_only,
                        url.from_mobile)
        cutoff = None
        if opts.within_hours is not None and opts.within_hours > 0:
            cutoff = self.now() - timedelta(hours=min(opts.within_hours, 24 * 365))
        max_posts = max(1, min(int(opts.max_posts), 200))
        candidates = []
        for page in range(1, MAX_LIST_PAGES + 1):
            resp = self.fetcher.get(listing.list_url(page))
            self._cache(f"{url.gall_id}/list-{page}.html", resp.text)
            rows = parse_list(resp.text)
            if not rows:
                break
            too_old = False
            for row in rows:
                if cutoff is not None and row.posted_at is not None and row.posted_at < cutoff:
                    too_old = True
                    continue
                if row.upvotes < opts.min_upvotes:
                    continue
                if self.store.has_post(url.gall_id, row.post_no):
                    report.skipped_known += 1
                    continue
                candidates.append(row)
            # 목록은 최신순. 기간 밖 글이 나왔거나 후보가 충분하면 다음 페이지를 안 본다.
            if too_old or len(candidates) >= max_posts:
                break
        for row in candidates[:max_posts]:
            self._ingest_post(url, row.post_no, report)
            self.fetcher.guard.post_fetched()

    def _ingest_post(self, url: DcUrl, post_no: int, report: IngestReport) -> None:
        if self.store.is_discarded(url.gall_id, post_no):
            report.skipped_known += 1
            return
        view = url.view_url(post_no)
        try:
            resp = self.fetcher.get(view, referer=url.list_url())
        except FetchError as err:
            report.errors.append(str(err))
            return
        self._cache(f"{url.gall_id}/{post_no}.html", resp.text)
        post = parse_post(resp.text, url.gall_id, post_no)
        if post.deleted:
            report.errors.append(f"{post_no}: 삭제된 글")
            return
        result = check_post(post.title, post.body, post.images, self.rules)
        comments: list[Comment] = []
        images = [{"url": u, "local": None, "inspected": None} for u in post.images]
        if result.verdict == "pass":
            if self.max_comments > 0 and post.comment_count > 0:
                comments = self._comments(url, post, view, report)
            images = self._images(url, post, report)
        post_id = self.store.upsert_post(_record(url, post, comments, images))
        self.store.add_filter_result(post_id, "rule", result.verdict, result.reasons)
        if result.verdict == "pass":
            self.store.add_job(post_id, "crawled")
            report.saved.append(post_id)
        else:
            report.blocked.append((post_no, result.reasons))

    def _images(self, url: DcUrl, post: Post, report: IngestReport) -> list[dict]:
        """규칙을 통과한 글의 첨부 그림만 받는다. 검사(얼굴·선정성)는 inspect 단계가 모아서 한다."""
        out: list[dict] = []
        for i, src in enumerate(post.images):
            item: dict = {"url": src, "local": None, "inspected": None}
            if self.image_dir is not None and i < self.max_images:
                try:
                    data = self.fetcher.get_image(src)
                    path = self.image_dir / url.gall_id / f"{post.post_no}-{i}.img"
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(data)
                    item.update(local=str(path), inspected=False)
                except FetchError as err:
                    report.errors.append(f"{post.post_no} 그림: {err}")
                except OSError as err:
                    report.errors.append(f"{post.post_no} 그림 저장 실패: {err}")
            out.append(item)
        return out

    def _comments(self, url: DcUrl, post: Post, referer: str,
                  report: IngestReport) -> list[Comment]:
        out: list[Comment] = []
        for page in range(1, MAX_COMMENT_PAGES + 1):
            form = {
                "id": url.gall_id,
                "no": str(post.post_no),
                "cmt_id": url.gall_id,
                "cmt_no": str(post.post_no),
                "focus_cno": "",
                "focus_pno": "",
                "e_s_n_o": post.e_s_n_o,
                "comment_page": str(page),
                "sort": "",
                "prevCnt": "",
                "board_type": "",
                "_GALLTYPE_": url.galltype_code,
            }
            try:
                resp = self.fetcher.post_form(COMMENT_URL, form, referer)
            except FetchError as err:
                report.errors.append(f"{post.post_no} 댓글: {err}")
                break
            got, total = parse_comments(resp.text)
            out.extend(got)
            if len(out) >= self.max_comments or not got or total <= 0:
                break
            # 한 페이지가 전체를 다 주면(보통 100개 단위) 더 묻지 않는다
            if _raw_count(resp.text) >= total:
                break
        return out[: self.max_comments]


def _raw_count(payload: str) -> int:
    try:
        data = json.loads(payload)
        return len(data.get("comments") or []) if isinstance(data, dict) else 0
    except (json.JSONDecodeError, TypeError):
        return 0


def _record(url: DcUrl, post: Post, comments: list[Comment], images: list[dict]) -> dict:
    digest = hashlib.sha256(f"{post.title}\n{post.body}".encode()).hexdigest()
    return {
        "gall_id": url.gall_id,
        "post_no": post.post_no,
        "gall_type": url.gall_type,
        "title": post.title,
        "subject": post.subject,
        "body": post.body,
        # 그림 판정(faces, nsfw, blurred, dropped)은 images.inspect_pending 이 채운다
        "images": images,
        "comments": [c.__dict__ for c in comments],
        "upvotes": post.upvotes,
        "posted_at": post.posted_at.isoformat() if post.posted_at else None,
        "content_hash": digest,
    }
