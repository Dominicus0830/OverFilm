import pytest

from overshorts.urls import UrlError, parse_url


@pytest.mark.parametrize(
    "url,kind,gall_type,gall_id,no,rec,mobile",
    [
        ("https://gall.dcinside.com/board/lists/?id=baseball_new11", "gallery", "board", "baseball_new11", None, False, False),
        ("https://gall.dcinside.com/mgallery/board/lists/?id=overwatch2", "gallery", "mgallery", "overwatch2", None, False, False),
        ("gall.dcinside.com/mini/board/lists/?id=ex_mini", "gallery", "mini", "ex_mini", None, False, False),
        ("https://gall.dcinside.com/mgallery/board/lists/?id=overwatch2&exception_mode=recommend", "gallery", "mgallery", "overwatch2", None, True, False),
        ("https://gall.dcinside.com/board/view/?id=game&no=123", "post", "board", "game", 123, False, False),
        ("https://gall.dcinside.com/mgallery/board/view/?id=overwatch2&no=1981822&exception_mode=recommend&page=1", "post", "mgallery", "overwatch2", 1981822, True, False),
        ("https://gall.dcinside.com/mini/board/view/?id=ex_mini&no=7", "post", "mini", "ex_mini", 7, False, False),
        ("https://m.dcinside.com/board/overwatch2", "gallery", "unknown", "overwatch2", None, False, True),
        ("https://m.dcinside.com/board/overwatch2/1981822", "post", "unknown", "overwatch2", 1981822, False, True),
        ("https://m.dcinside.com/mini/ex_mini/7", "post", "mini", "ex_mini", 7, False, True),
        ("https://m.dcinside.com/board/overwatch2?recommend=1", "gallery", "unknown", "overwatch2", None, True, True),
    ],
)
def test_parse(url, kind, gall_type, gall_id, no, rec, mobile):
    u = parse_url(url)
    assert (u.kind, u.gall_type, u.gall_id, u.post_no, u.recommend_only, u.from_mobile) == (
        kind, gall_type, gall_id, no, rec, mobile)


@pytest.mark.parametrize("url", [
    "", "https://example.com/board/lists/?id=x", "https://gall.dcinside.com/board/view/?id=x",
    "https://gall.dcinside.com/board/lists/?id=../etc", "https://gall.dcinside.com/board/view/?id=x&no=1e9",
    "https://m.dcinside.com/", "https://gall.dcinside.com/board/write/?id=x",
])
def test_rejects(url):
    with pytest.raises(UrlError):
        parse_url(url)


def test_pc_urls_and_label():
    u = parse_url("https://m.dcinside.com/board/overwatch2/5").with_gall_type("mgallery")
    assert u.view_url() == "https://gall.dcinside.com/mgallery/board/view/?id=overwatch2&no=5"
    assert u.label == "마이너 갤 게시물"
    g = parse_url("https://gall.dcinside.com/mini/board/lists/?id=a&exception_mode=recommend")
    assert g.list_url(2) == "https://gall.dcinside.com/mini/board/lists/?id=a&page=2&exception_mode=recommend"
    assert g.galltype_code == "MI"
