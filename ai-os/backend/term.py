"""Terminal plane backend: a real zsh on a pty, bridged to xterm.js over a websocket.

Protocol (client → server, JSON text frames): {"t":"i","d":"<input>"} | {"t":"r","c":cols,"r":rows}
Server → client: raw output text frames.
`cls` is provided via a ZDOTDIR shim that loads the user's own zsh config first.
"""

from __future__ import annotations

import asyncio
import codecs
import fcntl
import json
import os
import pty
import signal
import struct
import termios

from aiohttp import WSMsgType, web

from .config import DATA, HOME, ROOT

ZDOT = DATA / "zdotdir"


def _zdotdir() -> str:
    ZDOT.mkdir(exist_ok=True)
    for name in (".zshenv", ".zprofile", ".zlogin"):
        (ZDOT / name).write_text(f'[[ -f "$HOME/{name}" ]] && source "$HOME/{name}"\n')
    (ZDOT / ".zshrc").write_text(
        '[[ -f "$HOME/.zshrc" ]] && ZDOTDIR="$HOME" source "$HOME/.zshrc"\n'
        "alias cls='clear && printf \"\\e[3J\"'\n"
        "export YZKR_AIOS=1\n"
    )
    return str(ZDOT)


async def handle(request: web.Request) -> web.WebSocketResponse:
    ws = web.WebSocketResponse(heartbeat=30, max_msg_size=4 * 1024 * 1024)
    await ws.prepare(request)
    pid, fd = pty.fork()
    if pid == 0:  # child: become a login zsh in ~/YZKR
        os.chdir(request.query.get("cwd") or ROOT)
        env = {**os.environ, "TERM": "xterm-256color", "COLORTERM": "truecolor", "ZDOTDIR": _zdotdir(),
               "HOME": str(HOME), "LANG": os.environ.get("LANG", "en_US.UTF-8")}
        os.execvpe("/bin/zsh", ["-zsh", "-l"], env)

    loop = asyncio.get_running_loop()
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    out: asyncio.Queue[str | None] = asyncio.Queue()

    def on_readable():
        try:
            data = os.read(fd, 65536)
        except OSError:
            data = b""
        if not data:
            loop.remove_reader(fd)
            out.put_nowait(None)
            return
        out.put_nowait(decoder.decode(data))

    loop.add_reader(fd, on_readable)

    async def pump():
        while (chunk := await out.get()) is not None:
            # coalesce bursts (e.g. `ls -R`) into fewer frames
            while not out.empty() and (nxt := out.get_nowait()) is not None:
                chunk += nxt
            await ws.send_str(chunk)
        await ws.close()

    pump_task = asyncio.create_task(pump())
    try:
        async for msg in ws:
            if msg.type != WSMsgType.TEXT:
                continue
            m = json.loads(msg.data)
            if m.get("t") == "i":
                os.write(fd, m["d"].encode())
            elif m.get("t") == "r":
                fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", int(m["r"]), int(m["c"]), 0, 0))
    finally:
        pump_task.cancel()
        try:
            loop.remove_reader(fd)
        except (ValueError, OSError):
            pass
        try:
            os.kill(pid, signal.SIGHUP)
            os.close(fd)
            os.waitpid(pid, os.WNOHANG)
        except OSError:
            pass
    return ws
