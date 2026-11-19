#!/bin/sh
# Every measurement of the Phase 8-end draft, rerunnable. Read-only on the repository. Baseline of the first run: main 4679abc1 in
# the worktree ~/local_work/Ez-SDRv4-n111 (identical to origin/main on 2026-11-20). Run from this directory (plan/phase8/52-measure/); writes its outputs here (they are not committed).
# Usage: sh commands.sh [repository root] [end commit, default HEAD]   (at 8b's end: EZSDR_8B_START=<the day after Phase 8's Gate X> sh commands.sh ../../.. HEAD, so steps.md shows 8b as its own cohort)
R="$(cd "${1:-../../..}" && pwd)"   # the repository root (default: this directory's repository)
E="$(git -C "$R" rev-parse --short=8 "${2:-HEAD}")"   # the end commit ("now"; default HEAD). Phase 8's end stays in the lists as 4679abc1
export PYTHONDONTWRITEBYTECODE=1
M="$(pwd)"
python3 -I churn.py $R ec95e4b2 $E > churn.jsonl                 # per first-parent commit since Phase 7 Gate X (~1.5 min)
for r in ec95e4b2 03b8f94d 4679abc1 $E; do python3 -I totals.py $R $r > totals-$r.md; done   # Gate X, Phase 8 stage 1a, Phase 8 end, now
python3 -I rows_by_target.py $R $E > rows-by-target-$E.txt
python3 -I review_counts.py $R > review-counts.tsv
python3 -I steps.py churn.jsonl review-counts.tsv > steps.md
python3 -I review_compare.py > plan-rounds.txt
python3 -I crate_churn.py churn.jsonl > crate-churn.txt
python3 -I mech_findings.py > mech-findings.txt
python3 -I gate_times.py $R > gate-times.tsv
(cd $R && python3 -I "$M/bench_tools.py") > bench-tools.txt
(cd $R && python3 -I "$M/mechanisms.py") > mechanisms-phase8.txt
(cd $R && for r in ec95e4b2 03b8f94d d5498593 26350523 d8f23afb 24ab6f40 4679abc1 $E; do
  printf "%s %s rust#[test]=%s ignored=%s pytest=%s\n" $r "$(git show -s --format=%ad --date=short $r)" \
    "$(git grep -h -c -E '^\s*#\[test\]' $r -- crates | paste -sd+ | bc)" \
    "$(git grep -h -c -E '^\s*#\[ignore' $r -- crates | paste -sd+ | bc)" \
    "$(git grep -h -c -E '^\s*def test_' $r -- python | paste -sd+ | bc)"; done) > test-counts.txt
(cd $R && git grep -n -i "phase 8-end" origin/main -- plan; git grep -n -E "締めの振り返り|Phase 8 の締め|8-end|Phase 8 end" origin/main -- plan handoff.md AGENTS.md crates design KNOWN_LIMITATIONS.md) > debts-grep.txt
{ echo '$ gh issue list -R k3komatsu/Ez-SDRv4 --state all --limit 200 --json number,title,createdAt,closedAt,state,labels'; gh issue list -R k3komatsu/Ez-SDRv4 --state all --limit 200 --json number,title,createdAt,closedAt,state,labels; } > issues-2026-11-20.json.txt
# Per-binary test time of the last gate: grep -E "Running |test result:" ~/.cache/ezsdr-fixes/gates/n110/test.log | paste - -
# Step spans (rough): find ~/.cache/ezsdr-fixes/<date>-<step> -type f -printf '%T@\n' | sort -n | sed -n '1p;$p'
