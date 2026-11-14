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
        return Response(int(r.status_code), r.text or "", str(r.url))

    return send


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

    def post_form(self, url: str, form: dict[str, str], referer: str) -> Response:
        return self._send("POST", url, referer, form, xhr=True)
