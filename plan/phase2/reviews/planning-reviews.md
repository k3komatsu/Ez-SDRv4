# Phase 2 planning reviews

Three adversarial review passes (Opus, one at a time, AGENTS.md §8) ran over the Phase 2 documents before Gate P, on 2026-09-24. Each finding was triaged and applied to the specs and to `20-implementation-plan.md`; this file records the triage (OV-5: a review's findings are recorded with a verdict, never silently applied). Every verdict below is "accepted and applied" unless it says otherwise.

| Pass | Scope | P0 | P1 | P2 |
|---|---|---|---|---|
| 1 | everything: specs 00, 06–10, the plan, the patch | 10 | 22 | 24 |
| 2 | verify pass 1's resolutions; review the changed text; anything else | 2 | 10 | 22 |
| 3 | verify pass 2's resolutions; review the changed text; P0 only elsewhere | 1 | 6 | 11 |

## Pass 1

P0
1. KC-43 vs `RunStateMachine::new` (`Created` has `at: None`) → KC-43 amended; kc_01 test asserts `at` only after the first.
2. kc_15 fixture lacked `class` → fixture fixed in spec 06 §12 and plan step 5.
3. Scheduled actions admitted against the prepare-time configuration (RF envelope bypass) → KC-17: cumulative admission in (instant, index) order against a working copy; KC-18 dispatch in that order; new test kc_17_scheduled_updates_are_admitted_cumulatively (TestLimitsCheck gains an optional `gate`).
4. rs_17 Session joint-limit test impossible → TestProvider::with_effective, run_registry_classed (test.grid/test.flag class Cold).
5. kc_23 unknown target via schedule is refused by validate first → the test is a Session SetParameter { nothing } in step 7.
6. kc_24 module action during cleanup impossible → ProbeExecutor::submitting_at(10, …) with a Provider tail wakeup at 10.
7. v58_05 device_lost before block 0 → at_ns 5 ms; artifact partial, 4 000 samples.
8. v58_11 short lead impossible in a Spec Run → Session test.
9. v61_04 pps/rx never registered → output rec_pps fed from pps.rx.
10. MR-19 pseudo-code overwrote the overrun → emit_rx rewritten; lost = whole time jump; next = max(block end, restart).

P1
11. BurstTracker lifecycle → MR-15: new tracker/device model at each TX clock registration.
12. Cold changes before T0 → receive e = max(e, T0); end a clock only if registered.
13. Acceptance horizons → horizon column with derivations.
14. KC-24 violation texts → table in KC-24.
15. Fixture gaps (kc_23 clock within dev/0, rs_17 scheduled clock + class cold, kc_19/kc_09 inputs) → fixed.
16. Stop steps on unprepared instances → KC-39: only prepared instances; kc_12 asserts c:stop absent.
17. Drain vs deadline → drain counts same-instant wakeups, stops on abort and on a `closing` flag set when run_cleanup returns.
18. mr_25 off by one → expected last sample at stop + 999 µs.
19. MR-6 unreachable case → MR-6 checked-by text changed.
20. Hardware-timed updates unobservable → section ezsdr.radio.mock.applied.
21. Cut events disagree → spec 09 §4 and plan: receive cuts / transmit cuts; hardware_timed cuts nothing; "at until" dropped.
22. MR-19 zero gap / fan-out → k_g ≥ block end; a Block link must be the only rx link (MR-7).
23. Bad capture value kills the Run → HD-14: sink.REQUEST_REJECTED event, step returns Ok.
24. Kernel configuration vs refused updates → KC-27 states it records what was admitted; ceiling.
25. Scope/markers → KA-22 amends spec 04; RM-22 payload types and schemas; MA-17 → Phase 9, MA-19b → Phase 5; RS-31 carried by RM-22.
26. Session TX before S → MR-16 refuses a burst starting before S.
27. v58_13/16 before T0 → Session rows start with advance_to(T0 + 1 ms).
28. links_by_ref missing → added to the RunHandle listing.
29. Plan/spec divergences → resolved in the spec texts (MR-11 single wakeup, etc.).
30. start_lead 0 → MR-11's reason names both instants and ezsdr.time.start_lead_ns.
31. SB-16 sink targets with differing rates → documented as K7's ceiling.
32. KC-14 first failure; KC-17 refused agenda item → stated.

P2 (33–56): fixed in text (SimEngine `&self.time`; type slips; on_block handling; counts; KERNEL_SOURCE justification; MR-16 300-block case; publishing_every; ma_05a now==until via a `p:now:` probe line; commits forbidden outright; MemLink locks; Appendix B; Y11/PO-7/PO-9; RS-9 amended; RM-6 None for 0; mr_26 row; v61_01 citation; transport 1.0 GB/s (Y13); TM-13e amended for cold TX; wording of KC-31/KC-30/RM-17/UC-6; ezsdr.failure only when the Termination is Failed; KC-39 mode at call time; wakeup draws the planned length; SendAsap requested target; actual_start absent; rig contracts; waveform ids; v58_03 counts 11/9 with advance_to). Also: SimAuthority returns Some(t) on LimitExceeded; SE-5 ceiling for outputs; KA-4 validate map includes needs (patch changed, test extended, mutation-checked); KA-11's MA-30 sentence identified.

## Pass 2

P0
1. v58_03 block counts ignored the orderly stop tail → 12 (seed 7) and 11 (seed 8), with the derivation (blocks ending by sample 26 000; the tail block is cut there).
2. MR-19/MR-21 `lost` vs RM-17 when a gap is pending → MR-19, MR-21, MR-22 and RM-17 say `lost` is the whole time jump including pending loss; flags are added to pending flags; MR-19's k_g = max(b, a + ceil(gap·den/num)); mr_19 fixture pinned (x310-like, 1 Msps, Block cap 1, k_g 52 000, lost 50 000, rx_blocks 2).

P1
3. mr_25 on `ideal` (tail 0) → x310-like.
4. MR-11 reason in the plan → the spec's reason string, with the numbers.
5. Step 13's `start_k` Err → HD-14 event, pop, continue.
6. kc_39_abort could not fail → a pending wakeup at 10; assert no step after the first stop.
7. Abandoned drain during steps 3–7 → `closing` set on entry to every cleanup operation after the drain's own call; drain rounds skip instances already cleaned (done has (5, inst)); KA-12 text amended.
8. Mock checks updates against the wrong configuration → MR-18: coerce at application, against the configuration at `e`; new test mr_18_a_scheduled_pair_is_checked_when_it_applies.
9. catch_unwind around validate/plan/collect_prepare and instance() at assembly → KC-30 amended; plan uses contain_all.
10. Downgraded STEP_LIVELOCK loops forever → KC-22: if the reaction is neither stop nor abort, Failed { run } in abort mode; test kc_22_a_downgraded_livelock_still_ends_the_run; the drain also stops.
11. Sections from construction → MR-27: all six exist from construction.
12. Untested refusals → kc_24_a_module_stop_without_target_is_refused, kc_24_a_burst_to_a_non_provider_target_is_refused, kc_24_a_burst_time_on_an_unrelated_root_is_refused, kc_26_a_provider_that_applies_nothing_is_refused, kc_09 extended (size, uri), kc_16_off_root_negative_and_overflowing_times_are_refused; mutation checks listed.

P2
- §3 of spec 06 now says the documents, registries, machine and configuration are in Shared (the Module submitter needs them).
- StopTx uses the mode at call time (KC-39).
- KC-24 table wording used in the plan.
- KC-17's working copy is local (`admit_with(…, &working)`); configuration changes only at dispatch.
- Step 6 mutation check for the working copy.
- Policy compile failure is Failed { validate } (KC-7 amended).
- Drain runs at the top of perform(StopRx), before the filters.
- rx_samples uses `stop − next` before `next` changes.
- MR-19 exact k_g formula.
- TxBurst duplicate/behind-open/before-S checks on the final start.
- ideal repeat_max_samples = u32::MAX (waveform_len is u32).
- zero pattern written explicitly (MR-13).
- SE-4 says "first step".
- RM-17 flags only when a sample is lost.
- RM-11 names the device node as LATE_COMMAND/QUEUE_FULL/COMMAND_REJECTED source.
- mr_25 abort wording aligned.
- KC-42 floor/ceil stated.
- mr_18_a_cold_transmit_change_replaces_the_tracker added.
- kc_45 uses a non-NONE fidelity.
- Kernel sections/payload shapes stated in KC-45 with a "no schema in Phase 2" ceiling; KC-22 payload { "wakeups" } stated.
- Appendix B generator fixed (KA-22, MR-27 rows).
- §11 lists H1–H8.
- A dropped request takes no `<output>_<k>`.
- DropCarry only to a capture recording before the block.
- rs_17 scheduled fixture uses TestProvider::with_grid.
- se_10 spec row matches the plan (advance to tick 7).
- "32 MB/s at instances: 4".
- MA-10's "Phase 2 value" carried by RM-2 (host memory in 1.0.0), added to the marker table.
- A Spec cannot schedule a burst on a clock a scheduled cold change creates: stated as MR-16's ceiling.

## Pass 3

P0
1. RM-7 said a radio Provider refuses an over-envelope update on receipt, MR-18 at application → RM-7 amended to "when it takes effect", citing `mr_18_a_scheduled_pair_is_checked_when_it_applies`.

P1
2. The plan's `ideal` repeat maximum was still 2^62 → u32::MAX, as MR-3.
3. `from_binding` built one of MR-27's six sections → all six from construction.
4. A dropped capture request still consumed an `<output>_<k>` → the id is given when a capture starts; `hd_14` asserts `rec_0`.
5. `mr_18_a_scheduled_pair…` did not name its profile (would pass on `ideal`) → `x310-like`, fixture pinned; mutation check added.
6. A panic inside `with_inputs` poisons slot mutexes, which `try_lock` misreported as "still held" → `try_slot` (poisoned counts as acquired) at every cleanup and Manifest site; KC-44 amended.
7. KC-30's new containment had no test → `kc_30_a_panic_in_coerce_fails_validate` with `TestProvider::panicking_in_coerce()`; mutation checks.

P2
8. RM-17 "carries neither" → "adds neither (flags already pending stay)".
9. MR-18's checked-by list; two four-cell rows in spec 09's table → fixed.
10. The abandoned-drain rules have no deterministic test → the drain returns at once when `closing` is set; Appendix B marks them "by construction".
11. Stale comment on `closing` → updated.
12. Step 7 did not mention `admit_with` → it completes `admit_with`; `admit` delegates.
13. "Two additions" with three bullets → four additions, listed.
14. `contain_all` prose inside a code block; `collect_prepare` not wrapped; reason form → moved out, wrapped, "…during <stage>".
15. Garbled antenna sentence → rewritten.
16. Appendix B's HD-14 step → "12, 13".
17. `kc_39_abort…` needed two instances; `kc_24_a_module_stop…` needed an Island → fixtures state both.
18. `mr_25`'s 999 µs needs a stop on a sample instant → the fixture stops at block 0's last sample.
