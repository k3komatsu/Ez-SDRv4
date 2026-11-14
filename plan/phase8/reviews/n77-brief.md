# Brief — #77: MockRadio records pending faults after a loss in instant order

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n77` (from main d0a0df2). You edit the tree; you do **not** commit, stash, push or switch branches. Never touch `/home/komatsu/local_work/Ez-SDRv4-e3` (another lane). Not a risky step (a record order in MockRadio and the test generator): no test plan, no self-review; a fresh Opus reviewer reads the diff after you, Sol reviews the code.

## Decided (orchestrator, from the reproduction; issue `gh issue view 77`)
- Reproduced: faults booked `Overrun @3 ms, Sequence @2 ms, Loss @1 ms`. The oracle expects the pending rows after the loss in instant order `[device_lost, rx_sequence_error, rx_overflow]`; MockRadio writes booking order `[device_lost, rx_overflow, rx_sequence_error]` (both profiles). MockRadio's order comes from `record_pending_faults` (`crates/ezsdr-mock-radio/src/lib.rs:778-785`, called from `stop_at` :1203 after the loss at :2015-2022 and from `cleanup` :2427) looping over `self.faults` in document order. The oracle sorts by `(e, seq)` (`crates/ezsdr-mock-radio/tests/timeline.rs:128`, `crates/ezsdr-radio/tests/generator/mod.rs:175`). The spec is silent (MR-20, MR-27 in design/09-mock-radio.md; SE-4 in design/08 applies faults by instant, document order breaking ties).
- Fix: MockRadio records not-yet-fired faults in instant order, document order breaking ties (the same order the faults fire in, SE-4) — at the loss and at cleanup. One sentence in MR-20 (or MR-27, whichever states the pending rows) and its `*Test:*` line; a Changes row.
- The generator's Fault class may include a loss (generator/mod.rs ~:300-310: push the loss for `class >= Class::Fault` or an equivalent that keeps the Loss class's meaning), so `rm_26_the_timeline_against_mockradio` covers it; check the class distribution comment and any test that counts classes. Also add a hand-built unit/differential case of the minimal sequence above (both profiles), and one with two pending faults of the same kind whose `at` values must come out in instant order (the differential compares only `(name, applied, lost)`, so assert `at` in a MockRadio test directly).
- A mutation row for the sort (Rust token anchor, smallest unique text) killed by the new tests; check that existing rows on `record_pending_faults` still match and kill.

## Exit checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base d0a0df2` (foreground), locally.
- Rows: the new row and every existing row whose filter selects a test you changed or whose anchor moved, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>`; log to `~/.cache/ezsdr-fixes/2026-11-14-n77/rows.log`.

## Rules
- Mutation rows only through `remote_mutate.py`; never `mutate.py` locally; never apply a mutant to the tree.
- Wait in the foreground for long commands. Kill by PID. Temp files on /tmp. No build output in the tree.
- `~/.cache/ezsdr-fixes/` (except rows.log), `handoff.md`, `AGENTS.md`, plan notes: read only; design/09 (and design/08 only if needed) you may edit.
- No wall-clock tests; no mechanisms beyond the above. After `cargo fmt`, `rules.py` must still find every row exactly once.

## Report (short)
`git diff --stat`, checks' counts, rows' outcomes, deviations, INFERRED.
