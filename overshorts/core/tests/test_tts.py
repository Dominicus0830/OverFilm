import json
import wave

from overshorts.db import Store
from overshorts.tts.assemble import GAP_SEC, SAMPLE_RATE, build_voice, split_for_tts, syllable_words
from overshorts.tts.engines import Synth, Word, parse_voice
from overshorts.tts.stage import voice_pending


class FakeEngine:
    name = "edge"

    def __init__(self, with_words=True):
        self.calls = []
        self.with_words = with_words

    def synthesize(self, text, voice):
        self.calls.append((text, voice))
        words = None
        if self.with_words:
            words = [Word(t, 0.1 + i * 0.3, 0.35 + i * 0.3) for i, t in enumerate(text.split())]
        return Synth(text.encode(), words)


def fake_decode(audio: bytes) -> bytes:
    return b"\x01\x00" * int(SAMPLE_RATE * 0.25 * max(1, len(audio.decode().split())))


LINES = [
    {"part": "hook", "text": "받은 힐량이 \"화면에\" 뜬다. 딜러는 반긴다."},
    {"part": "comment", "text": "이거 진짜 필요했음"},
    {"part": "outro", "text": "숫자는 거짓말을 안 한다."},
]


def test_build_voice_times_and_files(tmp_path):
    eng = FakeEngine()
    track = build_voice(LINES, tmp_path, "edge:narr", "edge:cmt", {"edge": eng}, fake_decode)
    assert [s.text for s in track.sentences] == [
        "받은 힐량이 \"화면에\" 뜬다.", "딜러는 반긴다.", "이거 진짜 필요했음", "숫자는 거짓말을 안 한다."]
    assert eng.calls[0] == ("받은 힐량이 화면에 뜬다.", "narr")  # 따옴표는 읽지 않음
    assert eng.calls[2][1] == "cmt" and track.sentences[2].speaker == "comment"
    s0, s1 = track.sentences[0], track.sentences[1]
    assert s0.start == 0 and abs(s1.start - (s0.end + GAP_SEC)) < 1e-3
    assert all(w.start >= s1.start and w.end <= s1.end for w in s1.words)
    with wave.open(track.wav) as w:
        assert w.getframerate() == SAMPLE_RATE and abs(w.getnframes() / SAMPLE_RATE - track.duration) < 1e-3
    data = json.loads((tmp_path / "timing.json").read_text(encoding="utf-8"))
    assert data["sentences"][0]["words"][0]["text"] == "받은"


def test_syllable_fallback(tmp_path):
    track = build_voice(LINES[:1], tmp_path, "edge:n", None, {"edge": FakeEngine(False)}, fake_decode)
    s = track.sentences[0]
    assert s.timing == "syllable" and s.words[0].start == s.start and abs(s.words[-1].end - s.end) < 1e-2
    words = syllable_words("가 나다", 0.0, 3.0)
    assert (words[0].end, words[1].end) == (1.0, 3.0)


def test_split_and_voice_spec():
    assert split_for_tts("하나. 둘? ... 셋!") == ["하나.", "둘?", "셋!"]
    assert parse_voice("edge:ko-KR-InJoonNeural") == ("edge", "ko-KR-InJoonNeural")


def test_stage(tmp_path):
    store = Store(":memory:")
    pid = store.upsert_post({"gall_id": "g", "post_no": 1, "title": "t", "body": "b"})
    job = store.add_job(pid, "scripted")
    sid = store.add_script(pid, 1, "담백", {"lines": LINES}, 0.1, "x")
    store.set_job_status(job, "scripted", None, script_id=sid)
    counts = voice_pending(store, tmp_path, "edge:n", "edge:c", {"edge": FakeEngine()}, fake_decode)
    assert counts == {"voiced": 1, "failed": 0}
    assert store.conn.execute("SELECT status FROM jobs").fetchone()["status"] == "aligned"
    assert (tmp_path / str(job) / "voice.wav").exists()
