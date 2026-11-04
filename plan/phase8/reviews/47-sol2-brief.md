You are read-only (edit nothing, never run cargo fmt, no git state changes, no builds) in ~/local_work/Ez-SDRv4 at main 3dd281d.

Re-review plan/phase8/47-no-change-while-streaming.md (Japanese), second draft. You reviewed its first draft (your answer: plan/phase8/reviews/47-draft-sol.md, 12 findings). The owner's decisions are settled (plan/phase8/46-streaming-feature-ledger.md §8 and note 47 §6 O1-O3): do not argue for reversing them.

Check:
1. Each of your 12 findings: is it closed by the second draft? Say CLOSED / PARTLY / OPEN with the note's section.
2. The new owner decisions O1-O3 (§6): is each consistent with the rest of the note and with the Kernel (design/04-run-and-session.md, design/06-kernel-coordinator.md, crates/ezsdr-kernel)? In particular for O1: how a Session's implicit Spec can carry an initial configuration request today (crates/ezsdr-kernel/src/session.rs, design/16-easy-api.md EA-9/EA-10, design/03-spec-and-binding.md), what must change, and whether moving tx c0 into stream 0 by default breaks RM-31 with the default antennas (design/07-radio-model.md RM-30/RM-31, board defaults). For O2: whether a Session submit returns only after the Provider has handled the Action, in both a device-paced Run (KC-21a, MA-14b) and the default simulation-class Session with MockRadio (find the code path that steps the Mock after a Session submit), with path:line.
3. New problems the second draft introduces (minimal operation sequence and the wrong observable outcome).
4. Whether stage 1 of §4 (the initial channel-set path) is well-defined enough to brief an implementer: list the exact spec rules, code regions, tests and examples it touches, and the operation sequences its test plan must cover.

Output: the 12-finding status table, findings (section, problem, evidence path:line, correction), the stage-1 scope list. End with one line: NOTE OK, or NOTE NEEDS CHANGES with the number of findings.
