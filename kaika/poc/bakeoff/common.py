"""Shared pieces of the bake-off: corpora, memory measurement, scoring, reports.

Memory is measured with macOS `footprint`, which counts Metal and mapped memory that plain RSS misses.
"""
import json
import os
import re
import subprocess
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
RESULTS = os.path.join(HERE, "results")
SEVERITY = {"allow": 0, "warn": 1, "block": 2}


def corpus(name):
    with open(os.path.join(HERE, "corpus", f"{name}.json"), encoding="utf-8") as f:
        return json.load(f)


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round((len(xs) - 1) * p)))] if xs else 0.0


def footprint_mb(pid):
    """Physical footprint of a process in MB, or None."""
    try:
        out = subprocess.run(["footprint", "-p", str(pid)], capture_output=True, text=True, timeout=30).stdout
    except Exception:
        return None
    m = re.search(r"Footprint:\s*([\d.]+)\s*(KB|MB|GB)", out)
    if not m:
        return None
    return float(m.group(1)) * {"KB": 1 / 1024, "MB": 1, "GB": 1024}[m.group(2)]


def ollama_footprint_mb():
    """Sum of the footprints of every Ollama process (server plus model runners)."""
    import psutil
    total, parts = 0.0, 0
    for p in psutil.process_iter(["name", "cmdline"]):
        try:
            name = (p.info["name"] or "").lower()
            cmd = " ".join(p.info["cmdline"] or []).lower()
        except Exception:
            continue
        if "ollama" in name or "ollama" in cmd.split(" ")[0]:
            fp = footprint_mb(p.pid)
            if fp:
                total += fp
                parts += 1
    return total if parts else None


def ollama_isolate(keep):
    """Unload every Ollama model except `keep`, so footprint numbers describe one model only."""
    for m in http_json("http://127.0.0.1:11434/api/ps", timeout=10).get("models", []):
        if m["name"] != keep and m.get("model") != keep:
            http_json("http://127.0.0.1:11434/api/generate", {"model": m["name"], "keep_alive": 0}, timeout=60)
    time.sleep(1.5)


def ollama_ps(model):
    for m in http_json("http://127.0.0.1:11434/api/ps", timeout=10).get("models", []):
        if m["name"] == model or m.get("model") == model:
            return {"size_mb": round(m["size"] / 1e6), "vram_mb": round(m.get("size_vram", 0) / 1e6), "context": m.get("context_length")}
    return None


def http_json(url, body=None, timeout=180):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data, {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def timed(fn):
    t = time.perf_counter()
    out = fn()
    return out, (time.perf_counter() - t) * 1000


def classification_report(title, labels, rows, mem_mb, extra=None, save=None):
    """rows: [{name, expected, pred (None = abstained), ms}]"""
    n = len(rows)
    right = sum(r["pred"] == r["expected"] for r in rows)
    ms = [r["ms"] for r in rows]
    print(f"\n=== {title}")
    print(f"accuracy {right}/{n} = {right / n:.2f}   abstained {sum(r['pred'] is None for r in rows)}")
    for lab in labels:
        sub = [r for r in rows if r["expected"] == lab]
        if sub:
            print(f"  recall {lab:11} {sum(r['pred'] == lab for r in sub)}/{len(sub)}")
    print("  confusion (rows = expected, cols = predicted):")
    cols = labels + ["(none)"]
    print("   " + " " * 12 + "".join(f"{c[:8]:>9}" for c in cols))
    for lab in labels:
        cells = [sum(1 for r in rows if r["expected"] == lab and (r["pred"] or "(none)") == c) for c in cols]
        print("   " + f"{lab:12}" + "".join(f"{c:>9}" for c in cells))
    summary = {"title": title, "accuracy": right / n, "n": n, "latency_p50_ms": pct(ms, 0.5), "latency_p95_ms": pct(ms, 0.95), "memory_mb": mem_mb}
    if set(labels) == set(SEVERITY):
        under = [r for r in rows if r["pred"] in SEVERITY and SEVERITY[r["pred"]] < SEVERITY[r["expected"]]]
        danger = [r for r in rows if r["expected"] == "block" and r["pred"] == "allow"]
        over = [r for r in rows if r["pred"] in SEVERITY and SEVERITY[r["pred"]] > SEVERITY[r["expected"]]]
        print(f"  under-called risk {len(under)}   block called allow {len(danger)}   over-called {len(over)}")
        summary.update(under_called=len(under), block_called_allow=len(danger), over_called=len(over))
    print(f"  latency p50 {summary['latency_p50_ms']:.0f} ms   p95 {summary['latency_p95_ms']:.0f} ms   memory {mem_mb:.0f} MB" if mem_mb else f"  latency p50 {summary['latency_p50_ms']:.0f} ms   p95 {summary['latency_p95_ms']:.0f} ms")
    for k, v in (extra or {}).items():
        print(f"  {k}: {v}")
        summary[k] = v
    wrong = [r for r in rows if r["pred"] != r["expected"]]
    if wrong:
        print("  misses:")
        for r in wrong[:12]:
            print(f"    - {r['name']}: expected {r['expected']}, got {r['pred']}")
    if save:
        os.makedirs(RESULTS, exist_ok=True)
        with open(os.path.join(RESULTS, f"{save}.json"), "w") as f:
            json.dump({"summary": summary, "rows": rows}, f, indent=1)
    return summary


# ---------- risk and triage vocabularies shared by every gatekeeper candidate ----------

RISK = {
    "a": {"template": "This plan is {}.",
          "labels": {"allow": "routine and safe to proceed", "warn": "something the user should review before proceeding", "block": "unsafe and must not proceed"}},
    "b": {"template": "The risk of this plan is {}.",
          "labels": {"allow": "low", "warn": "moderate", "block": "severe"}},
}
RISK["c"] = {"template": "{}",
             "labels": {"allow": "Nothing is wrong with this plan.", "warn": "Something about this plan needs a person's attention.",
                        "block": "This plan is dangerous or failed a safety check."}}
TRIAGE = {
    "a": {"template": "The cause of this error is {}.",
          "labels": {"network": "a network or connection problem", "permission": "a file permission or access problem",
                     "disk_full": "the disk being full", "checksum": "a checksum or integrity mismatch",
                     "conflict": "an existing file or a conflicting package", "not_found": "a package that does not exist or is misspelled",
                     "other": "something else, such as a broken toolchain"}},
    "b": {"template": "{}.",
          "labels": {"network": "Network failure", "permission": "Permission denied", "disk_full": "No space left on device",
                     "checksum": "Checksum mismatch", "conflict": "File or package conflict", "not_found": "Package not found",
                     "other": "Other problem"}},
}
TASKS = {"risk": ("risk", RISK, ["allow", "warn", "block"]), "triage": ("triage", TRIAGE, ["network", "permission", "disk_full", "checksum", "conflict", "not_found", "other"])}

TOOL_SYSTEM = ("You are the recovery assistant of a package manager. If one of the provided tools fits the request, "
               "call exactly that tool with exact argument values. If none fits, or the request is unsafe, call no tool. "
               "Never write prose.")


def lenient_tool_call(text):
    """Pull one {name, arguments} out of free text, for models that print the call instead of returning it structured."""
    if not text:
        return None
    for m in re.finditer(r"[\[{]", text):
        depth, start = 0, m.start()
        for i in range(start, len(text)):
            if text[i] in "[{":
                depth += 1
            elif text[i] in "]}":
                depth -= 1
                if depth == 0:
                    try:
                        obj = json.loads(text[start:i + 1])
                    except Exception:
                        break
                    if isinstance(obj, dict) and "tool_calls" in obj:
                        obj = obj["tool_calls"]
                    if isinstance(obj, list) and obj:
                        obj = obj[0]
                    if isinstance(obj, dict) and "function" in obj:
                        obj = obj["function"]
                    if isinstance(obj, dict) and "name" in obj:
                        args = obj.get("arguments", obj.get("parameters", {}))
                        if isinstance(args, str):
                            try:
                                args = json.loads(args)
                            except Exception:
                                args = {}
                        return {"name": obj["name"], "arguments": args or {}}
                    break
    return None


def norm_args(a):
    return {k: str(v).strip().lower() for k, v in (a or {}).items()}


def score_tool_case(expected, got, registry):
    """-> dict of booleans for one case."""
    names = {t["name"] for t in registry}
    out = {"expected_none": expected is None, "called": got is not None}
    out["hallucinated"] = bool(got) and got["name"] not in names
    if expected is None:
        out["correct_name"] = out["correct_call"] = got is None
        out["valid_args"] = True
    else:
        out["correct_name"] = bool(got) and got["name"] == expected["name"]
        out["correct_call"] = out["correct_name"] and norm_args(got["arguments"]) == norm_args(expected["arguments"])
        spec = next((t for t in registry if got and t["name"] == got["name"]), None)
        valid = True
        if spec:
            props = spec["parameters"]["properties"]
            for k, v in (got["arguments"] or {}).items():
                if k not in props or ("enum" in props[k] and v not in props[k]["enum"]):
                    valid = False
            valid = valid and all(r in (got["arguments"] or {}) for r in spec["parameters"].get("required", []))
        out["valid_args"] = valid if got else False
    return out


def tool_report(title, rows, mem_mb, extra=None, save=None):
    n = len(rows)
    ms = [r["ms"] for r in rows]
    s = lambda key, pred=lambda r: True: sum(1 for r in rows if pred(r) and r["score"][key])
    use = [r for r in rows if not r["score"]["expected_none"]]
    none = [r for r in rows if r["score"]["expected_none"]]
    print(f"\n=== {title}")
    print(f"full call correct {s('correct_call')}/{n} = {s('correct_call') / n:.2f}")
    print(f"  tool chosen correctly {sum(r['score']['correct_name'] for r in use)}/{len(use)}   exact arguments {sum(r['score']['correct_call'] for r in use)}/{len(use)}")
    print(f"  correctly declined (no tool) {sum(r['score']['correct_call'] for r in none)}/{len(none)}")
    print(f"  hallucinated tool names {sum(r['score']['hallucinated'] for r in rows)}   invalid arguments {sum(1 for r in rows if r['score']['called'] and not r['score']['valid_args'])}   prose replies {sum(1 for r in rows if r.get('prose'))}")
    print(f"  latency p50 {pct(ms, 0.5):.0f} ms   p95 {pct(ms, 0.95):.0f} ms" + (f"   memory {mem_mb:.0f} MB" if mem_mb else ""))
    for k, v in (extra or {}).items():
        print(f"  {k}: {v}")
    bad = [r for r in rows if not r["score"]["correct_call"]]
    if bad:
        print("  misses:")
        for r in bad[:10]:
            print(f"    - {r['name']}: expected {r['expected']}, got {r['got']}")
    summary = {"title": title, "full_call_accuracy": s("correct_call") / n, "latency_p50_ms": pct(ms, 0.5), "latency_p95_ms": pct(ms, 0.95), "memory_mb": mem_mb,
               "hallucinated": sum(r["score"]["hallucinated"] for r in rows), "prose": sum(1 for r in rows if r.get("prose"))}
    if save:
        os.makedirs(RESULTS, exist_ok=True)
        with open(os.path.join(RESULTS, f"{save}.json"), "w") as f:
            json.dump({"summary": summary, "rows": rows}, f, indent=1)
    return summary
