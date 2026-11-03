# REVISE

The two-bound approach is sound, but several proposed witnesses and oracles cannot establish their stated claims. No P1 findings.

References: **P** = `/home/komatsu/.cache/ezsdr-fixes/2026-11-01-n45/test-plan-e4a.md`; code paths below are relative to `crates/ezsdr-radio-uhd/src/`.

## Findings

1. **P2 — H03’s proposed separating case cannot reach its witness** (`P:60–62,127`).
   `start_confirmed` does clamp progress and cut to origin (`brain/cell.rs:415–418`), but reservation then requires **origin ≤ e < cut**. With origin = cut = 4, e = 6 fails `Cut`, not `Progress`; every other e fails either `Origin` or `Cut` (`brain/cell.rs:242–250`).
   A real separating sequence is: `Streaming(origin=0, progress=8); A=Taken; Stop(8); Release(B,e=4)`. The pin keeps drain open and acceptance remains Streaming (`brain/core.rs:557–570`). Removing the progress guard wrongly reserves B. Keep the guard, but replace case 12b with this reachable sequence and demonstrate its DST kill.

2. **P2 — Case 10 cannot reach its declared Full witnesses** (`P:123–124`).
   Full uses K=4, D=64, but releases only three values. Sequence: `release(v1,v2,v3); explore every ordering` → neither K saturation nor D saturation is possible; W10a/W10c remain unreachable.
   Give this behavioral case explicit small bounds in every variant, or provide enough releases. Require witness evidence, not merely a successful bounded-search wrapper: `assert_bounded` accepts budget exhaustion with missing witnesses (`brain/dst.rs:139–143`).

3. **P2 — P6 misses the missed-re-evaluation defect in shutdown runs** (`P:81,124–131`).
   Sequence: `K=1; A occupies K; release B; Done(A,now≥e); omit reevaluate; Shutdown` → B is cancelled instead of applied. P6 checks only terminal states **before shutdown**, so it never objects. Existing shutdown disposes held values (`brain/core.rs:491–507`).
   Assert progress after the freeing input’s operations finish, before shutdown can erase the obligation; require a witness that B actually becomes reserved. A unit-test kill alone does not establish DST detection.

4. **P2 — Global stopping on a D refusal contradicts independent per-chain progress** (`P:46,81`).
   Sequence: `R0 has D residents; K has room; held=[A→R0,B→R1]; reevaluate` → refusal of A prevents B, although R1 has both bounds available.
   This also contradicts proposed P6. Scan later entries while preserving the order of successful reservations, or explicitly obtain approval for global head-of-line blocking and amend the oracle/expectation together.

5. **P2 — Immediate retirement on loss bypasses the new D rule** (`P:38,80`).
   Sequence: `A=Taken; loss boundary arrives; Done(A) still pending` → existing `lost()` reports and removes A immediately (`brain/core.rs:732–740`). The plan calls that acceptable, but note 45 requires the terminal input **and** discharged obligations before removal (`plan/phase8/45-uhd-decision-core.md:143`).
   I11′ only checks reservations, so a lost chain with no later reservation exposes no violation. Test resident retention/removal directly, including delayed Done after the loss report; do not use the report itself as proof of correct retirement.

6. **P2 — The destructor test can mask the missing native loss mark** (`P:84,105,115`).
   Sequence: `panic; F=true; omit mark_lost; cleanup skips Close/Destroy because F` → proposed cleanup still appears safe. It never reaches the destructor decision that E4-26/E4-27 should exercise.
   Native `RxStream::drop` checks the **native mark**, not provider F (`uhd.rs:345–356`). Model owner release/destructor entry separately from cleanup close, then observe retention versus freeing. Include the missing-mark path and loss between cleanup’s eligibility check and destruction.

7. **P3 — B15’s production justification is false** (`P:68`).
   Production dispatches the recv result: Samples calls `samples`, while LateCommand takes its own branch (`provider/rx.rs:623–640,651–673`). It does not publish a buffer before handling that LateCommand result.
   Boundaries-first is the existing thread-wrapper order (`brain/threads.rs:911–921`), unlike the world (`brain/world.rs:1044–1045,1094`). Sharing that order is reasonable, but justify it as the experiment’s chosen semantics. Also, publication already closes acceptance before sending `CutReached` (`brain/cell.rs:370–373`); the boundary is not itself that transition.

8. **P3 — Several advertised mutation killers lack distinguishing inputs** (`P:94,101,149–188`).
   B1 supplies only one completed future value when claiming to kill timer `min→max`; those choices are identical. B8d repeats the same timestamp, which cannot kill `max(now,input.now)→input.now`. E4-03’s Dropped alternative is assigned to a refused-path test.
   Add two distinct completed future instants, a genuinely decreasing timestamp, and explicit Dropped/Refused table rows. Keep behavior mutants separate from oracle self-tests; do not count the latter as DST behavioral detection.

## Independent-scenario coverage

| Scenario derived before reading P | Assessment |
|---|---|
| K shared across chains; future successful/partial results; long-running calls | **Covered:** B1–B5, subject to findings 2 and 8. |
| Capacity release must not clear a pin or sample obligation | **Partial:** B2/B3 check residency; add a lifecycle request while K frees but the call remains pinned. |
| D includes left; consumption before Done; loss before Done | **Partial:** B6/B7/B9 cover normal ordering; finding 5 covers the missing loss ordering. |
| Device time versus sample progress; out-of-order time inputs | **Partial:** B2b covers progress substitution; decreasing timestamps are missing. |
| Capacity versus drain timers; stale/duplicate timers | **Partial:** B8; its post-F test starts after `Boundary::Lost`, not in the F-set/boundary-pending window. |
| Held re-evaluation and cross-chain fairness | **Partial:** named units exist; findings 3–4 weaken DST coverage. |
| H03 and unauthorized reopening | **Partial:** B10/I17 are useful; H03 witness needs replacement. Preserve the issued start’s epoch through execution—`call_of` currently discards it (`brain/rules.rs:153–155`). |
| Recv/lifecycle ordering and replay | **Partial:** B15 covers wrapper agreement, not independent correctness of both wrappers. |
| Loss record ordering | **Covered:** B11/I18 for executor publication; panic intentionally has no record event. |
| Native destruction, detach, spawn failure | **Partial:** B12–B14; separate destructor entry, test late execution after detach, and test that failure prevents subsequent spawns. |
| Oracle independence, reachability, search budget | **Partial:** monitor facts help, but findings 1–3 and 5–6 remain blind spots. |

## Answers to §12

1. **Agree:** Done may carry a root-time observation. Clarify sources: forced runtime uses `ModelDevice::now` (`brain/threads.rs:68–73`); fake runtime calls `FakeDevice::time_now` (`:86–90`); measured runtime uses host-clock extrapolation (`brain/speed/newrt.rs:172–176`). Not all are lock-free reads. Production reads the root Authority (`provider/core.rs:1232–1234`).
2. **Agree conditionally:** qualifying consumption establishes that e has passed; removal after Done needs no second K record. State this reconciliation explicitly and test delayed/stale time inputs.
3. **Agree for legitimate pre-loss capacity release; disagree as a silent rewrite of B8.** Experiment §11 currently says “nor reserves” (`plan/phase8/45-experiment.md:225`). Obtain adjudication and retain the F-before-boundary case.
4. **Agree:** Lost is not a continuing capacity reservation. That does not waive D’s terminal-input requirement or other chains’ loss handling.
5. **Agree:** one timer-bookkeeping field is reasonable. Specify stale-delivery handling and test semantic idempotence, not equality of incidental timer bookkeeping.
6. **Agree:** the panic guard must set F and native mark; an extra loss-record requirement is not stated. Its mark must be tested independently of F-gated cleanup.
7. **Agree with keeping the guard; disagree with the counterexample.** Use finding 1’s pinned-drain sequence.
8. **Disagree with relying on mutation kills plus hand-built states alone.** Each behavioral mutation needs a reachable DST sequence; I17 must correlate the actual completed start with its epoch.
9. **Agree with adding progress coverage; revise P6** as finding 3 describes.
10. **Disagree with cutting spawn-failure thread coverage or required micro witnesses first.** Cut repeated variants/setup first.
11. **Agree with a startup prefix confined to case 15.** Also test failure at the first stream, suppression of later spawns, and a prior loss; an injected notification alone does not prove startup wiring.
12. **Agree with measure-and-shrink.** Record budgets and witnesses per case; budget exhaustion is not completeness or evidence that an unreached window was tested.

## Size: what to cut or merge

- Reuse `cases.rs`’s existing `Probe` machinery (`brain/cases.rs:888–960`) for case-specific checks; avoid another harness.
- Use one parameterized small script per named failure family. Full/Reduced/Micro copies add little when only timing constants differ.
- Table-test K states, time boundaries and input variants; retain thread tests for actual startup, cleanup and replay wiring.
- Retarget existing bound rows; remove obsolete rows. Keep the new mutations small and independently killable. No defensible exact line-saving estimate exists before the diff.

Not checked: builds, execution, mutation results, hardware behavior, or any excluded holdout material. No files or git state changed.
