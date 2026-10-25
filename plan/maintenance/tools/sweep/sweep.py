#!/usr/bin/env python3
"""Test-line sweep: every rule's Test line and §6 row against the tests' real assertions.

Stage 1 (`sweep`): one fresh read-only DeepSeek run per chunk of ~15 rules, JSON out.
Stage 2 (`verify`): one fresh run per chunk's flagged findings, adversarial, JSON out.
`check` validates the JSON mechanically (test names exist, path:line in range).
Setup: git worktree add --detach ~/.cache/ezsdr-sweep/tree <commit>; mkdir ~/.cache/ezsdr-sweep/out.
SWEEP_MODEL overrides the model (the free tiers hit 429 under parallel load, 2026-10-15).
Usage: sweep.py chunks | sweep.py sweep [N [keys…]] | sweep.py verify [N [keys…]] | sweep.py check | sweep.py summary
"""
import json, os, re, subprocess, sys, time
from concurrent.futures import ThreadPoolExecutor

W = os.path.expanduser("~/.cache/ezsdr-sweep")  # outputs; TREE is a detached worktree of the commit swept
TREE = f"{W}/tree"
HERE = os.path.dirname(os.path.abspath(__file__))  # the two briefs live beside this script
MODEL = os.environ.get("SWEEP_MODEL", "opencode-go/deepseek-v4.1-flash")
RULE = re.compile(r"^- \*\*([A-Z]{2,3}-[0-9]+[a-z]?)\*\*", re.M)
CHUNK = 15


def chunks():
    out = []
    for f in sorted(os.listdir(f"{TREE}/design")):
        if not re.match(r"\d\d-.*\.md$", f):
            continue
        ids = RULE.findall(open(f"{TREE}/design/{f}").read())
        for i in range(0, len(ids), CHUNK):
            part = ids[i:i + CHUNK]
            out.append({"key": f"{f[:2]}-{i // CHUNK + 1}", "spec": f"design/{f}", "rules": part})
    return out


def run(prompt, log):
    if os.path.exists(log) and "===JSON END===" in open(log, errors="replace").read():
        return  # done before: rerun only what failed
    for attempt in range(8):  # the free tier answers "Rate limit exceeded": wait and retry
        with open(log, "w") as fh:
            subprocess.run(["opencode2", "run", "--standalone", "-m", MODEL, "--agent", "plan", "--auto", prompt],
                           cwd=TREE, stdout=fh, stderr=subprocess.STDOUT, timeout=3 * 3600)
        if "Rate limit exceeded" not in open(log, errors="replace").read()[-2000:]:
            return
        time.sleep(300)


def extract(log):
    # The answer is the last JSON object in the log that has `findings` or `verdicts`; the
    # markers are not trusted: the brief's own copy of them appears when a run reads it
    # (2026-10-25), and a run sometimes drops one or both.
    t = re.sub(r"\x1b\[[0-9;]*m", "", open(log, errors="replace").read()) if os.path.exists(log) else ""
    dec = json.JSONDecoder(strict=False)
    for m in reversed(list(re.finditer(r"^\s*\{", t, re.M))):
        try:
            d, _ = dec.raw_decode(t, m.end() - 1)
        except json.JSONDecodeError:
            continue
        if isinstance(d, dict) and ("findings" in d or "verdicts" in d) and "<rule ids with no finding>" not in json.dumps(d):
            return d
    return None


def sweep_prompt(c):
    return open(f"{HERE}/sweep-brief.md").read().replace("{SPEC}", c["spec"]).replace("{RULES}", ", ".join(c["rules"]))


def verify_prompt(c, flagged):
    return open(f"{HERE}/verify-brief.md").read().replace("{SPEC}", c["spec"]).replace("{FINDINGS}", json.dumps(flagged, ensure_ascii=False, indent=1))


def test_names():
    r = subprocess.run(["git", "grep", "-hoE", r"(fn|def) [a-z0-9_]+", "--", "crates", "python"], cwd=TREE, capture_output=True, text=True)
    return {l.split()[1] for l in r.stdout.splitlines()}


def evidence_ok(ev):
    m = re.match(r"([^:\s]+):(\d+)(?:[-–](\d+))?", ev)
    if not m:
        return False
    p = f"{TREE}/{m[1]}"
    return os.path.isfile(p) and int(m[3] or m[2]) <= sum(1 for _ in open(p, errors="replace"))


def flagged_of(data):
    return [f for f in data.get("findings", []) if f.get("verdict") in ("overclaim", "silent_gap", "wrong_test", "stale_none_named")]


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "chunks"
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 8
    cs = chunks()
    if len(sys.argv) > 3:  # optional chunk keys
        cs = [c for c in cs if c["key"] in sys.argv[3:]]
    if cmd == "chunks":
        for c in cs:
            print(c["key"], c["spec"], len(c["rules"]))
        print(len(cs), "chunks")
    elif cmd == "sweep":
        with ThreadPoolExecutor(n) as ex:
            list(ex.map(lambda c: run(sweep_prompt(c), f"{W}/out/sweep-{c['key']}.log"), cs))
    elif cmd == "verify":
        jobs = []
        for c in cs:
            d = extract(f"{W}/out/sweep-{c['key']}.log")
            if d and flagged_of(d):
                jobs.append((c, flagged_of(d)))
        with ThreadPoolExecutor(n) as ex:
            list(ex.map(lambda j: run(verify_prompt(*j), f"{W}/out/verify-{j[0]['key']}.log"), jobs))
    elif cmd == "check":
        names = test_names()
        assert len(names) > 1000, f"only {len(names)} test names found"
        for c in cs:
            d = extract(f"{W}/out/sweep-{c['key']}.log")
            if d is None:
                print(c["key"], "NO JSON")
                continue
            seen = {f.get("rule") for f in d.get("findings", [])} | set(d.get("rules_ok", []))
            missing = [r for r in c["rules"] if r not in seen]
            bad = [(f.get("rule"), t) for f in d.get("findings", []) for t in f.get("tests", []) if t not in names]
            badev = [(f.get("rule"), e) for f in d.get("findings", []) for e in f.get("evidence", []) if not evidence_ok(e)]
            if missing or bad or badev:
                print(c["key"], "missing", missing, "unknown tests", bad[:5], "bad evidence", badev[:5])
    elif cmd == "summary":
        rows = []
        for c in cs:
            d = extract(f"{W}/out/sweep-{c['key']}.log") or {}
            v = extract(f"{W}/out/verify-{c['key']}.log") or {}
            verdicts = {(x.get("rule"), x.get("id")): x for x in v.get("verdicts", [])}
            for f in flagged_of(d):
                x = verdicts.get((f.get("rule"), f.get("id")), {})
                rows.append({**f, "spec": c["spec"], "chunk": c["key"], "verify": x.get("verdict"), "verify_note": x.get("note")})
        json.dump(rows, open(f"{W}/summary.json", "w"), ensure_ascii=False, indent=1)
        from collections import Counter
        print(Counter((r["verdict"], r["verify"]) for r in rows))


if __name__ == "__main__":
    main()
