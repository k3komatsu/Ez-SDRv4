# Sol code review: note 47 stage 1 (initial channel set and values at `connect`)

You are the independent reviewer (read-only). Repository: `~/local_work/Ez-SDRv4`, branch `main`, HEAD `b3d39ab`; the change is the **uncommitted working tree** (`git diff`, plus the untracked `crates/ezsdr-server/src/initial.rs`). Read `AGENTS.md` §9 (the review rules, maintainability, counterexamples). Do not edit, format, build into the tree, commit or run mutation tools. If you build, use `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` from a copy outside the tree (AGENTS.md §7); reading is usually enough.

## What the step is

- Design: `plan/phase8/47-no-change-while-streaming.md` §2.4 (connect contract, O1 revised: no channel joins a stream by default in either direction, O2, O3, the connect signature (B), naming, the stage-1 rulings Q1–Q4) and §4 stage 1.
- The implementer's brief: `~/.cache/ezsdr-fixes/2026-11-04-n46/brief-s1.md`. **Note:** the brief's item 3 ("transmit c0 joins stream 0 by default") was **overruled** by the owner after it was written (O1 revised, spec 07 §6 R2): the default is now *every channel of both directions in no stream*.
- The final test plan: `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s1.md` (rev 3). Your earlier plan review and its dispositions: `~/.cache/ezsdr-fixes/2026-11-04-n46/sol-s1-plan-final.md`, `dispositions-s1-plan.md`.
- Out of scope (later stages): refusing changes to a flowing stream, staging, start_rx with values, TX settings, overlap, the repeat switch, channel names (stage 1b), removing the runtime `.stream` setter or the move machinery (stage 2).

## What to do

First write your own failure scenarios from the step's purpose, the code and the specs, then compare with the plan and the tests. Then review:

1. **Correctness** of the new path end to end: Python `connect(rx=, tx=, radio=)` → wire `initial` → server `initial::expand` (after assemble, before `coordinator::connect`) → Kernel `implicit_spec` (`requires` as `Eq` before hash/validate, Vocabulary majors) → Provider validate/coerce/prepare → first samples/bursts. Look for: wrong stream/channel numbering when names and numbers mix; a refusal that happens after a Run (manifest, Session dir) exists; a value silently dropped or defaulted; two radios mixed up; a hash that does not change with the requests or changes without them; a refusal at the wrong stage (RM-31, RM-32, RF envelope, `tx_enabled`).
2. **Behaviour that must not change**: Spec Runs (`start_spec`), the runtime `.stream` setter (still present until stage 2), MockRadio/UHD parity, existing refusals.
3. **The new default (no stream anywhere)**: every caller, harness, example, fixture and bench driver that relied on receive `c0` in stream 0 by default; anything that now silently does nothing instead of failing loudly.
4. **The tests' power**: each planned sequence has its test; the test observes what the plan says; the mutation rows P1319–P1352 (appended to `plan/maintenance/tools/mutations.json`) and re-anchored P49/P137 break what they claim; the removal of P75 and of the `rm_32_throughput_cases` case is justified.
5. **Specs**: every `design/` text the change makes false (RS-12, SB-22c, SB-T2, KC-1, RM-30, §6 R2, MR-5, EA-4/9/10/10a/16/19, Vision §3/§57/§66) — missing amendments and wrong ones; accepted-spec format (AGENTS.md §2).
6. **Maintainability** (AGENTS.md §9): duplicated decisions (e.g. stream-name rules in Python and Rust, channel/stream counts derived from capabilities), leftovers, test fixture reuse, mutation-row anchors.
7. The implementer flags an **open-time zero flush**: a TX streamer open sends a two-zero burst. Say whether that is pre-existing, and whether stage 1 changes when it happens.

## Report

Write to `~/.cache/ezsdr-fixes/2026-11-04-n46/sol-s1-code-final.md` if you can write there; otherwise put it all in your final message. Verdict (PASS / NEEDS CHANGES), then findings ranked P1/P2/P3 (major/minor/note), each with `path:line`, a minimal operation sequence and the observable wrong outcome (AGENTS.md §9 counterexamples), and a suggested fix. Mark UHD-dependent claims VERIFIED/INFERRED with sources. At most 120 lines.
