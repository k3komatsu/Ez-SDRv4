What this change does: adds bench verdicts, restart-fill measurements, load trials, and a runbook. The review found false-FAIL paths, a wedge-stop gap, and lost measurement evidence.

1. **Major — payload-end+1 is not safe after a held-tail close.** `crates/ezsdr-radio-uhd/tests/hardware.rs:1477–1479`
   Sequence: send 30,000 samples → close with the held last sample plus zero padding → book B at payload-end+1. The correct late rejection now produces FAIL, blocking #78.
   UR-22 includes padding in the closing send’s exclusive end; the existing FakeDevice test explicitly rejects this position (`src/provider/tx.rs:3718–3735`). Restore the negative control and correct the runbook table.

2. **Major — a control timeout can allow another load trial.** `crates/ezsdr-radio-uhd/tests/hardware.rs:2639–2661`
   Sequence: configuration times out during prepare → `run_stage` returns `Ok(AdmissionRejection, left_open=false)` → this helper returns true → the next trial touches the possibly wedged handle.
   That return path exists at `tests/common/measurement/multichannel.rs:800–801`. Standalone `usrp()` has no record guard (`:312–315`). Reuse the existing guard and stop on its timeout latch, not only `Err`/`left_open`.

3. **Major — restart trial evidence disappears after the documented command.** `crates/ezsdr-radio-uhd/tests/hardware.rs:3629–3633,3677`
   Sequence: run `hw_iii_raw_rx_restart_fill` through `hw()` → collect trial ticks, ramp10, tone and overflow evidence → append it only to a thread-local buffer → process exits.
   `push_raw_observation` neither prints nor persists it (`tests/common/measurement.rs:242–245`). The log retains summaries, not the trial evidence needed to audit them. Print the observation or explicitly save it.

4. **Major — preemption expectations mistake the window ceiling for the actual decision boundary.** `crates/ezsdr-radio-uhd/tests/hardware.rs:753–757`
   Sequence: request preemption 10 ms ahead → control processing is delayed while the repeat advances beyond that target → UR-21 correctly emits TIME_ERROR → this test reports FAIL because it expects zero.
   UR-21 decides against the actual first-unhanded sample and current device lead, not the originally requested lead alone (`design/18-uhd-radio.md:246`). Observe that boundary or classify an unestablished timing case as INCONCLUSIVE.

5. **Minor — incomplete restart trials contaminate the published ranges.** `crates/ezsdr-radio-uhd/tests/hardware.rs:3638–3642`
   The summary includes every `fill`, even trials rejected for missing tone, overflow or rate mismatch. Three valid trials plus two DC-only trials can PASS while the reported minimum includes invalid zero/ramp counts. Use the same completeness predicate as the verdict.

6. **Minor — the requested restart-path rehearsal was removed rather than relocated.** `crates/ezsdr-radio-uhd/tests/hardware.rs:3598`
   Pure verdict tests do not exercise acquisition, coverage or observation wiring. Move the shared probe into `common/` and invoke its FakeDevice rehearsal from `fake.rs`; the hardware-test ignore rule does not prevent that.

7. **Minor — several mutation descriptions do not describe their mutations.** `plan/maintenance/tools/mutations.json:13374–13412`
   N48III-04 permits an **after**-planned block, not before; -06 makes equality fail, not excess errors pass; -07 makes equality inconclusive, not fewer errors pass. N48III-14 permits at most two underflows, not three. Correct the descriptions or mutations so recorded kills substantiate the named behavior.

Not checked: builds, hardware runs, or mutation execution; review was static and read-only.
**Verdict: fix majors 1–4 before the bench session.**
