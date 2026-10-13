#!/usr/bin/env python3
"""Exit (and so notify the orchestrator) when an agent breaks AGENTS.md §9's mutation rule.

Checks every 15 s:
- a local mutate.py process (remote_mutate.py is fine);
- a cargo test / test binary orphaned (ppid 1) or running longer than 20 min;
- a mutation row applied in place in the main tree: its `new` text present and its
  `old` text absent (rows with an empty `new` are skipped).

Run it in the background while an implementer works (no model in it):
    python3 plan/maintenance/tools/mutation_watchdog.py
It prints MUTATION RULE VIOLATION and exits 1 on a breach confirmed 20 s later; it never
exits otherwise, so stop it when the step's gate closes. Standard library only.
"""
import re
import json
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[3]
LISTS = [ROOT / "plan/maintenance/tools/mutations.json"] + sorted(ROOT.glob("plan/phase*/tools/mutations.json"))


def elapsed_s(etime):
    days, _, rest = etime.rpartition("-")
    parts = [int(p) for p in rest.split(":")]
    while len(parts) < 3:
        parts.insert(0, 0)
    h, m, s = parts
    return (int(days) if days else 0) * 86400 + h * 3600 + m * 60 + s


def processes():
    out = subprocess.run(["ps", "-axo", "pid=,ppid=,etime=,command="], capture_output=True, text=True).stdout
    for line in out.splitlines():
        pid, ppid, etime, cmd = line.split(None, 3)
        yield int(pid), int(ppid), elapsed_s(etime), cmd


def process_violations():
    for pid, ppid, secs, cmd in processes():
        if "mutate.py" in cmd and "remote_mutate.py" not in cmd and "mutation_watchdog" not in cmd:
            yield f"local mutate.py (pid {pid}): {cmd[:200]}"
        argv = cmd.split()
        exe = argv[0].rsplit("/", 1)[-1] if argv else ""
        if exe in ("rustfmt", "cargo-fmt") or (exe == "cargo" and "fmt" in argv[1:3]):
            yield f"rustfmt / cargo fmt (pid {pid}): the tree is not rustfmt-formatted: {cmd[:200]}"
        is_test = ("cargo test" in cmd or "/debug/deps/" in cmd) and "ssh " not in cmd
        if is_test and ppid == 1:
            yield f"orphaned test process (pid {pid}, {secs}s): {cmd[:200]}"
        elif is_test and secs > 1200:
            yield f"test running {secs // 60} min (pid {pid}): {cmd[:200]}"


def tree_violations():
    for path in LISTS:
        try:
            rows = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        for row in rows:
            new, old = row.get("new", ""), row.get("old", "")
            if not new.strip() or not old or "file" not in row:
                continue
            try:
                text = (ROOT / row["file"]).read_text()
            except OSError:
                continue
            if new in text and old not in text:
                yield f"mutant {row.get('id')} looks applied in the main tree: {row['file']}"


while True:
    found = list(process_violations()) + list(tree_violations())
    if found:
        time.sleep(20)  # a tree edit in progress can look like a mutant; confirm once
        again = list(process_violations()) + list(tree_violations())
        confirmed = [v for v in found if v in again]
        if confirmed:
            print("MUTATION RULE VIOLATION")
            print("\n".join(confirmed))
            sys.exit(1)
    time.sleep(15)
