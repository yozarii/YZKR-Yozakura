"""yzkr_macros — record, store and run named macros under ~/YZKR/scripts/macros/.

A macro is a zsh script whose header carries its typed step list (`# yzkr-steps: [...]`), so it stays
human-editable shell while remaining machine-readable. Step types are a registry, not a branch tree:
adding a type = adding one `StepType` entry.
"""

from __future__ import annotations

import json
import os
import re
import shlex
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from yzkr_llm import chat_json

YZKR = Path(os.environ.get("YZKR_HOME", Path.home() / "YZKR"))
MACRO_DIR = YZKR / "scripts" / "macros"
SOL_SCRIPTS = Path.home() / ".config" / "sol" / "scripts"
HEADER_RE = re.compile(r"^#\s*(name|icon|description|yzkr-steps):\s*(.*)$", re.M)


def _num(v: str) -> str:
    m = re.search(r"\d+(?:\.\d+)?", v)
    if not m:
        raise ValueError(f"no number in {v!r}")
    return m.group()


@dataclass(frozen=True)
class StepType:
    help: str
    render: Callable[[str], str]
    normalize: Callable[[str], str] = str.strip  # raises ValueError → step dropped


q = shlex.quote
STEP_TYPES: dict[str, StepType] = {
    "shell": StepType("a zsh command line", lambda v: v),
    "open_app": StepType("launch a macOS app by name", lambda v: f"open -a {q(v)}"),
    "open_url": StepType("open a URL in the default browser", lambda v: f"open {q(v)}"),
    "open_path": StepType("open a file or folder (~ allowed)", lambda v: f"open {q(os.path.expanduser(v))}"),
    "quit_app": StepType("quit a running app by name", lambda v: f"osascript -e {q(f'quit app {json.dumps(v)}')}"),
    "sleep": StepType("wait N seconds (value is just the number)", lambda v: f"sleep {float(v):g}", _num),
    "notify": StepType("show a macOS notification", lambda v: f"osascript -e {q(f'display notification {json.dumps(v)} with title \"YZKR\"')}"),
    "yzkr": StepType("run a natural-language YZKR spotlight query", lambda v: f"/opt/homebrew/bin/uv run -q {q(str(YZKR / 'scripts' / 'spotlight_engine.py'))} run {q(v)}"),
}


@dataclass
class Macro:
    slug: str
    name: str
    icon: str
    description: str
    steps: list[dict]

    @property
    def path(self) -> Path:
        return MACRO_DIR / f"{self.slug}.zsh"

    def render(self) -> str:
        body = "\n".join(STEP_TYPES[s["type"]].render(s["value"]) for s in self.steps)
        return (
            "#!/bin/zsh\n"
            f"# name: {self.name}\n# icon: {self.icon}\n# description: {self.description}\n"
            f"# yzkr-steps: {json.dumps(self.steps, ensure_ascii=False)}\n"
            "set -euo pipefail\n"
            'export PATH="/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:$PATH"  # launchers run with a bare PATH\n\n'
            f"{body}\n"
        )

    def to_dict(self) -> dict:
        return {"slug": self.slug, "name": self.name, "icon": self.icon, "description": self.description,
                "steps": self.steps, "path": str(self.path)}


def slugify(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-") or "macro"


def load(path: Path) -> Macro:
    meta = dict(HEADER_RE.findall(path.read_text(errors="ignore")))
    try:
        steps = json.loads(meta.get("yzkr-steps", "[]"))
    except json.JSONDecodeError:
        steps = []
    return Macro(path.stem, meta.get("name", path.stem), meta.get("icon", "⚡"), meta.get("description", ""), steps)


def list_macros() -> list[Macro]:
    MACRO_DIR.mkdir(parents=True, exist_ok=True)
    return [load(p) for p in sorted(MACRO_DIR.glob("*.zsh"))]


def get(slug_or_name: str) -> Macro:
    slug = slugify(slug_or_name)
    path = MACRO_DIR / f"{slug}.zsh"
    if not path.exists():
        raise FileNotFoundError(f"no macro '{slug_or_name}'")
    return load(path)


def save(m: Macro) -> Macro:
    unknown = {s["type"] for s in m.steps} - STEP_TYPES.keys()
    if unknown:
        raise ValueError(f"unknown step types: {sorted(unknown)}")
    MACRO_DIR.mkdir(parents=True, exist_ok=True)
    m.path.write_text(m.render())
    m.path.chmod(0o755)
    sol_sync()
    return m


def delete(slug: str) -> None:
    get(slug).path.unlink()
    sol_sync()


def run(slug: str, wait: bool = False) -> dict:
    m = get(slug)
    proc = subprocess.Popen(["/bin/zsh", str(m.path)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                            start_new_session=True)
    if not wait:
        return {"ok": True, "macro": m.slug, "pid": proc.pid}
    out, _ = proc.communicate(timeout=300)
    return {"ok": proc.returncode == 0, "macro": m.slug, "code": proc.returncode, "output": out[-4000:]}


# ---------- recording ----------
_RECORD_SYSTEM = (
    "You turn a request into a named macOS automation macro made of ordered typed steps.\n"
    "Step types:\n" + "\n".join(f"- {k}: {v.help}" for k, v in STEP_TYPES.items()) + "\n"
    "Prefer open_app/open_url/open_path/quit_app over shell when they fit. Keep shell commands safe and literal; "
    "never invent destructive commands (rm -rf, sudo) unless explicitly requested. "
    "name: 2-4 word title. icon: one emoji. description: one short sentence."
)


def _record_schema() -> dict:
    return {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "icon": {"type": "string"},
            "description": {"type": "string"},
            "steps": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "properties": {"type": {"enum": list(STEP_TYPES)}, "value": {"type": "string"}},
                    "required": ["type", "value"],
                },
            },
        },
        "required": ["name", "icon", "description", "steps"],
    }


def record_from_text(request: str, name: str | None = None, dry_run: bool = False) -> Macro:
    """Natural language → typed steps (qwen, schema-constrained) → saved macro."""
    out = chat_json(_RECORD_SYSTEM, request, _record_schema(), timeout=60, max_tokens=800)
    steps = []
    for s in out["steps"]:
        try:
            steps.append({"type": s["type"], "value": STEP_TYPES[s["type"]].normalize(s["value"])})
        except (KeyError, ValueError):
            continue  # drop malformed steps rather than emit a broken script
    if not steps:
        raise ValueError("model produced no usable steps")
    title = name or out["name"].strip() or "Macro"
    m = Macro(slugify(title), title, (out.get("icon") or "⚡").strip()[:2], out.get("description", "").strip(), steps)
    return m if dry_run else save(m)


def _history(n: int) -> list[str]:
    """Last n commands from zsh EXTENDED_HISTORY (`: ts:dur;cmd`, `\\`-continued lines joined)."""
    raw = (Path.home() / ".zsh_history").read_bytes().decode("utf-8", "ignore")
    cmds, buf = [], ""
    for line in raw.splitlines():
        buf = f"{buf}\n{line}" if buf else line
        if buf.endswith("\\"):
            buf = buf[:-1]
            continue
        cmds.append(re.sub(r"^: \d+:\d+;", "", buf).strip())
        buf = ""
    skip = ("yzkr macro", "spotlight_engine.py macro", "history")
    return [c for c in cmds if c and not c.startswith(skip)][-n:]


def record_from_history(n: int, name: str, description: str = "") -> Macro:
    steps = [{"type": "shell", "value": c} for c in _history(n)]
    return save(Macro(slugify(name), name, "⏺", description or f"Recorded from last {n} shell commands", steps))


# ---------- Sol integration ----------
def sol_sync() -> list[str]:
    """Mirror macros into Sol's script folder (bash-executed wrappers; Sol watches the folder)."""
    SOL_SCRIPTS.mkdir(parents=True, exist_ok=True)
    wanted = {}
    for m in list_macros():
        wanted[f"yzkr-macro-{m.slug}.sh"] = (
            f"#!/bin/bash\n# name: {m.name}\n# icon: {m.icon}\n# Generated by yzkr_macros.sol_sync — edit {m.path}\n"
            f"exec /bin/zsh {q(str(m.path))}\n"
        )
    for stale in SOL_SCRIPTS.glob("yzkr-macro-*.sh"):
        if stale.name not in wanted:
            stale.unlink()
    for fname, text in wanted.items():
        p = SOL_SCRIPTS / fname
        if not p.exists() or p.read_text() != text:
            p.write_text(text)
            p.chmod(0o755)
    return sorted(wanted)
