"""커스텀 유형 대화·저장 (S18), A/B 대본·피드백 (S5), 설명·해시태그 샘플·파일 (S7·S12)."""

import json

import pytest

from overshorts.llm import LlmError, Router
from overshorts.presets import get_preset, load_presets
from overshorts.presets_custom import chat, clean_refs, delete_custom_preset, save_custom_preset
from overshorts.script.prompts import script_task, Style
from overshorts.script.publish import (load_samples, normalize_tags, parse_sample, title_tone_block,
                                        write_publish)


class Fake:
    def __init__(self, answers, name="fake"):
        self.answers, self.name, self.prompts = list(answers), name, []

    def complete(self, task, schema):
        self.prompts.append(task)
        a = self.answers.pop(0)
        if isinstance(a, Exception):
            raise a
        return a


def router(*answers):
    f = Fake(answers)
    return Router({"preset": f, "publish": f}, f), f


MINE = {"id": "mine", "label": "내 유형", "tts": "on", "guide": "클립을 먼저 보인다",
        "sections": {"target_sec": [30, 45]}}


def test_chat_returns_summary_and_does_not_save(tmp_path):
    r, f = router({"reply": "댓글은 몇 개?", "preset": MINE})
    out = chat(r, [{"role": "user", "text": "게임 클립 위주 영상"}], ["C:\\영상\\예시.mp4", "https://youtu.be/x"])
    assert out["preset"]["id"] == "mine" and any("TTS: 전부" in s for s in out["summary"])
    assert "예시.mp4" in f.prompts[0].prompt and "C:" not in f.prompts[0].prompt  # 파일은 이름만
    assert not (tmp_path / "presets").exists()


def test_chat_works_without_reference_and_without_preset_yet():
    r, _ = router({"reply": "길이는 얼마로 할까요?", "preset": None})
    out = chat(r, [{"role": "user", "text": "새 유형 만들래"}])
    assert out["preset"] is None and out["summary"] == []


def test_chat_must_end_on_user_turn():
    r, _ = router()
    with pytest.raises(ValueError):
        chat(r, [{"role": "assistant", "text": "안녕"}])


def test_bad_preset_answer_is_retried_then_fails():
    bad = {"reply": "x", "preset": {"id": "BAD ID", "label": "x"}}
    r, _ = router(bad, bad, bad)
    with pytest.raises(LlmError):
        chat(r, [{"role": "user", "text": "만들어"}])


def test_save_validates_lists_and_never_overwrites_builtin(tmp_path):
    p = save_custom_preset(tmp_path, MINE, ["https://youtu.be/x"])
    assert p.id == "mine" and p.reference.videos == ["https://youtu.be/x"]
    assert "mine" in load_presets(tmp_path) and get_preset("mine", tmp_path).tts == "on"
    again = save_custom_preset(tmp_path, {**MINE, "id": "game"})  # 내장 id → 새 id
    assert again.id == "game-2" and load_presets(tmp_path)["game"].origin == "builtin"
    with pytest.raises(ValueError):
        save_custom_preset(tmp_path, {"id": "BAD ID", "label": "x"})
    assert delete_custom_preset(tmp_path, "mine") and not delete_custom_preset(tmp_path, "game")
    assert "origin" not in json.loads((tmp_path / "presets" / "game-2.json").read_text("utf-8"))


def test_clean_refs():
    assert clean_refs(None) == [] and clean_refs(["a/b/c.mp4", "", "http://x.y/z"]) == ["c.mp4", "http://x.y/z"]


def test_samples_fix_title_tone_and_publish_files(tmp_path):
    (tmp_path / "publish").mkdir()
    (tmp_path / "publish" / "a.md").write_text(
        "제목: 계정 해킹 레전드 ㅋㅋㅋ\n설명: 해킹당한 이야기\n둘째 줄\n해시태그: #오버워치 #옵치", encoding="utf-8")
    s = load_samples(tmp_path)
    assert s[0].title.startswith("계정") and s[0].description.endswith("둘째 줄") and s[0].hashtags == ["#오버워치", "#옵치"]
    assert "계정 해킹 레전드" in title_tone_block(s) and title_tone_block([]) == ""
    r, f = router({"description": "한 줄 설명", "hashtags": ["오버워치", "#옵치", "# 아나", "#오버워치"]})
    out = write_publish(r, tmp_path / "job", "제목", ["사실"], ["줄1"], s)
    assert out["hashtags"] == ["#오버워치", "#옵치", "#아나"] and "샘플 1" in f.prompts[0].prompt
    assert (tmp_path / "job" / "publish.txt").read_text("utf-8").startswith("제목\n\n한 줄 설명")
    assert write_publish(Router({}, None), tmp_path / "x", "t", [], [], []) is None  # LLM 없음 → 막지 않고 None
    assert normalize_tags(["#" + "가" * 40, "a b"]) == ["#ab"]


def test_ab_variant_and_feedback_in_prompt():
    a = script_task("t", "b", [], [], Style(), "담백", "반말", [], 7.0, variant="A", user_feedback="훅을 더 짧게")
    b = script_task("t", "b", [], [], Style(), "담백", "반말", [], 7.0, variant="B")
    assert "A안" in a.system and "B안" in b.system and "훅을 더 짧게" in a.prompt and "편집자 피드백" not in b.prompt
    assert parse_sample("").title == ""


def test_script_pending_ab_stores_both_and_publish_files(tmp_path):
    from overshorts.db import Store
    from overshorts.script.writer import ScriptSettings, script_pending
    from tests.test_script import BODY, CMTS, Fake as F, good_script

    store = Store(":memory:")
    pid = store.upsert_post({"gall_id": "g", "post_no": 1, "title": "제목", "body": BODY, "comments": CMTS})
    jid = store.add_job(pid, "filtered")
    w = F([good_script(), {"score": 9, "issues": []}, {**good_script(), "title": "다른 제목"}, {"score": 9, "issues": []},
           {"description": "설명", "hashtags": ["#오버워치"]}])
    r = Router({"script": w, "cringe": w, "publish": w})
    c = script_pending(store, r, ScriptSettings(ab=True), jobs_dir=tmp_path)
    assert c["scripted"] == 1
    rows = {x["id"]: json.loads(x["lines"]) for x in store.conn.execute("SELECT id, lines FROM scripts")}
    job = store.conn.execute("SELECT script_id FROM jobs WHERE id=?", (jid,)).fetchone()
    cur = rows[job["script_id"]]
    assert cur["variant"] == "A" and rows[cur["alt_script_id"]]["variant"] == "B"
    assert rows[cur["alt_script_id"]]["alt_script_id"] == job["script_id"]
    assert (tmp_path / str(jid) / "publish.txt").exists()
