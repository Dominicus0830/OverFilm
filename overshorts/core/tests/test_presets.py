"""유형 프리셋·댓글 화면 묶음·장당 시간·자막 줄 길이·클립 배속·전사 대조·린터 (S1·S2·S3·S4·S17·S19)."""

import json

import pytest

from overshorts.config import edit_preset, flag, load_config, script_settings, voice_options
from overshorts.presets import (CommentRules, Preset, apply_overrides, get_preset, load_presets,
                                page_seconds, parse_preset, prompt_block)
from overshorts.render.spec import caption_chunks, fit_rate, layout_spec
from overshorts.script.lint import lint_lines
from overshorts.script.prompts import script_task
from overshorts.script.segments import display_seconds, pack_screens, page_height
from overshorts.tts.verify import compare, srt_text, verify_voice


def _comments(n=12, reply_of=None):
    rows = []
    for i in range(n):
        c = {"no": 100 + i, "text": "ㅋㅋ " * 5, "depth": 0, "parent": 0, "nick": "ㅇㅇ"}
        if reply_of and i in reply_of:
            c.update(depth=1, parent=100 + reply_of[i])
        rows.append(c)
    return rows


def test_four_builtin_presets_load_and_differ():
    t = load_presets()
    assert {"issue", "ogal", "info", "game"} <= set(t)
    assert t["info"].tts == "auto" and t["ogal"].tts == "off"
    assert t["issue"].comments.per_screen == (3, 3)
    assert t["game"].sections.max_clip_sec == 30 and t["game"].effects.original_sound
    assert all(not p.layout.title_bar for p in t.values())  # 참고 쇼츠: 제목 바 없음


def test_unknown_or_broken_preset_never_crashes(tmp_path):
    (tmp_path / "presets").mkdir()
    (tmp_path / "presets" / "bad.json").write_text("{not json", encoding="utf-8")
    (tmp_path / "presets" / "worse.json").write_text(json.dumps({"id": "BAD ID", "label": "x"}), encoding="utf-8")
    (tmp_path / "presets" / "mine.json").write_text(json.dumps(
        {"id": "mine", "label": "내 유형", "comments": {"per_screen": [9, 1], "max_total": 40},
         "sections": {"target_sec": [500, -3]}, "unknown_field": 1}), encoding="utf-8")
    t = load_presets(tmp_path)
    assert "mine" in t and t["mine"].origin == "custom"
    assert t["mine"].comments.max_total == 40 and t["mine"].comments.per_screen == (1, 8)
    assert t["mine"].sections.target_sec == (0.0, 180.0)
    assert "worse" not in t and "bad" not in t  # 잘못된 파일은 건너뜀 (경고만, 죽지 않음)
    (tmp_path / "presets" / "mine.json").write_text(json.dumps({"id": "mine", "label": "x",
                                                                "comments": {"max_total": 9999}}), encoding="utf-8")
    assert "mine" not in load_presets(tmp_path)       # 범위 밖 숫자 하나면 그 파일은 쓰지 않음
    assert get_preset("nope", tmp_path).id == "ogal"  # 모르는 id → 기본
    with pytest.raises(ValueError):
        parse_preset([1, 2])


def test_custom_overrides_builtin_with_same_id(tmp_path):
    (tmp_path / "presets").mkdir()
    (tmp_path / "presets" / "ogal.json").write_text(
        json.dumps({"id": "ogal", "label": "내 옵갤", "tts": "on"}), encoding="utf-8")
    assert load_presets(tmp_path)["ogal"].label == "내 옵갤"


def test_page_seconds_formula():
    r = CommentRules()
    assert page_seconds(r, 0) == 2.8            # 최소 2.8초
    assert page_seconds(r, 60) == pytest.approx(4.2)  # 1.2 + 60*0.05
    assert page_seconds(r, 10_000) == r.max_screen_sec
    assert page_seconds(r, 60, dccons=1) > page_seconds(r, 60)


def test_overrides_from_config_beat_preset():
    p = apply_overrides(get_preset("ogal"), {"caption_max_chars": "16", "caption_max_words": 3,
                                             "max_comments": 5, "tts": "auto"})
    assert (p.caption.max_chars, p.caption.max_words, p.comments.max_total, p.tts) == (16, 3, 5, "auto")
    q = apply_overrides(get_preset("ogal"), {"caption_max_chars": None, "tts": "preset"})
    assert q.caption.max_chars == 13 and q.tts == "off"
    r = apply_overrides(get_preset("ogal"), {"caption_max_chars": "abc"})
    assert r.caption.max_chars == 13


def test_pack_screens_keeps_threads_and_fills_height():
    cs = _comments(10, reply_of={3: 2, 4: 2})
    # LLM 이 답글(3)만 골라도 원댓글(2)이 같이, 원댓글·답글은 한 장
    pages = pack_screens([[3], [0, 1]], [], cs, page_height(10, 76), 9, 27)
    flat = [n for pg in pages for n in pg]
    assert pages == [[0, 1], [2, 3]] and flat == sorted(flat)
    # 개수·장 수 상한
    many = pack_screens([[i] for i in range(10)], [], cs, 200, 3, 4)
    assert len(many) <= 3 and sum(len(p) for p in many) <= 4
    # 묶음이 없으면 높이·개수로 채움, 없는 번호는 버림
    auto = pack_screens([], [0, 1, 2, 3, 99, -1], cs, 4000, 9, 27, (1, 2))
    assert all(len(p) <= 2 for p in auto) and 99 not in [n for p in auto for n in p]
    # 높이를 넘으면 글타래 단위로 다음 장 (쪼개지 않음)
    tiny = pack_screens([[2, 3, 4]], [], cs, 70, 9, 27)
    assert [2, 3, 4] in tiny or all({2, 3, 4} >= set(p) or not (set(p) & {2, 3, 4}) for p in tiny)
    assert pack_screens([[]], [], cs, 100, 5, 5) == []


def test_display_seconds_uses_page_secs_for_comments():
    seg = {"kind": "comments", "pages": [[0, 1], [2]], "picked": [0, 1, 2], "page_secs": [3.0, 4.5]}
    assert display_seconds(seg, [], []) == 7.5
    assert display_seconds({"kind": "title", "hold": 3.0}, [], []) == 3.0
    assert display_seconds({"kind": "title"}, [], []) == 2.0  # 프리셋 없으면 옛 값


def test_caption_rules_replace_fixed_13():
    words = [{"text": t, "start": i * 0.3, "end": i * 0.3 + 0.25} for i, t in
             enumerate("오늘 아주 긴 이야기를 해볼게요 정말로 길어요".split())]
    sc = [{"words": words, "seg": 0}]
    narrow = caption_chunks(sc, max_chars=8, max_words=4)
    wide = caption_chunks(sc, max_chars=30, max_words=8)
    assert len(narrow) > len(wide)
    assert all(sum(len(w["text"]) for w in c["words"]) <= 8 + 3 for c in narrow)
    shown = caption_chunks(sc, 13, 4, min_show=2.0)
    assert all(c["end"] - c["start"] >= 0.25 for c in shown)


def test_fit_rate_clamps_to_25_percent():
    assert fit_rate(10, 10) == 1.0
    assert fit_rate(10, 9.9) == 1.0               # 3% 미만은 그대로
    assert fit_rate(12, 10) == pytest.approx(1.2)
    assert fit_rate(30, 10) == 1.25               # 한계 배속, 나머지는 자름
    assert fit_rate(5, 10) == 0.75
    assert fit_rate(0, 10) == 1.0 and fit_rate(10, 0) == 1.0 and fit_rate(float("nan"), 3) == 1.0


def test_layout_spec_legacy_and_preset():
    assert layout_spec(None)["title_bar"] is True
    ly = layout_spec(get_preset("issue"))
    assert ly["title_bar"] is False and ly["caption"] == "band" and ly["stage"][1] > 66


def test_prompt_block_asks_llm_to_judge_screens_and_keeps_text():
    text = prompt_block(get_preset("ogal"))
    assert "screens" in text and "2.8" in text and "고치지 않는다" in text and "TTS 는 쓰지 않는다" in text
    assert "screens" not in prompt_block(Preset(id="x", label="x", comments={"enabled": False}))


def test_script_task_is_verbatim_by_default_and_softens_only_when_asked():
    base = dict(title="t", body="b", comments=[], media=[], style=__import__("overshorts.script.prompts",
                fromlist=["Style"]).Style(), tone="담백", speech="반말", banned_words=[], chars_per_sec=7.0)
    plain = script_task(**base, preset=get_preset("ogal")).system
    soft = script_task(**base, preset=get_preset("ogal"), soften=True).system
    assert "바꾸거나 빼지 않는다" in plain and "순화하지 않고" in plain
    assert "욕설·혐오·특정 집단 비하·정치 댓글은 고르지 않는다" in soft
    assert "고르지 않는다" not in plain.split("댓글은 만들지 않는다")[1].split("반응이 좋은")[0]
    assert "댓글 2~3개" not in plain and "최대 27개" in plain


def test_config_defaults_off_and_typed():
    cfg = load_config(None)
    assert cfg["script"] == {"lint": False, "soften": False}
    assert cfg["render"]["edit_type"] == "ogal" and cfg["render"]["max_comments"] is None
    st = script_settings(cfg)
    assert st.soften is False and st.preset is not None and st.max_comments == 27
    cfg["render"]["edit_type"] = "game"
    cfg["render"]["max_comments"] = "5"
    assert edit_preset(cfg).id == "game" and script_settings(cfg).max_comments == 5
    assert flag(True) and flag("on") and flag("켬") and not flag("off") and not flag(None)
    assert voice_options(cfg)["lint"] is False and voice_options(cfg)["verify"] is False


def test_lint_reads_only_and_flags_personal_info():
    lines = [{"part": "body", "text": "연락처는 010-1234-5678 입니다"}, {"part": "pause", "text": ""},
             {"part": "body", "text": "정말 씨발 웃기다"}]
    res = lint_lines(lines)
    assert not res.ok and any("개인정보" in e for e in res.errors)
    assert any("욕설" in w for w in res.warnings)       # 욕설은 경고만 (순화·차단 아님)
    assert lines[0]["text"].endswith("입니다")           # 내용은 그대로
    assert lint_lines([{"part": "body", "text": "가" * 800}]).errors


def test_transcript_compare_finds_missing_misheard_repeated():
    lines = [{"part": "hook", "text": "경쟁전 세 판 연속으로 진 탱커가 있었다."},
             {"part": "body", "text": "그날 팀원들은 한마디도 하지 않았다."},
             {"part": "body", "text": "마지막에 이긴 판이 가장 어려웠다."},
             {"part": "body", "text": "끝나고 모두 같이 웃었다."}]
    heard = ("경쟁전 세 판 연속으로 진 탱커가 있었다 그날 팀원들은 한마디도 하지 않았다 "
             "그날 팀원들은 한마디도 하지 않았다 마지막에 이긴 놈이 가장 어렵다")
    kinds = {c.line: c.kind for c in compare(lines, heard)}
    assert kinds[0] == "ok" and kinds[1] == "repeated"
    assert kinds[2] in ("misheard", "ok") and kinds[3] == "missing"
    assert compare([{"part": "pause", "text": ""}], heard) == []
    assert srt_text("1\n00:00:01,000 --> 00:00:02,000\n안녕 하세요\n\n2\n00:00:02,000 --> 00:00:03,000\n반갑다") \
        == "안녕 하세요 반갑다"


def test_verify_skips_without_stt_and_writes_file(tmp_path):
    def boom(wav, cmd):
        raise RuntimeError("STT 없음")
    res = verify_voice(tmp_path, [{"part": "body", "text": "안녕하세요 여러분"}], stt=boom)
    assert res.skipped and (tmp_path / "verify.json").exists() and res.warnings()
    off = verify_voice(tmp_path, [{"part": "pause", "text": ""}], stt=boom)
    assert "TTS 꺼짐" in off.skipped


def test_validate_with_preset_uses_llm_screens_and_forces_tts_off():
    from overshorts.script.models import ScriptAnswer
    from overshorts.script.segments import validate

    cs = _comments(10, reply_of={4: 3})
    ans = ScriptAnswer.model_validate({
        "title": "제목", "composition": "A",
        "segments": [{"kind": "title", "tts": True, "lines": ["훅"]},
                     {"kind": "comments", "tts": True, "screens": [[0, 1, 2], [4], [7, 8]], "read": [0]}]})
    segs, _ = validate(ans, cs, [], "auto", 6, 6, "", None, "", get_preset("ogal"))
    com = next(s for s in segs if s["kind"] == "comments")
    assert com["pages"] == [[0, 1, 2], [3, 4], [7, 8]]   # 답글 4 → 원댓글 3 같이, 시간순
    assert all(s["tts"] is False for s in segs)            # 옵갤형: TTS 끔이 LLM 판단보다 우선
    assert len(com["page_secs"]) == 3 and all(x >= 2.8 for x in com["page_secs"])
    assert segs[0]["hold"] == 3.0
    info, _ = validate(ans, cs, [], "auto", 6, 6, "", None, "", get_preset("info"))
    assert info[0]["tts"] is True                          # 정보형: LLM 판단 유지
    old, _ = validate(ans, cs, [], "auto", 6, 6, "", None, "")  # 프리셋 없으면 옛 방식
    assert "page_secs" not in next(s for s in old if s["kind"] == "comments")


def test_frame_spec_with_preset_layout_band_and_page_times(tmp_path):
    from overshorts.render.spec import SpecOptions, build_frame_spec

    img = tmp_path / "a.img"
    img.write_bytes(b"\x89PNG\r\n\x1a\n" + b"\x00" * 8 + (16).to_bytes(4, "big") + (9).to_bytes(4, "big"))
    media = [{"name": "사진 1", "kind": "image", "path": str(img), "width": 16, "height": 9}]
    comments = [{"no": i + 1, "text": "댓글 " * 6, "nick": "ㅇㅇ"} for i in range(6)]
    script = {"title": "제목", "segments": [
        {"kind": "title", "tts": False, "lines": []},
        {"kind": "media", "tts": False, "lines": ["이 짤이 이슈다"], "media": [{"name": "사진 1", "line": 0}]},
        {"kind": "comments", "tts": False, "comments": list(range(6)), "pages": [[0, 1, 2], [3, 4, 5]],
         "picked": list(range(6))}],
        "lines": []}
    timing = {"duration": 15.0, "sentences": [],
              "segments": [{"seg": 0, "start": 0, "end": 3.0}, {"seg": 1, "start": 3.0, "end": 8.0},
                           {"seg": 2, "start": 8.0, "end": 15.0}]}
    pub = tmp_path / "public"
    pub.mkdir()
    edit = get_preset("ogal")
    spec = build_frame_spec(timing, script, {"title": "원문"}, pub, "출처", None,
                            SpecOptions(edit=edit), media, comments)
    assert spec["layout"]["title_bar"] is False and spec["edit_type"] == "ogal"
    assert spec["duration"] == 15.0 + edit.sections.outro_sec
    band = [c for c in spec["captions"] if c.get("style") == "band"]
    assert len(band) == 1 and band[0]["start"] == 3.0 + edit.caption.band_delay_sec  # 짤이 뜬 뒤 띠
    pages = {c["page"]: c["appear"] for c in spec["segments"][2]["comments"]}
    assert pages == {0: 8.0, 1: 11.5}  # 두 장이 같은 글 양 → 7초를 반씩
    legacy = build_frame_spec(timing, script, {"title": "원문"}, pub, "출처", None,
                              SpecOptions(), media, comments)
    assert legacy["layout"]["title_bar"] is True and legacy["duration"] == 15.6
