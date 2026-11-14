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
    part: Literal["hook", "body", "comment", "outro"]
    text: str = Field(min_length=1, max_length=200)
    comment_no: int | None = Field(default=None, description="part=comment 일 때 인용한 댓글 번호")


class ScriptAnswer(BaseModel):
    title: str = Field(min_length=1, max_length=30, description="화면 위 제목바, 15자 안팎")
    fact: str = Field(default="", max_length=120, description="영상에 넣은 '이건 몰랐네' 구체적 사실")
    lines: list[ScriptLine] = Field(min_length=3, max_length=40)


class CringeIssue(BaseModel):
    line: int = Field(ge=0, description="문제 있는 줄 번호 (0부터)")
    reason: str = Field(max_length=120)


class CringeAnswer(BaseModel):
    score: float = Field(ge=0, le=10, description="10 = 사람이 쓴 것처럼 담백함")
    issues: list[CringeIssue] = Field(default_factory=list, max_length=20)
