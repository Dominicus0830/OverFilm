"""유형별 편집 스타일 프리셋 (S17). 이슈·옵갤·정보·게임 4종을 JSON 파일로 정의하고 고르게 한다.

프리셋 파일 하나 = 편집 방식 하나. 유형마다 구간 배분, 화면 배치(제목 바·자막 모양), 댓글 노출,
효과, TTS 사용 여부가 다르다. 내장 4종은 이 패키지의 `presets/*.json`, 사용자가 만든 유형은
`<style 폴더>/presets/*.json` 에 같은 형식으로 두면 같은 id 의 내장 유형을 덮어쓰거나 새 유형으로 추가된다
(커스텀 유형: Claude Code 와 대화로 규칙을 만들어 이 형식으로 저장하는 것은 다음 단계, S18).

스키마는 `Preset` 하나뿐이다. 모르는 칸은 무시하고 빠진 칸은 기본값으로 채운다. [최소, 최대] 쌍은 범위 안으로
고쳐 쓰고, 그 밖의 숫자가 범위를 벗어나거나 형식이 틀린 파일은 경고만 남기고 건너뛴다 (죽지 않는다).

기본값 원칙 (사용자 지정 2026-11-20): 내용을 바꾸면 재미가 떨어지므로 프리셋은 제목·댓글·본문을 고치지 않는다.
순화·대본 린터는 프리셋이 아니라 별도 옵션(config `script.soften`·`script.lint`)이고 기본은 꺼짐이다.
"""

from __future__ import annotations

import json
import logging
import re
from functools import lru_cache
from pathlib import Path
from typing import Any, Literal

from pydantic import BaseModel, Field, ValidationError, field_validator

log = logging.getLogger(__name__)

SCHEMA_VERSION = 1
BUILTIN_DIR = Path(__file__).with_name("presets")
DEFAULT_PRESET = "ogal"
_ID = re.compile(r"^[a-z0-9][a-z0-9_-]{0,31}$")


def _pair(v: Any, lo: float, hi: float) -> tuple[float, float]:
    """[최소, 최대] 두 수. 순서가 바뀌었거나 범위 밖이면 고친다."""
    try:
        a, b = float(v[0]), float(v[1])
    except (TypeError, ValueError, IndexError, KeyError):
        raise ValueError("두 수 [최소, 최대] 여야 합니다") from None
    if a != a or b != b:  # NaN
        raise ValueError("숫자가 아닙니다")
    a, b = max(lo, min(hi, a)), max(lo, min(hi, b))
    return (min(a, b), max(a, b))


class Sections(BaseModel):
    """구간 배분 (LLM 에게 주는 안내이자 길이 검사 기준). 초 단위."""
    target_sec: tuple[float, float] = (30.0, 40.0)
    hook_sec: tuple[float, float] = (2.0, 4.0)       # 글 머리
    media_sec: tuple[float, float] = (3.0, 6.0)      # 본문 짤·글
    outro_sec: float = Field(default=1.0, ge=0, le=10)  # 검정 엔딩
    comment_share: float = Field(default=0.7, ge=0, le=0.95)  # 전체 중 댓글 비율
    max_clip_sec: float = Field(default=8.0, ge=2, le=60)  # 원문 영상 한 개를 보여 주는 최대 시간

    @field_validator("target_sec", "hook_sec", "media_sec", mode="before")
    @classmethod
    def _p(cls, v: Any) -> tuple[float, float]:
        return _pair(v, 0.0, 180.0)


class Layout(BaseModel):
    """화면 배치. 세로 위치는 화면 높이 % (플랫폼 UI에 가려지지 않는 안전영역)."""
    title_bar: bool = False                       # 위 고정 큰 제목 (끄면 유튜브 제목이 제목 역할)
    caption: Literal["yellow", "band", "none"] = "none"  # yellow=노란 2~4단어, band=흰 띠 한 줄, none=없음
    stage_top: float = Field(default=10.0, ge=0, le=60)
    stage_bottom: float = Field(default=76.0, ge=30, le=100)
    caption_top: float = Field(default=70.0, ge=0, le=100)
    caption_bottom: float = Field(default=76.0, ge=0, le=100)
    source_top: float = Field(default=77.5, ge=0, le=100)
    safe_bottom: float = Field(default=80.0, ge=50, le=100)  # 이 아래는 비움 (좋아요·설명이 덮는 자리)


class CaptionRules(BaseModel):
    """쇼츠용 자막 (OverFilm 편집기 자막 한 줄 25자와 별개). S1·S14."""
    max_chars: int = Field(default=13, ge=4, le=40)      # 한 줄 최대 글자 (74px 글씨 한 줄 = 13)
    max_words: int = Field(default=4, ge=1, le=8)        # 한 번에 보이는 최대 어절
    min_show_sec: float = Field(default=0.4, ge=0.1, le=3)  # 한 묶음이 화면에 있는 최소 시간
    band_delay_sec: float = Field(default=3.0, ge=0, le=20)  # band: 짤이 뜬 뒤 띠가 나오기까지


class CommentRules(BaseModel):
    """댓글 화면 (S19). 몇 개를 한 화면에 보일지는 LLM 이 이 범위 안에서 판단한다."""
    enabled: bool = True
    per_screen: tuple[int, int] = (1, 3)      # 한 화면 권장 개수 (한 줄 댓글이 많으면 위쪽, 디시콘·긴 글이면 아래쪽)
    max_total: int = Field(default=27, ge=1, le=60)  # 영상 전체 최대 댓글 수 (답글 포함)
    max_screens: int = Field(default=9, ge=1, le=20)
    min_screen_sec: float = Field(default=2.8, ge=1, le=10)  # 장당 최소 시간 (레퍼런스는 2.8초 칼같이)
    max_screen_sec: float = Field(default=8.0, ge=2, le=20)
    base_sec: float = Field(default=1.2, ge=0, le=5)         # 읽는 시간 = base + 글자 수 × per_char
    per_char_sec: float = Field(default=0.05, ge=0.01, le=0.3)
    dccon_sec: float = Field(default=0.6, ge=0, le=3)        # 디시콘 하나당 더 보는 시간
    skip: Literal["time_order", "contiguous"] = "time_order"
    # time_order = 시간순으로 건너뛰며 반응 흐름이 이어지게 고른다 / contiguous = 고른 글타래부터 이어지는 댓글로 칸을 채운다

    @field_validator("per_screen", mode="before")
    @classmethod
    def _ps(cls, v: Any) -> tuple[int, int]:
        a, b = _pair(v, 1, 8)
        return (int(a), int(b))


class Effects(BaseModel):
    bgm: bool = True                 # BGM 깔기 (폴더가 있을 때)
    sfx: bool = False                # 짤에 맞는 효과음 (소스 풀이 생기면; 지금은 표시만)
    original_sound: bool = False     # 원문 영상 소리 우선 (게임 클립)


class Reference(BaseModel):
    """이 유형의 참고 영상 (커스텀 유형은 사용자가 고르는 예시 영상, 선택 사항)."""
    videos: list[str] = Field(default_factory=list, max_length=10)
    note: str = Field(default="", max_length=400)

    @field_validator("videos", mode="before")
    @classmethod
    def _v(cls, v: Any) -> list[str]:
        return [str(x)[:300] for x in v][:10] if isinstance(v, list) else []


class Preset(BaseModel):
    version: int = SCHEMA_VERSION
    id: str
    label: str = Field(max_length=20)
    description: str = Field(default="", max_length=300)
    origin: Literal["builtin", "custom"] = "builtin"
    tts: Literal["off", "auto", "on"] = "off"   # off=읽지 않음, auto=세그먼트마다 LLM 이 판단, on=전부 읽음
    # 갤글 그대로: 원문에 없는 내용(새 제목·해설·요약·소제목·바깥 짤)을 더하지 않는다 (옵갤형, 2026-11-21 사용자 규칙)
    verbatim: bool = False
    sections: Sections = Field(default_factory=Sections)
    layout: Layout = Field(default_factory=Layout)
    caption: CaptionRules = Field(default_factory=CaptionRules)
    comments: CommentRules = Field(default_factory=CommentRules)
    effects: Effects = Field(default_factory=Effects)
    guide: str = Field(default="", max_length=3000)  # 이 유형만의 편집 규칙 (LLM 에게 그대로 전달)
    reference: Reference = Field(default_factory=Reference)

    @field_validator("id")
    @classmethod
    def _id(cls, v: str) -> str:
        if not _ID.match(v or ""):
            raise ValueError("id 는 영소문자·숫자·-·_ 1~32자")
        return v

    def stage_height_px(self, height: int = 1920) -> float:
        return (self.layout.stage_bottom - self.layout.stage_top) / 100 * height


def parse_preset(data: Any, origin: str | None = None) -> Preset:
    """dict → Preset. 모르는 칸은 버린다. 잘못된 값이면 ValidationError."""
    if not isinstance(data, dict):
        raise ValueError("프리셋은 JSON 객체여야 합니다")
    d = dict(data)
    if origin:
        d["origin"] = origin
    return Preset.model_validate(d)


def _read(path: Path, origin: str) -> Preset | None:
    try:
        return parse_preset(json.loads(path.read_text(encoding="utf-8")), origin)
    except (OSError, ValueError, ValidationError) as err:
        log.warning("프리셋 %s 를 읽지 못했습니다: %s", path.name, err)
        return None


@lru_cache(maxsize=1)
def _builtin() -> tuple[Preset, ...]:
    out = []
    try:
        files = sorted(BUILTIN_DIR.glob("*.json"))
    except OSError:
        files = []
    for p in files:
        pr = _read(p, "builtin")
        if pr is not None:
            out.append(pr)
    return tuple(out)


def load_presets(style_dir: str | Path | None = None) -> dict[str, Preset]:
    """내장 4종 + `<style>/presets/*.json` (같은 id 면 사용자가 만든 것이 우선). 잘못된 파일은 건너뛴다."""
    out = {p.id: p for p in _builtin()}
    if style_dir is not None:
        try:
            files = sorted((Path(style_dir) / "presets").glob("*.json"))[:100]
        except OSError:
            files = []
        for f in files:
            pr = _read(f, "custom")
            if pr is not None:
                out[pr.id] = pr
    return out


def get_preset(preset_id: str | None, style_dir: str | Path | None = None) -> Preset:
    """id 로 찾는다. 모르는 id 면 기본(옵갤형), 내장 파일까지 못 읽으면 코드 기본값."""
    table = load_presets(style_dir)
    pid = str(preset_id or DEFAULT_PRESET)
    if pid in table:
        return table[pid]
    if DEFAULT_PRESET in table:
        return table[DEFAULT_PRESET]
    return Preset(id=DEFAULT_PRESET, label="옵갤형")


def apply_overrides(preset: Preset, render_cfg: dict[str, Any]) -> Preset:
    """config 의 render 칸이 프리셋 값을 덮어쓴다 (비어 있으면 프리셋 값).

    caption_max_chars / caption_max_words: 쇼츠 자막 줄 길이 (S1·S14)
    max_comments: 영상 전체 댓글 수 상한   tts: preset | off | auto | on
    """
    p = preset.model_copy(deep=True)

    def num(key: str, lo: int, hi: int) -> int | None:
        v = render_cfg.get(key)
        try:
            return max(lo, min(hi, int(v))) if v not in (None, "", "preset") else None
        except (TypeError, ValueError):
            return None

    if (v := num("caption_max_chars", 4, 40)) is not None:
        p.caption.max_chars = v
    if (v := num("caption_max_words", 1, 8)) is not None:
        p.caption.max_words = v
    if (v := num("max_comments", 1, 60)) is not None:
        p.comments.max_total = v
    tts = str(render_cfg.get("tts") or "preset")
    if tts in ("off", "auto", "on"):
        p.tts = tts  # type: ignore[assignment]
    return p


def page_seconds(rules: CommentRules, chars: int, dccons: int = 0) -> float:
    """댓글 화면 한 장을 보여 줄 시간: 최소 2.8초 + 읽는 시간 (1.2 + 글자 수 × 0.05), 상한까지."""
    read = rules.base_sec + max(0, chars) * rules.per_char_sec + max(0, dccons) * rules.dccon_sec
    return round(max(rules.min_screen_sec, min(rules.max_screen_sec, read)), 2)


def prompt_block(p: Preset) -> str:
    """대본 LLM 에게 줄 유형 규칙 (구간 배분·댓글 판단 기준)."""
    s, c, ly = p.sections, p.comments, p.layout
    parts = [
        f"편집 유형: {p.label} — {p.description}".strip(" —"),
        f"목표 길이 {s.target_sec[0]:.0f}~{s.target_sec[1]:.0f}초. 글 머리 {s.hook_sec[0]:.0f}~{s.hook_sec[1]:.0f}초, "
        f"본문(짤·글) 장면 {s.media_sec[0]:.0f}~{s.media_sec[1]:.0f}초, 끝 {s.outro_sec:g}초"
        + (f", 댓글이 전체의 약 {s.comment_share:.0%}." if c.enabled else ", 댓글 장면 없음."),
    ]
    if c.enabled:
        parts.append(
            "댓글 화면(디시 댓글창을 스크린샷한 것처럼 칸을 채운다)은 네가 몇 개를 한 화면에 보일지 판단한다. "
            f"comments 세그먼트의 screens 에 화면 한 장마다 댓글 번호 묶음을 적는다 (화면 하나 {c.per_screen[0]}~"
            f"{c.per_screen[1]}개 권장, 한 줄 짧은 댓글이 많으면 많이, 디시콘·긴 글이면 1~2개). 규칙: "
            "① 화면 높이에 꽉 차게 ② 원댓글과 그 답글은 같은 화면에 두고 쪼개지 않는다 "
            f"③ 화면 한 장은 최소 {c.min_screen_sec:g}초, 읽는 시간은 {c.base_sec:g}초 + 글자 수 × {c.per_char_sec:g}초 "
            f"④ 목표 길이에서 머리·본문·끝을 뺀 시간을 평균 화면 시간으로 나눠 화면 수를 정한다 "
            f"(최대 {c.max_screens}장, 전체 {c.max_total}개) ⑤ "
            + ("전부 보이지 말고 시간순으로 건너뛰며 반응이 이어지게(웃음→반박→결론) 고른다 "
               if c.skip == "time_order" else "고른 글타래부터 이어지는 댓글을 원래 순서대로 보인다 ")
            + "⑥ 댓글 본문은 절대 고치지 않는다(번호로만 고른다).")
    if p.tts == "off":
        parts.append("TTS 는 쓰지 않는다: 모든 세그먼트 tts=false. 소리는 BGM뿐이다.")
    elif p.tts == "auto":
        parts.append("TTS 는 세그먼트마다 네가 판단한다 (긴 글·상황 설명만 읽기).")
    if ly.caption == "band":
        parts.append("자막은 노란 자막이 아니라 짤·클립 아래 흰 띠 한 줄이다: 짤·클립 세그먼트의 첫 lines 한 줄을 띠에 쓴다.")
    elif ly.caption == "none":
        parts.append("화면 자막은 없다 (reaction 대사 제외).")
    if not ly.title_bar:
        parts.append("위쪽 고정 제목 바가 없다. title 은 유튜브 제목으로만 쓰이니 화면 글에 의존하지 않는다.")
    if p.guide.strip():
        parts.append(p.guide.strip())
    return "\n".join(parts)
