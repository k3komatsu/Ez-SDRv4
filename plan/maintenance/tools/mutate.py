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

Prints per mutation a line `M01 <name>: killed` (or `killed (bound)`, `SURVIVED`, `ERROR`,
`NOT FOUND[: reason]`, `COMPILE ERROR`, `BASELINE FAILED`) and, first, the line a runner reads:
`EZSDR-ROW {"id": ..., "outcome": ..., "real_clock": ..., "reruns": ..., "why": ...}` (`why` only for `ERROR` and `killed (bound)`: `build timeout`, `test timeout`,
`infra marker`, `signal`, `no test result line`, `no test ran`, or `re-run unsettled` for two timing-marked re-runs; otherwise the decisive re-run's own reason; a fixed field, so a row's
name cannot be read as a result: #91). Exits 1 unless every selected mutation is killed.
A mutation recorded as equivalent carries an `equivalent` field saying why no test
can kill it in this tree; it must survive, and prints `equivalent (survived)` (spec 20,
Appendix A: F24 once VF-4 is in).

A row is `killed` only when the output shows a test failure (`test result: FAILED`, Python
`FAILED (`) with no `EZSDR-INFRA:` marker and no Cargo `(signal:`. A marker, a test timeout, a
signal, a failure with no `test result:` line, and a build timeout are `ERROR`: the mutant was not
shown to be caught. A row with `"kill": "bound"` declares that its only kill is a real-time bound; the
marker and the test timeout then give `killed (bound)`, which the gate counts. A failure whose tests all
carry an `EZSDR-TIMING:` marker (matched by name, not by count) and where nothing else went wrong (no infra marker, no
signal, no build error) is re-run one at a time while the mutant is applied (at most twice each): a re-run that fails
without the marker is conclusive and kills, a marker-free pass does not, two markers are `ERROR`; any other re-run
result is kept and stops the attempts.
The baseline of each row runs with `EZSDR_REAL_CLOCK_PROBE` set and `--show-output`, to report the rows
that select a real-clock FakeDevice test (`real_clock`).

`--self-test` checks the verdict table and the re-run rule on strings. `--e2e <scratch-dir>` runs the
`infra_fixture_*` tests of `crates/ezsdr-radio-uhd/tests/infra_fixtures/` through the same `cargo_test()`
and prints `EZSDR-E2E {...}` per case; run it through `remote_mutate.py --e2e`, in the mutation image.
Standard library only.
"""
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
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


class Ran:
    """What one `cargo_test` saw: the exit code (None after a timeout), the output of both streams,
    and which timeout fired (`"build"` or `"test"`)."""

    def __init__(self, returncode, output, timeout=None):
        self.returncode, self.output, self.timeout = returncode, output, timeout


def timed(command, cwd, env, timeout, phase):
    try:
        done = run(command, cwd, env, timeout)
    except subprocess.TimeoutExpired:
        return Ran(None, "", phase)
    return Ran(done.returncode, done.stdout + done.stderr)


def cargo_test(scratch, env, mutation, show_output=False):
    if "python" in mutation:
        return python_test(scratch, env, mutation)
    # `test_bin: "lib"` names the crate's own unit tests (`--lib`).
    target = ["--lib"] if mutation["test_bin"] == "lib" else ["--test", mutation["test_bin"]]
    command = ["cargo", "+stable", "test", "-q", "-p", mutation["crate"], *target, mutation["filter"]]
    build = timed([*command, "--no-run"], scratch, env, BUILD_TIMEOUT, "build")
    if build.timeout or build.returncode != 0:
        return build
    # A serial, exact re-run of one test (the `EZSDR-TIMING:` rule) goes through here too.
    flags = (["--exact", "--test-threads=1"] if mutation.get("exact") else []) + (["--show-output"] if show_output else [])
    return timed([*command, *(["--", *flags] if flags else [])], scratch, env, TEST_TIMEOUT, "test")


def python_test(scratch, env, mutation):
    try:
        build = subprocess.run(["cargo", "+stable", "build", "-q", "-p", "ezsdr-server"], cwd=scratch,
                               env=env, capture_output=True, text=True, timeout=BUILD_TIMEOUT)
    except subprocess.TimeoutExpired:
        return Ran(None, "", "build")
    if build.returncode != 0:
        return Ran(build.returncode, build.stdout + build.stderr)
    server = os.path.join(env["CARGO_TARGET_DIR"], "debug", "ezsdr-server")
    python = os.environ.get("EZSDR_PYTHON", sys.executable)
    command = [python, "-m", "unittest", mutation["python"]]
    run_env = dict(env, EZSDR_SERVER=server, PYTHONDONTWRITEBYTECODE="1")
    return timed(command, os.path.join(scratch, "python", "tests"), run_env, TEST_TIMEOUT, "test")


def ran_tests(output):
    cargo = sum(int(n) for n in re.findall(r"test result: ok\. (\d+) passed", output))
    python = sum(int(n) for n in re.findall(r"^Ran (\d+) tests?", output, re.MULTILINE))
    return cargo + (python if "\nOK" in output else 0) > 0


INFRA, SIGNAL = "EZSDR-INFRA:", "(signal:"
FAILURE_LINE = re.compile(r"test result: FAILED|^FAILED \(", re.MULTILINE)
TIMING = re.compile(r"EZSDR-TIMING: (\S+) (\S+) (\S+): ")
REAL_CLOCK = "EZSDR-CLASS: real-clock"
RERUNS = 2


def verdict(ran, bound=False):
    """The outcome of one test invocation, from its output (note 51 §6.3). Never `killed` for anything
    but a test failure that no marker and no signal explains."""
    if ran.timeout == "build":
        return "ERROR"
    if "could not compile" in ran.output:
        return "COMPILE ERROR"
    if ran.timeout == "test" or INFRA in ran.output:
        return "killed (bound)" if bound else "ERROR"
    if ran.returncode == 0:
        return "SURVIVED"
    return "killed" if FAILURE_LINE.search(ran.output) and SIGNAL not in ran.output else "ERROR"


def why(ran, outcome):
    """Why a row is `ERROR` or `killed (bound)`, for the row line (empty otherwise)."""
    if outcome not in ("ERROR", "killed (bound)"):
        return ""
    return ("build timeout" if ran.timeout == "build" else "test timeout" if ran.timeout == "test" else "infra marker" if INFRA in ran.output
            else "signal" if SIGNAL in ran.output else "no test ran" if ran.returncode == 0 else "no test result line")


def timing_tests(output):
    """The `(package, test binary, test path)` of each `EZSDR-TIMING:` marker, once, in order."""
    return list(dict.fromkeys(TIMING.findall(output)))


def failed_count(output):
    return sum(int(n) for n in re.findall(r"test result: FAILED\. \d+ passed; (\d+) failed", output))


CARGO_FAILED = re.compile(r"^error: test failed, to rerun pass `-p (\S+) ([^`]*)`\n", re.MULTILINE)


def failed_tests(output):
    """The `(package, test binary, test path)` of the failed tests, from `test x ... FAILED` lines and from the `failures:` list
    of a quiet run. A binary's package and name come from cargo's `error: test failed, to rerun pass` line that closes its
    output (`--test b` gives `b`, `--lib` gives `--lib`); a failure after the last such line has `None` for both, so no marker matches it."""
    parts = CARGO_FAILED.split(output)
    owners = [(package, target.removeprefix("--test ")) for package, target in zip(parts[1::3], parts[2::3])] + [(None, None)]
    failed = set()
    for text, (package, binary) in zip(parts[0::3], owners):
        listed = [name for block in re.findall(r"^failures:\n((?:    \S+\n)+)\n", text, re.MULTILINE) for name in block.split()]
        failed |= {(package, binary, name) for name in set(re.findall(r"^test (\S+) \.\.\. FAILED$", text, re.MULTILINE)) | set(listed)}
    return failed


def rerunnable(output):
    """The timing-marked tests to re-run, or `[]` when the failure is not wholly timing (note 51 Q16 b''): every failed test
    must be named and carry a marker for its own package, binary and path, and nothing else may be wrong anywhere in the output: no
    infra marker, no signal (another test binary's included), no build error, no test binary that ended without a `test result: FAILED` line."""
    names, marked = failed_tests(output), timing_tests(output)
    if (not names or failed_count(output) > len(names) or not names <= set(marked) or INFRA in output or SIGNAL in output
            or "could not compile" in output or output.count("error: test failed") != output.count("test result: FAILED")):
        return []
    return [test for test in marked if test in names]


def settle_test(test, rerun, bound=False):
    """Re-runs one timing-marked test alone, at most `RERUNS` times, and returns `(result, last Ran, attempts)`. `result` is
    `SURVIVED` for a marker-free pass, `unsettled` when every attempt was timing-marked, otherwise the first conclusive
    `verdict` of an attempt (a failure without the marker, a signal, a timeout, an infra marker ...), which stops at once."""
    for attempt in range(1, RERUNS + 1):
        again = rerun(*test)
        result = verdict(again, bound)
        if result == "SURVIVED":
            if not ran_tests(again.output):
                return "ERROR", again, attempt
            if not timing_tests(again.output):
                return "SURVIVED", again, attempt
        elif not (result == "killed" and timing_tests(again.output)):
            return result, again, attempt
    return "unsettled", again, RERUNS


def judge(ran, bound, rerun):
    """`(outcome, number of re-runs, why)`: `verdict`, then the re-run rule for a failure whose tests all carry a timing marker.
    `rerun(package, binary, path)` runs one test alone and returns a `Ran`."""
    outcome = verdict(ran, bound)
    tests = rerunnable(ran.output) if outcome == "killed" else []
    reruns, unsettled = 0, False
    for test in tests:
        result, again, attempts = settle_test(test, rerun, bound)
        reruns += attempts
        if result == "unsettled":
            unsettled = True
        elif result != "SURVIVED":
            return result, reruns, why(again, result)
    if tests:
        return ("ERROR", reruns, "re-run unsettled") if unsettled else ("SURVIVED", reruns, "")
    return outcome, 0, why(ran, outcome)


def row_line(row_id, outcome, real_clock, reruns, reason=""):
    fields = {"id": row_id, "outcome": outcome, "real_clock": real_clock, "reruns": reruns}
    return "EZSDR-ROW " + json.dumps(dict(fields, why=reason) if reason else fields)


BASELINES = {}
ORIGINALS = {}


def exact_rerun(scratch, env):
    return lambda package, binary, path: cargo_test(scratch, env, {"crate": package, "test_bin": binary, "filter": path, "exact": True})


def run_one(scratch, env, mutation):
    """`(outcome, real_clock, reruns, why)` for one row."""
    if mutation.get("kill") not in (None, "bound"):
        return f"NOT FOUND: unknown \"kill\" value {mutation['kill']!r}", False, 0, ""
    key = mutation.get("python") or (mutation["crate"], mutation["test_bin"], mutation["filter"])
    if key not in BASELINES:
        baseline = cargo_test(scratch, dict(env, EZSDR_REAL_CLOCK_PROBE="1"), mutation, show_output=True)
        BASELINES[key] = (baseline.returncode == 0 and ran_tests(baseline.output), REAL_CLOCK in baseline.output)
    ok, real_clock = BASELINES[key]
    if not ok:
        return "BASELINE FAILED", real_clock, 0, ""
    path = os.path.join(scratch, mutation["file"])
    if mutation["file"] not in ORIGINALS:
        ORIGINALS[mutation["file"]] = open(path, encoding="utf-8").read()
    original = ORIGINALS[mutation["file"]]
    try:
        mutated = anchor.apply(original, mutation)
    except anchor.AnchorError as error:
        return f"NOT FOUND: {error}", real_clock, 0, ""
    open(path, "w", encoding="utf-8").write(mutated)
    try:
        ran = cargo_test(scratch, env, mutation)
        outcome, reruns, reason = judge(ran, mutation.get("kill") == "bound", exact_rerun(scratch, env))
        return outcome, real_clock, reruns, reason
    finally:
        open(path, "w", encoding="utf-8").write(original)


# The end-to-end cases (note 51 §6.4): fixture, extra environment, whether the row declares `"kill": "bound"`,
# the outcome the runner must draw, and what else the output must show (counts, and `has`: patterns that must match). `{state}` is a directory made for the run.
E2E = [
    {"id": "1-assert-failure", "fixture": "assert_failure", "env": {}, "bound": False, "expect": "killed", "markers": 0},
    {"id": "2-stopped-clock", "fixture": "stopped_clock", "env": {"EZSDR_TEST_WATCHDOG_SECS": "1"}, "bound": False, "expect": "ERROR", "markers": 1, "signal": True},
    {"id": "2-stopped-clock-bound", "fixture": "stopped_clock", "env": {"EZSDR_TEST_WATCHDOG_SECS": "1"}, "bound": True, "expect": "killed (bound)", "markers": 1, "signal": True},
    {"id": "3-creation-limit", "fixture": "creation_limit", "env": {}, "bound": False, "expect": "ERROR", "markers": 1, "panics": 1,
     "has": [r"EZSDR-INFRA: [^\n]*KC-12a", r"test result: FAILED"]},
    {"id": "3b-creation-limit-spec", "fixture": "creation_limit_spec", "env": {}, "bound": False, "expect": "ERROR", "markers": 1, "panics": 1,
     "has": [r"EZSDR-INFRA: [^\n]*KC-12a", r"test result: FAILED"]},
    {"id": "5-abort-without-marker", "fixture": "abort_without_marker", "env": {}, "bound": False, "expect": "ERROR", "markers": 0, "signal": True},
    {"id": "5-abort-without-marker-bound", "fixture": "abort_without_marker", "env": {}, "bound": True, "expect": "ERROR", "markers": 0, "signal": True},
    {"id": "T1-timing-always", "fixture": "timing_always", "env": {}, "bound": False, "expect": "ERROR", "reruns": RERUNS},
    {"id": "T2-timing-once", "fixture": "timing_once", "env": {"EZSDR_INFRA_FIXTURE_STATE": "{state}"}, "bound": False, "expect": "SURVIVED", "reruns": 1},
]


def e2e_case(scratch, env, case, state):
    mutation = {"crate": "ezsdr-radio-uhd", "test_bin": "fake", "filter": f"infra_fixture_{case['fixture']}"}
    env = dict(env, EZSDR_INFRA_FIXTURE=case["fixture"], **{k: v.format(state=state) for k, v in case["env"].items()})
    ran = cargo_test(scratch, env, mutation)
    outcome, reruns, _ = judge(ran, case["bound"], exact_rerun(scratch, env))
    seen = {"markers": ran.output.count(INFRA), "panics": ran.output.count("panicked at"), "signal": SIGNAL in ran.output, "reruns": reruns}
    seen["has"] = [pattern for pattern in case.get("has", []) if re.search(pattern, ran.output)]
    ok = outcome == case["expect"] and all(seen[k] == case[k] for k in seen if k in case and k != "has") and seen["has"] == case.get("has", [])
    return {"id": case["id"], "bound": case["bound"], "outcome": outcome, "expected": case["expect"], "seen": seen, "ok": ok}


def e2e(scratch_arg):
    root = os.getcwd()
    scratch = prepare_scratch(root, os.path.abspath(scratch_arg))
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.expanduser("~/.cache/cargo-target/Ez-SDRv4-review"))
    failures = 0
    with tempfile.TemporaryDirectory() as state:
        for case in E2E:
            result = e2e_case(scratch, env, case, state)
            print("EZSDR-E2E " + json.dumps(result), flush=True)
            failures += not result["ok"]
    return 1 if failures else 0


def self_test():
    """The §6.3 verdict table, cell by cell, and the re-run rule, on strings."""
    failed = "running 1 test\ntest a ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n"
    ok = "running 1 test\n.\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n"
    marker = "EZSDR-INFRA: t::a: a bound expired\n"
    signal = "error: test failed\n  process didn't exit successfully: `fake-1` (signal: 6, SIGABRT: process abort signal)\n"
    python_failed = "FAIL: test_x\n\nRan 1 test in 0.1s\n\nFAILED (failures=1)\n"
    cells = [
        ("test failure", Ran(101, failed), "killed", "killed"),
        ("python failure", Ran(1, python_failed), "killed", "killed"),
        ("marker with a failure line", Ran(101, marker + failed), "ERROR", "killed (bound)"),
        ("marker with no failure line", Ran(101, marker), "ERROR", "killed (bound)"),
        ("marker and a signal (the watchdog)", Ran(101, marker + signal), "ERROR", "killed (bound)"),
        ("test timeout", Ran(None, "", "test"), "ERROR", "killed (bound)"),
        ("signal, no marker", Ran(101, signal), "ERROR", "ERROR"),
        ("signal with a failure line", Ran(101, failed + signal), "ERROR", "ERROR"),
        ("failure with no `test result:` line", Ran(101, "thread panicked\n"), "ERROR", "ERROR"),
        ("build timeout", Ran(None, "", "build"), "ERROR", "ERROR"),
        ("passes", Ran(0, ok), "SURVIVED", "SURVIVED"),
        ("does not compile", Ran(101, "error: could not compile `x`\n"), "COMPILE ERROR", "COMPILE ERROR"),
    ]
    for name, ran, assertion, bound in cells:
        assert (verdict(ran), verdict(ran, True)) == (assertion, bound), f"self-test: {name}: {(verdict(ran), verdict(ran, True))}"

    def tm(test):
        return f"EZSDR-TIMING: p b {test}: late (serial re-run: x)\n"

    def fails(*names):
        """A quiet cargo run with these tests failed: the `failures:` list, the result line and cargo's own error line."""
        return ("\nfailures:\n" + "".join(f"    {n}\n" for n in names) + f"\ntest result: FAILED. 0 passed; {len(names)} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n"
                "error: test failed, to rerun pass `-p p --test b`\n")
    one = tm("t::a") + fails("t::a")
    two = tm("t::a") + tm("t::b") + fails("t::a", "t::b")
    log = []

    def rerun_with(*results):
        queue = list(results)

        def rerun(package, binary, path):
            log.append((package, binary, path))
            return queue.pop(0)
        return rerun
    for name, ran, bound, results, expected in [
        ("a re-run that passes does not kill", Ran(101, one), False, [Ran(0, ok)], ("SURVIVED", 1, "")),
        ("a re-run that fails without the marker kills", Ran(101, one), False, [Ran(101, fails("t::a"))], ("killed", 1, "")),
        ("timing, then a pass", Ran(101, one), False, [Ran(101, one), Ran(0, ok)], ("SURVIVED", 2, "")),
        ("timing, then a failure kills", Ran(101, one), False, [Ran(101, one), Ran(101, fails("t::a"))], ("killed", 2, "")),
        ("timing, a conclusive failure, never a third attempt: the failure stays", Ran(101, one), False, [Ran(101, one), Ran(101, fails("t::a")), Ran(0, ok)], ("killed", 2, "")),
        ("the marker twice is ERROR", Ran(101, one), False, [Ran(101, one), Ran(101, one)], ("ERROR", 2, "re-run unsettled")),
        ("the marker twice is ERROR for a bound row too", Ran(101, one), True, [Ran(101, one), Ran(101, one)], ("ERROR", 2, "re-run unsettled")),
        ("a re-run that hits a marker", Ran(101, one), False, [Ran(101, marker + fails("t::a"))], ("ERROR", 1, "infra marker")),
        ("a re-run that hits a marker, bound row", Ran(101, one), True, [Ran(101, marker + fails("t::a"))], ("killed (bound)", 1, "infra marker")),
        ("a re-run that ran no test", Ran(101, one), False, [Ran(0, "running 0 tests\n")], ("ERROR", 1, "no test ran")),
        ("a re-run that passes with a marker is not a pass", Ran(101, one), False, [Ran(0, tm("t::a") + ok), Ran(0, tm("t::a") + ok)], ("ERROR", 2, "re-run unsettled")),
        ("a re-run that passes with a marker, then without", Ran(101, one), False, [Ran(0, tm("t::a") + ok), Ran(0, ok)], ("SURVIVED", 2, "")),
        ("a re-run that times out", Ran(101, one), False, [Ran(None, "", "test")], ("ERROR", 1, "test timeout")),
        ("a re-run that is signalled: the signal is the reason", Ran(101, one), False, [Ran(101, signal)], ("ERROR", 1, "signal")),
        ("a re-run that does not build", Ran(None, "", "build"), False, [], ("ERROR", 0, "build timeout")),
        ("a failure beyond the marked ones kills at once", Ran(101, tm("t::a") + fails("t::a", "t::c")), False, [], ("killed", 0, "")),
        ("an unnamed failure beside a marked one kills at once", Ran(101, tm("t::a") + fails("t::a").replace("1 failed", "2 failed")), False, [], ("killed", 0, "")),
        ("no marker, no re-run", Ran(101, fails("t::a")), False, [], ("killed", 0, "")),
        ("a signal with a marker is not re-run", Ran(101, one + signal), False, [], ("ERROR", 0, "signal")),
        ("another binary that ended with no result line is not excused", Ran(101, one + "error: test failed, to rerun pass `-p q --test c`\n"), False, [], ("killed", 0, "")),
        ("a pass is not re-run", Ran(0, ok), False, [], ("SURVIVED", 0, "")),
    ]:
        log.clear()
        assert judge(ran, bound, rerun_with(*results)) == expected, f"self-test: {name}: {judge(ran, bound, rerun_with(*results))}"
    log.clear()
    assert judge(Ran(101, two), False, rerun_with(Ran(0, ok), Ran(101, fails("t::b")))) == ("killed", 2, "") and log == [("p", "b", "t::a"), ("p", "b", "t::b")], "self-test: each marked test is re-run alone, one kill is a kill"
    log.clear()
    assert judge(Ran(101, two), False, rerun_with(Ran(0, ok), Ran(0, ok))) == ("SURVIVED", 2, ""), "self-test: all re-runs pass"
    assert timing_tests(tm("t::a") + tm("t::a") + tm("t::b")) == [("p", "b", "t::a"), ("p", "b", "t::b")], "self-test: markers once each"
    assert rerunnable(tm("t::a") + fails("t::a", "t::b")) == [] and rerunnable(one) == [("p", "b", "t::a")], "self-test: every failed test must carry a marker, by name, not by count"
    other_binary = tm("t::c") + fails("t::c").replace("--test b", "--test c")
    assert rerunnable(other_binary) == [] and rerunnable(tm("t::c").replace("p b", "p c") + other_binary) == [("p", "c", "t::c")], "self-test: a marker of another binary's test of the same path does not excuse a failure"
    assert rerunnable(one + signal) == [] and rerunnable(one + marker) == [], "self-test: a signal or an infra marker anywhere keeps the failure"

    sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "remote"))
    from remote_mutate import row_outcome
    tricky = row_line("M01", "ERROR", True, 2)
    for name, ran, outcome, _, expected in [
        ("build timeout", Ran(None, "", "build"), "ERROR", 0, "build timeout"), ("test timeout", Ran(None, "", "test"), "ERROR", 0, "test timeout"),
        ("marker", Ran(101, marker + failed), "ERROR", 0, "infra marker"), ("marker and signal", Ran(101, marker + signal), "ERROR", 0, "infra marker"),
        ("signal", Ran(101, signal), "ERROR", 0, "signal"), ("no result line", Ran(101, "thread panicked\n"), "ERROR", 0, "no test result line"),
        ("bound, marker", Ran(101, marker), "killed (bound)", 0, "infra marker"), ("passed, no test ran", Ran(0, "running 0 tests\n"), "ERROR", 0, "no test ran"),
        ("killed", Ran(101, failed), "killed", 0, ""), ("survived", Ran(0, ok), "SURVIVED", 0, ""),
    ]:
        assert why(ran, outcome) == expected, f"self-test: why: {name}"
    assert json.loads(row_line("M01", "ERROR", False, 0, "signal").split(" ", 1)[1])["why"] == "signal" and "why" not in row_line("M01", "killed", False, 0), "self-test: why in the row line"
    assert row_outcome(tricky) == ("M01", "ERROR") and row_outcome("M01 the row's name: killed") is None, "self-test: a result is read from the fixed field only"
    assert row_outcome(row_line("M01", "killed, but recorded as equivalent", False, 0)) == ("M01", "killed, but recorded as equivalent"), "self-test: the outcome text is the whole field"
    assert run_one(".", {}, {"kill": "unbound"})[0].startswith("NOT FOUND"), "self-test: an unknown kill value is refused"
    fixtures = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "crates", "ezsdr-radio-uhd", "tests", "infra_fixtures", "mod.rs"), encoding="utf-8").read()
    assert all(f"fn infra_fixture_{case['fixture']}()" in fixtures for case in E2E), "self-test: every e2e case names a fixture test"
    print("mutate.py self-test: ok")
    return 0


def main():
    if sys.argv[1:2] == ["--self-test"]:
        return self_test()
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    if sys.argv[1] == "--e2e":
        return e2e(sys.argv[2])
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
        outcome, real_clock, reruns, reason = run_one(scratch, env, mutation)
        if "equivalent" in mutation:
            # Recorded as equivalent: a kill means the record is wrong, not the code.
            outcome = "equivalent (survived)" if outcome == "SURVIVED" else f"{outcome}, but recorded as equivalent"
            failed = not outcome.startswith("equivalent")
        else:
            failed = not outcome.startswith("killed")
        print(row_line(mutation["id"], outcome, real_clock, reruns, reason))
        print(f"{mutation['id']} {mutation['name']}: {outcome}", flush=True)
        if failed:
            failures += 1
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
