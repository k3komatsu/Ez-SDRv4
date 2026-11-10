#!/usr/bin/env python3
"""Bench 2026-11-10: rate sweep of the front after a short closed burst (#81/#82), UHD only (tx_repro).
usage: sweep.py <board 37|36> <pass coarse|fine> [rate_MSps ...]
Logs: $REPRO_DIR/repro/<pass>-<case>-<board>-<rate>.log; the tx_repro binary is $REPRO_DIR/build/tx_repro.
RF: bench.md conditions (1 GHz, gain 0, >= 30 dB loopback); the owner confirms the attenuators before every run."""
import os, subprocess, sys, time

ADDR = {"37": "192.168.47.37", "36": "192.168.40.36"}
DIR = os.path.expanduser(os.environ.get("REPRO_DIR", "~/.cache/ezsdr-bench/2026-11-10"))  # holds build/tx_repro and repro/
board, pas = sys.argv[1], sys.argv[2]
rates = [float(r) for r in sys.argv[3:]] or [1, 2, 5, 10, 25, 50, 100, 200]


def us(r, xs, floor=1):
    return {max(floor, round(x * r)) for x in xs}


def grids(r):
    lens = sorted({1, 4, 8, 16, 32} | us(r, [1, 2, 4, 8, 16, 32, 64]))
    gaps = sorted({1, 10} | us(r, [0.5, 1, 2, 3, 5, 7, 10, 15, 20, 30]))
    tail_gaps = sorted({0, 1, 2, 3, 4, 5, 10} | us(r, [0.5, 1, 2, 5, 10, 20], 0))
    return lens, gaps, tail_gaps


def run(case, r, extra, trials):
    log = f"{DIR}/repro/{pas}-{case}-{board}-{r:g}.log"
    cmd = ["docker", "run", "--rm", "--network=host", "--cap-add=SYS_NICE", "--ulimit", "rtprio=99", "-v", f"{DIR}/build:/b", "-w", "/b",
           "ezsdr-v4-dev:uhd4.10", "./tx_repro", "--args", f"addr={ADDR[board]}", "--case", case, "--rate", f"{r * 1e6:.0f}", "--trials", str(trials)] + extra
    for attempt in (1, 2):
        t0 = time.time()
        with open(log, "w") as f:
            rc = subprocess.call(cmd, stdout=f, stderr=subprocess.STDOUT)
        text = open(log, errors="replace").read()
        print(f"{time.strftime('%T')} {pas} {case} board {board} rate {r:g} MS/s rc {rc} {time.time() - t0:.0f}s {log}", flush=True)
        if "PLL failed to lock" in text and attempt == 1:
            os.rename(log, log + ".pll-fail")
            continue
        if rc != 0 or "summary" not in text:
            print(f"STOP: {case} rate {r:g} board {board} rc {rc}; see {log}", flush=True)
            sys.exit(1)
        return


FINE = {  # rate: (lens, gaps, tail gaps); 10 trials per cell (bench 2026-11-10, after the coarse pass)
    1: ([1, 4, 8, 12, 16, 20, 24, 28, 32], list(range(1, 13)), None),
    2: ([1, 4, 8, 12, 16, 24, 32, 48, 64], [1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24], None),
    100: ([1, 4, 32, 1000], list(range(1, 11)), list(range(0, 7))),
    200: ([1, 4, 32, 1000], list(range(1, 11)), list(range(0, 7))),
}
if pas == "fine" and not sys.argv[3:]:
    rates = [1, 2, 100, 200]
for r in rates:
    lens, gaps, tail_gaps = grids(r)
    if pas == "fine":
        lens, gaps, tail = FINE[r]
        run("sweep", r, ["--firsts", "100,1996", "--lens", ",".join(map(str, lens)), "--gaps", ",".join(map(str, gaps))], 10)
        if tail:
            run("gap", r, ["--gaps", ",".join(map(str, tail))], 10)
    if pas == "coarse":
        run("sweep", r, ["--firsts", "100,1996", "--lens", ",".join(map(str, lens)), "--gaps", ",".join(map(str, gaps))], 3)
        run("gap", r, ["--gaps", ",".join(map(str, tail_gaps))], 5)
print("ALL DONE", flush=True)
