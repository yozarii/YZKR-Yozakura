"""Tool-use candidate A: FunctionGemma 270M through Ollama's /api/chat tool calling.

    uv run --python 3.13 --with psutil python tum_ollama.py --model functiongemma --ctx 8192
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", default="functiongemma")
    ap.add_argument("--ctx", type=int, default=8192)
    a = ap.parse_args()
    registry = common.corpus("tool_registry")
    tools = [{"type": "function", "function": t} for t in registry]
    common.ollama_isolate(a.model)

    def ask(state):
        body = {"model": a.model, "stream": False, "keep_alive": "15m", "tools": tools,
                "messages": [{"role": "system", "content": common.TOOL_SYSTEM}, {"role": "user", "content": state}],
                "options": {"temperature": 0, "num_ctx": a.ctx, "num_predict": 128}}
        m = common.http_json("http://127.0.0.1:11434/api/chat", body)["message"]
        calls = m.get("tool_calls") or []
        got = {"name": calls[0]["function"]["name"], "arguments": calls[0]["function"].get("arguments") or {}} if calls else common.lenient_tool_call(m.get("content", ""))
        return got, bool((m.get("content") or "").strip())

    cases = common.corpus("tools")
    (_, _), cold = common.timed(lambda: ask(cases[0]["state"]))
    rows = []
    for c in cases:
        (got, prose), ms = common.timed(lambda c=c: ask(c["state"]))
        rows.append({"name": c["name"], "expected": c["expected"], "got": got, "ms": ms, "prose": prose, "score": common.score_tool_case(c["expected"], got, registry)})
    common.tool_report(f"{a.model} via Ollama /api/chat (num_ctx {a.ctx})", rows, common.ollama_footprint_mb(),
                       extra={"cold: first call incl. model load (ms)": round(cold), "ollama ps": common.ollama_ps(a.model + ":latest") or common.ollama_ps(a.model)},
                       save=f"tum_ollama_{a.model.replace(':', '_')}_{a.ctx}")


if __name__ == "__main__":
    main()
