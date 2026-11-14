"""TTS 엔진. 지금은 Edge-TTS(기성 보이스 기본값)만. 클로닝(GPT-SoVITS)은 M6.

Edge-TTS: 무료, 단어 경계 시각을 준다. 비공식 엔드포인트라 언제든 막힐 수 있고 상업 이용 조건이
불명확하다 → 수익화 채널은 유료 API(Supertone·Typecast) 권장 (기획서 5.3).
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Protocol

# Edge 의 offset·duration 단위 (100ns)
_TICKS_PER_SEC = 10_000_000


class TtsError(Exception):
    pass


@dataclass
class Word:
    text: str
    start: float  # 초, 이 문장 오디오 기준
    end: float


@dataclass
class Synth:
    audio: bytes  # 인코딩된 오디오 (mp3 등). assemble 이 ffmpeg 로 PCM 으로 푼다
    words: list[Word] | None  # 엔진이 단어 시각을 안 주면 None


class Engine(Protocol):
    name: str

    def synthesize(self, text: str, voice: str) -> Synth: ...


class EdgeTts:
    name = "edge"

    def __init__(self, rate: str = "+0%", timeout: float = 60.0):
        self.rate = rate
        self.timeout = max(5.0, min(timeout, 600.0))

    def synthesize(self, text: str, voice: str) -> Synth:
        try:
            import edge_tts
        except ImportError as err:
            raise TtsError("edge-tts 가 설치되어 있지 않습니다: pip install edge-tts") from err

        async def run() -> Synth:
            com = edge_tts.Communicate(text, voice, rate=self.rate, boundary="WordBoundary")
            audio = bytearray()
            words: list[Word] = []
            async for chunk in com.stream():
                if chunk.get("type") == "audio":
                    audio.extend(chunk.get("data") or b"")
                    if len(audio) > 50 * 1024 * 1024:
                        raise TtsError("음성이 비정상적으로 깁니다")
                elif chunk.get("type") == "WordBoundary":
                    start = float(chunk.get("offset", 0)) / _TICKS_PER_SEC
                    dur = float(chunk.get("duration", 0)) / _TICKS_PER_SEC
                    words.append(Word(str(chunk.get("text", "")), start, start + max(0.0, dur)))
            return Synth(bytes(audio), words or None)

        try:
            return asyncio.run(asyncio.wait_for(run(), self.timeout))
        except TtsError:
            raise
        except asyncio.TimeoutError as err:
            raise TtsError(f"Edge-TTS 가 {int(self.timeout)}초 안에 답하지 않았습니다") from err
        except Exception as err:  # noqa: BLE001 - 네트워크·서버 오류
            raise TtsError(f"Edge-TTS 합성 실패 (막혔을 수 있음): {err}") from err


def parse_voice(spec: str) -> tuple[str, str]:
    """'edge:ko-KR-SunHiNeural' → ('edge', 'ko-KR-SunHiNeural')."""
    engine, sep, voice = (spec or "").partition(":")
    if not sep or not voice:
        return "edge", spec or "ko-KR-SunHiNeural"
    return engine, voice


def get_engine(name: str) -> Engine:
    if name == "edge":
        return EdgeTts()
    raise TtsError(f"아직 지원하지 않는 TTS 엔진: {name} (M6 에서 클로닝 추가)")
