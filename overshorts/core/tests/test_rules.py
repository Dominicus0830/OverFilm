from overshorts.filters.rules import RuleConfig, check_post

LONG = "오늘 경쟁전에서 있었던 일인데 우리 팀 탱커가 끝까지 포기 안 하고 버텨서 역전했다. " * 3


def test_pass():
    r = check_post("제목", LONG)
    assert r.verdict == "pass" and r.reasons == []


def test_personal_info():
    for text, word in [
        ("연락 010-1234-5678 로 줘", "전화번호"),
        ("01012345678 문자", "전화번호"),
        ("메일 abc.def@example.com", "이메일"),
        ("번호 900101-1234567", "주민등록번호"),
        ("입금 110-123-456789 신한", "계좌번호"),
    ]:
        r = check_post("제목", LONG + text)
        assert r.verdict == "block", text
        assert any(word in x for x in r.reasons), (text, r.reasons)


def test_numbers_that_are_not_personal():
    r = check_post("제목", LONG + " 점수 3-2, 2026-11-14 경기, 500힐, 120-130 정도")
    assert r.verdict == "pass", r.reasons


def test_length_and_image_only():
    assert "짧" in check_post("제목", "짧은 글").reasons[0]
    assert check_post("제목", "", ["https://x/img"]).reasons == ["그림만 있는 글입니다"]
    assert "깁" in check_post("제목", LONG * 100, cfg=RuleConfig(max_chars=500)).reasons[0]


def test_block_words():
    r = check_post("제목", LONG + " 광고문의", cfg=RuleConfig(block_words=["광고문의"]))
    assert r.reasons == ["금지어: 광고문의"]
