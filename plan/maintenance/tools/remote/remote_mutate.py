"""Run recorded mutation rows on remote hosts, sharded, each shard in its own container.

Usage (from the repository root):
    python3 plan/maintenance/tools/remote/remote_mutate.py <list> <host:shards>[,<host:shards>...] [ID ...]

<list> is `maintenance` or `phaseN` (its plan/<list>/tools/mutate.py and mutations.json).
Every selected row (all when no ID is given) is dealt round-robin to the shards; each shard
runs that list's own mutate.py in a container with its own scratch copy and its own Cargo
target directory, so no two builds share a directory (AGENTS.md §7). Keep the shards per
host well under its core count: a FakeDevice test failing under load would count a mutant
as killed. The tree is copied as it is on disk, uncommitted changes included. Prints each
shard's log and exits 1 unless every row is killed. Needs ssh and rsync locally, docker
without sudo on each host. `REMOTE_SSH` replaces the ssh command (for a tunnel's config file),
and `REMOTE_CPUS` is each shard's CPU limit (default 4). Standard library only.
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


def run(*args, **kw):
    return subprocess.run(args, check=True, text=True, **kw)


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    name, hosts = sys.argv[1], [(h.split(":")[0], int(h.split(":")[1])) for h in sys.argv[2].split(",")]
    rows = json.load(open(f"plan/{name}/tools/mutations.json", encoding="utf-8"))
    ids = [r["id"] for r in rows if not sys.argv[3:] or r["id"] in sys.argv[3:]]
    slots = [(host, i) for host, n in hosts for i in range(n)]
    shards = {slot: ids[k::len(slots)] for k, slot in enumerate(slots)}
    tag = f"{name}-{int(time.time())}"
    for host in {h for h, _ in hosts}:
        run(*SSH, host, f"mkdir -p {REMOTE}/src {REMOTE}/out && chmod 777 {REMOTE}/out")  # the container runs as its own user
        run("rsync", "-a", "--delete", "-e", " ".join(SSH), *[f"--exclude={e}" for e in EXCLUDES], "./", f"{host}:{REMOTE}/src/")
        run(*SSH, host, f"docker build -q -t {REMOTE} {REMOTE}/src/plan/maintenance/tools/remote >/dev/null")
    for (host, i), chunk in shards.items():
        if not chunk:
            continue
        inner = (f"cd /src && EZSDR_PYTHON=python3 python3 plan/{name}/tools/mutate.py plan/{name}/tools/mutations.json "
                 f"/tmp/scratch {' '.join(chunk)} > /out/{tag}-{i}.log 2>&1")
        run(*SSH, host, f"docker run -d --rm --cpus={CPUS} --name {REMOTE}-{tag}-{i} -v ~/{REMOTE}/src:/src:ro "
                         f"-v ~/{REMOTE}/out:/out {REMOTE} bash -c {json.dumps(inner)} >/dev/null")
    print(f"{len(ids)} rows on {len(slots)} shards ({tag})", flush=True)
    for host in {h for h, _ in hosts}:
        while run(*SSH, host, f"docker ps -q --filter name={REMOTE}-{tag}", capture_output=True).stdout.strip():
            time.sleep(20)
    outcomes = {}
    for host in {h for h, _ in hosts}:
        out = run(*SSH, host, f"cat {REMOTE}/out/{tag}-*.log 2>/dev/null || true", capture_output=True).stdout
        print(out, end="")
        for line in out.splitlines():
            if m := re.match(r"(\S+) .*: (killed|SURVIVED|NOT FOUND|COMPILE ERROR|BASELINE FAILED)", line):
                outcomes[m[1]] = m[2]
    missing = [i for i in ids if i not in outcomes]
    bad = sorted(i for i, o in outcomes.items() if o != "killed") + missing
    print(f"{sum(o == 'killed' for o in outcomes.values())}/{len(ids)} killed; not killed or missing: {bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
