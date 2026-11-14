"""대본 → 한 개의 wav + 문장·단어 시각 JSON (M3, 기획서 5.3·5.4).

- 문장 단위로 합성하고 사이에 0.15초 간격을 둔다. 문장 시작·끝 시각은 여기서 정확히 정해진다.
- 단어 시각: 엔진이 주면 그대로(Edge-TTS), 없으면 문장 길이를 음절 수 비율로 나눈다
  (클로닝·유료 API 의 강제 정렬은 M6).
- 댓글 줄은 댓글 보이스로 읽는다 (화자 분리, 선택).
오디오 디코딩은 FFmpeg(외부 실행, 기획서 6.4 필수 항목)로 48kHz 모노 16비트로 맞춘다.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import wave
from dataclasses import asdict, dataclass, field
from pathlib import Path

from .engines import Engine, Synth, TtsError, Word, get_engine, parse_voice

SAMPLE_RATE = 48_000
GAP_SEC = 0.15
_HANGUL_OR_ALNUM = re.compile(r"[가-힣A-Za-z0-9]")
# TTS 가 소리 내지 않을 기호 (따옴표·괄호 등)
_SPOKEN_CLEAN = re.compile(r"[\"'“”‘’()\[\]{}<>«»「」『』]")


@dataclass
class TimedWord:
    text: str
    start: float
    end: float


@dataclass
class TimedSentence:
    line: int
    part: str
    speaker: str  # narrator | comment
    text: str
    start: float
    end: float
    words: list[TimedWord] = field(default_factory=list)
    timing: str = "engine"  # engine | syllable


@dataclass
class VoiceTrack:
    wav: str
    duration: float
    sentences: list[TimedSentence]
    segments: list[dict] = field(default_factory=list)  # [{"seg", "start", "end"}]

    def to_json(self) -> dict:
        return {"sample_rate": SAMPLE_RATE, "duration": self.duration, "wav": self.wav,
                "sentences": [asdict(s) for s in self.sentences], "segments": self.segments}


def split_for_tts(text: str) -> list[str]:
    parts = re.split(r"(?<=[.?!])\s+", (text or "").strip())
    return [p.strip() for p in parts if _HANGUL_OR_ALNUM.search(p or "")]


def decode_to_pcm(audio: bytes, ffmpeg: str | None = None) -> bytes:
    exe = ffmpeg or shutil.which("ffmpeg")
    if not exe:
        raise TtsError("FFmpeg 가 없어 음성을 합칠 수 없습니다 (설치 후 PATH 에 추가)")
    try:
        proc = subprocess.run(
            [exe, "-hide_banner", "-loglevel", "error", "-i", "pipe:0", "-f", "s16le", "-ac", "1",
             "-ar", str(SAMPLE_RATE), "pipe:1"],
            input=audio, capture_output=True, timeout=120,
        )
    except (OSError, subprocess.TimeoutExpired) as err:
        raise TtsError(f"FFmpeg 실행 실패: {err}") from err
    if proc.returncode != 0 or not proc.stdout:
        raise TtsError(f"음성을 풀지 못했습니다: {proc.stderr.decode('utf-8', 'replace')[:200]}")
    return proc.stdout


def syllable_words(text: str, start: float, end: float) -> list[TimedWord]:
    """정렬 실패 시: 어절마다 음절 수 비율로 시간을 나눈다."""
    tokens = [t for t in text.split() if t]
    weights = [max(1, len(_HANGUL_OR_ALNUM.findall(t))) for t in tokens]
    total = sum(weights)
    span = max(0.0, end - start)
    out, t = [], start
    for tok, w in zip(tokens, weights):
        d = span * w / total if total else 0.0
        out.append(TimedWord(tok, round(t, 3), round(t + d, 3)))
        t += d
    return out


def _engine_words(words: list[Word], offset: float, length: float) -> list[TimedWord]:
    out = []
    for w in words:
        s = min(max(0.0, w.start), length)
        e = min(max(s, w.end), length)
        out.append(TimedWord(w.text, round(offset + s, 3), round(offset + e, 3)))
    return out


def build_voice(lines: list[dict], out_dir: Path, narrator: str, comment_voice: str | None,
                engines: dict[str, Engine] | None = None,
                decode=decode_to_pcm) -> VoiceTrack:
    """lines: 대본 줄 [{"part", "text", "seg"}]. out_dir 에 voice.wav, timing.json 을 쓴다.

    part = "pause" 줄은 소리 대신 무음을 넣어 그 세그먼트가 적어도 min_total 초 화면에 있게 한다
    (TTS 를 끈 세그먼트, 댓글·사진을 읽을 시간).
    """
    engines = dict(engines or {})
    pcm = bytearray()
    sentences: list[TimedSentence] = []
    gap = b"\x00\x00" * int(SAMPLE_RATE * GAP_SEC)
    spans: dict[int, list[float]] = {}

    def now() -> float:
        return len(pcm) / 2 / SAMPLE_RATE

    for i, line in enumerate(lines):
        part = str(line.get("part") or "body")
        seg_raw = line.get("seg")
        seg = int(seg_raw) if isinstance(seg_raw, int) and 0 <= seg_raw < 100 else 0
        span = spans.setdefault(seg, [now(), now()])
        if part == "pause":
            try:
                want = float(line.get("min_total") or 0)
            except (TypeError, ValueError):
                want = 0.0
            need = min(30.0, max(0.0, want - (now() - span[0])))
            pcm.extend(b"\x00\x00" * int(SAMPLE_RATE * need))
            span[1] = now()
            continue
        speaker = "comment" if part == "comment" and comment_voice else "narrator"
        engine_name, voice = parse_voice(comment_voice if speaker == "comment" else narrator)
        engine = engines.get(engine_name)
        if engine is None:
            engine = engines[engine_name] = get_engine(engine_name)
        for sent in split_for_tts(str(line.get("text") or "")):
            spoken = _SPOKEN_CLEAN.sub("", sent)
            synth: Synth = engine.synthesize(spoken, voice)
            audio = decode(synth.audio)
            if len(audio) % 2:
                audio = audio[:-1]
            if pcm:
                pcm.extend(gap)
            start = len(pcm) / 2 / SAMPLE_RATE
            pcm.extend(audio)
            end = len(pcm) / 2 / SAMPLE_RATE
            if len(pcm) > SAMPLE_RATE * 2 * 600:
                raise TtsError("음성이 10분을 넘습니다")
            if synth.words:
                words = _engine_words(synth.words, start, end - start)
                timing = "engine"
            else:
                words = syllable_words(sent, start, end)
                timing = "syllable"
            sentences.append(TimedSentence(i, part, speaker, sent, round(start, 3), round(end, 3),
                                           words, timing))
            span[1] = now()
    if not pcm:
        raise TtsError("읽을 문장이 없습니다")
    if len(pcm) > SAMPLE_RATE * 2 * 600:
        raise TtsError("음성이 10분을 넘습니다")
    out_dir.mkdir(parents=True, exist_ok=True)
    wav_path = out_dir / "voice.wav"
    with wave.open(str(wav_path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE)
        w.writeframes(bytes(pcm))
    seg_list = [{"seg": k, "start": round(v[0], 3), "end": round(v[1], 3)}
                for k, v in sorted(spans.items())]
    track = VoiceTrack(str(wav_path), round(len(pcm) / 2 / SAMPLE_RATE, 3), sentences, seg_list)
    (out_dir / "timing.json").write_text(json.dumps(track.to_json(), ensure_ascii=False, indent=1),
                                         encoding="utf-8")
    return track
