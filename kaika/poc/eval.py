#!/usr/bin/env python3
"""Measure the decision model on the labelled corpus (site and download choices: where a model is allowed to decide). Needs a running Ollama; not part of the hermetic tests.

    python3 -I poc/eval.py [--model NAME] [--threshold 0.6]

Reports, per case, the answer, probability and latency, then precision (accepted answers that were right),
coverage (cases answered) and median latency. Exits 1 if precision is below the gate. An abstention on a case
whose correct answer is "none" or on an ambiguous case counts as correct: not guessing is the right behaviour.
"""
import argparse
import json
import os
import statistics
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import resolver  # noqa: E402

GATE_PRECISION = 0.9


def load():
    with open(os.path.join(os.path.dirname(__file__), "corpus", "decisions.json"), encoding="utf-8") as f:
        return json.load(f)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", default=None)
    ap.add_argument("--threshold", type=float, default=0.6)
    a = ap.parse_args()
    llm = resolver.Ollama(model=a.model, timeout=120)
    esc = resolver.Escalation(None)
    llm.probs("warm up", "x", [{"id": "a", "text": "a"}, {"id": "b", "text": "b"}])  # load the model once
    accepted = right = answered = correct_abstain = 0
    times = []
    cases = load()
    for c in cases:
        t = time.time()
        out = resolver.choose(c["task"], c["query"], c["candidates"], {"threshold": a.threshold}, llm, esc)
        dt = time.time() - t
        times.append(dt)
        got, want = out["choice"], c["expected"]
        if got is not None:
            accepted += 1
            right += got == want
            verdict = "ok" if got == want else "WRONG"
        else:
            verdict = "abstained (ok)" if want is None else "abstained (missed)"
            correct_abstain += want is None
        answered += got is not None and want is not None
        print(f'{c["name"]:38} want={str(want):18} got={str(got):18} p={out["confidence"]:.2f} {dt*1000:5.0f}ms  {verdict}')
    precision = right / accepted if accepted else 1.0
    answerable = sum(1 for c in cases if c["expected"] is not None)
    print(f"\nmodel {llm.model}  threshold {a.threshold}")
    print(f"precision {precision:.2f} (gate {GATE_PRECISION})  answered {answered}/{answerable} answerable  correct abstentions {correct_abstain}")
    print(f"latency median {statistics.median(times)*1000:.0f} ms, max {max(times)*1000:.0f} ms")
    sys.exit(0 if precision >= GATE_PRECISION else 1)


if __name__ == "__main__":
    main()
