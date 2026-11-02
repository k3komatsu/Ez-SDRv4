# Sol's E2a-1 code review: the orchestrator's dispositions (2026-11-02)

Verdict: REVISE (1 P1, 4 P2, 1 P3). **All are adopted.**

| # | Sev | Finding | Disposition |
|---|---|---|---|
| 1 | P1 | Failure cleanup can lose a settings command routed between S's FIFO snapshot and its clear | **Adopted.** Remove the commands to abandon atomically (take them out of the live FIFO in one step), then process that batch. Later arrivals go to the discarder. Add a controlled concurrent regression: route a command while S is between taking the batch and processing it. It must end `Abandoned`, with its `Done`. |
| 2 | P2 | The panic guard covers only `Brain::step`, not the brain's operations | **Adopted.** `catch_unwind` covers the whole brain loop iteration, step and ops. A panic inside a cell section restores the scratch cells and then runs the guard: `F`, then `mark_lost`. Add a test that unwinds inside an op. |
| 3 | P2 | S5 accepts exit flags and forced teardown instead of real exits | **Adopted.** After the brain dies, every executor ends on its own: the closed sender, or the discarder's termination. S5 observes actual thread completion (join with a bound) **before** any `Runtime::stop` help. |
| 4 | P2 | S10 does not prove that the new-epoch Start is issued only after the quiesce ack | **Adopted.** Stop the forced sequence before the ack is processed, and assert that no new-epoch Start, Open or Close command has been routed (the brain's sends, not device calls). Retarget E2-14 so that it makes the brain issue early. |
| 5 | P2 | S1 full overwrites `declared` per seed, so case 6's W6a can go missing unreported | **Adopted.** Use the union of declarations over every panic position (or one builder per position), and add a regression in which W6a is deliberately missing and the test fails. |
| 6 | P3 | Sample-sized work runs under the cell lock | **Adopted** (note 45 §4.5 is the design under test, and E2b measures cell hold time). `publish_round` computes outside the cell from a snapshot, and inside the cell it only rechecks (epoch, cut, revision) and writes, retrying through the existing snapshot/recheck mechanism. Keep forced replay equal: the snapshot and the write keep their own event points (`RSnap`/`RPublish`). |

## Q1 (the acceptance-instant finding): recorded for E3
Sol confirms that the delayed timestamp is real in production. `provider/control.rs:882` passes a clock read taken after `apply` to `RxChain::hand`, and `provider/core.rs:245-248,316-322` qualifies consumption from it.

- **Direction:** conservative. Production can keep a value as debt although it was applied and received. That costs extra drain or flush work and, without later qualifying evidence, an `rx_drain` rejection or a skipped restore (`provider/rx.rs:321-324,425-430,453-455`).
- **The six visibility cases** are not production defects by themselves: production holds `rx_chain` across `hand`.
- **Labels:** keep "value" and "visibility" as replay interventions, and say they do not prove production equivalence.
- **Decision:** no production change in the experiment. The finding goes into the E3 report as a candidate for the migration: accept at the device's effective instant, which needs the device to report it.

## Re-review (2026-11-02): REVISE (1 P1, 2 P2); all adopted. Remote row run: E2-39 SURVIVED.
| # | Sev | Finding | Disposition |
|---|---|---|---|
| A | P1 | A panic inside the guard strands its own `G`; a panic during recovery's `run_ops` escapes the catch | **Adopted.** Guard recovery is ownership-safe: on unwind, release every gate or lock the brain holds before the guard restarts. A panic during recovery must not escape: a second catch, or the guard's minimal path (`F`, then `mark_lost`, then route terminations) made unconditional. Test unwinding at each guard operation, not only during reservation. |
| B | P2 | No row tests premature issuance | **Adopted.** Add a composite row that bypasses both barriers (`retiring` and `busy` for the new epoch's Start). S10b's pre-ack assertion must kill it. |
| C | P2 | The compute/write recheck is not exercised through the runtime | **Adopted.** Add one controlled runtime interleaving: compute before acceptance, then S records `accepted_at`, then the write. It fails if the runtime publishes without `unchanged_since`. Make sure `unchanged_since` covers the acceptance change: acceptance does not bump `revision` (`cell.rs:461-464`). Retarget E2-37 to the call site in `threads.rs`. |
| D | — | E2-39 survived on the remote run (killed in the implementer's copy) | **Adopted.** Find out why it survives there (a timing-dependent killer, or a test filter that does not reach the test), and make a deterministic killer. |

## Re-review 2 (2026-11-02): REVISE (1 P1, 1 P2). Remote rows 8/8 killed.
**Three strikes on one mechanism** (AGENTS.md §6). The panic guard's recovery produced a third major in a row:
1. it did not cover the ops;
2. it stranded `G`;
3. its minimal path releases `G` before publishing `F`, so R can close after `F` (I14).

The mechanism is a recovery that re-runs the guard through the brain's op machinery and patches lock ownership around it. **Design decision: stop patching and simplify.** The guard becomes one straight-line routine in the thread wrapper, run from the unwind handler:
- acquire `G` unless the brain already holds it;
- set `F` unless it is set;
- `mark_lost` unless it was marked;
- release `G`;
- route the dead-brain terminations.

It passes the same hook points in the same order as the DST's guard (`Gate`, `Set`, `Mark`, `Ungate` as `BOp` events), so forced replay is unchanged. It never runs through `run_ops`, so op-panic injection cannot reach it. `G` is held from before `F` is published until after it. `guard_after_panic`, `guard_minimal`, `release_brain_gate` and their special cases are deleted.

Tests:
- s5c: a panic at each brain op, including the ops before the guard starts; the guard completes.
- s5d becomes: a second injected panic cannot reach the guard. Assert that the guard is not op-driven, or delete s5d with that reason.
- A new free-run regression for Sol's I14 sequence: R waits for `G` while the guard runs; R must see `F` and make no close.

Rows: retarget E2-42/43 to the new routine (for example, release `G` before setting `F`, or skip `mark_lost`), or remove them as obsolete, with the reason.

| # | Sev | Finding | Disposition |
|---|---|---|---|
| P1 | P1 | Minimal recovery releases `G` before `F`: R closes after `F` | **Adopted** through the design decision above. |
| P2 | P2 | E2-44 makes S10b fail in its trace builder, before the pre-ack assertion | **Adopted.** S10b's trace becomes a literal event list (or one built once from the unmutated E1 world and stored in the test), so that a core mutant changes only the threaded runtime. The pre-ack assertion must be the one that kills E2-44. |
