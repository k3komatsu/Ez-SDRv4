#!/usr/bin/env python3
"""Rows of the standard count tables (| Source | Major | Minor | ... |) in plan/phase8/reviews/*-dispositions*.md.
Usage: python3 -I review_counts.py <repo>  -> TSV: step, source, major, minor, mechanism-or-note (raw cells)"""
import glob, os, re, sys
repo = sys.argv[1]
for f in sorted(glob.glob(os.path.join(repo, "plan/phase8/reviews/*-dispositions*.md"))):
    step = os.path.basename(f).split("-dispositions")[0]
    lines = open(f, encoding="utf-8").read().split("\n")
    i = 0
    while i < len(lines):
        h = [c.strip().lower() for c in lines[i].strip().strip("|").split("|")]
        if lines[i].startswith("|") and len(h) >= 3 and h[0] == "source" and "major" in h:
            mi, mn = h.index("major"), h.index("minor") if "minor" in h else None
            i += 2
            while i < len(lines) and lines[i].startswith("|"):
                c = [x.strip() for x in lines[i].strip().strip("|").split("|")]
                def num(x):
                    m = re.match(r"^\D*?(\d+)", x or "")
                    return int(m.group(1)) if m else (0 if (x or "").strip() in ("0", "—", "-", "", "none") else x)
                print("\t".join([step, c[0], str(num(c[mi])), str(num(c[mn])) if mn is not None and mn < len(c) else "", " / ".join(c[max(mi, mn or 0) + 1:])[:160]]))
                i += 1
            continue
        i += 1
