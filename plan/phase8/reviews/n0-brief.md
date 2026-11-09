# Brief — N0: the switch mark only on SendAsapAndFlag (X1), and RM-15's footprint sentence (§11.2)

Repository: /home/komatsu/local_work/Ez-SDRv4, branch `main`, base commit `25db29b`. Work in the main tree, uncommitted. Read first: AGENTS.md "Safety core", §2 (format of accepted specs), §3, §7, §9 (you are the implementer); design note `plan/phase8/49-time-and-booking-inventory.md` §6 principle 4, §7 (rows RM-12/13, RM-15, KL-36), §11.2, §11.3, §12 row N0 (read only; it is the orchestrator's file).

**Risk: not risky** (AGENTS §9 step 1). No test plan; implement directly. **Self-review is waived** (owner: Sonnet-implemented steps). The reviews that remain: the orchestrator's review, the full mutation run, and Sol's independent code review.

N0 is the first stage of direction D′. The behaviour of every reachable Session path must not change. The KL-36 title part of N0 is already done (step B, `e81176b`); do not touch KL-36.

## Part 1 — X1: the switch mark is accepted only on a repeat whose late policy is SendAsapAndFlag (owner, 2026-11-08, note 49 §11.3)

Today a repeat `TxBurst` with the switch mark `radio.repeat_operation = "switch"` is accepted under any late policy. Under DropAndFlag, UHD refuses the switch at take time when the handed cursor has passed its boundary (UR-38, `ur_21_s12_drop_and_flag_switch_handed_past_is_refused`). Session verbs never produce this: `start_repeat` compiles to SendAsapAndFlag (RM-12, `crates/ezsdr-kernel/src/session.rs:595`); a Spec action can (`crates/ezsdr-kernel/src/event.rs:926` carries any late policy).

The decision: the mark on a burst that is not a repeat **or whose late policy is not SendAsapAndFlag** is refused at receipt by RM-13's metadata check, on both Providers, with the reason each uses today for a misplaced mark (`MR-16: target, clock, waveform size, channel count or metadata is invalid` on MockRadio; the UHD Module's metadata refusal reason in `control.rs`). Then delete the take-time DropAndFlag switch branch, which becomes unreachable.

Expected results (both Providers unless marked):
1. A repeat with the mark under DropAndFlag (and under any other non-SendAsapAndFlag policy the type has) → refused at receipt with `COMMAND_REJECTED`, no `TIME_ERROR`, no `switches` row, the playing repeat plays on. Whether it is timed or untimed does not matter.
2. A repeat with the mark under SendAsapAndFlag → unchanged (every existing switch test still passes).
3. The mark on a non-repeat → refused as today.
4. UHD: the owner's take-time refusal of a DropAndFlag switch (`tx.rs`, the doc at about line 349 "False when the switch is refused (DropAndFlag, or the cursor is past it)", and its branch) is gone; what remains is only the SendAsapAndFlag move (UR-38). Search every place that tests the switch's late policy (`grep -n "DropAndFlag\|drop_and_flag" crates/ezsdr-radio-uhd/src crates/ezsdr-mock-radio/src crates/ezsdr-radio/src crates/ezsdr-acceptance/tests`) and remove the switch-specific DropAndFlag handling in production code; DropAndFlag for ordinary bursts stays.

Places: `crates/ezsdr-radio-uhd/src/provider/control.rs` (about line 408, `let switch = repeat && is_switch(metadata)`), `crates/ezsdr-mock-radio/src/lib.rs` (about line 1580, `let switch = repeat && is_switch(&metadata); let metadata_ok = …`), `crates/ezsdr-radio-uhd/src/provider/tx.rs` (the take-time refusal). Put the policy test in one place if the two Providers would otherwise each write it: `ezsdr_radio::staging::is_switch` is the shared helper (`crates/ezsdr-radio/src/staging.rs:672`); decide whether the late-policy condition belongs in it (a shared "is an accepted switch mark" decision) or at each caller, and say which and why in the report (AGENTS §9 Maintainability 1).

Tests:
- `ur_21_s12_drop_and_flag_switch_handed_past_is_refused` (`tx.rs` about line 6836) tests a now-unreachable path: replace it with a receipt-refusal test (sequence 1, UHD) and remove the old one. The helper `book_switch_under(rig, id, k, wave, policy)` (about line 6722) and its DropAndFlag caller (about line 6719), and the test near line 7184 that books a non-repeat with `DropAndFlag` and `switch_mark()`: keep what still tests a reachable sequence, simplify the helper if its `policy` parameter has one value left.
- Add sequence 1 to the existing metadata refusal tests (`ur_21_metadata_is_empty_or_the_mark_only`, `mr_16_metadata_is_empty_or_the_mark_only`) or a parity test in `crates/ezsdr-acceptance/tests/parity.rs` if both Providers' answers can be compared there with the existing fixtures — prefer one parity case over two copies.
- Grep `plan/maintenance/tools/mutations.json` for rows whose `filter` names a removed or renamed test and rows whose `old`/`old2`/`new` anchor removed code; retarget each to the new test or code if its behaviour remains reachable, or remove it with the reason in the report. Add a row that breaks the new receipt check (e.g. drops the late-policy condition) and names its killing test(s) in `filter`, for each Provider's check unless it is one shared helper.

Spec text (same change):
- `design/07-radio-model.md` RM-13's refusal list item "`metadata` is neither empty nor the switch mark (RM-37), or the mark is on a burst that does not repeat" → also "or whose late policy is not `send_asap_and_flag`". Check RM-12 and RM-37 for a sentence that still allows a DropAndFlag switch.
- `design/18-uhd-radio.md` UR-38 (about line 299): remove "Under DropAndFlag it is refused with its id, as `COMMAND_REJECTED`, and the old repeat plays on." and any other DropAndFlag-switch wording; the *Test:* lines (UR-38 about line 301, and line 295) drop the removed test and name the new one.
- `design/09-mock-radio.md` MR-16 / MR-42 if they state the switch's late policy.
- Each spec's closing *Changes* table: one row for N0 (date 2026-11-09, the rules touched, one line, "[note 49](../plan/phase8/49-time-and-booking-inventory.md) §11.3, N0").

## Part 2 — RM-15: the footprint sentence (owner, 2026-11-08, note 49 §11.2 (a), §6 principle 4)

No code change. RM-15 (`design/07-radio-model.md:161-167`) already says, in its third paragraph: "A booking that cannot start at its requested sample because of device state follows its late policy; one that meets another accepted booking is refused; an untimed one is placed at the first free sample." The decision makes explicit what "meets another accepted booking" and "device state" cover. Replace that sentence (do not add a second statement of the same topic; AGENTS §2) with one that says:

- An accepted booking's region is its **footprint**: its samples and its own device close, up to its front (the fronts RM-15's second paragraph states). A booking that meets it is refused at receipt whatever its late policy (KL-36). A finite booking that was abandoned (UR-22) keeps its region, so this still holds.
- What no accepted booking holds is device state, and a booking it delays follows its late policy: a `Stop` region, including the one a repeat abandoned at its cursor leaves (RM-16: a timed SendAsapAndFlag booking moves past it with `TIME_ERROR { send_asap }`, a timed DropAndFlag booking is refused, an untimed one is placed past it), and host lateness or a recovery at dispatch (RM-14's second paragraph, UR-22 F8).
- An untimed booking is placed at the first free sample (as today).

**Each clause must describe today's code.** Before writing it, verify each clause against both Providers' code and an existing test, and cite the test in RM-15's *Test:* line (e.g. `ur_21_s16_timed_drop_in_the_stop_region_is_refused`, `ur_21_s16_timed_send_asap_in_the_stop_region_moves_with_time_error`, the abandon tests that show a finite booking's region stays and a repeat's abandon books a Stop at its cursor, `rm_15_*` in `crates/ezsdr-radio/tests/staging.rs`). The clauses above are the orchestrator's reading of note 49 and the code, not verified line by line. **If a clause is false of today's code, do not change code to make it true: write the sentence to match the code, and list the mismatch at the top of the report** (it goes to the owner). If a clause has no test, write `none named for …` in the *Test:* line. Keep RM-15 in DA-1 form: normative text only, no rationale, no history, no issue numbers in the rule (they go in the *Changes* row). `rules.py` checks line lengths and markers.

## Rules (binding)

- Do not commit, push or `git add`. Do not run `mutate.py` or apply a mutant to the tree; mutation rows run only through `remote_mutate.py`, which the orchestrator runs. Do not run `gate.py`.
- Builds and tests: `python3 plan/maintenance/tools/remote/rdev.py -- cargo …` (sim02) in the foreground until it ends (use a long timeout, up to 10 minutes per call; split long runs by package). Run at least `cargo test -p ezsdr-radio -p ezsdr-mock-radio -p ezsdr-radio-uhd -p ezsdr-acceptance` and `cargo clippy --workspace --all-targets -- -D warnings`. Local cargo is allowed but prefer rdev. Test temp files under /tmp. `PYTHONDONTWRITEBYTECODE=1` for every Python command.
- Before reporting: `cargo fmt --all`, then `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 25db29b` = 0 (it also checks that every mutation row's anchors match exactly once).
- Do not edit `plan/phase8/49-*.md`, `handoff.md`, `AGENTS.md` or `KNOWN_LIMITATIONS.md`.
- Do not delete a test of a reachable sequence without a replacement. Do not add tests that depend on wall-clock timing. Do not add mechanisms the brief does not ask for (no new late-policy branches, no new types).
- Kill processes only by PID.

## Report

Write `/home/komatsu/.cache/ezsdr-fixes/2026-11-09-n0/report.md`: (1) any RM-15 clause that is false of today's code (first); (2) files changed and why; (3) where the X1 check lives and why; (4) tests removed, added, changed, each with the sequence it covers; (5) mutation rows removed, retargeted, added (ids and killing tests); (6) commands run with their pass/fail counts; (7) anything left open. End your final message with the report's path and a one-paragraph summary.
