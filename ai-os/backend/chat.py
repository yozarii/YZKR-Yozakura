"""Chat plane backend: one conversation surface routed across Claude, local Ollama workers and the terminal.

Providers are a registry keyed by id (`claude:opus`, `ollama:llama3.2:3b`, …). `auto` asks the local router model
where a message belongs: `local` (quick/simple), `claude` (reasoning, code, long-form), or `terminal` (the user
wants a shell command — we propose it, the user runs it; nothing executes implicitly).
"""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import subprocess
import threading
import time
import uuid
from collections.abc import AsyncIterator
from dataclasses import dataclass
from pathlib import Path

import yzkr_llm

from .config import DATA

CHATS = DATA / "chats"
SYSTEM = (
    "You are the assistant inside YZKR AI-OS, a single-window control hub on the user's Mac (files in ~/YZKR, "
    "local Ollama models, MCP servers, a NordPass vault, Sol launcher). Be concise and practical. When a shell command "
    "helps, put it in a ```zsh block so the user can send it to the terminal."
)
CLAUDE_MODELS = {"opus": "claude-opus-5-5", "sonnet": "claude-sonnet-5-5", "haiku": "claude-haiku-5-5"}
# Server-side refusal fallback: Opus/Sonnet support `fallbacks: "default"`; Haiku has no server-side fallback.
FALLBACK_BETA = "server-side-fallback-2026-07-01"
FALLBACK_MODELS = {"claude-opus-5-5", "claude-sonnet-5-5"}


# ───────────────────────── history ─────────────────────────
def history(chat_id: str) -> list[dict]:
    p = CHATS / f"{chat_id}.jsonl"
    return [json.loads(l) for l in p.read_text().splitlines() if l.strip()] if p.exists() else []


def append(chat_id: str, **msg) -> None:
    with (CHATS / f"{chat_id}.jsonl").open("a") as f:
        f.write(json.dumps({**msg, "ts": time.time()}, ensure_ascii=False) + "\n")


def list_chats() -> list[dict]:
    out = []
    for p in sorted(CHATS.glob("*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True)[:50]:
        msgs = history(p.stem)
        first = next((m["content"] for m in msgs if m["role"] == "user"), "")
        out.append({"id": p.stem, "title": " ".join(first.split())[:60] or p.stem, "turns": len(msgs), "mtime": p.stat().st_mtime})
    return out


def new_id() -> str:
    return time.strftime("%Y%m%d-%H%M%S-") + uuid.uuid4().hex[:4]


# ───────────────────────── providers ─────────────────────────
@dataclass
class Provider:
    id: str
    label: str
    model: str
    kind: str  # "claude" | "ollama"


def _anthropic_key() -> str | None:
    """SDK resolves env/`ant` profiles itself; NordPass item "Anthropic" is the YZKR fallback."""
    if os.environ.get("ANTHROPIC_API_KEY") or os.environ.get("ANTHROPIC_AUTH_TOKEN"):
        return None
    np = shutil.which("np") or "/opt/homebrew/bin/np"
    try:
        items = json.loads(subprocess.run([np, "item", "list", "--search", "Anthropic", "--limit", "1", "--format", "json"],
                                          capture_output=True, text=True, timeout=15).stdout or "[]")
        items = items if isinstance(items, list) else items.get("items", [])
        if items:
            return subprocess.run([np, "item", "get", items[0]["id"], "--field", "password"], capture_output=True,
                                  text=True, timeout=15).stdout.strip() or None
    except (OSError, json.JSONDecodeError, subprocess.SubprocessError, KeyError):
        pass
    return None


def claude_configured() -> bool:
    return bool(os.environ.get("ANTHROPIC_API_KEY") or os.environ.get("ANTHROPIC_AUTH_TOKEN")
                or (Path.home() / ".config" / "anthropic").exists())


def providers() -> list[Provider]:
    out = [Provider(f"claude:{k}", f"Claude {k.title()}", m, "claude") for k, m in CLAUDE_MODELS.items()]
    st = yzkr_llm.status()
    out += [Provider(f"ollama:{m}", m, m, "ollama") for m in st.get("models", [])]
    return out


def _provider(pid: str) -> Provider:
    for p in providers():
        if p.id == pid:
            return p
    if pid.startswith("ollama:"):
        return Provider(pid, pid[7:], pid[7:], "ollama")
    raise KeyError(f"unknown provider {pid}")


async def _stream_ollama(model: str, messages: list[dict]) -> AsyncIterator[str]:
    q: asyncio.Queue = asyncio.Queue()
    loop = asyncio.get_running_loop()
    done = object()

    def pump():
        try:
            for delta in yzkr_llm.chat_stream([{"role": "system", "content": SYSTEM}, *messages], model=model):
                loop.call_soon_threadsafe(q.put_nowait, delta)
        except Exception as e:
            loop.call_soon_threadsafe(q.put_nowait, e)
        loop.call_soon_threadsafe(q.put_nowait, done)

    threading.Thread(target=pump, daemon=True).start()
    while (item := await q.get()) is not done:
        if isinstance(item, Exception):
            raise item
        yield item


_client = None


async def _stream_claude(model: str, messages: list[dict]) -> AsyncIterator[str]:
    global _client
    import anthropic

    if _client is None:
        key = await asyncio.to_thread(_anthropic_key)
        _client = anthropic.AsyncAnthropic(api_key=key) if key else anthropic.AsyncAnthropic()
    extra = {"betas": [FALLBACK_BETA], "fallbacks": "default"} if model in FALLBACK_MODELS else {}
    try:
        async with _client.beta.messages.stream(
            model=model,
            max_tokens=64000,
            system=SYSTEM,
            messages=messages,
            output_config={"effort": "low"},  # chat panel: latency over depth; raise for heavy tasks
            **extra,
        ) as stream:
            async for text in stream.text_stream:
                yield text
            final = await stream.get_final_message()
        if final.stop_reason == "refusal":
            yield "\n\n_(Claude declined this request.)_"
        elif final.stop_reason == "max_tokens":
            yield "\n\n_(response truncated at max_tokens)_"
    except anthropic.AuthenticationError:
        raise RuntimeError("Claude auth failed — set ANTHROPIC_API_KEY, run `ant auth login`, or add a NordPass item named 'Anthropic'")
    except anthropic.RateLimitError as e:
        raise RuntimeError(f"Claude rate limited (retry after {e.response.headers.get('retry-after', '?')}s)")
    except anthropic.APIStatusError as e:
        raise RuntimeError(f"Claude API error {e.status_code}: {e.message}")
    except anthropic.APIConnectionError:
        raise RuntimeError("Claude unreachable — check network")


STREAMERS = {"claude": _stream_claude, "ollama": _stream_ollama}


# ───────────────────────── auto routing ─────────────────────────
_ROUTE_SYSTEM = (
    "Route a chat message to one destination.\n"
    "local: greetings, quick facts, short rewrites, summaries, simple formatting, brief tips.\n"
    "claude: reasoning, coding, debugging, planning, design, long or nuanced writing.\n"
    "terminal: ONLY an imperative request to perform an action on this Mac right now (list/move/find files, "
    "run git/brew commands, kill a process). Questions or advice ABOUT tools are never terminal.\n"
    "Examples: 'hey' → local. 'tips for naming git branches' → local. 'explain git rebase vs merge' → claude. "
    "'write a python script that dedupes photos' → claude. 'show me the git log' → terminal. "
    "'find all pdfs in downloads' → terminal. 'what does brew services do' → local."
)
_ROUTE_SCHEMA = {"type": "object", "properties": {"route": {"enum": ["local", "claude", "terminal"]}}, "required": ["route"]}


def route(text: str) -> str:
    try:
        return yzkr_llm.chat_json(_ROUTE_SYSTEM, text, _ROUTE_SCHEMA, max_tokens=24)["route"]
    except (yzkr_llm.LLMUnavailable, KeyError):
        return "claude" if claude_configured() else "local"


def propose_command(text: str) -> str:
    out = yzkr_llm.chat_json(
        "Translate the request into ONE safe macOS zsh command line. Never use sudo or rm -rf. Return JSON.",
        text, {"type": "object", "properties": {"cmd": {"type": "string", "maxLength": 200}}, "required": ["cmd"]},
        model=yzkr_llm.ROUTER_MODEL, max_tokens=90)
    return out["cmd"].strip()


async def respond(chat_id: str, text: str, provider_id: str = "auto") -> AsyncIterator[dict]:
    """Yield ndjson events: route → delta* → done (or command / error)."""
    append(chat_id, role="user", content=text)
    if provider_id == "auto":
        dest = await asyncio.to_thread(route, text)
        if dest == "terminal":
            try:
                cmd = await asyncio.to_thread(propose_command, text)
                yield {"t": "route", "provider": "terminal", "label": "terminal"}
                yield {"t": "command", "cmd": cmd}
                append(chat_id, role="assistant", content=f"```zsh\n{cmd}\n```", model="router")
                yield {"t": "done"}
                return
            except (yzkr_llm.LLMUnavailable, KeyError):
                dest = "local"
        provider_id = "claude:opus" if dest == "claude" else f"ollama:{yzkr_llm.WORKER_MODEL}"
        if dest == "claude" and not claude_configured() and not await asyncio.to_thread(_anthropic_key):
            provider_id = f"ollama:{yzkr_llm.WORKER_MODEL}"
    p = _provider(provider_id)
    yield {"t": "route", "provider": p.id, "label": p.label, "model": p.model}
    msgs = [{"role": m["role"], "content": m["content"]} for m in history(chat_id) if m["role"] in ("user", "assistant")]
    reply = []
    try:
        async for delta in STREAMERS[p.kind](p.model, msgs[-40:]):
            reply.append(delta)
            yield {"t": "delta", "text": delta}
    except Exception as e:
        yield {"t": "error", "msg": str(e)}
    if reply:
        append(chat_id, role="assistant", content="".join(reply), model=p.model)
    yield {"t": "done"}
