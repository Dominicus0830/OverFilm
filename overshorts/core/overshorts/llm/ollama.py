"""Ollama HTTP API (로컬 GPU LLM). `format` 에 JSON 스키마를 줘서 형식을 맞춘다.

라데온: Ollama 는 Windows 에서 AMD GPU(ROCm)를 지원한다. `keep_alive` 로 다 쓴 모델을 내려
다음 단계 모델(TTS 등)에 VRAM 을 넘긴다 (VRAM 스케줄러, 6.3).
"""

from __future__ import annotations

import json
import urllib.error
import urllib.request
from typing import Any

from . import LlmError, Task, extract_json


class OllamaClient:
    def __init__(self, model: str = "qwen3:8b", host: str = "http://127.0.0.1:11434",
                 timeout: float = 300.0, keep_alive: str | int = "5m"):
        self.model = model
        self.host = host.rstrip("/")
        self.timeout = max(5.0, min(float(timeout), 1800.0))
        self.keep_alive = keep_alive
        self.name = f"ollama:{model}"

    def _post(self, path: str, body: dict[str, Any]) -> dict[str, Any]:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        req = urllib.request.Request(self.host + path, data=data,
                                     headers={"Content-Type": "application/json"})
        # 로컬 서버이므로 프록시를 거치지 않는다
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            with opener.open(req, timeout=self.timeout) as r:
                raw = r.read(20 * 1024 * 1024)
        except urllib.error.HTTPError as err:
            raise LlmError(f"Ollama 오류 HTTP {err.code} (모델 '{self.model}' 이 설치됐는지 확인: "
                           f"ollama pull {self.model})") from err
        except (urllib.error.URLError, OSError) as err:
            raise LlmError(f"Ollama 에 연결하지 못했습니다 ({self.host}). Ollama 를 켜 주세요.") from err
        try:
            out = json.loads(raw.decode("utf-8", errors="replace"))
        except json.JSONDecodeError as err:
            raise LlmError("Ollama 대답을 읽지 못했습니다") from err
        return out if isinstance(out, dict) else {}

    def complete(self, task: Task, schema: dict[str, Any]) -> dict[str, Any]:
        out = self._post("/api/chat", {
            "model": self.model,
            "messages": [{"role": "system", "content": task.system},
                         {"role": "user", "content": task.prompt}],
            "format": schema,
            "stream": False,
            "think": False,
            "keep_alive": self.keep_alive,
            "options": {"temperature": 0.2},
        })
        content = ((out.get("message") or {}).get("content")) or ""
        v = extract_json(content)
        if not isinstance(v, dict):
            raise LlmError(f"Ollama 대답을 JSON 으로 읽지 못했습니다: {content[:200]}")
        return v

    def unload(self) -> None:
        """모델을 VRAM 에서 내린다 (keep_alive 0)."""
        try:
            self._post("/api/generate", {"model": self.model, "keep_alive": 0})
        except LlmError:
            pass

    def installed_models(self) -> list[str]:
        """설정 화면 드롭다운용: 설치된 모델만."""
        try:
            req = urllib.request.Request(self.host + "/api/tags")
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            with opener.open(req, timeout=5) as r:
                data = json.loads(r.read(5 * 1024 * 1024).decode("utf-8", errors="replace"))
        except (urllib.error.URLError, OSError, json.JSONDecodeError):
            return []
        models = data.get("models") if isinstance(data, dict) else None
        return [str(m.get("name")) for m in models or [] if isinstance(m, dict) and m.get("name")]
