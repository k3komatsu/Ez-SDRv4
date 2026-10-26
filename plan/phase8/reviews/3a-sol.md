# 設計メモ 41 と段階 3a：Sol 6.1 のレビュー

## 設計メモ 41 のレビュー 1（`sol-note.log`）

VERDICT: FAIL

## Independent failure checklist

- Evidence must identify the actual profile, configuration and simultaneous workload.
- Strictness must reject every looser profile, including grids and nonnumeric guarantees.
- Every required key needs a comparison rule; unknown keys must fail.
- Fake/raw evidence must not establish hardware parity.
- Records must preserve required observations, including results without capability keys.
- Measurements must observe hardware behaviour, not merely restate enforced profile limits.
- Shared rehearsal/hardware code must retain meaningful extraction checks.
- Schema identity must reject unsupported formats without preventing deliberate future evolution.

## Findings

### N1 — major — §2: selector is not the complete application condition

**Problem:** Stream membership, rates and simultaneous activity are not selector layout fields. Putting them in free-text `note` cannot support P2’s machine-checked conditions. Different operating configurations therefore produce the same Description and cannot be distinguished reliably.

**Evidence:** `plan/phase8/41-measurement-records.md:36`; RM-28 defines selector layout, while RM-30 defines membership and rates separately (`design/07-radio-model.md:284–308`). `Description::new` ignores active membership/rates (`crates/ezsdr-radio/src/channels.rs:348–364`).

**Fix:** Keep selector for physical layout, but add structured operating conditions: stream membership, rates and concurrently active streams, associated with each measurement or homogeneous measurement group. Validate them against the layout. Include BP4b/BP4c simultaneous RX/TX and reopen cases explicitly.

### N2 — major — §§2,4: selector cannot reconstruct the actual profile values

**Problem:** Description construction also requires `block_len` and `ProviderTerms`. The note never chooses their source or establishes that the comparator checks Mock’s actual values. Comparing UHD terms alone could pass while Mock remains looser.

**Evidence:** `Description::new(layout, block_len, terms)` (`crates/ezsdr-radio/src/channels.rs:348`). Current Mock lead is 2 ms (`crates/ezsdr-mock-radio/src/profile.rs:49–54`); UHD lead is 5 ms (`crates/ezsdr-radio-uhd/src/profile.rs:20–25`). Both use the same profile identity.

**Fix:** Specify the authoritative construction path and check the values actually consumed by both Providers, or first consolidate their remaining terms. Check the record’s complete `ProfileRef` against the constructed Description, not only its measurement-schema discriminator.

### N3 — major — §3: the direction table is incomplete

**Problem:** “Every description key” cannot satisfy the proposed direction table. Some listed concepts also have no standalone capability key.

**Complete inventory:**

- `radio.timing.`: `min_timed_command_lead_ns`, `startup_latency_ns`, `command_queue_depth`, `release_window_ns`, `overflow_restart_gap_ns`, `rx_start_lead_ns`, `tx_start_lead_ns`, `tx_in_flight_ns`.
- `radio.perf.`: `rx_bytes_per_s`, `tx_bytes_per_s`, `wire_bytes_per_sample`.
- For each `radio.{rx,tx}.c<n>.`: `frequency_hz`, `gain_db`, `antenna`, `stream`, `frequency_step_hz`, `gain_step_db`, `phase_behavior_on_retune`, `path_delay_samples`.
- Each `radio.{rx,tx}.s<k>.sample_rate_hz`.
- Other description capabilities: `radio.full_duplex`, `radio.hardware_time`, `radio.tx.repeat_max_samples`, `radio.tx.repeat_align_samples`, `radio.rx.block_len`.

**Evidence:** RM-4/RM-27/RM-29; `crates/ezsdr-radio/src/lib.rs:44–93`; `channels.rs:442–511`.

**Fix:** Define treatment for every key, including informational values. Compare frequency/gain **grids**, not only range endpoints; handle phase guarantees explicitly. Repeat alignment needs divisibility/admissible-length inclusion, not ordinary numeric ordering. Channel count comes from layout, not a separate key.

The eight timing/performance numeric directions stated in the note’s lower/upper categories are consistent with their intended meanings; completeness is the blocker.

### N4 — major — §§2,5: capability-only records discard required bench results

**Problem:** “Values without a capability key cannot be recorded” excludes H1/H2 behaviour, H3 TX packet capacity, H4 wake-up latency, H5’s held-sample deadline, #56 Stop behaviour, and cold-change interruption measurements. Naming them in 3c does not give their results a representable home.

**Evidence:** `plan/phase8/41-measurement-records.md:37,70–71`; required scope and exit results at `plan/phase8/00-overview.md:61–67,132`. Existing burst-gap probes produce outcome sequences, not capability values (`crates/ezsdr-radio-uhd/tests/hardware.rs:1131–1155,1192–1231`).

**Fix:** Separate comparable capability evidence from diagnostic/behavioural observations. Preserve both without inventing new runtime capabilities. Assign producers for BP4b/BP4c, NB-5 and the UR-17 bound review.

### N5 — major — §§2,4: raw evidence conflicts with exactly-once coverage

**Problem:** The same key cannot hold both its raw reference result and its Provider measurement because §4 requires exactly one occurrence. Conversely, a raw-only `measured` row satisfies coverage but is silently skipped by comparison.

**Evidence:** `plan/phase8/41-measurement-records.md:39,61–62`; P3(6b), `plan/phase8/00-overview.md:84`.

**Fix:** Give each capability exactly one comparison status: Provider-measured or unmeasured with reason. Store raw observations separately. Reject extra/unknown keys and prevent raw-only evidence from being counted as Provider-measured coverage.

### N6 — major — §5: no method separates measured boundaries from enforced limits

**Problem:** Repackaging existing probes does not establish how Provider measurements discover a less conservative limit. Admission and runtime already enforce the profile under measurement. A successful run at the configured limit cannot establish the minimum feasible lead; a rejected shorter trial cannot establish hardware failure.

**Evidence:** Existing `hw_b8_leads` submits through Session and prints outcomes (`crates/ezsdr-radio-uhd/tests/hardware.rs:168–187`); raw lead/restart probes bypass those checks (`:421–455,461–507`). P3 requires relaxing unnecessarily strict values (`plan/phase8/00-overview.md:84`).

**Fix:** Specify a bounded measurement-only candidate-setting mechanism and its observation boundary. Record candidate settings and distinguish admission rejection, Provider failure and observed hardware failure. Keep production behaviour unchanged.

### N7 — major — §§3,5: loopback delay cannot establish separate channel delays

**Problem:** `hw_b6_delay_by_rate` measures combined TX-to-RX delay. It cannot independently produce both RM-23 per-channel path delays, yet §3 requires exact equality for each.

**Evidence:** `crates/ezsdr-radio-uhd/tests/hardware.rs:1054–1074`; RM-23 (`design/07-radio-model.md:240`).

**Fix:** Record combined loopback delay as an observation. Leave individual delays unmeasured unless independently calibrated; do not attribute the whole result to TX by assumption.

### N8 — minor — §5: rehearsal checks only shape, not extraction correctness

**Problem:** A helper returning wrong keys or fabricated values can pass serialization/coverage checks. README timing rules prohibit scheduling-dependent verdicts, not deterministic checks of result extraction.

**Evidence:** `plan/phase8/41-measurement-records.md:69`; `crates/ezsdr-radio-uhd/tests/README.md:27–37`.

**Fix:** Test extraction/reduction with synthetic observations and controlled-clock cases. Preserve existing behavioural assertions when sharing helpers.

## Assessment

Keep P2/P3 and the staged approach; repair these contracts rather than add a framework. Risk labels are reasonable. Schema discrimination plus schema-freeze is a sound starting point.

Read-only review at `7043a29`; no edits, builds or hardware runs. Measurement feasibility remains unverified experimentally.

## 設計メモ 41 の再レビュー（`sol-note2.log`）

VERDICT: PASS

N1–N8 are resolved at design-note level. The revision stays within P2/P3: it adds evidence records, comparison rules and measurement support—not new runtime capabilities or automatic profile selection.

### 1. note — §§2–4: record and comparison contracts address N1–N5 and N7

**Evidence:** `conditions` now records operating configurations; both Providers’ actual terms are compared; `values` has exactly one comparison status per key; raw and non-capability results have `observations`; loopback delay is not assigned to separate TX/RX delays.

The §3 table covers every capability emitted by `channels::Description` and its envelopes (`crates/ezsdr-radio/src/channels.rs:442–511`), consistent with RM-4, RM-27 and RM-29. The stated strictness rules are sound, provided frequency/gain inclusion compares the complete grids, not merely capability-range endpoints. Antenna inclusion is ordinary set inclusion.

Using `ezsdr-acceptance` is appropriate: it already depends on both Providers (`crates/ezsdr-acceptance/Cargo.toml:18–24`). Publishing Mock’s existing `Profile::terms()` avoids copying its values. Its 2 ms command lead differs from UHD’s 5 ms, so comparing both is necessary (`crates/ezsdr-mock-radio/src/profile.rs:49–54`; `crates/ezsdr-radio-uhd/src/profile.rs:20–25`).

**Fix:** None beyond the implementation details below.

### 2. minor — §§2,4: make condition identities and references explicit in 3a

**Problem:** Validation names layout membership, rate grids and RM-30, but does not explicitly require unique condition IDs or define directional identities for `concurrent`. Duplicate IDs could make a value’s condition ambiguous; an unqualified stream number cannot distinguish RX s0 from TX s0.

**Evidence:** Note lines 23 and 52. Stream identities are directional, and their valid indices depend on the direction’s channel count (`channels.rs:410–428`; RM-30).

**Fix:** In the 3a brief, require unique condition IDs and unique `(direction, stream)` entries; valid stream indices; no channel assigned to multiple streams in one direction; and directional `concurrent` references to declared entries. Test malformed references. This needs validation, not another abstraction.

### 3. note — §5: N6 is resolved architecturally; 3b must pin the override boundary

A Provider-path lead below the current profile **can** be measured with the proposed construction mechanism. The existing `assembly(..., adjust)` hook applies before the Provider enters the Assembly (`crates/ezsdr-radio-uhd/tests/common/mod.rs:244–303`).

However, changing only the advertised command lead is insufficient. Runtime checks separately use `DEVICE_LEAD_NS` in control, RX and TX. Start leads and other timing values come from `x3xx::timing`, not `ProviderTerms` (`crates/ezsdr-radio/src/x3xx.rs:393–426`).

**Fix:** The 3b brief must specify, per measurement:

- Which admission value and runtime checks receive the candidate.
- Which internal constants remain fixed, and why.
- How default construction retains today’s values and behaviour.
- How integration tests access the measurement-only construction.
- Candidate steps, trial counts and the observable evidence required for success—including asynchronous failures.

Do not classify “admitted without an immediate error” as success. Record all candidate settings needed to interpret the result. These are bounded implementation decisions already reserved by §§5–6, not grounds for expanding this note.

### 4. note — §5: N8 is resolved

Synthetic observations and controlled-clock tests now check extraction and classification, while existing hardware assertions must remain. That addresses the earlier shape-only rehearsal without introducing scheduling-dependent FakeDevice verdicts.

**Fix:** None.

Read-only review; no edits, builds, tests or hardware runs. Experimental measurement feasibility remains unverified.

## 3a のコードレビュー（`sol-3a.log`）

VERDICT: FAIL

## Failure scenarios checked

- Equality and stricter/looser boundaries for every comparison rule, antenna sets, guarantee ranks, truth implications and alignment multiples.
- All grid representations, continuous intervals, singleton sets, fractional steps and values beyond exact `f64` integer precision.
- Missing, duplicate, unknown and numbered keys; measured/unmeasured metadata; schema, source and profile refusal.
- Duplicate condition identities, directional stream bounds, channel membership, rate grids and concurrent references.
- Actual Provider terms/block lengths, directory loading, and removal of either Provider’s comparison.

## Findings

### 1. Major — Integer precision loss can let a looser profile pass

**Where:** `crates/ezsdr-radio/src/measurement.rs:364–388`, `:599–608`

**Problem/evidence:** Integer capabilities are converted to `f64` before comparison. Set a valid Description’s `repeat_max_samples` to `9_007_199_254_740_993` and the measured maximum to `9_007_199_254_740_992`. Both become the same float, so the upper-bound check passes although the profile permits one unsupported sample.

Grid containment has the same loss: an Integer profile spanning `2^53..=2^53+1` passes against a continuous measured singleton at `2^53`, because both integer endpoints cast to that singleton.

Current X3xx constants do not reach these boundaries, but the exported comparison API and integer types permit them. The tests contain no large-integer cases.

**Fix:** Preserve integer values for integer capabilities and exact integer bounds in containment. If mixed float comparisons cannot preserve their meaning, reject them rather than round silently. Add regressions for both examples.

### 2. Major — File-loading tests do not prove either Provider’s rejection is enforced

**Where:** `crates/ezsdr-acceptance/tests/measurements.rs:41–49`, `:60–81`

**Problem/evidence:** The only directory fixture passes both Providers. The 3 ms rejection test calls `measurement::compare` directly, outside `compare_files`.

Deleting either Provider’s comparison/assertion from `compare_files` therefore leaves every new acceptance test passing: the directory test still loads one passing record, while the rejection test independently performs its own comparisons.

P687 checks the UHD terms used to construct a Description; it does not detect omission of a Provider from the file-processing gate.

**Fix:** Exercise the actual file-processing path with negative records: one rejected only by Mock, and one rejected only by UHD. Add mutation rows removing each Provider’s gate assertion.

### 3. Minor — Singleton containment incorrectly requires compatible step sizes

**Where:** `crates/ezsdr-radio/src/measurement.rs:624–636`

**Problem/evidence:** A profile grid `Step { lo: 6, hi: 6, step: 2 }` denotes `{6}`. A measured grid `Step { lo: 0, hi: 6, step: 3 }` contains it. Both endpoint checks succeed, but the step ratio `2/3` fails, so comparison rejects valid containment.

No test covers this branch. Existing grid tests also omit Integer-profile branches entirely.

**Fix:** After checking bounds, handle `first == last` by testing that single member. Add singleton and Integer cross-representation tests.

### 4. Minor — The implementation adds an unapproved minimum-evidence restriction

**Where:** `crates/ezsdr-radio/src/measurement.rs:454–455`; `design/07-radio-model.md:345`

**Problem/evidence:** Plan 41 §2–§4 permits each comparable key to be explicitly unmeasured and separately records observations. It does not require a measured capability in every record.

The added restriction rejects an otherwise complete record whose capability rows all explain why measurement was unavailable—even when it contains valid raw-device observations. This narrows the accepted format beyond the plan and can obstruct the observation-only measurements planned for 3b.

**Fix:** Remove the restriction and its normative statement, or obtain an explicit design decision before retaining it. Explicitly unmeasured values are not a successful numerical comparison.

## Other checks

- Provider construction currently uses the real Mock/UHD terms and correct block lengths; UHD selector handling matches its binding code.
- The fixture-directory count prevents a wholly vacuous loading test, subject to finding 2.
- P675–P694 target real decisions and have plausible killing assertions, but omit the cases above.
- RM-20’s twelve-schema count matches registration. RM-35 is normative, has a closing *Test:* line, and introduces no rationale/history duplication. Its extra restriction is finding 4.
- No measurement-suite, hardware-behaviour or profile-value changes escaped the planned scope.

Not run: builds, tests, mutation execution or schema regeneration, as instructed. Findings are from static inspection; the implementer’s reported gate results were not independently verified.

## 修正 1 回目の見直し（`sol-3a-fix.log`）

VERDICT: FAIL

### 1. Major — Provider gate assertions remain unprotected

**Where:** `crates/ezsdr-acceptance/tests/measurements.rs:53–57,66–82`

**Problem:** The new fixtures prove that `compare_files` produces both Providers’ violations, but not that the acceptance gate acts on both verdicts.

**Evidence:** Delete either assertion at line 55 or 56. The fixture test still passes because it calls `compare_files` directly and checks the returned vectors. The runtime-directory test passes while that directory has no records, as currently reported. Consequently, all new acceptance tests can pass with either Provider’s gate disabled.

P698/P699 remove verdict computation, not the gate assertions. They therefore cover only part of the original finding.

**Fix:** Exercise the same assertion path used by the runtime-directory test with each Provider-specific rejecting fixture, and assert rejection. Keep a passing-fixture case. Add mutation rows deleting each gate assertion.

### 2. Minor — Asymmetric bounds tests do not detect the RX direction being swapped

**Where:** `crates/ezsdr-radio/tests/measurement.rs:454–486`

**Problem:** The adopted self-review change tests invalid RX indices, but never proves that an index valid only for RX is accepted.

**Evidence:** The description has two RX channels and one TX channel. RX stream/channel `2` is invalid under either count. Changing the RX arm at `crates/ezsdr-radio/src/measurement.rs:476` to use `description.tx.len()` therefore still satisfies these assertions. Existing cross-stream tests check for a particular violation without excluding an additional bounds violation.

**Fix:** Add a complete valid record using RX stream/channel `1` and assert that comparison is clear. Pair it with the existing TX rejection, and add a mutation swapping the RX count.

### Fixes verified by inspection

- The `2^53` refusal closes both reported rounding cases. Ordinary X3xx rates, frequency bounds and `u32::MAX` repeat values remain below the cutoff.
- Singleton containment now checks its sole member before requiring a step ratio. Integer containment branches and their new positive/negative cases are consistent.
- Observation-only records with explicit unmeasured reasons are accepted.
- O1’s second selector parse is removed; RM-35 names the actual block-length constants.
- Duplicate-channel handling now avoids the misleading cross-stream violation.

No files changed. Builds, tests and mutations were not run, as instructed; reported execution results were not independently verified.

## 修正 2 回目の見直し（`sol-3a-fix2.log`）

VERDICT: PASS

No findings in the requested fixes.

- **Provider gates:** `crates/ezsdr-acceptance/tests/measurements.rs:53–98` routes real records and fixtures through `check_comparison`. Disabling either Provider’s rejection makes its provider-specific fixture assertion fail. The passing fixture remains checked. P703/P704 directly cover these rejection branches.

- **Asymmetric bounds:** `crates/ezsdr-radio/tests/measurement.rs:471–485` now requires RX stream/channel `1` to pass with two RX channels and one TX channel. Substituting the TX count produces violations and fails `assert_clear`. Existing TX rejection cases remain; P705 covers the RX-count substitution.

- **UR-21 deterministic refusal:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:2906–2950` uses manually advanced Core time and a frozen manual device clock. A’s handover progresses through bounded logical steps, not wall-clock waits. B/C booking and the refusal decision occur at unchanged times. Assertions retain C’s start, B’s removal, the exact UR-21 rejection, `TIME_ERROR { refused }`, and `COMMAND_REJECTED`. Disabling the collision guard breaks these assertions; C03 covers that guard.

The supplied mutation logs record P698–P701, P703–P705 and C03 killed, including C03 after the final test adjustment. No new path allowing a looser profile to pass was found within this fix round.

Read-only review: no files changed and no builds, tests or mutations run. Execution results were checked only through supplied logs.
