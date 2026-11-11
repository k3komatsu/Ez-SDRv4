# Brief — E2a-3: switches decided at receipt — (b1), the guard on any unread entry, A1, #84, #86, #88, the deletions

Repository /home/komatsu/local_work/Ez-SDRv4, branch `work/e2a`. Head: the E2a-2b commit (the orchestrator names the hash when launching). The step's start: `0ccb267`.

Read first:
- /home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/brief.md, whose rules bind;
- test-plan.md §1 (D2, D3, D9, D10, D12), §3.4, §4 and §5, plus the "Changes after …" sections;
- plan-adjudication.md and plan-dispositions.md;
- sketch.md, "Owner decisions" 1–7;
- fable-design.md, F1, F2, F5, A1;
- `git log -4`, covering E2a-1, E2a-2a and E2a-2b.

This sub-step changes behaviour, so the red-first protocol of plan §2.5 applies, in the order plan §3.4 gives: (1) `receive_switch`, delete `install`, widen the guard; (2) write R14 and run it red; (3) A1; (4) the rest. Put the failure texts in `e2a3-red.txt` in the same directory.

## Scope (plan §3.4)

- `TxBook::receive_switch`, shared by both Providers (D9), takes `(at_k, bound_k)` and returns `Accepted{boundary, requested_target}` or a `SwitchRefusal`: `NoPlayingRepeat`, `PendingSwitch` or `NotStarted`.
  - The started rule is a property of the Provider, given to `TxBook::new`: `Started::AtAcceptance` for the Mock, `Started::AtFirstIssue` for UHD (D2).
  - `issue` sets `playing.started` at the first issue of the playing booking, the switch's own payload included (D3).
  - A refused switch changes nothing.
- The switch is taken at receipt.
  - The step's guard widens from `halt_unread()` to `unread()`.
  - `dispatched(Move)` moves `playing.origin` when `playing.id` is that booking (#88).
  - A1: `cancel_switch` is the one switch-cancellation transition. It settles the row, clears `playing`, removes the payload from `held` **and** `unread`, and writes one note.
- #86: a timed switch the receipt pushed past `repeat_boundary(origin, period, at_k)` records `requested_target = at`. An untimed one, or a timed one on the rule, records none. Both Providers go through the same function.
- Delete:
  - `Bookings::install`, `refuse`, `Switch.installed`, `Switch.moved_to` and `Settled.moved_to`;
  - `SwitchOutcome::Refused` and `SwitchRecord.moved_to`;
  - `Tx::install` and its "not taken" `COMMAND_REJECTED`;
  - W15 and the dispatch-drop `cancel_switch` clause;
  - `TxList::halt_unread`;
  - the oracle's `moved_boundary`, `requested_target_of_a_moved_switch` and `outcome_after_repeat_dropped`.
  - Add the oracle's `switch_requested_target` and `switch_refused_unstarted`.
- The Mock's `HeldBurst` gains `switch` and `id` (D10).
- Refusal text: `"UR-38: the repeat the switch replaces has not started on the device"` (UHD only).

## Carried from E2a-2b

- **Row E2A2B-13** (UHD `repeat` passed to `TxBook::abandon`): its only killer, `ur_21_s4_a_finite_abandon_keeps_the_switch_of_the_repeat_it_has_not_started`, is deleted here (D7). Give it a new killer before deleting the test. The implementer's suggestion: extend `e2a_s6_…` (R4) to refuse a booking at `cursor + STOP_FRONT` inside A's kept region. Check the kill in a /tmp copy.
- **Doc comment of `Bookings::abandon`:** the m1 paragraph (a pending switch on a later repeat is also cancelled) becomes moot under (b1). Rewrite it to state why that state is now unreachable.

## Tests (plan §3.4 as amended)

- Pure tests P16–P21.
- Rig tests R14–R20, in `e2a.rs`.
  - R16 includes the positive chained case: once S1 is issued, S2 is accepted.
  - R18 has three cases.
- Mock tests M2 and M3.
- The changed tests the plan lists, including:
  - the six #86 successor-record flips (D12);
  - `mr_16_s8_record_fields` (no `moved_to`);
  - the parity test;
  - the staging tests that call `install`;
  - `iii.rs`, `o1_oracle.rs` and `n47s5.rs:289`.
- The deleted tests the plan lists, each with its evidence. Add every deletion and rename to /home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/renames.txt, one `old -> new` or `old -> -` per line.
- **Hardware rehearsal** (§4, AGENTS §9 "Hardware tests and bench values"): `n47s5_repeat_switch_rehearsal_on_the_fake` gains manifest assertions in both variants:
  - the switch row has no `moved_to` key;
  - the successor's `requested_target` is **None** (the timed trial asks now + 40 ms, which is on the rule);
  - nothing is refused.
- Report the audit table: test, expectation, and whether it is still right, fixed, or cannot be told.

## Text (plan §5)

- UR-38: rewrite it with the refusal, the receipt take and no move, and drop the unreachable clauses (F5).
- RM-37 (`design/07`), F5's list:
  - remove "and one that drops the repeat at dispatch under its late policy" and "An abandon of a finite burst leaves the switch … pending";
  - add #86's sentence and the untimed sentence;
  - do not rewrite "playing".
- UR-21, UR-23 and UR-30's row: change only sentences that become false.
- MR-42 (`moved_to`).
- EA-16's `switch` row (`design/16-easy-api.md`) and the `python/ezsdr/session.py` `switch` docstring: add the case.
- A new `KNOWN_LIMITATIONS.md` entry, **KL-41**, in the format of the others: symptom, when, workaround (retry once the repeat sounds), evidence (R17, `o1_uhd_t8`), status (Ceiling), and a note that the Mock never refuses it.
- Note 49 §7's RM-37 line belongs to the orchestrator; do not touch it.
- Keep every `*Test:*` line naming the tests that pin it.

## Trace comparison

Compare against E2a-2b's head. Plan: no differences expected. List and explain each one that appears.

## Mutation rows (plan §3.4, amended)

- Delete 21, retarget 7, re-anchor what breaks, and add 14. Every row names its killer in `filter`.
- Rows whose `filter` names a deleted test are retargeted or deleted, never left pointing at nothing.
- The rules from the earlier sub-steps hold:
  - no UHD-only or Mock-only row moved into shared code without the reason in its description;
  - no two rows identical;
  - every `new` compiles.
- Check killers in a /tmp copy.
- P650 (`core.rs` `late_by`): name a killer among the `late_by_ns` assertions and check it in /tmp. If none kills it, add an assertion to an existing `late_by_ns` test.

## Before reporting

- `cargo fmt --all`.
- In the foreground:
  - rdev workspace tests;
  - clippy, both `--workspace` and `--features uhd`;
  - `cargo test -p ezsdr-radio-uhd --tests --no-run` (hardware.rs compiles);
  - the Python suite the Python suite as the gate runs it (`plan/maintenance/tools/gate.py:~286`, `python -m unittest discover -s python/tests` with `EZSDR_SERVER` pointing at the server built from this tree, via rdev; if that is awkward, say so and leave it to the gate).
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 0ccb267 --renames …/renames.txt` = 0.

## Report

- `git diff --stat`;
- the API;
- the red-first results;
- every existing test changed, deleted or renamed, with evidence;
- the hardware audit table;
- the trace differences;
- the rows, by id and action;
- the `design/`, KNOWN_LIMITATIONS and Python text changed;
- every behaviour change, harmless ones included;
- anything in the plan that proved wrong.

Do not commit, push or `git add`. Do not edit AGENTS.md, handoff.md, plan/phase8/49-*.md, bench-results.md or bench/.
