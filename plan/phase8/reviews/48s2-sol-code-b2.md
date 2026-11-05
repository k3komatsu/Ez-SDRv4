
> plan · gpt-6.1-sol
What this change does: removes unused halt/configure-skip state and Mock’s empty section, and renames the TX cancellation reason. I found no P1/P2 correctness defect.

### Scenarios checked
- Refused RX start → Stop/start: prefix plans at 1,999 ms and 2,000 ms, refusal exclusion, admission state and pruning.
- Configure success, refusal and failed open: whether the owner repeats configuration or skips reopening.
- Clock ended at origin, insufficient first-send space, failed reopen → later burst: cancellation, reservation release and no payload sent.
- Mock sections, staged values, retained read-back evidence and mutation dispositions.

### Findings — P3

1. **The rewritten phase comment is false** (`crates/ezsdr-mock-radio/src/lib.rs:1809–1810`).
   It now says an untimed tune preserves phases generally. Stop → `start_rx` on a non-deterministic channel redraws them through `untimed_tune()` (`:1008–1015`), as MR-34 requires. Delete that sentence or qualify it as deterministic-only.

2. **The stale-contract sweep is incomplete**:
   - `design/05-module-api.md:236–237` still says MockRadio applies `hardware_timed` updates at their instant and cites MR-18 as its test contract. MR-18 now stages values for `start_rx`; remove those Mock-specific references while retaining generic UC-6.
   - `crates/ezsdr-acceptance/tests/parity.rs:12–13` still describes RM-34 channel moves as awaiting an already-finished implementation stage.
   - `design/18-uhd-radio.md:580` still claims the named parity tests compare channel joins and timed configuration changes. Narrow the row to their current assertions.

### Review conclusions
- A3’s required prefix-plan assertions **are present**, not replaced by `streaming()`. The fixed N6 plus seeded equivalence retarget is sufficient.
- B7’s production trace holds: successful configuration advances `from`; `Ok(false)` paths precede configuration. Dropping optional N8 is acceptable. “A duplicate configure is harmless” would not itself justify deletion.
- B5 preserves cancellation policy and counts; the new shared reason fits the retained paths.
- Mutation accounting matches: **7 deleted, 14 retargeted, 2 added**. No retained-code row was improperly dropped.
- `StreamState.configured` now only preserves otherwise-unused bookkeeping, but S10 explicitly requires retaining it; no removal requested here.

**Verdict: CODE NEEDS CHANGES — text-only corrections above; no runtime fix requested.**

Checked independently: `rules.py` and `git diff --check` pass. Tests, hardware and mutations were not rerun; supplied probe logs were inspected.
