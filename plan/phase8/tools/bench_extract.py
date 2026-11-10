#!/usr/bin/env python3
"""Make a bench session's machine-readable results from its raw logs (AGENTS.md §9, Bench records; owner 2026-11-10).

usage: bench_extract.py <raw log dir> <out results dir>

Writes, from every *.log under the raw dir (recursively):
  verdicts.tsv  log, board, verdict, text      one row per `VERDICT …` line of a hardware test (board = the log name's .36/.37)
  cells.csv     board, rate_hz, case, first, len, gap, trials, ok
                one row per grid cell of plan/phase8/tools/uhd-repro/tx_repro's `summary` lines (case sweep or gap); ok counts the
                trials with two BurstAcks and no TimeError or Underflow, the rule `table.py` applies. Per-trial rows stay in the raw logs.
  measurements/<path>.json   for every JSON file of schema `ezsdr.radio.measurement.v1` (`measurement.json`, shake-out records): the
                record without its `observations` (schema, source, profile, selector, device, host, load, commit, conditions, values);
                the observations and the `.rows` files stay in the raw logs.
UHD's console prints `U` and `L` characters anywhere in a line; they are removed first (no summary token contains an `L`, or a `U` not
followed by `n`). Standard library only.
"""
import csv, json, pathlib, re, sys

def clean(line):
    return re.sub(r"U(?!n)", "", line.replace("L", ""))

def board_of(name):
    m = re.search(r"[.-](36|37)(?:[.-]|$)", name)
    return m.group(1) if m else ""

def main(raw, out):
    raw, out = pathlib.Path(raw), pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    verdicts, sweeps = [], []
    for log in sorted(raw.rglob("*.log")):
        rel, rate = str(log.relative_to(raw)), ""
        board = board_of(log.name)
        for line in log.read_text(errors="replace").splitlines():
            if (i := line.find("VERDICT ")) >= 0:
                text = line[i + len("VERDICT "):]
                verdict = next((v for v in ("PASS", "FAIL", "INCONCLUSIVE") if f": {v}" in text), "")
                verdicts.append([rel, board, verdict, text])
                continue
            c = clean(line)
            if m := re.search(r"tx rate (\d+)", c):
                rate = m.group(1)
            if (i := c.find("summary ")) < 0:
                continue
            f = c[i:].split()
            counts = dict(re.findall(r"(BurstAck|TimeError|Underflow)=(\d+)", c))
            ack, te, un = (int(counts.get(k, 0)) for k in ("BurstAck", "TimeError", "Underflow"))
            ok = int(ack == 2 and te == 0 and un == 0)
            if f[1] == "gap" and len(f) > 3:
                sweeps.append((board, rate, "gap", "", "", f[2], ok))
            elif m := re.fullmatch(r"sweep-first(\d+)-len(\d+)-gap(-?\d+)", f[1]):
                sweeps.append((board, rate, "sweep", *m.groups(), ok))
    for js in sorted(raw.rglob("*.json")):
        try:
            with open(js) as fh:
                head = fh.read(200)
                if '"ezsdr.radio.measurement.v1"' not in head:
                    continue
                fh.seek(0)
                record = json.load(fh)
        except (OSError, ValueError):
            continue
        record.pop("observations", None)
        target = out / "measurements" / (str(js.relative_to(raw)).replace("/", "__"))
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(record, ensure_ascii=False, indent=1) + "\n")
    with open(out / "verdicts.tsv", "w", newline="") as fh:
        w = csv.writer(fh, delimiter="\t")
        w.writerow(["log", "board", "verdict", "text"])
        w.writerows(verdicts)
    cells = {}
    for *key, ok in sweeps:
        n, k = cells.get(tuple(key), (0, 0))
        cells[tuple(key)] = (n + 1, k + ok)
    if cells:
        num = lambda v: int(v) if v.lstrip("-").isdigit() else -1
        with open(out / "cells.csv", "w", newline="") as fh:
            w = csv.writer(fh)
            w.writerow(["board", "rate_hz", "case", "first", "len", "gap", "trials", "ok"])
            for key in sorted(cells, key=lambda k: (k[0], num(k[1]), k[2], num(k[3]), num(k[4]), num(k[5]))):
                w.writerow([*key, *cells[key]])
    print(f"{len(verdicts)} verdicts, {len(sweeps)} sweep trials in {len(cells)} cells -> {out}")

if __name__ == "__main__":
    assert board_of("hw_x.37.2.log") == "37" and board_of("coarse-sweep-36-200.log") == "36" and board_of("console.log") == ""
    assert clean("BurLLstAck=2 ULnderflow=1") == "BurstAck=2 Underflow=1"
    main(*sys.argv[1:3])
