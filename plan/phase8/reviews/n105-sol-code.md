The prepare check covers all `Value` fields in `PrepareReport`. I found a validate-side bypass to trace further: an unrequested coercion’s `requested` value is copied into a violation before the new ASCII check runs.
## Verdict: FAIL

The change fixes the intended prepare and coercion-preview paths, but #104 still has a bypass in the edited validate path.

### Findings

1. **Major — a stray coercion bypasses the ASCII check**  
   `crates/ezsdr-kernel/src/plan/matching.rs:133–157`

   The code partitions out unrequested coercions **before** checking ASCII keys. It copies their unchecked `requested` into `Violation.requested` at line 138.

   Minimal sequence:
   ```text
   Provider.coerce returns:
     coercion.key = an unrequested key
     coercion.requested = Map {"radio.ü": 1}
   validate → SB-44 violation containing that Map
   finish → Manifest.admission contains the non-ASCII key
   seal → NonAsciiKey; Manifest remains unsealed
   ```
   No transmission occurs, but the rejected Run still loses its Manifest hash—the refusal itself carries the forbidden value. Check **all** returned coercions before either partition or transcription.

   **VERIFIED:** unchecked data flow through `pipeline.rs:263`, `ending.rs:137,169–171`. **INFERRED:** counterexample outcome; not executed.

2. **Minor — the withdrawn-rule regression test trusts the list it should protect**  
   `crates/ezsdr-kernel/tests/kernel_surface.rs:107–115`

   Iterating `WITHDRAWN` tests rejection of its members, not completeness. Restore the old seven-member list and this test still passes, recreating #105 item 2. Add independent assertions for the five newly discovered IDs, or compare against the spec stubs.

   **VERIFIED** by test inspection.

### SB-9a entry-point inventory

| Producer path | Result |
|---|---|
| Provider/Sink/Executor `PrepareReport.effective`, coercion `requested` and `applied` | Checked together at `plan/prepare.rs:67–74`, before arm. Other report fields are typed keys and warning strings, not arbitrary objects. |
| `CoerceReport.coercions` → preview | Checked at `plan/matching.rs:156–166`; rejected coercions are omitted. **Stray branch remains open**, finding 1. |
| Runtime Provider `CoerceReport.applied` → Action/coercions | `coordinator/admission.rs:259–278` → `session.rs:254–260` → admission returns before dispatch (`admission.rs:202–203,284–289`). |
| Module Actions, including `Emit.payload` | Checked before dispatch at `coordinator/admission.rs:77–105`. |
| Control-event payloads | Checked before counting/queueing at `event.rs:671–677`. Hot-path payloads are byte arrays, with no object keys. |
| Provider sections, including records added during cleanup | Checked at `manifest.rs:420`, called by `coordinator/ending.rs:162–166`. This is cleanup-time ingestion—not a pre-transmit guarantee for content produced later. |
| `ModuleError.detail` | Does **not** reach these records. Paths retain `message`, e.g. `pipeline.rs:605`, `run.rs:587`, and device-loss payloads at `pipeline.rs:1266`. |
| Sink cleanup artifacts | `ArtifactRef` contains no arbitrary `Value`/JSON fields (`manifest.rs:24–44`); stored at `ending.rs:429–432`. |
| Registered Module/Vocabulary admission-check output | **Still open, pre-existing:** `Violation.requested: Option<Value>` is accepted unchecked at `binding.rs:360–361`; validate retains it through `plan/validation.rs:101` into the Manifest. A buggy check can independently produce the same unsealed-Manifest outcome. |

### Other checks

- **Refusal shape/stage:** The three new refusal blocks use safe `key: None`, `requested: None`, and reasons that do not transcribe the offending key. Prepare and validate occur before arm; Admitter returns before dispatch. Clean previews retain their order and content.
- **EventSource:** Both forms retain JSON `{"kind":"kernel"}` / `{"kind":"unforeseen"}`. Tags, field order, Display and hot-path encoding are unchanged; no hash-input change identified. Python returns decoded event dictionaries unchanged (`python/ezsdr/session.py:734–736`).
- **Schemas:** Freeze passes. Only two descriptions changed; schema structure is identical.
- **OV-23:** The 12-entry list matches the Kernel-prefix withdrawn stubs. Searches: case-insensitive `withdrawn` across `design/*.md` and both phase overviews.
- **OV-23b:** Correctly asserts **113 of 293**, consistent with the orchestrator’s “excludes” decision.
- **Tests/mutations:** All **390 tests passed** across coordinator, run_session, spec_binding, kernel_surface and schema_freeze. New refusal tests distinguish the old unchecked behavior, but miss finding 1. All nine requested mutation anchors match literally once; inspected `rows.log` shows their final outcomes are killed. Re-anchored rows preserve their intended behavior.
- **Maintainability:** No concrete inconsistency warrants a new abstraction for the three ASCII checks; they already reuse `Value::check_ascii_keys`.

Not checked: full workspace/gate, mutation reruns, hardware, or before/after whole-Manifest byte replay. No files changed or formatted.
