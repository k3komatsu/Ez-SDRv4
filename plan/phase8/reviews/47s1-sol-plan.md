## Findings

References to `test-plan-s1.md` below mean `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s1.md`.

### 1. Major — Cross-direction stream-name uniqueness is not tested

- **Plan:** §0.2, S2b, S5, Q1 (`:37,91–93,118–120,239`).
- **Problem:** The plan still permits names per direction. The ruling requires uniqueness across RX and TX.
- **Minimal sequence:** `connect(rx={"main":[0]}, tx={"main":[0]})` → both accepted and returned as aliases. This violates the decided contract, yet every named test can pass.
- **Evidence:** `rulings-s1-plan.md:2`; `plan/phase8/47-no-change-while-streaming.md:87`.
- **Correction:** Add client and raw-server cases rejecting the same name across directions. Keep a successful distinct-name RX/TX case to exercise P1343.

### 2. Major — S7 specifies the wrong throughput case and failure stage, and omits TX-disabled coverage

- **Plan:** S7 (`:131–138`).
- **Problem:** Two RX channels at 100 MS/s on the normal 10 GbE profile consume **800 MB/s**, below its **1 GB/s** limit. Also, a combination rejected by `coerce` is rejected during **Validate**, not Prepare.
- **Minimal sequence:** Connect two RX channels at 100 MS/s → correct implementation accepts, but the planned assertion demands refusal. With a genuinely excessive request, correct validation fails before prepare, contradicting the planned `Stage::Prepare`.
- **Evidence:** `crates/ezsdr-radio/src/x3xx.rs:395–396`; `crates/ezsdr-kernel/src/plan/validation.rs:52–58`; `crates/ezsdr-kernel/src/plan/matching.rs:105–125`; `crates/ezsdr-kernel/src/coordinator/pipeline.rs:252–266`. Existing valid boundary cases: `crates/ezsdr-radio/tests/channels.rs:591–604`.
- **Correction:** Use 200+100 MS/s as rejected and 200+50 MS/s as accepted; assert Validate failure and no device configuration/open/start. Add the missing `tx_enabled=false` case with explicitly assigned TX, plus receive-only success with **explicit RX membership**. `connect(tx={})` alone cannot capture under the new defaults. Use an assigned **TX** channel for the RF-band case: that check does not enforce RX frequency bands (`crates/ezsdr-radio/src/lib.rs:431–475`).

### 3. Major — “No Run exists” is not established, and S12 promises the wrong boundary

- **Plan:** S5, S12 (`:119–121,171`).
- **Problem:** A successful later connect and an unchanged device-open count do not prove that coordinator connect was never entered. Conversely, unknown Kernel keys are currently refused during validation **after a Run exists**.
- **Minimal sequence:** A server passes an out-of-range number to coordinator connect → validation creates a failed Run → server finishes it without retaining a live Session → another connect succeeds. The proposed recovery assertion does not establish pre-Run refusal.
- **Evidence:** Run identity is allocated at `crates/ezsdr-kernel/src/coordinator/pipeline.rs:126–131`; validation follows at `:252–266`. Server handling of a failed Run writes its Manifest at `crates/ezsdr-server/src/lib.rs:222–226`. Coordinator ordering: `crates/ezsdr-kernel/src/coordinator/mod.rs:170–176`.
- **Correction:** For server expansion errors, observe zero coordinator-entry calls or another explicit Run-creation observation, and no failed-Run Manifest. Separate this from S12: an undeclared key should produce a Validate-stage failed Run unless the orchestrator explicitly changes that contract.

### 4. Major — S3 does not expose channel-order bugs and has incompatible observations

- **Plan:** S3 (`:99–102`).
- **Problem:** `[0,2]` is already sorted, so preserving user order instead of logical order passes. A Session capture/transmission also requires Actions; asserting an entirely empty action log conflicts with observing captured, coupled samples.
- **Minimal sequence:** Connect with `[2,0]` → implementation preserves that order → channel 2 becomes the first sample channel. The planned `[0,2]` case cannot detect this.
- **Evidence:** Logical-order implementation: `crates/ezsdr-radio/src/channels.rs:441–452`. Implicit Session outputs have empty parameters: `crates/ezsdr-kernel/src/session.rs:471–475`; capture is an Action: `crates/ezsdr-sink/src/lib.rs:54–59`.
- **Correction:** Supply `[2,0]`, drive distinguishable channel signals, and assert channel-0/channel-2 sample ordering. Check an empty log **immediately after connect**, then assert no membership updates in the later log. Kernel `session_run_with` cannot itself accept stream names; test that variant through the server.

### 5. Major — S4’s trace checks do not establish configuration before arm

- **Plan:** S4, P1344 (`:108–111,223`).
- **Problem:** Comparing RX apply with `rx_start`, or TX apply with first send, permits configuration **after streamer opening during arm**. FakeDevice has no generic “arm call” to locate.
- **Minimal sequence:** Connect with nondefault initial settings → `rx_open`/`tx_open` happens first → settings are applied before start/send. The proposed comparisons pass despite the required configure-before-arm order being broken.
- **Evidence:** Configuration occurs in prepare at `crates/ezsdr-radio-uhd/src/provider/mod.rs:507–534`; streamer opening occurs in arm at `:551–566`; RX start occurs at `:632–635`. Fake trace entries: `crates/ezsdr-radio-uhd/src/device.rs:1476,1525,1822`.
- **Correction:** Require the relevant initial apply records before the first corresponding `rx_open`/`tx_open`, and check start/send separately. P1344 needs a concrete, compilable mutation: there are no adjacent prepare/arm calls to swap inside `provider/mod.rs:443–534`.

### 6. Major — Several mutation rows cannot be killed as specified

- **Plan:** §3–4 (`:192–223`).
- **Problem and corrections:**
  - **P49/P137:** Their proposed anchor includes both `channel_setting(...)` and `Value::from(...)`, but replacement contains only the value. Literally applied, this removes an `insert` argument. Preserve the key or narrow the anchor.
  - **P49:** Its proposed killer reads a **Present coerce report**, not `defaults()`. After replacing `default_stream` independently at the two call sites, changing only `defaults()` can leave those assertions unchanged. Add a direct `defaults()` assertion.
  - **P1328/P1329:** Omitting explicit `-1` requirements leaves the same effective configuration because defaults are now `-1`. Their effective-value tests cannot distinguish the mutant. Assert expanded `Manifest.spec.body.resources[…].requires`, or drop these rows if explicit redundant requirements are not contractual.
  - **P1331:** S5 names c5/s9, not the exact `count` boundary needed to kill `>=` → `>`.
  - **P1336:** Mutates the RX condition, but its stated frame assertion checks empty **TX**.
- **Minimal sequence:** Omit an unlisted channel’s `-1` requirement → default remains `-1` → S2 effective assertions still pass → P1328 survives.
- **Evidence:** Separate default/coerce paths: `crates/ezsdr-radio/src/channels.rs:423–438,479–484,498–508`; killer observations: `crates/ezsdr-radio/tests/channels.rs:392–399`. Replacement semantics: `plan/maintenance/tools/anchor.py:277–284`.
- **Correction:** Supply literal `old/new` anchors and matching observations. New rows currently have no executable anchor definitions; exact-one certification must remain pending implementation.

### 7. Major — The sweep example’s child Spec loses RX membership

- **Plan:** S10 and §3 (`:166,190`).
- **Problem:** Changing only the parent to `connect(rx={0:[0]})` does not configure its child Run.
- **Minimal sequence:** Parent connects with RX assigned → `sdr.run(build_experiment(gain))` starts a child whose requires contain gain only → new defaults leave the child’s RX unassigned → no capture artifact → `result.capture("rec")` fails.
- **Evidence:** `python/examples/sweep.py:11–15,28–31`; the child uses `start_spec_run`: `crates/ezsdr-kernel/src/coordinator/mod.rs:148–159`.
- **Correction:** Add RX membership to the **child Spec’s requires**. Audit explicit Spec fixtures as well as Session callers; the default change affects both.

### 8. Minor — Several observation names and the coercion-call assertion need correction

- **Plan:** S1, S3, S4, S7, S8.
- **Problem:** `Manifest.sample_clocks`, `SampleClockRecord.channels`, and `AdmissionResult.coercions` are not the current fields. Also, “coerce called once” must specify **during validation**, not across the entire connect lifecycle: both real Providers call coerce again in prepare.
- **Evidence:** `crates/ezsdr-kernel/src/manifest.rs:178–192`; `crates/ezsdr-kernel/src/time/domain.rs:109–124`; `crates/ezsdr-kernel/src/plan/validation.rs:110–112`; `crates/ezsdr-mock-radio/src/lib.rs:2512–2514`; `crates/ezsdr-radio-uhd/src/provider/mod.rs:453–459`.
- **Correction:** Use `manifest.clocks.sample_clocks`, block headers/Provider records for channel sets, and `coercions_preview`. Record validation and prepare calls separately.

### 9. Minor — Stage-1 input tests leave promised rejection paths unspecified

- **Plan:** §0.2–0.3, S5, S13–S14 (`:35,56,118–120,172–173`).
- **Problem:** The design promises rejection of nonfinite values, but S5 does not exercise them. Server handling of decimal-string channel items differs from Python, and malformed inputs have an unresolved “Protocol or Refused” verdict. S1 also tests capture failure, not the ruling’s **alias-read** failure.
- **Evidence:** Stage-1 integer channels and alias ruling: `plan/phase8/47-no-change-while-streaming.md:83–88`; `rulings-s1-plan.md:3`. Current alias selection only checks whether stream objects exist: `python/ezsdr/session.py:405–417`.
- **Correction:** Name exact error categories; test nonnumeric/nonfinite settings before any request, direct alias reads, and raw string-channel refusal. Keep channel-name resolution in stage 1b; accepting strings in the schema now does not prove that resolution is implemented.

## D3 and D4

- **D3 — VERIFIED:** Initial requires use Session **warn** without adding policy entries. Connect assembles `RunKind::Session` (`coordinator/mod.rs:174`); `CompileInputs.is_session` derives from that kind (`coordinator/pipeline.rs:870–879`); validation and prepare resolve policy through it (`plan/validation.rs:110–115`, `plan/prepare.rs:43,117`). `plan/coercion.rs:5–12` explicitly selects Warn. Keep the regression test; remove the speculative policy workaround.

- **D4 — VERIFIED by source inspection:** A declared producer port need not have an active stream or SampleClock. Endpoint validation checks declared ports/contracts, not clocks (`plan/links.rs:91–126,134–139`); link creation likewise has no clock prerequisite (`coordinator/pipeline.rs:395–435`). CaptureSink prepare accepts the attached input without requiring a clock (`crates/ezsdr-sink-capture/src/lib.rs:393–419`). Empty channel sets create no running timeline segment (`crates/ezsdr-radio/src/timeline.rs:896–899`); UHD skips empty stream configuration/opening (`provider/mod.rs:510–525,555–565`). **INFERRED:** the complete revised-default connection succeeds unchanged; S1 should confirm it. No Kernel link-rule change is presently justified.

## Mutation-row table

**FIX** for new rows includes the missing literal, uniquely checked anchor. None of P1319–P1344 exists in the current mutation list; future-source uniqueness cannot be certified against this checkout.

| Row | Verdict | Required correction / assessment |
|---|---|---|
| P1319 | FIX | `initial.get(name)` currently has zero matches; intended killer is suitable. |
| P1320 | FIX | Proposed `Constraint::Eq { value }` currently has zero matches at the site; pin literal replacement. |
| P1321 | FIX | Supply exact push/removal anchor; exact Vocabulary assertion is suitable. |
| P1322 | FIX | `v.version.major` currently absent at the site; exact-major assertion is suitable. |
| P1323 | FIX | Specify which Vocabulary is inserted for empty input and the executable mutation. |
| P1324 | FIX | Supply exact named-resource mutation; two-resource assertion is suitable. |
| P1325 | FIX | Supply exact refusal-removal anchor; unknown-resource test is suitable. |
| P1326 | FIX | Future connect call absent; hash integration killer is suitable. |
| P1327 | FIX | Future server call absent; S2 killer is suitable. |
| P1328 | FIX | Effective values cannot kill it; observe Spec requires. |
| P1329 | FIX | Same default-equivalence problem as P1328. |
| P1330 | FIX | Supply anchor; retain raw-server duplicate-channel cases. |
| P1331 | FIX | Add channel and stream exactly equal to their respective counts. |
| P1332 | FIX | Supply anchor; two-Provider test is suitable. |
| P1333 | FIX | Supply literal string-refusal mutation. |
| P1334 | FIX | Supply anchor; reserve explicit numbers before allocating names. |
| P1335 | FIX | Supply anchor; reply-table assertion is suitable. |
| P1336 | FIX | Assert empty RX frame for the RX mutation. |
| P1337 | FIX | Supply exact Python substring; bool-channel case is suitable. |
| P1338 | FIX | Supply exact substring; duplicate case is suitable. |
| P1339 | FIX | Supply exact substring; digit-only-name case is suitable. |
| P1340 | FIX | Supply exact substring; outside-membership value key is suitable. |
| P1341 | FIX | Define the actual per-stream/per-channel wire mutation. |
| P1342 | FIX | Supply exact substring; named-resource frame assertion is suitable. |
| P1343 | FIX | Pin distinct RX/TX names and numbers, then exact lookup mutation. |
| P1344 | FIX | Proposed adjacent-call swap does not exist; redesign mutation and arm-order assertion. |
| maintenance P49 | FIX | Current anchor matches once; proposed replacement is malformed and killer observes the wrong path. |
| maintenance P137 | FIX | Current anchor matches once; preserve both insert arguments in replacement. Effective-default killer is suitable. |
| maintenance P53/P54/P55/P73 | OK | Each current Rust anchor matches once; no default-helper anchor dependency. Preserve explicit case setup. |
| phase6 P01/P11 | OK | Each exact Python anchor matches once; explicit initial TX setup preserves the intended mismatch/match cases. |

Searching `plan/*/tools/mutations.json` found **P49 and P137 as the existing rows anchored on the removed `default_stream` helper**. Execution and mutation kills were not checked; no hardware behaviour was exercised.

**PLAN NEEDS CHANGES — 9 findings.**
