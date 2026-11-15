"""yzkr_llm — minimal Ollama client shared by spotlight_engine, macros and the AI-OS server.

Imported as a sibling module (not run directly); callers declare `httpx` in their PEP 723 header.
"""

from __future__ import annotations

import json
import os
from collections.abc import Iterator

import httpx

OLLAMA = os.environ.get("OLLAMA_HOST", "http://localhost:11434")
ROUTER_MODEL = os.environ.get("YZKR_ROUTER_MODEL", "qwen2.5:3b-instruct-q4_K_M")
WORKER_MODEL = os.environ.get("YZKR_WORKER_MODEL", "llama3.2:3b")


class LLMUnavailable(RuntimeError):
    """Ollama unreachable, model missing, or output not valid JSON."""


def chat_json(system: str, user: str, schema: dict, model: str = ROUTER_MODEL, timeout: float = 20,
              max_tokens: int = 256) -> dict:
    """One-shot structured completion constrained by a JSON schema (Ollama `format`)."""
    body = {
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "format": schema,
        "stream": False,
        "keep_alive": "30m",
        # cap output: small models under grammar constraints can otherwise pad whitespace until timeout
        "options": {"temperature": 0, "num_predict": max_tokens},
    }
    try:
        r = httpx.post(f"{OLLAMA}/api/chat", json=body, timeout=timeout)
        r.raise_for_status()
        return json.loads(r.json()["message"]["content"])
    except (httpx.HTTPError, KeyError, json.JSONDecodeError) as e:
        raise LLMUnavailable(str(e)) from e


def chat_stream(messages: list[dict], model: str = WORKER_MODEL, timeout: float = 300) -> Iterator[str]:
    """Stream assistant text deltas from Ollama /api/chat."""
    body = {"model": model, "messages": messages, "stream": True, "keep_alive": "30m"}
    try:
        with httpx.stream("POST", f"{OLLAMA}/api/chat", json=body, timeout=timeout) as r:
            r.raise_for_status()
            for line in r.iter_lines():
                if not line:
                    continue
                chunk = json.loads(line)
                if delta := chunk.get("message", {}).get("content"):
                    yield delta
                if chunk.get("done"):
                    return
    except httpx.HTTPError as e:
        raise LLMUnavailable(str(e)) from e


def status(timeout: float = 2) -> dict:
    """Installed + loaded models, or {'up': False}."""
    try:
        tags = httpx.get(f"{OLLAMA}/api/tags", timeout=timeout).json().get("models", [])
        ps = httpx.get(f"{OLLAMA}/api/ps", timeout=timeout).json().get("models", [])
    except httpx.HTTPError as e:
        return {"up": False, "error": str(e)}
    return {
        "up": True,
        "endpoint": OLLAMA,
        "models": [m["name"] for m in tags],
        "loaded": [m["name"] for m in ps],
    }
