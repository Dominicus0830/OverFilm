"""하위 프로그램(node·ffmpeg·claude) 출력 읽기.

Windows 에서 cmd·node 오류는 시스템 코드 페이지(한국어면 cp949)로 나오고, 정상 출력은 UTF-8 이라
한 가지로 읽으면 글자가 깨진다 (2026-11-15 설치본 렌더 오류가 � 로만 보인 일). 바이트로 받아 여기서 푼다.
"""

from __future__ import annotations

import locale


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
