"""전사 대조 검증 (S4): 만든 음성을 STT 로 다시 받아 대본과 비교한다. 옵션이고 기본은 꺼짐이다.

누락(대본엔 있는데 안 들림)·반복(같은 말이 두 번)·오발음(전혀 다른 말로 들림)을 줄 단위로 찾아
`verify.json` 에 쓰고 검수 화면 경고로 넘긴다. 음성이나 대본은 고치지 않는다.

STT 는 외부 명령이다 (OverFilmSubs 의 `overfilm-subs extract` 가 기본). `{wav}` 에 음성 파일, `{srt}` 에 결과
SRT 경로가 들어간다. 명령이 없거나 실패하면 검증만 건너뛴다.
"""

from __future__ import annotations

import json
import re
import shlex
import subprocess
from dataclasses import dataclass, field
from difflib import SequenceMatcher
from pathlib import Path
from typing import Any

from ..proc import decode_output
from ..script.checks import norm

DEFAULT_STT = "overfilm-subs extract {wav} -o {srt} --no-mask --no-digits --keep-fillers --speakers off"
MISSING_BELOW = 0.45    # 이보다 덜 비슷하면 누락 또는 오발음
MISHEARD_BELOW = 0.75   # 이보다 덜 비슷하면 오발음 의심
TIMEOUT_SEC = 900
_SRT_TIME = re.compile(r"^\d{2}:\d{2}:\d{2}[,.]\d{3}\s*-->")


@dataclass
class LineCheck:
    line: int
    text: str
    heard: str
    similarity: float
    kind: str  # ok | missing | misheard | repeated


@dataclass
class VerifyResult:
    checks: list[LineCheck] = field(default_factory=list)
    skipped: str = ""

    @property
    def problems(self) -> list[LineCheck]:
        return [c for c in self.checks if c.kind != "ok"]

    def warnings(self) -> list[str]:
        if self.skipped:
            return [f"전사 대조를 건너뜀: {self.skipped}"]
        label = {"missing": "누락", "misheard": "오발음 의심", "repeated": "반복"}
        return [f"{c.line + 1}번째 줄 {label[c.kind]} (일치 {c.similarity:.0%}): '{c.text[:20]}' → '{c.heard[:20]}'"
                for c in self.problems]

    def to_json(self) -> dict[str, Any]:
        return {"skipped": self.skipped, "checks": [c.__dict__ for c in self.checks]}


def srt_text(srt: str) -> str:
    """SRT → 이어 붙인 글 (번호·시각 줄 제외)."""
    out = [ln.strip() for ln in (srt or "").splitlines()
           if ln.strip() and not ln.strip().isdigit() and not _SRT_TIME.match(ln.strip())]
    return " ".join(out)


def compare(lines: list[dict[str, Any]], transcript: str) -> list[LineCheck]:
    """대본 줄 (읽은 줄만) 을 전사문에 하나씩 맞춰 본다. 줄 순서대로 전사문을 따라가며 찾는다."""
    heard = norm(transcript)
    out: list[LineCheck] = []
    pos = 0
    for i, ln in enumerate(lines):
        if ln.get("part") == "pause":
            continue
        text = str(ln.get("text") or "").strip()
        want = norm(text)
        if not want:
            continue
        # 줄 길이 ±30% 창을 현재 위치부터 앞으로 밀며 가장 비슷한 곳
        n = len(want)
        best, best_at = 0.0, pos
        for start in range(pos, min(len(heard), pos + 4 * n + 40), max(1, n // 6)):
            win = heard[start:start + int(n * 1.3) + 2]
            if not win:
                break
            r = SequenceMatcher(None, want, win).ratio()
            if r > best:
                best, best_at = r, start
        seg = heard[best_at:best_at + int(n * 1.3) + 2]
        kind = "ok"
        if best < MISSING_BELOW:
            kind = "missing"
        elif best < MISHEARD_BELOW:
            kind = "misheard"
        else:
            # 같은 줄이 바로 뒤에 또 들리면 반복
            after = heard[best_at + n:best_at + n + int(n * 1.3) + 2]
            if len(want) >= 6 and after and SequenceMatcher(None, want, after).ratio() >= 0.8:
                kind = "repeated"
        if kind != "missing":
            pos = best_at + n // 2
        out.append(LineCheck(i, text, seg if kind != "missing" else "", round(best, 2), kind))
    return out


def run_stt(wav: Path, command: str | None = None) -> str:
    """외부 STT 를 돌려 전사문을 받는다. 실패하면 RuntimeError."""
    srt = wav.with_suffix(".verify.srt")
    template = command or DEFAULT_STT
    try:
        argv = [a.replace("{wav}", str(wav)).replace("{srt}", str(srt)) for a in shlex.split(template)]
    except ValueError as err:
        raise RuntimeError(f"STT 명령을 읽지 못했습니다: {err}") from err
    if not argv:
        raise RuntimeError("STT 명령이 비어 있습니다")
    try:
        proc = subprocess.run(argv, capture_output=True, timeout=TIMEOUT_SEC)
    except (OSError, subprocess.TimeoutExpired) as err:
        raise RuntimeError(f"STT 실행 실패: {err}") from err
    if proc.returncode != 0:
        raise RuntimeError(f"STT 가 실패했습니다: {decode_output(proc.stderr)[:200]}")
    try:
        return srt_text(srt.read_text(encoding="utf-8"))
    except OSError:
        return decode_output(proc.stdout)  # 명령이 표준출력으로 글만 주는 경우


def verify_voice(job_dir: Path, lines: list[dict[str, Any]], command: str | None = None,
                 stt=run_stt) -> VerifyResult:
    """job_dir/voice.wav 를 대본과 비교하고 verify.json 을 쓴다. STT 를 못 돌리면 skipped 사유만 남긴다."""
    wav = job_dir / "voice.wav"
    res = VerifyResult()
    if not any(ln.get("part") != "pause" and str(ln.get("text") or "").strip() for ln in lines):
        res.skipped = "읽는 줄이 없습니다 (TTS 꺼짐)"
    else:
        try:
            res.checks = compare(lines, stt(wav, command))
        except (RuntimeError, OSError) as err:
            res.skipped = str(err)
    try:
        (job_dir / "verify.json").write_text(json.dumps(res.to_json(), ensure_ascii=False, indent=1),
                                             encoding="utf-8")
    except OSError:
        pass
    return res
