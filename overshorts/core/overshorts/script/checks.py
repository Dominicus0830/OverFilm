"""대본 규칙 검사 (기획서 3.3 '강제하는 방법' 2단계). LLM 없이 도는 싼 검사.

- 금지 표현 (style/banned.txt + 기본 목록), 특정 커뮤 어미(~노)
- 문장 길이: 한 문장 20자 안팎, 너무 긴 문장, 길이가 다 같아 기계처럼 들리는 경우
- 원문 겹침 비율: 대본 글자 5-gram 중 원문에도 있는 것의 비율 (댓글 인용 줄은 뺌)
- 지어낸 댓글: 댓글 줄이 실제 댓글에 없으면 경고
- 예상 길이: 글자 수 ÷ 보이스 말 속도
"""

from __future__ import annotations

import re
import statistics
from dataclasses import dataclass, field
from pathlib import Path

# (정규식, 사유) — banned.txt 가 없거나 비어도 이것은 늘 검사한다
DEFAULT_BANNED: list[tuple[str, str]] = [
    (r"여러분", "호명·인사"),
    (r"알아(?:볼게요|보겠습니다|보자)", "호명·인사"),
    (r"오늘은\s*.{0,20}(?:알아|소개|준비)", "호명·인사"),
    (r"충격", "낚시 감탄"),
    (r"실화냐", "낚시 감탄"),
    (r"소름\s*돋", "낚시 감탄"),
    (r"경악", "낚시 감탄"),
    (r"과연\s*그", "떡밥 질문"),
    (r"어떻게\s*(?:됐|되었)을까", "떡밥 질문"),
    (r"그\s*결과는\s*\?", "떡밥 질문"),
    (r"조심하세요", "훈계·교훈"),
    (r"우리\s*모두", "훈계·교훈"),
    (r"구독", "구독 요청"),
    (r"좋아요\s*(?:와|랑|눌러)", "구독 요청"),
    (r"알림\s*설정", "구독 요청"),
]
# 정치적 색이 있는 커뮤 어미 (~노): 문장 끝 낱말이 '줄기 + 노' 일 때.
# 받침 있는 줄기(맞노, 했노, 있노)나 하·되·가·이·나 뒤. 피아노·카지노 같은 낱말은 뺀다.
_NO_WORD = re.compile(r"([가-힣]+)노(?=[\s?!.~]*$)")
_NO_OPEN_STEMS = set("하되가이나냐보오러")
_NO_NOUNS = {"피아", "카지", "스", "제", "키", "미", "리", "도", "마", "사이", "다", "포", "부"}
_LAUGH = re.compile(r"ㅋ{2,}|ㄹㅇ")
_REPORTED = re.compile(r"[가-힣]다고\s*(?:합니다|해요|한다)")
_NORM = re.compile(r"[\s\W_]+", re.U)

MAX_SENTENCE_CHARS = 40
OVERLAP_LIMIT = 0.30
DEFAULT_CHARS_PER_SEC = 7.0  # 한국어 TTS 보통 속도 (보이스별로 보정)


@dataclass
class Issue:
    line: int  # 대본 줄 번호 (0부터), -1 = 전체
    kind: str
    text: str


@dataclass
class CheckResult:
    issues: list[Issue] = field(default_factory=list)
    overlap_ratio: float = 0.0
    est_seconds: float = 0.0
    sentence_lengths: list[int] = field(default_factory=list)

    @property
    def needs_rewrite(self) -> bool:
        """자동으로 한 번 다시 쓰게 할 문제: 금지 표현, 지어낸 댓글, 겹침 30% 초과."""
        hard = {"금지 표현", "커뮤 어미", "지어낸 댓글", "원문 겹침"}
        return any(i.kind in hard for i in self.issues)

    def reasons(self) -> list[str]:
        return [f"{i.line + 1}번째 줄 {i.kind}: {i.text}" if i.line >= 0 else f"{i.kind}: {i.text}"
                for i in self.issues]


def load_banned(style_dir: str | Path | None) -> list[tuple[str, str]]:
    """banned.txt: 한 줄에 하나. `re:` 로 시작하면 정규식, 아니면 글자 그대로. `#` 은 주석.
    `표현 | 사유` 처럼 사유를 붙일 수 있다."""
    out = list(DEFAULT_BANNED)
    if style_dir is None:
        return out
    path = Path(style_dir) / "banned.txt"
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return out
    for raw in lines[:2000]:
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        pat, _, why = line.partition("|")
        pat, why = pat.strip(), (why.strip() or "사용자 금지 목록")
        if pat.startswith("re:"):
            pat = pat[3:].strip()
            try:
                re.compile(pat)
            except re.error:
                continue
        else:
            pat = re.escape(pat)
        if pat:
            out.append((pat, why))
    return out


def norm(text: str) -> str:
    return _NORM.sub("", text or "")


def ngrams(text: str, n: int = 5) -> list[str]:
    t = norm(text)
    return [t[i:i + n] for i in range(len(t) - n + 1)]


def overlap_ratio(script_text: str, source: str, n: int = 5) -> float:
    grams = ngrams(script_text, n)
    if not grams:
        return 0.0
    src = set(ngrams(source, n))
    return sum(1 for g in grams if g in src) / len(grams)


def split_sentences(text: str) -> list[str]:
    parts = re.split(r"(?<=[.?!])\s+|\n+", text or "")
    return [p.strip() for p in parts if p and p.strip()]


def check_script(lines: list[dict], source_body: str, source_comments: list[str],
                 banned: list[tuple[str, str]] | None = None,
                 chars_per_sec: float = DEFAULT_CHARS_PER_SEC) -> CheckResult:
    """lines: [{"part": hook|body|comment|outro, "text": ...}]"""
    banned = banned if banned is not None else DEFAULT_BANNED
    res = CheckResult()
    narration: list[str] = []
    laughs = 0
    reported_run = 0
    for i, line in enumerate(lines):
        text = str(line.get("text") or "").strip()
        part = line.get("part")
        if not text:
            continue
        if part == "comment":
            if not _comment_exists(text, source_comments):
                res.issues.append(Issue(i, "지어낸 댓글", "원래 댓글에서 찾지 못했습니다"))
        else:
            narration.append(text)
            for pat, why in banned:
                m = re.search(pat, text)
                if m:
                    res.issues.append(Issue(i, "금지 표현", f"'{m.group(0)}' ({why})"))
            if any(_no_ending(s) for s in split_sentences(text)):
                res.issues.append(Issue(i, "커뮤 어미", "~노 어미는 쓰지 않습니다"))
            if _REPORTED.search(text):
                reported_run += 1
                if reported_run >= 2:
                    res.issues.append(Issue(i, "전언 반복", "'~했다고 합니다'가 이어집니다"))
            else:
                reported_run = 0
        laughs += len(_LAUGH.findall(text))
        for s in split_sentences(text):
            n = len(norm(s))
            res.sentence_lengths.append(n)
            if part != "comment" and n > MAX_SENTENCE_CHARS:
                res.issues.append(Issue(i, "긴 문장", f"{n}자 (20자 안팎 권장)"))
    if laughs > 2:
        res.issues.append(Issue(-1, "커뮤 말투 과다", f"ㅋㅋ·ㄹㅇ {laughs}번 (영상당 1~2번)"))
    body_lengths = res.sentence_lengths
    if len(body_lengths) >= 5 and statistics.pstdev(body_lengths) < 2.5:
        res.issues.append(Issue(-1, "길이 단조", "문장 길이가 다 비슷해 기계처럼 들립니다"))
    res.overlap_ratio = overlap_ratio("\n".join(narration), source_body)
    if res.overlap_ratio > OVERLAP_LIMIT:
        res.issues.append(Issue(-1, "원문 겹침",
                                f"{res.overlap_ratio:.0%} (30% 이하로, 원문을 그대로 읽지 말 것)"))
    total_chars = sum(len(norm(str(line.get("text") or ""))) for line in lines)
    cps = chars_per_sec if chars_per_sec > 0 else DEFAULT_CHARS_PER_SEC
    res.est_seconds = round(total_chars / cps + 0.15 * max(0, len(res.sentence_lengths) - 1), 1)
    if res.est_seconds > 60:
        res.issues.append(Issue(-1, "너무 김", f"예상 {res.est_seconds:.0f}초 (60초 이하)"))
    return res


def _no_ending(sentence: str) -> bool:
    m = _NO_WORD.search(sentence.strip())
    if not m:
        return False
    stem = m.group(1)
    if stem in _NO_NOUNS:
        return False
    last = ord(stem[-1]) - 0xAC00
    has_batchim = 0 <= last < 11172 and last % 28 != 0
    return has_batchim or stem[-1] in _NO_OPEN_STEMS


def _comment_exists(text: str, comments: list[str]) -> bool:
    t = norm(text)
    if not t:
        return True
    for c in comments:
        cn = norm(c)
        if t in cn or (cn in t and len(cn) >= max(4, len(t) // 2)):
            return True
        if len(t) >= 8 and overlap_ratio(text, c, 3) >= 0.6:
            return True
    return False
