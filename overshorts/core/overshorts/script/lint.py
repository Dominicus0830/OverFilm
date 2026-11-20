"""대본 린터 (S3): TTS 에 넣기 전에 읽을 줄을 검사한다. 옵션이고 기본은 꺼짐이다.

검사는 읽기만 한다 — 대본·제목·댓글을 고치지 않는다 (내용을 바꾸면 재미가 떨어진다는 사용자 규칙).
`error` 가 있으면 음성 단계가 그 작업을 멈추고(비용·시간을 쓰지 않음) 검수 화면에 사유를 보인다.
`warn` 은 표시만 한다.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any

from .checks import load_banned
from .segments import _HOSTILE

MAX_SPOKEN_CHARS = 700  # 한 영상에서 읽을 글자 수 상한 (약 100초)
_PHONE = re.compile(r"01[016789][\s.-]?\d{3,4}[\s.-]?\d{4}")
_RRN = re.compile(r"\d{6}[\s-]?[1-4]\d{6}")
_EMAIL = re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+")


@dataclass
class LintResult:
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.errors


def lint_lines(lines: list[dict[str, Any]], style_dir: str | None = None,
               max_chars: int = MAX_SPOKEN_CHARS) -> LintResult:
    """lines: 음성 단계가 읽을 줄 [{"part", "text"}] (pause 줄은 건너뜀)."""
    res = LintResult()
    banned = [(p, why) for p, why in load_banned(style_dir) if why == "사용자 금지 목록"]
    total = 0
    for i, ln in enumerate(lines):
        if ln.get("part") == "pause":
            continue
        text = str(ln.get("text") or "")
        total += len(re.sub(r"\s", "", text))
        where = f"{i + 1}번째 줄"
        if _PHONE.search(text) or _RRN.search(text) or _EMAIL.search(text):
            res.errors.append(f"{where}: 개인정보(전화·주민번호·이메일)로 보이는 글이 있습니다")
        if _HOSTILE.search(text):
            res.warnings.append(f"{where}: 욕설·혐오 표현이 있습니다")
        for pat, why in banned:
            try:
                m = re.search(pat, text)
            except re.error:
                continue
            if m:
                res.errors.append(f"{where}: 금지 표현 '{m.group(0)}' ({why})")
    if total > max_chars:
        res.errors.append(f"읽을 글자가 {total}자입니다 (상한 {max_chars}자)")
    return res
