#!/usr/bin/env python3
"""Make a bench session's machine-readable results from its raw logs (AGENTS.md §9, Bench records; owner 2026-11-10).

usage: bench_extract.py <raw log dir> <out results dir>

Writes, from every *.log under the raw dir (recursively):
  verdicts.tsv  log, board, verdict, text      one row per `VERDICT …` line of a hardware test (board = the log name's .36/.37)
  cells.csv     board, rate_hz, case, first, len, gap, trials, ok
                one row per grid cell of plan/phase8/tools/uhd-repro/tx_repro's `summary` lines (case sweep or gap); ok counts the
                trials with two BurstAcks and no TimeError or Underflow, the rule `table.py` applies. Per-trial rows stay in the raw logs.
  decisions.tsv log, board, text        one row per decision or summary line: `<NAME> decision …`, `T200 bare summary …`, the bare
                tool's `t200 summary …`, and for a v3 T200 log (`v3-t200-*.log`) one `v3 underflow characters N` row: the `U`s outside UHD's
                bracketed log lines once the words USRP, UHD, UBX, UTC, USB, Using and Unable are removed (v3 counts no async events)
  trials.jsonl  {"log", "board", "kind", "row"} for every `S<n> trial {json}` line (S2, S4: tens of rows a session); S5's grid rows
                (`1ch`, `lead`: thousands) stay in the raw logs, its decision lines here. A row whose JSON UHD's characters cut is counted on stdout
  measurements/<path>.json   for every JSON file of schema `ezsdr.radio.measurement.v1` (`measurement.json`, shake-out records): the
                record without its `observations` (schema, source, profile, selector, device, host, load, commit, conditions, values);
                the observations and the `.rows` files stay in the raw logs.
UHD's console prints `U` and `L` characters anywhere in a line; they are removed first (no summary token contains an `L`, or a `U` not
followed by `n`). Standard library only.
"""
import csv, json, pathlib, re, sys

def clean(line):
    return re.sub(r"U(?!n)", "", line.replace("L", ""))

def verdict_of(text):
    if text.startswith("E9 INCONCLUSIVE:"):  # ssh-drop-tx.sh: no measurement; the E9 measurement lines carry no verdict
        return "INCONCLUSIVE"
    if m := re.search(r": (Pass|Fail|Inconclusive)\(", text):  # the mt3 cells print the Rust `Verdict` with Debug
        return m.group(1).upper()
    return next((v for v in ("PASS", "FAIL", "INCONCLUSIVE") if f": {v}" in text), "")

# The hardware tests' decision names (tests/*.rs `format!("<NAME> decision`), matched on the raw line: UHD's characters in front of a
# name are not part of it, and `clean` would cut the L and U out of the text (INCONCLUSIVE, OVERFLOW).
DECISION = re.compile(r"(?:(?:OFS|P75 [a-z]|RXZPN|RXZ|S\d|T200) decision\b|T200 bare summary\b|t200 summary\b|MT3 C\d (?:cell|boundary|reading|marginal|clear cell)\b).*")
TRIAL = re.compile(r"(S\d) trial (\{.*\})\s*$")

def v3_underflows(text):
    return sum(re.sub(r"USRP|UHD|UBX|UTC|USB|Using|Unable", "", line).count("U") for line in text.splitlines() if not line.startswith("["))

def board_of(name):
    m = re.search(r"[.-](36|37)[a-z]?(?:[.-]|$)", name)
    return m.group(1) if m else ""

def main(raw, out):
    raw, out = pathlib.Path(raw), pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    verdicts, sweeps, decisions, trials, cut = [], [], [], [], 0
    for log in sorted(raw.rglob("*.log")):
        rel, rate = str(log.relative_to(raw)), ""
        board = board_of(log.name)
        text = log.read_text(errors="replace")
        if log.name.startswith("v3-t200-"):
            decisions.append([rel, board, f"v3 underflow characters {v3_underflows(text)}"])
        for line in text.splitlines():
            if m := TRIAL.search(line):
                try:
                    trials.append({"log": rel, "board": board, "kind": m.group(1), "row": json.loads(m.group(2))})
                except ValueError:
                    cut += 1
                continue
            if m := DECISION.search(line) if "VERDICT " not in line else None:
                decisions.append([rel, board, m.group(0)])
            if (i := line.find("VERDICT ")) >= 0:
                text = line[i + len("VERDICT "):]
                verdict = verdict_of(text)
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
    if decisions:
        with open(out / "decisions.tsv", "w", newline="") as fh:
            w = csv.writer(fh, delimiter="\t")
            w.writerow(["log", "board", "text"])
            w.writerows(decisions)
    if trials:
        with open(out / "trials.jsonl", "w") as fh:
            fh.writelines(json.dumps(t, ensure_ascii=False) + "\n" for t in trials)
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
    print(f"{len(verdicts)} verdicts, {len(decisions)} decision lines, {len(trials)} trial rows ({cut} cut by UHD characters), {len(sweeps)} sweep trials in {len(cells)} cells -> {out}")

if __name__ == "__main__":
    assert board_of("hw_x.37.2.log") == "37" and board_of("coarse-sweep-36-200.log") == "36" and board_of("console.log") == ""
    assert verdict_of("E9 INCONCLUSIVE: no tone before the kill (quiet)") == "INCONCLUSIVE"
    assert verdict_of("E9 the tone on the independent receiver ended after the kill: the first quiet reading at +80 ms") == ""
    assert verdict_of("C1 x: PASS: y") == "PASS" and verdict_of("C2 x: FAIL: y") == "FAIL"
    assert verdict_of('mt3 c1 1 MS/s trial 0: Pass("no tone")') == "PASS" and verdict_of('mt3 c4 1 MS/s: Inconclusive("x")') == "INCONCLUSIVE"
    assert DECISION.search("MT3 C4 boundary 1 MS/s: x").group(0) == "MT3 C4 boundary 1 MS/s: x" and not DECISION.search("MT3 C5 trace [{}]")
    assert clean("BurLLstAck=2 ULnderflow=1") == "BurstAck=2 Underflow=1"
    assert DECISION.search("ULLS2 decision tx-200msps: INCONCLUSIVE").group(0) == "S2 decision tx-200msps: INCONCLUSIVE"
    assert DECISION.search("OOFS decision: x").group(0) == "OFS decision: x" and board_of("tx_repro-t200-H0-37r-1.log") == "37"
    assert DECISION.search("P75 a decision: INCONCLUSIVE").group(0) == "P75 a decision: INCONCLUSIVE" and not DECISION.search("no decision here")
    assert DECISION.search("UUUt200 summary rate 200000000").group(0) == "t200 summary rate 200000000"
    assert TRIAL.search('DOOOS2 trial {"a": 1}').groups() == ("S2", '{"a": 1}') and not TRIAL.search('1ch trial {}')
    assert v3_underflows("[INFO] [UHD] USRP\nUsing Device\nUUU\nUPress Ctrl\n") == 4
    main(*sys.argv[1:3])
