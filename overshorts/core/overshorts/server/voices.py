"""보이스 화면 API (M6, 기획서 3.7): 목소리 목록·선택·미리듣기, 클로닝 등록, 작업별 목소리.

app.py 는 `app.include_router(voices_router(...))` 한 줄만 둔다 (다른 화면과 파일을 나눠 충돌을 줄임).
"""

from __future__ import annotations

import base64
import binascii
import hashlib
from pathlib import Path
from typing import Any

from fastapi import APIRouter, HTTPException
from fastapi.responses import FileResponse, Response
from pydantic import BaseModel, Field

from ..db import Store
from ..tts.engines import EdgeTts, TtsError, parse_voice
from ..tts.voices import (
    CONSENT_TEXT,
    READ_SENTENCES,
    REF_MAX_SEC,
    REF_MIN_SEC,
    VoiceLibrary,
    clean_text,
    job_override,
    job_override_path,
    resolve,
)

PREVIEW_DEFAULT = "오늘은 오버워치 갤러리에서 화제가 된 글을 가져왔어. 끝까지 보면 반전 있음."
_VOICE_SPEC = r"^(edge:[A-Za-z0-9-]{4,60}|clone:\d{1,9}|same)$"


class SelectionIn(BaseModel):
    narrator: str | None = Field(default=None, pattern=_VOICE_SPEC)
    comment: str | None = Field(default=None, pattern=_VOICE_SPEC)


class PreviewIn(BaseModel):
    voice: str = Field(pattern=_VOICE_SPEC)
    text: str = Field(default="", max_length=200)


class RegisterIn(BaseModel):
    name: str = Field(min_length=1, max_length=40)
    data: str = Field(max_length=21_000_000, description="녹음·업로드 파일 base64 (15MB 까지)")
    ref_text: str = Field(min_length=1, max_length=200, description="참조 구간에서 한 말 그대로")
    consent: bool = False
    start: float | None = Field(default=None, ge=0, le=600)
    end: float | None = Field(default=None, ge=0, le=600)
    lang: str = Field(default="ko", pattern="^(ko|all_ko|en|ja|zh|auto)$")
    gpt_weights: str = Field(default="", max_length=500)
    sovits_weights: str = Field(default="", max_length=500)


class RenameIn(BaseModel):
    name: str = Field(min_length=1, max_length=40)


class ServerIn(BaseModel):
    action: str = Field(pattern="^(start|stop)$")


class JobVoiceIn(BaseModel):
    narrator: str | None = Field(default=None, pattern=_VOICE_SPEC)
    comment: str | None = Field(default=None, pattern=_VOICE_SPEC)


def voices_router(cfg: dict[str, Any], data: Path, db_path: Path, worker: Any) -> APIRouter:
    from ..tts.align import StableTsAligner, make_aligner
    from ..tts.gpt_sovits import CloneEngine, CloneSettings, GptSovitsServer
    from ..tts.stage import cpu_warning, fallback_voices

    r = APIRouter(prefix="/api/voices")
    root = data / "voices"
    cfg_voice = cfg.get("voice") or {}
    settings = CloneSettings.from_cfg(cfg)
    state: dict[str, Any] = {}

    def server() -> GptSovitsServer:
        return GptSovitsServer.shared(settings, root / "gpt-sovits.log")

    def lib(store: Store) -> VoiceLibrary:
        return VoiceLibrary(store, root)

    def fail(err: Exception, code: int = 400) -> HTTPException:
        return HTTPException(code, str(err))

    @r.get("")
    def list_voices() -> dict[str, Any]:
        s = Store(db_path)
        try:
            v = lib(s)
            sel = v.selection()
            narrator, comment = resolve(cfg_voice, sel)
            voices = v.all_voices()
        finally:
            s.close()
        if "cpu" not in state:
            state["cpu"] = cpu_warning()
        srv = server()
        return {
            "voices": voices, "selection": sel,
            "effective": {"narrator": narrator, "comment": comment},
            "config": {"narrator": cfg_voice.get("narrator"), "comment": cfg_voice.get("comment")},
            "fallback": dict(zip(("narrator", "comment"), fallback_voices(cfg_voice))),
            "read_sentences": READ_SENTENCES, "consent_text": CONSENT_TEXT,
            "ref_sec": [REF_MIN_SEC, REF_MAX_SEC],
            "clone": {"server": settings.server, "dir": settings.dir, "alive": srv.alive(1.0),
                      "managed": srv.managed, "cpu_warning": state["cpu"],
                      "align": "stable_ts" if settings.align != "syllable"
                      and StableTsAligner.available() else "syllable"},
        }

    @r.post("/selection")
    def select(body: SelectionIn) -> dict[str, Any]:
        s = Store(db_path)
        try:
            sel = lib(s).save_selection(body.model_dump(exclude_unset=True))
        except TtsError as err:
            raise fail(err) from err
        finally:
            s.close()
        return {"ok": True, "selection": sel}

    @r.post("/register")
    def register(body: RegisterIn) -> dict[str, Any]:
        try:
            audio = base64.b64decode(body.data.split(",")[-1], validate=True)
        except (binascii.Error, ValueError) as err:
            raise HTTPException(400, "음성 파일을 읽지 못했습니다") from err
        s = Store(db_path)
        try:
            voice = lib(s).register(body.name, audio, body.ref_text, body.consent, body.start,
                                    body.end, body.lang, body.gpt_weights, body.sovits_weights)
        except TtsError as err:
            raise fail(err) from err
        finally:
            s.close()
        return {"ok": True, "voice": voice.to_json()}

    @r.post("/{voice_id}/rename")
    def rename(voice_id: int, body: RenameIn) -> dict[str, Any]:
        s = Store(db_path)
        try:
            lib(s).rename(voice_id, body.name)
        except TtsError as err:
            raise fail(err) from err
        finally:
            s.close()
        return {"ok": True}

    @r.delete("/{voice_id}")
    def delete(voice_id: int) -> dict[str, Any]:
        s = Store(db_path)
        try:
            lib(s).delete(voice_id)
        finally:
            s.close()
        return {"ok": True}

    @r.get("/{voice_id}/ref")
    def ref_audio(voice_id: int) -> FileResponse:
        s = Store(db_path)
        try:
            v = lib(s).get(voice_id)
        except TtsError as err:
            raise fail(err, 404) from err
        finally:
            s.close()
        return FileResponse(v.ref_wav, media_type="audio/wav")

    @r.post("/preview")
    def preview(body: PreviewIn) -> Response:
        """▶ 미리듣기: 같은 목소리·문장은 저장해 두고 다시 쓴다. 클로닝은 서버가 꺼져 있으면 띄운다."""
        text = clean_text(body.text) or PREVIEW_DEFAULT
        engine_name, voice = parse_voice(body.voice)
        if body.voice == "same":
            raise HTTPException(400, "미리들을 목소리를 골라 주세요")
        cache = root / "previews"
        key = hashlib.sha1(f"{body.voice}\n{text}".encode()).hexdigest()[:16]
        for ext, media in ((".wav", "audio/wav"), (".mp3", "audio/mpeg")):
            if (cache / (key + ext)).is_file():
                return FileResponse(cache / (key + ext), media_type=media)
        s = Store(db_path)
        try:
            if engine_name == "clone":
                v = lib(s)
                align = make_aligner("syllable")
                synth = CloneEngine(v, server(), align).synthesize(text, voice)
                ext, media = ".wav", "audio/wav"
            else:
                synth = EdgeTts().synthesize(text, voice)
                ext, media = ".mp3", "audio/mpeg"
        except TtsError as err:
            raise fail(err, 502 if engine_name == "edge" else 400) from err
        finally:
            s.close()
        try:
            cache.mkdir(parents=True, exist_ok=True)
            old = sorted(cache.glob("*.*"), key=lambda p: p.stat().st_mtime)
            for p in old[:-40]:  # 미리듣기는 40개까지만 남긴다
                p.unlink(missing_ok=True)
            (cache / (key + ext)).write_bytes(synth.audio)
        except OSError:
            pass
        return Response(synth.audio, media_type=media)

    @r.post("/server")
    def server_ctl(body: ServerIn) -> dict[str, Any]:
        srv = server()
        try:
            if body.action == "start":
                srv.ensure_running(wait=False)
            else:
                srv.stop()
        except TtsError as err:
            raise fail(err) from err
        return {"ok": True, "alive": srv.alive(1.0), "managed": srv.managed}

    # --- 검수: 이 작업만 다른 목소리 ---
    @r.get("/job/{job_id}")
    def job_voice(job_id: int) -> dict[str, Any]:
        jobs = data / "jobs"
        s = Store(db_path)
        try:
            sel = lib(s).selection()
        finally:
            s.close()
        over = job_override(jobs, job_id)
        narrator, comment = resolve(cfg_voice, sel, over)
        return {"override": over, "effective": {"narrator": narrator, "comment": comment}}

    @r.post("/job/{job_id}")
    def set_job_voice(job_id: int, body: JobVoiceIn) -> dict[str, Any]:
        s = Store(db_path)
        try:
            row = s.conn.execute("SELECT status, script_id FROM jobs WHERE id=?", (job_id,)).fetchone()
            v = lib(s)
            for spec in (body.narrator, body.comment):
                if spec and spec != "same" and not v.known(spec):
                    raise HTTPException(400, f"모르는 목소리입니다: {spec}")
        finally:
            s.close()
        if row is None:
            raise HTTPException(404, "작업이 없습니다")
        if row["script_id"] is None or row["status"] in ("approved", "rendered"):
            raise HTTPException(409, "대본이 있고 아직 승인 전인 작업만 목소리를 바꿀 수 있습니다")
        path = job_override_path(data / "jobs", job_id)
        over = {k: v for k, v in body.model_dump().items() if v}
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            if over:
                import json

                path.write_text(json.dumps(over, ensure_ascii=False), encoding="utf-8")
            else:
                path.unlink(missing_ok=True)  # 비우면 보이스 화면 기본값으로
        except OSError as err:
            raise HTTPException(500, f"저장 실패: {err}") from err

        def run(p: Any) -> None:
            p.store.set_job_status(job_id, "scripted", None)
            p.voice()
            p.preview()
            p.emit({"type": "jobs_changed"})

        worker.submit(f"작업 {job_id} 목소리 바꾸기", run)
        return {"ok": True, "override": over}

    return r
