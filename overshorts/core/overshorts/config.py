"""설정 (기획서 6.2). `config.yaml` 은 패널이 저장하는 파일이고, MVP 동안은 손으로 고쳐도 된다.

없는 값은 기본값으로 채운다. 숫자는 믿지 않고 범위 안으로 자른다.
"""

from __future__ import annotations

import copy
import re
from pathlib import Path
from typing import Any

import yaml

from .crawler.budget import CrawlLimits
from .filters.rules import RuleConfig

DEFAULTS: dict[str, Any] = {
    "sources": [],
    "filter": {
        "block_words": [],
        "allow_topics": [],
        "block_categories": [
            "politics", "hate", "sexual", "real_person", "personal_info", "crime_selfharm", "ad",
        ],
        "min_chars": 15,
        "max_chars": 4000,
        "blur_faces": True,
        "drop_nsfw_images": True,
    },
    "llm": {
        "filter": "ollama",
        "script": "claude_code",
        "fallback": "ollama",
        "ollama_model": "qwen3:8b",
        "ollama_host": "http://127.0.0.1:11434",
        "claude_model": "",
        "on_limit": "ask",
    },
    "voice": {"narrator": "edge:ko-KR-SunHiNeural", "comment": "edge:ko-KR-InJoonNeural"},
    "render": {"preset": "auto", "tone": "담백", "background_dir": "./assets/bg",
               "composition": "auto", "max_comments": 6, "max_media": 6, "nickname": "original",
               "bgm_dir": None},
    "style": {"dir": "../style", "min_cringe_score": 7, "speech": "반말", "title_tail": "auto"},
    "crawler": {
        "delay_sec": [1.5, 4],
        "rest_every": 10,
        "rest_sec": [20, 40],
        "hourly_limit": 60,
        "daily_limit": 200,
        "cooldown_min": 60,
        "max_comments": 50,
        "max_images": 6,
        "proxy": None,
    },
    "gpu_tier": "auto",
    # 라데온도 지원 (사용자 결정 2026-11-14): auto 는 NVIDIA면 nvenc, AMD면 amf, 없으면 cpu.
    "encoder": "auto",
    "data_dir": "./data",
}


def _merge(base: dict[str, Any], over: dict[str, Any]) -> dict[str, Any]:
    out = copy.deepcopy(base)
    for k, v in (over or {}).items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = _merge(out[k], v)
        else:
            out[k] = v
    return out


def load_config(path: str | Path | None) -> dict[str, Any]:
    data: dict[str, Any] = {}
    if path is not None and Path(path).exists():
        with open(path, encoding="utf-8") as f:
            loaded = yaml.safe_load(f)
        if isinstance(loaded, dict):
            data = loaded
    return _merge(DEFAULTS, data)


def _num(value: Any, default: float, lo: float, hi: float) -> float:
    try:
        x = float(value)
    except (TypeError, ValueError):
        return default
    if x != x:  # NaN
        return default
    return max(lo, min(hi, x))


def _pair(value: Any, default: tuple[float, float], lo: float, hi: float) -> tuple[float, float]:
    if isinstance(value, (list, tuple)) and len(value) == 2:
        a = _num(value[0], default[0], lo, hi)
        b = _num(value[1], default[1], lo, hi)
        return (min(a, b), max(a, b))
    return default


def crawl_limits(cfg: dict[str, Any]) -> CrawlLimits:
    c = cfg.get("crawler") or {}
    d = CrawlLimits()
    return CrawlLimits(
        delay_sec=_pair(c.get("delay_sec"), d.delay_sec, 0.0, 120.0),
        rest_every=int(_num(c.get("rest_every"), d.rest_every, 0, 1000)),
        rest_sec=_pair(c.get("rest_sec"), d.rest_sec, 0.0, 600.0),
        hourly_limit=int(_num(c.get("hourly_limit"), d.hourly_limit, 0, 10_000)),
        daily_limit=int(_num(c.get("daily_limit"), d.daily_limit, 0, 100_000)),
        cooldown_min=int(_num(c.get("cooldown_min"), d.cooldown_min, 1, 24 * 60)),
    )


def rule_config(cfg: dict[str, Any]) -> RuleConfig:
    f = cfg.get("filter") or {}
    words = f.get("block_words") or []
    return RuleConfig(
        min_chars=int(_num(f.get("min_chars"), 15, 0, 100_000)),
        max_chars=int(_num(f.get("max_chars"), 4000, 1, 1_000_000)),
        block_words=[str(w) for w in words if isinstance(w, (str, int)) and str(w).strip()],
    )


def spec_options(cfg: dict[str, Any]):
    from .render.spec import SpecOptions

    r = cfg.get("render") or {}
    bgm = r.get("bgm_dir")
    return SpecOptions(preset=str(r.get("preset") or "auto"),
                       nickname=str(r.get("nickname") or "original"),
                       bgm_dir=Path(str(bgm)) if bgm else None,
                       show_source=r.get("show_source", True) is not False)


def script_settings(cfg: dict[str, Any], tone: str | None = None, default_style: str = "../style"):
    from .script.writer import ScriptSettings

    render = cfg.get("render") or {}
    style = cfg.get("style") or {}
    return ScriptSettings(
        style_dir=style.get("dir") or default_style,
        tone=tone or str(render.get("tone") or "담백"),
        speech=str(style.get("speech") or "반말"),
        min_cringe_score=float(style.get("min_cringe_score") or 7),
        composition=str(render.get("composition") or "auto"),
        max_comments=max(1, min(int(render.get("max_comments") or 6), 10)),
        max_media=max(1, min(int(render.get("max_media") or 6), 6)),
        title_tail=str(style.get("title_tail") or "auto"),
        assets_dir=Path(str(cfg.get("data_dir") or "./data")) / "assets",
    )


# --- 패널 설정 화면 (2026-11-15: 설치본 config.yaml 에 llm 항목이 없어 손으로 못 고친 일) ---

# 패널에서 바꿀 수 있는 값: (구역, 키) → 고를 수 있는 값 (None = 짧은 글자 자유 입력)
SETTINGS: dict[tuple[str, str], tuple[str, ...] | None] = {
    ("llm", "filter"): ("ollama", "claude_code"),
    ("llm", "script"): ("claude_code", "ollama"),
    ("llm", "fallback"): ("ollama", "claude_code"),
    ("llm", "ollama_model"): None,
    ("llm", "claude_model"): None,
    ("llm", "on_limit"): ("ask", "fallback", "wait"),
    ("render", "tone"): ("담백", "드립 조금", "정리형"),
    ("render", "preset"): ("auto", "frame", "card"),
    ("render", "nickname"): ("original", "blur", "replace"),
    ("style", "speech"): ("반말", "해요체"),
    ("style", "title_tail"): ("auto", "none", "ㄷㄷ", "ㅋㅋㅋ"),
    ("voice", "narrator"): None,
    ("voice", "comment"): None,
}
_FREE_TEXT = re.compile(r"^[\w.:\-/가-힣 ]{0,80}$")


def settings_view(cfg: dict[str, Any]) -> dict[str, Any]:
    """패널에 보일 지금 값과 고를 수 있는 값."""
    values: dict[str, dict[str, Any]] = {}
    for (sec, key) in SETTINGS:
        values.setdefault(sec, {})[key] = (cfg.get(sec) or {}).get(key)
    return {"values": values,
            "choices": {f"{sec}.{key}": list(opts) for (sec, key), opts in SETTINGS.items() if opts}}


def check_settings(changes: dict[str, Any]) -> dict[tuple[str, str], str]:
    """패널이 보낸 값 검사. 모르는 항목·고를 수 없는 값은 ValueError."""
    out: dict[tuple[str, str], str] = {}
    for sec, items in (changes or {}).items():
        if not isinstance(items, dict):
            raise ValueError(f"'{sec}' 형식이 맞지 않습니다")
        for key, value in items.items():
            spec = (str(sec), str(key))
            if spec not in SETTINGS:
                raise ValueError(f"바꿀 수 없는 설정: {sec}.{key}")
            text = "" if value is None else str(value).strip()
            opts = SETTINGS[spec]
            if opts is not None and text not in opts:
                raise ValueError(f"{sec}.{key} 는 {', '.join(opts)} 중 하나여야 합니다")
            if opts is None and not _FREE_TEXT.match(text):
                raise ValueError(f"{sec}.{key} 값이 너무 길거나 쓸 수 없는 글자가 있습니다")
            out[spec] = text
    return out


def save_settings(path: str | Path, changes: dict[tuple[str, str], str]) -> None:
    """config.yaml 에 바뀐 값만 쓴다 (다른 항목은 그대로). 파일이 없으면 만든다."""
    p = Path(path)
    data: dict[str, Any] = {}
    if p.exists():
        loaded = yaml.safe_load(p.read_text(encoding="utf-8"))
        if isinstance(loaded, dict):
            data = loaded
    for (sec, key), value in changes.items():
        if not isinstance(data.get(sec), dict):
            data[sec] = {}
        data[sec][key] = value
    tmp = p.with_suffix(".yaml.tmp")
    tmp.write_text("# OverShorts 설정. 패널 '설정' 화면에서 바꾸거나 직접 고쳐도 됩니다.\n"
                   + yaml.safe_dump(data, allow_unicode=True, sort_keys=False), encoding="utf-8")
    tmp.replace(p)


# 설정 파일에 없으면 붙여 넣을 주요 구역 (손으로 고칠 수 있게 기본값을 눈에 보이게 둔다)
_STARTER_SECTIONS = ("llm", "voice", "render", "filter", "crawler")


def complete_config_file(path: str | Path) -> list[str]:
    """config.yaml 에 주요 구역이 통째로 없으면 기본값으로 끝에 붙인다. 있는 값은 건드리지 않는다.

    붙인 구역 이름 목록을 돌려준다. 파일이 없거나 읽을 수 없으면 아무것도 하지 않는다.
    """
    p = Path(path)
    try:
        text = p.read_text(encoding="utf-8")
        loaded = yaml.safe_load(text)
    except (OSError, yaml.YAMLError):
        return []
    data = loaded if isinstance(loaded, dict) else {}
    missing = [s for s in _STARTER_SECTIONS if s not in data]
    if not missing:
        return []
    block = {s: DEFAULTS[s] for s in missing}
    add = ("\n# --- 기본값 (OverShorts 가 채움, 고쳐도 됩니다. 항목 설명은 config.example.yaml) ---\n"
           + yaml.safe_dump(block, allow_unicode=True, sort_keys=False))
    try:
        with open(p, "a", encoding="utf-8") as f:
            f.write(("" if text.endswith("\n") or not text else "\n") + add)
    except OSError:
        return []
    return missing
