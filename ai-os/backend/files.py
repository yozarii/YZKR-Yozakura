"""Editor plane backend: sandboxed listing/reading/writing under ~/YZKR."""

from __future__ import annotations

import os
import tempfile
from pathlib import Path

from .config import ROOT

HIDDEN = {".git", "node_modules", "__pycache__", ".DS_Store", ".build", ".venv", "data"}
MAX_READ = 2_000_000


class SandboxError(PermissionError):
    pass


def resolve(path: str | None) -> Path:
    """Resolve a user path (absolute, ~, or ROOT-relative) and refuse anything outside ROOT."""
    p = Path(os.path.expanduser(path or str(ROOT)))
    p = (p if p.is_absolute() else ROOT / p).resolve()
    if p != ROOT and ROOT not in p.parents:
        raise SandboxError(f"outside {ROOT}: {p}")
    return p


def listdir(path: str | None) -> dict:
    d = resolve(path)
    entries = []
    for c in sorted(d.iterdir(), key=lambda c: (not c.is_dir(), c.name.lower())):
        if c.name in HIDDEN:
            continue
        st = c.stat()
        entries.append({"name": c.name, "path": str(c), "dir": c.is_dir(), "size": st.st_size, "mtime": st.st_mtime})
    return {"path": str(d), "rel": str(d.relative_to(ROOT)) if d != ROOT else "", "entries": entries}


def read(path: str) -> dict:
    p = resolve(path)
    if p.stat().st_size > MAX_READ:
        raise ValueError("file too large for the editor")
    raw = p.read_bytes()
    if b"\0" in raw[:4096]:
        raise ValueError("binary file")
    return {"path": str(p), "rel": str(p.relative_to(ROOT)), "text": raw.decode("utf-8", "replace"), "mtime": p.stat().st_mtime}


def write(path: str, text: str, expect_mtime: float | None = None) -> dict:
    """Atomic write; refuses to clobber a file changed on disk since it was opened."""
    p = resolve(path)
    if expect_mtime is not None and p.exists() and abs(p.stat().st_mtime - expect_mtime) > 1e-3:
        raise FileExistsError("file changed on disk since it was opened — reload first")
    p.parent.mkdir(parents=True, exist_ok=True)
    mode = p.stat().st_mode if p.exists() else 0o644
    fd, tmp = tempfile.mkstemp(dir=p.parent, prefix=f".{p.name}.")
    with os.fdopen(fd, "w") as f:
        f.write(text)
    os.chmod(tmp, mode & 0o7777)
    os.replace(tmp, p)
    return {"path": str(p), "mtime": p.stat().st_mtime}
