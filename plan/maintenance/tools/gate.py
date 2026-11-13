#!/usr/bin/env python3
"""One gate command for a checkout (main tree or worktree): every step in order, one log per step,
and a verdict in <dir>/gate.json.

Usage (from the checkout's root or any directory inside it):
    python3 plan/maintenance/tools/gate.py --out <dir> [--base <commit>] [--renames <file>] [--mutation all|<list>[,<list>...]]
        [--ids ID,ID,...] [--hosts sim02:8,sim03:8] [--only step,step] [--remote sim02|local]
    python3 plan/maintenance/tools/gate.py --self-test

Steps, in this order: fmt, test, clippy, rules, rules-self-test, uhd-clippy, uhd-test, hw-build,
py314, py39, e2e, and mutation (only with --mutation). Every step runs even after another fails, unless
--only leaves it out. <dir>/<step>.log keeps its output. A step passes only when its exit code is 0
and its output parses into counts that its own check accepts; output that cannot be parsed fails.
The mutation step is judged from the per-row and summary lines of remote_mutate.py, not from its
exit code: a row passes when it is killed (`killed`, or `killed (bound)` for a row that declares
`"kill": "bound"`), or when it is a survivor listed in plan/maintenance/tools/known-survivors.txt; an `ERROR`
row (a marker, a signal, a timeout: the mutant was not shown to be caught) is broken; every selected row must
appear in the results. A row whose baseline selected a real-clock FakeDevice test (`real_clock` in its row line) and that
does not declare `"kill": "bound"` (`declared` false or absent) is broken whatever its outcome (killed, survived, a known survivor,
`equivalent (survived)`; note 51 §6.5): the gate checks it from the row line itself, not from the outcome mutate.py drew. A row
that is only `BASELINE FAILED` is reported as that. gate.json's mutation record counts the `killed (bound)` rows, the `ERROR` rows,
the serial re-runs of timing-marked tests and, in `real-clock (declared)` and `real-clock (undeclared)`, the rows that select a
real-clock FakeDevice test; the second must be empty. The e2e step runs `remote_mutate.py --e2e`: mutate.py's
self-test and the infra fixtures through the same `cargo_test()` the rows use, in the mutation image,
and every case must draw its fixed verdict. A failure of the test step whose every failing test wrote an
`EZSDR-TIMING:` marker (matched by name, and with no signal, infra marker or build error anywhere in the output) is
re-run one test at a time (`--exact --test-threads=1`, at most twice each, and only a timing-marked re-run allows the
next); the step passes when each of those re-runs passes without a marker, and gate.json counts the re-runs. The
source identity (HEAD, git status, untracked files, and a sha256 over the diff and those files) is
recorded before the first step and checked after the last. --out must lie outside the checkout.

Verdict: FAIL when a check failed (exit 1); PARTIAL when every check passed but the run left
something out (a step, a mutation list, or --ids; exit 0, reasons name it); PASS only for the full
gate (exit 0). An exception ends the run as FAIL. <dir>/gate.json is removed when the run starts,
and is written atomically: RUNNING first, then the verdict. It also records the Cargo target volume
(one per checkout) and the mutation image id on each remote host. Writes every path with $HOME as ~.
--remote (default sim02) runs every build and test step on that host through remote/rdev.py, so the
local PC carries no build load (owner, 2026-11-01); `--renames <file>` is passed to the rules step as rules.py's
`--renames` (a test whose citation a recorded decision drops) and recorded in gate.json; rules and rules-self-test stay local (they need
git and are light); --remote local runs everything here as before. Standard library only.
"""
import argparse
import ast
import contextlib
import glob
import hashlib
import io
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent / "remote"))
from remote_mutate import E2E_LINE, HEADER, KILLED, PASSED, REMOTE, fixed_field, row_outcome, row_record  # one parser for mutate.py's result lines
from mutate import E2E, RERUNS, Ran, ran_tests, rerunnable, row_line, settle_test, timing_tests  # the timing rule is mutate.py's too

IMAGE = "ezsdr-v4-dev:uhd4.10"
VENV_PYTHON = os.path.expanduser("~/.cache/ezsdr-venv/bin/python")
KNOWN_FILE = "plan/maintenance/tools/known-survivors.txt"
REMOTE_SSH = "ssh -o BatchMode=yes"
STEPS = ["fmt", "test", "clippy", "rules", "rules-self-test", "uhd-clippy", "uhd-test", "hw-build", "py314", "py39", "e2e", "mutation"]
HOME = os.path.expanduser("~")
ROOT = None
LIST_SUMMARY = re.compile(r"^(\w+): (\d+)/(\d+) passed; not passed or missing: (\[.*\])$")
TOTAL_SUMMARY = re.compile(r"^(\d+)/(\d+) passed; not passed or missing: (\[.*\])$")


def parse_fmt(text):
    return {"diffs": len(re.findall(r"^Diff in ", text, re.M))}


def check_fmt(counts):
    return [f"{counts['diffs']} rustfmt hunk(s) differ"] if counts["diffs"] else []


def parse_test(text):
    results = re.findall(r"^test result: \w+\. (\d+) passed; (\d+) failed;", text, re.M)
    if not results:
        return None
    return {"passed": sum(int(p) for p, _ in results), "failed": sum(int(f) for _, f in results),
            "results": len(results), "failing": re.findall(r"^test (.+) \.\.\. FAILED$", text, re.M)}


def check_test(counts):
    return [f"{counts['failed']} failed: {', '.join(counts['failing']) or 'names not printed'}"] if counts["failed"] else []


def parse_build(text):
    if not re.search(r"^\s*Finished ", text, re.M):
        return None
    return {"errors": len(re.findall(r"^error(\[|:)", text, re.M)), "warnings": len(re.findall(r"^warning(\[|:)", text, re.M)),
            "executables": len(re.findall(r"^\s+Executable ", text, re.M))}


def check_build(counts):
    return [f"{counts['errors']} error(s)"] if counts["errors"] else []


def check_hw(counts):
    return check_build(counts) + ([] if counts["executables"] else ["no test executable built"])


def parse_rules(text):
    m = re.search(r"^rules\.py: (\d+) problem\(s\) against (\S+)$", text, re.M)
    return {"problems": int(m[1]), "base": m[2]} if m else None


def check_rules(counts):
    return [f"{counts['problems']} problem(s) against {counts['base']}"] if counts["problems"] else []


def parse_rules_self(text):
    return {"ok": 1} if re.search(r"^rules\.py self-test: ok$", text, re.M) else None


def parse_unittest(text):
    ran = re.search(r"^Ran (\d+) tests? in ", text, re.M)
    results = re.findall(r"^(OK|FAILED)\b", text, re.M)
    if not ran or not results:
        return None
    return {"ran": int(ran[1]), "result": results[-1]}


def check_unittest(counts):
    return ([] if counts["result"] == "OK" else [f"python tests {counts['result']}"]) + ([] if counts["ran"] else ["no tests ran"])


E2E_SUMMARY = re.compile(r"^e2e: (\d+)/(\d+) ok; failed: (\[.*\]); self-test: (ok|FAILED)$", re.M)


def parse_e2e(text):
    summary = E2E_SUMMARY.search(text)
    if not summary:
        return None
    cases = {}
    for line in text.splitlines():
        if case := fixed_field(E2E_LINE, line):
            cases[case.get("id")] = case.get("ok") is True and case.get("outcome") == case.get("expected")
    return {"cases": cases, "ok": int(summary[1]), "total": int(summary[2]), "self_test": summary[4]}


def check_e2e(counts):
    missing = [case["id"] for case in E2E if case["id"] not in counts["cases"]]
    wrong = [i for i, ok in counts["cases"].items() if not ok]
    return ([f"e2e case(s) missing from the output: {missing}"] if missing else []) + ([f"e2e case(s) drew the wrong verdict: {wrong}"] if wrong else []) + (
        [] if counts["self_test"] == "ok" else ["mutate.py self-test failed"])


PARSERS = {
    "fmt": (parse_fmt, check_fmt),
    "test": (parse_test, check_test),
    "clippy": (parse_build, check_build),
    "rules": (parse_rules, check_rules),
    "rules-self-test": (parse_rules_self, None),
    "uhd-clippy": (parse_build, check_build),
    "uhd-test": (parse_test, check_test),
    "hw-build": (parse_build, check_hw),
    "py314": (parse_unittest, check_unittest),
    "py39": (parse_unittest, check_unittest),
    "e2e": (parse_e2e, check_e2e),
}


def _ids(text):
    try:
        value = ast.literal_eval(text)
    except (ValueError, SyntaxError):
        return None
    return value if isinstance(value, list) else None


def parse_mutation(text, exit_code, expected, requested, known):
    """Returns (counts, mutation, reasons); counts is None when remote_mutate's total line is missing."""
    seen, conflicts, unattributed, remote, total, current, rows = {}, [], [], {}, None, None, {}
    for line in text.splitlines():
        if m := HEADER.match(line):
            current = m[1]
        elif found := row_record(line):
            if current is None:
                unattributed.append(line)
                continue
            key, outcome = f"{current}/{found['id']}", found["outcome"]
            if seen.get(key, outcome) != outcome:
                conflicts.append(key)
            seen[key], rows[key] = outcome, found
        elif m := LIST_SUMMARY.match(line):
            remote[m[1]] = (int(m[2]), int(m[3]), _ids(m[4]))
        elif m := TOTAL_SUMMARY.match(line):
            total = (int(m[1]), int(m[2]), _ids(m[3]))
    if total is None or total[2] is None:
        return None, None, ["mutation: remote_mutate's total summary line is missing or unreadable"]
    exp = [f"{n}/{i}" for n, i in expected]
    reasons = []
    if conflicts:
        reasons.append(f"mutation: conflicting outcomes for {sorted(set(conflicts))}")
    if unattributed:
        reasons.append(f"mutation: row lines outside any list block: {unattributed}")
    lists = {}
    for name in dict.fromkeys(n for n, _ in expected):
        here = [f"{name}/{i}" for n, i in expected if n == name]
        passed = [k for k in here if seen.get(k) in PASSED]
        lists[name] = {"passed": len(passed), "total": len(here)}
        entry = remote.get(name)
        if entry is None or entry[2] is None or entry[:2] != (len(passed), len(here)) or sorted(entry[2]) != sorted(k.split("/", 1)[1] for k in here if k not in passed):
            reasons.append(f"mutation: the {name} summary line disagrees with its rows")
    missing = [k for k in exp if k not in seen]
    not_killed = sorted(k for k in exp if seen.get(k) not in PASSED)
    passed_total = sum(seen.get(k) in PASSED for k in exp)
    if (total[0], total[1]) != (passed_total, len(exp)) or sorted(total[2]) != not_killed:
        reasons.append("mutation: the total summary line disagrees with its rows")
    if exit_code != (1 if total[2] else 0):
        reasons.append(f"mutation: remote_mutate exit {exit_code} but its summary has {len(total[2])} row(s) not passed")
    survived = [k for k in not_killed if seen.get(k) == "SURVIVED"]
    known_hits = [k for k in survived if k in known]
    unexpected = [k for k in survived if k not in known]
    baseline = sorted(k for k in not_killed if seen.get(k) == "BASELINE FAILED")
    real_clock = [k for k in exp if k in rows and rows[k].get("real_clock") and seen.get(k) != "BASELINE FAILED"]
    undeclared = [k for k in real_clock if rows[k].get("declared") is not True]
    broken = {k: seen[k] for k in not_killed if k in seen and k not in survived and k not in baseline}
    if missing:
        reasons.append(f"mutation: selected row(s) missing from the results: {missing}")
    if broken:
        reasons.append(f"mutation: not killed, and not a survivor (only SURVIVED may be known): {broken}")
    if unexpected:
        reasons.append(f"mutation: survived and not in {KNOWN_FILE}: {unexpected}")
    if baseline:
        reasons.append(f"mutation: BASELINE FAILED in {baseline}")
    if undeclared:
        reasons.append(f"mutation: real-clock row(s) without a \"kill\": \"bound\" declaration, whatever their outcome: {undeclared}")
    extra = sorted(k for k in seen if k not in exp)
    if extra:
        reasons.append(f"mutation: result rows that were not selected: {extra}")
    missing_ids = [i for i in requested if not any(k.split("/", 1)[1] == i for k in seen)]
    if missing_ids:
        reasons.append(f"mutation: requested id(s) not in the results: {missing_ids}")
    notes = [f"{k} was a known survivor and is now killed: remove it from {KNOWN_FILE}" for k in known if seen.get(k) in KILLED]
    mutation = {"lists": lists, "not_killed": not_killed, "known": known_hits, "unexpected": unexpected, "baseline_failed": baseline,
                "broken": broken, "missing": missing, "missing_ids": missing_ids, "notes": notes, "remote_exit": exit_code,
                "errors": sorted(k for k in exp if seen.get(k) == "ERROR"), "killed_by_bound": sorted(k for k in exp if seen.get(k) == "killed (bound)"),
                "timing_reruns": {k: rows[k]["reruns"] for k in exp if k in rows and rows[k].get("reruns")},
                "real-clock (declared)": sorted(set(real_clock) - set(undeclared)), "real-clock (undeclared)": sorted(undeclared)}
    return {"passed": total[0], "total": total[1]}, mutation, reasons


def parse_known(text):
    known = {}
    for line in text.splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            key, _, reason = line.partition(" ")
            known[key] = reason.strip()
    return known


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, check=True).stdout


def identity():
    digest = hashlib.sha256(git("diff", "HEAD", "--binary"))
    untracked = [os.fsdecode(raw) for raw in git("ls-files", "-o", "--exclude-standard", "-z").split(b"\0") if raw]
    for path in untracked:
        digest.update(b"\0untracked\0" + os.fsencode(path) + b"\0" + Path(path).read_bytes())
    return {"head": git("rev-parse", "HEAD").decode().strip(),
            "status": git("status", "--porcelain=v1", "--untracked-files=all").decode().splitlines(),
            "untracked": untracked, "sha256": digest.hexdigest()}


def identity_reasons(before, after):
    if before == after:
        return []
    changed = [k for k in ("head", "status", "untracked", "sha256") if before[k] != after[k]]
    return [f"tree changed during gate: {', '.join(changed)} differ between the start and the end"]


def cargo_target_volume(root):
    """One Docker volume per checkout path: two checkouts never share the artifacts they both mount at /work."""
    return "ezsdr-v4-cargo-target-" + hashlib.sha256(root.encode()).hexdigest()[:12]


def remote_toolchain(host):
    try:
        lines = subprocess.run([*shlex.split(REMOTE_SSH), host, f"docker image inspect --format '{{{{.Id}}}}' {IMAGE} && docker run --rm {IMAGE} sh -c "
                                "'rustc -V; cargo -V; rustfmt --version; python3 -V' && docker run --rm python:3.9-slim python -V"],
                               capture_output=True, text=True, check=True, timeout=600).stdout.split("\n")
    except (OSError, subprocess.SubprocessError) as error:
        return {"remote": host, "unavailable": str(error)}
    return dict(zip(["docker_image", "rustc", "cargo", "rustfmt", "python314", "python39"], lines), remote=host)


def toolchain(volume):
    def first(command):
        try:
            return subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, check=True, timeout=300).stdout.strip()
        except (OSError, subprocess.SubprocessError) as error:
            return f"unavailable: {error}"
    return {"rustc": first(["rustc", "-V"]), "cargo": first(["cargo", "-V"]), "rustfmt": first(["rustfmt", "--version"]),
            "docker_image": first(["docker", "image", "inspect", "--format", "{{.Id}}", IMAGE]), "cargo_target_volume": volume,
            "python314": first([VENV_PYTHON, "-V"]), "python39": first(["docker", "run", "--rm", "python:3.9-slim", "python", "-V"])}


def remote_images(hosts):
    """The image id of remote_mutate's image on each host: it is built there, so it is the toolchain the rows ran with."""
    images = {}
    for host in dict.fromkeys(h.split(":")[0] for h in hosts.split(",")):
        try:
            images[host] = subprocess.run([*shlex.split(REMOTE_SSH), host, shlex.join(["docker", "image", "inspect", "--format", "{{.Id}}", REMOTE])],
                                          capture_output=True, text=True, check=True, timeout=300).stdout.strip()
        except (OSError, subprocess.SubprocessError) as error:
            images[host] = f"unavailable: {error}"
    return images


def show(argv, env=None):
    return " ".join([f"{k}={shlex.quote(v)}" for k, v in (env or {}).items()] + [shlex.join(argv)])


def remote_commands_for(name, ctx):
    rdev = ["python3", "plan/maintenance/tools/remote/rdev.py", "--host", ctx.remote, "--"]
    feature = ["-p", "ezsdr-radio-uhd", "--features", "uhd"]
    py = "python -m unittest discover -s python/tests"
    table = {
        "fmt": [(rdev + ["cargo", "fmt", "--all", "--check"], {})],
        "test": [(rdev + ["cargo", "test", "--workspace", "--no-fail-fast"], {})],
        "clippy": [(rdev + ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"], {})],
        "uhd-clippy": [(rdev + ["cargo", "clippy", *feature, "--all-targets", "--", "-D", "warnings"], {})],
        "uhd-test": [(rdev + ["cargo", "test", *feature, "--lib", "--test", "uhd_api"], {})],
        "hw-build": [(rdev + ["cargo", "test", *feature, "--test", "hardware", "--no-run"], {})],
        "py314": [(rdev + ["sh", "-c", "cargo build -p ezsdr-server && python3 -m unittest discover -s python/tests"], {})],
        "py39": [(rdev[:-1] + ["--image", "python:3.9-slim", "--", "sh", "-c",
                               f"pip -q install --user numpy && EZSDR_SERVER=/cargo-target/debug/ezsdr-server PYTHONDONTWRITEBYTECODE=1 {py}"], {})],
    }
    return table[name] if name in table else local_commands_for(name, ctx)


def commands_for(name, ctx):
    return remote_commands_for(name, ctx) if ctx.remote != "local" else local_commands_for(name, ctx)


def local_commands_for(name, ctx):
    uid = f"{os.getuid()}:{os.getgid()}"
    uhd = ["docker", "run", "--rm", "--user", uid, "-v", f"{ROOT}:/work", "-v", f"{ctx.volume}:/cargo-target", "-w", "/work", IMAGE]
    feature = ["-p", "ezsdr-radio-uhd", "--features", "uhd"]
    table = {
        "fmt": [(["cargo", "fmt", "--all", "--check"], {})],
        "test": [(["cargo", "test", "--workspace", "--no-fail-fast"], {})],
        "clippy": [(["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"], {})],
        "rules": [(["python3", "plan/maintenance/tools/rules.py", "--base", ctx.base, *(["--renames", ctx.renames] if getattr(ctx, "renames", None) else [])], {})],
        "rules-self-test": [(["python3", "plan/maintenance/tools/rules.py", "--self-test"], {})],
        "uhd-clippy": [(uhd + ["cargo", "clippy", *feature, "--all-targets", "--", "-D", "warnings"], {})],
        "uhd-test": [(uhd + ["cargo", "test", *feature, "--lib", "--test", "uhd_api"], {})],
        "hw-build": [(uhd + ["cargo", "test", *feature, "--test", "hardware", "--no-run"], {})],
        "py314": [(["cargo", "build", "-p", "ezsdr-server"], {}),
                  ([VENV_PYTHON, "-m", "unittest", "discover", "-s", "python/tests"], {"EZSDR_SERVER": ctx.server})],
        "py39": [(["docker", "run", "--rm", "--user", uid, "-e", "HOME=/tmp", "-e", "PYTHONDONTWRITEBYTECODE=1",
                   "-e", "EZSDR_SERVER=/srv/ezsdr-server", "-v", f"{ROOT}:/work:ro", "-v", f"{ctx.server}:/srv/ezsdr-server:ro",
                   "-w", "/work", "python:3.9-slim", "sh", "-c", "pip -q install --user numpy && python -m unittest discover -s python/tests"], {})],
        "e2e": [(["python3", "plan/maintenance/tools/remote/remote_mutate.py", "--e2e", ctx.hosts], {"REMOTE_SSH": REMOTE_SSH})],
        "mutation": [(["python3", "plan/maintenance/tools/remote/remote_mutate.py", ctx.lists, ctx.hosts, *ctx.ids],
                      {"REMOTE_SSH": REMOTE_SSH})],
    }
    return table[name]


def run_step(name, commands, out):
    started = time.monotonic()
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    text, code = [], 0
    for argv, extra in commands:
        proc = subprocess.run(argv, cwd=ROOT, env={**env, **extra}, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
        text.append(f"$ {show(argv, extra)}\n{proc.stdout}")
        if proc.returncode:
            code = proc.returncode
            break
    log = out / f"{name}.log"
    log.write_text("".join(text), encoding="utf-8")
    return code, round(time.monotonic() - started, 1), "".join(text), log


def judge(name, code, text):
    parse, check = PARSERS[name]
    counts = parse(text)
    reasons = [] if code == 0 else [f"{name}: exit {code}"]
    if counts is None:
        reasons.append(f"{name}: output cannot be parsed")
    elif check:
        reasons += [f"{name}: {message}" for message in check(counts)]
    return counts, reasons


def rerun_argv(package, binary, path, remote):
    cargo = ["cargo", "test", "-p", package, "--test", binary, "--", "--exact", "--test-threads=1", path]
    return cargo if remote == "local" else ["python3", "plan/maintenance/tools/remote/rdev.py", "--host", remote, "--", *cargo]


def run_rerun(argv):
    proc = subprocess.run(argv, cwd=ROOT, env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
    return proc.returncode, f"$ {shlex.join(argv)}\n{proc.stdout}"


def excuse_timing(name, counts, reasons, text, rerun):
    """The result of a test step after the serial re-runs of its timing-marked failures (note 51 Q16 b'').
    Only when `rerunnable` finds every failed test named and marked, and nothing else wrong in the output (a signal or
    an infra marker anywhere, another binary's included, is never excused): each is re-run alone by `settle_test`, and
    the step passes if every one passes with no marker. A conclusive failure, or two marked re-runs, keeps the reasons.
    `rerun(package, binary, path)` returns (exit code, output)."""
    tests = rerunnable(text)
    if not counts or not counts["failed"] or not tests:
        return counts, reasons
    attempts = {}
    for package, binary, path in tests:
        result, again, attempts[path] = settle_test((package, binary, path), lambda p, b, t: Ran(*rerun(p, b, t)))
        if result != "SURVIVED":
            return {**counts, "timing_reruns": attempts}, reasons + [f"{name}: {path} fails again when re-run alone ({'timing-marked twice' if result == 'unsettled' else result})"]
    return {**counts, "timing_reruns": attempts}, []


def coverage_gaps(only, mutation, ids, available):
    """What a run left out. A run that passed everything but has gaps is PARTIAL, never PASS."""
    gaps = []
    skipped = [s for s in STEPS if (only is not None and s not in only) or (s == "mutation" and not mutation)]
    if skipped:
        gaps.append(f"not run: steps {', '.join(skipped)}")
    chosen = set(available) if mutation == "all" else set(mutation.split(",")) if mutation else set()
    left = [n for n in available if n not in chosen]
    if mutation and left:
        gaps.append(f"not run: mutation lists {', '.join(left)}")
    if ids:
        gaps.append(f"not run: mutation rows outside --ids ({', '.join(ids)})")
    return gaps


def verdict_of(failures, gaps):
    return "FAIL" if failures else "PARTIAL" if gaps else "PASS"


def sanitize(value):
    if isinstance(value, str):
        return re.sub(re.escape(HOME) + r"(?=/|$)", "~", value)
    if isinstance(value, list):
        return [sanitize(v) for v in value]
    if isinstance(value, dict):
        return {k: sanitize(v) for k, v in value.items()}
    return value


def now():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def write_manifest(out, record):
    temp = out / f".gate.{os.getpid()}.tmp"
    temp.write_text(json.dumps(sanitize(record), indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(temp, out / "gate.json")


def finish(out, record, failures, gaps):
    verdict = verdict_of(failures, gaps)
    record.update(verdict=verdict, reasons=failures + gaps, finished=now())
    write_manifest(out, record)
    print(f"verdict: {verdict}")
    for reason in record["reasons"]:
        print(f"  - {reason}")
    print(f"gate.json: {sanitize(str(out / 'gate.json'))}")
    return 1 if verdict == "FAIL" else 0


def run_guarded(out, record, gaps, body):
    """Runs body(failures) with a RUNNING manifest in place. An exception becomes a failure, so no run leaves an older verdict."""
    record.update(verdict="RUNNING", reasons=[])
    write_manifest(out, record)
    failures = []
    try:
        body(failures)
    except BaseException as error:
        failures.append(f"gate.py: {type(error).__name__}: {error}")
    return finish(out, record, failures, gaps)


def execute(out, record, failures, plan):
    metadata = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], capture_output=True, text=True, check=True).stdout)
    server = os.path.join(metadata["target_directory"], "debug", "ezsdr-server")
    if plan.remote == "local" and "py39" in plan.selected and "py314" not in plan.selected and not os.path.exists(server):
        failures.append(f"py39: {server} is missing; run the py314 step first (it builds ezsdr-server)")
        return
    known = parse_known(open(os.path.join(ROOT, KNOWN_FILE), encoding="utf-8").read())
    rows = [(name, r["id"]) for name in plan.lists for r in json.load(open(f"plan/{name}/tools/mutations.json", encoding="utf-8"))
            if not plan.ids or r["id"] in plan.ids]
    ctx = SimpleNamespace(base=plan.base, renames=plan.renames, server=server, lists=plan.mutation, hosts=plan.hosts, ids=plan.ids, volume=cargo_target_volume(ROOT), remote=plan.remote)
    before = identity()
    record["identity"] = {"before": before}
    record["toolchain"] = toolchain(ctx.volume) if plan.remote == "local" else remote_toolchain(plan.remote)
    for name in plan.selected:
        code, seconds, text, log = run_step(name, commands_for(name, ctx), out)
        if name == "mutation":
            counts, mutation, reasons = parse_mutation(text, code, rows, plan.ids, known)
            images = remote_images(plan.hosts)
            record["mutation"] = {**(mutation or {}), "hosts": plan.hosts, "images": images}
            reasons += [f"mutation: image id on {host} {image}" for host, image in images.items() if image.startswith("unavailable")]
        else:
            counts, reasons = judge(name, code, text)
            if name == "test":
                reruns = []
                counts, reasons = excuse_timing(name, counts, reasons, text, lambda p, b, t: reruns.append(run_rerun(rerun_argv(p, b, t, plan.remote))) or reruns[-1])
                if reruns:
                    log.write_text(log.read_text(encoding="utf-8") + "".join(again for _, again in reruns), encoding="utf-8")
        failures.extend(reasons)
        record["steps"].append({"name": name, "command": " && ".join(show(argv, extra) for argv, extra in commands_for(name, ctx)), "exit": code,
                                "seconds": seconds, "counts": counts, "log": str(log), "ok": not reasons})
        print(f"{name:<16}{'PASS' if not reasons else 'FAIL':<6}exit {code:<4}{seconds:>8.1f}s  {json.dumps(counts) if counts else 'unparsed'}", flush=True)
    after = identity()
    record["identity"]["after"] = after
    failures.extend(identity_reasons(before, after))


def die(message):
    print(f"gate.py: {message}", file=sys.stderr)
    return 2


def main(argv):
    global ROOT
    if "--self-test" in argv:
        return self_test()
    parser = argparse.ArgumentParser(description="One gate command; see the module docstring.")
    parser.add_argument("--out", required=True)
    parser.add_argument("--base", default="HEAD")
    parser.add_argument("--renames")
    parser.add_argument("--mutation")
    parser.add_argument("--ids")
    parser.add_argument("--hosts", default="sim02:8,sim03:8")
    parser.add_argument("--only")
    parser.add_argument("--remote", default="sim02")
    args = parser.parse_args(argv)
    try:
        ROOT = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip()
    except subprocess.CalledProcessError:
        return die("not inside a git checkout")
    os.chdir(ROOT)
    out = Path(args.out).expanduser().resolve()
    try:
        out.relative_to(ROOT)
        return die("--out must lie outside the checkout (its files would change the source identity)")
    except ValueError:
        pass
    out.mkdir(parents=True, exist_ok=True)
    (out / "gate.json").unlink(missing_ok=True)
    only = args.only.split(",") if args.only else None
    if only and any(s not in STEPS for s in only):
        return die(f"unknown step in --only; steps are {','.join(STEPS)}")
    if args.ids and not args.mutation:
        return die("--ids needs --mutation")
    if only and "mutation" in only and not args.mutation:
        return die("the mutation step needs --mutation")
    available = sorted(p.split("/")[1] for p in glob.glob("plan/*/tools/mutations.json"))
    lists = available if args.mutation == "all" else (args.mutation or "").split(",") if args.mutation else []
    if any(name not in available for name in lists):
        return die(f"unknown mutation list; lists are all,{','.join(available)}")
    ids = args.ids.split(",") if args.ids else []
    selected = [s for s in STEPS if (only is None or s in only) and (s != "mutation" or args.mutation)]
    plan = SimpleNamespace(base=args.base, renames=args.renames, mutation=args.mutation, lists=lists, hosts=args.hosts, ids=ids, selected=selected, remote=args.remote)
    record = {"verdict": "RUNNING", "reasons": [], "started": now(), "finished": None, "renames": args.renames, "identity": None, "toolchain": None, "steps": [], "mutation": None}
    return run_guarded(out, record, coverage_gaps(only, args.mutation, ids, available), lambda failures: execute(out, record, failures, plan))


TEST_OK = ("running 2 tests\ntest a::b ... ok\n\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n\n"
           "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n")
TEST_FAIL = "test x::y ... FAILED\n\ntest result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n"


def remote_text(rows, real_clock=(), declared=()):
    """remote_mutate's output for `(list, id, outcome)` rows: the headers, each row's fixed-field line and its free-text line, the summaries.
    The ids in `real_clock` have a marked baseline and those in `declared` say `"kill": "bound"`."""
    text, current = "", None
    for name, row_id, outcome in rows:
        text += (f"--- {name} (sim02)\n" if name != current else "") + row_line(row_id, outcome, row_id in real_clock, 0, declared=row_id in declared) + f"\n{row_id} a name: {outcome}\n"
        current = name
    for name in dict.fromkeys(r[0] for r in rows):
        mine = [r for r in rows if r[0] == name]
        text += f"{name}: {sum(o in PASSED for _, _, o in mine)}/{len(mine)} passed; not passed or missing: {[i for _, i, o in mine if o not in PASSED]}\n"
    return text + f"{sum(o in PASSED for _, _, o in rows)}/{len(rows)} passed; not passed or missing: {[f'{n}/{i}' for n, i, o in rows if o not in PASSED]}\n"


def mut_known(m39="killed", g09="SURVIVED", g10="killed", **marks):
    return remote_text([("phase3", "M39", m39), ("phase7", "G09", g09), ("phase7", "G10", g10)], **marks)


MUT_KNOWN = mut_known()
MUT_ROWS = [("phase3", "M39"), ("phase7", "G09"), ("phase7", "G10")]
AVAILABLE = ["maintenance", "phase3", "phase4", "phase5", "phase6", "phase7"]


def self_test():
    assert parse_test(TEST_OK) == {"passed": 3, "failed": 0, "results": 2, "failing": []}, "self-test: a passing summary"
    assert check_test(parse_test(TEST_FAIL)) == ["1 failed: x::y"], "self-test: a failing summary names the test"
    assert parse_test("error[E0432]: unresolved import\n") is None, "self-test: a test run that did not compile is unparsable"
    assert parse_build("    Finished `dev` profile in 3.2s\n") == {"errors": 0, "warnings": 0, "executables": 0}, "self-test: a build summary"
    assert parse_build("error: could not compile\n") is None, "self-test: a build without Finished is unparsable"
    assert parse_fmt("Diff in /x/a.rs at line 3:\n") == {"diffs": 1} and check_fmt({"diffs": 1}), "self-test: fmt hunks"
    assert parse_fmt("") == {"diffs": 0}, "self-test: a clean fmt check"
    assert check_rules(parse_rules("rules.py: 2 problem(s) against HEAD\n")), "self-test: rules problems fail"
    assert parse_rules("rules.py: HEAD is not a commit\n") is None, "self-test: a rules error is unparsable"
    assert parse_rules_self("rules.py self-test: ok\n") == {"ok": 1} and parse_rules_self("") is None, "self-test: rules self-test"
    assert parse_unittest("Ran 57 tests in 2.1s\n\nOK\n") == {"ran": 57, "result": "OK"}, "self-test: unittest OK"
    assert check_unittest(parse_unittest("Ran 3 tests in 0.1s\n\nFAILED (failures=1)\n")) == ["python tests FAILED"], "self-test: unittest FAILED"
    assert parse_unittest("garbage") is None, "self-test: unittest output without a result is unparsable"
    remote = SimpleNamespace(remote="sim03", base="HEAD", server="/s", lists="all", hosts="h", ids=[], volume="v")
    assert remote_commands_for("test", remote)[0][0][:5] == ["python3", "plan/maintenance/tools/remote/rdev.py", "--host", "sim03", "--"], "self-test: --remote runs tests through rdev"
    assert commands_for("rules", remote)[0][0][0] == "python3" and "rdev.py" not in " ".join(commands_for("rules", remote)[0][0]), "self-test: rules stay local"
    renamed = SimpleNamespace(**{**vars(remote), "renames": "r.txt"})
    assert commands_for("rules", renamed)[0][0][-2:] == ["--renames", "r.txt"] and "--renames" not in commands_for("rules", remote)[0][0], "self-test: --renames reaches the rules step only when given"
    assert all("rdev.py" in " ".join(commands_for(n, remote)[0][0]) for n in ("fmt", "clippy", "uhd-clippy", "uhd-test", "hw-build", "py314", "py39")), "self-test: every build step is remote"
    assert commands_for("test", SimpleNamespace(**{**vars(remote), "remote": "local"}))[0][0][0] == "cargo", "self-test: --remote local runs here"

    counts, mutation, reasons = parse_mutation(MUT_KNOWN, 1, MUT_ROWS, [], {"phase7/G09": "race"})
    assert reasons == [] and mutation["known"] == ["phase7/G09"] and mutation["unexpected"] == [], "self-test: a known survivor passes"
    assert counts == {"passed": 2, "total": 3}, "self-test: mutation counts"
    _, mutation, reasons = parse_mutation(mut_known(g09="killed", g10="SURVIVED"), 1, MUT_ROWS, [], {"phase7/G09": "race"})
    assert mutation["unexpected"] == ["phase7/G10"] and reasons, "self-test: an unexpected survivor fails"
    assert mutation["notes"] and mutation["known"] == [], "self-test: a known survivor now killed is a note"
    for outcome in ("COMPILE ERROR", "NOT FOUND: no anchor", "ERROR"):
        _, mutation, reasons = parse_mutation(mut_known(g09=outcome), 1, MUT_ROWS, [], {"phase7/G09": "race"})
        assert mutation["known"] == [] and mutation["broken"] == {"phase7/G09": outcome} and reasons, f"self-test: a known id with {outcome} fails"
    _, mutation, reasons = parse_mutation(mut_known(g09="ERROR"), 1, MUT_ROWS, [], {})
    assert mutation["errors"] == ["phase7/G09"] and any("not killed, and not a survivor" in r for r in reasons), "self-test: an ERROR row is broken, even when it is not a known survivor"
    text = remote_text([("phase3", "M39", "killed (bound)"), ("phase7", "G09", "killed"), ("phase7", "G10", "killed")]).replace(
        row_line("G10", "killed", False, 0), row_line("G10", "killed", True, 2, declared=True))
    _, mutation, reasons = parse_mutation(text, 0, MUT_ROWS, [], {})
    assert reasons == [] and mutation["killed_by_bound"] == ["phase3/M39"] and mutation["real-clock (declared)"] == ["phase7/G10"] and mutation["real-clock (undeclared)"] == [] and mutation["timing_reruns"] == {"phase7/G10": 2}, "self-test: a bound kill passes and is counted; declared real-clock rows and re-runs are reported"
    # Note 51 §6.5: a marked row without a declaration is broken whatever its outcome, read from the row line.
    for name, key, text in [
        ("killed", "phase7/G10", mut_known(real_clock={"G10"})),
        ("a known survivor (the known file does not excuse it)", "phase7/G09", mut_known(real_clock={"G09"})),
        ("equivalent (survived)", "phase7/G10", mut_known(g10="equivalent (survived)", real_clock={"G10"})),
        ("a row line with no `declared` key", "phase7/G10", mut_known(real_clock={"G10"}).replace(', "declared": false', "")),
    ]:
        _, mutation, reasons = parse_mutation(text, 1, MUT_ROWS, [], {"phase7/G09": "race"})
        assert mutation["real-clock (undeclared)"] == [key] and mutation["real-clock (declared)"] == [] and any("without a" in r and key in r for r in reasons), f"self-test: an undeclared marked row is broken: {name}"
    _, mutation, reasons = parse_mutation(mut_known(m39="killed (bound)", real_clock={"M39"}, declared={"M39"}), 1, MUT_ROWS, [], {"phase7/G09": "race"})
    assert reasons == [] and mutation["real-clock (declared)"] == ["phase3/M39"] and mutation["real-clock (undeclared)"] == [], "self-test: a declared marked row passes and is listed"
    _, mutation, reasons = parse_mutation(mut_known(m39="BASELINE FAILED", real_clock={"M39"}), 1, MUT_ROWS, [], {"phase7/G09": "race"})
    assert mutation["real-clock (undeclared)"] == [] and len(reasons) == 1 and "BASELINE FAILED" in reasons[0], "self-test: a marked baseline failure is only a baseline failure"
    _, mutation, reasons = parse_mutation(mut_known(declared={"G10"}), 1, MUT_ROWS, [], {"phase7/G09": "race"})
    assert reasons == [] and mutation["real-clock (undeclared)"] == [] and mutation["real-clock (declared)"] == [], "self-test: a row with no mark is not affected, declared or not"
    _, mutation, reasons = parse_mutation(MUT_KNOWN.replace("G09 a name: SURVIVED", "G09 a name: killed"), 1, MUT_ROWS, [], {"phase7/G09": "race"})
    assert mutation["known"] == ["phase7/G09"], "self-test: the free-text line is not a result"
    spoof = remote_text([("phase3", "M39", "SURVIVED")]).replace("M39 a name: SURVIVED", "M39 a: killed")
    assert parse_mutation(spoof, 1, [("phase3", "M39")], [], {})[1]["unexpected"] == ["phase3/M39"], "self-test: a row named `a: killed` is not read as killed (#91)"
    _, mutation, reasons = parse_mutation(MUT_KNOWN.replace(row_line("G09", "SURVIVED", False, 0) + "\n", ""), 1, MUT_ROWS, [], {"phase7/G09": "race"})
    assert mutation["missing"] == ["phase7/G09"] and any("missing" in r for r in reasons), "self-test: a known id with no result row fails"
    _, mutation, reasons = parse_mutation(mut_known(m39="BASELINE FAILED"), 1, MUT_ROWS, [], {"phase7/G09": "race"})
    assert mutation["baseline_failed"] == ["phase3/M39"] and any("BASELINE FAILED" in r for r in reasons), "self-test: BASELINE FAILED fails"
    only_m39 = remote_text([("phase3", "M39", "killed")])
    _, mutation, reasons = parse_mutation(only_m39, 0, [("phase3", "M39")], ["M39", "NOPE"], {})
    assert mutation["missing_ids"] == ["NOPE"] and any("NOPE" in r for r in reasons), "self-test: a requested id missing from the results fails"
    _, mutation, reasons = parse_mutation(remote_text([("phase3", "M39", "killed, but recorded as equivalent")]), 1, [("phase3", "M39")], [], {})
    assert mutation["broken"] == {"phase3/M39": "killed, but recorded as equivalent"} and reasons, "self-test: a killed row recorded as equivalent fails"
    assert row_outcome(row_line("M39", "killed, but recorded as equivalent", False, 0)) == ("M39", "killed, but recorded as equivalent") and row_outcome("phase3: 1/1 passed; not passed or missing: []") is None, "self-test: the shared row parser"
    assert "killed, but recorded as equivalent" not in PASSED and "killed (bound)" in PASSED and "ERROR" not in PASSED and "killed (timeout)" not in PASSED, "self-test: the shared outcome classes"
    assert parse_mutation("--- phase3 (sim02)\n" + row_line("M39", "killed", False, 0) + "\n", 0, [("phase3", "M39")], [], {})[2], "self-test: no summary is unparsable"
    assert parse_mutation(MUT_KNOWN, 0, MUT_ROWS, [], {"phase7/G09": "race"})[2], "self-test: an exit code that disagrees with the summary fails"

    marker = "EZSDR-TIMING: p b m::t: late\n"
    cargo_error = "error: test failed, to rerun pass `-p p --test b`\n"
    failed = "test m::t ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n" + cargo_error
    other = "test m::u ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n" + cargo_error
    sigkill = "error: test failed, to rerun pass `-p q --test c`\n\nCaused by:\n  process didn't exit successfully: `c` (signal: 9, SIGKILL: kill)\n"
    once = parse_test(marker + failed)
    calls = []

    def reruns(*results):
        queue = list(results)
        return lambda package, binary, path: calls.append((package, binary, path)) or queue.pop(0)

    def excused_by(text, *results, counts=None):
        calls.clear()
        return excuse_timing("test", counts or parse_test(text), ["test: exit 101"], text, reruns(*results))
    excused, reasons = excuse_timing("test", once, ["test: exit 101", "test: 1 failed: m::t"], marker + failed, reruns((0, TEST_OK)))
    assert reasons == [] and excused["timing_reruns"] == {"m::t": 1} and calls == [("p", "b", "m::t")], "self-test: a timing-marked failure that passes alone passes the step, and the re-run is counted"
    excused, reasons = excuse_timing("test", once, ["test: exit 101"], marker + failed, reruns((101, marker + failed), (0, TEST_OK)))
    assert reasons == [] and excused["timing_reruns"] == {"m::t": 2}, "self-test: a second re-run may pass"
    for again in ((101, marker + failed), (101, failed), (0, marker + TEST_OK), (0, "running 0 tests\n")):
        excused, reasons = excuse_timing("test", once, ["test: exit 101"], marker + failed, reruns(again, again))
        assert reasons and excused["timing_reruns"] == {"m::t": 1 if again[1] in (failed, "running 0 tests\n") else 2}, f"self-test: a re-run that fails, passes with a marker or runs no test does not excuse it: {again}"
    excused, reasons = excused_by(marker + failed, (101, marker + failed), (101, failed), (0, TEST_OK))
    assert reasons and excused["timing_reruns"] == {"m::t": 2} and len(calls) == 2, "self-test: a re-run that fails without the marker is conclusive: no third attempt that passes may erase it"
    excused, reasons = excused_by(marker + failed + other, (0, TEST_OK))
    assert reasons and calls == [], "self-test: a failed test with no marker beside a marked one is not excused, and nothing is re-run"
    text = marker + failed + sigkill
    excused, reasons = excused_by(text, (0, TEST_OK), counts=parse_test(text))
    assert reasons and calls == [], "self-test: another binary's SIGKILL is not excused by a passing re-run"
    text = marker + failed + "EZSDR-INFRA: x::y: a bound expired\n"
    assert excused_by(text, (0, TEST_OK))[1] and calls == [], "self-test: an infra marker anywhere is not excused"
    text = marker + failed + "error: test failed, to rerun pass `-p q --test c`\n"
    assert excused_by(text, (0, TEST_OK))[1] and calls == [], "self-test: a test binary that ended with no failure line is not excused"
    text = marker + failed.replace("--test b", "--test c")
    assert excused_by(text, (0, TEST_OK))[1] and calls == [], "self-test: a marker of another binary's test of the same path is not excused"
    unmarked = parse_test(marker + "test a ... FAILED\n" + failed)
    unmarked["failed"] = 2
    assert excuse_timing("test", unmarked, ["test: exit 101"], marker + "test a ... FAILED\n" + failed, reruns()) == (unmarked, ["test: exit 101"]), "self-test: a failure with no marker is not excused"
    assert excuse_timing("test", parse_test(TEST_FAIL), ["test: exit 101"], TEST_FAIL, reruns()) == (parse_test(TEST_FAIL), ["test: exit 101"]), "self-test: no marker, no re-run"
    assert rerun_argv("p", "b", "m::t", "local")[:3] == ["cargo", "test", "-p"] and rerun_argv("p", "b", "m::t", "sim03")[2:5] == ["--host", "sim03", "--"], "self-test: the re-run command is local or through rdev"
    assert rerun_argv("p", "b", "m::t", "local")[-3:] == ["--exact", "--test-threads=1", "m::t"], "self-test: the re-run is exact and serial"
    e2e_ok = "".join("EZSDR-E2E " + json.dumps({"id": c["id"], "outcome": c["expect"], "expected": c["expect"], "ok": True}) + "\n" for c in E2E)
    summary = f"e2e: {len(E2E)}/{len(E2E)} ok; failed: []; self-test: ok\n"
    counts = parse_e2e(e2e_ok + summary)
    assert counts and check_e2e(counts) == [], "self-test: every e2e case drew its verdict"
    assert parse_e2e(e2e_ok) is None and parse_e2e("garbage") is None, "self-test: e2e output with no summary is unparsable"
    assert any("missing" in r for r in check_e2e(parse_e2e("".join(e2e_ok.splitlines(True)[1:]) + summary))), "self-test: a missing e2e case fails"
    assert any("wrong verdict" in r for r in check_e2e(parse_e2e(e2e_ok.replace('"ok": true', '"ok": false', 1) + summary))), "self-test: a case with the wrong verdict fails"
    assert any("self-test" in r for r in check_e2e(parse_e2e(e2e_ok + summary.replace("self-test: ok", "self-test: FAILED")))), "self-test: a failed mutate.py self-test fails"

    assert coverage_gaps(None, "all", [], AVAILABLE) == [], "self-test: the full gate has no gaps"
    assert any("mutation" in g for g in coverage_gaps(None, None, [], AVAILABLE)), "self-test: no --mutation is a gap"
    assert any(g.startswith("not run: steps") and "test" in g for g in coverage_gaps(["fmt"], "all", [], AVAILABLE)), "self-test: --only is a gap"
    assert any("phase4" in g for g in coverage_gaps(None, "phase3", [], AVAILABLE)), "self-test: a list other than all is a gap"
    assert any("--ids" in g for g in coverage_gaps(None, "all", ["M39"], AVAILABLE)), "self-test: --ids is a gap"
    assert verdict_of([], []) == "PASS" and verdict_of([], ["x"]) == "PARTIAL" and verdict_of(["y"], ["x"]) == "FAIL", "self-test: the verdict"

    before = {"head": "h", "status": [], "untracked": [], "sha256": "x"}
    assert identity_reasons(before, dict(before)) == [], "self-test: an unchanged identity"
    assert identity_reasons(before, {**before, "status": [" M a.rs"]}) and "status" in identity_reasons(before, {**before, "status": [" M a.rs"]})[0], "self-test: an identity change"
    assert "untracked" in identity_reasons(before, {**before, "untracked": ["opencode.json"]})[0], "self-test: an untracked file change"
    assert re.fullmatch(r"ezsdr-v4-cargo-target-[0-9a-f]{12}", cargo_target_volume("/x/a")) and cargo_target_volume("/x/a") != cargo_target_volume("/x/b"), "self-test: one volume per checkout"
    assert parse_known("# c\n\nphase7/G09  race in stepping.rs\n") == {"phase7/G09": "race in stepping.rs"}, "self-test: known survivors file"
    assert sanitize({"p": HOME + "/x", "q": [HOME]}) == {"p": "~/x", "q": ["~"]}, "self-test: $HOME written as ~"

    with tempfile.TemporaryDirectory() as tmp, contextlib.redirect_stdout(io.StringIO()):
        out = Path(tmp)
        (out / "gate.json").write_text(json.dumps({"verdict": "PASS"}), encoding="utf-8")
        states = []

        def crash(failures):
            states.append(json.loads((out / "gate.json").read_text(encoding="utf-8"))["verdict"])
            raise OSError("injected launch failure")
        code = run_guarded(out, {"steps": []}, [], crash)
        manifest = json.loads((out / "gate.json").read_text(encoding="utf-8"))
        assert states == ["RUNNING"] and code == 1 and manifest["verdict"] == "FAIL" and "injected launch failure" in manifest["reasons"][0], "self-test: a stale PASS is replaced by RUNNING, then an exception is FAIL"
        assert sorted(p.name for p in out.iterdir()) == ["gate.json"], "self-test: no temporary file is left behind"
        assert run_guarded(out, {"steps": []}, ["not run: steps mutation"], lambda failures: None) == 0 and json.loads((out / "gate.json").read_text(encoding="utf-8"))["verdict"] == "PARTIAL", "self-test: a partial run exits 0 as PARTIAL"
        assert run_guarded(out, {"steps": []}, [], lambda failures: failures.append("x")) == 1 and json.loads((out / "gate.json").read_text(encoding="utf-8"))["verdict"] == "FAIL", "self-test: a failure is FAIL"
    print("gate.py self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
