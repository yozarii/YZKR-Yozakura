# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "aiohttp>=3.9",
#   "httpx>=0.27",
#   "anthropic>=1",
# ]
# ///
"""YZKR AI-OS server — one local process behind the single-window hub.

Binds 127.0.0.1 only. Every /api and /ws call needs the per-launch token embedded in index.html, plus a matching
Host (DNS-rebinding guard) and, for websockets, a same-origin Origin header — a localhost shell bridge must not be
reachable from arbitrary web pages.

  uv run ai-os/server.py            serve on http://127.0.0.1:7777
"""

from __future__ import annotations

import asyncio
import json
import secrets
import sys
import time
from pathlib import Path

from aiohttp import web

sys.path.insert(0, str(Path(__file__).resolve().parent))
from backend import chat, files, graph, system, term  # noqa: E402
from backend.config import DATA, HOST, PORT, WEB, theme  # noqa: E402

import spotlight_engine as se  # noqa: E402  (scripts/ on sys.path via backend.config)
import yzkr_macros as macros  # noqa: E402

TOKEN = secrets.token_urlsafe(24)
ALLOWED_HOSTS = {f"{HOST}:{PORT}", f"localhost:{PORT}"}
ORIGINS = {f"http://{h}" for h in ALLOWED_HOSTS}
REINDEX_EVERY = 15 * 60


# ───────────────────────── middleware ─────────────────────────
@web.middleware
async def guard(request: web.Request, handler):
    if request.host not in ALLOWED_HOSTS:
        raise web.HTTPForbidden(text="bad host")
    path = request.path
    if path.startswith(("/api/", "/ws/")):
        token = request.headers.get("X-YZKR-Token") or request.query.get("token")
        if not token or not secrets.compare_digest(token, TOKEN):
            raise web.HTTPUnauthorized(text="token required")
        if path.startswith("/ws/") and request.headers.get("Origin") not in ORIGINS:
            raise web.HTTPForbidden(text="bad origin")
    try:
        return await handler(request)
    except web.HTTPException:
        raise
    except files.SandboxError as e:
        return web.json_response({"error": str(e)}, status=403)
    except (FileNotFoundError, KeyError) as e:
        return web.json_response({"error": str(e)}, status=404)
    except (ValueError, FileExistsError) as e:
        return web.json_response({"error": str(e)}, status=409)


def j(data, status: int = 200) -> web.Response:
    return web.json_response(data, status=status, dumps=lambda o: json.dumps(o, ensure_ascii=False, default=str))


async def body(request: web.Request) -> dict:
    return await request.json() if request.can_read_body else {}


def threaded(fn):
    """Run a blocking handler body in a worker thread."""
    async def wrapper(request: web.Request):
        return j(await asyncio.to_thread(fn, request.query, await body(request)))
    return wrapper


# ───────────────────────── pages ─────────────────────────
async def index(request: web.Request) -> web.Response:
    html = (WEB / "index.html").read_text().replace("__YZKR_TOKEN__", TOKEN)
    # frame-ancestors only works as a header; stops other pages framing the token-bearing UI
    return web.Response(text=html, content_type="text/html", headers={
        "Cache-Control": "no-store", "Content-Security-Policy": "frame-ancestors 'none'", "X-Frame-Options": "DENY"})


async def theme_css(request: web.Request) -> web.Response:
    t = theme()
    kebab = lambda s: "".join(f"-{c.lower()}" if c.isupper() else c for c in s)
    lines = [f"  --{kebab(k)}: {v};" for k, v in t["colors"].items()]
    lines += [f"  --graph-{k}: {v};" for k, v in t["graph"].items()]
    lines += [f"  --font-{k}: {v};" for k, v in t["typography"].items()]
    lines += [f"  --radius-{k}: {v}px;" for k, v in t["radius"].items()]
    return web.Response(text=":root {\n" + "\n".join(lines) + "\n}\n", content_type="text/css")


# ───────────────────────── API ─────────────────────────
routes = web.RouteTableDef()

routes.get("/api/fs/list")(threaded(lambda q, b: files.listdir(q.get("path"))))
routes.get("/api/fs/read")(threaded(lambda q, b: files.read(q["path"])))
routes.post("/api/fs/write")(threaded(lambda q, b: files.write(b["path"], b["text"], b.get("mtime"))))

routes.get("/api/graph")(threaded(lambda q, b: graph.build(force=q.get("force") == "1")))

routes.get("/api/system")(threaded(lambda q, b: system.status()))
routes.post("/api/system/toggle")(threaded(lambda q, b: system.toggle(b["id"], bool(b.get("on")))))

routes.get("/api/spotlight/search")(threaded(lambda q, b: se.search(q.get("q", ""), int(q.get("limit", 12)))))
routes.post("/api/spotlight/resolve")(threaded(lambda q, b: se.resolve(b["q"])))
routes.post("/api/spotlight/run")(threaded(lambda q, b: se.run_query(b["q"])))
routes.post("/api/spotlight/exec")(threaded(lambda q, b: se.execute_doc(b["doc"], b.get("action"), b.get("q", ""))))
routes.post("/api/spotlight/index")(threaded(lambda q, b: se.build_index(b.get("only"))))

routes.get("/api/macros")(threaded(lambda q, b: [m.to_dict() for m in macros.list_macros()]))
routes.post("/api/macros/run")(threaded(lambda q, b: macros.run(b["slug"], wait=True)))
routes.post("/api/macros/delete")(threaded(lambda q, b: (macros.delete(b["slug"]), se.build_index(["macros"]))[1]))
routes.post("/api/macros/history")(threaded(lambda q, b: (m := macros.record_from_history(int(b["n"]), b["name"]),
                                                          se.build_index(["macros"]))[0].to_dict()))


@routes.post("/api/macros/record")
async def record_macro(request: web.Request) -> web.Response:
    b = await body(request)
    m = await asyncio.to_thread(macros.record_from_text, b["request"], b.get("name") or None, bool(b.get("dry_run")))
    if not b.get("dry_run"):
        await asyncio.to_thread(se.build_index, ["macros"])
    return j({**m.to_dict(), "script": m.render()})


@routes.post("/api/macros/save")
async def save_macro(request: web.Request) -> web.Response:
    b = await body(request)
    m = macros.Macro(macros.slugify(b["name"]), b["name"], b.get("icon", "⚡"), b.get("description", ""), b["steps"])
    await asyncio.to_thread(macros.save, m)
    await asyncio.to_thread(se.build_index, ["macros"])
    return j(m.to_dict())


@routes.get("/api/chat/providers")
async def chat_providers(request: web.Request) -> web.Response:
    ps = await asyncio.to_thread(chat.providers)
    return j({"providers": [p.__dict__ for p in ps], "claude_configured": chat.claude_configured()})


routes.get("/api/chat/list")(threaded(lambda q, b: chat.list_chats()))
routes.get("/api/chat/history")(threaded(lambda q, b: chat.history(q["id"])))


@routes.post("/api/chat/send")
async def chat_send(request: web.Request) -> web.StreamResponse:
    b = await body(request)
    chat_id = b.get("id") or chat.new_id()
    resp = web.StreamResponse(headers={"Content-Type": "application/x-ndjson", "Cache-Control": "no-store"})
    await resp.prepare(request)
    await resp.write(json.dumps({"t": "chat", "id": chat_id}).encode() + b"\n")
    async for ev in chat.respond(chat_id, b["text"], b.get("provider", "auto")):
        await resp.write(json.dumps(ev, ensure_ascii=False).encode() + b"\n")
    await resp.write_eof()
    return resp


@routes.get("/api/health")
async def health(request: web.Request) -> web.Response:
    return j({"ok": True, "pid": __import__("os").getpid(), "started": STARTED})


routes.get("/ws/term")(term.handle)


# ───────────────────────── lifecycle ─────────────────────────
async def reindex_loop(app: web.Application):
    async def loop():
        while True:
            try:
                await asyncio.to_thread(se.build_index)
            except Exception as e:  # keep serving even if a source breaks
                print(f"reindex failed: {e}", file=sys.stderr)
            await asyncio.sleep(REINDEX_EVERY)
    task = asyncio.create_task(loop())
    yield
    task.cancel()


STARTED = time.time()


def make_app() -> web.Application:
    app = web.Application(middlewares=[guard], client_max_size=8 * 1024 * 1024)
    app.router.add_get("/", index)
    app.router.add_get("/theme.css", theme_css)
    app.add_routes(routes)
    app.router.add_static("/", WEB, show_index=False, append_version=False)
    app.cleanup_ctx.append(reindex_loop)
    return app


if __name__ == "__main__":
    (DATA / "server.pid").write_text(str(__import__("os").getpid()))
    print(f"YZKR AI-OS → http://{HOST}:{PORT}", flush=True)
    web.run_app(make_app(), host=HOST, port=PORT, print=None, access_log=None)
