# Dispositions: Sol's E4a test-plan review (REVISE; `e4/sol-plan-e4a.md`, in the tree as `plan/phase8/reviews/45e4a-plan-sol.md`)

All findings adopted. Amend `test-plan-e4a.md` to match before code.

1. P2 H03 case 12b unreachable: replace with Sol's pinned-drain sequence `Streaming(origin 0, progress 8); A Taken; Stop(8); Release(B, e = 4)`; keep the guard; a DST test must kill the guard's removal.
2. P2 case 10 witnesses unreachable in Full: give the behavioural cases explicit small K/D in every variant (or enough releases); each declared witness must be reached; `assert_bounded` accepting budget exhaustion with missing witnesses is not enough for these cases.
3. P2 P6: check progress after the freeing input's ops finish (at the step), not only at pre-shutdown terminal states; a witness shows B actually reserved.
4. P2 held order — **orchestrator ruling: global head-of-line.** One release order for the whole device; the pass stops at the first value a bound or a reservation condition refuses (today's held list, `provider/control.rs` release; reason: reserving another chain's later value first can put it ahead of an earlier one in the in-order device queue). P6 checks the head only: no state where the held head is reservable (conditions, K, D all allow) after a step's ops end. Plan §11 "Held order" records it.
5. P2 loss retirement: on loss, report the value but keep it resident until its terminal input (`Done`/`Abandoned`) arrives, per note 45 §4.5 「上限は二つ」; test retention and removal directly, including a `Done` delayed past the loss report.
6. P2 destructor: model owner release / destructor entry separately from cleanup close; the destructor decides retention from the native mark, not F. Include the missing-mark path (panic guard without `mark_lost`) and a loss between cleanup's eligibility check and destruction.
7. P3 B15: justify boundaries-first as the experiment's chosen semantics (production dispatches a LateCommand result without publishing, `provider/rx.rs:623-673`); drop the production claim; `CutReached` is sent after publication closed acceptance (`cell.rs:370-373`).
8. P3 mutation killers: two distinct completed future instants for the timer min/max row; a genuinely decreasing timestamp for `max(now, input.now)`; explicit Dropped and Refused rows; behaviour mutants separate from oracle self-tests.

From Sol's coverage table and §12 answers (adopt):
- add: a lifecycle request while K frees but the call is still pinned (pin must hold); decreasing/stale time inputs; capacity timer in the F-set / loss-boundary-pending window (B8 as reworded in plan §11); I17 must correlate the completed start with its epoch (`rules.rs:153-155` `call_of` drops it: carry it); late execution after detach; spawn failure of the first stream, suppression of later spawns, a prior loss.
- §12: 1 agree (time on `Done`; name each runtime's time source: forced `ModelDevice::now`, fake `FakeDevice::time_now`, measured host-clock extrapolation); 2 agree, state the reconciliation and test stale time; 3 B8 as reworded; 4 agree; 5 one bookkeeping field OK, test semantic idempotence, not equality of bookkeeping; 6 guard sets F and mark, no record event, test the mark independently of F-gated cleanup; 7 keep guard, case per finding 1; 8 each behavioural row needs a reachable DST sequence; 9 P6 per finding 3; 10 cut repeated variants/setup first, keep spawn-failure thread coverage and micro witnesses; 11 startup prefix confined to case 15 plus the extra tests above; 12 measure and shrink, record budgets and witnesses per case.
- Size: reuse `cases.rs` `Probe` machinery (`:888-960`); one parameterized small script per failure family; table tests for K states, time boundaries and input variants. No line ceiling is enforced; report the size.
# Dispositions: Sol's E4a1 code review (REVISE; `plan/phase8/reviews/45e4a1-code-sol.md`)

All adopted.
1. P2 end-now cancel frees K without retry: put `Decide`'s cancellation path (Late → Stop → cancel) under the shared re-evaluation rule; two-stream regression: B reserved in that step.
2. P2 P6 window: check P6 when the freeing step's ops end, before the next `BStep`, independent of a queued Shutdown or a non-empty inbox; a test with `inbox = [Done(A, now ≥ e_A), Shutdown]` and the retry omitted (mutation) fails it.
3. P2 B8 after F — **orchestrator ruling: the implementer's reading stands** (the pure brain does not read F; executor admission enforces I1). `45-experiment.md` §11 B8 is amended. Add a deterministic DST test of the whole sequence: B held, F set, loss boundary delayed, capacity timer reserves B, S checks F and abandons B with no device call, B retired at its terminal.
4. Replay (Sol's recommendation, adopted): both wrappers record acceptance at the call's return with the same timestamp, keeping device application time separate; no normal execution depends on `ModelDevice`'s effective-time hook. Remove the Q1 tolerances and interventions from `s1b`/`s1c` (and `finding_q1_*` if it becomes obsolete, or retarget it to pin the new single rule); rerun `s1c_whole_walks_wide` and the walk sweeps at seeds 1–10,000; every divergence must be zero or classified with a cause that is not a tolerance. Check witnesses still reached.

# E4a1 later rounds (orchestrator)
- Re-review 2 (`45e4a1-code-sol2.md`, REVISE, P2): P6's blanket queue exemption → removed; the I4 refinement (application sample ≤ evidence sample) and instant-ordered releases accepted by Sol.
- Re-check 3 (`45e4a1-code-sol3.md`, REVISE, P2): the interleaving exemption was not exact → ruling: P6a (at the `Reserve` event) and P6b (per-step obligation), no exemptions.
- Re-check 4 (`45e4a1-code-sol4.md`, REVISE, P2): P6b credited `ReserveAll` → credits only a `Reserve` of the held head; row E4-55.
- Re-check 5 (`45e4a1-code-sol5.md`): PASS.
- The orchestrator reran the walk sweeps at seeds 1–10,000 on sim02 after round 3 (no log had been kept): 17 passed, no failing seed.
- Gate (no mutation step; rows E4-01..E4-55 run at E4a2's full gate): `45e4a1-gate.json`.
