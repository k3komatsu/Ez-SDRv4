You are the independent reviewer (AGENTS.md §9 step 8: re-review after major fixes), read-only: do not edit, format, build, test, run mutate.py or remote_mutate.py, or run any git command that changes state. Repository: ~/local_work/Ez-SDRv4. Never read ~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/.

Your previous review of stage E1 (note-45 experiment, crates/ezsdr-radio-uhd/src/brain/, uncommitted) is at ~/.cache/ezsdr-fixes/2026-11-01-n45/sol-e1-code.md (REVISE: P1 findings 1–6, P2 findings 7–8, notes). The orchestrator adopted all of them with the decisions in ~/.cache/ezsdr-fixes/2026-11-01-n45/dispositions-sol-e1.md; the plan's §10 draft 9 (~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md) records them. The implementer's fix report is summarized here:
- 1: cell `flushed` fact (set by a flush attempt, cleared by mark_left); retire_unread removed; Restore is Never when left remains after the flush was attempted.
- 2: Close/Open only from Off or Uncertain, not committed, not busy; Lost Never; I3 also checked at a Close/Open's execution.
- 3: Exit::Dead at MarkLost; BStep disabled; brain timers cleared; Terminate routed to S and every R; P1 makes no claim for a dead brain.
- 4: Streamer::promote after the tail; promotion past the start instant is late and recv returns LateCommand, which R reports to the brain. DEVIATION for your judgment: a start called directly with an instant in the past still begins at the next sample (not late); only promotion behind a tail follows FakeDevice's at<now late rule. The implementer says making direct starts late would make most scripted starts late, because the brain may lag device time without bound.
- 5: applied Ident's channel from the DSP chain key; consume uses streamer channels; I4 compares facts with the reservation; a Move reaches the device only through the next Open.
- 6: call_faults in every variant of case 7b; walk sweeps fail Vacuous on an unreached declared witness.
- 7: key keeps shortened, the witness names read later (READ_LATER), flushed, silent_from.
- 8: timers and recv timeouts fire at their real time; after the epilogue end a silent Tick is enabled while a deadline lies ahead.
- n1: hand-written DFS loops replaced by dst::explore through a Probe.
- Known gap reported: uniform random walks rarely reach witnesses that need late environment inputs (case 1 W1a, case 7b W7b3, case 8 W8a–d unreached at 10,000 seeds).

Task: for each of your findings 1–8, verify the fix in the code (path:line) closes the counterexample you gave, and look at the code next to each fix for new defects it introduces (same categories as before: model vs production/FakeDevice fidelity, unsound key merges, weakened or false-positive oracles, tests or rows that do not observe what they claim, maintainability). Judge the deviation in 4. Give your view on the walk-coverage gap (is a seeded swarm-style scheduler — per-seed random weights for Tick versus actor events — a sound way to reach late-input witnesses without narrowing windows?).

Output: verdict PASS or REVISE; per finding 1–8: CLOSED / NOT CLOSED with evidence; new findings ranked P1/P2/note, each with path:line and, for a P1, a minimal operation sequence and the observable wrong outcome. End with what you did not check.
