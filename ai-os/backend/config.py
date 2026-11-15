"""Shared paths and constants for the AI-OS backend."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

HOME = Path.home()
ROOT = Path(os.environ.get("YZKR_HOME", HOME / "YZKR")).resolve()
AIOS = ROOT / "ai-os"
DATA = AIOS / "data"
WEB = AIOS / "web"
SCRIPTS = ROOT / "scripts"
MEMORY_FILE = ROOT / "ai-memory" / "memory.jsonl"
CLAUDE_PROJECTS = HOME / ".claude" / "projects"
CLAUDE_DESKTOP_CFG = HOME / "Library" / "Application Support" / "Claude" / "claude_desktop_config.json"
HOST = "127.0.0.1"
PORT = int(os.environ.get("YZKR_AIOS_PORT", "7777"))

DATA.mkdir(parents=True, exist_ok=True)
(DATA / "chats").mkdir(exist_ok=True)

# scripts/ holds spotlight_engine, yzkr_macros, yzkr_llm — shared with the CLI/Sol path
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))


def theme() -> dict:
    return json.loads((AIOS / "theme.json").read_text())
