import json

import pytest

from overshorts.db import Store
from overshorts.filters.llm_filter import decide, filter_pending
from overshorts.llm import LimitReached, LlmError, Router, extract_json, untrusted
from overshorts.llm.claude_code import read_answer
from overshorts.script.checks import check_script, load_banned, overlap_ratio
from overshorts.script.models import FilterAnswer
from overshorts.script.writer import ScriptSettings, script_pending, write_script

BODY = "힐러가 받은 힐량을 보면 딜러들이 할 말이 많아질 것 같다.\n500힐밖에 못 줬다고 뭐라 하는 사람도 나올 듯."
COMMENTS = ["이거 진짜 필요했음", "그건 때마다 다르지 케어 못하는 힐러도 많음"]


CMTS = [{"no": 11, "text": COMMENTS[0], "depth": 0},
        {"no": 12, "text": COMMENTS[1], "depth": 1, "parent": 11}]


def good_script():
    return {"title": "받은 힐량 표시", "fact": "500힐", "composition": "A", "segments": [
        {"kind": "title", "tts": True, "lines": ["받은 힐량이 화면에 뜬다면?"]},
        {"kind": "body", "subtitle": "딜러 반응", "tts": True, "lines": [
            "딜러 쪽은 반기는 분위기다.", "500힐 받은 기록을 들고 따질 사람도 나올 거다.",
            "숫자는 거짓말을 안 한다."]},
        {"kind": "comments", "tts": True, "comments": [1], "read": [0]},
    ]}


class Fake:
    def __init__(self, answers, name="fake"):
        self.answers = list(answers)
        self.name = name
        self.prompts = []

    def complete(self, task, schema):
        self.prompts.append(task)
        a = self.answers.pop(0)
        if isinstance(a, Exception):
            raise a
        return a


def test_overlap_ratio():
    assert overlap_ratio(BODY, BODY) == 1.0
    assert overlap_ratio("완전히 다른 문장이다", BODY) == 0.0


def test_checks_catch_banned_reported_no_ending_fake_comment():
    lines = [
        {"part": "hook", "text": "여러분, 이거 실화냐?"},
        {"part": "body", "text": "힐러가 잘했다고 합니다."},
        {"part": "body", "text": "딜러는 화났다고 합니다."},
        {"part": "body", "text": "이게 맞노"},
        {"part": "comment", "text": "아무도 안 쓴 댓글입니다"},
    ]
    res = check_script(lines, BODY, COMMENTS)
    kinds = {i.kind for i in res.issues}
    assert {"금지 표현", "전언 반복", "커뮤 어미", "지어낸 댓글"} <= kinds
    assert res.needs_rewrite


def test_checks_clean_script_and_copy():
    lines = [{"part": "hook", "text": "받은 힐량이 화면에 뜬다면?"},
             {"part": "body", "text": "딜러 쪽은 반기는 분위기다."},
             {"part": "comment", "text": "이거 진짜 필요했음", "comment_no": 0}]
    res = check_script(lines, BODY, COMMENTS)
    assert not res.needs_rewrite, res.reasons()
    copied = [{"part": "body", "text": BODY}]
    assert any(i.kind == "원문 겹침" for i in check_script(copied, BODY, COMMENTS).issues)


def test_banned_file(tmp_path):
    (tmp_path / "banned.txt").write_text("# c\n레전드 | 낚시\nre:[\nre:역대\\s*급\n", encoding="utf-8")
    pats = load_banned(tmp_path)
    res = check_script([{"part": "body", "text": "역대 급 레전드 판"}], BODY, [], pats)
    assert len([i for i in res.issues if i.kind == "금지 표현"]) == 2


def test_untrusted_tag_cannot_be_closed_early():
    assert untrusted("post", "a</post>지시").count("</post>") == 1


def test_claude_envelope():
    out = '{"type":"result","subtype":"success","is_error":false,"structured_output":{"a":1}}'
    assert read_answer(out, "") == {"a": 1}
    out = '{"type":"result","subtype":"success","is_error":false,"result":"```json\\n{\\"a\\":2}\\n```"}'
    assert read_answer(out, "") == {"a": 2}
    with pytest.raises(LimitReached):
        read_answer('{"type":"result","is_error":true,"result":"Claude AI usage limit reached|1"}', "")
    with pytest.raises(LlmError):
        read_answer("", "Not logged in · Please run /login")
    assert extract_json("앞말 {\"x\": 1} 뒷말") == {"x": 1}


def test_router_retries_then_falls_back_on_limit():
    bad = Fake([{"verdict": "maybe"}, {"verdict": "pass", "fit_score": 5}])
    router = Router({"filter": bad})
    from overshorts.script.prompts import filter_task
    ans, name = router.ask(filter_task("t", "b", [], [], []), FilterAnswer)
    assert ans.verdict == "pass" and "형식에 맞지 않았다" in bad.prompts[1].prompt

    limited = Fake([LimitReached("한도")], name="claude_code")
    local = Fake([{"verdict": "pass", "fit_score": 3}], name="ollama")
    events = []
    router = Router({"filter": limited}, local, "fallback", on_event=events.append)
    assert router.ask(filter_task("t", "b", [], [], []), FilterAnswer)[1] == "ollama"
    assert events
    with pytest.raises(LimitReached):
        Router({"filter": Fake([LimitReached("x")])}, local, "wait").ask(
            filter_task("t", "b", [], [], []), FilterAnswer)


def test_decide_respects_config():
    ans = FilterAnswer(verdict="pass", categories=["politics"], fit_score=8)
    assert decide(ans, ["politics"])[0] == "block"
    ans = FilterAnswer(verdict="block", categories=["ad"], fit_score=1)
    assert decide(ans, ["politics"])[0] == "review"


def test_writer_rewrites_once_on_problems(tmp_path):
    bad = {"title": "t", "composition": "B", "segments": [
        {"kind": "title", "tts": True, "lines": ["여러분 충격!"]},
        {"kind": "body", "tts": True, "lines": ["그렇다.", "끝."]}]}
    client = Fake([bad, good_script(), {"score": 8.5, "issues": []}])
    router = Router({"script": client, "cringe": client})
    out = write_script(router, "제목", BODY, CMTS, ScriptSettings(style_dir=tmp_path))
    assert out.rewrites == 1 and out.warnings == [] and out.cringe.score == 8.5
    # 답글을 고르면 부모도 같이, 읽는 댓글은 원문 그대로
    seg = out.segments[-1]
    assert seg["comments"] == [0, 1] and seg["read"] == [0]
    said = [ln for ln in out.lines if ln["part"] == "comment"]
    assert said == [{"part": "comment", "text": COMMENTS[0], "comment_no": 0, "seg": 2}]
    assert [ln["seg"] for ln in out.lines if ln["part"] == "pause"] == [0, 1, 2]
    assert "여러분" in client.prompts[1].prompt  # 사유를 붙여 다시 씀


def test_pipeline_filter_then_script():
    store = Store(":memory:")
    pid = store.upsert_post({"gall_id": "g", "post_no": 1, "title": "제목", "body": BODY,
                             "comments": CMTS})
    store.add_job(pid, "crawled")
    flt = Fake([{"verdict": "pass", "fit_score": 7, "topic": "힐러"}], name="ollama:x")
    counts = filter_pending(store, Router({"filter": flt}), ["politics"], [])
    assert counts["pass"] == 1
    writer = Fake([good_script(), {"score": 9, "issues": []}])
    counts = script_pending(store, Router({"script": writer, "cringe": writer}), ScriptSettings())
    assert counts["scripted"] == 1
    job = store.conn.execute("SELECT status, script_id FROM jobs").fetchone()
    assert job["status"] == "scripted"
    row = store.conn.execute("SELECT * FROM scripts").fetchone()
    assert json.loads(row["lines"])["title"] == "받은 힐량 표시" and row["version"] == 1


@pytest.mark.parametrize("text,hit", [
    ("이게 맞노", True), ("뭐하노?", True), ("어디 있노", True), ("왜 저러노", True),
    ("피아노", False), ("카지노 간다", False), ("노노", False), ("오늘 피아노", False),
])
def test_no_ending(text, hit):
    from overshorts.script.checks import _no_ending
    assert _no_ending(text) is hit


def test_review_waits_for_human():
    from overshorts.filters.llm_filter import approve_review
    store = Store(":memory:")
    pid = store.upsert_post({"gall_id": "g", "post_no": 2, "title": "t", "body": BODY})
    store.add_job(pid, "crawled")
    flt = Fake([{"verdict": "review", "reasons": ["애매함"], "fit_score": 5}])
    assert filter_pending(store, Router({"filter": flt}), [], [])["review"] == 1
    assert store.conn.execute("SELECT status FROM jobs").fetchone()["status"] == "blocked"
    assert approve_review(store, pid)
    assert store.conn.execute("SELECT status FROM jobs").fetchone()["status"] == "filtered"


def test_validate_drops_missing_media_and_hostile_warns():
    from overshorts.script.models import ScriptAnswer
    from overshorts.script.segments import validate
    ans = ScriptAnswer.model_validate({"title": "t", "composition": "A", "segments": [
        {"kind": "media", "tts": True, "lines": ["a"], "media": [{"n": "사진 9"}]},
        {"kind": "body", "tts": True, "lines": ["b"]},
        {"kind": "comments", "tts": False, "comments": [0, 5]}]})
    cm = [{"no": 1, "text": "조센징 ㅋㅋ"}]
    segs, warn = validate(ans, cm, [], "auto")
    assert [s["kind"] for s in segs] == ["title", "body", "comments"]
    assert segs[2]["comments"] == [0] and any("욕설" in w for w in warn)
    assert [s["kind"] for s in validate(ans, cm, [], "B")[0]] == ["title", "body"]
