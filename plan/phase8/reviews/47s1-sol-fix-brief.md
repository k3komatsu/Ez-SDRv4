# Sol re-review: note 47 stage 1, fix round 1

Read-only, same rules as your previous review (`~/.cache/ezsdr-fixes/2026-11-04-n46/brief-sol-s1-code.md`). Your previous findings: `sol-s1-code-final.md`; the orchestrator's dispositions: `dispositions-s1-code.md`. The working tree (uncommitted, HEAD `b3d39ab`) now holds the fixes; the implementer's plan is `test-plan-s1.md` rev 4 (§6 lists the fix cases).

Review:
1. S1 (the major): `ModuleRegistry::vocabulary_of` / `key_decl` (`crates/ezsdr-kernel/src/module_api/registry.rs`) and its use in `crates/ezsdr-kernel/src/session.rs` implicit_spec. Is ownership now decided once and correctly for nested and shared prefixes? Do every other caller of `key_decl` (search them) keep their behaviour? Does the new test `rs_12_an_initial_key_declares_the_vocabulary_that_declares_it` observe it, and do P1327 and the re-anchored P22 break it?
2. The sequences next to it: an undeclared key under a registered prefix (must still be `unknown key prefix` / refused at validate), a key declared by no registered prefix, Spec Runs' `requires` validation through `key_decl`.
3. S2–S4 briefly: `python/ezsdr/session.py` connect `radio` handling; `crates/ezsdr-server/src/initial.rs` `member` canonical keys; the stream-name rule in both languages and EA-10a/EA-16 text agree; rows P1344, P1348, P1353–P1359.

Report in your final message: verdict (PASS / NEEDS CHANGES), findings P1/P2/P3 each with path:line, a minimal sequence and the wrong outcome. At most 50 lines.
