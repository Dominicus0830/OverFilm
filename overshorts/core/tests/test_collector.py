import json
from pathlib import Path
from datetime import datetime

from conftest import fixture
from overshorts.crawler.budget import CrawlGuard, CrawlLimits
from overshorts.crawler.collector import Collector, GalleryOptions
from overshorts.crawler.fetcher import Fetcher, Response
from overshorts.db import Store

NOW = datetime(2026, 11, 15, 4, 0, 0)


class FakeDc:
    def __init__(self, view_html=None, block_after=None):
        self.calls = []
        self.view_html = view_html or fixture("mgallery_view.html")
        self.block_after = block_after

    def __call__(self, method, url, headers, form):
        self.calls.append((method, url))
        if self.block_after is not None and len(self.calls) > self.block_after:
            return Response(429, "", url)
        if "/lists/" in url:
            if "page=1" in url:
                return Response(200, fixture("mgallery_list.html"), url)
            return Response(200, "<html><table class='gall_list'><tbody></tbody></table></html>", url)
        if "/comment/" in url:
            return Response(200, fixture("comments.json"), url)
        return Response(200, self.view_html, url)


def make(transport, limits=None):
    store = Store(":memory:")
    guard = CrawlGuard(store, limits or CrawlLimits(), sleep=lambda s: None)
    return store, Collector(store, Fetcher(guard, transport), now=lambda: NOW)


def test_gallery_filters_by_list_before_fetching_bodies():
    dc = FakeDc()
    store, col = make(dc)
    rep = col.ingest(
        "https://gall.dcinside.com/mgallery/board/lists/?id=example",
        GalleryOptions(min_upvotes=20, within_hours=24, max_posts=10),
    )
    # 1000300(추천 11)은 추천 수로, 999000은 기간으로 걸러져 본문 요청이 없다
    views = [u for m, u in dc.calls if "/view/" in u]
    assert views == ["https://gall.dcinside.com/mgallery/board/view/?id=example&no=1000200"]
    assert len(rep.saved) == 1 and rep.stopped is None
    row = store.conn.execute("SELECT * FROM posts").fetchone()
    assert row["upvotes"] == 31
    assert [c["no"] for c in json.loads(row["comments"])] == [5000001, 5000002, 5000003, 5000005]
    assert store.conn.execute("SELECT status FROM jobs").fetchone()["status"] == "crawled"
    # 다시 돌려도 이미 받은 글은 다시 안 받는다
    dc.calls.clear()
    rep2 = col.ingest("https://gall.dcinside.com/mgallery/board/lists/?id=example",
                      GalleryOptions(min_upvotes=20, within_hours=24))
    assert [u for m, u in dc.calls if "/view/" in u] == []
    assert rep2.skipped_known == 1


def test_short_post_blocked_and_no_comment_request():
    short = fixture("mgallery_view.html").replace(
        "힐러가 받은 힐량을 보면 딜러들이 할 말이 많아질 것 같다.", "").replace(
        "500힐밖에 못 줬다고 뭐라 하는 사람도 나올 듯.", "")
    dc = FakeDc(view_html=short)
    store, col = make(dc)
    rep = col.ingest("https://gall.dcinside.com/mgallery/board/view/?id=example&no=1000200")
    assert rep.saved == [] and rep.blocked
    assert not any("/comment/" in u for _, u in dc.calls)


def test_block_stops_immediately():
    dc = FakeDc(block_after=1)
    store, col = make(dc)
    rep = col.ingest("https://gall.dcinside.com/mgallery/board/lists/?id=example",
                     GalleryOptions(min_upvotes=0, within_hours=None))
    assert rep.stopped and "멈췄" in rep.stopped
    assert len(dc.calls) == 2  # 막힌 요청 뒤로는 더 보내지 않는다
    rep2 = col.ingest("https://gall.dcinside.com/mgallery/board/lists/?id=example")
    assert "쿨다운" in (rep2.stopped or "") and len(dc.calls) == 2


def test_budget_respected():
    dc = FakeDc()
    store, col = make(dc, CrawlLimits(hourly_limit=2))
    rep = col.ingest("https://gall.dcinside.com/mgallery/board/lists/?id=example",
                     GalleryOptions(min_upvotes=0, within_hours=None))
    assert len(dc.calls) == 2 and "예산" in (rep.stopped or "")


def test_mobile_link_resolves_mgallery():
    dc = FakeDc()
    redirect = "<script>location.replace('https://gall.dcinside.com/mgallery/board/lists/?id=example');</script>"

    def transport(method, url, headers, form):
        if url.startswith("https://gall.dcinside.com/board/"):
            dc.calls.append((method, url))
            return Response(200, redirect, url)
        return dc(method, url, headers, form)

    store, col = make(transport)
    rep = col.ingest("https://m.dcinside.com/board/example/1000200")
    assert rep.url.gall_type == "mgallery"
    assert any("/mgallery/board/view/" in u for _, u in dc.calls)
    assert len(rep.saved) == 1


def test_images_downloaded_with_referer(tmp_path):
    jpeg = b"\xff\xd8\xff\xe0" + b"0" * 100
    seen = {}
    dc = FakeDc()

    def transport(method, url, headers, form):
        if "viewimage.php" in url or "dccon.php" in url:
            seen["referer"] = headers.get("Referer")
            return Response(200, "", url, jpeg, "application/octet-stream")
        return dc(method, url, headers, form)

    store = Store(":memory:")
    guard = CrawlGuard(store, CrawlLimits(), sleep=lambda s: None)
    col = Collector(store, Fetcher(guard, transport), now=lambda: NOW, image_dir=tmp_path)
    rep = col.ingest("https://gall.dcinside.com/mgallery/board/view/?id=example&no=1000200")
    assert len(rep.saved) == 1 and rep.errors == []
    assert seen["referer"] == "https://gall.dcinside.com/"
    item = json.loads(store.conn.execute("SELECT images FROM posts").fetchone()["images"])[0]
    assert item["inspected"] is False and Path(item["local"]).read_bytes() == jpeg
    assert item["kind"] == "image"
    con = json.loads(store.conn.execute("SELECT comments FROM posts").fetchone()["comments"])[1]
    assert con["dccon"]["local"] and Path(con["dccon"]["local"]).read_bytes() == jpeg
