"""Sweep production code with cargo-mutants on remote hosts, sharded, each shard in its own container
(note 52 D1: a sweep before the v4.0 freeze, not a gate; the hand-written rows stay the gate).

Usage (from the repository root). It takes no lock: run it under `~/.cache/ezsdr-fixes/tools/simq` whenever another lane
may use the sims, and never beside a gate or a full mutation run:
    python3 plan/maintenance/tools/remote/sweep_mutants.py list <target> <host>
    python3 plan/maintenance/tools/remote/sweep_mutants.py baselines <host>
    python3 plan/maintenance/tools/remote/sweep_mutants.py run  <target> <host:shards>[,<host:shards>...] [--out DIR] [-- <cargo-mutants args>]

Targets: `radio` (the `ezsdr-radio` crate), `uhd` (`crates/ezsdr-radio-uhd/src/uhd.rs` with feature `uhd`; its mutants
run the gate's `uhd-test` set, `--lib --test uhd_api`), `workspace` (every crate; `list` only: the feature `uhd` is
on). `list` prints cargo-mutants' mutant list and its count from one container; `baselines` prints each crate's own build and test seconds. `run` copies the tree, builds the
image and runs one container per shard exactly as remote_mutate.py does (its `upload`, its image: the same
Dockerfile), each with `cargo mutants --shard k/n` on a copy of the tree that cargo-mutants makes in the
container, never `--in-place` and never a working tree (AGENTS.md Safety core, §7), and fetches every shard's
`mutants.out` (without the per-mutant logs) into `--out` (default `~/.cache/ezsdr-fixes/mutants-sweep/<tag>`).
Each run builds its own image (`ezsdr-mut-<tag>`, removed at the end with its dir and containers, on every exit path; the tag carries a random id), waits for every container's exit code (cargo-mutants'
0, 2 and 3 are results; any other code is a failed shard), needs per shard a readable `outcomes.json` with at least one mutant, and refuses a non-empty `--out`;
it exits 1 on any failed or missing shard, and so does `list` or `baselines` when a remote command fails.
It prints the totals, the wall time and the shards x CPUs it used. Keep shards x REMOTE_CPUS (default 4) within a
host's 32 cores (§7). `REMOTE_SSH` as for remote_mutate.py. Standard library only.
"""
import json
import os
import shlex
import subprocess
import sys
import time

import remote_mutate as rm

# cargo-mutants arguments per target, then what follows `--` (cargo test's own: the tests every mutant runs).
TARGETS = {
    "radio": (["-p", "ezsdr-radio"], []),
    "uhd": (["-p", "ezsdr-radio-uhd", "--file", "crates/ezsdr-radio-uhd/src/uhd.rs", "--features", "uhd"], ["--lib", "--test", "uhd_api"]),
    "workspace": (["--workspace", "--features", "ezsdr-radio-uhd/uhd"], []),
}
# CARGO_TARGET_DIR is the dev image's shared /cargo-target: unset, so each shard's copy builds in itself.
PRE = "cd /src && env -u CARGO_TARGET_DIR cargo mutants -d /src"


def command(target, extra, shard=None, out=None):
    args, tests = TARGETS[target]
    shard = ["--shard", f"{shard[0]}/{shard[1]}"] if shard else []
    return " ".join([PRE, *args, *extra, *shard, *(["--output", out] if out else []), *(["--", *tests] if tests else [])])


def list_mutants(target, host):
    work = f"{rm.REMOTE}/{rm.new_tag('list')}"
    try:
        image = rm.upload(host, work)
        inner = command(target, ["--list"])
        done = subprocess.run([*rm.SSH, host, f"docker run --rm --cpus={rm.CPUS} --name {rm.REMOTE}-{work.split('/')[-1]} -v ~/{work}/src:/src:ro {image} bash -c {json.dumps(inner)}"],
                              text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    finally:
        rm.cleanup_all([host], work)
    out = done.stdout
    print(out, end="")
    print(f"mutants listed for {target}: {sum(1 for line in out.splitlines() if ':' in line and not line.startswith(('error', 'warning')))}")
    return 1 if done.returncode else 0


def baselines(host):
    """Each crate's own test seconds (and its incremental build seconds on one shared target, so a crate is built after its dependencies),
    one crate after another in one container: a mutant of that crate costs about these plus the rebuild of what depends on it."""
    work = f"{rm.REMOTE}/{rm.new_tag('baselines')}"
    try:
        return baselines_run(host, work)
    finally:
        rm.cleanup_all([host], work)


def baselines_run(host, work):
    image = rm.upload(host, work)
    inner = ("cd /src && for c in $(ls crates); do s=$(date +%s); cargo test -q -p $c --no-run >/dev/null 2>&1 || { echo \"$c: build FAILED\"; exit 1; }; m=$(date +%s); "
             "cargo test -q -p $c >/dev/null 2>&1 || { echo \"$c: test FAILED\"; exit 1; }; e=$(date +%s); echo \"$c build $((m-s)) s, test $((e-m)) s\"; done")
    done = subprocess.run([*rm.SSH, host, f"docker run --rm --cpus={rm.CPUS} --name {rm.REMOTE}-{work.split('/')[-1]} -v ~/{work}/src:/src:ro {image} bash -c {shlex.quote(inner)}"],
                          text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)  # shlex: the remote shell must not expand `$c`
    print(done.stdout, end="")
    return 1 if done.returncode else 0


def shard_outcomes(path):
    """The mutant outcomes (the baseline excluded) of one `outcomes.json`; None when the file is missing or malformed."""
    try:
        found = [o for o in json.load(open(path, encoding="utf-8"))["outcomes"] if o["scenario"] != "Baseline"]
        if not all(isinstance(o.get("summary"), str) for o in found):
            return None
        return found
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        return None


def summarize(out_dir):
    """Counts and per-mutant seconds from every readable `outcomes.json` fetched under `out_dir`."""
    total, seconds, seen = {}, [], 0
    for root, _, files in os.walk(out_dir):
        found = shard_outcomes(os.path.join(root, "outcomes.json")) if "outcomes.json" in files else None
        if found is None:
            continue
        seen += 1
        for o in found:
            total[o["summary"]] = total.get(o["summary"], 0) + 1
            seconds.append(sum(p.get("duration", 0) if isinstance(p.get("duration"), (int, float)) else 0 for p in o.get("phase_results", [])))
    print(f"shards with results: {seen}; outcomes: {total}; mutants run: {len(seconds)}")
    return total, seconds


def sweep(target, spec, out_dir, extra):
    if os.path.exists(out_dir) and os.listdir(out_dir):
        print(f"refusing a non-empty --out: {out_dir}")  # stale results must not count
        return 1
    hosts = [(h.split(":")[0], int(h.split(":")[1])) for h in spec.split(",")]
    slots = [(host, i) for host, n in hosts for i in range(n)]
    uniq = list(dict.fromkeys(h for h, _ in hosts))
    tag = rm.new_tag(f"sweep-{target}")
    work = f"{rm.REMOTE}/{tag}"
    started = time.time()
    image = rm.image_of(work)
    failed = []  # a container is kept (no --rm) so `docker wait` can give its exit code
    try:  # whatever fails, every host's containers, dir and image are removed
        for host in uniq:
            rm.upload(host, work)
        for k, (host, i) in enumerate(slots):
            inner = command(target, extra, (k, len(slots)), f"/out/{i}") + f" > /out/{i}.log 2>&1"
            rm.run(*rm.SSH, host, f"docker run -d --cpus={rm.CPUS} --name {rm.REMOTE}-{tag}-{i} -v ~/{work}/src:/src:ro "
                                  f"-v ~/{work}/out:/out {image} bash -c {json.dumps(inner)} >/dev/null")
        print(f"{target}: {len(slots)} shards x {rm.CPUS} CPUs ({tag})", flush=True)
        for host, i in slots:
            name = f"{rm.REMOTE}-{tag}-{i}"
            code = rm.run(*rm.SSH, host, f"docker wait {name}; docker rm {name} >/dev/null", capture_output=True).stdout.split()[0]
            if code not in ("0", "2", "3"):  # cargo-mutants: 0 all caught, 2 missed, 3 timeouts; 1 and 4 are errors
                failed.append(f"{host}/{i}: exit {code}")
        wall = time.time() - started
        for host in uniq:
            os.makedirs(f"{out_dir}/{host}", exist_ok=True)
            rm.run("rsync", "-a", "-e", " ".join(rm.SSH), "--exclude=log", "--exclude=*.tmp", f"{host}:{work}/out/", f"{out_dir}/{host}/")
    finally:
        rm.cleanup_all(uniq, work)
    total, seconds = summarize(out_dir)
    for host, i in slots:  # one complete result per requested shard: a readable outcomes.json with at least one mutant
        if not shard_outcomes(f"{out_dir}/{host}/{i}/mutants.out/outcomes.json"):
            failed.append(f"{host}/{i}: no mutant outcomes (missing, malformed or empty outcomes.json)")
    if failed:
        print(f"FAILED shards: {failed}")
    print(f"wall {wall:.0f} s on {len(slots)} shards x {rm.CPUS} CPUs ({len(slots) * int(rm.CPUS)} cores); mean {sum(seconds) / max(len(seconds), 1):.1f} s per mutant (shard time, one core set)")
    return 1 if failed or not total else 0


def main():
    argv = sys.argv[1:]
    extra = argv[argv.index("--") + 1:] if "--" in argv else []
    argv = argv[:argv.index("--")] if "--" in argv else argv
    if len(argv) == 2 and argv[0] == "baselines":
        return baselines(argv[1])
    if len(argv) >= 3 and argv[0] == "list" and argv[1] in TARGETS:
        return list_mutants(argv[1], argv[2])
    out = os.path.expanduser("~/.cache/ezsdr-fixes/mutants-sweep/" + rm.new_tag("run"))
    if "--out" in argv:
        out = os.path.abspath(argv[argv.index("--out") + 1])
        del argv[argv.index("--out"):argv.index("--out") + 2]
    if len(argv) == 3 and argv[0] == "run" and argv[1] in TARGETS and argv[1] != "workspace":
        return sweep(argv[1], argv[2], out, extra)
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
