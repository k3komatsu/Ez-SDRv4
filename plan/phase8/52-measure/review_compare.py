#!/usr/bin/env python3
"""Per step: plan-review rounds, sub-step reviews, final code review, re-checks (majors in order), and the steps with both
sub-step and final code reviews. Usage (from this directory): python3 -I review_compare.py"""
import sys, collections
sys.argv=['steps.py','churn.jsonl','review-counts.tsv']
exec(open('steps.py').read().split('agg = defaultdict')[0])
print("step | plan rounds: majors | sub-step: majors | final code: majors | re-checks: majors")
both=[]
for step, rv in sorted(reviews.items()):
    seq = collections.defaultdict(list)
    for s, m, mi, rest in rv:
        seq[kind(s)].append(m)
    print(f"{step} | {len(seq['plan'])}: {','.join(seq['plan'])} | {len(seq['substep'])}: {','.join(seq['substep'])} | {len(seq['code'])}: {','.join(seq['code'])} | {len(seq['recheck'])}: {','.join(seq['recheck'])}")
    sub=[toint(m) for m in seq['substep'] if toint(m) is not None]; code=[toint(m) for m in seq['code'] if toint(m) is not None]
    if sub and code: both.append((step, sum(sub), sum(code)))
print("steps with both:", len(both), "sub-step majors", sum(b[1] for b in both), "final code majors", sum(b[2] for b in both), "final found >=1:", sum(1 for b in both if b[2] > 0))
