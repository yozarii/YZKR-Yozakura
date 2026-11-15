"""Graph plane backend: three linked graphs sharing node ids so they cross-reference.

  memory      memory.jsonl entities/relations + Markdown [[wikilinks]] / relative links / entity mentions
  filesystem  ~/YZKR directory tree + git topology (branches, recent commits → files touched)
  chat        Claude Code session logs + AI-OS chats → models used and files touched

Files are always `file:<abs path>`, so a note linked in memory, changed in a commit, and edited in a chat is one node.
"""

from __future__ import annotations

import json
import re
import subprocess
import time
from collections.abc import Callable
from pathlib import Path

from .config import CLAUDE_PROJECTS, DATA, MEMORY_FILE, ROOT

SKIP = {".git", "node_modules", "__pycache__", ".backup", ".build", "vendor", "data", ".venv", ".DS_Store"}
MAX_FS_NODES = 450
MAX_SESSIONS = 40


class Graph:
    def __init__(self) -> None:
        self.nodes: dict[str, dict] = {}
        self.edges: dict[tuple[str, str, str], dict] = {}

    def node(self, id: str, label: str, layer: str, type: str, **meta) -> str:
        if id not in self.nodes:
            self.nodes[id] = {"id": id, "label": label, "layer": layer, "type": type, **meta}
        return id

    def edge(self, a: str, b: str, layer: str, kind: str) -> None:
        if a != b and a in self.nodes and b in self.nodes:
            self.edges.setdefault((a, b, kind), {"source": a, "target": b, "layer": layer, "kind": kind})

    def file(self, path: Path, layer: str) -> str:
        rel = str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else str(path)
        return self.node(f"file:{path}", path.name + ("/" if path.is_dir() else ""), layer,
                         "dir" if path.is_dir() else "file", path=str(path), rel=rel)


LAYERS: dict[str, Callable[[Graph], None]] = {}


def layer(name: str):
    def deco(fn: Callable[[Graph], None]):
        LAYERS[name] = fn
        return fn
    return deco


# ───────────────────────── filesystem + git ─────────────────────────
@layer("filesystem")
def _filesystem(g: Graph) -> None:
    root = g.node(f"file:{ROOT}", "YZKR", "filesystem", "dir", path=str(ROOT), rel="")
    queue = [ROOT]
    while queue and len(g.nodes) < MAX_FS_NODES:
        d = queue.pop(0)
        try:
            children = sorted(d.iterdir())
        except OSError:
            continue
        for c in children:
            if c.name in SKIP or c.name.startswith(".") and c.name != ".claude":
                continue
            nid = g.file(c, "filesystem")
            g.edge(f"file:{d}" if d != ROOT else root, nid, "filesystem", "contains")
            if c.is_dir():
                queue.append(c)

    def git(*a: str) -> str:
        return subprocess.run(["git", "-C", str(ROOT), *a], capture_output=True, text=True, timeout=10).stdout
    head = git("branch", "--show-current").strip()
    for br in git("branch", "--format=%(refname:short)").split():
        g.node(f"git:branch:{br}", f"⎇ {br}", "filesystem", "branch", current=br == head)
        g.edge(root, f"git:branch:{br}", "filesystem", "branch")
    prev = None
    log = git("log", "-30", "--name-only", "--format=\x1e%h\x1f%s\x1f%ar")
    for block in log.split("\x1e")[1:]:
        header, *files = block.strip().split("\n")
        sha, subject, when = header.split("\x1f")
        cid = g.node(f"git:{sha}", f"● {subject[:40]}", "filesystem", "commit", sha=sha, when=when, subject=subject)
        g.edge(prev or (f"git:branch:{head}" if head else root), cid, "filesystem", "history")
        prev = cid
        for f in files:
            if f.strip():
                g.edge(cid, f"file:{ROOT / f.strip()}", "filesystem", "touches")


# ───────────────────────── memory + markdown ─────────────────────────
WIKI = re.compile(r"\[\[([^\]|#]+)")
MDLINK = re.compile(r"\]\(([^)#\s]+\.md)\)")
PATHREF = re.compile(r"(?:~/YZKR/|/Users/[^/]+/YZKR/)([\w./-]+)")


@layer("memory")
def _memory(g: Graph) -> None:
    entities = {}
    rels = []
    for line in MEMORY_FILE.read_text().splitlines() if MEMORY_FILE.exists() else []:
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        if rec.get("type") == "entity":
            entities[rec["name"]] = rec
        elif rec.get("type") == "relation":
            rels.append(rec)
    for name, rec in entities.items():
        g.node(f"mem:{name}", name, "memory", "entity", entityType=rec.get("entityType"), observations=rec.get("observations", []))
    for r in rels:
        for end in (r["from"], r["to"]):
            g.node(f"mem:{end}", end, "memory", "entity")
        g.edge(f"mem:{r['from']}", f"mem:{r['to']}", "memory", r.get("relationType", "related"))
    for name, rec in entities.items():
        for obs in rec.get("observations", []):
            for rel in PATHREF.findall(obs):
                p = ROOT / rel.rstrip(".,)")
                if p.exists():
                    g.edge(f"mem:{name}", g.file(p, "memory"), "memory", "mentions")

    notes = [p for p in ROOT.rglob("*.md") if not SKIP & set(p.relative_to(ROOT).parts)]
    by_stem = {p.stem.lower(): p for p in notes}
    lowered = {n.lower(): n for n in entities}
    for p in notes:
        nid = g.file(p, "memory")
        text = p.read_text(errors="ignore")
        for target in WIKI.findall(text):
            t = target.strip()
            if t.lower() in lowered:
                g.edge(nid, f"mem:{lowered[t.lower()]}", "memory", "wikilink")
            elif t.lower() in by_stem:
                g.edge(nid, g.file(by_stem[t.lower()], "memory"), "memory", "wikilink")
            else:
                g.edge(nid, g.node(f"mem:{t}", t, "memory", "dangling"), "memory", "wikilink")
        for href in MDLINK.findall(text):
            if not href.startswith("http") and (tp := (p.parent / href).resolve()).exists():
                g.edge(nid, g.file(tp, "memory"), "memory", "link")
        low = text.lower()
        for lname, name in lowered.items():
            if lname in low:
                g.edge(nid, f"mem:{name}", "memory", "mentions")
        for rel in PATHREF.findall(text):
            tp = ROOT / rel.rstrip(".,)`")
            if tp.exists():
                g.edge(nid, g.file(tp, "memory"), "memory", "mentions")


# ───────────────────────── chat context + history ─────────────────────────
FILE_KEYS = ("file_path", "path", "notebook_path")


def _session_title(path: Path, first_user: str) -> str:
    custom = path.with_suffix("") / "custom-title.json"
    if custom.exists():
        try:
            t = json.loads(custom.read_text())
            return t.get("title") or t.get("customTitle") or first_user
        except (json.JSONDecodeError, AttributeError):
            pass
    return first_user


def _claude_sessions(g: Graph, newest: float) -> None:
    logs = sorted(CLAUDE_PROJECTS.glob("*/*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True)[:MAX_SESSIONS]
    for path in logs:
        models, files, first_user, turns = set(), set(), "", 0
        for line in path.open(errors="ignore"):
            try:
                d = json.loads(line)
            except json.JSONDecodeError:
                continue
            msg = d.get("message") or {}
            if d.get("type") == "user" and not first_user:
                c = msg.get("content")
                text = c if isinstance(c, str) else " ".join(x.get("text", "") for x in c or [] if isinstance(x, dict))
                if text and not text.startswith("<"):
                    first_user = " ".join(text.split())[:60]
            if d.get("type") == "assistant":
                turns += 1
                if m := msg.get("model"):
                    models.add(m)
                for block in msg.get("content") or []:
                    if isinstance(block, dict) and block.get("type") == "tool_use":
                        inp = block.get("input") or {}
                        for k in FILE_KEYS:
                            if isinstance(inp.get(k), str):
                                files.add(inp[k])
        mtime = path.stat().st_mtime
        sid = g.node(f"chat:{path.stem}", _session_title(path, first_user) or path.stem[:8], "chat", "session",
                     project=path.parent.name, turns=turns, mtime=mtime, active=time.time() - mtime < 900,
                     latest=mtime == newest, source="claude-code")
        for m in models:
            if not m.startswith("<"):
                g.edge(sid, g.node(f"model:{m}", m, "chat", "model"), "chat", "model")
        for f in files:
            p = Path(f)
            if p.is_relative_to(ROOT) and p.exists():
                g.edge(sid, g.file(p, "chat"), "chat", "touched")


def _aios_chats(g: Graph) -> None:
    for path in sorted((DATA / "chats").glob("*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True)[:MAX_SESSIONS]:
        msgs = [json.loads(l) for l in path.read_text().splitlines() if l.strip()]
        first = next((m["content"] for m in msgs if m.get("role") == "user"), path.stem)
        cid = g.node(f"aichat:{path.stem}", " ".join(first.split())[:60], "chat", "session", turns=len(msgs),
                     mtime=path.stat().st_mtime, source="ai-os", active=time.time() - path.stat().st_mtime < 900)
        for m in {m.get("model") for m in msgs if m.get("model")}:
            g.edge(cid, g.node(f"model:{m}", m, "chat", "model"), "chat", "model")
        text = " ".join(m.get("content", "") for m in msgs)
        for rel in PATHREF.findall(text):
            p = ROOT / rel.rstrip(".,)`")
            if p.exists():
                g.edge(cid, g.file(p, "chat"), "chat", "mentions")


@layer("chat")
def _chat(g: Graph) -> None:
    logs = list(CLAUDE_PROJECTS.glob("*/*.jsonl"))
    newest = max((p.stat().st_mtime for p in logs), default=0)
    _claude_sessions(g, newest)
    _aios_chats(g)


# ───────────────────────── build + cache ─────────────────────────
_cache: dict = {"t": 0.0, "data": None}


def build(force: bool = False, ttl: float = 20) -> dict:
    if not force and _cache["data"] and time.time() - _cache["t"] < ttl:
        return _cache["data"]
    g = Graph()
    timings, errors = {}, {}
    # filesystem first so shared file nodes carry the filesystem layer by default
    for name in ("filesystem", "memory", "chat"):
        t0 = time.time()
        try:
            LAYERS[name](g)
        except Exception as e:  # one broken source shouldn't blank the whole map
            errors[name] = str(e)
        timings[name] = round(time.time() - t0, 3)
    degree: dict[str, int] = {}
    for e in g.edges.values():
        degree[e["source"]] = degree.get(e["source"], 0) + 1
        degree[e["target"]] = degree.get(e["target"], 0) + 1
    for n in g.nodes.values():
        n["degree"] = degree.get(n["id"], 0)
        n["layers"] = sorted({e["layer"] for e in g.edges.values() if n["id"] in (e["source"], e["target"])} | {n["layer"]})
    data = {"nodes": list(g.nodes.values()), "edges": list(g.edges.values()), "timings": timings, "errors": errors}
    _cache.update(t=time.time(), data=data)
    return data
