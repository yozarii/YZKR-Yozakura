"""Gatekeeper candidate B: a decision model through Ollama's /v1/systemone (typed `choice` question, probabilities).

    uv run --python 3.13 --with psutil python gate_systemone.py risk --model tev1:0.8b
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402

INSTR = {"risk": "How risky is the plan described in the state?", "triage": "What is the most likely cause of the error in the state?"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("task", choices=["risk", "triage"])
    ap.add_argument("--model", default="tev1:0.8b")
    ap.add_argument("--variant", default="a")
    a = ap.parse_args()
    _, vocab, labels = common.TASKS[a.task]
    criteria = vocab[a.variant]["labels"]
    url = "http://127.0.0.1:11434/v1/systemone"

    def ask(text):
        body = {"model": a.model, "state": text, "keep_alive": "15m",
                "questions": {"q": {"type": "choice", "instructions": INSTR[a.task], "criteria": criteria}}}
        r = common.http_json(url, body)["answers"]["q"]
        return r["choice"], r.get("probabilities", {}), r.get("confidence")

    common.ollama_isolate(a.model)
    cases = common.corpus(a.task)
    _, cold = common.timed(lambda: ask(cases[0]["text"]))
    ask(cases[0]["text"])
    rows = []
    for c in cases:
        (choice, probs, conf), ms = common.timed(lambda c=c: ask(c["text"]))
        rows.append({"name": c["name"], "expected": c["expected"], "pred": choice, "ms": ms, "conf": conf})
    fp = common.ollama_footprint_mb()
    common.classification_report(f"{a.model} via /v1/systemone on {a.task}", labels, rows, fp,
                                 extra={"cold: first call incl. model load (ms)": round(cold), "ollama ps": common.ollama_ps(a.model)}, save=f"gate_systemone_{a.model.replace(':', '_')}_{a.task}")


if __name__ == "__main__":
    main()
