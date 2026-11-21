"""패널을 브라우저 탭이 아니라 OverShorts 창 하나로 연다 (`overshorts serve --window`).

- Edge(없으면 Chrome)의 앱 창 모드(`--app=`)로 연다. 주소창·탭이 없는 창이고, 전용 프로필 폴더를
  써서 평소 쓰는 브라우저와 섞이지 않는다. 둘 다 없으면 기본 브라우저로 연다.
- 이미 같은 빌드의 서버가 켜져 있으면 새로 켜지 않고 창만 그 서버에 붙인다. 다른 빌드(예전 버전)면
  그 서버를 끄고 새로 켠다. 이전처럼 '포트 사용 중' 경고로 멈추지 않는다.
- 창을 모두 닫으면(패널 WebSocket 이 다 끊기면) 서버도 꺼진다: server/app.py 의 idle_exit.
"""

from __future__ import annotations

import hashlib
import json
import os
import secrets
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

PACKAGE = Path(__file__).resolve().parent
PANEL_INDEX = PACKAGE.parents[1] / "panel" / "dist" / "index.html"
SERVER_FILE = ".server.json"  # 데이터 폴더에 남기는 켜진 서버 정보 (포트·빌드·종료 토큰)


def build_id() -> str:
    """이 설치본의 지문. 파이썬 코드와 패널 index.html(자산 파일 이름에 해시가 들어 있다)로 만든다.
    버전 번호를 올리지 않아도 고친 코드가 들어오면 바뀐다."""
    h = hashlib.sha1()
    files = sorted(PACKAGE.rglob("*.py"))[:2000]
    if PANEL_INDEX.is_file():
        files.append(PANEL_INDEX)
    for f in files:
        try:
            h.update(str(f.relative_to(PACKAGE.parents[1])).encode("utf-8", "replace"))
            h.update(f.read_bytes())
        except (OSError, ValueError):
            continue
    return h.hexdigest()[:16]


def alert(text: str) -> None:
    """콘솔이 없어도 보이게 윈도우 메시지 상자로 알린다 (다른 OS 는 출력만)."""
    print(text, flush=True)
    if os.name != "nt":
        return
    try:
        import ctypes

        ctypes.windll.user32.MessageBoxW(None, text, "OverShorts", 0x30)
    except (AttributeError, OSError):
        pass


# --- 켜진 서버 확인·붙기·끄기 ---
def _get_json(url: str, timeout: float = 1.5) -> dict[str, Any] | None:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as res:  # noqa: S310 - 127.0.0.1 고정
            body = res.read(64_000)
        data = json.loads(body.decode("utf-8", "replace"))
        return data if isinstance(data, dict) else None
    except (OSError, ValueError, urllib.error.URLError):
        return None


def port_open(port: int) -> bool:
    import socket

    with socket.socket() as sock:
        sock.settimeout(0.5)
        return sock.connect_ex(("127.0.0.1", port)) == 0


def running_server(port: int) -> dict[str, Any] | None:
    """그 포트에 OverShorts 서버가 있으면 /api/hello 응답을, 없거나 다른 프로그램이면 None."""
    hello = _get_json(f"http://127.0.0.1:{port}/api/hello")
    if hello and hello.get("app") == "overshorts":
        return hello
    return None


def old_server(port: int) -> bool:
    """/api/hello 가 없던 예전 OverShorts 서버인지 (/api/status 로 알아본다)."""
    status = _get_json(f"http://127.0.0.1:{port}/api/status", timeout=3)
    return bool(status and "today" in status)


def write_server_file(home: Path, port: int, token: str) -> None:
    try:
        (home / SERVER_FILE).write_text(json.dumps(
            {"pid": os.getpid(), "port": port, "build": build_id(), "token": token}), encoding="utf-8")
    except OSError:
        pass


def new_token() -> str:
    return secrets.token_urlsafe(24)


def ask_shutdown(home: Path, port: int, wait: float = 15.0) -> bool:
    """예전 빌드 서버에게 꺼 달라고 한다 (토큰은 그 서버가 데이터 폴더에 남긴 것). 꺼지면 True."""
    try:
        info = json.loads((home / SERVER_FILE).read_text(encoding="utf-8"))
        token = str(info.get("token") or "")
    except (OSError, ValueError, AttributeError):
        token = ""
    if not token:
        return False
    req = urllib.request.Request(f"http://127.0.0.1:{port}/api/shutdown", data=b"{}", method="POST",
                                 headers={"X-OverShorts-Token": token, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=3):  # noqa: S310 - 127.0.0.1 고정
            pass
    except (OSError, urllib.error.URLError):
        return False
    end = time.monotonic() + wait
    while time.monotonic() < end:
        if not port_open(port):
            return True
        time.sleep(0.3)
    return False


# --- 창 열기 ---
def _app_paths(exe: str) -> list[Path]:
    """레지스트리 App Paths 에 등록된 실행 파일 (Edge·Chrome 설치 위치)."""
    if os.name != "nt":
        return []
    try:
        import winreg
    except ImportError:
        return []
    found: list[Path] = []
    for root in (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE):
        try:
            with winreg.OpenKey(root, rf"Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}") as key:
                value, _ = winreg.QueryValueEx(key, "")
                if value:
                    found.append(Path(str(value).strip('"')))
        except OSError:
            continue
    return found


def find_app_browser() -> Path | None:
    """앱 창 모드를 지원하는 브라우저: Edge 먼저(윈도우에 기본으로 있다), 다음 Chrome."""
    if os.name == "nt":
        env = [os.environ.get(k, "") for k in ("ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA")]
        for exe, sub in (("msedge.exe", r"Microsoft\Edge\Application"),
                         ("chrome.exe", r"Google\Chrome\Application")):
            candidates = _app_paths(exe) + [Path(base) / sub / exe for base in env if base]
            for c in candidates:
                if c.is_file():
                    return c
        return None
    if sys.platform == "darwin":
        for app in ("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"):
            if Path(app).is_file():
                return Path(app)
        return None
    for name in ("microsoft-edge", "microsoft-edge-stable", "google-chrome", "chromium", "chromium-browser"):
        found = shutil.which(name)
        if found:
            return Path(found)
    return None


def open_window(url: str, profile: Path) -> bool:
    """OverShorts 창을 연다. 앱 창으로 열었으면 True, 기본 브라우저로 대신 열었으면 False."""
    browser = find_app_browser()
    if browser is not None:
        try:
            profile.mkdir(parents=True, exist_ok=True)
            args = [str(browser), f"--app={url}", f"--user-data-dir={profile}",
                    "--no-first-run", "--no-default-browser-check", "--window-size=1440,900",
                    # 최소화·가려진 창도 재우지 않는다: 패널 연결이 끊기면 서버가 '창을 닫았다'고 본다
                    "--disable-background-timer-throttling", "--disable-renderer-backgrounding",
                    "--disable-backgrounding-occluded-windows"]
            flags = 0
            if os.name == "nt":
                flags = 0x0000_0008 | 0x0000_0200  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
            subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,  # noqa: S603
                             stderr=subprocess.DEVNULL, close_fds=True, creationflags=flags)
            return True
        except OSError as err:
            print(f"앱 창을 열지 못해 기본 브라우저로 엽니다: {err}", flush=True)
    import webbrowser

    webbrowser.open(url)
    return False
