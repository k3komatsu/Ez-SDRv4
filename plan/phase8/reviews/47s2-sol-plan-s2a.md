## PLAN NEEDS CHANGES

Read-only at `35afe16` plus the note-47 edit. **P** = `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s2a.md`; **U/M/A** = `crates/ezsdr-{radio-uhd,mock-radio,acceptance}`.

### 1. Major — ruling 1 is not incorporated or tested
**Evidence:** P:8,58,88–90,158; `rulings-s2a-plan.md:3`; U`/src/provider/tx.rs:1120–1191`.
- **Sequence:** initial TX membership → device refuses rate change → owner settles idle → corrected rate change → immediately submit next burst.
- **Wrong outcome:** S5 only tests successful changes; the retained `FromNone` plan cannot establish the newly required owner-side retry, clock availability, or configure-before-payload ordering.
- **Fix:** revise the removal map/specs/rows for deletion; add that sequence on both Providers through real booking/admission, asserting refusal-clock termination, corrected-clock registration, successful payload, returned claims, and configuration before payload. Keep T3/F8 ordering tests on their surviving open/recovery routes—not artificial `FromNone` retargets.

### 2. Major — the deletion heuristic removes retained behaviour
**Evidence:** P:172,203,277,538,669–672; M`/tests/mock_radio.rs:2247–2300`; U`/tests/fake.rs:2098–2143`.
- **Sequence:** fixed initial RX membership → injected overflow → resumed samples.
- **Wrong outcome:** Appendix A deletes `mr_21_overrun_shape`, although it contains no runtime parameter update and still checks loss counts, flags and overlapping-overflow accounting.
- Likewise, deleting `ur_12_every_advertised_rate_is_accepted` discards its initial-rate/grid/read-back checks; only its final RX switch becomes unreachable. Clipping coverage is also retained behaviour.
- **Fix:** replace name/direction-based DELETE decisions with actual subjects. Keep these tests, trimming only obsolete steps. Finalize surviving-anchor killers; “unanchored after editing” alone does not authorize deletion when incidental anchor context disappeared.

### 3. Major — S6’s initial-value retargets cannot reach the claimed states
**Evidence:** P:92; U`/tests/fake.rs:7105–7195`; U`/src/provider/core.rs:899–910`; M`/src/lib.rs:1681–1691,2194–2206`.
- **Sequence:** assign RX initially → Stop → `start_rx`.
- **Wrong outcome:** the streamer remains open, so this does not exercise the claimed first-open/rate-set comparison. Moving its failing-open fixture to initial membership instead fails startup.
- Mock’s refusal fault is consumed by `apply_update`, not `start_rx`; deleting RX updates removes the proposed refused-state prefix.
- **Fix:** specify reachable setups. Retain UHD owner-configure refusal/retry and peer isolation; retire genuinely unreachable first-open cases with evidence, retaining arm/recovery comparison coverage. Explicitly decide how Mock’s refused-start scenario is modeled rather than mechanically substituting `start_rx`.

### 4. Minor — readiness is simultaneously deferred and required green
**Evidence:** P:94,148; `plan/phase8/47-no-change-while-streaming.md:134`; U`/src/provider/rx.rs:520–540`.
- **Sequence:** Stop → restart whose configuration advances the clock 7 ms.
- **Wrong outcome:** current `begin` uses the already-booked origin; it does not propagate configuration-completion `ready`. S7/P1402 therefore require a new lifecycle mechanism while calling it a stage-3 scenario.
- **Fix:** explicitly defer this scenario/row as note 47 permits, preserving current start-lead tests; alternatively bring the mechanism forward with its own reviewed clock/replanning sequences.

### 5. Minor — two acceptance replacements do not prove their subjects
**Evidence:** P:106–107; A`/tests/v58.rs:296–327,541–563`.
- **Sequence:** initial TX clock at T0 → target T0+2 ms. That target is not before the origin; start lead does not move the initial clock.
- **Sequence:** device loss/`Provider::stop` → submit burst. Run-state rejection can precede the intended SC-23 ended-clock rejection.
- **Fix:** choose a genuinely past/pre-origin target and recompute assertions. For ended-clock coverage, use a refused TX change while the Run remains Running, or a focused Kernel fixture; “never had a clock” is not equivalent.

### 6. Minor — an RX-only configuration marker is missing from cleanup
**Evidence:** P:50–67; U`/src/provider/control.rs:95–99,126–129,173–177,814`.
- **Sequence:** remove `reach` → configure a segment.
- **Wrong outcome:** `ColdConfig.configured` remains written without a production reader.
- **Fix:** remove that marker/accessor and obsolete tests/rows. Preserve the snapshot/projection lock and the separate, still-used `Streams` configuration record.

### 7. Minor — spec and limitation amendments are incomplete
**Evidence:** P:158; `KNOWN_LIMITATIONS.md:62–102`; `design/18-uhd-radio.md:250,324–328,431`.
- **Sequence:** follow KL-32’s workaround → schedule RX retune.
- **Wrong outcome:** validation now rejects the documented workaround. KL-7/9/34 also describe removed RX changes, drain/restore or blocking; UR-23/36 still describe removed `FromNone` behaviour, and the H4 text promises retired observations.
- **Fix:** inventory these clauses too, preserving historical evidence and limitation IDs while updating present applicability.

**Confirmed:** classless declarations close Session, explicit-update, schedule and Vocabulary-alias routes. Children use separate assemblies; device-paced children are refused. S3’s routing assertions and normal-stop/loss split correct the earlier weaknesses. J32/J33/J34 have identical mutation anchors/replacements; consolidation is feasible.

**UHD:** VERIFIED from v4.10 sources: `multi_usrp_rfnoc.cpp:1999–2018` directly sets TX rate; `duc_block_control.cpp:569–591` writes interpolation registers—neither supplies a previous-payload drain guarantee. Existing T3 gate ordering is VERIFIED in U`/src/provider/core.rs:1540–1579`. Physical safety of the proposed retry remains INFERRED pending the missing sequence/bench evidence.

**Size:** keep delete-first staging, but isolate `FromNone` removal as a small separately reviewed substep. Freeze corrected dispositions before bulk deletion; do not split merely by file.

No files changed; no builds, mutation runs or hardware checks performed. Final per-row feasibility remains unverified.
