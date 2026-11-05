# Note 48 S2 — Opus re-checks

The re-check reviewer was a Claude Opus subagent (`general-purpose`, effort high). It was read-only.

## Test plan rev 2 (after Sol's review of rev 1)

**Verdict: PLAN NEEDS CHANGES.** The fixes were small and named, so no further Sol round was needed. Rulings S1–S12 are in `48s2-dispositions.md`.

**Closed:** R5, R7, R10.

**Partly closed**

- R1: N10 checked `freq_at`, not the samples, so the plan must keep the observation through `rx_recv`. N11 had no row, and N1/N2 were defined only in rev 1.
- R2: the replacement at `tests/timeline.rs:540` used `streaming()`. At 1 999 ms `streaming()` is true, because a refused S1 is replayed. It should use the prefix plan through `made()`.
- R4: the P7 assertion at `fake.rs` 7693–7695 is about the timed command (`d9.rs:383` passes `Some(e)`), so it must be deleted, not kept.
- R6: UR-22 (`design/18:249`) and further lines were missing. These are `design/18` :30, :193, :362–363, :368 and §6 rows, `design/07` :378/:384 counts, and `KNOWN_LIMITATIONS.md:67`.
- R8: reject `FirstSendShort` and use one `ClockEnded`.
- R9: the claim "no origin test can kill ceil→floor" is false. Choose values whose ceil and floor differ.

**Open: R3**

- These rows had no disposition: P215 and P219 (retarget), P990, P862/P895/P911 (delete), and P369 (`new` → `Owner::Configure`).
- These tests touched deleted code but were not mapped: `rx.rs:850`, `mod.rs:871`, `tx.rs:1344`, `fake.rs:7631`, `fake.rs:6514`.
- Recommendation: rows only in deleted code go to a mechanical sweep. Rows on retained code, and every deleted test, are decided in the plan.

**Answers to the implementer's questions**

- Q6: delete P863. Its `old2` is in a queue-only helper.
- Q7: keep P985 and retarget it. `Duc::packet`'s `at` drives the stale-header state.
- Q8, the kill power of the new rows:
  - N-09 is killed by the kept reader test.
  - N-10 needs a killer over `reopen_tx_streamer`.
  - N-11 is killed by `tx.rs:1545`.
  - N-12 is unverified.
  - N4 cannot fail in production, so drop it.

**New finding (P2):** deleting `fake.rs` 748–1423 as a block would have deleted the killers of P865–P867, P872, P873, P890, P900–P906, P909 and P912. These rows sit on retained code: the coercion latch, restore, lease, `ur_7_retries`, the probe array and `raw_group_*`.

**P3:** removing the flow code must keep `Dsp.log`, `tx_header` and `tx_header_stale`.

## S2a-ii fix round 1 (after Sol's code review)

**Verdict: FIXES OK.**

**Reader exclusion**
- `queued()` is deterministic. It counts entries in `lock.waiters`, which only `release_lock` removes (`host.rs:440,466`).
- The test covers both open paths.
- In a throwaway copy, removing either gate fails `core.rs:1579`: N-09 3/3 and N-10 3/3.
- Under CPU load it passed 40/40.
- P3, no change needed: the behavioural kill depends in theory on timing, but the final `(queued, reads)==(1,1)` makes the kill deterministic.

**Failed reopen**
- The later burst goes through `command`. The test checks the reservation is released, no clock, no send, one UR-25 row and one COMMAND_REJECTED.
- A mutant `cancel_unstarted` → `held.insert` fails `tx.rs:1548`.
- COMMAND_REJECTED is RM-25's observable (`design/07:261`).
- P3, pre-existing: the reason text is shared with `end_clock` and does not fit a failed reopen.

**host.rs:** `is_held` has no users left; `lock_queue` is test-only.
