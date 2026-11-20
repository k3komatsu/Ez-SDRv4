# #126 — reviews

Test plan: Sol (v1), the one re-check by Terra and Opus (v2). Code: Sol, then a re-check of the fix by Terra and Opus.

## Sol — test plan v1

**FAIL.** The plan needs revised device-loss assertions, a UHD abort test that exercises the actual shutdown path, and receive-loop tests with observable progress. I reviewed `work/n126` at `5bc555d3` read-only. No builds, tests, or mutations were run. Owner decisions on #126 (https://github.com/k3komatsu/Ez-SDRv4/issues/126#issuecomment-7039184805) · Binding plan decisions (`plan-decisions.md:3`)

### Major

1. **The mixed-loss tests reject the binding Q1 behavior.** Accept `0,1000)`, lose `[1000,5000)` at the device, discard `[5000,6000)`, then accept at 6000. U4 expects `5000` for both the header and `RX_OVERFLOW`. Correct is header `lost=5000`, event `lost=4000` at 1000, and discard stat `1000`. MK5c also needs to pin a fault *after* discards, and MK5d’s Stop at 30,000 should report 20,000 device-lost samples instead of 28,000. Update the shared parity expectations, the helper that equates event loss to header loss, the RM-17 draft, and the stamp mutation together. [Test plan §4.6 (`test-plan.md:135`) · Parity helper (`crates/ezsdr-acceptance/tests/parity.rs:881`)

2. **UHD’s `jumped` bit can give a later report credit for an earlier gap.** Accept `0,1000)`, refuse `[3000,4000)` after an unannounced jump, receive an Overrun report, then accept the contiguous `[4000,5000)`. The proposed `jumped || own` logic can manufacture `RESTARTED` and `RX_OVERFLOW` while losing `SEQ_DISCONTINUITY`. Expect `GAP_BEFORE | SEQ_DISCONTINUITY`, `lost=3000`, and zero overflow events. Keep the two cursors, qualify each report against its own device read, and carry the resulting flags and loss across `Full`. U4s and U5 test these conditions separately. [Test plan §2.3 (`test-plan.md:68`) · U4s and U5 (`test-plan.md:156`)

3. **U3c’s booked cut and actual clock end can diverge.** Accept `0,2000)`, book Abort inside the publish of `[2000,4000)`, return `Full`, then process `Shutdown(Abort)`. Booking sees 4000, but `refresh_intent` and `command` still request 2000 after the rollback. The chain takes the smaller cut, and without a pending device report the clock ends at 2000. Q11 requires 4000. Drive the owner through shutdown and assert that the plan cut, applied stop, and clock end all agree. [U3c (`test-plan.md:132`) · Shutdown (`crates/ezsdr-radio-uhd/src/provider/rx.rs:263`) · Cut selection (`crates/ezsdr-radio-uhd/src/provider/core.rs:199`)

4. **The parity rig starts too late for s4 and assumes an impossible prefix for s5.** `Started` already advances UHD through T0 + 10 ms. A Stop submitted after construction cannot exercise a cut at sample 9,000 at 1 MS/s. In s5, `Overflow(ZERO)` fires before any sample block, so a required `0,2000)` prefix rejects a correct trace. Boot the rig before the action window, specify the poll margin and observed cut, and use row-specific preconditions. Only independent evidence of timing drift should trigger an infrastructure verdict. [Test plan §§4.5, 4.10 (`test-plan.md:177`) · Rig startup (`crates/ezsdr-acceptance/tests/parity.rs:811`) · Timing rules §3 (`plan/phase8/51-test-timing.md:92`)

5. **FM10’s receive-count assertion fails on a healthy reader.** Run the proposed 400 ms Full stretch with unmodified FakeDevice receive pacing. `FakeDevice::calls()` records no `rx_recv` entries, so the lower bound fails even if all 400,000 samples were read and discarded. Use the existing `LockSpy` receive observer or returned sample spans. Give the 150 ms stall mutation a manual-clock observation point where the fake’s behind overflow yields an assertion failure before teardown. FM10 (`test-plan.md:190`) · Receive observer (`crates/ezsdr-radio-uhd/src/provider/test_support.rs:605`)

### Minor

- **Kernel registration and policy.** K1 observes the missing counter row. K2’s `stop` assertion still passes through the fallback row, which preserves the kind. Use an artifact span covering the event to verify the default mark. In the negative case, include a second Provider without a `Block` output in the same Run to catch registration across all Providers. Test plan §3 (`test-plan.md:85`) · Collector fallback (`crates/ezsdr-kernel/src/event.rs:551`)
- **Lifecycle coverage and mutation audit.** Reuse the same UHD `Rx` owner across a segment change and exercise a refusal on `rx1`. A fresh owner or exclusively `s0` emissions can hide misplaced state or a hard-coded source. In §5, MK1 has no pending report to kill N108-98, and MK6’s backpressure timestamp does not exercise N108-96’s overflow-tail branch. N126-M04 can spin in Mock’s publish loop before reaching `assert_gap_law`. Test plan §4.4 (`test-plan.md:121`) · Mutation audit (`test-plan.md:204`)
- **Spec completeness.** Keep admission and discard semantics in `ezsdr-radio` and cite RM-38 from the Provider specs. Update MR-27’s tail formula, MR-28’s event list, and UR-19’s whole-jump overflow wording. Qualify SC-18’s `GAP_BEFORE`-alone sentence for discard-only gaps, and give each amended rule an indented `*Test:*` line. Spec drafts §7 (`test-plan.md:265`) · MR-27 and MR-28 (`design/09-mock-radio.md:180`) · UR-19 (`design/18-uhd-radio.md:253`)
## Terra — test plan v2 re-check

**FAIL.** V2 addresses much of Sol’s review, but its new UHD tail and abort-floor mechanisms have correctness gaps, and the publish-order change needs an observable regression test.

My independent failure scenarios were: a later report claims an earlier unannounced jump; a carried device loss is counted again at a cut; a floor read on one stream affects another stream, or a later refusal erases a pinned floor; a parity rig passes its action window before the test starts; overlapping Mock fault intervals are summed twice; and a consumer or policy observes a block before its device event.

**Major findings**

1. **The carried-loss tail has contradictory expected results.** In §2.3 and U7t (`test-plan.md:97`), accept `[0,1000)`, report an overrun, discard `[5000,6000)`, then cut at 9000 with no further report. The algorithm says the pending report was consumed and carried with device loss 4000, so its formula yields 4000. U7t instead expects 7000, charging `[6000,9000)` as device loss without a pending report. This conflicts with Q1 and can either fail correct code or require an unsupported overflow. Specify what evidence makes the remaining span device loss and make the algorithm and test agree.

2. **`floor_reads` can pin or erase the wrong floor.** §2.3 (`test-plan.md:99`) describes one counter on `Streams`: while rx0 publishes a block that returns `Full`, a booking reads rx1’s floor. The changed counter falsely pins rx0’s discarded block as delivered; a later rx0 abort can cut too late. Conversely, after an rx0 booking legitimately pins a floor, a subsequent `Full` with no new booking can roll it back to the older `expected`. Track the relevant stream/segment and preserve previously pinned floors; add deterministic tests for both sequences. U3b/U3c alone do not cover them.

3. **Post-publish device events change observable behavior on drop-class ports.** §4.9 (`test-plan.md:206`) audits link implementations, but the consumer and coordinator run concurrently. A consumer can receive the jump block and complete work, or the coordinator can process another event, before the overflow/alignment event is emitted; that can change policy reaction or delivered-event ordering. Existing UR-18 also states that a `LATE_COMMAND` follows the stream’s `RX_OVERFLOW` (design/18 (`design/18-uhd-radio.md:245`)); KL-8 records that ordering as well. The sequential tests cannot detect an inversion. Preserve the existing ordering where required, or explicitly resolve the contract change and test it with a publish hook and concurrent event/policy observation. I cannot establish from the plan that the new order is harmless.

**Minor findings**

- The universal loss law in D3 and §4.1 (`test-plan.md:39`) omits unannounced sequence jumps and alignment-only loss; U5 itself is a counterexample. Qualify the helper’s domain or account for these causes so correct code does not fail.
- The `PendingReport` interval vector needs a test with overlapping or nested device-loss intervals and a cut through one: summing raw interval lengths can double-count the same samples. The proposed two-fault test does not establish that the intervals are disjoint.
- Q9 explicitly binds `read + jumped`; v2 replaces `jumped` with `Carry` (changes table (`test-plan.md:10`)). The counterexample supports the correction, but the binding decision should be amended or adjudicated, rather than silently treated as superseded.

Sol’s five majors are **substantively addressed in the proposed tests**, subject to the new failures above: mixed-loss values, per-read report qualification, shutdown-path testing, earlier parity boot, and observable receive progress. His kernel, lifecycle, mutation and spec minors are also addressed in the plan, apart from the ordering and loss-law gaps identified here. The other binding answers Q1–Q8 and Q10–Q11 are reflected, with the tail contradiction above affecting Q1.

For the plan’s listed open points: confirm the floor-read callers **and fix the counter’s scope and lifetime before implementation**. The stall mutation’s clock behavior, Mock step instants, parity block shapes, and K-rig construction can be resolved with failing-first tests and the first code review; retain their explicit preconditions and do not classify an ordinary failed assertion as infrastructure failure. Reader cost is reasonably left for code review unless implementation introduces extra work beyond constructing the normal block.

No files were changed, and no builds or tests were run.
## Opus — test plan v2 re-check

# #126 test plan v2 — Opus re-check (read-only, 2026-11-20)

Reviewed `test-plan.md` (v2) against `work/n126` = `5bc555d3`, `brief.md`, `plan-decisions.md` and `sol.md`. Nothing was built or run. I wrote the scenarios below after reading v2 but before going through Sol's list. I did not open `terra-recheck.*` or `sol.log`.

## Verdict: PASS, on condition of three text-only fixes (no second re-check)

v2's design closes all five of Sol's majors and follows Q1–Q7, Q10 and Q11 (on Q9, see O1). Three test expectations in v2 would fail on correct code (majors 1–3 below). Each one is fixed by editing the plan text; none is a design question. All other points go to failing-first tests or the first code review, as routed below.

## Own failure scenarios (on v2's new mechanisms)

| # | Scenario | Result against v2 |
|---|---|---|
| S1 | UHD carry: overrun → discarded jump block → stream cut before any accepted block | v2's step 5 and U7t disagree (**major 1**) |
| S2 | UHD carry: a later report vs an earlier unannounced gap (Sol M2) | U5b on v2's rules: `GAP|SEQ`, lost 3000, no event. **Holds.** |
| S3 | Abort floor: a booking of *another* stream, or a `Start` on the same stream, lands inside a discard's publish window | `floor_reads` blocks the rollback, so the floor stays at the discarded block's end (**minor m1**) |
| S4 | Abort floor: overlap (`Repeat`) inside a stretch | the optimistic write at `rx.rs:649` runs, then the overlap return at `:655-662` skips the rollback, so the floor gets stuck (**minor m2**) |
| S5 | P8 orderly Stop in a stretch: what the cut is | v2's lower bound is reversed (**major 2**) |
| S6 | P8 abort in a stretch on the thread-driven UHD | the Q11 window is reachable, so "cut 4000" flakes (**major 3**) |
| S7 | Mock `PendingReport` intervals, with an abort before `kf` | `Σ max(0, min(kg,cut) − kf) = 0`. `rx_overflow_payload` returns `None` for lost 0 (`ezsdr-radio/src/lib.rs:659`), so there is no tail, as today. **Holds.** |
| S8 | Publish order moved after the publish (drop-class ports too) | **Holds.** The Mock already emits after the publish (`mock-radio/src/lib.rs:703-713`), so UHD now matches it. No spec or KL text orders events against blocks: KL l.353 orders the Manifest's events by path. Emission still comes before `try_end`, so events stay inside the clock. Two corners remain (m5). |
| S9 | Discard → retry-forever implementation (SC-20a's other branch) | UL10 would hang, not fail (**minor m6**) |
| S10 | Second stop: a gated stale overrun vs a carried overrun, both followed by a discarded block | treated differently (**O2**) |

## Majors

**1. U7t's expected tail (7 000) contradicts v2's own step 5 and Q1.** (§4.6 U7t; §2.3 step 5)
Sequence: `A[0,1000)`, Overflow, `F[5000,6000)`, plan cut 9 000, no accepted block.
- Step 2 consumes the report at `F` (U4a asserts `pending` is empty after `F`). That gives `carry.overflow = (1000, 4000)`, `read = 6000`.
- Step 5 then gives a tail of `4000 + (pending has a loss cause ? … : 0)` = **4 000**, stamped at 1000.
- The Mock's formula `Σ max(0, min(kg,cut) − kf)` also gives 4 000.
- U7t expects `4000 + (9000 − 6000) = 7000`. That counts unread samples after the device resumed as device loss, against Q1.

Fix: U7t expects `{overrun, 4000}` at `(domain, 1000)`, clock end 9 000. Row N126-U13 keeps U7t as its killer.

**2. P8-4 / P8-1 s4: `cut ≥ 12 000 + 6 000` is reversed.** (§4.5 P8-4; §4.10 s4)
- The Mock cuts where its Stop is booked (≈ 12 000; compare `mock_cut` in `vh_8_an_overrun_cut_before_its_restart…`, `parity.rs:1012-1014`).
- UHD's Stop "cuts at its booking" (design/18 UR-25 l.320), so it also cuts at ≈ 12 000 plus uhd-control's poll latency.
- The poll margin bounds the cut from above, not from below.

Sequence: boot, step to 12 ms, submit the Stop, run to 40 ms. The Mock's cut is 12 000, so the assertion fails on correct code.

Fix: assert `12 000 ≤ cut` per Provider, plus `stat == own cut − 4000`, which still holds because the reader cannot have read past device time at booking. An upper bound `≤ 12 000 + margin` is optional. On UHD it depends on the manual clock's real-speed drift (`parity.rs` comment at the abort test, "host.rs M5"), so it may only be a precondition with an `infra::timing` verdict, never a failure.

**3. The abort row of P8-4 asserts "both cut at 4 000", but UHD can land in the Q11 window.** (§4.5 P8-4 abort; §4.10 s4)
- P8 is thread-driven, and the manual clock runs while participants are idle, so uhd-rx keeps receiving and discarding blocks of the `[4000,∞)` stretch.
- An abort booked while one of those blocks is being published reads `first_undelivered = k+len`, which bumps `floor_reads`. The rollback is then skipped, and plan cut, applied stop and clock end all equal `k+len`. This is correct under Q11.

Sequence: abort at 20 ms while uhd-rx is publishing `[18000,20000)`. UHD cuts at 20 000, so the test fails intermittently, which is worse when repeated 20 times.

Fix: on UHD, assert `cut ∈ {4000} ∪ {first+len of a refused block}` and that plan cut == applied `rx_stop` == clock end. The exact value 4 000 stays with U3b, and the window value with U3c, both deterministic. The Mock row stays exact (4 000). Also make "no event" read "no `RX_OVERFLOW`": the stretch's one `LINK_BACKPRESSURE` at 4 000 exists in this row.

## Closure of Sol's review

| Sol | v2 | Verdict |
|---|---|---|
| M1 (Q1 device-only loss) | D3; MK5a–e, U4a–e and P8-5 recomputed; parity helper; RM-17 draft; N126-M10/M13 | **Closed.** Arithmetic checked for U4a, MK5c, MK5d (20 000 at 10 000) and MK5e(2) (143 000/141 000 at 1 000). Caveat: m3 on the loss-law helper. |
| M2 (`jumped` credit) | D8 per-read qualification plus `Carry`; U5b | **Closed** (S2). |
| M3 (U3c cut divergence) | floor from `Streams.delivered` in `refresh_intent` and `command(Abort)`; `floor_reads`; U3c driven through `Shutdown(Abort)` | **Closed** for the stated sequence. m1/m2 are holes next to it. |
| M4 (P8 boot, s5 prefix) | `Started::booted`; row preconditions; s5 has no prefix | **Closed**, except that v2 adds majors 2 and 3. |
| M5 (FM10 observation) | UL10 uses `LockSpy` spans and per-turn stats | **Closed** for observation. m6 and m7 cover its killing power. |
| Minor K | K1n with a second Provider; K3 artifact span; K2 reframed | Closed |
| Minor lifecycle/rows | U2 on one owner; MK9/U12 on rx1; N108-98 → MK5a; N108-96 → MK6c; N126-M04 | Closed, but M04 can still spin (m4) |
| Minor spec | RM-38; MR-27/28; UR-19/20; SC-18; Test lines | Closed |

## Minors (each routed)

- **m1 — `floor_reads` scope.** Route: plan text, plus a failing-first test U3d.
  - `book()` calls `first_undelivered` for every item kind (`core.rs:610`), but takes the floor only for an Rx `Kind::End` whose segment it cuts (`core.rs:611-614`).
  - Count only there, per stream key. A global counter, or one bumped by a `Start` or tx booking, skips the rollback with no abort reading the floor. A later abort then cuts at the discarded block's end, which contradicts v2's SC-20a draft and the Mock (P8 mismatch).
  - Simpler alternative with no counter: after a `Full`, under the lock, keep `delivered = k+len` iff the line holds an End item whose `delivered == Some(k+len)`. That reads the fact from its owning state (AGENTS §9 maintainability 2).
  - U3d: a `start_rx`, or another stream's booking, inside the window still rolls back to 2 000.
- **m2 — overlap inside a stretch leaves the floor stuck.** Route: plan text, plus U6 asserting `Streams.delivered`.
  - `rx.rs:649` writes `delivered` when `k+len > expected`. In v2 that is true for a block wholly behind `read` but past `expected`.
  - The overlap drop (`:655-662`) then returns without a publish, so no rollback happens.
  - U6's second `[1000,2000)` leaves the floor at 2 000 with `expected` at 1 000.
  - Fix: test the write against `read`, or do it after the trim.
- **m3 — `assert_loss_law` ("Σ RX_OVERFLOW lost + discarded == header lost") is false wherever a device gap has no `RX_OVERFLOW`.** Route: plan text.
  - It fails for U5/U5b (unannounced jump), U4c (alignment), U4d (missed start) and U4e (both events carry 4 000).
  - Scope it to `RX_OVERFLOW` jumps with no unreported or alignment gap, or state it as `≤`. Use it only where those hold (P8 s1–s6 and the MK rows).
- **m4 — N126-M04 (`rx.next = stop - 1`) still spins on a one-sample discarded block.** Route: plan text.
  - In MK4a's 29 500 run, `[28000,29500)` is discarded, then `[29499,29500)` (len 1) gives `next = 29499` forever.
  - Use `stop + 1` instead: it is killed by MK1's `first == 6000` and by `assert_gap_law`.
- **m5 — publish-order corners in UHD.** Route: first code review.
  - The `SampleBlock::new_host` `Err` arm (`rx.rs:737-748`) and a port with zero links must still emit the block's events and advance `expected`, `read` and the carry as today. Otherwise the reports are lost silently.
  - The plan should say which branch ("any other outcome") they take.
- **m6 — `ScriptedLink` should panic on a second publish of the same first sample.** Route: failing-first.
  - A retry-forever Provider (SC-20a's waiting branch) would otherwise hang UL10 and the U tests rather than fail.
  - The same guard serves both Providers.
- **m7 — N126-U11 may survive.** Route: first mutation run.
  - UL10 advances device time by a fixed 2 ms per turn. A `host.sleep(150 ms)` in the `Full` branch moves the host clock, not the fake's device time, so the fake's "behind" check (device time) never fires.
  - The loop must derive device time from the host clock, or the mutant must advance device time itself. v2's §9 names the INFERRED part; the row's run settles it.
- **m8 — `end_cut` and the second-stop text.** Route: first code review.
  - §2.3 step 5 says "a carried Overrun" counts as a pending loss report for `end_cut` (UR-18, the plan's cut kept). It must be any carried loss report, including Sequence.
  - The `expected` readers v2 lists omit `try_end`'s fallback `clock.instant(expected)` (`rx.rs:297-298`); decide its source with the others.
- **m9 — MK5e keeps the name `…_keep_the_first_sample_and_the_causes_not_a_list`, and the `PendingReport` comments say "nothing grows" (`mock-radio/src/lib.rs:126, 152`).** Route: first code review.
  - v2 adds a `Vec` of intervals. The Vec is justified: an orderly cut can fall inside an earlier fault's gap while a later fault waits (`kf₂ = kg₁ > cut`), so a fixed sum is not enough.
  - Rename the test and fix the comments.
- **m10 — §1 says `Started` advances to T0 + 5 ms.** Sol M4 said 10 ms; the code is `wait_until(uhd_t0 + ms(5))` (`parity.rs:826`), so v2 is right. Nothing to change.

## Points for the orchestrator

- **O1 — Q9.** v2 drops the binding `jumped` for `Carry` because of Sol M2. Sol's counterexample refutes Q9's premise, and `jumped` is an internal decision, so record Q9 as re-decided by the orchestrator ("`read` kept; `jumped` replaced by `Carry`, Sol M2").
- **O2 — Q8, the second stop.** In v2 a report is consumed at its own read, and `stale_overrun` is cleared by any returned block, discarded ones included (§2.3 step 1, as today). A carried Overrun would still force the second stop (U7). That gives two different meanings of "an overrun since …" in one condition (`rx.rs:283`).
  - Device fact: UR-17's one-packet clause says a returned packet ends the restart, "published or not". Under that fact, today's unchanged condition `pending.contains(Overrun) || stale_overrun` already means "since the last returned packet".
  - Re-deciding Q8 to leave the condition unchanged would be simpler. It flips U7's expectation to one stop and drops the N108-41c retarget.
  - KL-8 (`KNOWN_LIMITATIONS.md:74`) "stops the next stream twice unless a block came first" stays true either way.
  - If Q8 stays, U7 as written is consistent.

## Routing summary

- **Plan text before phase B:** majors 1–3, m1 (text), m2 (text), m3, m4.
- **Failing-first tests in phase B:** U3d (m1), U6 floor assert (m2), the ScriptedLink double-publish guard (m6).
- **First code review:** m5, m8, m9.
- **First mutation run:** m7.
- **Orchestrator:** O1, O2.
- Nothing goes back to step 0.

---
Not checked: the K rig composition (`two_link_docs` with `distinct_resource_docs`), the exact `harness.step` instants of MK1/MK4/MK5, and each existing mutation row's anchor. No build or test was run.
Risk: major 3 is a race. If it is not fixed, P8-4 will fail intermittently under `repeat 20` and look like a code defect.

## Sol — code

**FAIL** for `work/n126` at `88e45d40`. Two major findings.

### Major

1. **UHD reports device loss beyond the abort cut.** Accept `0,1000)`, receive an overrun, return `Full` for the restart block `[5000,6000)`, then abort after publish returns. The clock ends at sample `1000`, but `emit_tail` emits `RX_OVERFLOW { lost: 4000 }` at sample `1000`. Mock reports no overflow for that cut. Clip carried loss to the effective cut while preserving an earlier booked Stop’s cut. [rx.rs:401 (`crates/ezsdr-radio-uhd/src/provider/rx.rs:401`) · Mock lib.rs:1336 (`crates/ezsdr-mock-radio/src/lib.rs:1336`)

2. **The new Kernel registration path lets a Link panic escape Run startup.** Start a Run with a Provider StreamOut whose Link creates successfully and whose `DataLink::policy()` panics. `create_collector` calls it outside panic containment, so `start_spec_run` unwinds instead of recording a KC-30 stage failure. Use the policy from the validated link declaration. pipeline.rs:481 (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:481`) · design/06:375 (`design/06-kernel-coordinator.md:375`)

### Minor

- **P8’s restart comparison assumes equal packet lengths.** With a valid 1,000-sample FakeDevice packet, UHD discards 1,000 samples after the overrun and Mock discards 2,000. Both satisfy the per-Provider loss checks, then fail the cross-Provider header `lost` equality. The table and standalone test duplicate this assertion. parity.rs:1549 (`crates/ezsdr-acceptance/tests/parity.rs:1549`) · parity.rs:1658 (`crates/ezsdr-acceptance/tests/parity.rs:1658`)
- **K’s negative fixture leaves `right` unattached.** Both links originate at `left`. A regression that registers every Provider with a StreamOut whenever any `Block` exists would pass both negative cases. Give `right` a drop-class output in the mixed Run. coordinator.rs:2295 (`crates/ezsdr-kernel/tests/coordinator.rs:2295`)
- **The UHD order test can pass with reversed emission calls.** The collector drains hot events before control events, so reversing the two calls still puts overflow first in this test. Record the calls at the `EventSink` boundary. rx.rs:2348 (`crates/ezsdr-radio-uhd/src/provider/rx.rs:2348`) · event.rs:492 (`crates/ezsdr-kernel/src/event.rs:492`)
- **SC-18 gives the wrong `lost` for a mixed gap.** It specifies the discarded span, while RM-38 specifies the whole time jump, including device loss. design/02:220 (`design/02-stream-contract.md:220`) · design/07:378 (`design/07-radio-model.md:378`)
- **`N126-M04` still uses `stop - 1`.** A one-sample clipped block would leave the cursor unchanged. The binding re-check chose `stop + 1` to keep the mutant advancing. mutations.json:31331 (`plan/maintenance/tools/mutations.json:31331`)

**Verification.** Read-only review. `git diff --check` passed, and `rules.py` passed with the supplied rename map. The existing mutation log reports 3,545 of 3,546 rows passed, with only the known `phase7/G09` survivor. Rust test and mutation results came from existing records.
## Terra — fix re-check

**PASS with two minor test-coverage findings.** Sol’s two majors are closed: UHD clips carried loss to the effective cut, including an earlier Stop’s cut, and Kernel registration reads the validated declaration rather than calling the Link’s `policy()`.

- **Minor — parity abort test can miss a lost tail.** If the cut is a refused block’s end, the device loss is before that cut, but the assertions accept zero overflow events or an undercount because they impose only upper bounds. Assert the expected loss for each observed cut. parity.rs:1704 (`crates/ezsdr-acceptance/tests/parity.rs:1704`)
- **Minor — N126-U17 is a weaker substitute for moving the emission.** It inserts a duplicate before publish, so a test that checked event count but ignored order could also kill it. The actual sink-boundary log does assert the correct order and would detect a pure move; the mutant alone does not demonstrate that distinction. mutations.json:31731 (`plan/maintenance/tools/mutations.json:31731`) · rx.rs:2376 (`crates/ezsdr-radio-uhd/src/provider/rx.rs:2376`)

The other reported minors are addressed: the Kernel fixture attaches the second Provider, the order test observes sink calls, P8 checks each Provider’s own packet length, SC-18 states the whole jump, and M04 advances by one sample.

For the requested edge cases, `lost_before` handles overlap, nesting, clipping and interval edges; UHD’s `read` advances past each carried interval, so its intervals cannot overlap in normal operation. A pending loss starts at `read`: if it contributes positive loss before the cut, every carried interval is already before the cut. Thus the admitted case of a wholly clipped-away carry *and a positive pending loss* is unreachable under the current invariant; removing the zero-loss filter has no observable effect on that path. The earlier-Stop and partial-cut case is covered by the new UHD test.

Read-only review; `git diff --check` passed. I did not run builds, tests or mutations.
## Opus — fix re-check

# #126 fix round: Opus re-check (read-only)

**PASS.** Both of Sol's majors are closed and the fix introduces no regression. Two minor findings and two gate notes follow.

## Own failure scenarios (written before the checklist), with outcomes

1. **Overlapping or nested carried intervals, and a cut through one.** UHD pushes `(read, k)` (rx.rs:736) only while `own > 0`, then sets `read = k + len` (full) or takes the carry (accepted). Intervals are therefore disjoint, increasing, non-empty, and each one ends at or before `read`. `lost_before` still merges overlaps (crates/ezsdr-radio/src/rx_link.rs), and `rm_*` in tests/rx_link.rs:68-78 pins overlap, nesting, repeats, a cut through a gap and both edges. OK.
2. **Pending report plus carried intervals.** The pending interval begins at `read`, which is at or after every carried end, so the merge never double-counts. Cut between the carry and `read`: the carry is clipped and the pending part is `max(0)` = 0, so it emits nothing. Cut past `read`: carry + `(cut - read)`, with `first` the carry's first. OK.
3. **Stop booked before the abort.** `end_cut` keeps the plan's cut while `carry.loss` is set (rx.rs:357). The new test's `(Some(3000), 2000)` case (rx.rs:2267) gives a tail of 2000 at 1000 and a clock end of 3000. The Mock reaches the same result: the orderly Stop's `rx_finish` runs `rx_tail` with `lost_before(gaps, 3000)` (mock lib.rs:1335-1345) and ends the stream before the abort. OK.
4. **Cut exactly at an interval edge.** The test's `None` case is the edge: the gap is `[1000, 5000)` and the cut is 1000, so the result is 0 and there is no tail. Cut equal to the gap's end counts the whole gap (rx_link.rs test :75). OK.
5. **Parity abort test, `cut = refused end` branch.** In that branch the old code and the fixed code emit the same thing (the device loss `[0, G)` lies wholly before the cut). The test cannot be passed by wrong code there, except by *under*-reporting (see Minor 1). The `cut == 0` branch fails on the pre-fix code (`0 + G <= 0`). The deterministic kill of M1 is the unit test, through N126-U14 (killed in mut-3).
6. **N126-U17 substitute.** The tap log is asserted as an exact sequence (`publish:0, publish:3000, RX_OVERFLOW, ALIGNMENT_ERROR`), so a pure move of the emission before the publish fails too, not only the inserted duplicate. N126-U16 (the swap) is killed. TapSink forwards all three `EventSink` methods (resolve/emit/emit_control, event.rs:298-305), and an unresolved handle logs `?`, which fails the test loudly instead of passing it. Adequate.
7. **The admitted gap, `.filter(|report| report.lost > 0)` (rx.rs:407).** This is an equivalent mutant, not a defect risk.
   - If the pending loss is positive, `cut > read`, which is at or after every carried end. The clipped carry is then the full sum, and that sum is positive.
   - If no positive pending loss exists, a carry clipped to 0 gives `report.payload()` = `None`, because `rx_overflow_payload` filters `lost > 0` (crates/ezsdr-radio/src/lib.rs:660).

   The untested case, "a carry wholly past the cut plus a positive pending loss", cannot be reached under the `read` invariant above. Either delete the filter (less code) or record the row as equivalent with this reason.
8. **Kernel registration (M2).** pipeline.rs:482-489 reads `decl.policy` from `shared.links`. That list is filled in the same `create_links` loop as `attached_links` (pipeline.rs:455-458), so the set of Providers is unchanged: from-side owner equals the StreamOut side. An Island-owned `Block` link adds an `Inst::Island` that the Provider loop never matches. `routing` is a `OnceLock`, so taking the `links` lock with it held cannot deadlock. The panic test (`kc_08_a_link_whose_policy_panics_is_not_called_by_the_registration`) asserts that `policy()` is never called and that no KC-30 failure is recorded. N126-K03 is killed.

## Sol's findings

- **M1 closed.** `Carry.gaps` + `lost_before(…, cut)` in `emit_tail` (rx.rs:403-418). The unit test covers both the abort-only cut and the earlier-Stop cut. The Mock counterpart is `mr_19_an_abort_after_a_fault_and_a_discard_counts_the_device_loss_to_its_cut`.
- **M2 closed** (above).
- **Minors closed:** K fixture (`right → rec2`, with an assert that the link exists), order test at the `EventSink` boundary, P8's cross-Provider equality removed (each Provider is checked against its own refused span), SC-18 wording ("its whole time jump"), N126-M04 changed to `stop + 1` (killed).

## Findings

**Minor 1 — the parity abort test checks only upper bounds** (crates/ezsdr-acceptance/tests/parity.rs:1704-1716). When the UHD's cut lands on a refused block's end (`cut > G`), the correct output is one tail `{lost: G}` at 0. Zero events also passes. The expectation is exact on both branches: total `lost == min(G, cut)`, which means no event when `cut == 0` and one event at 0 otherwise. Asserting it closes the under-report hole without depending on which branch the race takes.

**Minor 2 — design/18-uhd-radio.md:252 splits UR-18's `*Test:*` list in the middle.** Line 251 ends with `,` and line 253 continues the same list (`ur_26_a_reached_stop_cut…`). The inserted `*Test (abort after a discard):* … .` line sits between them, so the rendered paragraph reads "…`ur_18_a_silent_end_with_no_report_makes_no_tail`, *Test (abort after a discard):* `ur_18_an_abort…`. `ur_26_…`". Put the test name into the list instead. `rules.py --base 5bc555d3 --renames renames.txt` reports 0 problems, so this is text only.

## Gate notes (not findings)

- **N126-U12 was a COMPILE ERROR in mut-3** (mut-3.log:45-46, run 03:36). Its row was fixed afterwards: `gaps: vec![(0, 1)]` was added, and mutations.json:31651 was modified at 03:46. The fixed row has not been re-run. `LossReport`, `RxOverflowCause` and `BlockFlags` are imported in rx.rs:15-19, so it should compile now. Run it at the gate.
- **test_support.rs was modified at 03:46, after mut-3.** I ran `ur_20_the_overflow_follows…`, `ur_18_an_abort_after_a_full_publish…`, `ur_18_a_tail_counts…` and `ur_17_a_segment_change…` (4 passed), and the Kernel `kc_08` set (8 passed). Both runs used `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0` and `--locked`. N126-U16 and U17, which depend on the tap, should be re-run with the full lists. Anchors for N108-21/22/24, U13, U14 and U15 each match once in rx.rs. `git diff --check HEAD` is clean.

Not checked: the whole test suites, clippy, or fmt; I did not run any mutation rows. I did not verify which branch the parity abort test takes on the UHD in practice.
