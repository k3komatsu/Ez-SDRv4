#!/usr/bin/env python3
"""Phase 3 mutation checks (plan/phase3/00-overview.md GV-6, 20-implementation-plan.md
Appendix C).

Copies the repository into a scratch directory, and there, for each mutation in the
JSON list: runs the mutation's test command unmutated (it must pass and run at least one
test), replaces `old` by `new` in `file` (the text must occur exactly once), runs
`cargo +stable test -p <crate> --test <test_bin> <filter>` again, expects it to FAIL, and
restores the file. The working tree is never modified.

Usage (from the repository root, after Steps 1-4):
    python3 plan/phase3/tools/mutate.py plan/phase3/tools/mutations.json <scratch-dir> [M01 M02 ...]

The scratch directory must lie outside the repository. If it exists it must be one this
script created (it holds the marker file `.ezsdr-mutation-scratch`); anything else is
refused and nothing is deleted.

Prints one line per mutation, `M01 <name>: killed`, or `SURVIVED`, `NOT FOUND`,
`COMPILE ERROR`, `BASELINE FAILED`, and exits 1 unless every selected mutation is
killed. Standard library only.
"""
import json
import os
import re
import shutil
import subprocess
import sys

IGNORED = {".git", "target", "v3", "tmp", ".cargo"}
MARKER = ".ezsdr-mutation-scratch"


def refuse(message):
    print(f"refused: {message}")
    sys.exit(2)


def prepare_scratch(root, scratch):
    root = os.path.realpath(root)
    parent = os.path.realpath(os.path.dirname(scratch.rstrip(os.sep)) or ".")
    scratch = os.path.join(parent, os.path.basename(scratch.rstrip(os.sep)))
    if os.path.commonpath([root, scratch]) in (root, scratch):
        refuse(f"the scratch directory {scratch} is the repository, inside it, or contains it")
    if os.path.exists(scratch):
        if not os.path.isfile(os.path.join(scratch, MARKER)):
            refuse(f"{scratch} exists and was not created by this script (no {MARKER})")
        shutil.rmtree(scratch)

    def ignore(directory, names):
        if os.path.realpath(directory) != root:
            return []
        return [name for name in names if name in IGNORED]

    shutil.copytree(root, scratch, ignore=ignore)
    open(os.path.join(scratch, MARKER), "w").close()
    return scratch


def cargo_test(scratch, env, mutation):
    command = ["cargo", "+stable", "test", "-q", "-p", mutation["crate"],
               "--test", mutation["test_bin"], mutation["filter"]]
    return subprocess.run(command, cwd=scratch, env=env, capture_output=True, text=True,
                          timeout=900)


def ran_tests(output):
    return sum(int(n) for n in re.findall(r"test result: ok\. (\d+) passed", output)) > 0


BASELINES = {}


def run_one(scratch, env, mutation):
    key = (mutation["crate"], mutation["test_bin"], mutation["filter"])
    if key not in BASELINES:
        baseline = cargo_test(scratch, env, mutation)
        BASELINES[key] = baseline.returncode == 0 and ran_tests(baseline.stdout + baseline.stderr)
    if not BASELINES[key]:
        return "BASELINE FAILED"
    path = os.path.join(scratch, mutation["file"])
    original = open(path, encoding="utf-8").read()
    if original.count(mutation["old"]) != 1:
        return "NOT FOUND"
    open(path, "w", encoding="utf-8").write(original.replace(mutation["old"], mutation["new"]))
    try:
        result = cargo_test(scratch, env, mutation)
        if "could not compile" in result.stdout + result.stderr:
            return "COMPILE ERROR"
        return "killed" if result.returncode != 0 else "SURVIVED"
    except subprocess.TimeoutExpired:
        return "killed (timeout)"
    finally:
        open(path, "w", encoding="utf-8").write(original)


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    root = os.getcwd()
    if not os.path.isfile(os.path.join(root, "Cargo.toml")) or not os.path.isdir(os.path.join(root, "plan", "phase3")):
        refuse("run from the repository root")
    mutations = json.load(open(sys.argv[1], encoding="utf-8"))
    selected = set(sys.argv[3:])
    scratch = prepare_scratch(root, os.path.abspath(sys.argv[2]))
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(scratch, "target"))
    failures = 0
    for mutation in mutations:
        if selected and mutation["id"] not in selected:
            continue
        outcome = run_one(scratch, env, mutation)
        print(f"{mutation['id']} {mutation['name']}: {outcome}", flush=True)
        if not outcome.startswith("killed"):
            failures += 1
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
