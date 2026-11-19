#!/usr/bin/env python3
"""Review-table rows (review-counts.tsv, column 5) that name each mechanism, with their majors. Usage: python3 -I mech_findings.py"""
import collections
mechs = ["planned-state-read-as-actual", "radio-rule-implemented-twice", "stream-lifecycle-scattered", "failure-collapsed-into-value",
         "correlation-by-partial-key", "pending-provenance-collapsed", "admission-per-origin", "supplied-callback-under-lock",
         "wall-clock-test-oracle", "device-state-outlives-call", "refusal-where-late-policy-decides", "invariant-outside-type"]
rows = collections.Counter(); maj = collections.Counter(); steps = collections.defaultdict(set)
for line in open("review-counts.tsv", encoding="utf-8"):
    step, src, m, mi, rest = (line.rstrip("\n").split("\t") + [""] * 5)[:5]
    for k in mechs:
        if k in rest:
            rows[k] += 1; steps[k].add(step)
            maj[k] += int(m) if m.isdigit() and int(m) < 100 else 0
for k in mechs:
    if rows[k]: print(f"{k}: rows {rows[k]}, majors {maj[k]}, steps {sorted(steps[k])}")
