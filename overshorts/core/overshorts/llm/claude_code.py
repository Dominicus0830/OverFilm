"""로컬 `claude -p` 로 묻는다 (사용자 구독). OverFilm 의 Claude 검토(overfilm-review)와 같은 방식.

- 프롬프트는 stdin 으로 (한국어·길이 제한 없음), 대답은 `--output-format json` + `--json-schema`.
- 빈 임시 폴더에서 실행하고 도구를 끈다(`--tools ""`): 프로젝트 파일에 손대지 못한다.
- ANTHROPIC_API_KEY 는 지워서 구독으로만 돈다.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any

from . import LimitReached, LlmError, Task, extract_json

_LOGIN = ("not logged in", "/login", "invalid api key", "authentication", "unauthorized", "oauth")
_LIMIT = ("usage limit", "limit reached", "rate limit", "rate_limit", "429", "overloaded", "quota")


def classify(text: str) -> LlmError:
    t = (text or "").lower()
    if any(k in t for k in _LOGIN):
        return LlmError("Claude Code 에 로그인되어 있지 않습니다 (터미널에서 claude 실행 후 /login)")
    if any(k in t for k in _LIMIT):
        return LimitReached(f"Claude 사용량 한도: {text.strip()[:200]}")
    return LlmError(f"Claude Code 실패: {text.strip()[:300]}")


def read_answer(stdout: str, stderr: str) -> dict[str, Any]:
    v = extract_json(stdout)
    if not isinstance(v, dict):
        raise classify(stderr or stdout or "출력 없음")
    if v.get("is_error") or (v.get("subtype") not in (None, "success")):
        raise classify(f"{v.get('result', '')} {v.get('subtype', '')} {stderr}")
    out = v.get("structured_output")
    if isinstance(out, dict):
        return out
    if "type" not in v and "result" not in v:
        return v
    inner = extract_json(str(v.get("result") or ""))
    if not isinstance(inner, dict):
        raise LlmError(f"대답을 JSON 으로 읽지 못했습니다: {str(v.get('result'))[:200]}")
    return inner


class ClaudeCodeClient:
    name = "claude_code"

    def __init__(self, exe: str | None = None, model: str = "", timeout: float = 300.0):
        self.exe = exe or shutil.which("claude") or "claude"
        self.model = model
        self.timeout = max(10.0, min(float(timeout), 1800.0))
        self.workdir = Path(tempfile.gettempdir()) / "overshorts-claude"

    def args(self, task: Task, schema: dict[str, Any]) -> list[str]:
        a = [self.exe, "-p", "--output-format", "json", "--no-session-persistence",
             "--strict-mcp-config", "--json-schema", json.dumps(schema, ensure_ascii=False),
             "--system-prompt", task.system, "--tools", ""]
        if self.model:
            a += ["--model", self.model]
        return a

    def complete(self, task: Task, schema: dict[str, Any]) -> dict[str, Any]:
        env = {k: v for k, v in os.environ.items()
               if k not in ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN")}
        self.workdir.mkdir(parents=True, exist_ok=True)
        flags = 0x0800_0000 if os.name == "nt" else 0  # CREATE_NO_WINDOW
        try:
            proc = subprocess.run(
                self.args(task, schema), input=task.prompt, capture_output=True, text=True,
                encoding="utf-8", errors="replace", timeout=self.timeout, cwd=self.workdir,
                env=env, creationflags=flags,
            )
        except FileNotFoundError as err:
            raise LlmError("Claude Code 가 설치되어 있지 않습니다") from err
        except subprocess.TimeoutExpired as err:
            raise LlmError(f"Claude Code 가 {int(self.timeout)}초 안에 답하지 않았습니다") from err
        except OSError as err:
            raise LlmError(f"Claude Code 를 실행하지 못했습니다: {err}") from err
        return read_answer(proc.stdout or "", proc.stderr or "")
