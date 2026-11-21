"""Tool-use candidate C: a decision model CHOOSES the tool (typed choice incl. "none"); arguments are filled by deterministic code.

Slot filling assumptions (what kaika really has): the manifest lists the installed package names, versions look like 1.2.3,
backends and cache scopes are fixed vocabularies. Nothing is generated, so nothing can be hallucinated.

    uv run --python 3.13 --with psutil python tum_choice.py --model tev1:0.8b
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402

SOURCES = {"brew": ["homebrew", "brew", "formula"], "cask": ["cask"], "npm": ["npm"], "uv": [" uv "], "mas": ["app store"], "direct": ["vendor", "directly", "straight from"]}


def slot(tool, text, known):
    low = f" {text.lower()} "
    names = [(low.find(n.lower()), n) for n in known if re.search(rf"\b{re.escape(n.lower())}\b", low)]
    name = min(names)[1] if names else None
    args = {}
    if tool in ("manifest_inspect", "pin_version", "unpin", "retry_install") and name:
        args["name"] = name
    if tool == "pin_version":
        m = re.search(r"\d+(?:\.\d+)+", text)
        if m:
            args["version"] = m.group(0)
    if tool == "cleanup_cache":
        args["scope"] = "stage" if re.search(r"unpack|temporar|staging", low) else "all" if re.search(r"everything|all\b", low) else "downloads"
    if tool == "retry_install":
        last = max(((low.rfind(k), s) for s, ks in SOURCES.items() for k in ks if low.rfind(k) >= 0), default=(-1, None))
        if last[1]:
            args["source"] = last[1]
    return {"name": tool, "arguments": args}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", default="tev1:0.8b")
    a = ap.parse_args()
    registry = common.corpus("tool_registry")
    cases = common.corpus("tools")
    known = sorted({v for c in cases if c["expected"] for k, v in c["expected"]["arguments"].items() if k == "name"} | {"Xcode"})
    criteria = {t["name"]: t["description"] for t in registry}
    criteria["none"] = "No tool fits, the request is unsafe, or it is unrelated to package recovery."
    common.ollama_isolate(a.model)

    def ask(text):
        body = {"model": a.model, "state": text, "keep_alive": "15m",
                "questions": {"q": {"type": "choice", "instructions": "Which recovery tool should handle this request?", "criteria": criteria}}}
        return common.http_json("http://127.0.0.1:11434/v1/systemone", body)["answers"]["q"]["choice"]

    _, cold = common.timed(lambda: ask(cases[0]["state"]))
    rows = []
    for c in cases:
        choice, ms = common.timed(lambda c=c: ask(c["state"]))
        got = None if choice == "none" else slot(choice, c["state"], known)
        rows.append({"name": c["name"], "expected": c["expected"], "got": got, "ms": ms, "prose": False, "score": common.score_tool_case(c["expected"], got, registry)})
    common.tool_report(f"{a.model}: decision model picks the tool, code fills the arguments", rows, common.ollama_footprint_mb(),
                       extra={"cold: first call (ms)": round(cold), "ollama ps": common.ollama_ps(a.model)}, save=f"tum_choice_{a.model.replace(':', '_')}")


if __name__ == "__main__":
    main()
