from datetime import datetime

from conftest import fixture
from overshorts.crawler.dc_parser import looks_blocked, parse_comments, parse_list, parse_post


def test_list_skips_notice_ad_survey():
    rows = parse_list(fixture("mgallery_list.html"))
    assert [r.post_no for r in rows] == [1000300, 1000200, 999000]
    first = rows[0]
    assert first.title == "낙사 지역으로 떨어진 이유"
    assert (first.upvotes, first.views, first.reply_count) == (11, 1157, 6)
    assert first.posted_at == datetime(2026, 11, 15, 3, 19, 24)
    assert first.has_image and first.recommended
    assert not rows[1].has_image


def test_post_body_and_meta():
    p = parse_post(fixture("mgallery_view.html"), "example", 1000200)
    assert p.title == "받은 힐량 표시는 딜러 입장에서 환영"
    assert p.subject == "일반"
    assert p.body.splitlines() == [
        "힐러가 받은 힐량을 보면 딜러들이 할 말이 많아질 것 같다.",
        "500힐밖에 못 줬다고 뭐라 하는 사람도 나올 듯.",
        "그래도 숫자로 보이면 서로 덜 싸울 것 같음.",
        "지난 시즌에도 비슷한 얘기가 나왔는데 결국 흐지부지됐었다.",
    ]
    assert p.images == ["https://dcimg8.dcinside.co.kr/viewimage.php?id=example&no=abc"]
    assert (p.upvotes, p.views, p.comment_count) == (31, 1192, 13)
    assert p.e_s_n_o == "0f00test00esno"
    assert not p.deleted


def test_deleted_post():
    assert parse_post(fixture("deleted_view.html"), "example", 1).deleted


def test_comments_drop_dccon_bot_deleted():
    comments, total = parse_comments(fixture("comments.json"))
    assert total == 6
    assert [c.no for c in comments] == [5000001, 5000003, 5000005]
    assert comments[1].depth == 1
    assert comments[1].text == "그건 때마다 다르지 케어 못하는 힐러도 많음"


def test_comments_bad_payload():
    assert parse_comments("<html>") == ([], 0)
    assert parse_comments("[1,2]") == ([], 0)


def test_block_detection():
    assert looks_blocked(fixture("captcha.html"))
    # 정상 글 페이지의 추천용 kcaptcha 는 차단이 아니다
    assert not looks_blocked(fixture("mgallery_view.html"))
    assert not looks_blocked(fixture("mgallery_list.html"))
    assert not looks_blocked(fixture("comments.json"))
