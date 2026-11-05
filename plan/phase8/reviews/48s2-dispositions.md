# Note 48 S2 — dispositions and rulings

## Rulings on Sol's review of the S2 test plan (orchestrator, 2026-11-05)

Sol: PLAN NEEDS CHANGES (sol-plan.log). All eight findings are adopted.

| # | Sev | Ruling |
|---|---|---|
| R1 | P2 | Keep the sample-time frequency history pinned: move the assertions of `ur_33_a_later_write_does_not_change_samples_already_produced` (device.rs ~3194–3210) into an untimed test (produce at f1 → stop → apply f2 → the old samples keep f1). Give §1.1's front-end preservation assertion a named destination. |
| R2 | P2 | `halted(t)` and `streaming()` are not equivalent. Keep the prefix-plan / prune assertions at their times (including the 2.050 s origin in `tests/timeline.rs:535–540`) through the public plan, assert admission's `streaming()` separately, observe refusal exclusion through the public plan, no private state. |
| R3 | P2 | Redo the mutation ledger by each row's actual `file` + `old`/`old2` location and `filter`, not by id or name match. At least J06, P832, P844–P847, P864, P871, P894, P1001–P1002, P1038–P1040, P1044, P1175–P1178. Rows that become equivalent (P1307's retarget, P1310) are deleted and replaced by rows that break the retained failure behaviour (failed reopen sends nothing). |
| R4 | P2 | Keep the report-reader exclusion test independent of DSP gates (a competing shutdown reader cannot take the flush ACK; core.rs ~1605–1647). Give the mixed FakeDevice P5/P6/P7/D70 test (fake.rs ~7654–7707) an assertion-by-assertion disposition: timed tuning assertions go, packet-state / starvation / D70 assertions stay. |
| R5 | P2 | G6 per boundary: envelope deserialization and schema rejection; each Provider's Binding-selector refusal of a removed key, if any selector carries one; measurement comparison refusing a removed `Key` (`measurement.rs` ~183–188, 405–406). Regenerate schemas through `rm_20_schema_freeze` (`crates/ezsdr-radio/tests/radio_model.rs:302`) as well. |
| R6 | P2 | Every normative amendment lands in the sub-step that changes its contract (UR-2/UR-33 with S2a-i; RM-20 counts, UR-3 `design/18:63`, UR-9 `:193`, UR-34 `:347,359,362–363` with the sub-step that removes their machinery). Correct §3.3: the receive-command FIFO (starts; STOP bypasses and clears it) and the transmit data path are not the removed timed DDC FIFO. Read UC-6 and decide it precisely. Name the current bench instructions and probe registrations that change; historical records stay. |
| R7 | P3 | Complete the `owner_step` caller/state trace in the plan, before code: for `Open` and `Nothing`, a production path (path:line) or evidence none exists. A synthetic second `owner_configure` does not prove reachability. Keep reachable configure/refusal/loss ordering; delete an arm with no production path. |
| R8 | P3 | B5: rename `CancelReason::ColdChange` to `ClockEnded` (orphaned clocks and clocks ended by a reopen failure); report mapping and counts unchanged. |
| R9 | — | B3: Sol confirms the remaining origin `ceil_to_grid(max(effective, cut + lead, max(item.ready, cut + ceil(call × old_ratio) + allowance) + lead))`. Add an exact-origin case with non-trivial rate/grid rounding in the shared timeline test (Provider agreement alone cannot see a shared bug). Profile lead stays 50 ms. |
| R10 | — | The four-way split is accepted with Sol's conditions: queue measurement removal with timed `apply`; every interface-dependent test adaptation in S2a-i; specs, fixtures, mutation dispositions and schemas with the sub-step that changes their contract. Bench probes: delete the queue and timed-stop probes (their measurements are gone with A2/A1′); the restart-lead probes stay and are re-measured after B3. |

### Round 2 (Opus re-check of rev 2: PLAN NEEDS CHANGES, small and named; no further Sol round)

Adopt every fix the re-check names (its text is in the orchestrator's records; summary here):

| # | Ruling |
|---|---|
| S1 | R1: keep the deleted sample test's observation untimed: an untimed f2 written in the middle of a loopback burst; samples before the write are the burst's, after it are not (through `rx_recv`, not `freq_at`). Add a row for N11 on `set_subdev_spec`'s settings remap (device.rs ~1306–1310). Define N1 and N2 in rev 3 (they exist only in v1). |
| S2 | R2: the replacement at tests/timeline.rs:540 and the retargets at :469 and :677 use the prefix plan through the public `made()` (timeline.rs:243) or a line booked with only items `e <= t`: at 1 999 ms `[(0, 500 ms)]`, at 2 000 ms also `(2 050 ms, None)`. Not `streaming()`. |
| S3 | R3: rows whose `old`/`old2` lie only in deleted code go to each sub-step's mechanical sweep (a script lists every `NOT FOUND` row, saved in the sub-step record, then deleted). Decided in the plan before code: rows on retained code whose anchor or killer the step removes (P215, P219 retarget; P990; P369 `new` → `Owner::Configure`; P985 retarget to a kept packet-state test; P865–P867, P872, P873, P890, P900–P906, P909, P912 keep their killers), every deleted test, and P862, P895, P911 (delete). Add the missing tests to the map: rx.rs:850, mod.rs:871, tx.rs:1344, fake.rs:7631, fake.rs:6514. |
| S4 | R4: delete the P7 assertion at fake.rs 7693–7695 (it is about the timed command, d9.rs:383 `Some(e)`). |
| S5 | R5: drop N7b. tests/measurement.rs:76–82 and :155 are retargeted to `radio.tx.repeat_max_samples` in S2b-i, not deleted. |
| S6 | R6: add design/18:249 (UR-22 sentences on timed TX DSP and the DUC FIFO; reword in S2a), :30, :193, :362–363, :368, §6 rows :451, :502, :545; design/07:378 and :384 counts; KNOWN_LIMITATIONS.md:67. Each in the sub-step that changes its contract. |
| S7 | R7 closed; N8 optional (keep as a cheap pin). |
| S8 | R8: **no `FirstSendShort`**. One `ClockEnded` for all three constructors (the short first send is cancelled because the clock's cut comes before it can start). |
| S9 | R9: choose N9's values so ceil and floor differ (e.g. old 3 MS/s, call 1 sample, new 200 MS/s grid 5 ns, base ≡ 2 mod 5). |
| S10 | R10: **keep `hw_b8_raw_timed_tune_phase`** on `UhdDevice::tune_dsp` (note 48 A1 keeps the D9 probes as UhdDevice bench functions; it is the evidence for `phase_behavior_on_retune`). Keep `tune_dsp`/`tune_auto` with `at` on `UhdDevice` only (Q3 yes). Keep `StreamState.configured` (Q5 yes). |
| S11 | Q6: delete P863. Q7: keep P985, retargeted (S3). Q8: N-10 needs a killer — run the kept reader-exclusion test over both `open_tx_device` and `reopen_tx_streamer`; keep tx.rs:1545's assertion for N-11; confirm N-12 at S2a-i; drop N4. |
| S12 | New P2: S2a-i must not delete fake.rs 748–1423 as a block. Name the queue tests one by one; keep the lease, restore, record assembly, `ur_7_retries…`, probe-array, `coercion_*`, `raw_group_*` tests, stripping only queue assertions from 1352 and 1361. P3: "delete the flows" keeps `Dsp.log`, `tx_header` and `tx_header_stale` (the TX header update inside the flows block, device.rs ~2004–2012). |

### Round 3 (on rev 3)

| # | Ruling |
|---|---|
| T1 | Split S2a-i into **α** (the queue measurement group: its functions and tests in `measurement.rs`, `MeasurementPlan` queue fields, env parsing and validation, its fixtures, rows and the hardware queue probes; the trait keeps `at`) and **β** (the interface change, FIFO, D9, N1/N2/N10/N11, A1′, A5). α first is consistent with R10: removing a caller of timed `apply` before `apply`'s `at`; the condition was "not after". Each is gated on its own. |
| T2 | S10: `hw_b8_raw_timed_tune_phase` uses `UhdDevice::tune_auto` with `at` (both bench methods stay, Q3). `hw_b8_raw_timed_retune` is deleted. |
| T3 | Dropping N4 is accepted: the flush → ACK → payload order runs in sequence inside the opener. |

## S2a-i-α code

Sol: CODE OK, no findings. Self-review (Go DeepSeek; free refused): four adopted (stale bench lines, design/18:30 clause, a stray comma, an unused import), `FakeFault::TimeReadFails` removed with its only test, one rejected (`RxRecv` is used by `measurement/multichannel.rs`). Open, outside S2: UR-34's Test line names `manifest_evidence_extracts_six_classes_and_rejects_boundary_acknowledgements`, absent at c58cb32.

## S2a-i-β code

- Implementer's questions: P1224 — first accepted for deletion, then retargeted after Sol's P2 (below). `design/18:249` (UR-22's timed TX DSP sentences) and the `hw_n47_raw_control_call_durations` rename (its timed row now times `tune_auto`) carried to S2a-ii.
- Self-review (Go DeepSeek; free refused): six adopted, one rejected (A4 thread names belong to S2a-ii), one recorded (`hw_n47` timed row).
- Sol: CODE NEEDS CHANGES, one P2 — the deleted P4 test also pinned that case C closes and reopens the streamer and case A does not, independent of the FIFO. Fixed: `ur_24_d9_p4_c_closes_and_reopens_the_streamer_and_a_does_not` with an untimed tune; P1224 retargeted to it and killed.
- Gate 1: FAIL, P1187 survived. Its killer observed the lost DUC header only through the removed FIFO (`dsp_pending`); the kept assertions do not change under the mutant. Fixed with an assertion on the DUC header (`tx_header`). Gate 2: PASS.
- Orchestrator: KNOWN_LIMITATIONS KL-9 (Gone) notes that its FakeDevice FIFO test went in S2a-i-β.

## S2a-ii code

- Implementer's questions: `reopen_failed`'s `burst = Closed` kept as the contract (a no-op on reachable paths); the dead plan cursor `self.at` left to note 47 stage 4 (B4); bench.md:549 fixed in the fix round; KL-9 already annotated in β.
- Self-review (Go DeepSeek; free refused): no P1/P2; a stale "Halted" message fixed.
- Sol: CODE NEEDS CHANGES. P2: the reader-exclusion test killed N-09/N-10 only through the `is_held` precondition. P2: the failed-reopen branch never delivered a later burst. P3: bench.md described the renamed `tune_auto` row like the others. All fixed: a test-only `ClockMutex::queued()` checkpoint with behavioural assertions on both open paths; a later burst through `command`, released reservation, no clock, one UR-25 row and one COMMAND_REJECTED; bench.md T6.
- Opus re-check: FIXES OK. N-09 and N-10 die at the behavioural assertion (3/3 each); 40/40 under CPU load; COMMAND_REJECTED is RM-25's observable. P3 for S2b: the shared reason text "its transmit clock was never started" does not fit a clock ended by a failed reopen.
- Gate 1: FAIL, P1309 survived. `tx_chain_recovery_transitions` called `provider_stop` after `reopen_failed`, already `Clean` once `Halted` was folded. The orchestrator made the test call it from `ReopenNeeded`; P1309 killed. Gate 2: PASS.

## S2b-i code

- Implementer's deviations, all accepted on Sol's evidence:
  1. N9 uses one rate. A stopped stream's rate cannot change today: rate updates are classless and staging takes frequency, gain and antenna only.
  2. N5 is on `plan()`, with Provider agreement through `rm_26`.
  3. `invalid-comparisons.json`'s repeat worst value is 15.
  4. A fake configure delay went from 8 to 5 ms; the test still asserts LATE, SHORT, OK and the measured result.
  5. The timing rows are in S2b-i's Changes table.
- `check_start_lead` (only `rx_start_lead_ns >= release_window_ns`) is removed within B3.
- KL-34 stays as its "Gone" history.
- Self-review (Go DeepSeek; free refused): adopted the P626 anchor and stale text in design/06, 07 and 18. It rejected the N9 old-ratio argument.
- Sol: no runtime defect; two P3 documentation fixes, both done by the orchestrator:
  - UR-9 now says which timing paths remain: the receive-command FIFO, which a STOP bypasses and clears, and transmit timestamps in data packets.
  - The KNOWN_LIMITATIONS lead row marks the earlier restart observations as pre-B3 and asks for re-measurement before the profile lead is lowered.
- Gate PASS (the first launch failed in chmod over the deleted schema file, before any step ran).
- Open for the bench: re-measure the receive restart lead (`hw_b8_raw_restart_lead`) after B3.

## S2b-ii code

- Implementer's deviations accepted:
  - The 1 999 ms check asserts `streaming()` true, as RM-21/RM-25 say. The prefix-plan assertions through `made()` are also written; Sol confirmed this.
  - N6 is one fixed sequence. The seeded equivalence stays in a retargeted test.
  - `owner_step` loses `by`.
- N8 is not written (it was optional). Sol accepts the deletion on the caller trace, not on "a duplicate configure is harmless".
- `StreamState.configured` is kept per S10. It has no production decision reader now; its removal is a later step's question.
- Self-review (Go DeepSeek; free refused): five minor findings, all fixed.
- Sol found no P1 or P2. Its four P3 text corrections were applied by the orchestrator:
  - the Mock phase comment now says deterministic channels only;
  - UC-6 no longer cites MockRadio and MR-18;
  - `parity.rs`'s RM-34 note says it is withdrawn;
  - the `vh_8` row in `design/18` §6 is narrowed to what those tests assert.
- Gate PASS, mutation 1693/1694 (known survivor).

## Counts (S2)

| Source | Major | Minor |
|---|---|---|
| Sol, test plan rev 1 | 6 P2 | 2 P3 |
| Opus, plan rev 2 re-check | 1 P2 (a block deletion that would have dropped retained tests' killers) | several named fixes (S1–S12) |
| Sol, code α | 0 | 0 |
| Sol, code β | 1 P2 (P4 C/A reopen sequence) | 0 |
| Gate β | 1 survivor (P1187: its killer observed through the removed FIFO) | — |
| Sol, code S2a-ii | 2 P2 (reader exclusion pinned only by a precondition; the failed reopen never delivered a later burst) | 1 P3 |
| Opus, S2a-ii fix re-check | 0 | 2 P3 |
| Gate S2a-ii | 1 survivor (P1309: the killer started from an already-clean state) | — |
| Sol, code S2b-i | 0 | 2 P3 (docs) |
| Sol, code S2b-ii | 0 | 4 P3 (text) |

Mechanisms: none of the definitions fits. Every major was a test that lost or never had its observation after a deletion, or a plan gap. The pattern to remember for later deletion steps: when a mechanism goes, check that each kept test still observes its behaviour by another path. Both gate survivors came from a test that had observed through the deleted mechanism.
