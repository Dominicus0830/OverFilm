"""설정 (기획서 6.2). `config.yaml` 은 패널이 저장하는 파일이고, MVP 동안은 손으로 고쳐도 된다.

없는 값은 기본값으로 채운다. 숫자는 믿지 않고 범위 안으로 자른다.
"""

from __future__ import annotations

import copy
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
        "min_chars": 80,
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
    "style": {"dir": "../style", "min_cringe_score": 7, "speech": "반말"},
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
        min_chars=int(_num(f.get("min_chars"), 80, 0, 100_000)),
        max_chars=int(_num(f.get("max_chars"), 4000, 1, 1_000_000)),
        block_words=[str(w) for w in words if isinstance(w, (str, int)) and str(w).strip()],
    )


def spec_options(cfg: dict[str, Any]):
    from .render.spec import SpecOptions

    r = cfg.get("render") or {}
    bgm = r.get("bgm_dir")
    return SpecOptions(preset=str(r.get("preset") or "auto"),
                       nickname=str(r.get("nickname") or "original"),
                       bgm_dir=Path(str(bgm)) if bgm else None)


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
    )
