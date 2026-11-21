"""Gatekeeper candidate A: ModernBERT-base-zeroshot-v2.0 INT8 through ONNX Runtime (what `ort` would run in Rust).

    uv run --with onnxruntime --with tokenizers --with numpy --with psutil python gate_onnx.py risk --variant a --threads 4
"""
import argparse
import json
import os
import sys
import time

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402

MODEL_DIR = os.path.expanduser("~/.kaika/models/modernbert")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("task", choices=["risk", "triage"])
    ap.add_argument("--variant", default="a")
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--maxlen", type=int, default=256)
    ap.add_argument("--provider", default="cpu", choices=["cpu", "coreml"])
    a = ap.parse_args()

    name, vocab, labels = common.TASKS[a.task]
    v = vocab[a.variant]
    cfg = json.load(open(os.path.join(MODEL_DIR, "config.json")))
    ent = next(int(k) for k, lab in cfg["id2label"].items() if lab.lower().startswith("entail"))

    t0 = time.perf_counter()
    so = ort.SessionOptions()
    so.intra_op_num_threads = a.threads
    so.inter_op_num_threads = 1
    so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    providers = ["CoreMLExecutionProvider", "CPUExecutionProvider"] if a.provider == "coreml" else ["CPUExecutionProvider"]
    sess = ort.InferenceSession(os.path.join(MODEL_DIR, "model_int8.onnx"), so, providers=providers)
    init_ms = (time.perf_counter() - t0) * 1000
    tok = Tokenizer.from_file(os.path.join(MODEL_DIR, "tokenizer.json"))
    tok.enable_truncation(max_length=a.maxlen)
    tok.enable_padding(pad_id=tok.token_to_id("[PAD]"), pad_token="[PAD]")
    inputs = {i.name for i in sess.get_inputs()}

    def score(text):
        pairs = [(text, v["template"].format(desc)) for desc in v["labels"].values()]
        enc = tok.encode_batch(pairs)
        feed = {"input_ids": np.array([e.ids for e in enc], dtype=np.int64), "attention_mask": np.array([e.attention_mask for e in enc], dtype=np.int64)}
        logits = sess.run(None, {k: val for k, val in feed.items() if k in inputs})[0]
        z = np.exp(logits - logits.max(axis=1, keepdims=True))
        p = (z / z.sum(axis=1, keepdims=True))[:, ent]
        return dict(zip(v["labels"].keys(), p.tolist())), feed["input_ids"].shape

    cases = common.corpus(a.task)
    first, first_ms = common.timed(lambda: score(cases[0]["text"]))
    for _ in range(3):
        score(cases[0]["text"])
    rows, tokens = [], []
    for c in cases:
        (scores, shape), ms = common.timed(lambda c=c: score(c["text"]))
        tokens.append(shape[1])
        rows.append({"name": c["name"], "expected": c["expected"], "pred": max(scores, key=scores.get), "ms": ms, "scores": {k: round(x, 3) for k, x in scores.items()}})
    fp = common.footprint_mb(os.getpid())
    common.classification_report(
        f"ModernBERT INT8 / ONNX Runtime ({a.provider}, {a.threads} threads) on {a.task}, variant {a.variant}", labels, rows, fp,
        extra={"cold: session init (ms)": round(init_ms), "cold: first inference (ms)": round(first_ms), "hypotheses per decision": len(labels), "padded tokens per hypothesis (max)": max(tokens)},
        save=f"gate_onnx_{a.task}_{a.variant}_{a.provider}{a.threads}")


if __name__ == "__main__":
    main()
