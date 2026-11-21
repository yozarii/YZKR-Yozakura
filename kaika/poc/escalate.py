#!/usr/bin/env python3
"""Escalation adapter: asks a bigger model to settle a decision the small one was unsure about.

Contract (the same one every escalation command follows): the request JSON on stdin, the answer JSON on stdout.

    in   {"task", "query", "candidates": [{"id", "text"}], "threshold"}
    out  {"choice": id | null, "confidence": 0-1, "evidence": "..."}

This adapter runs the user's own Claude Code CLI headless. It is locked down: an empty temp directory as the
working directory (no project files or CLAUDE.md), every tool disabled, no session saved, no skills, no MCP
servers, only project-level settings (so none of the user's hooks run), and a short system prompt of its own.
The model sees the task, the query and the candidate texts, which are public web data, and nothing else.
It answers with one letter, exactly as the small model does, and the letter is mapped back to an id here.

Environment: KAIKA_CLAUDE_BIN (default: claude), KAIKA_ESCALATION_MODEL (default: haiku).
"""
import json
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import resolver  # noqa: E402

DECISIVE = 0.9  # a bigger model that names a candidate is treated as decisive; "none" and noise are not


def ask_claude(system, user, model, claude_bin, timeout=120):
    with tempfile.TemporaryDirectory() as cwd:
        argv = [
            claude_bin, "-p", user,
            "--model", model,
            "--output-format", "text",
            "--tools", "",
            "--no-session-persistence",
            "--disable-slash-commands",
            "--strict-mcp-config",
            "--setting-sources", "project",
            "--system-prompt", system,
        ]
        p = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)
    if p.returncode != 0:
        raise RuntimeError(f"claude exited with {p.returncode}")
    return p.stdout


def answer(req, runner=ask_claude):
    cands = req.get("candidates", [])[: resolver.MAX_CANDIDATES]
    if len(cands) < 2:
        return {"choice": cands[0]["id"] if cands else None, "confidence": DECISIVE if cands else 0.0, "evidence": "nothing to decide"}
    system, user = resolver.build_prompt(req.get("task", "pick the best candidate"), req.get("query", ""), cands)
    model = os.environ.get("KAIKA_ESCALATION_MODEL", "haiku")
    text = runner(system, user, model, os.environ.get("KAIKA_CLAUDE_BIN", "claude")).strip().upper()
    letter = text[:1]
    valid = resolver.LABELS[: len(cands)]
    if letter and letter in valid:  # an empty reply is not "A": "" is a substring of everything
        return {"choice": cands[valid.index(letter)]["id"], "confidence": DECISIVE, "evidence": f"asked {model}"}
    if letter == resolver.NONE_LABEL:
        return {"choice": None, "confidence": 0.0, "evidence": f"{model} says none of the candidates fit"}
    return {"choice": None, "confidence": 0.0, "evidence": f"{model} gave an answer that is not a candidate letter"}


def main():
    try:
        out = answer(json.load(sys.stdin))
    except Exception as e:  # unsure, never a crash
        out = {"choice": None, "confidence": 0.0, "evidence": f"escalation failed: {type(e).__name__}: {e}"}
    json.dump(out, sys.stdout)


if __name__ == "__main__":
    main()
