# Phase 3 planning reviews

Adversarial review passes (Opus, one at a time, AGENTS.md §8) over the Phase 3 plan before Gate P. Each reviewer worked on a clone of `612b9eb` with `plan/phase3/` copied in, applied the patches, ran the tests, the mutations of Appendix C and mutations and probes of its own, and read the documents. Each finding was triaged; this file records the triage (OV-5: a review's findings are recorded with a verdict, never silently applied). A finding that changed code or tests changed the patches, which were then re-verified from a fresh clone.

| Pass | Scope | Verdict | P0 | P1 | P2 |
|---|---|---|---|---|---|
| 1 | Patches 01–05 applied to a fresh clone of `612b9eb`; every count, toolchain and gate re-run; Appendix C re-run plus 19 mutations (R01–R19) and 11 probes of its own; all documents read | NOT_READY | 2 | 6 | 14 |
| 2 | The pass-1 series applied to a fresh clone; every count re-run; pass 1's probes and mutations re-run; 13 probes (Q1–Q12 and a Session probe) and 21 mutations (N01–N18, A20p–A22p) of its own; all documents read | READY_WITH_CHANGES | 0 | 3 | 8 |
| 3 | Scoped: the pass-2 resolutions, VB-8 and the new tests, and document consistency, on the pass-2 series applied to a fresh clone; every count re-run; pass 2's probes and mutations re-run; 10 probes and 10 mutations of its own | NOT_READY | 1 | 1 | 6 |
| 4 | Scoped: the pass-3 resolutions and the `<=` refusal, on the pass-3 series applied to a fresh clone; every count re-run; pass 3's probes and mutations re-run; probes, a 260-run fuzz harness (≈ 8 400 burst records) and 13 mutations of its own | NOT_READY | 2 | 2 | 7 |
| 5 | Scoped: strict transmit emission on every transmit path and the pass-4 resolutions, on the pass-4 series applied to a fresh clone; every count re-run; pass 4's probes, fuzz harness and mutations re-run; probes and 14 mutations of its own | READY_WITH_CHANGES | 0 | 1 | 4 |

## Pass 1

Reviewer: Opus, scratch directory `$CLAUDE_JOB_DIR/tmp/review1/` (not delivered). The patches under review were the first series (583 tests, 45 mutations). Every finding below was accepted unless its verdict says otherwise. Code and test fixes were made in the prototype, the five patches were regenerated from it, and the series was re-verified from a fresh clone of `612b9eb`: 550 → 553 → 561 → 580 → 587 → 587 tests on stable, 587 on 1.85.0, Clippy clean after every patch, `116 NEW: items of 292 public items`, `ok: 310 links` (311 once `prompts/README.md` gained its link to this file), 67 of 67 mutations killed. The reviewer's probes that had failed (`probe_1`, `probe_1b`, `probe_2`, `probe_2b`, `probe_3`, `probe_4b`) pass on the new series. Of its 19 mutations, 18 are killed as written; R18's text no longer occurs in the changed stop code (`NOT FOUND`), and its adapted form M65 is killed.

### P0

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P0-1 | A stop or `cold` change in the sample period before a held burst's start — including the round at that start — left the transmitter radiating without end: the burst had already ended with `END_OF_BURST` at the held start, so the cut found no open burst, removed the held burst's segment and un-shadowed the unended repeating segment. Reached by a `Stop` Action, a `cold` transmit change and `Provider::stop`. | Accepted | `cut_segments(k)` now ends every segment of the current transmit clock at `k` and removes those that start at or after `k` (MR-32 text); the emission-refusal branches use `end_open_segment` (P2-8). New test `mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter` (stop one half-sample before and in the round of the held start, a `cold` change, `Provider::stop` with a second Mock). Mutation M49. |
| P0-2 | With a non-integer sample instant, a held burst whose first samples lie before `now` was withdrawn whole by a stop in the round `⌈s⌉`, so a receiver stepped earlier heard it and one stepped later did not; MR-32's "removes every held burst's segment" contradicted CH-9(1) and RM-16. | Accepted | A stop or `cold` change now first transmits every burst that starts before the cut (`transmit_until_cut` opens and emits held bursts in start order), then cuts; only held bursts starting at or after the cut are cancelled. RM-16, MR-25, MR-18's transmit clause, MR-32 and CH-9(1) reworded accordingly (spec 12 VB-1, VB-6, VB-7; spec 11 CH-9). The reviewer's `probe_2b` became `ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order`. Mutation M52. |

### P1

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P1-1 | `v58_08_the_receive_output_does_not_depend_on_instance_order` is vacuous: the burst was admitted seconds ahead, so nothing in the scenario could depend on order; six finality mutations aimed at it survived. | Accepted, with a correction | The reviewer's first reason is wrong: the stepping order is role rank then instance id, and a Provider's id is its first fragment, i.e. the resource name (`stepping.rs:170`), so renaming `a` to `z` does flip the order. Its second reason stands, so the test was replaced by `v58_08_a_session_hears_a_burst_from_its_first_sample_in_either_instance_order`: a Session on `ideal` radios whose `start_repeat` starts on the round of a receive block's last sample, and a `Stop`, run with the transmitter named `a` and `z`. Mutation M67 (strict publication made inclusive) kills it. |
| P1-2 | MR-32's tests miss the sample-instant evaluation of gain, phase and frequency (R04) and the channel-interleaved layout of a multi-channel waveform (R13). | Accepted | Assertions inside the 45-sample path-delay window; new `mr_32_a_two_channel_waveform_is_channel_interleaved`. Mutations M53, M62. |
| P1-3 | MR-33's "in application order" is untested (R05). | Accepted | A second, different gain change in `mr_33_gain_scales_the_samples_from_its_instant`. Mutation M54. |
| P1-4 | MR-31's receive-channel bound refusal is untested (R07). | Accepted | A coupling into `rx_channel` 2 on `x310-like` in `mr_31_…`. Mutation M56. |
| P1-5 | `mutate.py` deleted any existing scratch path, including the repository root, and could copy the repository into itself. | Accepted | The runner refuses a scratch directory equal to, inside or containing the repository root, deletes an existing directory only if it holds the marker `.ezsdr-mutation-scratch` that the runner writes, and skips `.git`, `target`, `v3`, `tmp` and `.cargo` when copying. Step 6 says so. |
| P1-6 | Phase 2's M9 kept a rejected reason that the MR-33 reversal contradicts, and Vision §13 rule 3 and §16's list (RF behaviour, clipping, in the channel) conflict with radio-side gain, LO phase, clipping and path delay. | Accepted | M9's rejected reason rewritten; decision M15 (device RF behaviour is the radio's, propagation is the channel's); Vision issue 4 in spec 12 §3. |

### P2

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P2-1 | Spec 11 cites a nonexistent VB-8. | Accepted | VB-7. |
| P2-2 | GV-4 lists `se_12_*` among changed Phase 2 tests; patch 02 does not change its code. | Accepted | Removed from GV-4. |
| P2-3 | Step 3 item 3 names only `…_000 → …_001`. | Accepted | Step 3 lists every kind of change in the diff. |
| P2-4 | Step 5's grep link check prints false positives without their containing file. | Accepted | `tools/check_links.py`, which resolves each link from its file; Steps 5 and 7 use it. |
| P2-5 | `mutate.py` counts any failure as killed, even a misspelled test binary. | Accepted | An unmutated baseline run per test command must pass with at least one test; otherwise `BASELINE FAILED`. |
| P2-6 | Survivors R08, R09 (clip counters with one clipped component), R11 (`TIME_ERROR` time), R18 (a stop on a sample instant). | Accepted | Assertions in `mr_36_…` and `mr_17_…`; new `mr_25_a_stop_on_a_sample_instant_does_not_transmit_that_sample`. Mutations M57, M58, M60, M65. |
| P2-7 | MR-17's `late_by_ns` is measured from the rounded-up instant while the event time is floored. | Accepted | MR-17 states "`late_by` is measured from `now_up`". |
| P2-8 | The unreachable emission-refusal branches cut held bursts' segments while `held` keeps them. | Accepted | They call `end_open_segment`, which cuts only the open burst's segment. The ceiling stays in Appendix B (unreachable from MockRadio's own emission). |
| P2-9 | `Medium::join` refuses a delay that overflows the root; CH-6 does not list it. | Accepted | CH-6 lists "CH-6: a delay overflows the root"; `ch_06_…` pins it (2^62 ns refused at 2 GHz, accepted at 1 GHz). Mutation M19. |
| P2-10 | The file-count done-list items cannot be read from `git status`. | Accepted | They use `git apply --stat` on the step's patch. |
| P2-11 | `ezsdr-sim`'s doc comment cites `design/11-simulation-channel.md`, which exists only after Step X. | Accepted | The comment cites "spec 11". |
| P2-12 | CH-5's noise is indexed by evaluation, so a skipped sample shifts every later draw. | Accepted | Recorded as a ceiling of CH-5 (spec 11) and in Appendix B's CH-5 row. |
| P2-13 | When a stop coincides with a held start, the open burst's record says `eob` while RM-16 names `BurstEnd::Stop`. | Accepted; decided `eob` | A burst that had already ended keeps its end; only the burst transmitting at the stop instant closes with `Stop`. RM-16 and MR-25 say so; `mr_25_a_stop_at_a_held_burst_s_start_…` pins "ends `eob` with 4 000 samples". |
| P2-14 | §0.3 does not stop on a non-count check mismatch. | Accepted | §0.3: "any other check a step names prints something other than the step says". |

Confirmed by the reviewer and unchanged: every evidence citation of spec 12 at `612b9eb`; all tagged amendments, MR-31…MR-36 and RM-23 verbatim in patch 05; UHD's `fc32 → sc16` saturation (VERIFIED) and srsRAN 4G's 45-sample X300 advance (VERIFIED; the OpenAirInterface claim stays unverified, so the profile value remains INFERRED); Z1–Z11, C1–C10 and M11–M14 supported; no scope creep; `InputStore` the only new Kernel item.

## Pass 2

Reviewer: Opus, scratch directory `$CLAUDE_JOB_DIR/tmp/review2/` (not delivered), on the pass-1 series (587 tests, 67 mutations). It confirmed every pass-1 resolution (all six failing probes of pass 1 pass; the new stop semantics survived its own probes Q1–Q9 — several held bursts before a cut, 3.84 Msps, a stop after a cold change, a Stop and a TxBurst in one round, the 45-sample path delay in a publication round, promotion bookkeeping, a held start equal to the cut, a cold change in the round of a fractional held start, `Provider::stop(Abort)`), reproduced every count, and found the following. Every finding was accepted. Fixes were made in the prototype, the patches regenerated, and the series re-verified from a fresh clone of `612b9eb`: 550 → 553 → 561 → 583 → 590 → 590 tests on stable, 590 on 1.85.0, Clippy clean after every patch, the tree equal to the prototype's, `116 NEW: items of 292 public items`, `ok: 311 links`, 75 of 75 mutations killed.

### P1

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P1-A | A stop after a `cold` transmit change is untested: ending or withdrawing segments of every clock generation, or not starting a new generation, rewrites what the old clock already radiated, and all three survived every shipped test. | Accepted | New `mr_32_a_stop_after_a_cold_change_keeps_the_old_clock_s_radiation` (the reviewer's Q3b, with the records asserted from the rules: 1 500 samples `eob`, 501 `Stop`, 1 `Stop`). Mutations M68, M69, M70. |
| P1-B | `v58_03_channel_noise_reproduces_with_its_seed`'s seeds-differ assertion passes when MockRadio joins the medium with seed 0, because on `x310-like` the LO phases already differ by seed. | Accepted | New `mr_31_the_medium_takes_the_run_s_seed` (`ideal`, no LO draws): receive sample 0 is `σ · gaussian_pair(SimRng::new(seed, "sim.channel/radio/0"))` for seeds 7 and 8. Mutation M71. The acceptance test is unchanged; Appendix B names the new test for SE-6. |
| P1-C | A second burst at the start of a burst opened in this round passes MR-16 (the open burst's next sample is still its start) and overwrites it: one burst is neither recorded nor rejected, and whether this happens depends on whether a step fell between the two Actions — as it does for two Session `start_repeat` calls at one instant. A Phase 2 defect that Phase 3's Session carriers reach. | Accepted: fixed in Phase 3 | New amendment VB-8 (spec 12; `00-overview.md` §2): MR-16 also refuses a burst whose start the open burst has, with the reason "MR-16: the open burst already has this start". New test `mr_16_a_burst_at_the_open_burst_s_start_is_refused`; the reviewer's Session probe then recorded the rejection. Mutation M72. Replacing the open burst instead was rejected (MR-16 already refuses the same start while both bursts are held). *Superseded by pass 3's P0-1: the refusal became "at or before the open burst's next sample", and the test `mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused`.* |

### P2

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P2-1 | Nothing pins the exact-equality frequency gate (C4). | Accepted | `ch_04_…` adds a transmitter at 1 GHz + 1 Hz, asked and not counted. Mutation M73. |
| P2-2 | MR-31's reason prefixes ("MR-7: " for a reader error, "MR-31: " for a join refusal) are not pinned. | Accepted | `mr_31_channel_mode_…` asserts `starts_with("MR-31: CH-6: …")` and, for a section that is not an object, `starts_with("MR-7: CH-1: ")`. Mutations M74, M75. |
| P2-3 | CH-9 holds only if the loop steps at every instant a radio scheduled. | Accepted | CH-9 states the precondition (KC-20, MA-30) and that a harness must step at every due wakeup; Appendix B's CH-9 row names it. |
| P2-4 | Step 5 item 2 and spec 12's introduction say every quoted passage ends with a *(Phase 3, …)* tag; KC-11's and the InputStore row's end "(KB-1)", and table rows carry none. | Accepted | Spec 12's introduction now says a quote is copied tag included and which tag forms occur; Step 5 item 2 checks verbatim presence, tag included. |
| P2-5 | CH-9 claimed `ch_09_the_receive_output_…` swaps the order at a mid-block stop, where the receiver publishes nothing. | Accepted | The claim is reduced to what the test does: the round in which a burst starts at the instant of the receiver's last block sample. |
| P2-6 | Pass 1's record says `ok: 310 links` and "its 19 mutations are all killed"; a fresh clone prints 311, and R18 is `NOT FOUND` against the changed code. | Accepted | Pass 1's paragraph corrected. |
| P2-7 | M45's name ("no segment registered") does not match its mutation (a wrong origin). | Accepted | Renamed "segment registered on a wrong origin". |
| P2-8 | In `design/09` as patched, the *Checked by* markers of MR-14, MR-17 and MR-25 name only Phase 2 tests. | Accepted | Spec 12 VB-4, VB-5 and VB-6 amend the three markers to add the Phase 3 carriers; VB-8 amends MR-16's. |

Found while applying pass 2, beyond its findings: spec 12 VB-7's **Code** said patch 04 hands the rig its `Medium` (it is patch 03, as Step 3 says); spec 12 quoted MR-25's `Stop` sentence differently from patch 05; and spec 12 did not state the status-row amendments of specs 04–06. All three corrected; the texts of spec 12 and patch 05 were compared mechanically again.

## Pass 3

Reviewer: Opus, scoped to what pass 2 changed, scratch directory `$CLAUDE_JOB_DIR/tmp/review3/` (not delivered), on the pass-2 series (590 tests, 75 mutations). It confirmed every pass-2 resolution except P1-C, reproduced every count, found Appendix C equal to `mutations.json` row for row, and found the following. Every finding was accepted. Fixes were made in the prototype, the patches regenerated, and the series re-verified from a fresh clone of `612b9eb`: 550 → 553 → 561 → 584 → 591 → 591 tests on stable, 591 on 1.85.0, Clippy clean after every patch, the tree equal to the prototype's, `116 NEW: items of 292 public items`, `ok: 311 links`, 78 of 78 mutations killed.

### P0

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P0-1 | VB-8 closed only one case of its defect. MR-16 still admitted a burst that starts exactly at the open burst's next sample once the block ending there had been emitted without `END_OF_BURST`: the first burst was recorded as ending `discontinuity` ("MR-15: the burst's blocks were not contiguous"), the second was cut after one sample ("MR-24: the device rejected an unclosed burst"), and on a channel a sample was radiated that no record held. Reached through a Session (a `start_repeat` handled between a block's last sample and the next sample on `ideal`); also present at `612b9eb`. The false ceiling "the MR-15 and MR-24 branches are unreachable, as Phase 2 recorded" rested on it — Phase 2 recorded no such thing. | Accepted | MR-16 refuses a burst that starts **at or before** the open burst's next sample, one comparison (`start <= open.next`) with the reason "MR-16: burst begins at or before the open burst's next sample"; it subsumes pass 2's separate same-start refusal, which was removed. VB-8 rewritten (spec 12: evidence of both cases, the rejected alternatives — closing with `Stop`, a zero-length `END_OF_BURST` block — and the consequence on `ideal`); `00-overview.md` §2 and Z11; Appendix B's MR-32 ceiling corrected to an INFERRED statement that holds with VB-8. The test became `mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused` with both cases; M72 now reverts `<=` to `<`. The reviewer's probes PA, PB, PC and its Session probe now show the second burst refused and every radiated sample recorded. |

### P1

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P1-1 | The VB-8 test does not pin the `SendAsap` path MR-16 covers (judged on the moved start; `TIME_ERROR { outcome: refused }` before `COMMAND_REJECTED`). | Accepted | The test's third case: a late `send_asap_and_flag` burst for sample 500 moved onto the open burst's start is refused with one `TIME_ERROR { outcome: refused }` and one `COMMAND_REJECTED`. Mutation M78. |

### P2

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P2-1 | VB-8's reason after the open burst has emitted is not pinned. | Accepted | Moot: one refusal and one reason now, asserted in all three cases. |
| P2-2 | The CH-4 gate is pinned as exact only to 1 Hz. | Accepted | `ch_04_…`'s off-frequency transmitter is now one `f64` step above 1 GHz. Mutation M77 (a 0.5 Hz tolerance). |
| P2-3 | MR-25's "every held burst that starts before that sample" is tested with one such burst only. | Accepted | New `mr_25_a_stop_transmits_every_held_burst_before_it` (pass 2's probe Q1: three held bursts before a stop whose round is the first after them, both orders, values derived from MR-25 and MR-32). Mutation M76. |
| P2-4 | Spec 12 VB-3's evidence says "VB-4…VB-7". | Accepted | VB-4…VB-8. |
| P2-5 | Step 3's "Read first" omits VB-8. | Accepted | VB-3…VB-8. |
| P2-6 | The Z11 row omits VB-8. | Accepted | "VB-4…VB-6 and VB-8 without a channel". |

## Pass 4

Reviewer: Opus, scoped to what pass 3 changed, scratch directory `$CLAUDE_JOB_DIR/tmp/review4/` (not delivered), on the pass-3 series (591 tests, 78 mutations). It confirmed the `<=` refusal closes both of pass 3's cases and found no way for MockRadio's own emission to reach the MR-15 or MR-24 refusal branches (260 fuzz runs, `ideal` at 1, 3 and 3.84 Msps and `x310-like`). It found two P0s with one cause: a transmit block was emitted in the first pass of the round at its last sample's instant, so an Action handled in a later pass of that round found the sample at its own instant already recorded. Every finding was accepted except P2-5. Fixes were made in the prototype, the patches regenerated, and the series re-verified from a fresh clone of `612b9eb`: 550 → 553 → 561 → 585 → 592 → 592 tests on stable, 592 on 1.85.0, Clippy clean after every patch, the tree equal to the prototype's, `116 NEW: items of 292 public items`, `ok: 311 links`, 82 of 82 mutations killed.

### P0

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P0-1 | A burst whose last block (with `END_OF_BURST`) was emitted in the first pass of a round leaves no open burst, so a burst admitted in a later pass at that last sample was on time and MR-16 had nothing to compare it with: two records held one sample and only the second was radiated. Reached through a Session; present in the Phase 2 code. | Accepted | Root cause fixed rather than a second frontier added: VB-4 now also makes MR-15 strict — a transmit block is emitted once its last sample is in the past, as MR-14 says of a receive block (MR-15 already said "as MR-14 says", which VB-4 had made strict while the code stayed inclusive). In every pass of the round at a sample's instant that sample is untransmitted, the burst is still open, and a burst at its last sample takes over there with a consistent record. New `mr_15_a_transmit_block_is_emitted_only_after_its_last_sample`; mutation M79. |
| P0-2 | A stop or `cold` change handled in a later pass of that round cut the radiation at the sample the first pass had already recorded: records of 1 000 samples, 999 heard. Reached by a Session `Stop`, a `cold` change and `Provider::stop`; present since pass 1's stop cut. | Accepted | The same change: the stop's cut is the first sample at or after its instant, and that sample is never emitted before the stop in any pass, so RM-16 ("every transmit sample before the stop instant") holds exactly and record and radiation agree. The reviewer's candidate — tracking the first sample not yet transmitted and raising the cut and MR-16's refusal to it — was rejected: a second notion of "now", and a stop would transmit the sample at its own instant in one pass and not in another, against RM-16. Four Phase 2 tests step one root tick later (GV-4, spec 12 VB-4). The reviewer's Session probes now give recorded = heard for every stop instant, and its fuzz harness finds no overlap and no mismatch. |

### P1

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P1-1 | The VB-8 `SendAsap` case cannot tell the moved start from the original target (both at or before the open burst's next sample). | Accepted | A further case: in the round at 1 000 500 ns a late burst for sample 500 is moved to 1 001 and admitted, and the first burst ends `eob` after one sample. Mutation M80. |
| P1-2 | Nothing checks that a burst just past the refusal is admitted (`next + 1` refused survives). | Accepted | Case b adds a burst at 1 002, admitted; the first burst ends `eob` after two samples. Mutation M81. |

### P2

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P2-1 | In spec 12, `---` directly under VB-8's **Tests.** paragraph turns it into a heading. | Accepted | A blank line before it. |
| P2-2 | VB-8's *Consequence* sentence was false, and omitted that every asap switch of a one-sample waveform on `ideal` is refused. | Accepted | Rewritten for strict emission: refused when handled after a transmit block has been emitted and no later than the next sample's instant; for a one-sample waveform, every such switch; never on `x310-like`. |
| P2-3 | The VB-8 row in spec 09 §6 claimed a `Stop`, a receiver and a record check for every case. | Accepted | The row describes each case as the test does. |
| P2-4 | `00-overview.md` §3 and Step 3 still described VB-8 as "the open burst's start". | Accepted | "no burst at or before the open burst's next sample". |
| P2-5 | The order of `TIME_ERROR { refused }` and `COMMAND_REJECTED` is not asserted, and MR-16 does not fix one. | Rejected | The order is not part of MR-16 (Phase 2's `mr_16_burst_refusals` does not order them either), and fixing one would add an obligation no consumer needs; the test asserts one of each. |
| P2-6 | VB-8 without a channel is untested (a channel-only refusal survives). | Accepted | Case a runs with a loopback channel and without one. Mutation M82. |
| P2-7 | The test's `late` closure did nothing (`burst()` already sets `send_asap_and_flag`). | Accepted | Removed. |

## Pass 5

Reviewer: Opus, scoped to strict transmit emission and the pass-4 resolutions, scratch directory `$CLAUDE_JOB_DIR/tmp/review5/` (not delivered), on the pass-4 series (592 tests, 82 mutations). It found no transmit path where strict emission breaks a rule or where records and radiation disagree — the end of a Spec Run, `Provider::stop` in both modes, cold changes at a block's last sample, wraps, 3 and 3.84 Msps, two `x310-like` Mocks with the path delay in either order, Sessions — and no livelock of the quiescence loop; it confirmed every pass-4 resolution, judged the rejection of pass 4's P2-5 sound, and re-ran pass 4's fuzz harness (8 231 burst records, no mismatch). It found the following; every finding was accepted. Fixes were made in the prototype, the patches regenerated, and the series re-verified from a fresh clone of `612b9eb`: 550 → 553 → 561 → 585 → 592 → 592 tests on stable, 592 on 1.85.0, Clippy clean after every patch, the tree equal to the prototype's, `116 NEW: items of 292 public items`, `ok: 311 links`, 83 of 83 mutations killed.

### P1

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P1-1 | Strict transmit emission is tested only in channel mode, although Z11 says VB-4 holds in both: strict emission in channel mode only survived every test, and without a channel it lets two records hold one sample again. | Accepted | `mr_15_a_transmit_block_is_emitted_only_after_its_last_sample` runs both cases with a loopback channel and without one (records always; samples heard with the channel). Mutation M83. |

### P2

| Id | Finding | Verdict | Resolution |
|---|---|---|---|
| P2-1 | Spec 12 VB-8 said `mr_16_burst_refusals` is unchanged, against VB-4 and GV-4; and with strict emission the "before" half of MR-16's open-burst refusal is unreachable. | Accepted | VB-8's **Tests** says the test changes only by VB-4's step instant and that the refusal fires only at the open burst's next sample; Appendix B's VB-8 row records the unreachable half as a ceiling. |
| P2-2 | Appendix A said eight Phase 2 tests step one root tick later; twelve do. | Accepted | "twelve (eight for receive publication, four for transmit emission)". |
| P2-3 | VB-8's refusal depends on which pass of the round that emits a block handles the Action (a Spec's agenda Action comes in the first pass, before the emission; a Session's after it); both outcomes agree with the radiation. | Accepted | One sentence in VB-8's *Consequence*. Emitting before handling Actions was rejected (it reorders spec 09 §4, settled in Phase 2). |
| P2-4 | The Review C prompt did not name strict transmit emission. | Accepted | "strict publication and emission (MR-14, MR-15) and the next wakeups". |

