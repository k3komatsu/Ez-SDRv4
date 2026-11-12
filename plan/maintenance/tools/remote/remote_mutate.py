"""Run recorded mutation rows on remote hosts, sharded, each shard in its own container.

Usage (from the repository root):
    python3 plan/maintenance/tools/remote/remote_mutate.py <lists> <host:shards>[,<host:shards>...] [ID ...]
    python3 plan/maintenance/tools/remote/remote_mutate.py --e2e <host:shards>[,...]

<lists> is `maintenance`, `phaseN`, a comma-separated set of them, or `all` (the six lists);
every list is run by plan/maintenance/tools/mutate.py with its own mutations.json. Every
selected row (all when no ID is given; an ID selects that row in every named list that has
it) is dealt round-robin to the shards; each shard is one container, with its own scratch
copy and its own Cargo target directory, that runs its rows list by list, so a shard builds
the workspace once for all the lists it runs and no two shards share a build directory
(AGENTS.md §7). Each run uploads into a directory of its own on each host
(`~/ezsdr-mut/<tag>`) and deletes it at the end, so two runs never overwrite each other.
The tree is copied as it is on disk, uncommitted changes included. Prints each shard's log
and exits 1 unless every row is killed. Needs ssh and rsync locally, docker without sudo on
each host. `REMOTE_SSH` replaces the ssh command (for a tunnel's config file), and
`REMOTE_CPUS` is each shard's CPU limit (default 4). Results are read from the `EZSDR-ROW {...}` line
mutate.py prints per row, never from the free-text line (#91). `--e2e` runs `mutate.py --self-test` and
`mutate.py --e2e` (the infra fixtures, through the same `cargo_test()` the rows use) in one container of
the first host, and prints `e2e: N/M ok; failed: [...]; self-test: ok`. Standard library only.
"""
import json
import os
import re
import subprocess
import sys
import time

REMOTE = "ezsdr-mut"
SSH = os.environ.get("REMOTE_SSH", "ssh").split()  # e.g. "ssh -F tunnels.conf"
CPUS = os.environ.get("REMOTE_CPUS", "4")  # per shard
EXCLUDES = [".git", "target", "v3", "tmp", ".cargo", "ezsdr-runs", ".claude", "__pycache__"]
ALL = ["maintenance", "phase3", "phase4", "phase5", "phase6", "phase7"]
KILLED = ("killed", "killed (bound)")  # `killed (bound)`: the row declares that a real-time bound is its kill (mutate.py)
PASSED = KILLED + ("equivalent (survived)",)  # a recorded equivalent must survive (mutate.py)
ROW = re.compile(r"^EZSDR-ROW (\{.*\})$")
E2E_LINE = re.compile(r"^EZSDR-E2E (\{.*\})$")
HEADER = re.compile(r"^--- (\S+) \((\S+)\)$")


def fixed_field(pattern, line):
    """The JSON object of a `EZSDR-ROW`/`EZSDR-E2E` line, or None for any other line (and for a malformed one)."""
    m = pattern.match(line)
    try:
        found = json.loads(m[1]) if m else None
    except ValueError:
        return None
    return found if isinstance(found, dict) else None


def row_record(line):
    """The row's fields (`id`, `outcome`, `real_clock`, `reruns`, and `why` for an ERROR) from a mutate.py `EZSDR-ROW` line; None for other lines."""
    found = fixed_field(ROW, line)
    return found if found and isinstance(found.get("id"), str) and isinstance(found.get("outcome"), str) else None


def row_outcome(line):
    """(id, outcome) for a mutate.py row line; None for other lines. gate.py reads the results through this
    too, so the two cannot disagree about what passes."""
    found = row_record(line)
    return (found["id"], found["outcome"]) if found else None


def run(*args, **kw):
    return subprocess.run(args, check=True, text=True, **kw)


def upload(host, work):
    run(*SSH, host, f"mkdir -p {work}/src {work}/out && chmod 777 {work}/out")  # the container runs as its own user
    run("rsync", "-a", "--chmod=Dgo+rx,Fgo+r", "--delete",  # a 0600 file (umask 0077) is unreadable to the container user
        "-e", " ".join(SSH), *[f"--exclude={e}" for e in EXCLUDES], "./", f"{host}:{work}/src/")
    run(*SSH, host, f"docker build -q -t {REMOTE} {work}/src/plan/maintenance/tools/remote >/dev/null")


def e2e(spec):
    """One container on the first host runs mutate.py's self-test and its --e2e cases in the image the rows run in."""
    host = spec.split(",")[0].split(":")[0]
    work = f"{REMOTE}/e2e-{int(time.time())}"
    upload(host, work)
    inner = ("cd /src && export EZSDR_PYTHON=python3; python3 plan/maintenance/tools/mutate.py --self-test; "
             "python3 plan/maintenance/tools/mutate.py --e2e /tmp/scratch")
    out = subprocess.run([*SSH, host, f"docker run --rm --cpus={CPUS} -v ~/{work}/src:/src:ro {REMOTE} bash -c {json.dumps(inner)}"],
                         text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT).stdout
    run(*SSH, host, f"rm -rf {work}")
    print(out, end="")
    cases = [c for c in (fixed_field(E2E_LINE, line) for line in out.splitlines()) if c and "id" in c]
    bad = [c["id"] for c in cases if c.get("ok") is not True]
    selftest = "mutate.py self-test: ok" in out.splitlines()
    print(f"e2e: {len(cases) - len(bad)}/{len(cases)} ok; failed: {bad}; self-test: {'ok' if selftest else 'FAILED'}")
    return 0 if cases and not bad and selftest else 1


def main():
    if sys.argv[1:2] == ["--e2e"] and len(sys.argv) == 3:
        return e2e(sys.argv[2])
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    names = ALL if sys.argv[1] == "all" else sys.argv[1].split(",")
    hosts = [(h.split(":")[0], int(h.split(":")[1])) for h in sys.argv[2].split(",")]
    wanted = set(sys.argv[3:])
    # Rows are keyed by (list, id): an id can occur in two lists (maintenance's G rows and
    # phase7's, for example).
    rows = [(name, r["id"]) for name in names
            for r in json.load(open(f"plan/{name}/tools/mutations.json", encoding="utf-8"))
            if not wanted or r["id"] in wanted]
    slots = [(host, i) for host, n in hosts for i in range(n)]
    shards = {slot: rows[k::len(slots)] for k, slot in enumerate(slots)}
    tag = f"{'-'.join(names) if len(names) < 3 else 'multi'}-{int(time.time())}"
    work = f"{REMOTE}/{tag}"
    for host in {h for h, _ in hosts}:
        upload(host, work)
    for (host, i), chunk in shards.items():
        if not chunk:
            continue
        steps = []
        for name in names:
            ids = [rid for n, rid in chunk if n == name]
            if ids:
                steps.append(f"python3 plan/maintenance/tools/mutate.py plan/{name}/tools/mutations.json "
                             f"/tmp/scratch {' '.join(ids)} > /out/{i}-{name}.log 2>&1")
        inner = "cd /src && export EZSDR_PYTHON=python3; " + "; ".join(steps)
        run(*SSH, host, f"docker run -d --rm --cpus={CPUS} --name {REMOTE}-{tag}-{i} -v ~/{work}/src:/src:ro "
                         f"-v ~/{work}/out:/out {REMOTE} bash -c {json.dumps(inner)} >/dev/null")
    print(f"{len(rows)} rows of {', '.join(names)} on {len(slots)} shards ({tag})", flush=True)
    for host in {h for h, _ in hosts}:
        while run(*SSH, host, f"docker ps -q --filter name={REMOTE}-{tag}", capture_output=True).stdout.strip():
            time.sleep(20)
    outcomes = {}
    for host in {h for h, _ in hosts}:
        for name in names:
            out = run(*SSH, host, f"cat {work}/out/*-{name}.log 2>/dev/null || true", capture_output=True).stdout
            if out:
                print(f"--- {name} ({host})\n{out}", end="")
            for line in out.splitlines():
                if found := row_outcome(line):
                    outcomes[(name, found[0])] = found[1]
        run(*SSH, host, f"rm -rf {work}")
    missing = [row for row in rows if row not in outcomes]
    bad = sorted(row for row, o in outcomes.items() if o not in PASSED) + missing
    for name in names:
        n = [row for row in rows if row[0] == name]
        ok = sum(outcomes.get(row) in PASSED for row in n)
        print(f"{name}: {ok}/{len(n)} passed; not passed or missing: {[rid for nm, rid in bad if nm == name]}")
    print(f"{sum(o in PASSED for o in outcomes.values())}/{len(rows)} passed; not passed or missing: {[f'{n}/{r}' for n, r in bad]}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
