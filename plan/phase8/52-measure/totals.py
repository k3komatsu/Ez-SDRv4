#!/usr/bin/env python3
"""Current line totals per crate and category at one revision, plus mutation rows per crate.

Usage (rerun):  python3 -I totals.py <repo> <rev>   e.g. python3 -I totals.py ~/local_work/Ez-SDRv4-n111 4679abc1
Categories as in churn.py (prod, uhdffi, fake, utest, itest, icommon, brain). Lines = physical lines of
the file at <rev> (blank and comment lines included). Mutation rows: the six lists plan/{maintenance,
phase3..phase7}/tools/mutations.json; JSON lines per row = its fields + 2 (the file's one-field-per-line layout).
"""
import importlib.util
import json
import os
import subprocess
import sys
from collections import Counter, defaultdict

REPO, REV = sys.argv[1], sys.argv[2]
sys.argv = ["churn.py", REPO, "x", "y"]
spec = importlib.util.spec_from_file_location("churn", os.path.join(os.path.dirname(os.path.abspath(__file__)), "churn.py"))
ch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ch)

files = subprocess.run(["git", "-C", REPO, "ls-tree", "-r", "--name-only", REV], capture_output=True, text=True).stdout.split()
tot = defaultdict(Counter)
nws = defaultdict(Counter)  # non-whitespace characters: a size that rustfmt's 2026-10-31 reflow does not change
for p in files:
    if not p.startswith("crates/") or not p.endswith(".rs"):
        continue
    crate = p.split("/")[1]
    text = ch.show(REV, p) or ""
    n = text.count("\n")
    pc = ch.path_category(p)
    lines = text.split("\n")[:n]
    if pc is None:
        m = ch.region_map(p, text, REV)[:n]
        for c, l in zip(m, lines):
            tot[crate][c] += 1
            nws[crate][c] += len("".join(l.split()))
    else:
        tot[crate][pc] += n
        nws[crate][pc] += sum(len("".join(l.split())) for l in lines)
for p in files:
    if p.startswith("python/") and p.endswith(".py"):
        text = ch.show(REV, p) or ""
        tot["python"][ch.path_category(p)] += text.count("\n")
        nws["python"][ch.path_category(p)] += len("".join(text.split()))

rows = Counter()
rowlines = Counter()
for lst in ["maintenance", "phase3", "phase4", "phase5", "phase6", "phase7"]:
    src = ch.show(REV, f"plan/{lst}/tools/mutations.json")
    if src is None:
        continue
    data = json.loads(src)
    for r in data:
        crate = r.get("crate") or ("python" if "python" in r else "?")
        rows[crate] += 1
        rowlines[crate] += len(r) + 2

cats = ["prod", "uhdffi", "fake", "utest", "itest", "icommon", "py", "pytest"]
print(f"revision {REV}")
print("| crate | " + " | ".join(cats) + " | tests÷prod | mutation rows | row JSON lines |")
print("|---|" + "---|" * (len(cats) + 3))
grand = Counter()
for crate in sorted(tot, key=lambda c: -sum(tot[c].values())):
    t = tot[crate]
    grand.update(t)
    prodlike = t["prod"] + t["uhdffi"] + t["py"]
    testlike = t["utest"] + t["itest"] + t["icommon"] + t["pytest"]
    ratio = f"{testlike / prodlike:.1f}" if prodlike else "-"
    print(f"| {crate} | " + " | ".join(str(t[c]) for c in cats) + f" | {ratio} | {rows[crate]} | {rowlines[crate]} |")
prodlike = grand["prod"] + grand["uhdffi"] + grand["py"]
testlike = grand["utest"] + grand["itest"] + grand["icommon"] + grand["pytest"]
print("| **total** | " + " | ".join(str(grand[c]) for c in cats) + f" | {testlike / prodlike:.1f} | {sum(rows.values())} | {sum(rowlines.values())} |")
print("brain lines:", grand["brain"])
print()
print("non-whitespace kchars (format-invariant size)")
print("| crate | " + " | ".join(cats) + " | tests÷prod |")
print("|---|" + "---|" * (len(cats) + 1))
g2 = Counter()
for crate in sorted(nws, key=lambda c: -sum(nws[c].values())):
    t = nws[crate]
    g2.update(t)
    pl = t["prod"] + t["uhdffi"] + t["py"]
    tl = t["utest"] + t["itest"] + t["icommon"] + t["pytest"]
    print(f"| {crate} | " + " | ".join(f"{t[c]/1000:.0f}" for c in cats) + f" | {tl/pl:.1f} |" if pl else f"| {crate} | " + " | ".join(f"{t[c]/1000:.0f}" for c in cats) + " | - |")
pl = g2["prod"] + g2["uhdffi"] + g2["py"]
tl = g2["utest"] + g2["itest"] + g2["icommon"] + g2["pytest"]
print("| **total** | " + " | ".join(f"{g2[c]/1000:.0f}" for c in cats) + f" | {tl/pl:.1f} |")
