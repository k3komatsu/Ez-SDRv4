#!/usr/bin/env python3
"""Mutation rows by what they mutate (category of the mutated line, as churn.py classifies it) and by list.
Usage (rerun): python3 -I rows_by_target.py <repo> <rev>"""
import importlib.util, json, os, sys
from collections import Counter, defaultdict
REPO, REV = sys.argv[1], sys.argv[2]
sys.argv = ["churn.py", REPO, "x", "y"]
spec = importlib.util.spec_from_file_location("churn", os.path.join(os.path.dirname(os.path.abspath(__file__)), "churn.py"))
ch = importlib.util.module_from_spec(spec); spec.loader.exec_module(ch)
by = Counter(); bylist = defaultdict(Counter); byfile = Counter(); bycrate = defaultdict(Counter)
for lst in ["maintenance", "phase3", "phase4", "phase5", "phase6", "phase7"]:
    for r in json.loads(ch.show(REV, f"plan/{lst}/tools/mutations.json")):
        f = r["file"]
        text = ch.show(REV, f) or ""
        pc = ch.path_category(f)
        if pc is None:
            # locate the anchor's first token line roughly: first line containing the anchor's first 25 chars
            key = r["old"].strip().split("\n")[0][:25]
            idx = next((i for i, l in enumerate(text.split("\n")) if key and key in l), None)
            cmap = ch.region_map(f, text, REV)
            pc = cmap[idx] if idx is not None else "prod?"
        by[pc] += 1; bylist[lst][pc] += 1; byfile[f] += 1
        bycrate[r.get("crate") or "python"][pc] += 1
print("rows by mutated category:", dict(by), "total", sum(by.values()))
for l, c in bylist.items(): print(" ", l, dict(c))
print("by crate:")
for cr, c in sorted(bycrate.items(), key=lambda x: -sum(x[1].values())): print(" ", cr, dict(c))
print("top files:")
for f, n in byfile.most_common(25): print(f"  {n:5d} {f}")
