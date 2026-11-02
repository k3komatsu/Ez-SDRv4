# Brief for Sol: code review of note-45 experiment stage E2a-2, read-only

You are the independent reviewer (AGENTS.md §9 step 7) of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, format nothing, run no cargo command that writes into the tree, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.**

## Scope
E2a-2 adds S6–S9 on top of E2a-1, which you passed in `sol-e2a1-rereview3.md`. Both are uncommitted in the working tree.

E2a-2's files:
- new `crates/ezsdr-radio-uhd/src/brain/device_tests.rs` and `crates/ezsdr-radio-uhd/src/brain/conformance.rs`;
- small changes to `threads.rs`, `shared.rs`, `model_device.rs`, `thread_tests.rs` and `mod.rs`;
- rows E2-49…E2-54 in `plan/maintenance/tools/mutations.json`.

Inputs:
- the brief `~/.cache/ezsdr-fixes/2026-11-01-n45/brief-e2a2.md`;
- the plan `test-plan-e2a.md`: §7–§8, §13–§15 (S6–S9 and conformance scripts a–h), and §17, the implementer's results.

The implementer reports:
- S6, S6b, S7, S8, S8b with the clock-contention trace, S8c, and S9 a–h pass;
- FakeDevice and ModelDevice are identical on all eight scripts, with no waivers;
- the FakeDevice manual clock drives the scripts deterministically;
- script (g) did not show Q1 on one channel (the §14 derivation was wrong), while a two-channel variant (g2) does; it is pinned in `finding_q1_two_channels_evidence_precedes_application`;
- planned rows E2-23/24/25/30 were dropped, with reasons.

## Review
1. **Your own scenarios first.**
   - Where can S9's conformance pass while FakeDevice and ModelDevice differ in a way that matters to E1's verdicts? Think of scripts that compare too little, both sides sharing a helper that hides a difference, or ticks read at the wrong point.
   - Where can S6–S8 pass vacuously?
2. **Each test against the plan.** Do S6–S9 match the plan, with §15's F2/F3 corrections? Does each assertion observe what it claims? Does each row break its test?
3. **Wall-clock dependence (important).** Issue #76 was filed today for five existing tests that fail under machine load (`mech:wall-clock-test-oracle`). Design note 37's rule is: "a test's verdict never depends on when the test thread runs" (`plan/maintenance/failure-mechanisms.md:19`, `plan/phase8/37-test-thread-timing.md`).
   - S6 and S7 run FakeDevice on the real clock, with 1 s bounds and a 500-tick margin. Can a loaded host fail them?
   - Should they move to the manual clock, or can a real-clock smoke keep its verdict independent of the test thread's timing?
   - Check every new test the same way.
4. **The dropped rows.** E2-23 is covered by E2-20, E2-24 by E2-49, E2-25 is equivalent, and E2-30 cannot be built. Are these reasons sound?
5. **The Q1 two-channel variant.** Is (g2)'s sequence legal for production? Is the finding's classification right?
6. **Maintainability.** About 810 lines of tests: duplicated fixtures with E2a-1's tests, and setup that could be shared without hiding the sequences.

## Output
- Verdict: PASS or REVISE.
- Findings P1/P2/P3 with `path:line`. A P1 comes with a minimal sequence and its observable wrong outcome.
- At most 90 lines.
- End with "Not checked:".
