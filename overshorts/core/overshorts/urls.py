"""DC 갤러리 링크 자동 인식 (기획서 3.1, 5.1 URL 파서).

링크 하나를 받아 갤러리 목록인지 게시물인지, 정식·마이너·미니 중 어느 갤러리인지 판단한다.
모바일 링크는 PC 형식으로 바꾼다. 모바일 `m.dcinside.com/board/{id}`는 정식인지 마이너인지
주소만으로 알 수 없어서 `gall_type`을 `unknown`으로 두고, 크롤러가 한 번 확인한다.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from urllib.parse import parse_qs, urlsplit

PC_HOST = "gall.dcinside.com"
MOBILE_HOST = "m.dcinside.com"

# 정식(board) · 마이너(mgallery) · 미니(mini) · 모바일에서 알 수 없음(unknown)
GALL_TYPES = ("board", "mgallery", "mini", "unknown")

# 댓글 AJAX의 _GALLTYPE_ 값
GALLTYPE_CODE = {"board": "G", "mgallery": "M", "mini": "MI"}

_ID_RE = re.compile(r"^[A-Za-z0-9_]{1,64}$")
_NO_RE = re.compile(r"^[0-9]{1,12}$")


class UrlError(ValueError):
    """DC 갤러리 링크로 알아볼 수 없는 주소."""


@dataclass(frozen=True)
class DcUrl:
    kind: str  # "gallery" | "post"
    gall_type: str  # GALL_TYPES 중 하나
    gall_id: str
    post_no: int | None = None
    recommend_only: bool = False  # 개념글 목록 (exception_mode=recommend)
    from_mobile: bool = False

    @property
    def label(self) -> str:
        """패널의 종류 배지에 쓰는 한국어 이름."""
        names = {"board": "정식 갤", "mgallery": "마이너 갤", "mini": "미니 갤", "unknown": "갤"}
        base = names.get(self.gall_type, "갤")
        if self.kind == "post":
            return f"{base} 게시물"
        return f"{base} 개념글 목록" if self.recommend_only else f"{base} 목록"

    def with_gall_type(self, gall_type: str) -> DcUrl:
        if gall_type not in GALL_TYPES:
            raise UrlError(f"알 수 없는 갤러리 종류: {gall_type}")
        return DcUrl(
            self.kind, gall_type, self.gall_id, self.post_no, self.recommend_only, self.from_mobile
        )

    def _prefix(self) -> str:
        # unknown은 정식 주소로 둔다. 마이너 갤이면 DC가 마이너 주소로 보낸다.
        if self.gall_type == "mgallery":
            return "/mgallery/board"
        if self.gall_type == "mini":
            return "/mini/board"
        return "/board"

    def list_url(self, page: int = 1) -> str:
        page = max(1, min(int(page), 10_000))
        url = f"https://{PC_HOST}{self._prefix()}/lists/?id={self.gall_id}&page={page}"
        if self.recommend_only:
            url += "&exception_mode=recommend"
        return url

    def view_url(self, post_no: int | None = None) -> str:
        no = post_no if post_no is not None else self.post_no
        if no is None:
            raise UrlError("게시물 번호가 없습니다")
        return f"https://{PC_HOST}{self._prefix()}/view/?id={self.gall_id}&no={int(no)}"

    @property
    def galltype_code(self) -> str:
        return GALLTYPE_CODE.get(self.gall_type, "G")


def _check_id(gall_id: str | None) -> str:
    if not gall_id or not _ID_RE.match(gall_id):
        raise UrlError("갤러리 id를 찾지 못했습니다")
    return gall_id


def _check_no(no: str | None) -> int | None:
    if no is None or no == "":
        return None
    if not _NO_RE.match(no):
        raise UrlError("게시물 번호가 숫자가 아닙니다")
    return int(no)


def parse_url(raw: str) -> DcUrl:
    """링크를 읽어 DcUrl로 돌려준다. 알아볼 수 없으면 UrlError."""
    text = (raw or "").strip()
    if not text:
        raise UrlError("링크가 비어 있습니다")
    if len(text) > 2048:
        raise UrlError("링크가 너무 깁니다")
    if "://" not in text:
        text = "https://" + text
    parts = urlsplit(text)
    host = (parts.hostname or "").lower()
    query = parse_qs(parts.query)

    def q(name: str) -> str | None:
        values = query.get(name)
        return values[0] if values else None

    recommend = q("exception_mode") == "recommend"
    segs = [s for s in parts.path.split("/") if s]

    if host in (PC_HOST, "www." + PC_HOST):
        gall_type = "board"
        if segs and segs[0] in ("mgallery", "mini"):
            gall_type = segs[0]
            segs = segs[1:]
        if len(segs) < 2 or segs[0] != "board":
            raise UrlError("갤러리 목록이나 게시물 주소가 아닙니다")
        action = segs[1]
        gall_id = _check_id(q("id"))
        if action == "lists":
            return DcUrl("gallery", gall_type, gall_id, None, recommend)
        if action == "view":
            no = _check_no(q("no"))
            if no is None:
                raise UrlError("게시물 번호(no)가 없습니다")
            return DcUrl("post", gall_type, gall_id, no, recommend)
        raise UrlError("갤러리 목록이나 게시물 주소가 아닙니다")

    if host == MOBILE_HOST:
        # /board/{id}, /board/{id}/{no}, /mini/{id}, /mini/{id}/{no}
        # 예전 형식 /board/lists?id=..., /board/view?id=...&no=... 도 받는다.
        if not segs:
            raise UrlError("모바일 갤러리 주소가 아닙니다")
        gall_type = "mini" if segs[0] == "mini" else "unknown"
        if segs[0] not in ("board", "mini"):
            raise UrlError("모바일 갤러리 주소가 아닙니다")
        rest = segs[1:]
        if rest and rest[0] in ("lists", "view"):
            gall_id = _check_id(q("id"))
            no = _check_no(q("no"))
        else:
            gall_id = _check_id(rest[0] if rest else None)
            no = _check_no(rest[1] if len(rest) > 1 else None)
        recommend = recommend or q("recommend") == "1"
        kind = "post" if no is not None else "gallery"
        return DcUrl(kind, gall_type, gall_id, no, recommend, from_mobile=True)

    raise UrlError("DC인사이드 갤러리 주소가 아닙니다")
