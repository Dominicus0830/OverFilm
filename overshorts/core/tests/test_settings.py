"""패널 '설정' 화면: 저장·바로 적용·설치본 config.yaml 채우기."""

import json

import pytest
import yaml

from overshorts.config import check_settings, complete_config_file, load_config, render_config
from overshorts.script.writer import apply_tts_default

INSTALLED = "# OverShorts 설정.\nstyle:\n  dir: ./style\ndata_dir: ./data\n"


def test_installed_config_gets_every_main_item(tmp_path):
    p = tmp_path / "config.yaml"
    p.write_text(INSTALLED, encoding="utf-8")
    added = complete_config_file(p)
    assert {"llm", "style", "render", "crawler", "encoder"} <= set(added)
    text = p.read_text(encoding="utf-8")
    assert "filter: ollama" in text and "title_tail: auto" in text and "# " in text
    data = yaml.safe_load(text)
    assert data["style"]["dir"] == "./style" and data["render"]["show_source"] is True
    assert (tmp_path / "config.yaml.bak").read_text(encoding="utf-8") == INSTALLED
    assert complete_config_file(p) == []


def test_render_config_round_trips():
    data = load_config(None)
    data["sources"] = [{"url": "https://gall.dcinside.com/x?id=a", "recommend_only": True}]
    data["filter"]["block_words"] = ["광고, 홍보", "#해시", "- 줄"]
    assert yaml.safe_load(render_config(data)) == data


def test_check_settings_types_and_ranges():
    got = check_settings({"crawler": {"delay_sec": [5, 2], "hourly_limit": 30, "proxy": ""},
                          "filter": {"block_words": "광고\n 홍보 \n광고"},
                          "general": {"encoder": "amf"}, "style": {"tts": "off"}})
    assert got[("crawler", "delay_sec")] == [2, 5] and got[("crawler", "proxy")] is None
    assert got[("filter", "block_words")] == ["광고", "홍보"]
    for bad in ({"crawler": {"hourly_limit": -1}}, {"render": {"show_source": "yes"}},
                {"render": {"max_media": 2.5}}, {"llm": {"filter": "gpt"}},
                {"filter": {"min_chars": 500, "max_chars": 100}}):
        with pytest.raises(ValueError):
            check_settings(bad)


def test_tts_default():
    segs = [{"kind": "title", "tts": False}, {"kind": "body", "tts": False, "lines": ["a"]},
            {"kind": "media", "tts": False, "media": [{"name": "영상 1"}]},
            {"kind": "comments", "tts": False, "picked": [0, 1, 2]}]
    apply_tts_default(segs, "on", [{"name": "영상 1", "kind": "video"}],
                      [{"text": ""}, {"text": "ㄹㅇ"}, {"text": "ㅋㅋ"}])
    assert [s["tts"] for s in segs] == [False, True, False, True]
    assert segs[3]["read"] == [1, 2]
    apply_tts_default(segs, "off", [])
    assert not any(s["tts"] for s in segs)


def test_settings_api_saves_and_applies(tmp_path):
    pytest.importorskip("fastapi")
    from fastapi.testclient import TestClient

    from overshorts.server.app import create_app

    p = tmp_path / "config.yaml"
    p.write_text(INSTALLED.replace("./data", str(tmp_path / "data")), encoding="utf-8")
    app = create_app(str(p), start_worker=False)
    c = TestClient(app)
    got = c.get("/api/settings").json()
    assert got["values"]["llm"]["filter"] == "ollama"
    assert got["fields"]["crawler.hourly_limit"]["hi"] == 10_000
    r = c.post("/api/settings", json={"values": {
        "llm": {"filter": "claude_code"}, "style": {"title_tail": "ㅋㅋㅋ"},
        "crawler": {"hourly_limit": 30}, "render": {"show_source": False},
        "general": {"data_dir": str(tmp_path / "other")}}})
    assert r.status_code == 200, r.text
    assert r.json()["restart"] == ["general.data_dir"]
    data = yaml.safe_load(p.read_text(encoding="utf-8"))
    assert data["llm"]["filter"] == "claude_code" and data["crawler"]["hourly_limit"] == 30
    assert data["style"] == {"dir": "./style", "title_tail": "ㅋㅋㅋ"}
    assert c.get("/api/status").json()["budget"]["hour_limit"] == 30  # 다시 켜지 않아도 적용
    prefs = json.loads((tmp_path / "data" / "panel_prefs.json").read_text(encoding="utf-8"))
    assert prefs["show_source"] is False
    labels = [label for label, _ in list(app.state.worker.tasks.queue)]
    assert "설정 적용" in labels and "출처 표시 끔" in labels
    assert c.post("/api/settings", json={"values": {"llm": {"filter": "gpt"}}}).status_code == 400
