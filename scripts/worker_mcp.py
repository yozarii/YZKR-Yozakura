# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "mcp>=2,<3",
#   "httpx>=0.27",
#   "pydantic>=2",
# ]
# ///
"""worker_mcp — local worker bridge: typed intent routing, small-model helper, tri-engine search."""

import json
import os
import shutil
import subprocess
from typing import Literal

import httpx
from mcp.server import MCPServer
from pydantic import BaseModel, Field, ValidationError

OLLAMA = os.environ.get("OLLAMA_HOST", "http://localhost:11434")
ROUTER_MODEL = "qwen2.5:3b-instruct-q4_K_M"
WORKER_MODEL = "llama3.2:3b"
NP_BIN = shutil.which("np") or "/opt/homebrew/bin/np"

mcp = MCPServer("worker")


# ---------- ollama ----------
def _chat(model: str, messages: list[dict], fmt: dict | None = None, temperature: float = 0.0) -> str:
    body = {"model": model, "messages": messages, "stream": False, "options": {"temperature": temperature}}
    if fmt:
        body["format"] = fmt
    r = httpx.post(f"{OLLAMA}/api/chat", json=body, timeout=120)
    r.raise_for_status()
    return r.json()["message"]["content"]


# ---------- route_intent ----------
Intent = Literal["open", "goto", "search", "macro", "graph_query"]


class RoutedIntent(BaseModel):
    intent: Intent
    target: str = Field("", description="app, path, URL, macro name, or graph entity")
    query: str = Field("", description="free-text search or graph query")
    args: list[str] = Field(default_factory=list)
    confidence: float = Field(ge=0, le=1)


_ROUTER_SYSTEM = (
    "Classify the user's command into exactly one intent: "
    "open (launch an app or file), goto (navigate to a path, URL or plane), "
    "search (look something up), macro (run or record a named command sequence), "
    "graph_query (question about relations between notes, files, memory or chats). "
    "Put app/path/URL/macro name in target; put free text to look up in query; args only for extra flags. "
    "confidence is 0-1. Output JSON only.\n"
    "Examples:\n"
    "'start spotify' -> open, target=Spotify\n"
    "'jump to ~/YZKR/scripts' -> goto, target=~/YZKR/scripts\n"
    "'best pizza in rome' -> search, query='best pizza in rome'\n"
    "'run my backup macro' -> macro, target=backup\n"
    "'which notes link to ollama' -> graph_query, query='notes linking to ollama'"
)


@mcp.tool()
def route_intent(command: str) -> dict:
    """Classify a natural-language command into open|goto|search|macro|graph_query with typed params and confidence."""
    try:
        raw = _chat(
            ROUTER_MODEL,
            [{"role": "system", "content": _ROUTER_SYSTEM}, {"role": "user", "content": command}],
            fmt=RoutedIntent.model_json_schema(),
        )
        return RoutedIntent.model_validate_json(raw).model_dump()
    except (httpx.HTTPError, ValidationError, KeyError) as e:
        return {"intent": "search", "target": "", "query": command, "args": [], "confidence": 0.0, "error": str(e)}


# ---------- quick_worker ----------
_TASKS = {
    "summarize": "Summarize the text in at most 5 concise bullet points.",
    "tag": "Return 3-6 lowercase kebab-case tags for the text, comma-separated, nothing else.",
    "commit": "Write a conventional-commit message (<=72 char subject, optional short body) for this diff. Output only the message.",
    "format": "Clean up and format the text as tidy Markdown without changing meaning.",
}


@mcp.tool()
def quick_worker(task: Literal["summarize", "tag", "commit", "format"], text: str) -> str:
    """Fast local SLM (llama3.2:3b): summarize, tag, commit-message from diff, or format text."""
    try:
        return _chat(WORKER_MODEL, [{"role": "system", "content": _TASKS[task]}, {"role": "user", "content": text}], temperature=0.2).strip()
    except httpx.HTTPError as e:
        return f"ollama error: {e}"


# ---------- tri_search ----------
_KEY_ITEMS = {"you": "You.com", "perplexity": "Perplexity", "firecrawl": "Firecrawl"}
_ENV = {"you": "YDC_API_KEY", "perplexity": "PERPLEXITY_API_KEY", "firecrawl": "FIRECRAWL_API_KEY"}


def _np(args: list[str]) -> str:
    res = subprocess.run([NP_BIN, *args], capture_output=True, text=True, timeout=20)
    if res.returncode != 0:
        raise RuntimeError((res.stderr or res.stdout).strip())
    return res.stdout.strip()


def _key(engine: str) -> str:
    """Resolve an API key: env var first, else NordPass item whose name matches the engine."""
    if v := os.environ.get(_ENV[engine]):
        return v
    items = json.loads(_np(["item", "list", "--search", _KEY_ITEMS[engine], "--limit", "1", "--format", "json"]))
    items = items if isinstance(items, list) else items.get("items", [])
    if not items:
        raise RuntimeError(f"no NordPass item matching '{_KEY_ITEMS[engine]}' (or set {_ENV[engine]})")
    return _np(["item", "get", items[0]["id"], "--field", "password"])


def _you(q: str) -> str:
    r = httpx.get("https://ydc-index.io/v1/search", params={"query": q}, headers={"X-API-Key": _key("you")}, timeout=30)
    r.raise_for_status()
    hits = r.json().get("results", {}).get("web", [])[:6]
    return "\n".join(f"- {h.get('title')} — {h.get('url')}\n  {' '.join(h.get('snippets', []))[:300]}" for h in hits)


def _perplexity(q: str) -> str:
    r = httpx.post(
        "https://api.perplexity.ai/chat/completions",
        headers={"Authorization": f"Bearer {_key('perplexity')}"},
        json={"model": "sonar", "messages": [{"role": "user", "content": q}]},
        timeout=90,
    )
    r.raise_for_status()
    return r.json()["choices"][0]["message"]["content"]


def _firecrawl(q: str) -> str:
    h = {"Authorization": f"Bearer {_key('firecrawl')}"}
    if q.startswith(("http://", "https://")):
        r = httpx.post("https://api.firecrawl.dev/v1/scrape", headers=h, json={"url": q, "formats": ["markdown"]}, timeout=90)
        r.raise_for_status()
        return r.json()["data"]["markdown"][:8000]
    r = httpx.post("https://api.firecrawl.dev/v1/search", headers=h, json={"query": q, "limit": 3, "scrapeOptions": {"formats": ["markdown"]}}, timeout=90)
    r.raise_for_status()
    return "\n\n".join(f"## {d.get('title')}\n{d.get('url')}\n{(d.get('markdown') or '')[:2500]}" for d in r.json().get("data", []))


_ENGINES = {"you": _you, "perplexity": _perplexity, "firecrawl": _firecrawl}
_AUTO = {"fast": "you", "deep": "perplexity", "extract": "firecrawl"}


@mcp.tool()
def tri_search(query: str, mode: Literal["fast", "deep", "extract", "all"] = "fast") -> str:
    """Route a query: fast=You.com snippets, deep=Perplexity synthesis, extract=Firecrawl Markdown (pass a URL to scrape), all=every engine."""
    names = list(_ENGINES) if mode == "all" else [_AUTO[mode]]
    out = []
    for n in names:
        try:
            out.append(f"# {n}\n{_ENGINES[n](query)}")
        except Exception as e:  # keep other engines usable when one key/API fails
            out.append(f"# {n}\nerror: {e}")
    return "\n\n".join(out)


if __name__ == "__main__":
    mcp.run()
