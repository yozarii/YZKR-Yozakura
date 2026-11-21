"""Tool-use candidates through llama-server (llama.cpp) with the GGUF's own chat template (--jinja).

    uv run --python 3.13 --with psutil python tum_llama.py --gguf PATH --label xlam --ctx 8192 --kv q8_0
"""
import argparse
import json
import os
import signal
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402

PORT = 8089


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--gguf", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--ctx", type=int, default=8192)
    ap.add_argument("--kv", default="f16")
    ap.add_argument("--system", default="tools", choices=["tools", "developer"])
    a = ap.parse_args()
    registry = common.corpus("tool_registry")
    tools = [{"type": "function", "function": t} for t in registry]
    cmd = ["llama-server", "-m", a.gguf, "--host", "127.0.0.1", "--port", str(PORT), "-c", str(a.ctx), "-ngl", "99", "-fa", "on",
           "-ctk", a.kv, "-ctv", a.kv, "-np", "1", "--jinja", "--no-webui", "--no-warmup"]
    log = open(os.path.join(common.HERE, f"results/llama_{a.label}.log"), "w") if os.path.isdir(common.RESULTS) else subprocess.DEVNULL
    os.makedirs(common.RESULTS, exist_ok=True)
    log = open(os.path.join(common.RESULTS, f"llama_{a.label}.log"), "w")
    t0 = time.perf_counter()
    proc = subprocess.Popen(cmd, stdout=log, stderr=log, start_new_session=True)
    try:
        for _ in range(240):
            try:
                if common.http_json(f"http://127.0.0.1:{PORT}/health", timeout=2).get("status") == "ok":
                    break
            except Exception:
                time.sleep(0.5)
        else:
            sys.exit("llama-server did not become healthy; see results/llama_%s.log" % a.label)
        load_ms = (time.perf_counter() - t0) * 1000

        def ask(state):
            # Render the prompt with the GGUF's own chat template, then read the model's raw text ourselves:
            # kaika parses tool calls in Rust, so llama.cpp's server-side parser (which rejects some formats) is not the point.
            msgs = ([{"role": "system", "content": common.TOOL_SYSTEM}] if a.system == "tools"
                    else [{"role": "developer", "content": "You are a model that can do function calling with the following functions"}])
            msgs.append({"role": "user", "content": state})
            prompt = common.http_json(f"http://127.0.0.1:{PORT}/apply-template", {"messages": msgs, "tools": tools})["prompt"]
            out = common.http_json(f"http://127.0.0.1:{PORT}/completion", {"prompt": prompt, "n_predict": 128, "temperature": 0, "cache_prompt": True})
            text = (out.get("content") or "").strip()
            got = common.lenient_tool_call(text)
            return got, bool(text) and got is None, text[:90].replace("\n", " ")

        cases = common.corpus("tools")
        (_, _, raw0), cold = common.timed(lambda: ask(cases[0]["state"]))
        rows = []
        for c in cases:
            (got, prose, raw), ms = common.timed(lambda c=c: ask(c["state"]))
            rows.append({"name": c["name"], "expected": c["expected"], "got": got, "ms": ms, "prose": prose and got is None, "raw": raw, "score": common.score_tool_case(c["expected"], got, registry)})
        import psutil
        fp = common.footprint_mb(proc.pid)
        rss = round(psutil.Process(proc.pid).memory_info().rss / 1e6)
        common.tool_report(f"{a.label} via llama-server (ctx {a.ctx}, kv {a.kv})", rows, fp,
                           extra={"resident (RSS, includes mapped weights) MB": rss, "load time (s)": round(load_ms / 1000, 1), "cold: first call (ms)": round(cold), "sample raw reply": raw0 or rows[0]["raw"]},
                           save=f"tum_llama_{a.label}_{a.ctx}_{a.kv}")
    finally:
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
        except Exception:
            pass
        log.close()


if __name__ == "__main__":
    main()
