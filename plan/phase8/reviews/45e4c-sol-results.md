**Verdict: 11/20 found. The ≥16/20 criterion fails.** N01 and N20 both count; no row’s found/not-found classification needs changing. Excluding N05 gives 11/19, supplementary only.

Paths below: `E = ~/.cache/ezsdr-fixes/2026-11-01-n45/e4c`; `B = crates/ezsdr-radio-uhd/src/brain`.
**Row verification** — qualifying DST evidence, excluding earlier unit/thread failures; concurrent test output has no stable global “first” order:
- **N01 found:** `oracle_happy_path_passes`, P6a: an eligible head remains held (`E/logs/N01/suite.log:549–559`). This calls `explore` on a complete `World`, not a procedure-only assertion (`B/oracle.rs:1720–1731`). The default suite explicitly belongs to DST (`../brief-e4c.md:17–20`, relative to E). Count it; ignore the separate `u01c` panic.
- **N02 not found:** `u05a_stopping_cancels_queued_and_lists_obligations`, `[2]` instead of `[2,3]`; unit only (`E/logs/N02/suite.log:546–549`). Missing observation of an unused return value, not search exhaustion.
- **N03 not found:** `u03_cancel_before_take_drops_without_call`, `Call` instead of `Dropped`; unit only (`E/logs/N03/suite.log:546–549`). Missing queued-value-at-shortened-cut case.
- **N04 not found:** `u07a` first asserts Published instead of Stale; the walk failure is only unreachable W3b (`E/logs/N04/suite.log:546–549`; `dst-walks.log:56–57`). Model granularity: planning and commit share one event and use the live bound (`B/rules.rs:485`).
- **N05/N07/N11/N19 not found:** no scored translation, as ruled; all four remain in the denominator. N07d’s I19 detection adds no point (`E/rulings.md:6–7`).
- **N06 found:** `case8_lifecycle_across_loss_walks`, seed 1163, I14: lifecycle admission under G after F (`E/logs/N06/dst-walks.log:36`).
- **N08 found:** default `seeded_config_reduced`, I3: Move issued while pinned; walks independently agree (`E/logs/N08/suite.log:546–547`; `dst-walks.log:36`).
- **N09 found:** `case2_loss_everywhere_walks`, seed 1, I4: second disposition (`E/logs/N09/dst-walks.log:36`).
- **N10 not found:** all three runs accepted (`E/logs/N10/suite.log:542`; `dst-walks.log:32`; `dst-micro.log:50`). P1 checks delivered deadlines only (`B/oracle.rs:670–680`); the additional scheduling explanation remains **inference**, not demonstrated by clean logs.
- **N12 found:** `case11_shared_k_and_cancel_micro_full`, I11: reservation with device K already reached (`E/logs/N12/dst-micro.log:58–61`).
- **N13 found:** first walk failure block is `case11_shared_k_and_cancel_walks`, I11, seed 4; case10 also detects it (`E/logs/N13/dst-walks.log:35,1937–1938`).
- **N14 found:** `case11_shared_k_and_cancel_micro_full`, I11 (`E/logs/N14/dst-micro.log:58–61`).
- **N15 not found:** `b7_left_value_stays_resident_and_is_consumed_later`, incorrect D admission; unit only (`E/logs/N15/suite.log:546–549`). Missing saturation case with a resident left value.
- **N16 found:** `case7a_handoff_loss_late_restart_walks`, seed 163, P2: zero dispositions (`E/logs/N16/dst-walks.log:36`). Other failures do not add points.
- **N17 found:** `case12a_capacity_timer_stopping_walks`, seed 119, B8: capacity timer changes lifecycle state (`E/logs/N17/dst-walks.log:36`).
- **N18 found:** only `case2_loss_everywhere_micro_full`, I4: Loss report despite qualifying consumption evidence (`E/logs/N18/dst-micro.log:58–61`).
- **N20 found:** default `case15c_first_spawn_fails_micro` reaches `Incomplete: … teardown`; full micro and walks corroborate (`E/logs/N20/suite.log:551–554`). This is modelled shutdown failing to finish, **not** explorer budget exhaustion or a witness failure.

**Past bugs:** the 4 found / 4 structurally excluded / 2 unsolved classification stands, with the stated PB3 caveat.
- **PB8 genuinely found:** case15c, seed 1, I21: started stream stays Streaming instead of becoming Uncertain (`E/logs/PB8/dst-walks.log:35`). This exercises the spawn-failure wiring, not merely its helper.
- **PB7 genuinely found by I3:** case7a issues Start while the lost chain retains a pin (`E/logs/PB7/dst-micro.log:58–61`). HO11 separately finds I1; together they still represent one past-bug item.
- PB2b finds I13; PB6 finds P2; PB3 remains unit-only (`E/logs/PB2b/dst-walks.log:39`; `PB6/dst-micro.log:58`; `PB3/dst-micro.log:50`).

**Minor reporting corrections; none changes the score:**
1. `E/past-bugs.md:8` attributes all 5,047/5,033 failing seeds to I13. Actual splits are **2,956 I13 + 2,091 Incomplete**, and **2,925 I13 + 2,108 Incomplete** (`E/logs/PB2b/dst-walks.log:37–39` onward).
2. `E/holdout-results.md:34`: “D = 64” is not universal; case10 uses **D=2** (`B/cases.rs:381–394`). Retain the narrower missing-left-value-saturation explanation.
3. `E/past-bugs.md:14`: “no head-of-line holding across … values” is stale: reservation drains the shared held list from its head (`B/core.rs:708–725`). Missing record-fold semantics still justifies “not solved.”

**Baseline split:** acceptable; no scoring change warranted. Every row uses the same split commands (`E/run_row.py:33–35`); BASE accepts 456 default tests, 25 sweeps and 24 bounded searches (`E/logs/BASE/{suite.log:542,dst-walks.log:32,dst-micro.log:50}`). Different wall-budget coverage prevents claiming identical explored states to the killed combined run.

Not checked: reruns, hardware, remote-copy provenance, or the discarded SIGKILL run’s cause; no files or git state changed.
