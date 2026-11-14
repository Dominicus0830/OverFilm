"""HTTP 요청. 모든 요청은 CrawlGuard를 거쳐 한 줄로만 나간다 (기획서 5.1).

curl_cffi 의 impersonate="chrome" 으로 TLS 지문과 User-Agent를 실제 크롬과 맞춘다.
테스트에서는 transport 를 바꿔 끼워 네트워크 없이 돈다.
"""

from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass

from .budget import CrawlGuard
from .dc_parser import looks_blocked

REFERER = "https://gall.dcinside.com/"
MAX_REDIRECTS = 5
TIMEOUT_SEC = 20


@dataclass
class Response:
    status: int
    text: str
    url: str
    content: bytes = b""
    content_type: str = ""


class FetchError(Exception):
    """네트워크 오류 등 차단이 아닌 실패."""


class RedirectLoop(Exception):
    pass


# (method, url, headers, form) -> Response. RedirectLoop 이나 FetchError 를 던질 수 있다.
Transport = Callable[[str, str, dict[str, str], dict[str, str] | None], Response]


def curl_transport(proxy: str | None = None) -> Transport:
    try:
        from curl_cffi import requests as creq
        from curl_cffi.requests.exceptions import TooManyRedirects
    except ImportError as exc:  # pragma: no cover - 설치 안내
        raise FetchError("curl_cffi 가 설치되어 있지 않습니다: pip install curl_cffi") from exc

    session = creq.Session(impersonate="chrome")
    proxies = {"http": proxy, "https": proxy} if proxy else None

    def send(method: str, url: str, headers: dict[str, str], form: dict[str, str] | None):
        try:
            r = session.request(
                method,
                url,
                headers=headers,
                data=form,
                timeout=TIMEOUT_SEC,
                allow_redirects=True,
                max_redirects=MAX_REDIRECTS,
                proxies=proxies,
            )
        except TooManyRedirects as exc:
            raise RedirectLoop(str(exc)) from exc
        except Exception as exc:  # noqa: BLE001 - 네트워크 오류는 모두 FetchError로
            raise FetchError(f"요청 실패: {exc}") from exc
        ctype = str(r.headers.get("content-type") or "")
        body = r.content or b""
        text = "" if ctype.startswith("image/") else (r.text or "")
        return Response(int(r.status_code), text, str(r.url), body, ctype)

    return send


def _image_magic(data: bytes) -> bool:
    """content-type 이 틀리게 오는 경우를 위해 파일 앞부분으로 그림인지 본다."""
    return (
        data[:3] == b"\xff\xd8\xff"
        or data[:8] == b"\x89PNG\r\n\x1a\n"
        or data[:6] in (b"GIF87a", b"GIF89a")
        or (data[:4] == b"RIFF" and data[8:12] == b"WEBP")
    )


class Fetcher:
    def __init__(self, guard: CrawlGuard, transport: Transport | None = None,
                 proxy: str | None = None):
        self.guard = guard
        self._transport = transport
        self._proxy = proxy

    @property
    def transport(self) -> Transport:
        if self._transport is None:
            self._transport = curl_transport(self._proxy)
        return self._transport

    def _send(self, method: str, url: str, referer: str, form: dict[str, str] | None,
              xhr: bool = False) -> Response:
        self.guard.check()
        self.guard.wait_before_request()
        headers = {"Referer": referer, "Accept-Language": "ko-KR,ko;q=0.9"}
        if xhr:
            headers["X-Requested-With"] = "XMLHttpRequest"
        start = time.monotonic()
        try:
            resp = self.transport(method, url, headers, form)
        except RedirectLoop:
            self.guard.record(url, None, None, blocked=True)
            raise  # record 가 CrawlBlocked 를 던지므로 여기는 오지 않는다
        except FetchError:
            self.guard.record(url, None, None, blocked=False)
            raise
        elapsed = time.monotonic() - start
        blocked = resp.status in (403, 429) or (resp.status == 200 and looks_blocked(resp.text))
        self.guard.record(url, resp.status, elapsed, blocked)
        if resp.status >= 400:
            raise FetchError(f"HTTP {resp.status}: {url}")
        return resp

    def get(self, url: str, referer: str = REFERER) -> Response:
        return self._send("GET", url, referer, None)

    def get_image(self, url: str, max_bytes: int = 15 * 1024 * 1024) -> bytes:
        """첨부 그림. DC 그림 서버는 Referer 가 있어야 준다. 요청 예산에 같이 센다."""
        resp = self._send("GET", url, REFERER, None)
        if not (resp.content_type.startswith("image/") or _image_magic(resp.content)):
            raise FetchError(f"그림이 아닙니다 ({resp.content_type or '종류 없음'}): {url}")
        if len(resp.content) > max_bytes:
            raise FetchError(f"그림이 너무 큽니다 ({len(resp.content)} 바이트): {url}")
        return resp.content

    def get_video(self, url: str, max_bytes: int = 80 * 1024 * 1024) -> bytes:
        """본문 속 동영상 (mp4·webm). 그림과 같이 요청 예산에 센다."""
        resp = self._send("GET", url, REFERER, None)
        head = resp.content[:12]
        is_video = (resp.content_type.startswith("video/") or head[4:8] == b"ftyp"
                    or head[:4] == b"\x1a\x45\xdf\xa3")
        if not is_video:
            raise FetchError(f"동영상이 아닙니다 ({resp.content_type or '종류 없음'}): {url}")
        if len(resp.content) > max_bytes:
            raise FetchError(f"동영상이 너무 큽니다 ({len(resp.content)} 바이트): {url}")
        return resp.content

    def post_form(self, url: str, form: dict[str, str], referer: str) -> Response:
        return self._send("POST", url, referer, form, xhr=True)
