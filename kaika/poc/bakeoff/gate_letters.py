"""Baseline: kaika's current method (lettered candidates, one token, two orders) on a general model through Ollama.

    uv run --python 3.13 --with psutil python gate_letters.py risk --model qwen2.5:3b-instruct-q4_K_M
"""
import argparse
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, ".."))
import common  # noqa: E402
import resolver  # noqa: E402

TASK = {"risk": "pick how risky this install plan is", "triage": "pick the most likely cause of this package manager error"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("task", choices=["risk", "triage"])
    ap.add_argument("--model", default=resolver.DEFAULT_MODEL)
    a = ap.parse_args()
    _, vocab, labels = common.TASKS[a.task]
    desc = vocab["a"]["labels"]
    cands = [{"id": k, "text": f"{k}: {v}"} for k, v in desc.items()]
    llm = resolver.Ollama(model=a.model, timeout=120)

    def ask(text):
        r = resolver.ask(llm, TASK[a.task] + "\n" + text, "", cands)
        return None if (r is None or r["index"] is None) else cands[r["index"]]["id"]

    common.ollama_isolate(a.model)
    cases = common.corpus(a.task)
    _, cold = common.timed(lambda: ask(cases[0]["text"]))
    rows = []
    for c in cases:
        pred, ms = common.timed(lambda c=c: ask(c["text"]))
        rows.append({"name": c["name"], "expected": c["expected"], "pred": pred, "ms": ms})
    common.classification_report(f"{a.model} (letters, two orders) on {a.task}", labels, rows, common.ollama_footprint_mb(),
                                 extra={"cold: first call incl. model load (ms)": round(cold), "ollama ps": common.ollama_ps(a.model)}, save=f"gate_letters_{a.model.split(':')[0]}_{a.task}")


if __name__ == "__main__":
    main()
