#!/usr/bin/env python3
"""Per-crate churn by cohort from churn.jsonl. Usage: python3 -I crate_churn.py churn.jsonl"""
import json, collections, sys
rows=[json.loads(l) for l in open(sys.argv[1])]
co=lambda d: "P0" if d['date']<"2026-10-17" else ("P8a" if d['date']<"2026-11-07" else "P8b")
agg=collections.defaultdict(lambda: collections.defaultdict(lambda:[0,0]))
for d in rows:
    if d['sha'].startswith('245dbf0f'): continue
    for cr, cats in d['crates'].items():
        for cat,(a,de) in cats.items():
            agg[(co(d),cr)][cat][0]+=a; agg[(co(d),cr)][cat][1]+=de
print("cohort crate | prod+ffi +/- | fake +/- | utest +/- | itest +/- | icommon +/- | tests changed ÷ prod changed")
for (c,cr),cats in sorted(agg.items()):
    g=lambda k: cats.get(k,[0,0])
    p=[g('prod')[0]+g('uhdffi')[0], g('prod')[1]+g('uhdffi')[1]]
    t=sum(sum(g(k)) for k in ('utest','itest','icommon'))
    if sum(p)+t < 300: continue
    print(f"{c} {cr} | {p[0]}/{p[1]} | {g('fake')[0]}/{g('fake')[1]} | {g('utest')[0]}/{g('utest')[1]} | {g('itest')[0]}/{g('itest')[1]} | {g('icommon')[0]}/{g('icommon')[1]} | {t/sum(p):.1f}" if sum(p) else f"{c} {cr} | 0/0 | tests {t}")
