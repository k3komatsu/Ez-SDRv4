
> plan · gpt-6.1-sol
## Verdict: PLAN NEEDS CHANGES

Read-only review at `c58cb32`. The removal decisions stand, but the test map and mutation ledger are not implementation-ready.

### Independent scenarios

1. Untimed configuration loses coercion, read-back, refusal, delay, or historical frequency semantics.
2. Flush cleanup loses report ownership, allowing another reader to consume its ACK.
3. Failed TX reopen sends held or subsequently queued payload.
4. RX restart loses an old-rate call, allowance, lead, or lattice rounding along with the release window.
5. Obsolete serialized keys are silently accepted.
6. Owner simplification skips/repeats configuration or invents an unreachable recovery path.
7. Deleting `halted` accidentally changes refusal, streaming admission, or pruning.

### Findings

**1. P2 — The frequency-history deletion removes a retained behavior without a real replacement.**  
Plan §1.2 deletes `ur_33_a_later_write_does_not_change_samples_already_produced`. Its assertions check sample-time frequency history, not merely timed FIFO consumption (`crates/ezsdr-radio-uhd/src/device.rs:3194–3210`). That history remains used by `sample_at` (`:1172–1178`).

- Counterexample: produce RX samples at one frequency → stop → apply another frequency → retrieve already-produced samples. A simplified fake consulting only current settings changes those old samples retroactively.
- The retained immediate-write assertion (§1.1) cannot detect this. N1 only tests delay, despite §1.1 also assigning it a front-end preservation assertion.
- **Required:** explicitly move historical-frequency/sample assertions into an untimed test; give the front-end preservation assertion its actual destination.

**2. P2 — The proposed `halted` replacements assert the wrong semantics.**  
Plan §1.9 proposes `!streaming` at 1,999 ms and `streaming` at 2,000 ms. But `streaming()` takes no time argument and replays **all booked items**; a refused start deliberately leaves it true (`crates/ezsdr-radio/src/timeline.rs:258–266`). `halted(t)` instead replays a time prefix (`:249–255`).

- Counterexample: Stop → refused S1 → Stop at 2 s → S2 at 2 s. The existing test has S2’s sample origin at **2.050 s**, not 2 s (`crates/ezsdr-radio/tests/timeline.rs:535–540`).
- The proposed replacement either fails correctly implemented code or loses the original time-sensitive assertion.
- `refused_running` is private planner state, not an existing integration-test query.
- **Required:** retain prefix-plan/prune assertions, separately assert admission’s `streaming()` semantics, and observe refusal exclusion through the public plan. Do not claim these queries are equivalent.

**3. P2 — The mutation inventory is materially incomplete; some proposed retargets become equivalent mutants.**  
Plan §2 misses at least:
- `J06`;
- `P832`, `P844–P847`, `P864`, `P871`, `P894`;
- `P1001–P1002`, `P1038–P1040`, `P1044`;
- `P1175–P1178`.

These include anchors inside the removed queue implementation and the removed DSP-gate paths (`plan/maintenance/tools/mutations.json:448,8856–9006,9178–9478,10352–10622,11445–11475`).

- Failure sequence: remove queue/gate code → retain these rows → the gate cannot find anchors or invokes obsolete filters.
- After `reopen_failed` sets `Clean`, merely retargeting P1307’s old variant makes its current replacement identical. P1310’s substitution of `reopen_succeeded()` also becomes behaviorally equivalent when both transitions close the burst and set `Clean`.
- **Required:** classify rows by their actual source locations and filters, not identifier matches alone. Delete equivalent rows; replace them with mutations that break the retained failure behavior.

**4. P2 — Mixed TX tests are not fully mapped, and N4 does not preserve the existing report-reader exclusion test.**  
Plan §1.6 replaces the timed-DSP barrier with flush → ACK → payload ordering. But the existing test also starts a **competing shutdown report reader** and proves it cannot steal the flush ACK (`crates/ezsdr-radio-uhd/src/provider/core.rs:1605–1647`). N4 contains no competing reader.

- Counterexample: opener waits for flush ACK → shutdown reader calls `tx_async` → shutdown consumes ACK → opener times out or misattributes reports. N4 can pass without testing this.
- Likewise, the mixed FakeDevice P5/P6/P7/D70 test uses timed tuning and timed-history tuples (`crates/ezsdr-radio-uhd/tests/fake.rs:7654–7707`), but has no assertion-by-assertion disposition. Naming ignored hardware tests is not its replacement.
- **Required:** retain the report-reader exclusion assertions independently of DSP gates; explicitly split obsolete tuning assertions from retained packet-state, starvation, and D70 assertions.

**5. P2 — G6 tests only an envelope, not the claimed document boundaries.**  
Plan G6 attributes profile/Binding refusal to `TimingEnvelope::deny_unknown_fields`, but `Binding` contains selectors and profile references—not a `TimingEnvelope` (`crates/ezsdr-kernel/src/binding.rs:31–46`). Selector refusal occurs in each Provider. Measurement keys are values of a generic `Key`; their rejection occurs in `measurement::compare`, not JSON unknown-field validation (`crates/ezsdr-radio/src/measurement.rs:183–188,405–406`).

- Counterexample regression: load a measurement record containing a removed key → comparison silently ignores that row. N7’s envelope-deserialization test remains green.
- **Required:** specify and test envelope deserialization/schema rejection, both Providers’ relevant Binding-selector rejection, and measurement comparison rejection separately. Distinguish schema-valid parsing from semantic refusal.
- Schema regeneration must invoke radio’s `rm_20_schema_freeze` (`crates/ezsdr-radio/tests/radio_model.rs:302`), not only the kernel test named in §3.7.

**6. P2 — Spec changes are deferred past their implementation gates and include incorrect replacement text.**  
Plan §6 puts interfaces/schema/origin changes in earlier substeps but their normative text in S2b-ii.

- Failure sequence: gate S2a-i → commit a trait without timed arguments while UR-2 still requires them; gate S2b-i → publish six timing members while RM-20 still specifies eight.
- §3.3’s proposed “receive-command FIFO … [is] the fake’s DUC and DDC streams” conflates separate mechanisms. UHD’s RX command FIFO handles starts; STOP bypasses and clears it. It is not the removed timed-frequency DDC FIFO ([FPGA source](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v#L157-L199)).
- Missing exact amendments include UR-3’s release-window prose (`design/18-uhd-radio.md:63`), UR-9’s queue-accounting prose (`:193`), UR-34’s remaining queue-group text (`:347,359,362–363`), and RM-20’s member/schema counts.
- **Required:** attach each normative amendment to its implementing substep. Read and decide UC-6 precisely rather than broadly removing a generic update-class contract. Preserve historical records, but identify current bench instructions/probe registrations that must change.

**7. P3 — G7 establishes a state transition, not production reachability.**  
`by: None → Configure` is correctly established. However, “failed open → Unknown → next segment opens” is incomplete: an RX open failure during `arm` returns an error before RUN (`crates/ezsdr-radio-uhd/src/provider/mod.rs:555–560`). Runtime apply failure preserves the open set (`core.rs:1345–1347`).

N8 also leaves the production cursor-reset path unresolved. Complete the caller/state trace before implementation; a synthetic second `owner_configure` call does not prove production reachability. Preserve reachable configure/refusal/loss ordering, but do not retain `Open` solely because `Unknown` exists.

**8. P3 — `Refusal` does not describe every B5 constructor.**  
A burst can be cancelled because its clock ended at origin without any configuration refusal (`crates/ezsdr-radio-uhd/src/provider/tx.rs:312–348`). The cited `cold_change_refused` mechanism is withdrawn. Prefer **`ClockEnded`**, covering both orphaned clocks and clocks ended by reopen failure; preserve report mapping and counts.

### B3 and split assessment

The intended post-removal origin is correct:

`ceil_to_grid(max(effective, cut + lead, max(item.ready, cut + ceil(call × old_ratio) + allowance) + lead))`.

Keep the 50 ms profile lead. Strengthen the exact-origin case with nontrivial rate/grid rounding; Provider agreement alone cannot detect a shared timeline bug.

The four-way split is reasonable, with these conditions:
- queue measurement removal accompanies timed-`apply` removal;
- every interface-dependent test adaptation happens in S2a-i;
- specs, fixtures, mutation dispositions, and schemas accompany the substep changing their contract;
- the unresolved deletion classifications are completed before implementation.

No files changed; no builds, tests, mutations, or bench runs performed. Hardware timing margins remain unverified.
