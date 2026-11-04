# Sol test-plan review: note 47 stage 1b (channel names in the radio selector)

Read-only. Repository `~/local_work/Ez-SDRv4`, HEAD `b07e032` (stage 1 committed; records `plan/phase8/reviews/47s1-*`). Read `AGENTS.md` §9.

- Design: `plan/phase8/47-no-change-while-streaming.md` §2.4 bullet **名前** and §4 stage 1b.
- Brief: `~/.cache/ezsdr-fixes/2026-11-04-n46/brief-s1b.md`.
- The implementer's test plan: `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s1b.md`.
- The orchestrator's rulings on its questions: `~/.cache/ezsdr-fixes/2026-11-04-n46/rulings-s1b-plan.md`.

First write your own failure scenarios from the purpose, the current code (selector parsing `crates/ezsdr-radio/src/channels.rs`, MockRadio, the UHD Module's layout/device record, the server's `catalogue.rs` and `initial.rs`, Python `session.py`) and the specs (RM-28, EA-10a, EA-16); only then compare with the brief's sequences and the plan. Check in particular: the plan's list of every selector reader is complete (run your own searches and give them); a named and an unnamed selector cannot behave differently anywhere (Provider identity, device record, parity, coerce, effective, capability keys); the wire rule for name-only streams has no ambiguous case; refusals happen before any Run; each planned mutation row (P1361–P1385, retargeted P1334/P1335/P1344/P1345) would be killed by its named test; the rulings are sound.

Report in your final message: verdict (PLAN OK / PLAN NEEDS CHANGES), findings ranked major/minor/note with path:line, a minimal sequence and the wrong outcome, and the fix to the plan. At most 80 lines.
