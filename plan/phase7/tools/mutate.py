#!/usr/bin/env python3
"""Phase 7 mutation checks (plan/phase7/00-overview.md GZ-6, 19-amendments.md
Appendix A, design/18-uhd-radio.md §8); Phase 6's tool with its phase directory changed and
`.claude` (a clone's worktrees) left out of the copy.

Copies the repository into a scratch directory, and there, for each mutation in the
JSON list: runs the mutation's test command unmutated (it must pass and run at least one
test), replaces `old` by `new` in `file` (the text must occur exactly once), runs
`cargo +stable test -p <crate> --test <test_bin> <filter>` again, expects it to FAIL, and
restores the file (a mutation may name a second edit in the same file as `old2` / `new2`).
The working tree is never modified. A mutation with a `python` field
instead runs that unittest id (for example `test_easy_api.EasyApi.test_ea_17_two_channels_round_trip`)
from the scratch copy's `python/tests`, with the interpreter in `EZSDR_PYTHON` (default: this
one) and the scratch copy's `ezsdr-server`, built first.

Usage (from the repository root, after Steps 1-5):
    python3 plan/phase7/tools/mutate.py plan/phase7/tools/mutations.json <scratch-dir> [F01 F02 ...]

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
import signal
import subprocess
import sys

IGNORED = {".git", "target", "v3", "tmp", ".cargo", "ezsdr-runs", ".claude"}
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

    # Fresh timestamps (shutil.copy, not copytree's default copy2): Cargo judges freshness
    # by mtime, so a copy that keeps old timestamps reuses whatever another copy last built
    # in the shared target directory, mutated builds included (Phase 5 Review F, P1-4).
    shutil.copytree(root, scratch, ignore=ignore, copy_function=shutil.copy)
    open(os.path.join(scratch, MARKER), "w").close()
    return scratch


def run(command, cwd, env, timeout):
    """`subprocess.run` whose timeout kills the whole process group: cargo's test
    binary, not only cargo, which would leave a hung mutant spinning (Phase 7, G16)."""
    process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, start_new_session=True)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate()
        raise
    return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def cargo_test(scratch, env, mutation):
    if "python" in mutation:
        return python_test(scratch, env, mutation)
    command = ["cargo", "+stable", "test", "-q", "-p", mutation["crate"],
               "--test", mutation["test_bin"], mutation["filter"]]
    return run(command, scratch, env, 900)


def python_test(scratch, env, mutation):
    build = subprocess.run(["cargo", "+stable", "build", "-q", "-p", "ezsdr-server"], cwd=scratch,
                           env=env, capture_output=True, text=True, timeout=900)
    if build.returncode != 0:
        return build
    server = os.path.join(env["CARGO_TARGET_DIR"], "debug", "ezsdr-server")
    python = os.environ.get("EZSDR_PYTHON", sys.executable)
    command = [python, "-m", "unittest", mutation["python"]]
    run_env = dict(env, EZSDR_SERVER=server, PYTHONDONTWRITEBYTECODE="1")
    return run(command, os.path.join(scratch, "python", "tests"), run_env, 900)


def ran_tests(output):
    cargo = sum(int(n) for n in re.findall(r"test result: ok\. (\d+) passed", output))
    python = sum(int(n) for n in re.findall(r"^Ran (\d+) tests?", output, re.MULTILINE))
    return cargo + (python if "\nOK" in output else 0) > 0


BASELINES = {}


def run_one(scratch, env, mutation):
    key = mutation.get("python") or (mutation["crate"], mutation["test_bin"], mutation["filter"])
    if key not in BASELINES:
        baseline = cargo_test(scratch, env, mutation)
        BASELINES[key] = baseline.returncode == 0 and ran_tests(baseline.stdout + baseline.stderr)
    if not BASELINES[key]:
        return "BASELINE FAILED"
    path = os.path.join(scratch, mutation["file"])
    original = open(path, encoding="utf-8").read()
    if original.count(mutation["old"]) != 1:
        return "NOT FOUND"
    mutated = original.replace(mutation["old"], mutation["new"])
    # A mutation that needs a second edit in the same file names it `old2` / `new2`.
    if "old2" in mutation:
        if mutated.count(mutation["old2"]) != 1:
            return "NOT FOUND"
        mutated = mutated.replace(mutation["old2"], mutation["new2"])
    open(path, "w", encoding="utf-8").write(mutated)
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
    if not os.path.isfile(os.path.join(root, "Cargo.toml")) or not os.path.isdir(os.path.join(root, "plan", "phase7")):
        refuse("run from the repository root")
    mutations = json.load(open(sys.argv[1], encoding="utf-8"))
    selected = set(sys.argv[3:])
    scratch = prepare_scratch(root, os.path.abspath(sys.argv[2]))
    # Shared by every review copy (AGENTS.md §7); a per-scratch target cost ~2 GB each.
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.expanduser("~/.cache/cargo-target/Ez-SDRv4-review"))
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
