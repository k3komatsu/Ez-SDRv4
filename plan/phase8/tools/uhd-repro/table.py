#!/usr/bin/env python3
"""Tabulate tx_repro sweep/gap logs: pass = BurstAck=2 with no TimeError or Underflow. usage: table.py <log>..."""
import re, sys
from collections import defaultdict

for path in sys.argv[1:]:
    cells = defaultdict(lambda: [0, 0])
    rate = None
    for line in open(path, errors="replace"):
        line = re.sub(r"U(?!n)", "", line.replace("L", ""))  # UHD's U/L console characters land anywhere; no summary token has L or a U not before n
        if m := re.search(r"tx rate (\d+)", line):
            rate = int(m.group(1)) / 1e6
        if (i := line.find("summary ")) < 0:
            continue
        f = line[i:].split()
        ok = "BurstAck=2" in line and "TimeError" not in line and "Underflow" not in line
        if f[1] == "gap":
            key = ("tail", "-", int(f[2]))
        else:
            if not (m := re.fullmatch(r"sweep-first(\d+)-len(\d+)-gap(-?\d+)", f[1])):
                continue  # a line interleaved with UHD's U/L console characters
            first, length, gap = m.groups()
            key = (f"first{first}", int(length), int(gap))
        cells[key][0] += ok
        cells[key][1] += 1
    print(f"== {path}  rate {rate} MS/s")
    rows = defaultdict(dict)
    for (first, length, gap), (p, n) in cells.items():
        rows[(first, length)][gap] = (p, n)
    for (first, length) in sorted(rows, key=lambda k: (k[0], -1 if k[1] == "-" else k[1])):
        r = rows[(first, length)]
        # first gap at or after which every larger gap passes all trials
        gs = sorted(r)
        ok_from = next((g for j, g in enumerate(gs) if all(r[h][0] == r[h][1] for h in gs[j:])), None)
        us = f"{ok_from / rate:.2f} us" if ok_from is not None and rate else "-"
        print(f"{first:>10} L={length!s:>6} ok_from={ok_from!s:>5} ({us})  " + " ".join(f"g{g}={r[g][0]}/{r[g][1]}" for g in gs))
