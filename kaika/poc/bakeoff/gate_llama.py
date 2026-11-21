"""Gatekeeper candidate C: the tool-use model classifies by calling one `report` tool whose argument is an enum of the labels.

    uv run --python 3.13 --with psutil python gate_llama.py triage --gguf PATH
"""
import argparse
import os
import signal
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402

PORT = 8091
INSTR = {"risk": "Judge how risky this install plan is.", "triage": "Find the most likely cause of this package manager error."}
WHAT = {"risk": ("report_risk", "level", "Report how risky the described install plan is. allow = routine and safe; warn = a person should review it first; block = unsafe, must not proceed."),
        "triage": ("report_cause", "cause", "Report the most likely cause of the package manager error. other = anything that is none of the named causes.")}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("task", choices=["risk", "triage"])
    ap.add_argument("--gguf", default=os.path.expanduser("~/.kaika/models/xlam/xLAM-2-1B-fc-r-Q4_K_M.gguf"))
    ap.add_argument("--ctx", type=int, default=4096)
    a = ap.parse_args()
    _, _, labels = common.TASKS[a.task]
    fn, arg, desc = WHAT[a.task]
    tools = [{"type": "function", "function": {"name": fn, "description": desc, "parameters": {"type": "object", "properties": {arg: {"type": "string", "enum": labels}}, "required": [arg]}}}]
    os.makedirs(common.RESULTS, exist_ok=True)
    log = open(os.path.join(common.RESULTS, "llama_gate.log"), "w")
    proc = subprocess.Popen(["llama-server", "-m", a.gguf, "--host", "127.0.0.1", "--port", str(PORT), "-c", str(a.ctx), "-ngl", "99", "-fa", "on", "-ctk", "q8_0", "-ctv", "q8_0", "-np", "1", "--jinja", "--no-webui", "--no-warmup"],
                            stdout=log, stderr=log, start_new_session=True)
    try:
        for _ in range(240):
            try:
                if common.http_json(f"http://127.0.0.1:{PORT}/health", timeout=2).get("status") == "ok":
                    break
            except Exception:
                time.sleep(0.5)

        def ask(text):
            msgs = [{"role": "user", "content": f"{INSTR[a.task]} Use the tool to report it.\n\n{text}"}]
            prompt = common.http_json(f"http://127.0.0.1:{PORT}/apply-template", {"messages": msgs, "tools": tools})["prompt"]
            out = common.http_json(f"http://127.0.0.1:{PORT}/completion", {"prompt": prompt, "n_predict": 64, "temperature": 0, "cache_prompt": True})
            call = common.lenient_tool_call(out.get("content") or "")
            v = (call or {}).get("arguments", {}).get(arg)
            return v if v in labels else None

        cases = common.corpus(a.task)
        _, cold = common.timed(lambda: ask(cases[0]["text"]))
        rows = []
        for c in cases:
            pred, ms = common.timed(lambda c=c: ask(c["text"]))
            rows.append({"name": c["name"], "expected": c["expected"], "pred": pred, "ms": ms})
        import psutil
        common.classification_report(f"xLAM-2-1B as classifier (report tool) on {a.task}", labels, rows, common.footprint_mb(proc.pid),
                                     extra={"resident (RSS) MB": round(psutil.Process(proc.pid).memory_info().rss / 1e6), "cold: first call (ms)": round(cold)}, save=f"gate_llama_xlam_{a.task}")
    finally:
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
        except Exception:
            pass
        log.close()


if __name__ == "__main__":
    main()
