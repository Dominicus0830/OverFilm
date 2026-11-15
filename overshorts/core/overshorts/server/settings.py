"""패널 '설정' 화면 API (M8). 바꾼 값은 config.yaml 에 저장하고, 다시 켜지 않아도 다음 작업부터 쓴다.

- GET  /api/settings → 지금 값, 항목 종류(고를 값·범위), 설정 파일 위치, '다시 묻지 않기' 상태
- POST /api/settings {"values": {"llm": {"filter": "claude_code"}, ...}, "prefs": {...}}
"""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

from fastapi import APIRouter, HTTPException

from ..config import (
    GENERAL,
    SETTINGS,
    apply_settings,
    check_settings,
    crawl_limits,
    save_settings,
    settings_view,
)

# panel_prefs.json 에 남는 '다시 묻지 않기' 류 (설정 화면에서 되돌림)
PREF_KEYS = {"skip_crawl_confirm": bool, "on_limit": str}


def _apply_to_pipeline(p: Any, proxy_changed: bool) -> None:
    """작업 스레드에서: LLM·수집 예산을 새 설정으로 (다음 작업부터)."""
    p._router = None
    guard = getattr(p, "guard", None)
    if guard is not None:
        guard.limits = crawl_limits(p.cfg)
    if proxy_changed and guard is not None:
        from ..crawler.fetcher import Fetcher

        p.fetcher = Fetcher(guard, proxy=(p.cfg.get("crawler") or {}).get("proxy"))


def settings_router(cfg: dict[str, Any], config_path: str | None, data: Path,
                    submit: Callable[[str, Callable[[Any], None]], None]) -> APIRouter:
    router = APIRouter()
    prefs_path = data / "panel_prefs.json"

    def read_prefs() -> dict[str, Any]:
        try:
            got = json.loads(prefs_path.read_text(encoding="utf-8"))
            return got if isinstance(got, dict) else {}
        except (OSError, json.JSONDecodeError):
            return {}

    def write_prefs(prefs: dict[str, Any]) -> None:
        data.mkdir(parents=True, exist_ok=True)
        prefs_path.write_text(json.dumps(prefs, ensure_ascii=False), encoding="utf-8")

    def view() -> dict[str, Any]:
        out = settings_view(cfg)
        prefs = read_prefs()
        if isinstance(prefs.get("show_source"), bool):  # 검수 화면에서 켜고 끈 값이 우선
            out["values"].setdefault("render", {})["show_source"] = prefs["show_source"]
        out["prefs"] = {"skip_crawl_confirm": bool(prefs.get("skip_crawl_confirm")),
                        "on_limit": prefs.get("on_limit") if prefs.get("on_limit") in
                        ("fallback", "wait") else None}
        out["path"] = str(Path(config_path).resolve()) if config_path else None
        out["data_dir"] = str(data.resolve())
        return out

    @router.get("/api/settings")
    def settings_get() -> dict[str, Any]:
        return view()

    @router.post("/api/settings")
    def settings_set(body: dict[str, Any]) -> dict[str, Any]:
        if not isinstance(body, dict):
            raise HTTPException(400, "형식이 맞지 않습니다")
        raw = body.get("values") if isinstance(body.get("values"), dict) else {}
        try:
            changes = check_settings(raw)
        except ValueError as err:
            raise HTTPException(400, str(err)) from err
        before = {k: (cfg if k[0] == GENERAL else cfg.get(k[0]) or {}).get(k[1]) for k in changes}
        if changes:
            if not config_path:
                raise HTTPException(400, "설정 파일 없이 켠 패널이라 저장할 수 없습니다")
            try:
                save_settings(config_path, changes)
            except OSError as err:
                raise HTTPException(500, f"설정 파일을 저장하지 못했습니다: {err}") from err
            # data_dir 처럼 다시 켜야 하는 값은 파일에만 (지금 도는 패널은 옛 폴더를 계속 씀)
            apply_settings(cfg, {k: v for k, v in changes.items() if not SETTINGS[k].restart})

        prefs = read_prefs()
        new_prefs = dict(prefs)
        asked = body.get("prefs") if isinstance(body.get("prefs"), dict) else {}
        if asked.get("skip_crawl_confirm") is False:
            new_prefs.pop("skip_crawl_confirm", None)
        if asked.get("on_limit") is None and "on_limit" in asked:
            new_prefs.pop("on_limit", None)
        if ("llm", "on_limit") in changes:  # 설정에서 고르면 확인창의 '다시 묻지 않기'보다 우선
            new_prefs.pop("on_limit", None)
        show = changes.get(("render", "show_source"))
        was = prefs.get("show_source")
        was = was if isinstance(was, bool) else before.get(("render", "show_source")) is not False
        if isinstance(show, bool) and was is not show:
            new_prefs["show_source"] = show
            # 검수 화면의 출처 표시와 같은 값: 이미 만든 미리보기도 맞춘다
            submit("출처 표시 " + ("켬" if show else "끔"), lambda p: p.set_show_source(show))
        if new_prefs != prefs:
            try:
                write_prefs(new_prefs)
            except OSError as err:
                raise HTTPException(500, f"패널 설정을 저장하지 못했습니다: {err}") from err

        changed = [k for k in changes if before.get(k) != changes[k]]
        if changed:
            proxy = ("crawler", "proxy") in changed
            submit("설정 적용", lambda p: _apply_to_pipeline(p, proxy))
        restart = [f"{s}.{k}" for (s, k) in changed if SETTINGS[(s, k)].restart]
        return {"ok": True, "changed": [f"{s}.{k}" for (s, k) in changed], "restart": restart,
                **view()}

    return router
