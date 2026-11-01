#!/usr/bin/env python3
"""Run a build or test command for this checkout on a lab server instead of the local PC (owner,
2026-11-01: no build or test load on usrp-lnx03).

Usage (from the checkout's root or any directory inside it):
    python3 plan/maintenance/tools/remote/rdev.py [--host sim02] [--image IMG] -- <command...>
    e.g. rdev.py -- cargo test -p ezsdr-radio-uhd --test fake
         rdev.py -- cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings

Each call rsyncs the working tree (uncommitted changes included; .git, v3/, .cargo/ and build output
left out) to <host>:~/ezsdr-dev/<key>/, where <key> is the sha256 of the checkout path's first 12 hex
digits, then runs the command there in a container of IMG (default ezsdr-v4-dev:uhd4.10, which must
exist on the host) with the copy at /work, a per-checkout Cargo target volume at /cargo-target (which also keeps
the crate registry: a fresh volume elsewhere would be root-owned). The exit code is the command's. Standard library only.
shortcut: two calls for the same checkout at once share one copy and target; run them one at a time.
"""
import hashlib
import os
import shlex
import subprocess
import sys

IMAGE = "ezsdr-v4-dev:uhd4.10"
SSH = shlex.split(os.environ.get("REMOTE_SSH", "ssh -o BatchMode=yes"))
EXCLUDES = [".git", "v3", ".cargo", "target", "ezsdr-runs", "__pycache__"]


def key(root):
    return hashlib.sha256(root.encode()).hexdigest()[:12]


def remote_argv(host, root, command, image=IMAGE):
    k = key(root)
    docker = ["docker", "run", "--rm", "--user", f"{os.getuid()}:{os.getgid()}", "-e", "HOME=/tmp",
              "-v", f"$HOME/ezsdr-dev/{k}:/work", "-v", f"ezsdr-dev-target-{k}:/cargo-target",
              "-e", "CARGO_HOME=/cargo-target/cargo-home", "-w", "/work", image, *command]
    # $HOME expands on the remote shell; everything else is quoted.
    line = " ".join(a if a.startswith("$HOME/") else shlex.quote(a) for a in docker)
    return [*SSH, host, line]


def sync(host, root):
    k = key(root)
    subprocess.run([*SSH, host, f"mkdir -p ~/ezsdr-dev/{k}"], check=True)
    excludes = [f"--exclude=/{e}" if e != "__pycache__" else "--exclude=__pycache__" for e in EXCLUDES]
    subprocess.run(["rsync", "-a", "--delete", "--chmod=Dgo+rx,Fgo+r", *excludes, "-e", shlex.join(SSH),
                    f"{root}/", f"{host}:ezsdr-dev/{k}/"], check=True)


def main(argv):
    host, image = "sim02", IMAGE
    while argv and argv[0] != "--":
        flag = argv.pop(0)
        if flag == "--host":
            host = argv.pop(0)
        elif flag == "--image":
            image = argv.pop(0)
        elif flag == "--self-test":
            return self_test()
        else:
            sys.exit(f"rdev.py: unknown flag {flag}")
    if not argv[1:]:
        sys.exit(__doc__)
    root = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip()
    sync(host, root)
    return subprocess.run(remote_argv(host, root, argv[1:], image)).returncode


def self_test():
    argv = remote_argv("sim03", "/x", ["sh", "-c", "a && b"])
    assert argv[-2] == "sim03" and "$HOME/ezsdr-dev/" + key("/x") + ":/work" in argv[-1], argv
    assert "'a && b'" in argv[-1] and argv[-1].endswith(IMAGE + " sh -c 'a && b'"), argv
    print("rdev self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
