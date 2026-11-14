"""LLM 대답 형식 (pydantic). 이 모델의 JSON 스키마를 그대로 LLM 에 준다."""

from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field

CATEGORIES = ("politics", "hate", "sexual", "real_person", "personal_info", "crime_selfharm", "ad")
CATEGORY_KO = {
    "politics": "정치",
    "hate": "혐오·차별",
    "sexual": "성적",
    "real_person": "실존 인물 비방",
    "personal_info": "개인정보",
    "crime_selfharm": "범죄·자해",
    "ad": "광고",
}


class FilterAnswer(BaseModel):
    verdict: Literal["pass", "block", "review"]
    categories: list[Literal["politics", "hate", "sexual", "real_person", "personal_info",
                             "crime_selfharm", "ad"]] = Field(default_factory=list, max_length=7)
    reasons: list[str] = Field(default_factory=list, max_length=5)
    topic: str = Field(default="", max_length=40, description="글 주제 한두 단어")
    fit_score: float = Field(ge=0, le=10, description="쇼츠 적합도 0~10")


class ScriptLine(BaseModel):
    """나레이션 한 줄 (검수 화면·음성 단계가 쓰는 평평한 목록). 세그먼트에서 만든다."""
    part: Literal["hook", "body", "comment", "outro"]
    text: str = Field(min_length=1, max_length=200)
    comment_no: int | None = Field(default=None, description="part=comment 일 때 인용한 댓글 번호")


DISPLAYS = ("fill", "crop", "full", "tall", "scroll")


class MediaPick(BaseModel):
    n: str = Field(description="미디어 이름 그대로: '사진 3' 또는 '영상 1'")
    display: Literal["fill", "crop", "full", "tall", "scroll"] = Field(
        default="fill",
        description="fill=미디어 영역 꽉 채움(가로 기본), crop=핵심 부분만 잘라 채움(세로에서 볼 것이 "
                    "일부에 모였을 때), full=2~5초 제목 숨기고 전체 화면(세로 전체가 의미 있을 때), "
                    "tall=영역을 위아래로 늘려 자르지 않음, scroll=글자 많은 캡처를 천천히 아래로")
    line: int = Field(default=0, ge=0, description="이 세그먼트 lines 중 몇 번째 줄(0부터)에 나올지")


class Segment(BaseModel):
    kind: Literal["title", "media", "body", "comments"] = Field(
        description="title=제목, media=원문 사진·영상, body=디시 본문처럼 그린 글 카드, "
                    "comments=디시 댓글 화면")
    subtitle: str = Field(default="", max_length=20, description="소제목: 이 장면 한 줄 요약 (12자 안팎)")
    tts: bool = Field(description="나레이션을 읽을지")
    tts_reason: str = Field(default="", max_length=80, description="tts 를 켜거나 끈 이유")
    lines: list[str] = Field(default_factory=list, max_length=12,
                             description="나레이션 문장들. tts 를 꺼도 켤 때를 위해 써 둔다 "
                                         "(comments 세그먼트는 비움)")
    media: list[MediaPick] = Field(default_factory=list, max_length=6)
    comments: list[int] = Field(default_factory=list, max_length=10,
                                description="보여줄 실제 댓글 번호 ([번호] 그대로), 원래 순서대로")
    quote: str = Field(default="", max_length=600,
                       description="body 일 때 글 카드에 보일 원문 부분 (원문 그대로 복사, 300자 이내)")
    read: list[int] = Field(default_factory=list, max_length=2,
                            description="그중 TTS 가 원문 그대로 읽을 핵심 댓글 번호 (0~2개)")


class ScriptAnswer(BaseModel):
    title: str = Field(min_length=1, max_length=34,
                       description="화면 위 고정 제목, 1~2줄. 원문 제목 그대로이거나 다듬은 것")
    fact: str = Field(default="", max_length=120, description="영상에 넣은 '이건 몰랐네' 구체적 사실")
    composition: Literal["A", "B"] = Field(
        description="A=제목→본문→댓글 반응(댓글이 재밌거나 내용을 더할 때), B=제목→본문")
    composition_reason: str = Field(default="", max_length=80)
    segments: list[Segment] = Field(min_length=2, max_length=8)


class CringeIssue(BaseModel):
    line: int = Field(ge=0, description="문제 있는 줄 번호 (0부터)")
    reason: str = Field(max_length=120)


class CringeAnswer(BaseModel):
    score: float = Field(ge=0, le=10, description="10 = 사람이 쓴 것처럼 담백함")
    issues: list[CringeIssue] = Field(default_factory=list, max_length=20)
