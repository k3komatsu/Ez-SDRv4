#!/usr/bin/env python3
"""Run mutation rows from any phase using one matcher and one runner.

Copies the repository into a scratch directory, and there, for each mutation in the
JSON list: runs its test unmutated (it must pass and run at least one test), applies the
`old`/`new` replacement in `file`, runs the test again, expects it to FAIL, then restores
the file. Rust `.rs` anchors match lexer tokens while ignoring whitespace between tokens;
strings remain exact and comments match with internal whitespace collapsed. Other files
use exact substrings. An optional `scope` uniquely names a Rust block by text before its
first following balanced `{ ... }`; it limits both `old` and `old2`. A row may add a second
replacement as `old2` / `new2`. The working tree is never modified. A mutation with a
`python` field
instead runs that unittest id (for example `test_easy_api.EasyApi.test_ea_17_two_channels_round_trip`)
from the scratch copy's `python/tests`, with the interpreter in `EZSDR_PYTHON` (default: this
one) and the scratch copy's `ezsdr-server`, built first.

Usage (from the repository root):
    python3 plan/maintenance/tools/mutate.py plan/maintenance/tools/mutations.json <scratch-dir> [H01 H02 ...]

The scratch directory must lie outside the repository. If it exists it must be one this
script created (it holds the marker file `.ezsdr-mutation-scratch`); anything else is
refused and nothing is deleted.

Prints one line per mutation, `M01 <name>: killed`, or `SURVIVED`, `NOT FOUND[: reason]`,
`COMPILE ERROR`, `BASELINE FAILED`, and exits 1 unless every selected mutation is killed.
A mutation recorded as equivalent carries an `equivalent` field saying why no test
can kill it in this tree; it must survive, and prints `equivalent (survived)` (spec 20,
Appendix A: F24 once VF-4 is in). Standard library only.
"""
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import anchor

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


# A build may take minutes on a cold target; the tests themselves take well under a
# minute (the slowest, 35 s). A mutant that hangs is killed after TEST_TIMEOUT, not after
# the build's allowance (G16 and G21 used to cost 15 minutes each).
BUILD_TIMEOUT, TEST_TIMEOUT = 900, 180


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
    # `test_bin: "lib"` names the crate's own unit tests (`--lib`).
    target = ["--lib"] if mutation["test_bin"] == "lib" else ["--test", mutation["test_bin"]]
    command = ["cargo", "+stable", "test", "-q", "-p", mutation["crate"], *target, mutation["filter"]]
    build = run([*command, "--no-run"], scratch, env, BUILD_TIMEOUT)
    if build.returncode != 0:
        return build
    return run(command, scratch, env, TEST_TIMEOUT)


def python_test(scratch, env, mutation):
    build = subprocess.run(["cargo", "+stable", "build", "-q", "-p", "ezsdr-server"], cwd=scratch,
                           env=env, capture_output=True, text=True, timeout=BUILD_TIMEOUT)
    if build.returncode != 0:
        return build
    server = os.path.join(env["CARGO_TARGET_DIR"], "debug", "ezsdr-server")
    python = os.environ.get("EZSDR_PYTHON", sys.executable)
    command = [python, "-m", "unittest", mutation["python"]]
    run_env = dict(env, EZSDR_SERVER=server, PYTHONDONTWRITEBYTECODE="1")
    return run(command, os.path.join(scratch, "python", "tests"), run_env, TEST_TIMEOUT)


def ran_tests(output):
    cargo = sum(int(n) for n in re.findall(r"test result: ok\. (\d+) passed", output))
    python = sum(int(n) for n in re.findall(r"^Ran (\d+) tests?", output, re.MULTILINE))
    return cargo + (python if "\nOK" in output else 0) > 0


BASELINES = {}
ORIGINALS = {}


def run_one(scratch, env, mutation):
    key = mutation.get("python") or (mutation["crate"], mutation["test_bin"], mutation["filter"])
    if key not in BASELINES:
        baseline = cargo_test(scratch, env, mutation)
        BASELINES[key] = baseline.returncode == 0 and ran_tests(baseline.stdout + baseline.stderr)
    if not BASELINES[key]:
        return "BASELINE FAILED"
    path = os.path.join(scratch, mutation["file"])
    if mutation["file"] not in ORIGINALS:
        ORIGINALS[mutation["file"]] = open(path, encoding="utf-8").read()
    original = ORIGINALS[mutation["file"]]
    try:
        mutated = anchor.apply(original, mutation)
    except anchor.AnchorError as error:
        return f"NOT FOUND: {error}"
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
    if not os.path.isfile(os.path.join(root, "Cargo.toml")) or not os.path.isdir(os.path.join(root, "plan", "maintenance")):
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
        if "equivalent" in mutation:
            # Recorded as equivalent: a kill means the record is wrong, not the code.
            outcome = "equivalent (survived)" if outcome == "SURVIVED" else f"{outcome}, but recorded as equivalent"
            failed = not outcome.startswith("equivalent")
        else:
            failed = not outcome.startswith("killed")
        print(f"{mutation['id']} {mutation['name']}: {outcome}", flush=True)
        if failed:
            failures += 1
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
