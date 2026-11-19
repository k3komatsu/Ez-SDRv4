#!/usr/bin/env python3
"""Issues per mechanism in plan/maintenance/failure-mechanisms.md, and those >= #63. Usage (repo root): python3 -I mechanisms.py"""
import re
t = open('plan/maintenance/failure-mechanisms.md', encoding='utf-8').read()
for line in t.split('\n'):
    m = re.match(r'^\| `mech:([a-z-]+)` \| .*? \| (.*?) \| ', line)
    if m:
        nums = sorted(set(int(x) for x in re.findall(r'#(\d+)', m.group(2))))
        print(m.group(1), len(nums), [n for n in nums if n >= 63])
