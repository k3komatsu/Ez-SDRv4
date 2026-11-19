#!/usr/bin/env python3
"""Group churn.jsonl (churn.py) by landed step and join the review count tables (review-counts.tsv).

Usage (rerun, from this directory):  python3 -I steps.py churn.jsonl review-counts.tsv > steps.md
Step of a commit: the prefix (before the first '-') of the plan/phase8/reviews/ files the commit adds; with
several prefixes, the one the subject names; commits that add none are 'unlabelled' and listed by subject.
Excluded from the sums: 245dbf0f (rustfmt of the whole tree, 2026-10-31) and the brain category (note 45).
Cohorts: P0 = before Phase 8 (10-01..10-16), P8a = 10-17..11-06 (steps could land as several commits),
P8b = 11-07..11-20 (each step one squashed commit, AGENTS §9).
Review kinds from the Source cell: design (Fable / 'design'), plan ('plan' / 'brief'), substep (sub-step names,
'S1', 'B2', '4a' …), code ('code'), recheck ('re-check'), other.
"""
import os
import json
import re
import statistics
import sys
from collections import Counter, defaultdict

churn = [json.loads(l) for l in open(sys.argv[1])]
reviews = defaultdict(list)
for line in open(sys.argv[2], encoding="utf-8"):
    step, source, major, minor, rest = (line.rstrip("\n").split("\t") + [""] * 5)[:5]
    reviews[step].append((source, major, minor, rest))

EXCLUDE = {"245dbf0f"}
PRODC = ("prod", "uhdffi")
TESTC = ("utest", "itest", "icommon")


def norm(s):
    return re.sub(r"[^a-z0-9]", "", s.lower())


def step_of(d):
    keyed = sorted(set(re.split(r"-(dispositions|gate)", r)[0] for r in d["records"] if re.search(r"-(dispositions|gate)", r)))
    keyed = [k for k in keyed if not k.startswith(("bench-prep", "a-gate"))] or keyed
    if len(keyed) == 1:
        return keyed[0]
    prefs = sorted(set(r.split("-")[0] for r in d["records"]))
    prefs = [p for p in prefs if p not in ("workflow", "flaky", "bench")] or prefs
    if not prefs:
        return None
    if len(prefs) == 1:
        return prefs[0]
    subj = norm(d["subject"])
    named = [p for p in prefs if norm(p) in subj]
    return (named or prefs)[0]


def kind(source):
    """design / plan / substep / code (Sol's, or the step's one independent code review) / recheck / other."""
    s = source.lower()
    if "fable" in s or "design" in s or "direction" in s:
        return "design"
    if "gate" in s or "implementer" in s or "owner" in s or "orchestrator" in s:
        return "other"
    if "sub-step" in s or "self-review" in s or "haiku" in s or "sonnet" in s:
        return "substep"
    if "plan" in s or "brief" in s:
        return "plan"
    if "re-check" in s or "recheck" in s or "text check" in s:
        return "recheck"
    if re.search(r"\bcode\b", s) or "independent review" in s:
        return "code" if ("sol" in s or "independent" in s) else "substep"
    return "substep"


def reviewer(source):
    """The reviewer named at the head of the Source cell (before its first comma): "Opus, re-check of Sol's fixes" is
    Opus's; "Opus + Terra re-check, Sol fix round 1" is both."""
    s = source.lower().split(",")[0]
    found = [k for k in ("sol", "opus", "terra", "fable", "haiku", "sonnet", "self-review", "deepseek") if k in s]
    if "opus" in found and "terra" in found:
        return "opus+terra"
    return found[0] if found else "other"


def toint(x):
    try:
        return int(x)
    except ValueError:
        return None


agg = defaultdict(lambda: {"cats": defaultdict(lambda: [0, 0]), "tests": Counter(), "mut": Counter(), "date": None, "commits": 0, "subject": ""})
order = []
for d in churn:
    if d["sha"][:8] in EXCLUDE:
        continue
    s = step_of(d)
    key = s or ("~" + d["sha"][:8])
    a = agg[key]
    if key not in order:
        order.append(key)
        a["date"] = d["date"]
        a["subject"] = d["subject"]
    a["commits"] += 1
    for c, (ad, de) in d["cats"].items():
        a["cats"][c][0] += ad
        a["cats"][c][1] += de
    a["tests"].update(d["tests"])
    for v in d["mut"].values():
        for k in ("added", "removed", "changed", "anchor", "mutant", "killer"):
            a["mut"][k] += v.get(k, 0)


PHASE_8B = os.environ.get("EZSDR_8B_START", "9999-12-31")  # first day of Phase 8b (the day after Phase 8's Gate X); "P8b" is Phase 8's late period


def cohort(date):
    if date >= PHASE_8B:
        return "8b"
    return "P0" if date < "2026-10-17" else ("P8a" if date < "2026-11-07" else "P8b")


def s2(c, a):
    return sum(a["cats"].get(k, [0, 0])[0] + a["cats"].get(k, [0, 0])[1] for k in c)


print("| step | date | commits | prod +/− | fake +/− | tests +/− | t÷p | rows +/−/~ | tests fn +/−/ren/~ | review rows | majors (plan/sub/code/re/design) |")
print("|---|---|---|---|---|---|---|---|---|---|---|")
tot = defaultdict(Counter)
ratios = defaultdict(list)
for key in order:
    a = agg[key]
    if key.startswith("~") and s2(PRODC + TESTC + ("fake",), a) == 0 and not a["mut"]:
        continue
    pa = sum(a["cats"].get(k, [0, 0])[0] for k in PRODC)
    pd = sum(a["cats"].get(k, [0, 0])[1] for k in PRODC)
    fa, fd = a["cats"].get("fake", [0, 0])
    ta = sum(a["cats"].get(k, [0, 0])[0] for k in TESTC)
    td = sum(a["cats"].get(k, [0, 0])[1] for k in TESTC)
    ratio = (ta + td) / (pa + pd) if pa + pd else None
    rv = reviews.get(key, [])
    maj = Counter()
    for src, m, mi, rest in rv:
        v = toint(m)
        if v is not None and v < 100:
            maj[kind(src)] += v
    co = cohort(a["date"])
    t = tot[co]
    t["steps"] += 1
    t["pa"] += pa; t["pd"] += pd; t["fa"] += fa; t["fd"] += fd; t["ta"] += ta; t["td"] += td
    t["ra"] += a["mut"]["added"]; t["rr"] += a["mut"]["removed"]; t["rc"] += a["mut"]["changed"]
    t["fn+"] += a["tests"]["added"]; t["fn-"] += a["tests"]["removed"]; t["fnr"] += a["tests"]["renamed"]; t["fn~"] += a["tests"]["changed"]
    if rv:
        t["reviewed"] += 1
        t["rounds"] += len(rv)
        for k, v in maj.items():
            t["maj_" + k] += v
    if ratio is not None and pa + pd >= 50:
        ratios[co].append(ratio)
    label = key if not key.startswith("~") else "· " + a["subject"][:40]
    print(f"| {label} | {a['date'][5:]} | {a['commits']} | {pa}/{pd} | {fa}/{fd} | {ta}/{td} | {'' if ratio is None else f'{ratio:.1f}'} | {a['mut']['added']}/{a['mut']['removed']}/{a['mut']['changed']} | {a['tests']['added']}/{a['tests']['removed']}/{a['tests']['renamed']}/{a['tests']['changed']} | {len(rv) or ''} | {'/'.join(str(maj[k]) for k in ('plan', 'substep', 'code', 'recheck', 'design')) if rv else ''} |")

print()
print("## Review rows by kind and reviewer (steps with a standard count table)")
kc = defaultdict(Counter)
for step, rv in reviews.items():
    for src, m, mi, rest in rv:
        k, r = kind(src), reviewer(src)
        v = toint(m)
        kc[(k, r)]["rows"] += 1
        if v is not None and v < 100:
            kc[(k, r)]["majors"] += v
            kc[(k, r)]["rows_counted"] += 1
            kc[(k, r)]["with_major"] += 1 if v > 0 else 0
        mv = toint(mi)
        if mv is not None and mv < 100:
            kc[(k, r)]["minors"] += mv
print("| kind | reviewer | rows | rows with a number | rows with ≥1 major | majors | minors |")
print("|---|---|---|---|---|---|---|")
for (k, r), c in sorted(kc.items()):
    print(f"| {k} | {r} | {c['rows']} | {c['rows_counted']} | {c['with_major']} | {c['majors']} | {c['minors']} |")
print()
print("## Cohorts (excluding 245dbf0f; brain excluded)")
print("| cohort | steps (rows) | prod +/− | fake +/− | tests +/− | tests÷prod (sum) | median t÷p (steps with ≥50 prod lines) | rows +/−/~ | test fns +/−/ren/~ | steps with count table | review rows | majors plan/sub/code/re/design |")
print("|---|---|---|---|---|---|---|---|---|---|---|---|")
for co in ("P0", "P8a", "P8b", "8b"):
    t = tot[co]
    r = (t["ta"] + t["td"]) / (t["pa"] + t["pd"]) if t["pa"] + t["pd"] else 0
    med = statistics.median(ratios[co]) if ratios[co] else 0
    print(f"| {co} | {t['steps']} | {t['pa']}/{t['pd']} | {t['fa']}/{t['fd']} | {t['ta']}/{t['td']} | {r:.1f} | {med:.1f} (n={len(ratios[co])}) | {t['ra']}/{t['rr']}/{t['rc']} | {t['fn+']}/{t['fn-']}/{t['fnr']}/{t['fn~']} | {t['reviewed']} | {t['rounds']} | {t['maj_plan']}/{t['maj_substep']}/{t['maj_code']}/{t['maj_recheck']}/{t['maj_design']} |")
