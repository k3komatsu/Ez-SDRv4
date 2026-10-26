# 段階 3b-1：Sol 6.1 のレビュー

## テスト計画のレビュー 1（`sol-plan-3b.log`）

VERDICT: FAIL

### Independent failure scenarios

- Candidate values reach admission but not runtime, or leak into default construction.
- A late device report, partial transmission, moved target, or absent completion counts as success.
- A lucky trial or incomplete sweep determines an optimistic boundary.
- Reports from one burst are attributed to another.
- Cold-change measurements lose their operating conditions.
- Fake rehearsal depends on test-thread scheduling or tests only serialization.
- Per-trial Runs make the suite impractical; metadata overstates what was verified.

### Findings

Paths below are repository-relative; **test plan** means `/home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/test-plan-3b.md`.

#### 1. Major — Queue success checks cannot detect late hardware execution

**Where:** Test plan:60; brief §2.2 queue-depth row.

The planned `applied` timestamp is the booked instant, not an observation of when hardware executed the tune. `record_given` writes `at=e`, including after a blocking `apply` (`crates/ezsdr-radio-uhd/src/provider/control.rs:630–688`). UHD can block on a full queue and execute late commands immediately without reporting lateness (`design/18-uhd-radio.md:22`, UR-24). Consequently, every asserted row can look correct while capacity is overestimated.

**Fix:** Define a hardware-observable deadline criterion through the Provider measurement path, with a deterministic rehearsal that delays execution past its target while leaving nominal `applied` rows intact. If that observation cannot be obtained, keep queue capacity `unmeasured`; nominal timestamps cannot establish it.

#### 2. Major — The admission-floor test does not exercise that floor

**Where:** Test plan:73, test #2.

Session `send` compiles to `DropAndFlag` (`crates/ezsdr-radio/src/lib.rs:284–289`). Its admission path validates the target and waveform, not the declared command lead (`crates/ezsdr-kernel/src/coordinator/admission.rs:208–268`). The explicit lead comparison is for scheduled `RejectAtPlan` bursts (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:1098–1126`, KC-19/SC-27).

Thus, a 4 ms Session send being admitted does not demonstrate that changing 5 ms to 3 ms changed admission; the unchanged implementation can pass.

**Fix:** Test default rejection versus candidate acceptance using the actual `RejectAtPlan` boundary. Keep a separate controlled runtime test proving that the fixed device lead remains enforced.

#### 3. Major — Fake sequence #2 is neither scheduling-independent nor correctly expected

**Where:** Test plan:73.

Preselecting an absolute target does not guarantee submission before it. The test thread can be descheduled after the Run starts; this violates `crates/ezsdr-radio-uhd/tests/README.md:27–37`.

Additionally, a burst dropped by the Provider’s 2 ms device-lead check never reaches hardware (`crates/ezsdr-radio-uhd/src/provider/control.rs:1161–1163`, UR-21). Requiring **both** Provider `TIME_ERROR` and device async `TimeError` is wrong.

**Fix:** Move the short-window runtime sequence into the controlled-clock library rig. Assert Provider failure and no device transmission. Exercise device lateness separately through an injected report.

#### 4. Major — Classification mistakes a device failure for a Provider failure

**Where:** Test plan:30, 38–43, 73–74.

Provider-event-first precedence labels async `TimeError` plus `TIME_ERROR { outcome: late_at_device }` as Provider failure. The latter event explicitly represents a device failure, merely transported through the Provider (`crates/ezsdr-radio-uhd/src/provider/tx.rs:917–933`, UR-28).

The claim that every device `TimeError` also emits `TIME_ERROR` is false: an unmatched report returns after recording the async row (`tx.rs:889–918`).

**Fix:** Classify by failure origin, inspecting `TIME_ERROR.outcome`, and preserve both evidence channels. Test matched device lateness, unmatched device lateness, Provider-side drop, underflow, admission rejection, and genuine success.

#### 5. Major — Missing completion and short transmission have no complete verdict rule

**Where:** Test plan:38–45, 57–58, tests #3–5.

Four classes are listed, but admitted/error-free trials lacking the expected result fit none. A burst record starting at the requested target also does not establish that the whole waveform was transmitted. Short sends can end a record through `BurstTracker::stop` and only add a `rejected` note (`crates/ezsdr-radio-uhd/src/provider/tx.rs:464–471, 544–567`; UR-22).

**Fix:** Specify bounded completion, expected sample count/end, rejection-note handling, and an explicit non-success outcome for missing evidence. Add synthetic tests for missing completion and a correctly started but truncated burst; neither may contribute a passing trial.

#### 6. Major — Trial attribution is asserted, not designed or tested

**Where:** Test plan:45, 57–62, 78.

“Action ID/target/key and Run interval” does not resolve delayed or timestamp-less reports. Async rows contain stream/channel and an optional tick, not Action IDs (`crates/ezsdr-radio-uhd/src/provider/tx.rs:889–895`). Moreover, adjacent Kernel bursts can share one device acknowledgement (UR-23, `design/18-uhd-radio.md:246`).

**Fix:** Define trial isolation and report ownership, including repeat setup, preemption and teardown. Add sequences where A’s delayed error arrives during B, a report lacks its tick, and an acknowledgement covers a continuation. Ambiguous evidence must not silently become success.

#### 7. Major — Lifecycle and runtime estimates do not support the proposed stage

**Where:** Test plan:12, 60, 94.

The plan specifies 3,500 trials without deciding Run reuse, reset or batching. Existing helpers request 2 s start lead (`crates/ezsdr-radio-uhd/tests/common/mod.rs:161–163`): one Run per trial already costs about **117 minutes**, before measurements.

Queue batches cannot simply all be scheduled in one Run: scheduled timed actions are dispatched together (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:462–474`), while queue occupancy counts all future bookings (`crates/ezsdr-radio-uhd/src/provider/control.rs:575–586`, UR-24).

**Fix:** Specify lifecycle and isolation per measurement, then calculate startup, settling and trial costs. **Split the stage**: first land the builder, record infrastructure and pure evidence tests; then add reviewed measurement groups. The 900–1,400-line estimate is not yet substantiated.

### Remaining disagreement resolutions

- **`Spec.schedule` instead of Session timed updates:** sound API choice; batching remains unresolved above.
- **Release window:** correctly identified as a profile value; marking it unmeasured is honest, pending the orchestrator’s scope decision.
- **In-flight window:** candidate timing matches current UR-23; update the stale P3 wording explicitly.
- **Cold conditions:** before/after observations are reasonable, but specify aggregation: choose a candidate only when **both** sweeps pass all trials, and link the value to its limiting transition and block configuration.
- **Environment metadata:** reasonable when supplied from verified probe/script/git output; add explicit missing/invalid metadata tests rather than relying only on writer tests.

No files changed; no cargo, builds or hardware tests run. Hardware timing guarantees remain unverified.

## テスト計画のレビュー 2（`sol-plan2-3b.log`）

VERDICT: FAIL

The split is sound, but two test sequences still need correction before implementation. Here, **P** means `/home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/test-plan-3b1.md`; code paths are repository-relative.

#### 1. Major — #3 does not identify the operations that consume startup/start leads

**Where:** P:23, sequence #3.

**Problem:** “Arm/start to stream start equals candidate startup latency” is not the implemented contract. `arm()` stores an earliest permitted start; `start(t0)` rejects an earlier T0 but accepts a later one without moving it to that boundary. Likewise, initial RX starts at T0, not T0 plus `rx_start_lead_ns`. A first-sample assertion alone therefore cannot establish consumption of those candidate values.

**Evidence:** `crates/ezsdr-radio-uhd/src/provider/mod.rs:538–544,602–617`. Directional leads enter the stream timelines at `provider/control.rs:247–255`; missed RX starts use the RX lead separately at `provider/rx.rs:450`.

**Fix:** Replace these descriptions with concrete controlled-clock sequences:
- Startup: arm at A; assert stored boundary A+candidate, reject a start immediately before it, and accept one at/after it.
- TX/RX leads: exercise a named cold-change or enable operation, asserting its resulting origin; separately cover RX missed-start recovery if claiming every existing reader.
- In-flight: distinguish the handoff boundary from chunk sizing. Both read the candidate (`provider/tx.rs:301–302,404`); one boundary assertion need not kill a mutation of chunk sizing.

Release-window and queue-depth observations are feasible as written, provided bookings remain future-dated and valid channels exist.

#### 2. Major — #4 chooses the correct admission path but does not create its 4 ms boundary

**Where:** P:24, sequence #4.

**Problem:** `RejectAtPlan` is correct, but the existing integration setup cannot simply supply a “4 ms lead.” Scheduled offsets are nonnegative and relative to T0. The common profile puts T0 about **2 seconds** ahead, so both default 5 ms and candidate 3 ms floors accept a small-offset burst. Reducing the environment lead to 4 ms alone also conflicts with the default **2-second startup latency**.

**Evidence:** `crates/ezsdr-radio-uhd/tests/common/mod.rs:161–163`; `crates/ezsdr-radio/src/x3xx.rs:396`; `crates/ezsdr-kernel/src/coordinator/pipeline.rs:930–966,1049–1068,1078–1115`.

**Fix:** Specify the clock/setup that makes the lead observed by `resolve_schedule` lie strictly between 3 and 5 ms, with startup independently satisfied. Hold/control that clock rather than relying on the test thread completing quickly. Assert the default failure is specifically KC-19/SC-27, and that the candidate passes that same check. If this needs a small test seam, name it now.

#### 3. Minor — #11 prohibits unsafe timing but leaves the rehearsal’s behavioral assertion unspecified

**Where:** P:31, sequence #11.

**Problem:** All-unmeasured rows plus a source-only comparison violation can pass without any FakeDevice-derived observation. “Prebooked or controlled-clock” follows the README, but does not state which operation runs, what evidence it produces, or which assertion rejects a serialization-only rehearsal.

**Evidence:** P:31; `crates/ezsdr-radio-uhd/tests/README.md:27–37`. Some raw hardware probes explicitly depend on read-time/send-time intervals or wall-clock pacing (`tests/hardware.rs:432–452,604–619`), so they cannot simply become fake timing verdicts.

**Fix:** Name one shared observation-producing operation, its safe setup, and an assertion tying the serialized observation to the actual device/Run evidence. Keep hardware timing probes out of fake verdicts unless given a controlled-clock adaptation.

#### Notes — previous findings and remaining scope

- **Previous 1:** resolved for 3b-1 by keeping queue capacity unmeasured. Its measurement method remains deferred.
- **Previous 2:** correct API selected; finding 2 above remains.
- **Previous 3:** #5 correctly uses a controlled-clock Provider drop and zero device sends. The rig need not duplicate Kernel admission; #4 proves that separately.
- **Previous 4–6:** resolved at the pure-test-plan level: failure-origin classification, incomplete outcomes, and conservative attribution are explicit. Concrete Provider sweep isolation remains a **3b-2 deferral**, not a validated implementation.
- **Previous 7:** the split resolves the immediate lifecycle/runtime objection. **500–800 lines is plausible but optimistic**, not substantiated, especially with six runtime checks and probe extraction.
- **Overflow gap:** propagation-only is correct for 3b-1. Search `overflow_restart_gap_ns` across `crates/` finds no UHD runtime reader. UHD overflow recovery marks continuity loss (`provider/rx.rs:1117–1133`); missed-start recovery uses RX start lead, not overflow gap. Do not invent runtime semantics here.
- #1–2 and #6–10 have usable observation targets; #12 remains a manual hardware check, not CI coverage.

No files changed; no cargo, builds, mutations, or hardware tests run. Hardware behavior and the line estimate remain unverified.

## コードのレビュー（`sol-3b1.log`）

VERDICT: FAIL

### Failure scenarios

- Candidate timing reaches advertised capabilities but leaves runtime readers using defaults.
- Candidate construction changes ordinary construction or the independent device lead/delivery allowance.
- Missing, short, late or ambiguously attributed evidence becomes a successful trial.
- Different candidate sets across conditions hide failed or under-trialled stages.
- Raw observations become comparable values, or comparable keys are missing/duplicated.
- Invalid metadata or an unusable output path wastes a hardware run.
- Fake rehearsal records serialize correctly but contain no actual Run evidence.

### Findings

#### 1. Major — Intersecting candidates first hides failed stages

**Where:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:464–495`

**Problem:** The summarizer intersects candidate values before calculating each condition’s successful suffix or prefix. A failed stage disappears when another condition did not test that value, producing a looser reading.

**Evidence:** With two complete trials per stage:

- Condition A: `1=pass, 2=fail, 3=pass`
- Condition B: `1=pass, 3=pass`

The common values become `[1,3]`. Minimum lead returns **1**, although A’s successful suffix starts at **3**. Capacity returns **3**, although A’s successful prefix ends at **1**. An under-trialled stage at 2 is similarly hidden.

The test at lines 648–690 covers identical candidate sets and completely disjoint sets, but not partially overlapping sets with a hidden failure. P731’s union mutation does not cover this defect.

**Fix:** Calculate eligible suffix/prefix values using every stage in each condition first, then intersect those eligible sets. Add the examples above, including an under-trialled excluded stage.

#### 2. Major — A report extending outside a trial can supply its completion

**Where:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:339–358`

**Problem:** Span attribution requires only overlap, not containment. A report overlapping exactly one trial is credited even when part of its span lies outside that trial.

**Evidence:** For trials `[0,10)` and `[20,30)`, a `BurstAck` spanning `(5,15)` matches only the first trial. If that TX trial has the expected record but no previous acknowledgement, this report supplies its `BurstAck`, and `classify_trial` returns `Success`. The acknowledgement does not lie wholly within the trial interval.

The existing spanning test uses `(5,25)`, which overlaps two trials. It therefore misses the single-overlap boundary case.

**Fix:** Require a valid span wholly contained within exactly one trial before crediting it. Otherwise mark the affected trial(s) unattributable. Test spans crossing either interval edge while overlapping only one trial.

#### 3. Major — Output failure is discovered only after every hardware probe

**Where:** `crates/ezsdr-radio-uhd/tests/hardware.rs:1468–1513`; `crates/ezsdr-radio-uhd/tests/common/measurement.rs:53–56,230–238`

**Problem:** Initial metadata validation checks only that the output path is nonempty and absolute. The entry point runs all nine probes before attempting to create the output.

**Evidence:** An existing output file—or an absolute path with a nonexistent parent—passes `process_metadata()`. All probes execute, then `write_record()` fails. Non-overwrite protection is correct, but the bench run is wasted and its collected record is not saved.

**Fix:** Preflight the output destination before opening the USRP or running probes. Keep the final exclusive-create protection against races. Add a hardware-free entry/preflight test proving an existing destination or missing parent prevents probe execution.

#### 4. Minor — P734 does not test the acceptance defect its name claims

**Where:** `plan/maintenance/tools/mutations.json:10795–10803`

**Problem:** P734 is named “metadata validation supplies a default for missing environment values,” but changes an error return into a panic.

**Evidence:** The mutant still refuses missing metadata. Its killing test fails because validation panics, not because invalid metadata was accepted. This does not demonstrate coverage of the claimed fail-open decision.

**Fix:** Substitute a plausible nonempty fallback for missing metadata, or remove the missing-value rejection. Verify the missing-environment assertions kill that mutant.

### Other checks

The builder updates Description, capabilities, admission lead and envelope; `prepare()` passes the updated Description into Core. Ordinary construction and the independent constants are unchanged. Runtime-reader tests use controlled clocks, and the fake rehearsal records actual burst/capture evidence. Raw results remain observations, with comparable values unmeasured.

No out-of-scope measurement sweeps, rig kinds, faults, retries or version bumps were found. The cited UR-34 tests exist; UR-17’s changed test checks the candidate restart lead.

**Not checked:** No builds, tests, mutations or hardware runs were executed, as instructed. No files were changed.

## 修正の見直し（`sol-3b1-fix.log`）

VERDICT: PASS

No major or minor findings. Findings 1–4 and O1/O5 are fixed as adjudicated.

- **Candidate summaries:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:482–514` computes each condition’s eligible suffix/prefix before intersection. Failed and under-trialled stages cannot disappear through another condition’s omissions. The asymmetric regressions at lines 676–694 would fail on the old code. Identical, disjoint and nested candidate sets give the required result in both modes.
- **Span attribution:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:353–381` requires containment before crediting evidence. Spans crossing either edge mark overlapping trials unattributable; tick attribution remains half-open. The regressions at lines 636–643 distinguish the fix from the old overlap-only behavior.
- **Output preflight:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:57–88` rejects existing destinations and missing parents. `crates/ezsdr-radio-uhd/tests/hardware.rs:1468` performs this validation before any probe. Tests at measurement.rs:898–908 would fail without preflight; final exclusive creation remains intact.
- **Shared derivation:** `crates/ezsdr-radio-uhd/src/provider/mod.rs:66–73` preserves the previous tree, admission-lead and envelope expressions. Both construction paths use it, without changing ordinary construction’s results.
- **Mutation checks:** All 16 requested rows match exactly once, including both P710 replacements. Their changes target real behavior. P734 now supplies a nonempty fallback; missing-metadata assertions reject that fail-open behavior rather than merely detecting a panic.
- **Wording:** `design/18-uhd-radio.md:353` contains the requested O5 correction.

No regression found in the fixes or neighboring sequences.

**Not checked:** No builds, tests, mutation runs or hardware operations were executed, as instructed. This is a static re-review; no files were changed.
