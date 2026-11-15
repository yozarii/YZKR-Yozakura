"""Home plane backend: daemon probes and service toggles, both declared as registries."""

from __future__ import annotations

import json
import shlex
import shutil
import subprocess
from collections.abc import Callable
from dataclasses import dataclass

import yzkr_llm

from .config import CLAUDE_DESKTOP_CFG

NP = shutil.which("np") or "/opt/homebrew/bin/np"
BREW = shutil.which("brew") or "/opt/homebrew/bin/brew"


def sh(cmd: list[str], timeout: float = 10) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)


def running(pattern: str, exact: bool = False) -> bool:
    return sh(["pgrep", "-x" if exact else "-f", pattern]).returncode == 0


# ───────────────────────── probes ─────────────────────────
PROBES: dict[str, Callable[[], list[dict]]] = {}


def probe(name: str):
    def deco(fn):
        PROBES[name] = fn
        return fn
    return deco


def card(id: str, label: str, state: str, detail: str = "", group: str = "", **extra) -> dict:
    return {"id": id, "label": label, "state": state, "detail": detail, "group": group, **extra}


@probe("ollama")
def _ollama() -> list[dict]:
    s = yzkr_llm.status()
    if not s["up"]:
        return [card("ollama", "Ollama", "down", s.get("error", "unreachable"), "AI", toggle="ollama")]
    want = {yzkr_llm.ROUTER_MODEL, yzkr_llm.WORKER_MODEL}
    missing = want - set(s["models"])
    detail = f"{len(s['models'])} models · loaded: {', '.join(s['loaded']) or 'none'}"
    return [card("ollama", "Ollama", "warn" if missing else "up", detail + (f" · missing {', '.join(missing)}" if missing else ""),
                 "AI", toggle="ollama", models=s["models"], loaded=s["loaded"])]


def _mcp_pattern(cfg: dict) -> str:
    """Most distinctive argv token of an MCP server (script path, package or repo) for pgrep -f."""
    args = cfg.get("args", [])
    for a in args:
        if a.endswith(".py") or a.startswith("@") or a.startswith("mcp-server"):
            return a
    return " ".join([cfg.get("command", "")] + args)


@probe("mcp")
def _mcp() -> list[dict]:
    try:
        servers = json.loads(CLAUDE_DESKTOP_CFG.read_text()).get("mcpServers", {})
    except (OSError, json.JSONDecodeError) as e:
        return [card("mcp", "MCP config", "down", str(e), "MCP")]
    out = []
    for name, cfg in servers.items():
        pat = _mcp_pattern(cfg)
        up = running(pat)
        out.append(card(f"mcp:{name}", name, "up" if up else "idle", ("running · " if up else "not running · ") + pat, "MCP",
                        hint="started by Claude Desktop on demand"))
    return out


@probe("vault")
def _vault() -> list[dict]:
    r = sh([NP, "whoami", "--format", "json"], timeout=15)
    if r.returncode != 0:
        return [card("np", "NordPass vault", "down", (r.stderr or r.stdout).strip()[:120], "Security")]
    d = json.loads(r.stdout or "{}")
    if not d.get("authenticated"):
        return [card("np", "NordPass vault", "warn", "signed out — run np signin", "Security", toggle="np")]
    unlocked = bool(d.get("vaultUnlocked"))
    return [card("np", "NordPass vault", "up" if unlocked else "warn", ("unlocked" if unlocked else "locked") + f" · {d.get('email', '')}",
                 "Security", toggle="np", on=unlocked)]


@probe("apps")
def _apps() -> list[dict]:
    return [card(f"app:{a}", a, "up" if running(a, exact=True) else "idle", "running" if running(a, exact=True) else "stopped",
                 "Desktop", toggle=f"app:{a}", on=running(a, exact=True)) for a in ("Sol", "Rectangle")]


@probe("spotlight")
def _spotlight() -> list[dict]:
    import spotlight_engine as se
    import time
    s = se.stats()
    age = f"{int((time.time() - s['indexed_at']) / 60)} min ago" if s["indexed_at"] else "never"
    return [card("spotlight", "Spotlight index", "up" if s["total"] else "warn", f"{s['total']} items · indexed {age}",
                 "AI", action="reindex", kinds=s["kinds"])]


def status() -> list[dict]:
    out = []
    for name, fn in PROBES.items():
        try:
            out.extend(fn())
        except Exception as e:
            out.append(card(name, name, "down", f"probe failed: {e}"))
    return out


# ───────────────────────── toggles ─────────────────────────
@dataclass
class Toggle:
    on: list[str] | None
    off: list[str] | None
    note: str = ""


TOGGLES: dict[str, Toggle] = {
    "ollama": Toggle([BREW, "services", "start", "ollama"], [BREW, "services", "stop", "ollama"]),
    "app:Sol": Toggle(["open", "-a", "Sol"], ["osascript", "-e", 'quit app "Sol"']),
    "app:Rectangle": Toggle(["open", "-a", "Rectangle"], ["osascript", "-e", 'quit app "Rectangle"']),
    # unlocking needs the master password interactively → handled in the terminal plane, never here
    "np": Toggle(None, [NP, "lock"], note="unlock in the terminal: np unlock"),
}


def toggle(name: str, on: bool) -> dict:
    t = TOGGLES.get(name)
    if t is None:
        raise KeyError(f"unknown toggle {name}")
    cmd = t.on if on else t.off
    if cmd is None:
        return {"ok": False, "terminal": "np unlock", "note": t.note}
    r = sh(cmd, timeout=30)
    return {"ok": r.returncode == 0, "cmd": shlex.join(cmd), "output": (r.stdout + r.stderr).strip()[-500:]}
