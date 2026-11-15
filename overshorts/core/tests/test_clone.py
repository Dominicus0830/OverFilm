"""목소리 클로닝 (M6): 등록 검사, 고르기 우선순위, 클로닝 실패 시 Edge 대체, 서버 응답 처리."""

import json
import wave

import pytest
from test_tts import LINES, FakeEngine, fake_decode

from overshorts.db import Store
from overshorts.tts import voices as voices_mod
from overshorts.tts.engines import TtsError
from overshorts.tts.gpt_sovits import CloneSettings, _message
from overshorts.tts.stage import fallback_voices, voice_pending
from overshorts.tts.voices import SAMPLE_RATE, VoiceLibrary, resolve, speech_span


def tone(sec: float, silence: float = 1.0) -> bytes:
    """앞뒤 무음 + 가운데 소리 (16비트 PCM)."""
    import math

    quiet = b"\x00\x00" * int(SAMPLE_RATE * silence)
    loud = b"".join(int(9000 * math.sin(i / 7)).to_bytes(2, "little", signed=True)
                    for i in range(int(SAMPLE_RATE * sec)))
    return quiet + loud + quiet


def test_speech_span_trims_silence():
    s, e = speech_span(tone(4.0))
    assert 0.8 <= s <= 1.0 and 4.9 <= e <= 5.2


@pytest.fixture
def lib(tmp_path, monkeypatch):
    monkeypatch.setattr(voices_mod, "to_pcm", lambda audio, max_sec=90: audio)  # FFmpeg 없이
    return VoiceLibrary(Store(":memory:"), tmp_path / "voices")


def test_register_checks(lib):
    text = "오늘은 오버워치 갤러리에서 가져왔어"
    with pytest.raises(TtsError, match="동의"):
        lib.register("나", tone(5), text, consent=False)
    with pytest.raises(TtsError, match="3~10초"):
        lib.register("나", tone(14), text, consent=True)
    with pytest.raises(TtsError, match="3초"):
        lib.register("나", tone(2), text, consent=True)
    v = lib.register("나", tone(14), text, consent=True, start=1.0, end=9.0)
    with wave.open(v.ref_wav) as w:
        assert 7.5 <= w.getnframes() / w.getframerate() <= 8.2
    assert v.spec == "clone:1" and lib.known("clone:1") and not lib.known("clone:2")
    lib.save_selection({"narrator": "clone:1", "comment": "same"})
    assert resolve({"narrator": "edge:a"}, lib.selection()) == ("clone:1", None)
    assert resolve({}, lib.selection(), {"narrator": "edge:ko-KR-InJoonNeural"})[0] == "edge:ko-KR-InJoonNeural"
    with pytest.raises(TtsError):
        lib.save_selection({"narrator": "clone:9"})
    lib.delete(1)
    assert lib.selection().get("narrator") is None and lib.clones() == []


class FailingClone:
    name = "clone"

    def synthesize(self, text, voice):
        raise TtsError("서버 꺼짐")


def test_clone_failure_falls_back_to_edge(tmp_path):
    store = Store(":memory:")
    pid = store.upsert_post({"gall_id": "g", "post_no": 1, "title": "t", "body": "b"})
    job = store.add_job(pid, "scripted")
    sid = store.add_script(pid, 1, "담백", {"lines": LINES}, 0.1, "x")
    store.set_job_status(job, "scripted", None, script_id=sid)
    edge = FakeEngine()
    counts = voice_pending(store, tmp_path, "clone:1", None, {"edge": edge, "clone": FailingClone()},
                           fake_decode, fallback=fallback_voices({}))
    assert counts == {"voiced": 1, "failed": 0, "fallback": 1}
    assert {v for _, v in edge.calls} == {"ko-KR-SunHiNeural"}
    timing = json.loads((tmp_path / str(job) / "timing.json").read_text(encoding="utf-8"))
    assert "클로닝 실패" in timing["warnings"][0]


def test_settings_and_server_messages():
    s = CloneSettings.from_cfg({"voice": {"clone": {"server": "127.0.0.1:9000", "speed": 9}}})
    assert s.server == "http://127.0.0.1:9000" and s.speed == 1.6
    assert "3~10초" in _message('{"message":"tts failed","Exception":"参考音频在3~10秒范围外，请更换！"}'.encode())
