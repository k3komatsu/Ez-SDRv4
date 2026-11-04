<!-- brief-s2-a.md -->
# Brief: note 47 stage 2, part (a) — every radio key classless; decide each failing test by subject

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `45a4294` plus an uncommitted edit to `plan/phase8/47-no-change-while-streaming.md`. Read `AGENTS.md` first, then note 47 §4 in full — especially 「段階 2 以降の組み替え」, its 「Sol の確認で足したこと」 (the list of **pieces that must survive**) and 「2a と 2b をまとめる」, which is the design of this step.

**Risky step.** The plans and reviews so far are background, not a checklist to recompute: `~/.cache/ezsdr-fixes/2026-11-04-n46/` `test-plan-s2a.md` (rev 2), `sol-s2a-plan-final.md`, `opus-s2a-plan2-final.md` (its M2/M3 per-test decisions still apply where the subject survives without TX Run-time changes), `dispositions-s2a-plan.md`, `dispositions-s2a-plan2.md`. A previous implementer measured that flipping only the receive keys fails 271 tests (`s2a-failed-after-class-flip.txt`, patch `s2a-d1-class-flip.patch`).

## What changed in the design

Stage 2 now removes Run-time value changes in **both** directions at once: every `radio.{rx,tx}.c<n>.{frequency_hz,gain_db,antenna,stream}` and `radio.{rx,tx}.s<k>.sample_rate_hz` gets no update class. The Kernel then refuses them during a Run (admission) and in a Spec `schedule` (validation). There is no "KEEP-TX-KEY" any more: TX Run-time changes are gone too. The TX retry after a refused change (S10) is stage 4's.

## Part (a), this task

1. Apply the class flip for all radio keys in `crates/ezsdr-radio/src/lib.rs` (comment naming note 47 stage 2).
2. Run `cargo test --workspace --no-fail-fast` (and the UHD crate with `--features uhd` via `plan/maintenance/tools/remote/rdev.py`, and the Python suites) as the inventory. Every test that fails is decided **by its subject** after reading it:
   - **REWRITE-INITIAL**: its subject survives (Stop/`start_rx`, bursts, continuation, repeat, F8 recovery, loss, `Provider::stop`, T3, refusal at `start_rx`/prepare, read-back refusal, initial-envelope checks, channel routing, overflow/sequence faults, coercion at connect, admission logging) — move its stimulus to initial values (`connect` initial requests / Spec `requires`) or a surviving action, keeping its assertions.
   - **REFUSAL**: its subject is "a Run-time change is admitted/refused" — turn it into the classless refusal (rejected, logged, not dispatched), where that is still a useful pin (e.g. §58 #16 `v58.rs` and `uhd.rs`, `ea_11`, Python `sdr.set`), otherwise delete.
   - **DELETE**: its subject is the Run-time change machinery itself (timed release, hand-over, restore, move, from none, live `cold`, instance floor, timed queue/slots, retune phase on a timed tune, H4's control-dispatch proxy, rate-transition campaigns).
   Tests that do not fail now but test internal APIs of the removed machinery (`book_timed`, `book_cold`, …) are **not** part (a); they go with the code in part (b).
3. Python: `.stream` and the frequency/gain/antenna/sample_rate setters refuse in the client during a Run with an `Error` naming `connect(rx=ezsdr.Stream(...))`/`tx=`; the Kernel refusal stays pinned through `sdr.set`/`submit` (catch `Rejected`).
4. The tree must be **green** at the end of (a): every test passes or is deleted with its line in the table; dead production code may remain (part (b) deletes it). No `#[ignore]`.
5. Write `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s2.md` **rev 3** (keep older revs as `.v1`/`.v2` copies if not already): one table row per failing test — path:line, test name, subject in a few words, decision, and for REWRITE-INITIAL the new stimulus; plus a list of the tests the re-check (`opus-s2a-plan2-final.md`) named, each with where it landed.

Not in part (a): deleting production code, mutation rows, specs, KNOWN_LIMITATIONS (parts (b) and (c)). Do not touch `plan/maintenance/tools/mutations.json`.

## Rules

- Local cargo allowed; build output in `~/.cache/cargo-target/Ez-SDRv4`; test temp files on `/tmp`. Python with `PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`; the 3.9 container per handoff §5.
- Never run `mutate.py` or `remote_mutate.py`; never apply a mutant to the tree. Never git commit, push, stash, reset or checkout.
- `cargo fmt --all`, clippy `-D warnings`, `git ls-files -m -o --exclude-standard | xargs chmod go+r`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` at the end (rules.py may report rows whose killer test you deleted — list them, do not fix rows).
- If a subject is unclear, decide REWRITE-INITIAL when in doubt and mark it `?` in the table for the reviewer.
- If your context gets tight, stop at a green point, write the table so far and report; a fresh implementer continues.

## Report (at most 20 lines)

Counts per decision; files touched; suites; rows `rules.py` reports without killer; `?` items; anything in the design you think is wrong.

<!-- brief-s2-a-fix.md -->
# Brief: note 47 stage 2, part (a) fix round

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `45a4294`. The working tree holds part (a) of stage 2, uncommitted (do not revert it). Read `AGENTS.md`, then:
- the design, note 47 §4 (`plan/phase8/47-no-change-while-streaming.md`: 「段階 2 以降の組み替え」, 「Sol の確認で足したこと」, 「2a と 2b をまとめる」);
- part (a)'s brief `~/.cache/ezsdr-fixes/2026-11-04-n46/brief-s2-a.md` (its rules apply to you);
- the implementer's per-test table `test-plan-s2.md` rev 3 and the work files in `work/`;
- the re-check `opus-s2-a-recheck.md` and the orchestrator's dispositions `dispositions-s2-a.md` (same directory).

## Task

Implement every adopted row of `dispositions-s2-a.md` (M1–M3, m1–m6) in the tree. Read deleted tests with `git show HEAD:<path>` and restore or rewrite them; the re-check gives the stimuli. Keep the tree green: every test passes or is deleted with a reason in the table. The rows marked "part (b)" (m3's in-source cases) and the notes are not this task; leave them.

Then update `test-plan-s2.md` to **rev 4**: fix the table (m6), add rows for every restored or rewritten test, and add a short section "for part (b)" listing what part (b) must keep (from the notes) and the tests m3 hands to it.

## Rules

Those of `brief-s2-a.md`: no mutate tools, no git state changes (no `git rm`, `git add`, stash, reset, checkout, commit), fmt, clippy `-D warnings` (workspace and `ezsdr-radio-uhd --features uhd --all-targets` via `plan/maintenance/tools/remote/rdev.py`), the Python 3.14 suite and the 3.9 container (handoff §5), `chmod go+r`, `rules.py --base HEAD` (report its count; do not fix mutation rows or spec citations — that is part (c)). If your context gets tight, stop at a green point and report.

## Report (at most 20 lines)

Per disposition row: done / how; counts of restored and rewritten tests; suites; `rules.py` count; anything you could not do.

<!-- brief-s2-b.md -->
# Brief: note 47 stage 2, part (b) — delete the unreachable Run-time change machinery

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `45a4294`. The working tree holds stage 2 part (a) and its fix round, uncommitted (do not revert it). Read `AGENTS.md`, then:
- note 47 §4 (`plan/phase8/47-no-change-while-streaming.md`): 「段階 2 以降の組み替え」, **「Sol の確認で足したこと」 — the list of pieces that must survive**, 「2a と 2b をまとめる」;
- `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s2.md` rev 4, especially its section **"for part (b)"** (keep list and handed tests), and the removal maps in `test-plan-s2a.md` rev 2 (§1, §3b, Appendix A/B — background, verify against today's code);
- `sol-restage-final.md` §3 (what must survive) and `opus-s2-a-recheck.md` notes.

## Why

After part (a) every radio key is classless, so the Kernel refuses Run-time radio value changes and Spec schedules of them. The Providers' change paths are unreachable; this part deletes them with their in-source tests.

## Scope

0. **First, the part (a) re-check minors**: implement every "Adopt" row of `dispositions-s2-a-fix.md` (findings in `opus-s2-a-fix-recheck.md`), rows 1–5 and Q3; add them to the rev-5 table.
1. **Delete**, in MockRadio (`crates/ezsdr-mock-radio/src/`), the UHD Module (`crates/ezsdr-radio-uhd/src/provider/{control,core,rx,tx,mod}.rs`, `device.rs` callers) and the shared timeline (`crates/ezsdr-radio/src/timeline.rs`, `channels.rs` update-class helpers): move/enable/disable, channel release and waits, move prune, membership dependency propagation, from none (`claim_from_none`, `Holder::FromNone`, `Holder::Release`), live `cold` booking and its segment cuts, `hardware_timed` release/queue/slots/window/floor, eligibility/debt/drain/flush/`left`/restore/hand-over, `covering`, instance floor, `ColdConfig.configured` (if still there), `Holder::Owner { seq }`'s `seq`, the Provider-side update entrances (`book_cold`, `book_timed`, `apply_update` arms) — and every in-source or `tests/` test whose only subject is that machinery, plus the two tests rev 4 hands to part (b) and `ProbeDevice::matching_timed_apply` (`probe_device.rs:135`) with its test (`fake.rs:7466`). Keep the `tx_start_lead_ns` key (stage 4 decides).
2. **Keep** everything on the must-survive list (note 47 and `sol-restage-final.md` §3): `RxChain` segment identity and lifecycle, cut/Stop, uncertain/lost, first-sample transition, owner configure snapshot and refusal halt with claim return and peer isolation, `Core::configure` and read-back refusal, start-lead rule (cut + old-rate receive call + allowance + lead + lattice) and missed-start retry, clock registration/end order, claim/read exclusion, generation/report correlation, T3, F8, the TX device-burst continuation and held tail, Stop/`start_rx`, loss, `Provider::stop`, cleanup; MockRadio's per-channel value history (`M/src/channel.rs`); `rx/differential.rs` layer 3; the in-source D14 twin near `control.rs:5430`. The Provider must still **refuse** a classless update if a harness calls it directly (MockRadio `lib.rs` near :1984–1996, UHD `control.rs` near :428–450) — keep that guard, simplified.
3. **The VH generator** (`crates/ezsdr-radio/tests/generator/mod.rs`): remove the Cold/Timed operations and rebalance the classes so no class generates empty sequences; remove the Mock-side filter part (a) added; the restored uhd-control differential uses the generator directly.
4. **The TX send path after part (a)**: a transmit stream's settings now change only at the start of the Run; remove TX live-cold/clock-switch code that no caller reaches; keep exact continuation, repeat, Stop front, sample accounting, recovery.
5. Shared dead code found by the compiler after the deletions goes too, each with the search that shows no caller.

Not in part (b): mutation rows, specs, KNOWN_LIMITATIONS, design text (part (c)). Do not touch `plan/maintenance/tools/mutations.json`.

## Method

Delete in small compiling steps; after each, run the affected crate's tests. A test that fails after a deletion is decided by subject as in part (a): if its subject survives, its stimulus moves to initial values or Stop/`start_rx`; never delete a test of a surviving behaviour. Append each decision to `test-plan-s2.md` (rev 5, keep rev 4 as `.v4`) as a part (b) table: deleted production region (path:line at HEAD, what it was), deleted tests (name, subject), rewritten tests. If a kept item needs a deleted piece, stop and report.

## Rules

As in `brief-s2-a.md`: no mutate tools, no git state changes, fmt, clippy `-D warnings` (workspace and `ezsdr-radio-uhd --features uhd --all-targets` via `rdev.py`), workspace tests, Python 3.14 and the 3.9 container, `chmod go+r`, `rules.py --base HEAD` (report its count; rows and citations are part (c)). Stop at a green point and report if your context gets tight.

## Report (at most 20 lines)

Production lines deleted per crate; tests deleted/rewritten; suites; `rules.py` count; anything kept that you expected to delete (and why); anything you were unsure about.

<!-- brief-s2-c1.md -->
# Brief: note 47 stage 2, part (c1) — mutation rows after the deletion

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `45a4294`. The working tree holds stage 2 parts (a) and (b), uncommitted (do not revert them). Read `AGENTS.md` (§7 mutation rows, §9 maintainability rule 5), then:
- note 47 §4 (`plan/phase8/47-no-change-while-streaming.md`): 「段階 2 以降の組み替え」, 「Sol の確認で足したこと」, 「2a と 2b をまとめる」;
- `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s2.md` rev 5 (what was deleted, rewritten and kept in parts (a) and (b), with HEAD line numbers).

## Task

`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` reports 396 mutation-row problems (anchors that match 0 times, scopes that match 0 times). Decide each row of `plan/maintenance/tools/mutations.json` that it names, and also every row whose `filter` names a test that no longer exists (search all rows, not only the reported ones):

- **DELETE** when the behaviour the row mutates was deleted (part (a)/(b) removed the code, or it is reachable only through a Run-time value change). About 337 reported rows have a killing test that is gone; most are this, but check that the code is gone, not moved.
- **RETARGET** when the behaviour survives (it is on the must-survive list, or reachable through Stop/`start_rx`/initial values/loss/bursts): re-anchor `old` (and `old2`) to today's code with the smallest uniquely matching text (Rust token anchors ignore whitespace), keep `new` producing the same wrong behaviour, and set `filter` to a surviving test that you have read and that asserts what the mutation breaks. Say in the table why that test fails on the mutant (the assertion and the line).
- **J33/J34**: J33 is consolidated into J32 (note 47 §4 「Sol の確認で足したこと」: J32's killer is the Kernel `SteppedProvider.with_arm_origin_after` test in `crates/ezsdr-kernel/tests/coordinator.rs`; check it still names a live test). Delete J33 and J34 if their killers are gone and no surviving Provider-level test can kill the same mutant; otherwise retarget.
- **NEW rows** for decisions parts (a)/(b) added or kept that no row covers, one row each, each with its killing test: the Python client refusal of the frequency/gain/antenna/sample_rate/`.stream` setters during a Run (`python/ezsdr/session.py`); the MockRadio and UHD Provider guards that refuse a classless update (MockRadio `lib.rs`, UHD `control.rs`); the classless declaration of the radio keys in `crates/ezsdr-radio/src/lib.rs` (check whether an existing row already covers each before adding).

Each kept, retargeted or new row must match exactly once (`rules.py`). A retarget that you cannot make killable by a surviving test is a DELETE with that reason, never a row whose `filter` does not kill it.

## Editing mutations.json

It is append-only text: **never re-serialize it** (no `json.dump` of the whole file). Delete a row by removing exactly its element span (the `{ ... }` object and one adjoining comma) as text; retarget by replacing only the field values in place; add new rows at the end of the array. After editing, `python3 -c "import json; json.load(open('plan/maintenance/tools/mutations.json'))"` must pass and `git diff` must show only the rows you touched.

## Rules

- No mutate tools: never run `mutate.py`, `remote_mutate.py` or `gate.py`, never apply a mutant to the tree. To check a doubtful row you may apply it in a throwaway copy **outside the repository** (`rsync -a --no-times --exclude .cargo --exclude target` to `/tmp/…`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`), run only its `filter`, and record the outcome in the table; a compile failure counts as **not killed**; save each run's output to a named log under `~/.cache/ezsdr-fixes/2026-11-04-n46/work/c1-probes/`.
- No git state changes. No edits outside `mutations.json` except fixing a test that you find does not assert what its row needs (then say so in the table and keep the tree green). Do not touch `design/`, `KNOWN_LIMITATIONS.md`, `plan/` text — that is part (c2).
- At the end: `cargo fmt --all` if you touched Rust, `git ls-files -m -o --exclude-standard | xargs chmod go+r`, `rules.py --base HEAD`: no mutation-row problems may remain (the cited-test problems are part (c2); report their count).
- If your context gets tight, stop at a consistent point (valid JSON, table written) and report.

## Output

`~/.cache/ezsdr-fixes/2026-11-04-n46/rows-s2.md`: one line per touched row — id, list, decision (DELETE / RETARGET / NEW / KEEP), reason, and for RETARGET/NEW the killing test and why it fails (path:line of the assertion).

## Report (at most 20 lines)

Counts per decision; rows you were unsure about; probes run and their outcomes; `rules.py` count by kind.

<!-- brief-s2-c2.md -->
# Brief: note 47 stage 2, part (c2) — specs, KNOWN_LIMITATIONS and citations after the deletion

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `45a4294`. The working tree holds stage 2 parts (a), (b) and (c1), uncommitted (do not revert them). Read `AGENTS.md` (§2: format of accepted specs, rule ids never renumbered or reused, a withdrawn rule keeps a one-line stub; KNOWN_LIMITATIONS), then:
- note 47 in full (`plan/phase8/47-no-change-while-streaming.md`), especially §4 「段階 2 以降の組み替え」, 「Sol の確認で足したこと」 (its temporary gaps), 「2a と 2b をまとめる」;
- `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s2.md` rev 5 (every deleted, rewritten and kept test, with subjects) and `rows-s2.md` (the row decisions of part (c1));
- `~/.cache/ezsdr-fixes/2026-11-04-n46/dispositions-s2-a-fix.md` (N3) and `opus-s2-a-fix-recheck.md` (notes).

## Why

After parts (a) and (b), no radio key has an update class: the Kernel refuses Run-time radio value changes (RS-52) and Spec schedules of them (SB-16), and the Providers' change paths are deleted. The accepted specs still describe those paths and cite 501 deleted tests (`rules.py --base HEAD`).

## Scope

1. **Specs** `design/07-radio-model.md`, `09-mock-radio.md`, `18-uhd-radio.md` (most of the work), and `01`, `03`, `04`, `05`, `08`, `11`, `16` where they cite deleted tests or describe a Run-time radio value change:
   - A rule whose whole subject is the deleted machinery (timed release/queue/slots/floor, hand-over, restore, move/enable/disable during a Run, from none, live `cold`, instance floor, retune phase on a timed tune, H4 dispatch, rate-transition campaigns) becomes a one-line stub: `- **XX-n** Withdrawn (note 47 stage 2): <what it was, a few words>.` Never delete or renumber an id.
   - A rule whose subject survives (the must-survive list in note 47 §4; Stop/`start_rx`, loss, bursts, continuation, T3, F8, configure and read-back refusal, start-lead rule, clock registration, initial values at connect) keeps its text, minus clauses about Run-time changes, and its `*Test:*` line names surviving tests (from rev 5's rewritten tests). A surviving rule must keep at least one live test; if none exists, say so in your report rather than inventing one.
   - State the classless keys where the radio keys are declared (spec 07): every `radio.{rx,tx}.c<n>.{frequency_hz,gain_db,antenna,stream}` and `radio.{rx,tx}.s<k>.sample_rate_hz` has no update class; values are set at connect / in the Spec's `requires` (cite the existing RS-52 / SB-16 rules, do not restate them).
   - Record the temporary gaps of note 47 §4 where the rule lives, as normative text of today's behaviour (not history): §58 #16 is tested as a classless refusal and the in-Run envelope refusal returns in stages 3–4; the RX/TX start-lead measurement groups are deleted until stages 3–4; H4 dispatch is retired (TX headroom and load stay); UR-34 no longer cites `hw_h4_dispatch` and its deleted tests.
   - Each touched spec gets one row in its closing *Changes* table citing note 47 stage 2. History, rationale and issue numbers go there, not in rules.
2. **`cold_change_refused` fault**: it can no longer fire. Withdraw it (delete-first, note 47 §4): remove `FaultKind::ColdChangeRefused` from `crates/ezsdr-sim/src/lib.rs`, its const in `schemas/sim/fault_entry.v1.json` (regenerate the schema the way the crate's schema test does, keep `schema_freeze` green), MockRadio's branches and its sim test (`crates/ezsdr-sim/tests/sim_vocabulary.rs:64–71`: keep the test, drop that entry, add an assertion that a `cold_change_refused` entry is refused), and spec 08 SE-3's list. A fault schedule naming it is then refused, never ignored.
3. **KNOWN_LIMITATIONS.md**: KL-3, KL-7, KL-9, KL-11, KL-24, KL-32, KL-33, KL-34 and any other entry that cites a deleted test or a Run-time change. An entry whose symptom can no longer happen gets status "Gone in note 47 stage 2 (2026-11-04): <why>" (numbers are never reused, entries are not deleted); an entry that still applies at connect time is reworded to that. Add an entry for the user-visible new limit: a radio value (frequency, gain, antenna, sample rate, stream membership) cannot change during a Run; it is set at connect (`connect(rx=ezsdr.Stream(...))`) or in the Spec, until stages 3–4 re-add changes on stopped RX / idle TX streams.
4. **`--renames` through the gate**: `plan/maintenance/tools/gate.py` runs `rules.py --base` without `--renames`, so a deleted test whose citation is dropped fails the gate. Add a `--renames FILE` option to `gate.py` passed through to the `rules` step and recorded in `gate.json`, with a self-test assertion next to the existing rules ones. Write the file as `plan/phase8/reviews/47s2-renames.txt`: `old -> -` for each deleted test whose citation you drop, `old -> new` where a rewritten test replaced it (format in `rules.py`'s docstring).
5. **The Vision** (`design/vision/`): do not edit. List in your report every Vision sentence that is now false (part file, §N, sentence), for the orchestrator.

Not in part (c2): `plan/maintenance/tools/mutations.json`, note 47, handoff, Rust/Python code other than item 2.

## Rules

- No mutate tools, no git state changes. Format of accepted specs per AGENTS §2; `rules.py` must not report history markers or long lines growing.
- At the end: `cargo fmt --all`, workspace tests and clippy `-D warnings` (item 2 touches code), Python 3.14 suite, `git ls-files -m -o --exclude-standard | xargs chmod go+r`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames plan/phase8/reviews/47s2-renames.txt` → **0 problems**, `python3 plan/maintenance/tools/gate.py --self-test` (or however its self-test runs) passes.
- If your context gets tight, stop at a consistent point (one spec finished) and report what is left.

## Report (at most 20 lines)

Per spec: rules withdrawn / rewritten / unchanged-but-repointed; KL entries changed and added; surviving rules left without a live test; Vision sentences now false; `rules.py` count; anything you were unsure about.

<!-- brief-s2-d.md -->
# Brief: note 47 stage 2, part (d) — the pins the deletion lost, and the last dead code

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `45a4294`. The working tree holds stage 2 parts (a), (b), (c1), (c2), uncommitted (do not revert them). Read `AGENTS.md`, note 47 §4 (`plan/phase8/47-no-change-while-streaming.md`: the must-survive list in 「Sol の確認で足したこと」), then in `~/.cache/ezsdr-fixes/2026-11-04-n46/`:
- `opus-s2-c1-unkilled.md` — the re-check reviewer's classification of 62 rows part (c1) deleted as unkilled (RETARGET / OWED / DEAD / EQUIVALENT), with sequences and path:line;
- `work/opus-c1-probes/opus-probe-tests.diff` — probe tests the reviewer wrote and saw fail on the mutants (a starting point: rewrite them to the house style of the file they land in, do not paste blindly);
- `rows-s2.md` (part (c1)'s row table), `test-plan-s2.md` rev 5, `brief-s2-c1.md` (row rules), `brief-s2-c2.md` (spec rules).

## Task

1. **RETARGET (re-add) F36, P1220, V06, T40** as the reviewer says: append each row again at the end of its list (`plan/maintenance/tools/mutations.json`) with its HEAD text (`git show HEAD:plan/maintenance/tools/mutations.json`), the anchor fixed to today's code (T40: `old` = `timeline.rs:270` `at.unwrap_or(now).max(now).max(floor)`, keep `old2`) and `filter` = the killing test the reviewer names.
2. **OWED tests**, each with its row re-added (HEAD text, re-anchored where needed, `filter` = the new test):
   - `fake.rs`: one test for B06 + B07 (slow link, Stop then `start_rx` 60 ms later: the old stream's tail never lands on the new clock; the stopped stream is delivered up to its cut).
   - `rx.rs` units: P1290 (no restart call after a loss), P391 (loss between two channels of a configure), P513 (a Stop drops the plan before `owner_configure`).
   - **Restart keeps the connect values** (a gap no row showed): connect at 2 GHz (initial value), Stop, `start_rx`, the device still reads 2 GHz — the restart snapshot at `control.rs:296` mutated to the profile defaults must fail it. Put P420 (UR-36: the second restart is configured too) in the same test; add a NEW row for the snapshot mutation.
   - `control.rs` unit: U13 (MA-14b: each Action is booked before the next `recv`), re-anchored to `control.rs:200-204`.
   - MockRadio: M68, M69 (per-stream TX Stop in the shared plan); P153 (a `start_rx` on a stream with no link — add assertions to `mock_layout.rs:1079`), P154 (channel order), P172 (`ended_at`); P183, P190, P191 (untimed tune at restart: a CBX channel redraws; channel order; stream order).
   - **MockRadio classless-update guard**: add a direct `update_action("radio.rx.c0.frequency_hz", …)` to `mr_29_other_actions_are_command_rejected` (`mock_radio.rs:2248`): one COMMAND_REJECTED with "MR-18: … not updateable"; NEW row mutating the guard to `Ok(())`. This pins MR-18, which (c2) left with no live test.
   - NEW rows for the RX clock registration at the first block (`rx.rs:574`) and its end at the cut (`rx.rs:322`), with the existing tests the reviewer says kill them (run them to confirm).
3. **DEAD code: delete** (the rows stay deleted): UHD `reconcile` branches reached only by a second TX clock (`core.rs:690-703`: P444, P261, J21); MockRadio's TX untimed-tune path (`lib.rs:1015-1027`, the TX clause at `:993`, `:1849`: P182, P193, P195); the `host_entry_ns` field without a reader (`probe_device.rs:295`, P1142). Check each with a search that shows no surviving caller; if one has a caller after all, stop and say so.
4. **Four live tests whose citations part (c2) dropped** because their subject is gone: `cancellation_reports_only_cold_change`, `ur_26_a_finished_stream_clears_only_its_own_floor`, `ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment`, the `the_ma1_phase_is_read_…` probe. For each, read it: if its only subject is deleted machinery, delete the test (and any row naming it); if it pins a surviving behaviour, cite it again in the spec rule it pins and take it out of `plan/phase8/reviews/47s2-renames.txt`.
5. **Records**: fix `rows-s2.md` — the ten rows P141–P152, P165, P173 were deleted because their code is already gone (not "no surviving test kills"); mark each row re-added or newly added here. Append a part (d) table to `test-plan-s2.md` (rev 6, keep rev 5 as `.v5`): new tests (name, what it asserts, the mutant it kills), deleted regions, deleted tests.
6. **Spec citations**: cite each new test on the `*Test:*` line of the rule it pins (UR-36, MA-14b/UR-14 if it is the control loop, MR-18, the MockRadio rules for segments and restart tunes, UR-17/UR-25 for B06/B07); `rules.py` must stay at 0 with the renames file.

## Rules

- Mutation rows: append-only text edits, never re-serialize; each row matches exactly once. **Probe width**: confirm each row you add or re-add by applying it in a throwaway copy outside the repository and running the named test (a compile failure counts as not killed); logs to `work/d-probes/`. Never run `mutate.py`, `remote_mutate.py` or `gate.py`; never apply a mutant to the tree. Build one copy at a time in `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0`; leave no test process running when you finish (kill your own by PID).
- No git state changes. `cargo fmt --all`; clippy `-D warnings` (workspace, and `ezsdr-radio-uhd --features uhd --all-targets` via `plan/maintenance/tools/remote/rdev.py`); workspace tests; Python 3.14 suite and the 3.9 container (handoff §5); `git ls-files -m -o --exclude-standard | xargs chmod go+r`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames plan/phase8/reviews/47s2-renames.txt` → 0.
- New tests whose verdict can depend on timing (threads, a wall clock): run them 20 times and report.
- If your context gets tight, stop at a green point and report.

## Report (at most 20 lines)

Per item: done / how; tests added, rows added or re-added (ids), regions deleted (lines), tests deleted; probe outcomes; suites; `rules.py`; anything you could not do.

<!-- brief-s2-fix.md -->
# Brief: note 47 stage 2 — fix round after Sol's code review

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `45a4294`. The working tree holds stage 2 parts (a)–(d), uncommitted (do not revert them). Read `AGENTS.md`, then in `~/.cache/ezsdr-fixes/2026-11-04-n46/`: `sol-s2-code-final.md` (the review), `dispositions-s2-code.md` (what to do), `orch-review-s2.md` (the TX clock note), `test-plan-s2.md` rev 6 §6 (why `reconcile` went), and `git show HEAD:crates/ezsdr-radio-uhd/src/provider/core.rs` lines 985–1031 (`changed`/`reconcile` at HEAD) and `HEAD:…/control.rs:318–332` (the clocks seed).

## Task

1. **Finding 1 (P2), the TX clock end.** In `crates/ezsdr-radio-uhd/src/provider/core.rs`, restore the end of a transmit stream's clock when its plan acquires a cut: at booking (in `Streams::book`/`refuse`/`end` — wherever the plan changes, including the loss path `core.rs:942` that bypasses uhd-tx), for a TX stream whose `StreamState.clocks` holds its clock and whose plan segment now has a cut, call `core.clocks.end(clock.domain, core.at(clock.instant(cut)))` once (an ended flag or removing the clock; keep one representation — AGENTS §9 maintainability rule 2). Only the single-clock case (stages 3–4 bring more); no registration branch. A failure to end is recorded as HEAD did (`reject_note({"clock_not_ended": …})` or the `command_rejected` path — choose the one HEAD used for the cut case and say which).
   Tests on the controlled clock (a `core.rs` or `control.rs` unit, house style): loss at S < O and `Provider::stop` at S = O each end the TX clock at O (its registry record's `ended_at`); a `Provider::stop` after O leaves… — check today's behaviour first: does the TX plan get a cut on a stop after O? Assert whatever HEAD did there (run the same sequence against HEAD in a throwaway copy to see), and say it in the table. Add a mutation row for the end call (`filter` = the new test), confirmed by a probe.
2. **Finding 2 (P3), stale spec text.** `design/09-mock-radio.md` MR-15 (no `cold` transmit change), MR-34 (no timed-tune constants), the test-table rows at about 09:135, 204, 325, 347, 364 and `design/18-uhd-radio.md` about 530, 567: remove clauses describing deleted machinery and rewrite each row from what the cited test asserts today (read the test). Then sweep every remaining test-table row and rule of `design/07`, `09`, `18` (and `01`, `03`, `04`, `05`, `08`, `11`, `16`) for the same: a description of a Run-time value change, a membership change during a Run, a timed update, a release, a move, from none, live `cold` — rewrite from today's assertions. History goes only in the Changes tables.
3. Append a "fix round" section to `test-plan-s2.md` (rev 7, keep rev 6 as `.v6`).

## Rules

As in `brief-s2-d.md`: append-only mutation rows confirmed by a probe in a copy outside the repository (logs to `work/fix-probes/`), no mutate tools, no git state changes; `cargo fmt --all`; clippy `-D warnings` (workspace, and `ezsdr-radio-uhd --features uhd --all-targets` via `rdev.py`); workspace tests; the Python 3.14 suite; new tests whose verdict can depend on timing run 20 times; `chmod go+r`; `rules.py --base HEAD --renames plan/phase8/reviews/47s2-renames.txt` → 0. Leave no test process running.

## Report (at most 15 lines)

Finding 1: where the end is, what HEAD did after O, tests, row, probe. Finding 2: rules and rows rewritten per spec. Suites; `rules.py`.

<!-- brief-s2-fix2.md -->
# Brief: note 47 stage 2 — last leftover (dead `timed_tune` basis in MockRadio)

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `45a4294`; the working tree holds note 47 stage 2 (uncommitted; do not revert it). Read `AGENTS.md`, then the re-check `~/.cache/ezsdr-fixes/2026-11-04-n46/opus-s2-fix-recheck.md` ("Leftover" and the P3 clause).

## Task
1. Delete exactly what the re-check names: the `phase_fixed` fields (`crates/ezsdr-mock-radio/src/channel.rs` ~61, ~117), their initialisers (`lib.rs` ~2037, ~2051) and the `.set` at ~986; in `layout_phase_basis` (~995–1037) the `is_fixed` element, the `timed` variable, the `random_unless_timed_tune` arm (folds into `_ => fixed = false`) and the `timed_tune` return (basis: "deterministic" if every channel is deterministic, else "none"); `update_layout_segment_coherence` (~1040–1085) and its call (~990). Fix the doc comment at ~978. If any of these has a surviving reader the re-check missed, stop and report.
2. Mutation rows (`plan/maintenance/tools/mutations.json`, text edits only, never re-serialize): delete P170 (its target goes); every other row must still match exactly once — re-anchor any that a deletion moved (P178, P183 stay), confirmed by a probe in a copy outside the repository (logs to `work/fix2-probes/`; a compile failure counts as not killed).
3. Spec: `design/09-mock-radio.md` MR-34 (~205) drop "not coherent until its next timed tune"; check MR-27 and the segment-coherence text still describe the code; one line added to 09's existing note 47 stage 2 Changes row if needed.
4. Append a short "fix round 2" section to `test-plan-s2.md` (rev 8, keep rev 7 as `.v7`).

## Rules
No mutate tools, no git state changes. `cargo fmt --all`, clippy `-D warnings` (workspace), workspace tests, Python 3.14 suite, `chmod go+r`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames plan/phase8/reviews/47s2-renames.txt` → 0. Leave no test process running.

## Report (at most 10 lines)
Lines deleted, rows deleted/re-anchored (probe outcomes), suites, `rules.py`.

<!-- brief-s2-gate-fix.md -->
# Brief: note 47 stage 2 — gate fix round (17 mutation rows)

You are the implementer (Claude Sonnet subagent, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `45a4294`; the working tree holds note 47 stage 2 (uncommitted; do not revert it, and do not touch the index). Read `AGENTS.md` (§7, §9 maintainability), note 47 §4 (the must-survive list in 「Sol の確認で足したこと」), and in `~/.cache/ezsdr-fixes/2026-11-04-n46/`: `test-plan-s2.md` rev 8, `rows-s2.md`, `opus-s2-c1-unkilled.md` (how the re-check classified rows, and why part (c1)'s probes were too narrow), and the gate's `gate-s2/mutation.log`, `gate-s2/gate.json`.

## Task

The gate's full mutation run failed on 17 rows (all in `plan/maintenance/tools/mutations.json` except M30, M53, M54, M66 in `plan/phase3/tools/mutations.json`):

- **COMPILE ERROR** (the mutant no longer compiles — the code around the anchor changed): P392, P445, P464, P469. Re-anchor so the mutant compiles and still breaks the behaviour the row names, with a killing test; if the behaviour is gone, delete the row with the reason.
- **SURVIVED**: J01, J02, J05, J12 (`crates/ezsdr-radio/src/timeline.rs`, killer `rm_26_the_timeline_cases`: the rebalanced generator may no longer produce their cases), J08 (MockRadio fault row), P136 (MockRadio TX clock per stream), P368 (UHD owner configure once), P539, P540 (Python `Session.rx`/`.tx` aliases), M30, M53, M54, M66 (MockRadio per-channel value history and its use at the sample instant — on the must-survive list).

For each row decide, after reading the code and the HEAD killer (`git show HEAD:<test path>`):
- **RETARGET**: a surviving test kills it (set `filter`); or **OWED**: the behaviour is reachable through surviving operations (connect with initial values, Spec `requires`, Stop, `start_rx`, bursts/repeat, loss/faults, `Provider::stop`) or is on the must-survive list, and lost its pin — add the smallest test that observes it (for the timeline rows, prefer explicit cases in `rm_26_the_timeline_cases` or the timeline unit tests; for the value history, a direct unit test of the history in `crates/ezsdr-mock-radio/src/channel.rs` is fine) and set `filter` to it;
- **DEAD**: reachable only through deleted machinery — delete the code (search every path that changes a plan or a value, and say the search) and the row;
- **EQUIVALENT**: the mutant cannot change observable behaviour any more — delete the row with the reason. Never add to `known-survivors.txt`.

**Probe width** (the lesson of part (c1)): confirm every kept, re-anchored or new row in a throwaway copy outside the repository by running its `filter` test, and confirm "no surviving test kills it" only after running every test binary of the row's crate, the Mock and `ezsdr-radio` timeline differentials, acceptance `parity`, `v58`, `v61`, `x3xx`, `reactive`, `uhd`, server `protocol`, and the Python suites for Python rows, with the row's `old2` applied when present. A compile failure counts as not killed. Logs to `work/gate-fix-probes/`.

## Rules

Mutation rows: text edits only, never re-serialize; each row matches exactly once. Never run `mutate.py`, `remote_mutate.py` or `gate.py`; never apply a mutant to the tree; build one copy at a time in `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0`; leave no test process running. No git state changes. If you change specs, AGENTS §2 format and cite new tests on the rule's `*Test:*` line. At the end: `cargo fmt --all`, clippy `-D warnings` (workspace; and `ezsdr-radio-uhd --features uhd --all-targets` via `rdev.py` if you touched that crate), workspace tests, Python 3.14 suite, `chmod go+r`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames plan/phase8/reviews/47s2-renames.txt` → 0. Append a "gate fix round" section to `test-plan-s2.md` (rev 9, keep rev 8 as `.v8`) and the row decisions to `rows-s2.md`.

## Report (at most 15 lines)

Per row: decision, killing test (new or existing), code deleted; probes; suites; `rules.py`.

