"""클로닝 음성의 단어 시각 (기획서 5.4).

대본 글자를 이미 알기 때문에 음성 인식이 아니라 강제 정렬을 쓴다.
- stable-ts(MIT, Whisper) 가 깔려 있으면 `align()` 으로 어절마다 시각을 잰다 (CPU 로도 돈다).
- 없거나 실패하면 None 을 돌려주고, assemble 이 음절 수 비율로 나눈다.

설치(선택): pip install stable-ts  (torch 가 같이 깔린다, 처음 쓸 때 Whisper base 약 140MB 받음)
"""

from __future__ import annotations

import io
import threading
import wave

from .engines import Word


def _wav_to_float(audio: bytes):
    """GPT-SoVITS 가 주는 16비트 wav → (float32 배열, 표본율). numpy 필요."""
    import numpy as np

    with wave.open(io.BytesIO(audio), "rb") as w:
        rate, width, ch = w.getframerate(), w.getsampwidth(), w.getnchannels()
        frames = w.readframes(w.getnframes())
    if width != 2:
        raise ValueError("16비트 wav 가 아닙니다")
    a = np.frombuffer(frames, dtype="<i2").astype("float32") / 32768.0
    if ch > 1:
        a = a.reshape(-1, ch).mean(axis=1)
    return a, rate


class StableTsAligner:
    """모델은 한 번만 올리고, 음성 단계가 끝나면 unload() 로 내린다."""

    def __init__(self, model: str = "base", language: str = "ko"):
        self.model_name = model
        self.language = language
        self.model = None
        self.lock = threading.Lock()

    @staticmethod
    def available() -> bool:
        try:
            import importlib.util

            return importlib.util.find_spec("stable_whisper") is not None
        except (ImportError, ValueError):
            return False

    def align(self, audio: bytes, text: str) -> list[Word] | None:
        tokens = [t for t in text.split() if t]
        if not tokens:
            return None
        try:
            import numpy as np
            import stable_whisper

            samples, rate = _wav_to_float(audio)
            if rate != 16_000:  # Whisper 는 16kHz
                n = max(1, int(len(samples) * 16_000 / rate))
                samples = np.interp(np.linspace(0, len(samples) - 1, n),
                                    np.arange(len(samples)), samples).astype("float32")
            with self.lock:
                if self.model is None:
                    self.model = stable_whisper.load_model(self.model_name, device="cpu")
                result = self.model.align(samples, text, language=self.language)
            words = [w for seg in result.segments for w in seg.words]
        except Exception:  # noqa: BLE001 - 정렬은 덤: 실패하면 음절 비율로
            return None
        got = [(str(w.word).strip(), float(w.start), float(w.end)) for w in words if str(w.word).strip()]
        if len(got) != len(tokens):
            return None  # 어절 수가 다르면 믿지 않는다
        return [Word(tok, max(0.0, s), max(s, e)) for tok, (_, s, e) in zip(tokens, got)]

    def unload(self) -> None:
        with self.lock:
            self.model = None


def make_aligner(mode: str, model: str = "base") -> StableTsAligner | None:
    if mode == "syllable":
        return None
    if mode in ("auto", "stable_ts") and StableTsAligner.available():
        return StableTsAligner(model)
    return None
