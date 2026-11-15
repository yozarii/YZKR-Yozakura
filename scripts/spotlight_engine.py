# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "httpx>=0.27",
# ]
# ///
"""spotlight_engine — YZKR semantic launcher: local index + grounded LLM intent resolution.

Pipeline (no command prefixes, no if/else trees):
  sources (registry) ─▶ SQLite FTS5 trigram index ─▶ lexical candidates (+usage boost)
      ─▶ qwen2.5 picks {action, candidate} from schema enums built from the registries
      ─▶ action registry executes; falls back to lexical top hit when Ollama is down.

CLI:
  spotlight_engine.py index                      rebuild index
  spotlight_engine.py search "<text>"            lexical candidates (JSON)
  spotlight_engine.py resolve "<text>"           resolution only (JSON)
  spotlight_engine.py run "<text>"               resolve + execute
  spotlight_engine.py ask                        native prompt → run (Sol fallback when AI-OS absent)
  spotlight_engine.py macro list|show|run|rm <name>
  spotlight_engine.py macro record "<what it should do>" [--name N] [--dry-run]
  spotlight_engine.py macro history <N> --name N
  spotlight_engine.py sol-sync                   install Sol launcher scripts
"""

from __future__ import annotations

import argparse
import json
import math
import os
import plistlib
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.parse
from collections.abc import Callable, Iterator
from dataclasses import asdict, dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import yzkr_macros as macros  # noqa: E402
from yzkr_llm import LLMUnavailable, chat_json  # noqa: E402

HOME = Path.home()
YZKR = Path(os.environ.get("YZKR_HOME", HOME / "YZKR"))
DB_PATH = YZKR / "ai-os" / "data" / "spotlight.db"
AIOS_APP = HOME / "Applications" / "YZKR AI-OS.app"
AIOS_HTTP = os.environ.get("YZKR_AIOS_URL", "http://127.0.0.1:7777")
SEARCH_URL = os.environ.get("YZKR_SEARCH_URL", "https://www.google.com/search?q={q}")

SKIP_DIRS = {".git", "node_modules", ".backup", "__pycache__", ".build", "data", "vendor", ".venv", "dist", "build"}
TEXT_EXT = {".md", ".txt", ".py", ".sh", ".zsh", ".js", ".ts", ".json", ".jsonl", ".swift", ".html", ".css", ".toml", ".yaml", ".yml"}
STOPWORDS = set("a an the my me i to of in on for and or please can you could would open launch start show go goto jump find run "
                "search look up with from that this thing app file folder edit view".split())


# ═══════════════════════════ documents & sources ═══════════════════════════
@dataclass
class Doc:
    kind: str
    title: str
    uri: str
    body: str = ""
    action: str = "open"  # default action when the LLM is unavailable or unsure

    @property
    def id(self) -> str:
        return f"{self.kind}:{self.uri}"


SOURCES: dict[str, Callable[[], Iterator[Doc]]] = {}


def source(name: str):
    def deco(fn: Callable[[], Iterator[Doc]]):
        SOURCES[name] = fn
        return fn
    return deco


@source("apps")
def _apps() -> Iterator[Doc]:
    roots = [Path("/Applications"), Path("/Applications/Utilities"), Path("/System/Applications"),
             Path("/System/Applications/Utilities"), HOME / "Applications"]
    seen = set()
    for root in roots:
        for app in root.glob("*.app") if root.exists() else []:
            if app.stem in seen:
                continue
            seen.add(app.stem)
            info = {}
            try:
                info = plistlib.loads((app / "Contents" / "Info.plist").read_bytes())
            except Exception:
                pass
            meta = " ".join(str(info.get(k, "")) for k in ("CFBundleIdentifier", "CFBundleDisplayName", "LSApplicationCategoryType"))
            yield Doc("app", app.stem, str(app), f"application {meta}", "open")


def _walk(root: Path, max_depth: int) -> Iterator[Path]:
    root_depth = len(root.parts)
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS and not d.startswith(".") or d == ".claude"]
        depth = len(Path(dirpath).parts) - root_depth
        if depth >= max_depth:
            dirnames[:] = []
        yield Path(dirpath)
        for f in filenames:
            yield Path(dirpath) / f


@source("yzkr")
def _yzkr_files() -> Iterator[Doc]:
    for p in _walk(YZKR, 6):
        rel = p.relative_to(YZKR)
        if p.is_dir():
            if p != YZKR:
                yield Doc("folder", f"{p.name}/", str(p), f"folder directory {rel}", "goto")
            continue
        if p.suffix not in TEXT_EXT or p.stat().st_size > 256_000:
            continue
        head = p.read_text(errors="ignore")[:4000]
        kind = "note" if p.suffix == ".md" else "script" if os.access(p, os.X_OK) or p.suffix in {".sh", ".zsh"} else "file"
        yield Doc(kind, p.name, str(p), f"{rel} {head}", "edit")


@source("bin")
def _bin() -> Iterator[Doc]:
    for p in sorted((YZKR / "bin").glob("*")):
        if p.is_file() and os.access(p, os.X_OK):
            yield Doc("script", p.name, str(p), f"command {p.read_text(errors='ignore')[:1500]}", "edit")


@source("repos")
def _repos() -> Iterator[Doc]:
    roots = [YZKR, HOME / "Developer", HOME / "Projects", HOME / "code", HOME / "src", HOME / "Documents"]
    for root in (r for r in roots if r.exists()):
        for git in [root / ".git", *root.glob("*/.git"), *root.glob("*/*/.git")]:
            if not git.exists():
                continue
            repo = git.parent
            def g(*a: str) -> str:
                return subprocess.run(["git", "-C", str(repo), *a], capture_output=True, text=True, timeout=5).stdout.strip()
            yield Doc("repo", repo.name, str(repo), f"git repository {g('branch', '--show-current')} {g('log', '-1', '--format=%s')} {g('remote', 'get-url', 'origin')}", "goto")


@source("shell")
def _shell() -> Iterator[Doc]:
    rc = HOME / ".zshrc"
    text = rc.read_text(errors="ignore") if rc.exists() else ""
    for name, cmd in re.findall(r"^\s*alias\s+([\w.-]+)=['\"]?(.+?)['\"]?\s*$", text, re.M):
        yield Doc("alias", name, name, f"zsh alias {cmd}", "run")
    for name in re.findall(r"^\s*(?:function\s+)?([\w-]+)\s*\(\)\s*\{", text, re.M):
        yield Doc("alias", name, name, "zsh function", "run")


@source("macros")
def _macros() -> Iterator[Doc]:
    for m in macros.list_macros():
        yield Doc("macro", m.name, m.slug, f"macro {m.description} {' '.join(s['value'] for s in m.steps)}", "run")


def _firefox_bookmarks(profile_root: Path, browser: str) -> Iterator[Doc]:
    for places in profile_root.glob("*/places.sqlite"):
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td) / "p.sqlite"
            shutil.copy2(places, tmp)
            con = sqlite3.connect(tmp)
            rows = con.execute("SELECT b.title, p.url FROM moz_bookmarks b JOIN moz_places p ON p.id=b.fk "
                               "WHERE b.type=1 AND p.url LIKE 'http%'").fetchall()
            con.close()
        for title, url in rows:
            yield Doc("bookmark", title or url, url, f"{browser} bookmark {url}", "open")


def _chromium_bookmarks(path: Path, browser: str) -> Iterator[Doc]:
    def walk(node: dict) -> Iterator[Doc]:
        if node.get("type") == "url":
            yield Doc("bookmark", node.get("name") or node["url"], node["url"], f"{browser} bookmark {node['url']}", "open")
        for child in node.get("children", []):
            yield from walk(child)
    for root in json.loads(path.read_text()).get("roots", {}).values():
        if isinstance(root, dict):
            yield from walk(root)


def _safari_bookmarks(path: Path) -> Iterator[Doc]:
    def walk(node: dict) -> Iterator[Doc]:
        if node.get("WebBookmarkType") == "WebBookmarkTypeLeaf" and (url := node.get("URLString")):
            title = node.get("URIDictionary", {}).get("title") or url
            yield Doc("bookmark", title, url, f"safari bookmark {url}", "open")
        for child in node.get("Children", []):
            yield from walk(child)
    yield from walk(plistlib.loads(path.read_bytes()))


BOOKMARK_STORES: list[tuple[Path, Callable[[Path], Iterator[Doc]]]] = [
    (HOME / "Library/Application Support/zen/Profiles", lambda p: _firefox_bookmarks(p, "zen")),
    (HOME / "Library/Application Support/Firefox/Profiles", lambda p: _firefox_bookmarks(p, "firefox")),
    (HOME / "Library/Safari/Bookmarks.plist", _safari_bookmarks),
    *[(HOME / f"Library/Application Support/{d}/Default/Bookmarks", lambda p, n=n: _chromium_bookmarks(p, n))
      for d, n in [("Google/Chrome", "chrome"), ("BraveSoftware/Brave-Browser", "brave"), ("Arc/User Data", "arc"),
                   ("Microsoft Edge", "edge")]],
]


@source("bookmarks")
def _bookmarks() -> Iterator[Doc]:
    for path, reader in BOOKMARK_STORES:
        if not path.exists():
            continue
        try:
            yield from reader(path)
        except (PermissionError, sqlite3.Error, OSError, ValueError):
            continue  # Safari needs Full Disk Access; locked DBs are skipped, not fatal


@source("memory")
def _memory() -> Iterator[Doc]:
    mem = YZKR / "ai-memory" / "memory.jsonl"
    for line in mem.read_text().splitlines() if mem.exists() else []:
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        if rec.get("type") == "entity":
            yield Doc("memory", rec["name"], rec["name"], f"memory {rec.get('entityType', '')} {' '.join(rec.get('observations', []))}", "graph")


PLANES = {
    "editor": "Editor plane — markdown and code editor for YZKR files and notes",
    "graph": "Graph mindmap plane — memory, filesystem and chat graphs",
    "home": "Home control panel — daemons, ollama, mcp servers, vault, macros, toggles",
    "chat": "AI control chat and terminal — talk to claude or local ollama, shell",
}


@source("planes")
def _planes() -> Iterator[Doc]:
    for name, desc in PLANES.items():
        yield Doc("plane", f"AI-OS {name.title()}", f"yzkr://{name}", desc, "open")


# ═══════════════════════════ index ═══════════════════════════
def db() -> sqlite3.Connection:
    DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    con = sqlite3.connect(DB_PATH)
    con.executescript(
        """
        CREATE VIRTUAL TABLE IF NOT EXISTS docs USING fts5(title, body, kind UNINDEXED, uri UNINDEXED,
            action UNINDEXED, tokenize='trigram');
        CREATE TABLE IF NOT EXISTS usage (uri TEXT PRIMARY KEY, count INTEGER NOT NULL DEFAULT 0, last REAL);
        CREATE TABLE IF NOT EXISTS resolutions (q TEXT PRIMARY KEY, json TEXT NOT NULL, ts REAL NOT NULL);
        CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT);
        """
    )
    return con


def build_index(only: list[str] | None = None) -> dict:
    t0 = time.time()
    counts, docs = {}, []
    for name, fn in SOURCES.items():
        if only and name not in only:
            continue
        try:
            batch = list(fn())
        except Exception as e:  # one broken source must not take down the index
            counts[name] = f"error: {e}"
            continue
        counts[name] = len(batch)
        docs.extend(batch)
    seen, unique = set(), []
    for d in docs:
        if d.id not in seen:
            seen.add(d.id)
            unique.append(d)
    with db() as con:
        con.execute("DELETE FROM docs")
        con.executemany("INSERT INTO docs(title, body, kind, uri, action) VALUES (?,?,?,?,?)",
                        [(d.title, d.body, d.kind, d.uri, d.action) for d in unique])
        con.execute("DELETE FROM resolutions")
        con.execute("INSERT OR REPLACE INTO meta VALUES ('indexed_at', ?)", (str(time.time()),))
    return {"docs": len(unique), "sources": counts, "seconds": round(time.time() - t0, 2)}


def stats() -> dict:
    with db() as con:
        rows = con.execute("SELECT kind, count(*) FROM docs GROUP BY kind").fetchall()
        ts = con.execute("SELECT v FROM meta WHERE k='indexed_at'").fetchone()
    return {"kinds": dict(rows), "total": sum(n for _, n in rows), "indexed_at": float(ts[0]) if ts else None}


def _terms(q: str) -> list[str]:
    words = re.findall(r"[\w.~/-]+", q.lower())
    return [w for w in words if w not in STOPWORDS] or words


def _grams(w: str) -> set[str]:
    return {w[i:i + 3] for i in range(len(w) - 2)}


def _fuzzy(term: str, title: str) -> float:
    """Best trigram Jaccard between a query term and any title word (1.0 = exact word)."""
    g = _grams(term)
    if not g:
        return 0.0
    return max((len(g & _grams(w)) / len(g | _grams(w)) for w in re.findall(r"[\w]+", title) if len(w) >= 3), default=0.0)


def search(q: str, limit: int = 12) -> list[dict]:
    """Lexical candidates: trigram FTS (OR of terms) reranked by title match, term coverage and usage."""
    terms = _terms(q)
    if not terms:
        return []
    with db() as con:
        grams = sorted({g for t in terms if len(t) >= 3 for g in _grams(t)})  # trigram OR = typo tolerance
        if grams:
            match = " OR ".join('"' + g.replace('"', '""') + '"' for g in grams[:64])
            rows = con.execute(
                "SELECT title, body, kind, uri, action, bm25(docs, 10.0, 1.0) FROM docs WHERE docs MATCH ? "
                "ORDER BY bm25(docs, 10.0, 1.0) LIMIT 200", (match,)).fetchall()
        else:
            like = f"%{terms[0]}%"
            rows = con.execute("SELECT title, body, kind, uri, action, 0 FROM docs WHERE title LIKE ? LIMIT 200",
                               (like,)).fetchall()
        usage = dict(con.execute("SELECT uri, count FROM usage").fetchall())
    phrase = " ".join(terms)
    scored = []
    for title, body, kind, uri, action, bm in rows:
        t = title.lower()
        stem = re.sub(r"\.\w+$|/$", "", t)
        covered = sum(1 for w in terms if w in t or w in body.lower())
        score = (
            -bm * 0.5
            + 12 * (stem == phrase)
            + 6 * t.startswith(phrase)
            + 3 * sum(1 for w in terms if w in t)
            + 4 * covered / len(terms)
            + 8 * sum(_fuzzy(w, t) for w in terms) / len(terms)
            + 2.5 * math.log1p(usage.get(uri, 0))
            + 1.5 * (kind in {"app", "macro", "plane"})
        )
        scored.append({"kind": kind, "title": title, "uri": uri, "action": action, "score": round(score, 2),
                       "snippet": body[:140].replace("\n", " ")})
    scored.sort(key=lambda r: r["score"], reverse=True)
    return scored[:limit]


# ═══════════════════════════ actions ═══════════════════════════
@dataclass
class Action:
    help: str
    run: Callable[[dict | None, str], dict]
    needs_target: bool = True
    kinds: set[str] = field(default_factory=set)  # empty = any kind
    verbs: set[str] = field(default_factory=set)  # if set, query must say one unless it's the target's default
    takes_target: bool = True  # False: the action ignores any candidate (pure text actions)


ACTIONS: dict[str, Action] = {}


def action(name: str, help: str, needs_target: bool = True, kinds: set[str] | None = None,
           verbs: set[str] | None = None, takes_target: bool = True):
    def deco(fn: Callable[[dict | None, str], dict]):
        ACTIONS[name] = Action(help, fn, needs_target, kinds or set(), verbs or set(), takes_target)
        return fn
    return deco


def _open(*args: str) -> dict:
    subprocess.Popen(["open", *args], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return {"ok": True, "opened": args[-1]}


def open_aios(route: str, **params: str) -> dict:
    """Route into the AI-OS window: native yzkr:// scheme when the app is installed, else the web UI."""
    qs = urllib.parse.urlencode({k: v for k, v in params.items() if v})
    if AIOS_APP.exists():
        return _open(f"yzkr://{route}" + (f"?{qs}" if qs else ""))
    return _open(f"{AIOS_HTTP}/#/{route}" + (f"?{qs}" if qs else ""))


@action("open", "launch an app, open a file, URL, bookmark or AI-OS plane")
def _a_open(doc, q):
    if doc["uri"].startswith("yzkr://"):
        return open_aios(doc["uri"].removeprefix("yzkr://"))
    return _open(doc["uri"])


@action("edit", "open a text file or note in the YZKR editor", kinds={"note", "file", "script"})
def _a_edit(doc, q):
    if doc["uri"].startswith(str(YZKR)):
        return open_aios("editor", path=doc["uri"])
    return _open("-t", doc["uri"])


@action("goto", "navigate to a folder or git repository", kinds={"folder", "repo"})
def _a_goto(doc, q):
    return _open(doc["uri"])


@action("reveal", "reveal an item in Finder", kinds={"app", "note", "file", "script", "folder", "repo"})
def _a_reveal(doc, q):
    return _open("-R", doc["uri"])


@action("run", "execute a macro, script or shell alias", kinds={"macro", "script", "alias"},
        verbs={"run", "execute", "exec", "do", "start", "trigger", "fire", "launch"})
def _a_run(doc, q):
    runners = {
        "macro": lambda: macros.run(doc["uri"]),
        "script": lambda: {"ok": True, "pid": subprocess.Popen([doc["uri"]], start_new_session=True,
                                                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).pid},
        "alias": lambda: open_aios("chat", term=doc["uri"]),
    }
    return runners[doc["kind"]]()


@action("graph", "explore relations between notes, files, memory and chats in the graph", needs_target=False)
def _a_graph(doc, q):
    return open_aios("graph", focus=doc["title"] if doc else "", q=q)


@action("ask_ai", "ask the AI assistant a question or give it a task", needs_target=False, takes_target=False)
def _a_ask(doc, q):
    return open_aios("chat", q=q)


@action("search_web", "search the web for information", needs_target=False, takes_target=False)
def _a_web(doc, q):
    return _open(SEARCH_URL.format(q=urllib.parse.quote_plus(q)))


@action("record_macro", "create a new reusable macro from a description of steps", needs_target=False, takes_target=False)
def _a_record(doc, q):
    m = macros.record_from_text(q)
    build_index(["macros"])
    return {"ok": True, "recorded": m.to_dict()}


# ═══════════════════════════ resolution ═══════════════════════════
_RESOLVE_SYSTEM = """You resolve a macOS launcher query to one action and (when needed) one candidate item.
Actions:
{actions}
Candidates (best lexical match first):
{cands}
Rules: pick the candidate the user most plausibly means despite loose phrasing or typos; prefer c0 unless the wording
clearly points to another. Use "none" only with actions that need no item. Questions or tasks for an assistant → ask_ai.
Facts about the outside world → search_web. Requests to make/record a macro or shortcut → record_macro. Choose run only
when the user asks to run/execute something.
Examples: "spotfy" → c0 Spotify, open. "jump to my scripts dir" → scripts/, goto. "how do I rebase" → none, ask_ai."""


def _schema(n: int) -> dict:
    # deliberately no free-text fields: 3B models under grammar constraints degenerate inside long strings
    return {
        "type": "object",
        "properties": {
            "candidate": {"enum": [f"c{i}" for i in range(n)] + ["none"]},
            "action": {"enum": list(ACTIONS)},
            "confidence": {"type": "number"},
        },
        "required": ["candidate", "action", "confidence"],
    }


def _fallback(q: str, cands: list[dict], why: str) -> dict:
    if cands and cands[0]["score"] >= 4.5:
        return {"action": cands[0]["action"], "target": cands[0], "query": q, "confidence": 0.5, "via": f"lexical ({why})"}
    return {"action": "search_web", "target": None, "query": q, "confidence": 0.3, "via": f"default ({why})"}


def _validate(res: dict, cands: list[dict], q: str) -> dict:
    """Make the LLM's pick consistent with the action registry's declared constraints."""
    act = ACTIONS.get(res["action"])
    target = res.get("target")
    if act is None:
        return _fallback(q, cands, "unknown action")
    if act.needs_target and target is None:
        return _fallback(q, cands, f"{res['action']} needs a target")
    if not act.takes_target:
        res["target"] = None
        return res
    if target and act.kinds and target["kind"] not in act.kinds:
        res["action"] = target["action"]  # e.g. "run" on a note → its default "edit"
    elif target and act.verbs and res["action"] != target["action"] and not act.verbs & set(q.lower().split()):
        res["action"] = target["action"]  # side-effecting action needs an explicit verb
    return res


def resolve(q: str, use_cache: bool = True) -> dict:
    q = q.strip()
    cands = search(q, 8)
    top = cands[0] if cands else None
    # unambiguous exact title hit: skip the model entirely (latency matters in a launcher)
    if top and top["score"] >= 20 and (len(cands) == 1 or top["score"] - cands[1]["score"] >= 8):
        return {"action": top["action"], "target": top, "query": q, "confidence": 0.99, "via": "exact", "candidates": cands}
    key = q.lower()
    with db() as con:
        if use_cache and (row := con.execute("SELECT json FROM resolutions WHERE q=?", (key,)).fetchone()):
            return {**json.loads(row[0]), "via": "cache", "candidates": cands}
    system = _RESOLVE_SYSTEM.format(
        actions="\n".join(f"- {k}: {a.help}" for k, a in ACTIONS.items()),
        cands="\n".join(f"c{i}: {c['title']} ({c['kind']})" for i, c in enumerate(cands)) or "(none)",
    )
    try:
        out = chat_json(system, f"Query: {q}", _schema(len(cands)), max_tokens=64)
    except LLMUnavailable as e:
        return {**_fallback(q, cands, f"llm: {e}"), "candidates": cands}
    idx = out.get("candidate", "none")
    target = cands[int(idx[1:])] if idx != "none" and idx[1:].isdigit() and int(idx[1:]) < len(cands) else None
    res = _validate({"action": out.get("action"), "target": target, "query": q,
                     "confidence": float(out.get("confidence", 0.5)), "via": "llm"}, cands, q)
    with db() as con:
        con.execute("INSERT OR REPLACE INTO resolutions VALUES (?,?,?)", (key, json.dumps(res), time.time()))
    return {**res, "candidates": cands}


def execute(res: dict) -> dict:
    target = res.get("target")
    out = ACTIONS[res["action"]].run(target, res.get("query", ""))
    if target:
        with db() as con:
            con.execute("INSERT INTO usage(uri, count, last) VALUES (?,1,?) ON CONFLICT(uri) DO UPDATE SET "
                        "count=count+1, last=excluded.last", (target["uri"], time.time()))
    return {"action": res["action"], "target": target and target["title"], **out}


def run_query(q: str) -> dict:
    return execute(resolve(q))


def execute_doc(doc: dict, action_name: str | None = None, q: str = "") -> dict:
    """Run a specific candidate (UI click) with its default or a chosen action."""
    return execute({"action": action_name or doc["action"], "target": doc, "query": q})


# ═══════════════════════════ Sol + native prompt ═══════════════════════════
SOL_CONFIG = HOME / ".config" / "sol" / "config.json"
ENGINE = Path(__file__).resolve()
UV = shutil.which("uv") or "/opt/homebrew/bin/uv"


def sol_sync(set_hotkey: bool = False) -> dict:
    """Install Sol launcher scripts (palette + one per macro); optionally make Cmd+Space Sol's hotkey."""
    macros.SOL_SCRIPTS.mkdir(parents=True, exist_ok=True)
    palette = macros.SOL_SCRIPTS / "yzkr-spotlight.sh"
    palette.write_text(
        "#!/bin/bash\n# name: YZKR Spotlight\n# icon: 🔮\n"
        f"if [ -d {json.dumps(str(AIOS_APP))} ]; then open 'yzkr://palette'; else {UV} run -q {json.dumps(str(ENGINE))} ask; fi\n"
    )
    reindex = macros.SOL_SCRIPTS / "yzkr-reindex.sh"
    reindex.write_text(f"#!/bin/bash\n# name: YZKR Reindex\n# icon: 🗂️\n{UV} run -q {json.dumps(str(ENGINE))} index >/dev/null\n")
    record = macros.SOL_SCRIPTS / "yzkr-record-macro.sh"
    record.write_text(f"#!/bin/bash\n# name: YZKR Record Macro\n# icon: ⏺\n{UV} run -q {json.dumps(str(ENGINE))} ask --record\n")
    for p in (palette, reindex, record):
        p.chmod(0o755)
    scripts = [p.name for p in (palette, reindex, record)] + macros.sol_sync()
    hotkey = None
    if set_hotkey:
        cfg = json.loads(SOL_CONFIG.read_text()) if SOL_CONFIG.exists() else {}
        cfg["globalShortcut"] = "command"  # Sol: "command" = ⌘Space, "option" = ⌥Space
        SOL_CONFIG.write_text(json.dumps(cfg, indent=2))
        hotkey = "⌘Space"
    return {"scripts": scripts, "dir": str(macros.SOL_SCRIPTS), "hotkey": hotkey}


def native_prompt(title: str, hint: str) -> str | None:
    script = (f'display dialog {json.dumps(hint)} default answer "" with title {json.dumps(title)} '
              'buttons {"Cancel", "Go"} default button "Go"\ntext returned of result')
    r = subprocess.run(["osascript", "-e", script], capture_output=True, text=True)
    return r.stdout.strip() or None if r.returncode == 0 else None


def notify(text: str) -> None:
    subprocess.run(["osascript", "-e", f"display notification {json.dumps(text)} with title \"YZKR\""])


# ═══════════════════════════ CLI ═══════════════════════════
def _print(obj) -> None:
    print(json.dumps(obj, indent=2, ensure_ascii=False, default=str))


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(prog="spotlight_engine", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("index"); p.add_argument("--only", nargs="*")
    sub.add_parser("stats")
    for name in ("search", "resolve", "run"):
        p = sub.add_parser(name); p.add_argument("query", nargs="+")
    p = sub.add_parser("ask"); p.add_argument("--record", action="store_true")
    p = sub.add_parser("sol-sync"); p.add_argument("--hotkey", action="store_true", help="set Sol's hotkey to ⌘Space")
    mp = sub.add_parser("macro").add_subparsers(dest="mcmd", required=True)
    mp.add_parser("list")
    for name in ("show", "run", "rm"):
        p = mp.add_parser(name); p.add_argument("name")
    p = mp.add_parser("record"); p.add_argument("request", nargs="+"); p.add_argument("--name"); p.add_argument("--dry-run", action="store_true")
    p = mp.add_parser("history"); p.add_argument("n", type=int); p.add_argument("--name", required=True)
    a = ap.parse_args(argv)

    if a.cmd == "macro":
        handlers = {
            "list": lambda: [m.to_dict() for m in macros.list_macros()],
            "show": lambda: macros.get(a.name).to_dict() | {"script": macros.get(a.name).render()},
            "run": lambda: macros.run(a.name, wait=True),
            "rm": lambda: (macros.delete(a.name), {"deleted": a.name})[1],
            "record": lambda: (m := macros.record_from_text(" ".join(a.request), a.name, a.dry_run)).to_dict()
                              | ({"script": m.render()} if a.dry_run else {}),
            "history": lambda: macros.record_from_history(a.n, a.name).to_dict(),
        }
        result = handlers[a.mcmd]()
        if a.mcmd in {"record", "history", "rm"} and not getattr(a, "dry_run", False):
            build_index(["macros"])
        return _print(result)

    def ask() -> dict:
        prompt = native_prompt("YZKR Record Macro", "Describe the steps:") if a.record else \
            native_prompt("YZKR Spotlight", "Open, find, run, ask…")
        if not prompt:
            return {"cancelled": True}
        out = run_query(f"record a macro: {prompt}" if a.record else prompt)
        notify(f"{out.get('action')}: {out.get('target') or out.get('opened') or 'done'}")
        return out

    handlers = {
        "index": lambda: build_index(a.only),
        "stats": stats,
        "search": lambda: search(" ".join(a.query)),
        "resolve": lambda: resolve(" ".join(a.query)),
        "run": lambda: run_query(" ".join(a.query)),
        "ask": ask,
        "sol-sync": lambda: sol_sync(a.hotkey),
    }
    _print(handlers[a.cmd]())


if __name__ == "__main__":
    main()
