"""One comparison table over results/discover_*.json: precision, coverage, wrong answers, latency and memory."""
import glob
import json
import os

rows = []
for f in sorted(glob.glob(os.path.join(os.path.dirname(os.path.abspath(__file__)), "results", "discover_*.json"))):
    d = json.load(open(f))
    ps = d.get("ollama_ps") or {}
    ram = ps.get("size_mb") or ps.get("rss_mb") or d.get("memory_mb")
    tau = lambda t: [(r["conf"], r["top"], r["expected"]) for r in d["rows"] if r.get("top") and r["top"] != "none" and r["conf"] >= t]
    ans = sum(1 for r in d["rows"] if r["expected"])
    best = None
    for t in (0.5, 0.6, 0.7, 0.8, 0.9):                       # highest coverage that still has at most one wrong answer
        acc = tau(t); ok = sum(1 for _, top, exp in acc if exp and top == exp); wrong = len(acc) - ok
        if wrong <= 1 and (best is None or ok > best[1]):
            best = (t, ok, wrong)
    rows.append((d["model"], d["api"], d["precision"], d["coverage"], d["wrong"], best, d["p50_ms"], ram, ans))
rows.sort(key=lambda r: (-(r[5][1] if r[5] else 0), r[7] or 0))
print(f"{'model':34}{'api':10}{'prec@0.6':>9}{'cov@0.6':>9}{'wrong':>6}   best with <=1 wrong      p50 ms  memory MB")
for m, api, p, c, w, best, ms, ram, ans in rows:
    b = f"tau {best[0]:.1f}: {best[1]}/{ans} right, {best[2]} wrong" if best else "none"
    print(f"{m[:33]:34}{api:10}{p:>9.2f}{c:>9.2f}{w:>6}   {b:24}{ms:>7.0f}  {ram or '?':>9}")
