"""1차 규칙 필터 (기획서 3.2). 싸고 빠른 규칙을 LLM보다 먼저 돌린다.

걸러내는 것: 전화번호·이메일·주민번호 패턴, 짧은 글, 너무 긴 글, 그림만 있는 글, 금지어.
결과는 pass / block 과 한국어 사유 목록. LLM 필터(2차)는 pass 인 글에만 돈다.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field

# 휴대폰·지역번호 (구분자 -, ., 공백 허용). 숫자 사이 구분이 없는 11자리 010 번호도 잡는다.
_PHONE = re.compile(
    r"(?<!\d)(?:01[016789]|02|0[3-6][1-5]|070)[-.\s]?\d{3,4}[-.\s]?\d{4}(?!\d)"
)
_EMAIL = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
# 주민등록번호·외국인등록번호: 생년월일 6자리 - 성별 1~8 + 6자리
_RRN = re.compile(r"(?<!\d)\d{2}(?:0[1-9]|1[0-2])(?:0[1-9]|[12]\d|3[01])\s?-?\s?[1-8]\d{6}(?!\d)")
# 계좌번호처럼 보이는 긴 숫자 묶음 (3덩어리 이상, 합쳐 10자리 이상)
_ACCOUNT = re.compile(r"(?<!\d)\d{2,6}-\d{2,6}-\d{2,8}(?:-\d{1,6})?(?!\d)")
_SPACE = re.compile(r"\s+")


@dataclass
class RuleConfig:
    min_chars: int = 80
    max_chars: int = 4000
    block_words: list[str] = field(default_factory=list)


@dataclass
class RuleResult:
    verdict: str  # "pass" | "block"
    reasons: list[str]
    chars: int


def _visible_len(text: str) -> int:
    return len(_SPACE.sub("", text or ""))


def check_post(title: str, body: str, images: list[str] | None = None,
               cfg: RuleConfig | None = None) -> RuleResult:
    cfg = cfg or RuleConfig()
    text = f"{title or ''}\n{body or ''}"
    reasons: list[str] = []
    chars = _visible_len(body)

    if _PHONE.search(text):
        reasons.append("전화번호로 보이는 숫자가 있습니다")
    if _EMAIL.search(text):
        reasons.append("이메일 주소가 있습니다")
    if _RRN.search(text):
        reasons.append("주민등록번호 형식이 있습니다")
    for m in _ACCOUNT.finditer(text):
        chunk = m.group(0)
        if not _PHONE.fullmatch(chunk) and sum(ch.isdigit() for ch in chunk) >= 10:
            reasons.append("계좌번호로 보이는 숫자가 있습니다")
            break

    if chars == 0 and images:
        reasons.append("그림만 있는 글입니다")
    elif chars < cfg.min_chars:
        reasons.append(f"글이 짧습니다 ({chars}자 < {cfg.min_chars}자)")
    elif chars > cfg.max_chars:
        reasons.append(f"글이 너무 깁니다 ({chars}자 > {cfg.max_chars}자)")

    lowered = text.lower()
    hits = [w for w in cfg.block_words if w and w.lower() in lowered]
    if hits:
        reasons.append("금지어: " + ", ".join(sorted(set(hits))[:10]))

    return RuleResult("block" if reasons else "pass", reasons, chars)
