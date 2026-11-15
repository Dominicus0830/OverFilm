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
        "[사진 1]",  # 본문 속 그림 자리
    ]
    assert p.images == ["https://dcimg8.dcinside.co.kr/viewimage.php?id=example&no=abc"]
    assert (p.upvotes, p.views, p.comment_count) == (31, 1192, 13)
    assert p.e_s_n_o == "0f00test00esno"
    assert p.author["nick"] == "가나다"
    assert p.gall_name == "예시 마이너 갤러리"
    assert not p.deleted


def test_deleted_post():
    assert parse_post(fixture("deleted_view.html"), "example", 1).deleted


def test_comments_keep_dccon_drop_bot_deleted():
    comments, total = parse_comments(fixture("comments.json"))
    assert total == 6
    assert [c.no for c in comments] == [5000001, 5000002, 5000003, 5000005]
    con = comments[1]
    assert con.text == "" and con.dccon and con.dccon["url"].startswith("https://dcimg5")
    assert con.depth == 1 and con.parent == 5000001  # c_no 가 없으면 앞 댓글
    reply = comments[2]
    assert reply.text == "그건 때마다 다르지\n케어 못하는 힐러도 많음"
    assert reply.ip == "1.2" and reply.icon == "" and reply.nick == "다"


def test_comments_bad_payload():
    assert parse_comments("<html>") == ([], 0)
    assert parse_comments("[1,2]") == ([], 0)


def test_block_detection():
    assert looks_blocked(fixture("captcha.html"))
    # 정상 글 페이지의 추천용 kcaptcha 는 차단이 아니다
    assert not looks_blocked(fixture("mgallery_view.html"))
    assert not looks_blocked(fixture("mgallery_list.html"))
    assert not looks_blocked(fixture("comments.json"))


def test_post_keeps_writer_styles():
    html = (
        '<html><head><title>글 - 오버워치 2 마이너 갤러리</title></head><body>'
        '<span class="title_subject">퀸</span><div class="write_div">'
        '<p><span style="font-size:36px;"><font color="#ff0000">크아악 퀸장연!!</font></span></p>'
        '<p>그냥 줄</p>'
        '<p><b>굵게</b> 그리고 <i style="font-family:궁서">기울임</i></p>'
        '<p><span style="color:rgb(51,51,51);font-size:13px">기본 글자</span></p>'
        '</div></body></html>'
    )
    post = parse_post(html, "overwatch2", 1)
    assert post.body.splitlines() == ["크아악 퀸장연!!", "그냥 줄", "굵게 그리고 기울임", "기본 글자"]
    styled = {s["text"]: s["runs"] for s in post.body_styles}
    assert styled["크아악 퀸장연!!"] == [{"t": "크아악 퀸장연!!", "color": "#ff0000", "size": 36}]
    assert styled["굵게 그리고 기울임"] == [{"t": "굵게", "bold": True}, {"t": " 그리고 "},
                                       {"t": "기울임", "font": "궁서", "italic": True}]
    assert "그냥 줄" not in styled and "기본 글자" not in styled
