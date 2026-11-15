"""게임 사실 점검 (2026-11-15 사용자 의견: 옵갤은 실제 게임과 다르면 특히 욕먹는다).

제목·나레이션이 원글·댓글에 없는 게임 정보를 지어냈는지 기계로 볼 수 있는 만큼 본다.
단어집은 OverFilm 의 packs/overwatch/ko/glossary (heroes.tsv) 를 그대로 읽는다. 없으면 영웅 점검은 건너뛴다.

- 원문에 없는 영웅 이름
- 한 팀 같은 영웅 여러 명 (원문에 워크샵·사용자 지정 등이 있을 때만 허용)
- 역할 자리와 영웅이 안 맞음 ("힐러 자리에 겐지")
- 원문은 빠대·워크샵인데 '경쟁전'
- 오래된 글(30일 넘음)에 '근황/요즘'
- 제목 숫자가 원문에 없음
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from datetime import datetime
from functools import lru_cache
from pathlib import Path

ROLE_OF_NOTE = {"돌격": "탱커", "공격": "딜러", "지원": "힐러"}
ROLE_WORD = {"탱커": "탱커", "탱": "탱커", "딜러": "딜러", "딜": "딜러", "힐러": "힐러", "힐": "힐러"}
CUSTOM = re.compile(r"워크샵|워크숍|사용자\s*지정|커스텀|아케이드|난투|이벤트|모드")
CASUAL = re.compile(r"빠대|빠른\s*대전|워크샵|워크숍|사용자\s*지정|아케이드")
RANKED = re.compile(r"경쟁|랭겜|랭크|점수|티어|브론즈|실버|골드|플래|다이아|마스터|그마|챔피언")
COUNT = r"(?:둘|셋|넷|다섯|여섯|두\s*명|세\s*명|네\s*명|[2-6]\s*명)"
STALE = re.compile(r"근황|요즘")
STALE_DAYS = 30


@dataclass(frozen=True)
class Hero:
    ko: str
    names: tuple[str, ...]  # 글에서 찾을 이름 (ko, en, 별명)
    role: str


def glossary_dir() -> Path | None:
    env = os.environ.get("OVERSHORTS_GLOSSARY")
    cands = [Path(env)] if env else []
    cands.append(Path(__file__).resolve().parents[4] / "packs" / "overwatch" / "ko" / "glossary")
    for c in cands:
        if (c / "heroes.tsv").is_file():
            return c
    return None


@lru_cache(maxsize=4)
def load_heroes(directory: str | None = None) -> tuple[Hero, ...]:
    d = Path(directory) if directory else glossary_dir()
    if d is None:
        return ()
    try:
        rows = (d / "heroes.tsv").read_text(encoding="utf-8").splitlines()
    except OSError:
        return ()
    out = []
    for row in rows[1:]:
        cols = row.split("\t")
        if len(cols) < 2 or not cols[1].strip():
            continue
        get = lambda i: cols[i] if i < len(cols) else ""  # noqa: E731
        names = [cols[1].strip(), get(2).strip()] + [a.strip() for a in get(3).split(";")]
        note = get(7)
        role = ""
        hits = [(note.find(k), v) for k, v in ROLE_OF_NOTE.items() if k in note]
        if hits:
            role = (max(hits) if "부터" in note else min(hits))[1]
        out.append(Hero(cols[1].strip(), tuple(n for n in names if len(n) >= 2), role))
    return tuple(out)


def _mentioned(heroes: tuple[Hero, ...], text: str) -> list[Hero]:
    return [h for h in heroes if any(n in text for n in h.names)]


def _hero_by_name(heroes: tuple[Hero, ...], word: str) -> Hero | None:
    for h in heroes:
        if any(word.startswith(n) for n in h.names):
            return h
    return None


def _age_days(posted_at: str | None, now: datetime | None = None) -> float | None:
    if not posted_at:
        return None
    s = posted_at.strip().replace(".", "-").replace("/", "-")
    try:
        when = datetime.fromisoformat(s[:19])
    except ValueError:
        return None
    return ((now or datetime.now()) - when.replace(tzinfo=None)).total_seconds() / 86400


def check_game(text: str, source: str, *, title: bool = False, posted_at: str | None = None,
               heroes: tuple[Hero, ...] | None = None, now: datetime | None = None) -> list[str]:
    """text(제목 또는 나레이션 한 줄)가 원문(source)과 게임 규칙에 맞는지. 문제 목록."""
    heroes = load_heroes() if heroes is None else heroes
    where = "제목" if title else "나레이션"
    out: list[str] = []
    for h in _mentioned(heroes, text):
        if not any(n in source for n in h.names):
            out.append(f"{where}에 원문에 없는 영웅 '{h.ko}'")
    if not CUSTOM.search(source):
        for h in heroes:
            for n in h.names:
                if re.search(re.escape(n) + r"\s*" + COUNT, text):
                    out.append(f"{where}: 한 팀에 같은 영웅 '{h.ko}' 여러 명은 불가능 (원문에 워크샵·사용자 지정 없음)")
                    break
    for m in re.finditer(r"(탱커|딜러|힐러|탱|딜|힐)\s*(?:자리|포지션|칸|롤)\S*\s+(\S+)", text):
        h = _hero_by_name(heroes, m.group(2))
        if h and h.role and h.role != ROLE_WORD[m.group(1)]:
            out.append(f"{where}: {h.ko}는 {h.role}라 {ROLE_WORD[m.group(1)]} 자리에 올 수 없음")
    if "경쟁전" in text and CASUAL.search(source) and not RANKED.search(source):
        out.append(f"{where}: 원문은 빠대·워크샵인데 '경쟁전'")
    age = _age_days(posted_at, now)
    if title and age is not None and age > STALE_DAYS and STALE.search(text):
        out.append(f"제목: {int(age)}일 지난 글에 '근황/요즘'")
    if title:
        for num in re.findall(r"\d+", text):
            if num not in source:
                out.append(f"제목 숫자 '{num}' 이 원문에 없음")
    return out


def check_script_game(title: str, spoken: list[str], source: str, posted_at: str | None = None,
                      heroes: tuple[Hero, ...] | None = None) -> list[str]:
    out = check_game(title, source, title=True, posted_at=posted_at, heroes=heroes)
    for i, line in enumerate(spoken):
        out += [f"{i + 1}번째 줄 {p}" for p in check_game(line, source, posted_at=posted_at, heroes=heroes)]
    return list(dict.fromkeys(out))


GAME_RULES = """게임 사실 규칙 (제목·나레이션 모두, 옵갤은 실제 게임과 다르면 특히 욕먹는다):
- 원글·댓글·사진에 없는 게임 정보(영웅, 스킬, 처치 수, 티어, 판 수, 맵, 모드)는 쓰지 않는다.
- 영웅·스킬·역할·티어·맵 이름은 단어집 표기를 따른다. 확실하지 않으면 더 일반적인 말로 쓴다.
- 처치 수·티어·판 수는 원글 숫자 그대로.
- 한 팀에 같은 영웅은 1명이다(원글에 워크샵·사용자 지정이라고 있을 때만 예외). 경쟁전은 역할 고정이다.
- 빠대·워크샵·아케이드를 경쟁전이라고 쓰지 않는다.
- 오래된 글이면 '근황/요즘'을 쓰지 않는다.
- 밈·은어는 원글에서 쓴 뜻 그대로 쓴다."""
