## PLAN NEEDS CHANGES

Reviewed read-only at `35afe16`. Below, **P** means `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s2.md`.

The core removal is sound: cross-stream release/dependency machinery becomes unnecessary. Retaining idle-stream `FromNone`, owner claims, generation, and T3/F8 is correct.

### 1. Major — retained-path retargets are not implementation-ready
**Evidence:** P:148–160,178,215–227; `crates/ezsdr-radio-uhd/src/provider/tx.rs:366–386,4157–4225`.

- **Sequence:** fixed TX membership → burst finishes or Stop → rate change.
- **Wrong outcome:** this does **not** reproduce the deleted enable/disable fixture. TX retains its clock and remains Active; an empty burst queue does not make the owner Idle. A mechanically substituted rate change can therefore miss `FromNone` and generation/report-queue assertions.
- Appendix A still marks retained-path tests—including claim/read exclusion and T3 barriers—as D, while the body says retain them. Appendix B supplies candidate families, not final per-row dispositions.
- **Fix:** resolve each retained-path test and row before implementation. Specify reachable prefixes—RX Stop/owner completion, TX configure-refusal/owner completion where appropriate—and assert the intended holder, phase, generation and report correlation. Replace the name-based D classifications. Ruling 3 is feasible, but “use a rate change” alone is insufficient.

### 2. Major — S3 requires unsafe, forbidden cleanup after loss
**Evidence:** P:108,197; `crates/ezsdr-radio-uhd/src/provider/mod.rs:722–738,1233–1236`; `provider/core.rs:1630–1640`.

- **Sequence:** open fixed RX/TX streams → publish device loss → finish/cleanup.
- **Wrong outcome:** S3 expects one `rx_close`/`tx_close` per opened stream. Production deliberately makes no closes after loss; satisfying this assertion would violate loss safety.
- **Fix:** separate normal-stop and loss expectations. Normal cleanup uses `close_streams`, not necessarily individual close calls. After loss assert no subsequent device operations, no cleanup close, completed sections, and returned claims through the actual owner/control exit paths. Manually calling `give_back` in S3’s unit test does not test those exits.

### 3. Minor — U5 mistakes a recovery barrier for membership machinery
**Evidence:** P:48; `crates/ezsdr-radio-uhd/src/provider/tx.rs:1786–1850`.

- **Sequence:** gate F8 recovery immediately after owner claim → publish Stop/loss → release gate.
- **Wrong outcome:** deleting the local `claim_release` removes this test’s synchronization. It is an `Arc<Barrier>`, **not** `Streams::claim_release` or `Holder::Release`.
- **Fix:** explicitly retain this barrier and the four recovery orderings; only adapt `Holder::Owner { seq: None }` to the simplified variant.

### 4. Minor — the short-lead retarget cannot produce its asserted burst
**Evidence:** P:135; `crates/ezsdr-acceptance/tests/v58.rs:296–327`.

- **Sequence:** connect with RX only → change TX stream rate → timed repeat.
- **Wrong outcome:** TX still has no channel and no SampleClock, so admission refuses the repeat rather than emitting the expected late burst.
- **Fix:** assign TX c0 initially. Choose the rate explicitly: at 2 Msps, −49 ms is **−98,000 samples**, not −49,000. Recompute target assertions or retain 1 Msps.

### 5. Minor — S1’s Python test expects a return where the API raises
**Evidence:** P:79; `python/ezsdr/session.py:112–115,619–622`.

- **Sequence:** call `sdr.set(...stream..., 1)`.
- **Wrong outcome:** `Rejected` is raised before `entry` is assigned.
- **Fix:** catch `Rejected` and inspect its entry, or use `submit` for the raw rejected entry. Close the Session before inspecting its Manifest.

### 6. Minor — UR-34 is misidentified as a membership rule
**Evidence:** P:186; `design/18-uhd-radio.md:391`.

- **Sequence:** apply the proposed “UR-34 withdrawn or reduced” amendment.
- **Wrong outcome:** removes or repurposes the hardware-bench obligation. The `ur_34_*` membership tests are listed under **UR-25**; their names do not identify their normative rule.
- **Fix:** retain UR-34’s bench contract, adapting affected measurement carriers only. The remaining amendment list is broadly appropriate; no additional Vision membership edit was found.

### 7. Minor — S2 does not observe the requested sample behavior
**Evidence:** P:94–95.

- **Sequence:** fixed non-adjacent channels → Stop s0 → restart s0.
- **Wrong outcome:** matching clock/record rows can pass despite wrong channel routing or samples; identical Provider defects also pass equality-only checks.
- **Fix:** define an unambiguous `{0,2}` layout and assert channel-distinct samples, channel order, stopped-stream silence and unaffected-peer continuity.

**Confirmed:** J33’s S4 replacement works conceptually: initially assigned TX plus a cold rate change creates a future origin; the simulation rig advances virtual time, so thread timing is irrelevant. Scheduled membership updates already fail validation (`validation.rs:359–361`); child Runs use separate assemblies, hardware children are refused, and both Providers guard repeated prepare.

No files changed; no builds, tests, mutation runs or hardware checks executed. Final row-by-row killer coverage remains unverified.
