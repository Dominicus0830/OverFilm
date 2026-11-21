"""하위 프로그램(node·ffmpeg·claude) 출력 읽기와 작업 취소.

Windows 에서 cmd·node 오류는 시스템 코드 페이지(한국어면 cp949)로 나오고, 정상 출력은 UTF-8 이라
한 가지로 읽으면 글자가 깨진다 (2026-11-15 설치본 렌더 오류가 � 로만 보인 일). 바이트로 받아 여기서 푼다.
"""

from __future__ import annotations

import locale
import os
import signal
import subprocess
import threading
import time
from collections.abc import Sequence
from typing import Any


def decode_output(data: bytes | str | None) -> str:
    """UTF-8 로 읽고, 안 되면 시스템 코드 페이지(cp949 등)로, 그래도 안 되면 깨진 글자만 바꾼다."""
    if data is None:
        return ""
    if isinstance(data, str):
        return data
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        pass
    for enc in (locale.getpreferredencoding(False), "cp949"):
        try:
            return data.decode(enc)
        except (UnicodeDecodeError, LookupError):
            continue
    return data.decode("utf-8", "replace")


# --- 작업 취소 (작업 큐의 '취소' 버튼, 2026-11-21) ---
# 패널의 작업 스레드는 자기 취소 신호를 bind_cancel 로 걸어 둔다. 그 스레드에서 도는 하위 프로그램
# (렌더·Claude Code·FFmpeg…)과 기다림은 취소되면 바로 멈추고 Cancelled 를 던진다.
# 다른 스레드(패널 요청 처리)는 신호가 없으니 예전과 똑같이 돈다.

_local = threading.local()
_POLL_SEC = 0.25


class Cancelled(BaseException):
    """사용자가 작업을 취소했다. BaseException 이라 단계 안의 `except Exception` 에 먹히지 않고
    작업 스레드까지 올라간다 (글 하나 실패로 처리되어 다음 글로 넘어가면 안 된다)."""

    def __init__(self, message: str = "사용자가 취소했습니다") -> None:
        super().__init__(message)


def bind_cancel(event: threading.Event | None) -> None:
    """지금 스레드의 취소 신호를 건다 (작업 스레드가 시작할 때 한 번)."""
    _local.cancel = event


def cancel_event() -> threading.Event | None:
    return getattr(_local, "cancel", None)


def check_cancel() -> None:
    ev = cancel_event()
    if ev is not None and ev.is_set():
        raise Cancelled()


def sleep(seconds: float) -> None:
    """time.sleep 대신. 취소되면 바로 깨어나 Cancelled."""
    ev = cancel_event()
    if ev is None:
        time.sleep(max(0.0, seconds))
        return
    if ev.wait(max(0.0, seconds)):
        raise Cancelled()


def _kill_tree(proc: subprocess.Popen) -> None:
    """node → 크롬·ffmpeg 처럼 자식까지 끈다."""
    try:
        if os.name == "nt":
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True,
                           timeout=15, creationflags=0x0800_0000)
        else:
            os.killpg(proc.pid, signal.SIGKILL)
    except (OSError, subprocess.SubprocessError):
        pass
    try:
        proc.kill()
    except OSError:
        pass


def run(args: Sequence[str], *, input: bytes | None = None, timeout: float | None = None,
        cwd: Any = None, env: dict[str, str] | None = None, creationflags: int = 0,
        ) -> subprocess.CompletedProcess:
    """`subprocess.run(args, capture_output=True, ...)` 과 같다 (bytes). 다만 작업 스레드에서
    취소되면 하위 프로그램을 자식까지 끄고 Cancelled 를 던진다. 시간 초과는 subprocess.TimeoutExpired."""
    ev = cancel_event()
    if ev is None:
        return subprocess.run(list(args), input=input, capture_output=True, timeout=timeout,
                              cwd=cwd, env=env, creationflags=creationflags, check=False)
    check_cancel()
    extra: dict[str, Any] = {} if os.name == "nt" else {"start_new_session": True}
    proc = subprocess.Popen(list(args), stdin=subprocess.PIPE if input is not None else None,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=cwd, env=env,
                            creationflags=creationflags, **extra)
    deadline = None if timeout is None else time.monotonic() + timeout
    pending_input = input
    while True:
        step = _POLL_SEC
        if deadline is not None:
            step = max(0.0, min(step, deadline - time.monotonic()))
        try:
            out, err = proc.communicate(pending_input, timeout=step)
            return subprocess.CompletedProcess(list(args), proc.returncode, out, err)
        except subprocess.TimeoutExpired:
            pending_input = None  # 입력은 첫 communicate 가 이미 보냈다
        if ev.is_set():
            _kill_tree(proc)
            proc.communicate()
            raise Cancelled()
        if deadline is not None and time.monotonic() >= deadline:
            _kill_tree(proc)
            proc.communicate()
            raise subprocess.TimeoutExpired(list(args), timeout or 0.0)
