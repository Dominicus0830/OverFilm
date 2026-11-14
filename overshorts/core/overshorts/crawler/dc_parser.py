"""DC 페이지 파서. 셀렉터는 모두 이 파일에만 둔다 (기획서 5.1).

DC가 화면을 바꾸면 이 파일만 고치고, tests/fixtures 의 저장 HTML로 확인한다.
2026-11-14 실측: 목록은 `tbody tr.us-post`, 본문은 `div.write_div`,
댓글은 HTML에 없고 `POST /board/comment/` 가 JSON으로 준다 (브라우저 없이 받을 수 있음).
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from datetime import datetime

from bs4 import BeautifulSoup

# 목록에서 건너뛸 말머리·종류
SKIP_SUBJECTS = {"공지", "AD", "설문", "이슈"}
SKIP_DATA_TYPES = {"icon_notice", "icon_survey", "icon_ad"}
# 글에 그림이 있다는 표시
IMAGE_DATA_TYPES = {"icon_pic", "icon_recomimg", "icon_movie", "icon_recommovie"}

_INT_RE = re.compile(r"-?[0-9][0-9,]*")
_REPLY_RE = re.compile(r"\[(\d+)(?:/\d+)?\]")
_MAX_COMMENT_CHARS = 2000
_MAX_BODY_CHARS = 50_000


MEDIA_MARK = re.compile(r"\[(사진|영상) (\d+)\]")


@dataclass
class ListRow:
    post_no: int
    title: str
    subject: str  # 말머리
    upvotes: int
    views: int
    reply_count: int
    posted_at: datetime | None
    has_image: bool
    recommended: bool  # 개념글 표시


@dataclass
class Post:
    gall_id: str
    post_no: int
    title: str
    subject: str
    body: str
    images: list[str] = field(default_factory=list)
    videos: list[str] = field(default_factory=list)  # 본문 속 동영상 주소 (직접 받을 수 있는 것)
    upvotes: int = 0
    views: int = 0
    comment_count: int = 0
    posted_at: datetime | None = None
    deleted: bool = False
    e_s_n_o: str = ""  # 댓글 요청에 필요한 값
    gall_name: str = ""  # 출처 표기용 갤러리 이름 ("오버워치 2 마이너 갤러리")


@dataclass
class Comment:
    no: int
    text: str  # 디시콘만 있으면 빈 글자
    depth: int  # 0 = 댓글, 1 = 답글
    posted: str
    parent: int = 0  # 답글이면 부모 댓글 번호
    nick: str = ""
    ip: str = ""  # 유동이면 IP 앞 두 자리 (DC가 뒷자리를 가려서 준다)
    icon: str = ""  # fix = 고닉, half = 반고닉, "" = 유동
    dccon: dict | None = None  # {"url", "name"} 디시콘 그림


def _int(text: str | None, default: int = 0) -> int:
    if not text:
        return default
    m = _INT_RE.search(text)
    if not m:
        return default
    try:
        return int(m.group(0).replace(",", ""))
    except ValueError:
        return default


def _dt(text: str | None) -> datetime | None:
    if not text:
        return None
    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y.%m.%d %H:%M:%S", "%Y-%m-%d %H:%M"):
        try:
            return datetime.strptime(text.strip(), fmt)
        except ValueError:
            continue
    return None


def _text(node) -> str:
    return node.get_text(" ", strip=True) if node is not None else ""


def parse_list(html: str) -> list[ListRow]:
    """갤러리 목록 페이지 → 글 목록 (공지·광고·설문 제외)."""
    soup = BeautifulSoup(html or "", "html.parser")
    rows: list[ListRow] = []
    seen: set[int] = set()
    for tr in soup.select("tbody tr.ub-content"):
        data_type = tr.get("data-type") or ""
        if data_type in SKIP_DATA_TYPES:
            continue
        num_text = _text(tr.select_one("td.gall_num"))
        if not num_text.isdigit():
            continue
        subject = _text(tr.select_one("td.gall_subject"))
        if subject in SKIP_SUBJECTS:
            continue
        no = _int(tr.get("data-no") or num_text, -1)
        if no <= 0 or no in seen:
            continue
        seen.add(no)
        tit = tr.select_one("td.gall_tit")
        link = tit.select_one("a:not(.reply_numbox)") if tit is not None else None
        title = _text(link)
        reply = _REPLY_RE.search(_text(tit.select_one(".reply_num")) if tit is not None else "")
        date_td = tr.select_one("td.gall_date")
        rows.append(
            ListRow(
                post_no=no,
                title=title,
                subject=subject,
                upvotes=_int(_text(tr.select_one("td.gall_recommend"))),
                views=_int(_text(tr.select_one("td.gall_count"))),
                reply_count=int(reply.group(1)) if reply else 0,
                posted_at=_dt(date_td.get("title") if date_td is not None else None),
                has_image=data_type in IMAGE_DATA_TYPES,
                recommended=data_type.startswith("icon_recom"),
            )
        )
    return rows


def parse_post(html: str, gall_id: str, post_no: int) -> Post:
    """게시물 페이지 → 제목·본문·그림 주소·추천 수."""
    soup = BeautifulSoup(html or "", "html.parser")
    deleted = soup.select_one(".delet") is not None
    write = soup.select_one("div.write_div")
    images: list[str] = []
    videos: list[str] = []
    body = ""
    if write is not None:
        # 본문 속 자리를 [사진 N]·[영상 N] 표시로 남긴다: 대본이 어느 문장에 어떤 미디어를 둘지 고른다
        for img in write.select("img"):
            src = img.get("data-original") or img.get("src") or ""
            if src.startswith("//"):
                src = "https:" + src
            if src.startswith("http") and src not in images:
                images.append(src)
                img.replace_with(f"\n[사진 {len(images)}]\n")
        for vid in write.select("video"):
            srcs = [vid.get("data-src") or vid.get("src") or ""]
            srcs += [s.get("data-src") or s.get("src") or "" for s in vid.select("source")]
            for src in srcs:
                if src.startswith("//"):
                    src = "https:" + src
                if src.startswith("http") and src not in videos:
                    videos.append(src)
                    vid.replace_with(f"\n[영상 {len(videos)}]\n")
                    break
        for br in write.select("br"):
            br.replace_with("\n")
        for block in write.select("p, div"):
            block.insert_after("\n")
        lines = [ln.strip() for ln in write.get_text("").splitlines()]
        body = "\n".join(ln for ln in lines if ln)[:_MAX_BODY_CHARS]
    if write is None and not deleted:
        deleted = soup.select_one(".title_subject") is None
    esno = soup.find("input", {"id": "e_s_n_o"})
    page_title = _text(soup.title)
    gall_name = page_title.rsplit(" - ", 1)[-1].strip() if " - " in page_title else ""
    date = soup.select_one(".gallview_head .gall_date") or soup.select_one(".gall_date")
    return Post(
        gall_id=gall_id,
        post_no=post_no,
        title=_text(soup.select_one(".title_subject")),
        subject=_text(soup.select_one(".title_headtext")).strip("[]"),
        body=body,
        images=images,
        videos=videos,
        upvotes=_int(_text(soup.select_one(".up_num"))),
        views=_int(_text(soup.select_one(".gall_count"))),
        comment_count=_int(_text(soup.select_one(".gall_comment"))),
        posted_at=_dt(date.get("title") if date is not None else None),
        deleted=deleted,
        e_s_n_o=(esno.get("value") or "") if esno is not None else "",
        gall_name=gall_name[:60] if gall_name.endswith("갤러리") else "",
    )


def parse_comments(payload: str) -> tuple[list[Comment], int]:
    """댓글 JSON → (댓글, 전체 개수). 댓글돌이·삭제된 댓글은 뺀다.

    답글 관계(c_no), 닉네임·IP·고닉 아이콘, 디시콘(그림 주소·이름)까지 남긴다 (기획서 3.10).
    """
    try:
        data = json.loads(payload or "{}")
    except (json.JSONDecodeError, TypeError):
        return [], 0
    if not isinstance(data, dict):
        return [], 0
    total = _int(str(data.get("total_cnt", "0")))
    out: list[Comment] = []
    last_root = 0
    for c in data.get("comments") or []:
        if not isinstance(c, dict):
            continue
        no = _int(str(c.get("no", "0")))
        if no <= 0 or c.get("nicktype") == "COMMENT_BOY":
            continue
        depth = _int(str(c.get("depth", "0")))
        if depth == 0:
            last_root = no
        if c.get("del_yn") == "Y" or str(c.get("is_delete", "0")) != "0":
            continue
        memo = str(c.get("memo") or "")
        soup = BeautifulSoup(memo, "html.parser")
        dccon = None
        for img in soup.select("img"):
            src = str(img.get("src") or img.get("data-src") or "")
            if dccon is None and "dccon" in src and src.startswith(("http", "//")):
                name = str(img.get("conalt") or img.get("title") or img.get("alt") or "")
                dccon = {"url": "https:" + src if src.startswith("//") else src,
                         "name": name[:30]}
            img.decompose()
        for br in soup.select("br"):
            br.replace_with("\n")
        text = "\n".join(ln.strip() for ln in soup.get_text("").splitlines() if ln.strip())
        text = text[:_MAX_COMMENT_CHARS]
        if not text and dccon is None:
            continue  # 그림만 있는 댓글 (디시콘이 아님)
        parent = _int(str(c.get("c_no") or "0")) if depth > 0 else 0
        if depth > 0 and parent <= 0:
            parent = last_root
        icon_html = str(c.get("gallog_icon") or "")
        icon = "fix" if "fix_" in icon_html else ("half" if c.get("user_id") else "")
        out.append(Comment(no, text, depth, str(c.get("reg_date") or ""), parent,
                           str(c.get("name") or "")[:30], str(c.get("ip") or "")[:20], icon,
                           dccon))
    return out, total


# --- 차단 신호 -----------------------------------------------------------

_BLOCK_MARKERS = ("captcha", "자동입력 방지", "접근이 제한", "비정상적인 접근", "too many requests")
# 정상 페이지에만 있는 표시. 정상 글 페이지에도 추천·댓글용 kcaptcha 가 있으므로
# 이 표시가 있으면 캡차 글자가 있어도 차단으로 보지 않는다.
_NORMAL_MARKERS = ("gallview_head", "write_div", "gall_list", "ub-content", '"comments"')


def looks_blocked(html: str) -> bool:
    """캡차·접근 제한 페이지인지 (정상 내용이 없고 차단 표시가 있음)."""
    text = (html or "").lower()
    if any(m in text for m in _NORMAL_MARKERS):
        return False
    return any(m in text for m in _BLOCK_MARKERS)


_MGALLERY_REDIRECT = re.compile(r"/mgallery/board/lists/?\?id=")


def redirects_to_mgallery(html: str) -> bool:
    """정식 갤 주소로 마이너 갤을 열면 DC가 스크립트로 마이너 주소로 보낸다."""
    head = (html or "")[:5000]
    return "location" in head and bool(_MGALLERY_REDIRECT.search(head))
