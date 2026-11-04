<!-- brief-s3.md -->
# Brief: note 47 stage 3 — receive values change again, on a stopped stream (TEST PLAN FIRST)

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `d8f23af` (stage 2 committed: every radio key classless, the Run-time change machinery deleted). Read `AGENTS.md` (§9: this is a **risky** step — you write the test plan first, Sol reviews it, then you implement), then:
- note 47 (`plan/phase8/47-no-change-while-streaming.md`) in full: §1, §2.1 items 2 and 4, §2.2, §2.3, §2.4 (「開始と設定」, 「送る順と部分的な失敗」, 「読み返す値は admitted の値」, 「Provider の拒否の扱い」), §2.5 (§58 #16), §3 (Q4, Q6), §4 stage 3 and stage 2's 済 paragraph with **「段階 3・4 に残したこと」**;
- the stage 2 records `plan/phase8/reviews/47s2-review.md`, `47s2-test-plan.md` (what was deleted and why; what stage 3 inherits), `47s2-orch-review.md`.

**This task is the test plan only.** Write it to `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s3.md` and stop. Do not change the tree.

## The step

1. **Classes**: `radio.rx.c<n>.frequency_hz`, `gain_db`, `antenna` get class `cold` again. `radio.rx.s<k>.sample_rate_hz` stays classless: a receive rate change is refused until the bench confirms it on a held-open streamer (note 47 §1, the acceptance condition). Transmit keys stay classless (stage 4). `stream` stays classless.
2. **"Not streaming" and staging, both Providers** (RM-21 rewritten; MockRadio MR-18, UHD UR-26): a `cold` receive value is accepted only when its stream is **not streaming** — after a `Stop` and before a `start_rx` for that stream is booked. Otherwise the Provider emits `radio.COMMAND_REJECTED` with a reason naming the rule and the action id, and does not use the value (the Kernel's admitted configuration keeps it, KC-27). An accepted value goes to the stream's configuration of record (UHD: uhd-control's `config`, which `start_rx` snapshots at booking, `control.rs` ~293 `ColdConfig::new(self.config.clone())`; MockRadio: its equivalent — find it). Later values for the same key overwrite earlier ones in arrival order. `Stop`, loss and `Provider::stop` do not clear it. A channel's value is accepted only if the channel's own stream is not streaming; another stream streaming is irrelevant (peer isolation).
3. **The configure time counts** (the `ready` requirement, note 47 §4 「Sol の確認で足したこと」): a `start_rx` whose configure (the device's apply) takes longer than expected must not silently eat the start lead: the new segment's origin is the first lattice instant at or after both the booked instant and the end of its configuration plus the start lead, or the start is reported late — decide which from today's start-lead rule (RM-25, `crates/ezsdr-radio/src/timeline.rs`) and the deleted scenario `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration` (`git show 45a4294:crates/ezsdr-radio-uhd/tests/fake.rs`, ~2783), and say it in the plan. MockRadio must model the same (its configure takes no time? then say how parity stays exact).
4. **O2, Provider refusals reach the submit reply** (note 47 §6 O2, §2.4 「送る順と部分的な失敗」): the server gathers `radio.COMMAND_REJECTED` events carrying the submitted Action's id that exist when the submit returns, and returns them in the reply; Python raises `Rejected` for them. Find where the submit reply is built (`crates/ezsdr-server/src/`), the reply schema, and `python/ezsdr/session.py` ~110. Note 47 §2.1 says the booking decision is made before submit returns (VERIFIED in note 47 第 3 版 for both the device-time Run and MockRadio) — re-check that claim in today's code and cite it.
5. **Easy API** (spec 16, EA-16/EA-17): `rx_streams[k].start(at=None, *, frequency=None, gain=None, antenna=None)` (dicts keyed by logical channel; `sample_rate=` raises `Error` until the bench) sends one `UpdateParameter` per value, then `start_rx`; on the first `Rejected` it raises and sends nothing more. The channel attribute setters (`rx_channels[n].frequency = …`) send the update and raise `Rejected` when the Provider refuses it (streaming). Client-side checks before sending (channel belongs to the stream, types). TX setters keep raising `Error` (stage 4).
6. **§58 #16** returns to its envelope form (note 47 §2.5): on a stopped stream, a frequency outside the RF envelope is refused at admission, logged, not dispatched; a separate test: an allowed value sent to a streaming stream is refused by the Provider and matched by action id. Update `v58.rs`, `uhd.rs` and KL-35.
7. **VH differential**: the generator (`crates/ezsdr-radio/tests/generator/mod.rs`) gains a receive cold-change operation (on stopped and on streaming streams, so both accept and refuse are generated); the oracle, MockRadio and the uhd-control differential agree. No class may generate empty sequences.
8. **Inherited from stage 2** (note 47 §4 「段階 3・4 に残したこと」): fix the label `"update_parameter"` on the `start_rx` refusal in `crates/ezsdr-radio-uhd/src/provider/rx.rs` (and decide the clock-end failure and RM-25 book-error labels in `core.rs`); the MockRadio phase redraw at each start's configure (Q4: decide and test how many draws a same-value configure, a failed configure and a cancelled start make); restore the receive start-lead measurement group for the bench if the restart-with-configure path makes it meaningful again (compile-only; say what it measures); the UHD owner's configure-once comparison (`Owner::Nothing`, the `configured` mark) now matters again — test it through a sequence.
9. Specs 07 (RM-21, RM-27), 09 (MR-18, MR-33–35), 16 (EA-16, EA-17), 18 (UR-26 and the start path), KNOWN_LIMITATIONS (KL-35, and KL-3/KL-32 on retune phase), with Changes rows.

## Sequences that matter (each needs a test; expected result)

- S1 connect rx0 c0 at f1, streaming → `Stop` → set c0 f2 → `start_rx`: the device is tuned to f2 before the first sample of the new segment; the old segment's samples carry f1 and the new segment's f2 (MockRadio value history and channel model; UHD: the `apply` call precedes the new stream command).
- S2 streaming → set c0 f2: `Rejected` (Python), one `COMMAND_REJECTED` with the action id, no device call; after `Stop` and `start_rx` the stream still uses f1 (admitted shows f2: document it).
- S3 `Stop` → `start_rx` booked at a future `at` → set f2 before `at`: refused (start booked); the start uses f1.
- S4 `Stop` → set f2 → set f3 → `start_rx`: f3. `Stop` → set f2 → `Stop` again → `start_rx`: f2.
- S5 two streams: `Stop` stream 0 → set its c0 (accepted) and stream 1's c2 (streaming: refused); stream 1 is undisturbed (no gap, no device call for it).
- S6 the device refuses the configure at `start_rx` (read-back or apply failure): the refusal halt as today; then `Stop`/set a corrected value/`start_rx` succeeds ("fail → fix → resend").
- S7 slow configure (the `ready` case): the origin respects the configure's end plus the lead, or the start is reported late — exactly what the plan decides.
- S8 out-of-envelope frequency on a stopped stream: refused at admission, logged, not dispatched (§58 #16); `sample_rate_hz`: refused (classless) with the reason naming the bench condition.
- S9 Python `rx.start(at, frequency={0: f2, 1: f3})` where the second value is refused: `Rejected`, `start_rx` not sent, the first value stays staged (document).
- S10 MockRadio `random_unless_timed_tune` channel: each start's configure redraws its phase (Q4 counts); a `deterministic` channel keeps phase 0.
- S11 the VH differential with the new operation agrees across oracle, MockRadio and uhd-control.

Hardware/UHD behaviour these rely on — mark each VERIFIED (with a path:line or URL) or INFERRED in the plan: the receive configure (tune, gain, antenna) with the streamer stopped and its completion before the next stream command (UR-13, UR-36 paths); UBX/OBX phase after an untimed tune (note 47 Q4, VERIFIED there); the DDC settings being taken only with samples (design note 39 §1).

## The test plan

Per AGENTS §9 step 2: for each sequence S1–S11 and each item of the step that adds a state or failure path: the test (file, name), what it asserts, how the assertion observes the behaviour, and the wrong behaviour that fails it; the mutation rows you will add (Rust token anchors, exact substrings elsewhere) with their killing tests; the tests and rows of stage 2 that change; the spec rules each test pins. Plus your own failure scenarios beyond S1–S11. Keep it to what the step needs; no new mechanisms beyond the step.

## Report (at most 15 lines)

Where the plan is; open questions you need the orchestrator to decide (each with your recommendation); the claims you could not verify.

<!-- brief-s3-plan2.md -->
# Brief: note 47 stage 3 — test plan revision 2

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, HEAD `d8f23af`, clean tree. Read `AGENTS.md` §9 and, in `~/.cache/ezsdr-fixes/2026-11-04-n46/`: the brief `brief-s3.md`, the plan `test-plan-s3.md` (rev 1, by a previous implementer), the orchestrator's `rulings-s3-plan.md`, Sol's review `sol-s3-plan-final.md` and the dispositions `dispositions-s3-plan.md`.

Task: revise the plan to **rev 2** (keep rev 1 as `test-plan-s3.v1.md`) implementing every adopted row of the dispositions (M1–M3, m1–m4); note D4's new form (report, origin unchanged) in §0; mark what changed per row at the top. Read the code each change touches and cite path:line. Do not change the tree. Report (at most 10 lines): per disposition row, where it landed; anything you could not do.

<!-- brief-s3-impl.md -->
# Brief: note 47 stage 3 — implementation (receive values change again, on a stopped stream)

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `d8f23af`, clean tree. Read `AGENTS.md` (§7, §9 including maintainability), note 47 (`plan/phase8/47-no-change-while-streaming.md`), and in `~/.cache/ezsdr-fixes/2026-11-04-n46/`: `brief-s3.md` (the step), `test-plan-s3.md` rev 2, the decisions in order — `rulings-s3-plan.md`, `sol-s3-plan-final.md` + `dispositions-s3-plan.md`, `rulings-s3-plan2.md`, `opus-s3-plan2-recheck.md` + `dispositions-s3-plan2.md`.

## Task

1. **First revise the plan to rev 3** (keep rev 2 as `test-plan-s3.v2.md`) with every adopted row of `dispositions-s3-plan2.md`: ruling 7 as corrected (threshold = `min_timed_command_lead_ns`, origin unchanged, the seven plan changes), D2 as "streaming = the line's final state after replay", the new D2 test (`Stop` → `start_rx(at = now+1 s)` → `Stop` → set f2: refused), `at` on generated `StartRx`, the hold-after-take knob for `ThreadedProvider`.
2. **Then implement rev 3** in the tree: classes, the shared pure `Staging` decision (D1), both Providers, `START_LEAD_SHORT` (schema regenerated, `schema_freeze` green), O2 (`Submitted.refused`, the Ended rule, Python `Rejected`), the Easy API (`rx_streams[k].start(at, frequency=, gain=, antenna=)`, setters, `sample_rate=` raising `Error`), §58 #16's receive-antenna form, the generator's `Value` class with its independent oracle, the inherited items (labels, Q4 draws, the receive start-lead bench group, configure-once), specs 07/09/16/18 and KNOWN_LIMITATIONS with Changes rows, and the mutation rows with their killing tests. Extend the plan for every state or failure path the implementation adds (AGENTS §9 step 4).

## Rules

- Mutation rows: text edits only in `plan/maintenance/tools/mutations.json` (never re-serialize), each matches exactly once, each names its killing test; confirm each new row by applying it in a throwaway copy outside the repository (`rsync -a --no-times --exclude .cargo --exclude target`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, touch the copy before building, one copy at a time) and running its test; a compile failure counts as not killed; logs under `work/s3-probes/`. Never run `mutate.py`, `remote_mutate.py` or `gate.py`; never apply a mutant to the tree. Sonnet steps waive the DeepSeek self-review (owner): your report is reviewed by Sol.
- Stage 2's rows touched by your change: retarget or re-anchor; never delete a row of surviving behaviour without a killing replacement.
- No git state changes. `cargo fmt --all`; clippy `-D warnings` (workspace, and `ezsdr-radio-uhd --features uhd --all-targets` via `plan/maintenance/tools/remote/rdev.py`); workspace tests; Python 3.14 suite and the 3.9 container (handoff §5); new tests whose verdict can depend on timing run 20 times; `git ls-files -m -o --exclude-standard | xargs chmod go+r`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` → 0 (use `--renames <file>` under `plan/phase8/reviews/47s3-renames.txt` only for tests you rename or drop with a reason). Test temp files on `/tmp`. Leave no test process running.
- No version bumps, no `SCHEMA_CHANGELOG` entries.
- If your context gets tight, stop at a green point, write what is done and left into the plan, and report.

## Report (at most 20 lines)

What is implemented per plan item; tests added/changed; rows added/retargeted with probe outcomes; suites; `rules.py`; deviations from the plan and why; open questions.

<!-- brief-s3-fix.md -->
# Brief: note 47 stage 3 — fix round after Sol's code review

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, HEAD `d8f23af`; the working tree holds stage 3, uncommitted (do not revert it, do not touch the index). Read `AGENTS.md`, then in `~/.cache/ezsdr-fixes/2026-11-04-n46/`: `brief-s3-impl.md` (its rules apply), `test-plan-s3.md` rev 3 (§10 deviations), `sol-s3-code-final.md`, `dispositions-s3-code.md`.

Task: implement every row of `dispositions-s3-code.md` (1, 2, 3, 5, 6 in code/tests/specs; 4 as a record in note 47 §4's stage 3 paragraph and KL-35). For row 1, both Providers refuse a `start_rx` on a stream that is streaming after replay, through the shared decision (no second copy); restore `at` on generated `StartRx` and in uhd-control differential layer 3; Sol's sequence becomes a fake test; check every stage-2/3 test that sends a second `start_rx` to a started stream and decide it by subject. Mutation rows for each new decision (refusal of a second start, the negative-key check, the id on booking failures), confirmed by probes as in `brief-s3-impl.md`; logs under `work/s3-fix-probes/`. Update `test-plan-s3.md` to rev 4 (keep rev 3 as `.v3`) and its copy `plan/phase8/reviews/47s3-test-plan.md`.

Rules, suites and the end checks as in `brief-s3-impl.md` (fmt, clippy both, workspace tests, Python 3.14 and 3.9, 20 runs for timing-dependent new tests, chmod, `rules.py --base HEAD --renames plan/phase8/reviews/47s3-renames.txt` → 0, no test process left).

Report (at most 15 lines): per row, done/how; tests and rows; probes; suites; `rules.py`.

<!-- brief-s3-fix2.md -->
# Brief: note 47 stage 3 — fix round 2 (minor findings of the re-check)

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, HEAD `d8f23af`; the working tree holds stage 3, uncommitted (do not revert it, do not touch the index). Read `AGENTS.md`, `~/.cache/ezsdr-fixes/2026-11-04-n46/brief-s3-impl.md` (its rules apply) and the re-check `opus-s3-fix-recheck.md` (same directory).

Implement, as the re-check says:
1. MockRadio overflow (`crates/ezsdr-mock-radio/src/lib.rs` ~1511): a `start_rx` booking failure becomes a refusal with the Action's id (`reject_action_at(Some(id), "command", "RM-25: …")`), the snapshot not inserted before a failed booking; add a stepped Run to the `ea_11_a_start_the_line_cannot_represent_…` overflow test (`Rejected` with the id, the Run continues).
2. 4(a): delete `|| self.halted` and the then-dead `self.cut` line in `crates/ezsdr-radio/src/timeline.rs` ~468; delete rows P1059 and J04 (equivalent now); rewrite `rm_25_a_start_after_a_refusal_begins_from_the_refused_segment_s_cut` (`crates/ezsdr-radio/tests/timeline.rs` ~542) and the H04 case of `rm_26_the_timeline_cases` as Stop then start; keep the `halted` field and rows P90, P460 (check they still match once and are killed).
3. 4(b): bound `ur_25_a_short_lead_and_a_missed_start…` by advancing the fake device's clock to `origin.max(completion) + core.ticks(5_000_000)` before the first `turn()` (`rx.rs` ~2530), so a regression fails instead of hanging; confirm both tests using the helper pass and that removing `LateStart` fails fast.
4. Nits: `core.rs` ~676 clock-end failure kind follows the booking that reached it (`command` for a start) or say why not; a Python test with a float and a `None` channel key; RM-25 lines ~261–262 in `design/07-radio-model.md` (a start no longer cuts a halted stream; MockRadio now refuses a `start_rx` on a streaming stream).
5. Update the plan to rev 5 (keep rev 4 as `.v4`) and its copy `plan/phase8/reviews/47s3-test-plan.md`.

Rows: confirm every row you add or re-anchor by a probe (logs `work/s3-fix2-probes/`). End checks as in `brief-s3-impl.md` (fmt, clippy both, workspace tests, Python 3.14 and 3.9, chmod, `rules.py --base HEAD --renames plan/phase8/reviews/47s3-renames.txt` → 0, no test process left). Report in at most 10 lines.

<!-- brief-s3-gate-fix.md -->
# Brief: note 47 stage 3 — gate fix round (3 mutation rows)

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, HEAD `d8f23af`; the working tree holds stage 3, uncommitted (do not revert it, do not touch the index). Read `AGENTS.md` (§7, §9), `~/.cache/ezsdr-fixes/2026-11-04-n46/brief-s3-impl.md` (its rules apply), `brief-s2-gate-fix.md` (same decision rules: RETARGET / OWED / DEAD / EQUIVALENT, and the **probe width** before claiming no surviving test kills a row), and the gate output `gate-s3/mutation.log`, `gate-s3/gate.json` (same directory).

The gate failed on three rows of `plan/maintenance/tools/mutations.json`:
- **P469** (`start_rx` of a channel-less stream is refused, UHD `control.rs`): COMPILE ERROR — the code around its anchor changed in stage 3; re-anchor to the same behaviour with a killing test.
- **P464** (a refusal halts stream 0 whatever stream booked it, UHD `core.rs` `self.lines.refuse(stream, seq)`): SURVIVED its filter `ur_25_a_successful_start_clears_a_later_stream_s_halt` after stage 3 rewrote that test (Stop before the start). Find or add a test where a refusal on stream 1 must halt stream 1 and not stream 0.
- **P1056** (a refused segment's cut goes into the plan, `crates/ezsdr-radio/src/timeline.rs` `if !refused && (cut > 0 || …)`): SURVIVED `rm_25_a_refused_receive_segment_is_not_in_the_plan` after stage 3's fix round 2 rewrote refused-segment tests. Decide whether the `!refused` term is still reachable (a refused segment's cut through Stop-then-start); retarget, add a test, or delete as equivalent with the reason.

Decide each, confirm by a probe in a throwaway copy outside the repository (logs `work/s3-gate-fix-probes/`), record the decisions in `test-plan-s3.md` (rev 6, keep rev 5 as `.v5`) and its copy `plan/phase8/reviews/47s3-test-plan.md`. End checks as in `brief-s3-impl.md`. Report in at most 8 lines.

