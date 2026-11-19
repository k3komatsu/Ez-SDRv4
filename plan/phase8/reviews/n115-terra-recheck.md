1. **F1 — CLOSED.** Runtime admission saves the Action’s value before Provider coercion and uses it to replace each registered check’s `requested` by key; the check still judges the coerced configuration. Scheduled and Session Actions share this path. SB-46 runs afterward and is untouched; validate and prepare continue through `admit` without an override. The key-only behavior across fragments was explicitly accepted in the decisions. admission.rs:141 (`crates/ezsdr-kernel/src/coordinator/admission.rs:141`) · session.rs:233 (`crates/ezsdr-kernel/src/session.rs:233`)

2. **F2 — CLOSED.** N114-6 now targets a reachable `Present` request, for which SB-46 must report `requested = None`. spec_binding.rs:1988 (`crates/ezsdr-kernel/tests/spec_binding.rs:1988`) · mutations.json:27396 (`plan/maintenance/tools/mutations.json:27396`)

3. **F3 — CLOSED.** The SB-30 fixture comment now says `coerce` runs and returns no coercion; RM-19’s Test line distinguishes the assertions at each stage. spec_binding.rs:2052 (`crates/ezsdr-kernel/tests/spec_binding.rs:2052`) · 07-radio-model.md:215 (`design/07-radio-model.md:215`)

4. **Minor — SB-30 retains a stale implementation name.** It says the runtime point is `Admitter::admit`, but the coordinator now calls `Admitter::admit_asked`. KC-24 names the new method correctly. 03-spec-and-binding.md:285 (`design/03-spec-and-binding.md:285`) · admission.rs:160 (`crates/ezsdr-kernel/src/coordinator/admission.rs:160`)

5. **Note — New test sensitivity:** the saved probes show the following named assertions fail under the indicated small wrong changes; I did not run them myself.

   - Mock Session test (`crates/ezsdr-acceptance/tests/v58.rs:588`): passing no `asked` map reports `3.0`, failing the original `2.9` assertion.
   - Coordinator test (`crates/ezsdr-kernel/tests/coordinator.rs:4004`): removing the overwrite reports `20.0` instead of `19.5`; retaining the check’s value for an absent key fails the `None` assertion.
   - SB-46 test (`crates/ezsdr-kernel/tests/spec_binding.rs:1988`): reporting the Module’s value fails the `None` assertion with `Some(false)`.

**Overall: NOT CLOSED** — F1–F3 are closed; the minor stale SB-30 reference remains.