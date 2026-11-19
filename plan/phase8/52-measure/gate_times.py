#!/usr/bin/env python3
"""Gate wall time and mutation stage per gate.json (plan/phase8/reviews/*gate*.json). Usage: python3 -I gate_times.py <repo>"""
import glob, json, os, sys
repo = sys.argv[1]
rows = []
for f in glob.glob(os.path.join(repo, "plan/phase8/reviews/*gate*.json")):
    try:
        d = json.load(open(f))
    except Exception as e:
        continue
    steps = {s.get("name"): s for s in (d.get("steps") or []) if isinstance(s, dict)}
    m = d.get("mutation") or {}
    total = sum(v.get("total", 0) for v in m.get("lists", {}).values()) if isinstance(m.get("lists"), dict) else None
    ms = steps.get("mutation", {}).get("seconds") or m.get("seconds")
    ts = steps.get("test", {}).get("seconds")
    rows.append((d.get("started", "?"), os.path.basename(f), total, ms, ts, m.get("hosts")))
for r in sorted(rows, key=lambda r: str(r[0])):
    print("\t".join(str(x) for x in r))
