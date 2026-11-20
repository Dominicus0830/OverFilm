"""설정 (기획서 6.2). `config.yaml` 은 패널이 저장하는 파일이고, MVP 동안은 손으로 고쳐도 된다.

없는 값은 기본값으로 채운다. 숫자는 믿지 않고 범위 안으로 자른다.
"""

from __future__ import annotations

import copy
import math
import re
from dataclasses import dataclass
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
               "composition": "auto", "max_media": 6, "nickname": "original",
               "bgm_dir": None, "show_source": True, "out_dir": None, "browser_executable": None,
               # 편집 유형(S17): ogal 옵갤형 | issue 이슈형 | info 정보형 | game 게임형 | <style>/presets 의 커스텀 id.
               # 아래 값이 비어(None) 있으면 유형 프리셋의 값을 쓴다.
               "edit_type": "ogal", "max_comments": None, "tts": "preset",
               "caption_max_chars": None, "caption_max_words": None},
    # 사용자 규칙(2026-11-20): 내용을 바꾸면 재미가 떨어지므로 둘 다 꺼짐이 기본이다.
    "script": {"lint": False, "soften": False},
    # 전사 대조(S4): 만든 음성을 STT 로 다시 받아 대본과 비교. stt_command 비우면 overfilm-subs extract.
    "verify": {"enabled": False, "stt_command": None},
    "style": {"dir": "../style", "min_cringe_score": 7, "speech": "반말", "title_tail": "auto",
              "tts": "auto"},
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
    # OverFilm 편집기로 넘기기: editor 비우면 설치본(%LOCALAPPDATA%\\Programs\\OverFilm)·PATH 에서 찾음
    "overfilm": {"editor": None, "open": True},
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
                       show_source=r.get("show_source", True) is not False,
                       edit=edit_preset(cfg))


def flag(value: Any) -> bool:
    """설정의 켜기/끄기: true·on·켬·1 이면 켜짐 (YAML 이 on 을 True 로 읽기도 한다)."""
    if isinstance(value, bool):
        return value
    return str(value or "").strip().lower() in ("true", "on", "yes", "1", "켬", "켜기")


def edit_preset(cfg: dict[str, Any]):
    """config 의 편집 유형 (render.edit_type) + render 덮어쓰기 값 → 프리셋. 모르는 유형이면 옵갤형."""
    from .presets import apply_overrides, get_preset

    render = cfg.get("render") or {}
    style = cfg.get("style") or {}
    return apply_overrides(get_preset(str(render.get("edit_type") or ""), style.get("dir") or "../style"),
                           render)


def voice_options(cfg: dict[str, Any]) -> dict[str, Any]:
    """voice_pending 에 넘길 옵션: 대본 린터(S3), 전사 대조(S4). 둘 다 기본 꺼짐."""
    v = cfg.get("verify") or {}
    return {"lint": flag((cfg.get("script") or {}).get("lint")),
            "style_dir": str((cfg.get("style") or {}).get("dir") or "../style"),
            "verify": flag(v.get("enabled")),
            "stt_command": str(v.get("stt_command") or "") or None}


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
        max_comments=edit_preset(cfg).comments.max_total,
        max_media=max(1, min(int(render.get("max_media") or 6), 6)),
        title_tail=str(style.get("title_tail") or "auto"),
        # 읽어 주기: 유형 프리셋이 정했거나(render.tts 로 덮어쓴 값 포함) 아니면 설정의 style.tts
        tts=(lambda pr: pr.tts if pr.tts != "auto" else str(style.get("tts") or "auto"))(edit_preset(cfg)),
        assets_dir=Path(str(cfg.get("data_dir") or "./data")) / "assets",
        preset=edit_preset(cfg),
        soften=flag((cfg.get("script") or {}).get("soften")),
    )


# --- 패널 '설정' 화면 (M8) ---
# 2026-11-15: 설치본 config.yaml 에 llm 항목이 없어 손으로 고치려다 막힌 일 → 패널에서 고르고 저장.
# "general" 구역은 config.yaml 맨 위 키(encoder, data_dir)를 뜻한다.
GENERAL = "general"


@dataclass(frozen=True)
class Setting:
    kind: str  # choice | text | path | int | float | bool | pair | words
    choices: tuple[str, ...] = ()
    lo: float = 0
    hi: float = 0
    restart: bool = False  # 패널을 다시 켜야 적용


def _choice(*opts: str) -> Setting:
    return Setting("choice", opts)


SETTINGS: dict[tuple[str, str], Setting] = {
    ("llm", "filter"): _choice("ollama", "claude_code"),
    ("llm", "script"): _choice("claude_code", "ollama"),
    ("llm", "fallback"): _choice("ollama", "claude_code"),
    ("llm", "on_limit"): _choice("ask", "fallback", "wait"),
    ("llm", "ollama_model"): Setting("text"),
    ("llm", "ollama_host"): Setting("text"),
    ("llm", "claude_model"): Setting("text"),
    ("style", "speech"): _choice("반말", "해요체"),
    ("style", "title_tail"): _choice("auto", "none", "ㄷㄷ", "ㅋㅋㅋ"),
    ("style", "tts"): _choice("auto", "on", "off"),
    ("style", "min_cringe_score"): Setting("float", lo=0, hi=10),
    ("style", "dir"): Setting("path"),
    ("render", "tone"): _choice("담백", "드립 조금", "정리형"),
    ("render", "preset"): _choice("auto", "frame", "card"),
    ("render", "composition"): _choice("auto", "A", "B"),
    ("render", "nickname"): _choice("original", "blur", "replace"),
    ("render", "show_source"): Setting("bool"),
    ("render", "max_comments"): Setting("int", lo=1, hi=60),
    ("render", "max_media"): Setting("int", lo=1, hi=6),
    ("render", "out_dir"): Setting("path"),
    ("render", "background_dir"): Setting("path"),
    ("render", "bgm_dir"): Setting("path"),
    ("render", "browser_executable"): Setting("path"),
    ("filter", "min_chars"): Setting("int", lo=0, hi=100_000),
    ("filter", "max_chars"): Setting("int", lo=1, hi=1_000_000),
    ("filter", "blur_faces"): Setting("bool"),
    ("filter", "drop_nsfw_images"): Setting("bool"),
    ("filter", "block_words"): Setting("words"),
    ("crawler", "hourly_limit"): Setting("int", lo=0, hi=10_000),
    ("crawler", "daily_limit"): Setting("int", lo=0, hi=100_000),
    ("crawler", "delay_sec"): Setting("pair", lo=0, hi=120),
    ("crawler", "rest_every"): Setting("int", lo=0, hi=1000),
    ("crawler", "rest_sec"): Setting("pair", lo=0, hi=600),
    ("crawler", "cooldown_min"): Setting("int", lo=1, hi=24 * 60),
    ("crawler", "max_comments"): Setting("int", lo=0, hi=500),
    ("crawler", "max_images"): Setting("int", lo=0, hi=6),
    ("crawler", "proxy"): Setting("text"),
    (GENERAL, "encoder"): _choice("auto", "nvenc", "amf", "cpu"),
    (GENERAL, "data_dir"): Setting("path", restart=True),
    # 편집 유형 프리셋(S17)·옵션(S3·S4)
    ("render", "edit_type"): Setting("text"),  # ogal·issue·info·game 또는 커스텀 유형 id
    ("render", "tts"): _choice("preset", "off", "auto", "on"),
    ("render", "caption_max_chars"): Setting("int", lo=4, hi=40),
    ("render", "caption_max_words"): Setting("int", lo=1, hi=8),
    ("script", "lint"): Setting("bool"),
    ("script", "soften"): Setting("bool"),
    ("verify", "enabled"): Setting("bool"),
}
_FREE_TEXT = re.compile(r"^[\w.:\-/@가-힣 ]{0,120}$")


def _section(cfg: dict[str, Any], sec: str) -> dict[str, Any]:
    if sec == GENERAL:
        return cfg
    got = cfg.get(sec)
    return got if isinstance(got, dict) else {}


def settings_view(cfg: dict[str, Any]) -> dict[str, Any]:
    """패널에 보일 지금 값과 항목 종류(고를 값·범위)."""
    values: dict[str, dict[str, Any]] = {}
    for (sec, key), spec in SETTINGS.items():
        v = _section(cfg, sec).get(key)
        if spec.kind == "path" and v is None:
            v = ""
        values.setdefault(sec, {})[key] = v
    fields = {f"{sec}.{key}": {"kind": s.kind, "choices": list(s.choices), "lo": s.lo, "hi": s.hi,
                               "restart": s.restart}
              for (sec, key), s in SETTINGS.items()}
    return {"values": values, "fields": fields,
            "choices": {k: f["choices"] for k, f in fields.items() if f["choices"]}}


def _check_num(name: str, value: Any, spec: Setting, integer: bool) -> float:
    if isinstance(value, bool):
        raise ValueError(f"{name} 는 숫자여야 합니다")
    try:
        x = float(value)
    except (TypeError, ValueError):
        raise ValueError(f"{name} 는 숫자여야 합니다") from None
    if math.isnan(x) or not spec.lo <= x <= spec.hi:
        raise ValueError(f"{name} 는 {spec.lo:g}~{spec.hi:g} 사이여야 합니다")
    if integer and x != int(x):
        raise ValueError(f"{name} 는 정수여야 합니다")
    return x


def _check_value(name: str, value: Any, spec: Setting) -> Any:
    if spec.kind == "bool":
        if not isinstance(value, bool):
            raise ValueError(f"{name} 는 켬/끔이어야 합니다")
        return value
    if spec.kind == "int":
        return int(_check_num(name, value, spec, True))
    if spec.kind == "float":
        x = _check_num(name, value, spec, False)
        return int(x) if x == int(x) else x
    if spec.kind == "pair":
        if not isinstance(value, (list, tuple)) or len(value) != 2:
            raise ValueError(f"{name} 는 [최소, 최대] 두 숫자여야 합니다")
        a, b = sorted(_check_num(name, v, spec, False) for v in value)
        return [int(x) if x == int(x) else x for x in (a, b)]
    if spec.kind == "words":
        items = value.splitlines() if isinstance(value, str) else value
        if not isinstance(items, (list, tuple)):
            raise ValueError(f"{name} 는 낱말 목록이어야 합니다")
        words = [str(w).strip() for w in items if isinstance(w, (str, int)) and str(w).strip()]
        if len(words) > 500 or any(len(w) > 60 or not w.isprintable() for w in words):
            raise ValueError(f"{name} 는 60자 이하 낱말 500개까지입니다")
        return list(dict.fromkeys(words))
    text = "" if value is None else str(value).strip()
    if spec.kind == "choice":
        if text not in spec.choices:
            raise ValueError(f"{name} 는 {', '.join(spec.choices)} 중 하나여야 합니다")
        return text
    if spec.kind == "path":
        if len(text) > 400 or not text.isprintable():
            raise ValueError(f"{name} 경로가 너무 길거나 쓸 수 없는 글자가 있습니다")
        return text or None
    if not _FREE_TEXT.match(text):
        raise ValueError(f"{name} 값이 너무 길거나 쓸 수 없는 글자가 있습니다")
    if key_is_nullable(name) and not text:
        return None
    return text


def key_is_nullable(name: str) -> bool:
    """비우면 '없음'(null)이 되는 글자 항목."""
    return name == "crawler.proxy"


def check_settings(changes: dict[str, Any]) -> dict[tuple[str, str], Any]:
    """패널이 보낸 값 검사. 모르는 항목·범위 밖 값은 ValueError."""
    out: dict[tuple[str, str], Any] = {}
    for sec, items in (changes or {}).items():
        if not isinstance(items, dict):
            raise ValueError(f"'{sec}' 형식이 맞지 않습니다")
        for key, value in items.items():
            spec = SETTINGS.get((str(sec), str(key)))
            if spec is None:
                raise ValueError(f"바꿀 수 없는 설정: {sec}.{key}")
            out[(str(sec), str(key))] = _check_value(f"{sec}.{key}", value, spec)
    max_c = out.get(("filter", "max_chars"))
    min_c = out.get(("filter", "min_chars"))
    if isinstance(max_c, int) and isinstance(min_c, int) and min_c > max_c:
        raise ValueError("filter.min_chars 가 max_chars 보다 큽니다")
    return out


def apply_settings(cfg: dict[str, Any], changes: dict[tuple[str, str], Any]) -> None:
    """검사한 값을 메모리의 설정(dict)에 넣는다."""
    for (sec, key), value in changes.items():
        if sec == GENERAL:
            cfg[key] = value
            continue
        if not isinstance(cfg.get(sec), dict):
            cfg[sec] = {}
        cfg[sec][key] = value


def _read_yaml(p: Path) -> dict[str, Any]:
    if not p.exists():
        return {}
    loaded = yaml.safe_load(p.read_text(encoding="utf-8"))
    return loaded if isinstance(loaded, dict) else {}


def _write_atomic(p: Path, text: str) -> None:
    tmp = p.with_suffix(".yaml.tmp")
    tmp.write_text(text, encoding="utf-8")
    tmp.replace(p)


def save_settings(path: str | Path, changes: dict[tuple[str, str], Any]) -> None:
    """config.yaml 에 바뀐 값만 쓴다 (다른 항목은 그대로, 설명 주석을 붙여 다시 씀). 파일이 없으면 만든다."""
    p = Path(path)
    try:
        data = _read_yaml(p)
    except yaml.YAMLError as err:
        raise OSError(f"config.yaml 을 읽지 못했습니다 (문법 오류): {err}") from err
    apply_settings(data, changes)
    _write_atomic(p, render_config(data))


# 설정 파일에 붙는 설명 (패널 '설정' 화면과 같은 말)
_HEADER = ("# OverShorts 설정. 패널 '설정' 화면에서 바꾸면 이 파일에 저장됩니다. 직접 고쳐도 됩니다.\n"
           "# 고친 뒤에는 패널을 다시 켜야 적용됩니다 (패널에서 바꾼 값은 바로 적용).\n")
_SECTION_NOTES = {
    "sources": "갤러리 자동 수집 목록 (url, recommend_only, min_upvotes, max_posts, within_hours)",
    "filter": "글 거르기",
    "llm": "AI 모델 (filter = 글 거르기, script = 대본·오글 검사)",
    "voice": "목소리 (패널 '보이스' 화면)",
    "render": "영상 만들기",
    "style": "대본 말투",
    "crawler": "수집 예산·속도 (디시 차단을 피하려고 천천히)",
}
_NOTES = {
    ("filter", "block_words"): "이 낱말이 든 글은 받지 않음",
    ("filter", "allow_topics"): "비우면 전부 허용",
    ("filter", "min_chars"): "제목+본문 글자 수 (짧은 뻘글도 받음)",
    ("filter", "blur_faces"): "첨부 그림 얼굴 흐림",
    ("filter", "drop_nsfw_images"): "선정성 그림 빼기",
    ("llm", "filter"): "ollama (로컬 GPU, 라데온은 Ollama AMD) | claude_code (구독, claude -p)",
    ("llm", "script"): "claude_code | ollama",
    ("llm", "fallback"): "Claude 한도·오류 때 넘길 모델",
    ("llm", "ollama_model"): "VRAM 6~7GB면 qwen3:4b",
    ("llm", "claude_model"): "비우면 Claude Code 기본 모델",
    ("llm", "on_limit"): "ask = 패널이 물음 | fallback = 로컬 LLM 으로 | wait = 대기열에 둠",
    ("render", "tone"): "담백 | 드립 조금 | 정리형",
    ("render", "preset"): "auto = 원문 사진·영상 있으면 프레임형, 없으면 썰 카드형 | frame | card",
    ("render", "composition"): "auto = AI 가 A(제목→본문→댓글)/B(제목→본문) 고름 | A | B",
    ("render", "max_comments"): "영상 전체 댓글 최대 (답글 포함, 1~60). 비우면 편집 유형 값",
    ("render", "edit_type"): "편집 유형: ogal 옵갤형 | issue 이슈형 | info 정보형 | game 게임형 | 커스텀 id",
    ("render", "tts"): "preset = 편집 유형을 따름 | off | auto | on",
    ("render", "caption_max_chars"): "쇼츠 자막 한 줄 최대 글자 (비우면 편집 유형 값, 편집기 25자와 별개)",
    ("render", "caption_max_words"): "한 번에 보이는 최대 어절 (비우면 편집 유형 값)",
    ("script", "lint"): "대본 린터: 읽기 전 개인정보·금지 표현·길이 검사 (끔이 기본, 내용은 고치지 않음)",
    ("script", "soften"): "순화: 욕설·혐오 표현을 피함 (끔이 기본 = 원문 그대로)",
    ("verify", "enabled"): "전사 대조: 만든 음성을 STT 로 다시 받아 대본과 비교 (끔이 기본)",
    ("render", "max_media"): "한 영상에 넣을 원문 미디어 최대 (1~6)",
    ("render", "nickname"): "original = 원본 그대로 | blur = 흐림 | replace = ㅇㅇ 치환",
    ("render", "bgm_dir"): "무음이 5초 넘지 않게 깔 BGM (라이선스 있는 것만)",
    ("render", "background_dir"): "직접 녹화했거나 라이선스 있는 배경 영상만",
    ("render", "show_source"): "영상 아래 '출처: DC인사이드 …' 표시",
    ("render", "out_dir"): "다 만든 영상 폴더 (비우면 데이터 폴더/approved)",
    ("render", "browser_executable"): "비우면 Remotion 이 받은 Chrome",
    ("style", "dir"): "말투 예시·금지 표현·짤 폴더",
    ("style", "speech"): "반말 | 해요체 (채널당 하나)",
    ("style", "min_cringe_score"): "오글 검사 기준 (0~10, 낮으면 다시 씀)",
    ("style", "title_tail"): "제목 꼬리표: auto (글 분위기에 맞춰 0~1개) | none | ㄷㄷ | ㅋㅋㅋ",
    ("style", "tts"): "읽어 주기: auto = AI 가 장면마다 정함 | on = 본문·댓글 읽음 | off = 자막만",
    ("crawler", "delay_sec"): "요청마다 랜덤 간격 (1초 미만은 차단 위험)",
    ("crawler", "rest_every"): "글 N개마다 긴 휴식",
    ("crawler", "hourly_limit"): "시간당 요청 상한",
    ("crawler", "daily_limit"): "하루 요청 상한",
    ("crawler", "cooldown_min"): "403·429·캡차 뒤 쉬는 시간 (분)",
    ("crawler", "max_images"): "글당 받을 첨부 사진 (최대 6)",
    (GENERAL, "encoder"): "auto | nvenc | amf (라데온) | cpu",
    (GENERAL, "data_dir"): "작업·DB 폴더 (바꾸면 패널을 다시 켜야 함)",
}


def _flow(value: Any) -> str:
    """한 줄 YAML 값."""
    text = yaml.safe_dump([value], default_flow_style=True, allow_unicode=True, width=1_000_000)
    return text.strip()[1:-1]


def _block(value: Any, indent: int) -> str:
    text = yaml.safe_dump(value, default_flow_style=False, allow_unicode=True, sort_keys=False,
                          width=1_000_000)
    pad = " " * indent
    return "".join(pad + ln + "\n" for ln in text.splitlines())


def _line(key: str, value: Any, note: str | None, indent: int) -> str:
    pad = " " * indent
    if isinstance(value, dict) or (isinstance(value, list) and any(isinstance(v, (dict, list))
                                                                     for v in value)):
        head = f"{pad}{key}:" + (f"  # {note}" if note else "") + "\n"
        return head + (_block(value, indent + 2) if value else "")
    return f"{pad}{key}: {_flow(value)}" + (f"  # {note}" if note else "") + "\n"


def _ordered(data: dict[str, Any], ref: dict[str, Any]) -> list[str]:
    return [k for k in ref if k in data] + [k for k in data if k not in ref]


def render_config(data: dict[str, Any]) -> str:
    """설정 dict → 설명 주석이 붙은 config.yaml 글. 다시 읽으면 같은 값이 나온다."""
    out = [_HEADER]
    for key in _ordered(data, DEFAULTS):
        value = data[key]
        sec_ref = DEFAULTS.get(key)
        if isinstance(value, dict) and isinstance(sec_ref, dict):
            note = _SECTION_NOTES.get(key)
            out.append("\n" + (f"# {note}\n" if note else "") + f"{key}:\n")
            for sub in _ordered(value, sec_ref):
                out.append(_line(str(sub), value[sub], _NOTES.get((key, str(sub))), 2))
            continue
        note = _NOTES.get((GENERAL, key)) or _SECTION_NOTES.get(key)
        out.append(("\n" if isinstance(value, (dict, list)) else "") + _line(str(key), value, note, 0))
    return "".join(out)


def complete_config_file(path: str | Path) -> list[str]:
    """config.yaml 에 빠진 항목을 기본값으로 채워, 손으로 고칠 수 있게 모든 주요 항목을 보이게 한다.

    있는 값은 바꾸지 않는다. 채운 게 있으면 원래 파일을 config.yaml.bak 에 남기고 설명 주석을 붙여
    다시 쓴다. 채운 구역(맨 위 키) 이름 목록을 돌려준다. 파일이 없거나 읽을 수 없으면 아무것도 안 한다.
    """
    p = Path(path)
    try:
        text = p.read_text(encoding="utf-8")
        loaded = yaml.safe_load(text)
    except (OSError, yaml.YAMLError):
        return []
    if loaded is not None and not isinstance(loaded, dict):
        return []
    data: dict[str, Any] = loaded or {}
    added: list[str] = []
    for key, default in DEFAULTS.items():
        if key not in data:
            data[key] = copy.deepcopy(default)
            added.append(key)
        elif isinstance(default, dict) and isinstance(data[key], dict):
            missing = [k for k in default if k not in data[key]]
            for k in missing:
                data[key][k] = copy.deepcopy(default[k])
            if missing:
                added.append(key)
    if not added:
        return []
    try:
        p.with_name(p.name + ".bak").write_text(text, encoding="utf-8")
        _write_atomic(p, render_config(data))
    except OSError:
        return []
    return added
