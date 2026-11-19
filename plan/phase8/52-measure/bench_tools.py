#!/usr/bin/env python3
"""Bench-tool modules of crates/ezsdr-radio-uhd/tests/common: lines, mutation rows that mutate them, hardware tests
that use them (`mod::` or `common::mod::`, plus `use common::n47s5::*` users), how many of those hardware tests a bench
runbook (plan/phase7/bench.md, plan/phase8/bench.md) names, and fake.rs tests that use them. Run from the repo root.
Usage: python3 -I bench_tools.py"""
import json, os, re
hw = open('crates/ezsdr-radio-uhd/tests/hardware.rs').read()
fake = open('crates/ezsdr-radio-uhd/tests/fake.rs').read()
bench = open('plan/phase8/bench.md').read() + open('plan/phase7/bench.md').read()
def tests(src):
    out = {}
    pos = [m.start() for m in re.finditer(r'#\[test\]', src)] + [len(src)]
    for a, b in zip(pos, pos[1:]):
        m = re.search(r'fn ([a-z0-9_]+)\s*\(', src[a:b])
        if m: out[m.group(1)] = src[a:b]
    return out
H, F = tests(hw), tests(fake)
rows = {}
for l in ['maintenance','phase3','phase4','phase5','phase6','phase7']:
    for r in json.load(open(f'plan/{l}/tools/mutations.json')):
        rows[r['file']] = rows.get(r['file'], 0) + 1
glob_use = {'n47s5'}  # hardware.rs: use common::n47s5::*; also glob-imports common::raw, whose generic names (end, iq) cannot be matched this way: raw is read by hand
print("| module | lines | rows | hw tests using it | named in a runbook | fake.rs tests using it |")
print("|---|---|---|---|---|---|")
tot = [0, 0]
for f in sorted(os.listdir('crates/ezsdr-radio-uhd/tests/common')):
    if not f.endswith('.rs') or f == 'mod.rs': continue
    mod = f[:-3]; path = f'crates/ezsdr-radio-uhd/tests/common/{f}'
    pat = re.compile(r'(?<![a-z0-9_])' + mod + r'::')
    used = [n for n, b in H.items() if pat.search(b)]
    if mod in glob_use:
        names = set(re.findall(r'pub fn ([a-z0-9_]+)', open(path).read()))
        used = sorted(set(used) | {n for n, b in H.items() if any(re.search(r'\b' + x + r'\(', b) for x in names)})
    fk = [n for n, b in F.items() if pat.search(b)]
    lines = sum(1 for _ in open(path))
    print(f"| {mod} | {lines} | {rows.get(path, 0)} | {len(used)} | {sum(1 for n in used if n in bench)} | {len(fk)} |")
    tot[0] += lines; tot[1] += rows.get(path, 0)
print(f"| total (excl. mod.rs, measurement/) | {tot[0]} | {tot[1]} | | | |")
for sub in ['measurement.rs', 'measurement/multichannel.rs', 'measurement/h4.rs', 'mod.rs']:
    p = f'crates/ezsdr-radio-uhd/tests/common/{sub}'
    if os.path.exists(p): print(f"{sub}: {sum(1 for _ in open(p))} lines, {rows.get(p, 0)} rows")
