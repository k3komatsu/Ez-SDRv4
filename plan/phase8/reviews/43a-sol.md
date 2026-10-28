# 段階 43a：Sol のレビュー

## 設計メモ 43 のレビュー 1

> plan · gpt-6.1-sol
VERDICT: ADOPT WITH CHANGES

Prefer (d) over finishing D9a2 or rejecting updates: retaining scheduling constraints in the shared timeline removes the specific refusal-induced Deferred-loss mechanism without adding another delivery/retry mechanism. The draft needs a narrower proof and stronger tests before implementation.

References to **draft** below mean `/home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/note43-draft.md`. Production references are from HEAD.

## Major findings

### 1. Refusal is not the only operation that advances an origin

**Draft:** lines 29–30, 81–84.

A later `Stop` can advance a previously booked move’s destination without any refusal. For example, with millisecond-aligned samples, lead 50 ms, and zero call/allowance:

- Book a move at 100 ms from a running source into an empty destination, ready at 0.
- Source release is 100 ms; destination origin is 150 ms.
- Subsequently book a source `Stop` at its receipt, 80 ms.
- The source releases at 80 ms. The destination origin becomes 130 ms.

This follows `timeline.rs:953–955,965–971,289–302,777–801,1038–1050`.

“Cuts never move later” is also false as a general replay invariant. An inserted source start can increase the release and propagate later origins/cuts across moves; existing tests explicitly demonstrate this (`crates/ezsdr-radio/tests/timeline.rs:707–724,1037–1060`).

**Change:** Replace the global claims with separate, narrower obligations:

1. Recording a receive refusal cannot add coverage or advance surviving origins/releases.
2. A newly booked operation cannot add coverage **inside the release horizon**, under explicitly stated timing assumptions.

The Stop example does not invalidate (d) with today’s 50 ms lead and 3 ms window. It invalidates the draft’s stated proof.

### 2. The timing assumption must be enforced, not left INFERRED

**Draft:** lines 28, 84, 86.

The argument depends on start lead versus release window. Those are currently 50 ms and 3 ms (`crates/ezsdr-radio/src/x3xx.rs:398–400`), but a **1 ms start lead is already a measurement candidate** (`KNOWN_LIMITATIONS.md:138–140`).

With lead 1 ms and window 3 ms:

1. A stopped channel’s update is released at 97 ms for `e = 100 ms`, taking Deferred.
2. At 98 ms, a new start is booked.
3. Its origin is 99 ms.
4. Configure excludes the update because its instant is after the origin (`provider/control.rs:146–153`).
5. Release has already removed it from `held` (`:669–673,784–788`).

The value is lost without any refusal. Changing refusal semantics does not prevent this.

**Change:** Make the required relation an enforced profile/publication constraint, or explicitly limit the recommendation to profiles satisfying it and block incompatible timing reductions. Use strict inequality for the draft’s “no new coverage through `e`” proof, or handle equality separately. Add the short-lead counterexample as a regression/validation test.

### 3. T1–T8 do not yet pin the new persistent state

**Draft:** lines 112–121.

Three gaps matter:

- **m2 does not necessarily fail T1.** Returning the refused segment in the plan can leave coverage exactly equal to the pre-refusal plan, satisfying T1’s subset assertion. Assert explicitly that refused segments never appear in the plan, clocks or device work.
- **T5 needs a surviving downstream destination.** Refusing the move itself removes its destination, making an origin comparison potentially empty. Include refusal of a source change/start that an otherwise successful move depends on.
- **Pruning must preserve the ghost scheduling state.** Exercise refusal → prune while the virtual segment remains pending/running → Stop/Start/change/move. Compare pruned and unpruned plans, releases and halted state.

Existing tests already assert origins/releases that must change beyond the two MR-34 tests: `crates/ezsdr-radio/tests/timeline.rs:269–271,833–842,1084–1095`. The source-first and destination-first prune cases both need adaptation.

**Change:** Extend T1/T5 and mutation mappings accordingly; include chained moves, from-none, repeated refusals, and unchanged transmit behavior. Preserve the existing VH-8 comparisons rather than replacing expected origins with loose lower bounds.

## Minor findings

### 4. “All majors disappear” needs an explicit disposition, not one root-cause label

**Draft:** lines 18, 54, 83, 139.

Dropping D9a2 eliminates its retry-specific obsolete replay, lost provenance, clock-based retry decision, deficient provenance tests and revised queue sentence. None requires retaining `carried`, `base_freq` or `written_freq`.

However, the operation sequences do not all cease to exist:

- Configure supersession and pruning remain relevant to ordinary release.
- A release can still wait for projection locks across stop publication. HEAD checks loss, not stop, before applying (`provider/control.rs:768–788`).
- That existing path uses hand-over time and end settlement (`:834–845`); it is not equivalent to D9a2’s new retry promise prohibiting post-stop recovery writes.

**Change:** Add a six-row disposition: eliminated retry path, existing behavior/ceiling retained, or new test required. Do not claim (d) fixes every analogous lifecycle race.

Also correct §9’s mechanism accounting: missing assertions and stale normative text are not all instances of `planned-state-read-as-actual`, and AGENTS.md §6 explicitly excludes test-only/stale-rule findings from the three-strikes count.

### 5. Qualify the delay and update the affected contracts

**Draft:** lines 85, 88, 99, 127.

The “one receive call + 3 ms + 50 ms” figure should describe a bounded case, not a universal total delay. Repeated refusals and dependency chains retain multiple scheduling constraints; lattice rounding and readiness also participate (`timeline.rs:1034–1052`). State the cost per relevant transition and avoid promising an aggregate maximum.

The retained virtual segment also broadens `Omitted`: its current documentation says its end is its origin (`timeline.rs:867–877`), which is no longer generally true. Clarify RM-34’s virtual release accounting (`design/07-radio-model.md:321`) alongside RM-25.

No public type/schema change is inherently needed. Records must contain only real planned segments, with changed surviving origins—not ghost clocks. Session and Spec Run need the same rule; retain tests for both delivery schedules.

## Overall assessment

Under today’s timing terms, (d) closes the documented D9a refusal-loss case and supports removing MR-34’s **refusal-processing** prerequisite, not its timed-start/host-delay prerequisite. Rejecting updates sacrifices useful stopped-stream configuration; finishing D9a2 adds substantially more state. Neither is preferable to (d) with the changes above.

Not checked: builds, tests, mutations or hardware, as instructed. No files changed; the revised invariants remain implementation/test obligations.

## 設計メモ 43 のレビュー 2

> plan · gpt-6.1-sol
VERDICT: ADOPT WITH CHANGES

Recommend **(d)**. The revised note addresses all five previous findings. One minor correction remains; no further design blocker found.

### Remaining finding

**Minor — §8 row ① incorrectly excludes equality** (`note43-draft.md:154`).

“A covered value has `e > origin`” is false: `timeline::covering` includes `origin == e` (`crates/ezsdr-radio/src/timeline.rs:194–198`). Configure also includes updates at that instant (`crates/ezsdr-radio-uhd/src/provider/control.rs:148`).

**Concrete change:** Replace that sentence with:

> For ordinary release, coverage implies `origin ≤ e`. Configure carries updates with effective instant `≤ origin`; when `e > origin`, the released update is later than that configure boundary. When `e = origin`, configure may already have carried updates at that instant; the existing same-instant device-queue behavior remains unchanged.

Keep the following qualification about same-instant writes. This corrects the disposition’s explanation without requiring D9a2’s provenance machinery.

### Previous findings checked

1. **O1/O2 replace the invalid global claims.** The revised proof permits later bookings to advance move destinations while requiring that they add no coverage before `b + lead`. Release after each booked Action is verified at `control.rs:358–365`. O2 appropriately remains an implementation/test obligation, not an established theorem.

2. **`lead ≥ W` is sufficient; strict inequality is unnecessary.** With `b ≥ e − W`, O2 excludes new coverage strictly before `e`. At equality, configure carries the update. Newly created configurations retain released-but-unsettled updates through `held_updates` (`control.rs:475,498–505`), closing the other necessary part of that argument. RM-6 validation, all-published-description tests, and installation before stage 4 address the measured 1 ms candidate correctly.

3. **T1–T9 now cover the missing persistent-state cases:** explicit absence of refused segments, surviving downstream destinations, chained moves, pending/running prune state, repeated refusals, from-none, unchanged transmit behavior, and both Session/Spec delivery schedules. Existing exact-origin expectations and VH-8 comparisons are preserved rather than weakened.

4. **The six-row D9a2 disposition distinguishes eliminated retry paths from retained behavior.** Stop publication and ordinary late release are no longer claimed fixed. Mechanism accounting excludes test-only and stale-text findings and avoids recounting MA1.

5. **Delay is qualified per transition**, without an aggregate bound. The proposed `Omitted` documentation and RM-34 amendment correctly account for refused virtual segments without publishing ghost clocks or segment records.

Not checked: builds, tests, mutations or hardware; no files changed. O1/O2 still require the planned implementation and regression evidence.

## 試験計画のレビュー 1

> plan · gpt-6.1-sol
## Independent failure scenarios

Before reading the test plan, I identified these cases:

- Refuse a pending or running RX change/start. Preserve later origins and move releases, but exclude the refused segment from coverage, clocks, records and device work.
- Repeat refusals; refuse a from-none enable or either side of chained moves. Preserve scheduling through full and one-sided pruning.
- Stop, abort, lose or restart a stream while its refused segment is virtual. Do not interpret a real segment’s delivered-sample index on that virtual segment.
- Exercise both refusal entry paths, Session and Spec Run, and Mock/UHD consumers.
- For TX, preserve the registered clock’s end separately from the cut used to schedule successors.
- Enforce RX lead ≥ release window at timing overrides too; test the equality boundary and actual deferred-value delivery.

## Comparison with the plan

The shared-timeline approach is appropriate. The main hand calculations are correct:

- RX successor at **2050 ms**, not 2000 ms.
- Closely following RX change at **1100 ms**, not 1055 ms.
- `pruned_move_order`: release and destination comparisons become **equality**, in both pruning orders.
- Refused-source-change case: release stays **1050 ms**, destination stays **1100 ms**.
- The call-zero restart cases retain their existing values.

The claimed TX advance is also real: `State::replay` ends the refused clock at its origin before processing a later-ready item; `cut` consequently replaces `previous` with that origin (`crates/ezsdr-radio/src/timeline.rs:938–941,1001–1021`). The A4 example gives **1200 ms versus 1250 ms**. Aligning successor scheduling with the no-refusal schedule follows the owner’s instruction; preserving the refused clock’s existing record avoids an unnecessary change.

However, the following plan changes are required.

### Majors

**M1 — The measurement exception defeats adopted rule 2.**  
Plan T8/D1 proposes leaving `with_measurement_timing` unchecked. That method changes the actual description **and publishes its capabilities and envelope**, not merely private measurement metadata (`crates/ezsdr-radio-uhd/src/provider/mod.rs:367–373`).

A measurement instance can therefore reproduce F7: defer V for e=100 ms at 97 ms, book a start at 98 ms with lead=1 ms, configure at origin=99 ms, and omit V because e exceeds origin. The current RX-lead sweep submits rate changes and captures, not that frequency sequence (`tests/common/measurement.rs:2548–2575`), so this does **not** establish corruption of today’s sweep. It does establish an unsafe, unrestricted Provider path.

**Change:** enforce the same check on measurement overrides and test rejection there. Restrict Provider-path candidates to lead ≥ W. A special exception for hardware-only measurement needs an explicit owner decision; it is not part of the accepted note.

**M2 — T3’s equality case has the wrong expected device behavior.**  
At **e=o3**, S3 covers e. `reach` returns `Timed` before considering whether S3 is configured (`crates/ezsdr-radio-uhd/src/provider/control.rs:743–756`). Therefore T3 cannot require Deferred, zero timed calls and zero pending DSP entries for both e=o3−1 and e=o3.

**Change:** keep **e=o3−1 tick** as the deferred-loss regression. Give e=o3 its own expected timed issuance and observe its consumption separately. For rule 2’s equality proof, add the distinct sequence: defer V → book start at b=e−W with lead=W → configure origin=e → verify configure carries V.

**M3 — T6 moves the test instant out of the gap it intends to test.**  
After the fix, `refusal_origins()` returns `early == o3`. The existing tests derive e as `early+5 ms` and `early+10 ms` (`crates/ezsdr-mock-radio/tests/mock_layout.rs:3614–3617,3636–3638`). Merely changing the origin assertion and reversing “timed” to “untimed” puts those updates **inside S3**, where timed behavior remains correct.

**Change:** choose fixed receipt/effective instants inside the old advance interval, independently of the fixed implementation’s `early`. Assert their relationship to C2’s cut and unchanged o3. Check the exact untimed basis, seeded phase draw and record—not just inequality with the timed constant. Keep equivalent Session and genuine pre-admitted Spec Run cases.

**M4 — T1 does not specify the multi-line test needed to establish O2.**  
T1(ii) describes adding items to a `Line`. The difficult case explicitly identified in D2 is a booking on one stream changing a move release and thereby advancing another stream, possibly through a chain. Testing the source `Line` alone cannot detect new destination coverage.

Similarly, T1(i) specifies generated origin/cut comparisons but no generated refusal comparison of move releases.

**Change:** run those comparisons through `Lines`, inspect every affected stream, and compare move releases before/after refusal. Include the later-Stop case that reduces a source release and an A→B→C propagation case. Inspect coverage boundaries from **both** schedules throughout the claimed interval before b+lead, including boundaries before b. Retain the stop-and-report instruction if O2 fails.

**M5 — O-eq and the delivered-index injection conflict.**  
O-eq compares identical items with only `refused` changed, while T1 injects delivered indices and expects O-eq to catch reading them on virtual segments. But the proposed correct implementation deliberately ignores an index on a virtual segment while the no-refusal implementation applies it to a real segment. Those schedules need not have equal successor origins.

For example, a delivered-floored Stop after C can cut real C later than the virtual cut; a subsequent restart then legitimately differs between the two artificial schedules.

Also, D4’s source inventory is incomplete: UHD populates `item.delivered` in `Streams::book` (`crates/ezsdr-radio-uhd/src/provider/core.rs:504–509`).

**Change:** use identity-consistent inputs for O-eq. Test stale delivered indices separately: on virtual segments, adding/removing the stale index must not alter the virtual cut or release; on real segments, preserve RM-16’s delivered floor and abort behavior. Specify concrete Stop/End cases and correct D4’s inventory.

**M6 — TX integration coverage is insufficient for the proposed scheduling/record split.**  
T-X1–X3 establish the timeline result, but T-X4 mainly establishes that a burst eventually plays. It does not establish that the clock registry, plan and emitted burst agree on the delayed successor origin.

Both consumers register TX clocks by plan position and retain existing registrations (`crates/ezsdr-mock-radio/src/lib.rs:1427–1444`; `crates/ezsdr-radio-uhd/src/provider/core.rs:706–717`). This makes refusal timing and already-booked successors important.

**Change:** add Mock and UHD cases for successor booked before versus after refusal, then assert exact plan origin, registered clock origin, burst target and preserved refused-clock end. Add repeated refusals and prune/Stop/End between refusal and successor. Do not adjust observation times from execution results as T-X4 permits; derive them from the required schedule.

### Minors

**m1 — Mutation coverage claims do not match the scenarios.**  
The plan says m4 is killed by T6 and T3/T4, but those described cases have no move dependency; removing omitted-segment release accounting need not affect them. Testing a validation helper alone also does not prove constructors call it.

**Change:** name actual Mock/UHD move cases that kill m4; distinguish removing validation’s comparison from removing its invocation. Replace m5’s architectural description with an executable mutation once code exists.

**m2 — Resolve D5/D6 before implementation.**  
“Run and decide” is not an expected result. Specify virtual-segment behavior after Stop/End and subsequent moves. Complete the nonzero-call arithmetic: for T4’s first example with initial 1 MS/s, refused 2 MS/s, call=2000 and allowance=3 ms, the resumed origin is **1554 ms**.

D7’s configuration-of-record behavior should remain unchanged. D8’s programming-error assertion is reasonable.

## Verdict: **FAIL**

Revise M1–M6 before implementation. The design direction and principal timeline expectations are sound; the present plan contains incorrect oracles and leaves important cross-stream and TX behavior unobserved.

Not checked: execution, mutation results or hardware. No files were changed; production code was read from HEAD `afbb125`.

## 試験計画のレビュー 2

> plan · gpt-6.1-sol
VERDICT: FAIL

The main corrections are sound: T3-a/b/c now distinguish Deferred, timed issuance and the lead=W boundary; T1-b/c inspect `Lines`; T4’s **1554 ms** and T-X5’s **1550 ms** are correct. Four issues remain.

### 1. Major — T-X requires the wrong refused-clock end for a pre-booked successor

**Plan: lines 140–143.**

Case (a) books N before C’s refusal. N therefore already cuts C at **2300 ms**. The existing rule preserves that cut—not C’s origin, **2150 ms** (`crates/ezsdr-radio/tests/timeline.rs:324–328`; `src/timeline.rs:938–941`).

The plan nevertheless requires C’s `ended_at = 2150 ms` in both cases. This contradicts T-X3 and would reject the intended record-preserving implementation.

**Change:** give the cases separate expectations:

- N booked before refusal: C ends at **2300 ms**.
- N booked after refusal: C ends at **2150 ms**.
- Both: N begins at **2350 ms**, with sample 16,000 at **2354 ms**.

Also correct line 143’s HEAD characterization. The existing `mr_20_a_burst_on_a_clock_booked_after_a_refusal_plays` books N at **2290 ms**, so HEAD’s ready-plus-lead bound gives **2340 ms**, not `eb = 2300 ms` (`mock_layout.rs:2691–2694`; `timeline.rs:1043–1053`). Its required new origin remains **2350 ms**.

### 2. Major — T8 still cannot kill removal of the constructor check

**Plan: lines 121, 161.**

All ordinary `Description::new` inputs produce valid timing. Removing its validation call therefore passes the proposed description tests.

Making W=60 ms in a **separate mutation row** does not solve this. Each row starts from the correct implementation; that row can fail while the independent “remove validation call” row survives. The proposed two-row dependency is not a killing test.

**Change:** specify a test that supplies invalid timing through the construction boundary whose invocation is being protected. State the injection mechanism and expected panic, then show that removing that boundary’s validation call makes this test fail. Keep the comparison and invocation mutations separate, but independently killable.

### 3. Minor — The measurement candidate replacement has an impossible assertion

**Plan: line 124.**

If TX remains unchanged, its default candidates are:

`[1,2,5,10,25,50,100] ms`

The proposed RX list is:

`[3,5,10,25,50,100] ms`

Thus **TX is not a superset of RX**: it lacks 3 ms. Replacing equality with `tx ⊇ rx` still fails.

**Change:** preserve TX’s independent sweep and assert each list’s intended values separately. Assert RX candidates satisfy W, including the equality candidate. Update the rehearsal list to `[3,5] ms` as planned.

### 4. Minor — T1-d asks for an unreachable restart after End

**Plan: lines 54–56.**

The abort variant books `End { abort: true }` and then proposes comparing the next Start’s origin. RX End sets `over`; subsequent Start is ignored (`timeline.rs:934–936,949–951`). There is no next origin to compare. Also, virtual segments are absent from `plan`, so plan equality alone cannot observe their different cuts.

**Change:** separate the observations:

- **Abort:** assert End prevents restart, and expose the virtual end through a concrete `Lines::released` query with a specified move.
- **Stop:** compare release and the subsequent Start’s exact origin, with and without the stale delivered index.
- Retain the real-segment delivered-floor controls.

T6’s revised fixed-gap approach addresses M3. Its baseline constants are still deferred to implementation; the HEAD arithmetic gives offsets **o2=65 ms, o3=124 ms, o3′=70 ms** from T0. Record and pin those before changing production code.

Not checked: execution, mutation results or hardware, as instructed. No files were changed; production code was read from HEAD `afbb125`.

## 試験計画のレビュー 3

> plan · gpt-6.1-sol
VERDICT: PASS

Plan v3 addresses both round-2 majors and both minors. No blocking findings remain.

1. **T-X now separates scheduling from clock records correctly** (`43a-test-plan.md:147–153`).
   - Successor booked before refusal: C ends at **2300 ms**.
   - Successor booked after refusal: C ends at **2150 ms**.
   - Both: N begins at **2350 ms**, and sample 16,000 at 4 MS/s starts at **2354 ms**.

   Mock and UHD each observe the plan, registered clock, burst target and preserved clock end. These assertions distinguish an incorrectly advanced successor from an incorrectly extended refused-clock record. The existing test’s HEAD origin is correctly revised to **2340 ms**.

2. **T8 now makes each validation invocation independently testable** (`:121–130`, `:170–173`).
   - B1 injects invalid timing directly into the private description builder.
   - B2 supplies lead 1 ms to `with_measurement_timing`.
   - B3 requires `candidate_timing` to return `Err` before Provider construction.

   Removing each boundary’s own validation invocation makes its corresponding test fail without another mutation. Comparison mutations remain separate.

   **The private timing-taking builder is an appropriate minimal seam for B1.** Ordinary constructor inputs cannot supply invalid timing. Moving the existing construction body behind one private builder permits that injection without public API expansion, mutable global state or a test-only construction path. Keep `new` a thin delegate and avoid additional builder machinery.

3. **The candidate-list assertions are corrected** (`:130`).

   RX and TX lists are asserted separately. RX includes the **3 ms equality boundary** and excludes values below W; TX retains its independent sweep. The impossible superset assertion is gone.

4. **T1-d now observes reachable behavior** (`:54–57`).

   Abort tests no longer expect a restart after End. They check restart suppression and expose the virtual cut through `Lines::released`, expecting **1500 ms** with either delivered value. Reading the stale abort index would instead produce **1400 ms**.

   The previously INFERRED move-admission premise is supported by HEAD: `book_move` checks membership through `config_at`, which reads cold configurations without excluding refused configurations or clearing membership after End (`crates/ezsdr-radio/src/timeline.rs`, `Lines::book_move` and `config_at`). With a destination not already holding the channel, this construction is admissible.

   The separate Stop case correctly expects release **1500 ms** and restart **1650 ms**; applying its stale delivered floor would delay restart to **2100 ms**. Real-segment controls remain intact.

5. **T6 pins the intended old gap** (`:108–110`).

   The stated HEAD offsets—**65 ms, 124 ms and 70 ms**—match the static calculation. Fixed updates at **75 ms and 80 ms** remain before the unchanged successor origin, rather than moving with the repaired implementation. Baseline execution and literal pinning remain explicit pre-implementation requirements.

No new blocking issue was found in v3. Implementation must still establish the exact mutation matches and killing results.

Not checked: execution, mutation results or hardware, as instructed. No files were changed; production code was read from HEAD `afbb125`.

## コードレビュー

> plan · gpt-6.1-sol
VERDICT: FAIL

## Independent scenarios checked

- Refused RX changes and starts, before and after their origins; repeated refusals; Stop, End and abort with stale delivered indices.
- Virtual segments through `cut`, `finish`, pruning and move-release calculation, without publishing clocks, records or device work.
- Source refusal before a move, initially empty destinations, and chained moves.
- TX successor scheduling separately from the refused clock’s recorded end; registered clocks and burst timing.
- Deferred receive frequencies at `e = origin − 1 tick`, equality at the origin, channel selection, and lead/window validation at all three construction boundaries.

## Findings

### 1. Major — TX moves can still begin earlier after a source refusal

**Location:** `crates/ezsdr-radio/src/timeline.rs:945–948,979–982`; release calculation at `:289–302`.

The TX correction changes `previous` on the refused clock’s own line. It does not preserve the scheduling constraint read by another line through `released()`. That function still sees the refused clock’s recorded end at its origin.

**Concrete case, all times in milliseconds:**

1. TX source holds channel 0 at 1 MS/s; destination is empty. Both leads are 50.
2. Book source C, changing to 2 MS/s, with `e=100`, `ready=0`. Its origin is 150.
3. Refuse C.
4. At receipt 100, book a move of channel 0 to the empty destination for `e=300`.

Without refusal, C ends at 300, release is 300, and the destination starts at **350**.

With refusal, C’s recorded end is **150**. The new code computes the hypothetical cut at 300 for the source’s `previous`, but `released()` returns **150**. The destination therefore starts at **300**, one lead early.

This contradicts the adopted TX alignment requirement and RM-25’s new no-earlier-start sentence. The added TX generated tests use individual lines; the new cross-line refusal tests use RX, so neither catches this case.

**Fix:** preserve TX’s hypothetical scheduling end for move dependencies separately from its unchanged clock record. Add TX `Lines` and Mock/UHD regressions with an initially empty destination, checking release, destination origin, registered clock and burst target. Keep the refused clock’s recorded end at 150.

### 2. Minor — UHD TX tests stop before the planned burst observation

**Location:** `crates/ezsdr-radio-uhd/src/provider/control.rs:6311–6370`.

These tests inspect plan origins, registered clocks and recorded ends. They discard the device and TX command receiver and never submit or transmit a burst.

Consequently, they do not establish T-X’s required agreement between scheduling and actual burst handling. A correct clock registration with an incorrectly translated or dispatched burst would pass.

**Fix:** extend the controlled-clock cases to submit the sample-16,000 burst and observe its root target/device send at **354 ms**, for successors booked before and after refusal. Include the repeated-refusal case’s intended burst observation.

### 3. Minor — The promised shortening-Stop and prune comparisons are not exercised

**Location:** `crates/ezsdr-radio/tests/timeline.rs:1883–1960`.

The fixed test says it covers a later Stop that shortens the source release, but:

- The first Stop is ordered **after the move at the same effective instant**; release remains 1050.
- The second Stop cuts a pending segment at its origin; release again remains 1050.
- The chain helper always prunes before returning. Its `[false, true]` loop does not compare an unpruned result with a pruned result.

Thus these assertions cannot catch failures specific to a genuinely reduced release or a pruning-induced change in the chain.

**Fix:** book a later Stop strictly between the refused segment’s origin and an already-booked move. Assert that release actually decreases, then compare refused/unrefused results. Capture chain plans and releases before pruning and compare them afterward.

### 4. Minor — The omitted m9 record-cut mutation is straightforward to express

**Location:** `crates/ezsdr-radio/src/timeline.rs:945–948`; `plan/maintenance/tools/mutations.json:14045` onward.

P1065–P1067 protect successor scheduling, but not the other half of m9: accidentally extending the refused clock’s record to its hypothetical cut. The report’s explanation that no suitable exact text exists is not supported by the code.

For example, mutate the refusal-handling block from:

```rust
self.refused_clock = self.running;
self.end_at_origin(stream)?;
self.refusal = None;
```

to the same block using:

```rust
self.cut(stream, item, false)?;
```

This records the triggering item’s cut instead of the refused clock’s origin while retaining the successor calculation.

**Fix:** add this mutation and kill it with the existing exact refused-clock-end assertions.

## J25 recommendation

**Keep the guard in this step; retire or replace obsolete J25 rather than changing owner production code solely to kill it.**

`crates/ezsdr-radio-uhd/src/provider/rx.rs:315–320` is not an unreachable branch: successful refusal is exercised by the existing test. What disappeared is that test’s same-origin replacement, which previously required the unchanged cursor. Its successor is now at 119 ms rather than 65 ms.

That explains this mutant’s survival, but does not itself prove unconditional cursor advancement safe across concurrent replanning. Any simplification should address that explicitly. Also update the stale comment at `rx.rs:285`, which still says refusal may advance the next segment.

## Other checks

- The RX virtual-segment implementation and Deferred/equality regressions are consistent with the adopted design on static inspection.
- Validation is called at `Description::build`, `with_measurement_timing` and `candidate_timing`. Equality passes; the Result-returning helper is appropriate.
- The additional existing-test values—109 ms, 254 ms, 55 ms restart offset, 354 ms and 119 ms—match the scheduling arithmetic.
- P1052–P1075 and J04/J25/P102/P608 each match their source exactly once. The revised J04, P102 and P608 filters observe their intended differences.
- Every test name added to the three specs resolves. The amendments retain normative wording and Changes records; KL-7/KL-15/KL-33 describe the intended change.
- UHD production changes remain confined to measurement timing validation. Profile values and the existing differential harnesses are unchanged.

**Not checked:** execution, builds, tests, mutation outcomes or hardware, as instructed. No files changed. The TX move counterexample is a static trace, not an executed reproduction.

## 見直し

> plan · gpt-6.1-sol
VERDICT: PASS

No new findings. The previous major and minors 2–4 are closed on static review.

- **TX move release:** `timeline.rs:979–983` preserves the hypothetical cut in `omitted`, separately from the refused clock’s recorded end. `released()` reads both; cross-line settling and pruning obtain their release through that function. Pruning retains the planning state and saves completed moves’ release floors.
- **Regression observations:** timeline tests assert release **300 ms**, destination origin **350 ms**, and refused-clock end **150 ms**, for refusal supplied initially or recorded afterward. Mock additionally checks burst playback at **354 ms**; UHD checks the destination’s plan and registered clock.
- **UHD burst tests:** successor and repeated-refusal cases now submit bursts and inspect their handoff to uhd-tx, checking clock identity and sample index corresponding to **354 ms** and **551 ms**. This observes command handoff, **not device transmission**.
- **Stop/prune:** the revised test genuinely reduces release from **1200 to 1100 ms**, checks the resulting destination origin, and compares chain plans before and after pruning.
- **Mutations:** P1076 protects the refused-clock record cut; P1079–P1081 protect the virtual release through timeline, Mock and UHD tests. Each source pattern matches exactly once. Dropping P1077/P1078 is consistent with the providers’ already-ended clock records.
- **J25 and adjacent behavior:** the guard remains and its stale comment is corrected. No regression found in RX virtual segments, Deferred/equality cases or the unchanged VH-8 differential harnesses.

Not checked: execution, reported test/mutation outcomes, builds or hardware, as instructed. No files changed.
