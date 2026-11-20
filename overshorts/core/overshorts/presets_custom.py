"""커스텀 편집 유형 만들기 (S18): Claude Code 와 대화로 규칙을 정해 프리셋 JSON 으로 저장한다.

- 대화는 로컬 `claude -p` (사용자 구독)로 한다. Router 의 'preset' 작업으로 묻는다.
- 예시 영상(링크 또는 파일 이름)은 선택이다. Claude 는 영상을 직접 볼 수 없어서 링크·파일 이름과
  사용자가 말로 적은 설명만 참고하고, 없어도 대화만으로 만든다.
- 결과는 `Preset` 스키마로 검증하고 요약을 사용자에게 보여 준 뒤, 사용자가 저장을 눌러야 파일로 쓴다
  (`<style 폴더>/presets/<id>.json`, 유형 목록에 바로 나타난다). 이 파일은 저장 전에 한 번 더 읽어 확인한다.
- 내장 유형(이슈·옵갤·정보·게임)은 덮어쓰지 않는다. 같은 id 로 저장하면 -2, -3 을 붙인다.
"""

from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import Any

from pydantic import BaseModel, Field, ValidationError

from .llm import UNTRUSTED_RULE, Router, Task, untrusted
from .presets import Preset, load_presets, parse_preset

MAX_TURNS = 30
MAX_REFS = 5


class ChatTurn(BaseModel):
    role: str = Field(pattern="^(user|assistant)$")
    text: str = Field(max_length=4000)


class PresetChatAnswer(BaseModel):
    reply: str = Field(max_length=2000, description="사용자에게 보여 줄 말 (질문이나 방금 정한 내용 설명)")
    preset: Preset | None = Field(
        default=None, description="규칙이 어느 정도 정해졌으면 지금까지의 전체 프리셋. 더 물어야 하면 null")


_SYSTEM = (
    "너는 오버워치 커뮤 글을 세로 쇼츠로 만드는 도구의 '편집 유형' 설계 도우미다. 사용자와 대화하며 "
    "유형 하나의 편집 규칙을 정해 주어진 JSON 스키마(preset)로 낸다. "
    "한 번에 한두 가지만 짧게 묻는다: 어떤 영상인지(목표 길이), 댓글을 얼마나 보일지, 클립·짤 비중, "
    "TTS(읽기) 여부, 자막 모양, 효과. 사용자가 '알아서'라고 하면 스스로 정하고 이유를 말한다. "
    "대화가 쌓이면 매번 지금까지 정한 전체 preset 을 함께 낸다 (바뀐 부분만이 아니라 전부). "
    "preset.id 는 영소문자·숫자·-·_ 1~32자, label 은 20자 이내 한국어 이름, guide 에는 이 유형만의 편집 규칙을 "
    "대본 작가에게 그대로 줄 문장으로 쓴다. 숫자 범위 안에서 쓰고 모르는 칸은 비워 둔다(기본값이 채워진다). "
    "사용자가 시키지 않은 규칙을 넣지 않는다: 욕설·표현 순화, 제목·댓글 고치기, 대본 검사 같은 것은 "
    "사용자가 말한 경우에만 guide 에 쓴다 (기본은 원문 그대로). "
    "영상을 직접 볼 수 없으니 예시 영상은 링크·파일 이름과 사용자가 적은 설명으로만 짐작하고, "
    "보지 못했다는 사실을 숨기지 않는다. " + UNTRUSTED_RULE.replace("<post>, <comments>", "<reference>")
)


def clean_refs(refs: Any) -> list[str]:
    """예시 영상: 링크나 파일 이름 문자열만, 최대 5개. 파일은 이름만 남긴다 (경로는 모델에 보내지 않는다)."""
    if not isinstance(refs, list):
        return []
    out = []
    for r in refs[:MAX_REFS]:
        t = str(r or "").strip()[:300]
        if not t:
            continue
        if not re.match(r"^https?://", t, re.I):
            t = os.path.basename(t.replace("\\", "/"))
        out.append(t)
    return out


def chat_task(history: list[ChatTurn], refs: list[str], current: Preset | None) -> Task:
    parts = []
    if refs:
        parts.append(untrusted("reference", "\n".join(refs)))
    if current is not None:
        parts.append("지금까지 정한 프리셋:\n" + current.model_dump_json(exclude={"origin"}))
    convo = "\n".join(f"{'사용자' if t.role == 'user' else '도우미'}: {t.text}" for t in history[-MAX_TURNS:])
    parts.append("대화:\n" + convo)
    parts.append("위 대화에 이어 도우미로서 답한다 (reply + preset).")
    return Task("preset", _SYSTEM, "\n\n".join(parts))


def summarize(p: Preset) -> list[str]:
    """저장 전에 보여 줄 요약 (한 줄씩)."""
    s, c, ly = p.sections, p.comments, p.layout
    out = [f"이름: {p.label} ({p.id})" + (f" — {p.description}" if p.description else ""),
           f"길이 {s.target_sec[0]:.0f}~{s.target_sec[1]:.0f}초, 글 머리 {s.hook_sec[0]:.0f}~{s.hook_sec[1]:.0f}초, "
           f"본문 장면 {s.media_sec[0]:.0f}~{s.media_sec[1]:.0f}초"]
    out.append(f"댓글: 화면당 {c.per_screen[0]}~{c.per_screen[1]}개, 최대 {c.max_screens}장·{c.max_total}개"
               if c.enabled else "댓글: 없음")
    out.append({"off": "TTS: 읽지 않음", "auto": "TTS: 장면마다 판단", "on": "TTS: 전부 읽음"}[p.tts])
    out.append(f"자막: {ly.caption}, 한 줄 {p.caption.max_chars}자·{p.caption.max_words}단어"
               + (", 위 제목 바 있음" if ly.title_bar else ""))
    fx = [n for n, on in (("BGM", p.effects.bgm), ("효과음", p.effects.sfx),
                          ("원본 소리 유지", p.effects.original_sound)) if on]
    out.append("효과: " + (", ".join(fx) if fx else "없음"))
    if p.guide.strip():
        out.append("편집 규칙: " + p.guide.strip()[:200])
    return out


def chat(router: Router, history: list[dict[str, Any]], refs: list[Any] | None = None,
         current: dict[str, Any] | None = None) -> dict[str, Any]:
    """한 번 주고받기. 돌려주는 값: reply, preset(없으면 None), summary. 대답을 못 받으면 LlmError."""
    turns = [ChatTurn.model_validate(t) for t in history[-MAX_TURNS:]]
    if not turns or turns[-1].role != "user":
        raise ValueError("마지막 말은 사용자의 말이어야 합니다")
    cur: Preset | None = None
    if current:
        try:
            cur = parse_preset(current, "custom")
        except (ValueError, ValidationError):
            cur = None
    ans, _ = router.ask(chat_task(turns, clean_refs(refs), cur), PresetChatAnswer)
    preset = parse_preset(ans.preset.model_dump(), "custom") if ans.preset else None
    return {"reply": ans.reply, "preset": preset.model_dump() if preset else None,
            "summary": summarize(preset) if preset else []}


def _unique_id(pid: str, taken: set[str]) -> str:
    if pid not in taken:
        return pid
    for n in range(2, 100):
        cand = f"{pid[:28]}-{n}"
        if cand not in taken:
            return cand
    raise ValueError("같은 이름의 유형이 너무 많습니다")


def save_custom_preset(style_dir: str | Path, data: dict[str, Any], refs: list[Any] | None = None) -> Preset:
    """검증 → `<style>/presets/<id>.json` 으로 저장. 내장 id 와 겹치면 새 id. 잘못된 값이면 ValueError."""
    try:
        p = parse_preset(data, "custom")
    except ValidationError as err:
        raise ValueError(f"규칙이 형식에 맞지 않습니다: {str(err)[:300]}") from err
    folder = Path(style_dir) / "presets"
    builtin = {k for k, v in load_presets().items() if v.origin == "builtin"}
    # 이미 있는 내 유형과 같은 id 는 덮어쓰기(고치기)로 본다
    p.id = _unique_id(p.id, builtin)
    p.origin = "custom"
    if refs:
        p.reference.videos = clean_refs(refs)
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / f"{p.id}.json"
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(json.loads(p.model_dump_json(exclude={"origin"})), ensure_ascii=False,
                              indent=2) + "\n", encoding="utf-8")
    tmp.replace(path)
    if parse_preset(json.loads(path.read_text(encoding="utf-8")), "custom").id != p.id:  # 쓴 것 다시 읽어 확인
        raise ValueError("저장한 파일을 다시 읽지 못했습니다")
    return p


def delete_custom_preset(style_dir: str | Path, preset_id: str) -> bool:
    """내가 만든 유형 파일만 지운다 (내장은 지울 수 없음)."""
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,31}", preset_id or ""):
        return False
    path = Path(style_dir) / "presets" / f"{preset_id}.json"
    try:
        path.unlink()
    except OSError:
        return False
    return True
