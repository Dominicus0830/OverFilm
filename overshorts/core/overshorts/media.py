"""원문 미디어 크기·길이 재기 (기획서 3.9 1단계 '비율·길이 측정').

그림은 파일 머리만 읽어 크기를 잰다 (의존성 없음). 동영상은 ffprobe(외부 실행).
"""

from __future__ import annotations

import json
import shutil
import struct
import subprocess
from pathlib import Path


def image_size(data: bytes) -> tuple[int, int] | None:
    """JPEG·PNG·GIF·WebP 의 (가로, 세로). 모르면 None."""
    if data[:8] == b"\x89PNG\r\n\x1a\n" and len(data) >= 24:
        w, h = struct.unpack(">II", data[16:24])
        return w, h
    if data[:6] in (b"GIF87a", b"GIF89a") and len(data) >= 10:
        w, h = struct.unpack("<HH", data[6:10])
        return w, h
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP" and len(data) >= 30:
        chunk = data[12:16]
        if chunk == b"VP8X":
            w = 1 + int.from_bytes(data[24:27], "little")
            h = 1 + int.from_bytes(data[27:30], "little")
            return w, h
        if chunk == b"VP8 " and len(data) >= 30:
            w, h = struct.unpack("<HH", data[26:30])
            return w & 0x3FFF, h & 0x3FFF
        if chunk == b"VP8L" and len(data) >= 25:
            b = data[21:25]
            w = 1 + (((b[1] & 0x3F) << 8) | b[0])
            h = 1 + (((b[3] & 0xF) << 10) | (b[2] << 2) | ((b[1] & 0xC0) >> 6))
            return w, h
    if data[:2] == b"\xff\xd8":
        i = 2
        n = len(data)
        while i + 9 < n:
            if data[i] != 0xFF:
                i += 1
                continue
            marker = data[i + 1]
            if marker in (0xD8, 0x01) or 0xD0 <= marker <= 0xD7:
                i += 2
                continue
            seg_len = struct.unpack(">H", data[i + 2:i + 4])[0]
            if 0xC0 <= marker <= 0xCF and marker not in (0xC4, 0xC8, 0xCC):
                h, w = struct.unpack(">HH", data[i + 5:i + 9])
                return w, h
            i += 2 + max(2, seg_len)
    return None


def file_image_size(path: str | Path) -> tuple[int, int] | None:
    try:
        with open(path, "rb") as f:
            return image_size(f.read(512 * 1024))
    except OSError:
        return None


def video_info(path: str | Path, ffprobe: str | None = None) -> dict | None:
    """{"width", "height", "seconds", "audio"} 또는 None."""
    exe = ffprobe or shutil.which("ffprobe")
    if not exe:
        return None
    try:
        proc = subprocess.run(
            [exe, "-v", "error", "-show_entries", "stream=codec_type,width,height:format=duration",
             "-of", "json", str(path)], capture_output=True, timeout=30)
        data = json.loads(proc.stdout or b"{}")
    except (OSError, subprocess.TimeoutExpired, json.JSONDecodeError):
        return None
    streams = data.get("streams") or []
    v = next((s for s in streams if s.get("codec_type") == "video"), None)
    if v is None:
        return None
    try:
        seconds = float((data.get("format") or {}).get("duration") or 0)
    except (TypeError, ValueError):
        seconds = 0.0
    return {"width": int(v.get("width") or 0), "height": int(v.get("height") or 0),
            "seconds": round(seconds, 2),
            "audio": any(s.get("codec_type") == "audio" for s in streams)}
