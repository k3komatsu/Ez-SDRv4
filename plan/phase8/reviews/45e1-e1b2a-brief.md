# Brief: stage E1b part 2a of the design-note-45 experiment — the simulated world (actors, events, monitor)

You are the implementer (AGENTS.md §9 step 4). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Contract
- Experiment plan (draft 4): `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`, §3.1 (actors), §3.2 (events) and §5's three kinds of fact (device application, qualifying evidence, publication) for the monitor.
- Test plan (approved by Sol): `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md`, section A (types: `World`, `Monitor`, `Ident`, `SettingsExec`, `RxExec`, `Holder`; the events table; the executor rules and the device model) and section C (the World tests listed below).
- Design: `plan/phase8/45-uhd-decision-core.md` §4.4–§4.7.
- Already done and uncommitted in `crates/ezsdr-radio-uhd/src/brain/`:
  - E1a: `core.rs` (Brain, `step`/`op`), `cell.rs`, `rules.rs`, `model.rs` (device model), `mod.rs` fixtures.
  - E1b part 1: `dst.rs` (the generic explorer: `Model` with `key`, `enabled`, `apply`, `safety`, `transition`, `progress`, `witnesses`, `incomplete`; `explore`, `walk`, `replay`, `shrink`, `assert_complete`) and `toy.rs`.
  - Read them first. The world drives them: do not reimplement a decision that already lives there.

## Scope of 2a
- New file `brain/world.rs` holds `World` and its events, and implements `dst::Model` for `World`. This is a deviation from the plan's file list: there, `World` sits in `dst.rs`. Here `dst.rs` stays the generic explorer. Say so in the report.
- **Actors and resources:** the brain (inbox FIFO; `B.step` queues ops; `B.op` runs one op as one event); the settings executor S (FIFO, command in hand); one RX executor R_k per stream (FIFO, loop position, handle state, recv in progress); the cells; the native lock L; the lifecycle gate G; the failure flag F; the device model D (from `model.rs`); the clock reader; the environment slots (each input with an earliest and a latest time, delivered once); the monitor.
- **Events:** every event of §3.2 and the test plan's events table is one `Event` variant, applied as one atomic step. Per-actor FIFO order and causality hold by construction: effect after take, completion after effect, arrival after delivery.
  - `Tick` produces sample `T` and then sets `T += 1`, so the first Tick produces sample 0 (E1-62). It is disabled once an undelivered input's latest is ≤ `T` (E1-53), and when `T` reaches horizon + epilogue.
  - `Timer(d)` is enabled only when `T ≥ d` (E1-64), and its delivery is recorded as a deadline observation (E1-55).
  - `S.take` of a cancelled value sends `Done{Dropped}` itself (E1-42).
  - `S.result` comes before `S.send` (E1-54).
  - The loss publication follows the event order the table gives: release L first; `Loss.gate` / `Loss.set` / `Loss.ungate` / `Loss.mark`; a lifecycle caller already holding G sets F inside it, with no recursive `Loss.gate`.
  - The R and S checks of F come before their calls (`rules::loop_check`, `rules::settings_check`).
  - `R.result` writes the phase through `Cell::device_result`.
  - `R.publish` goes through `Cell::publish`, and the consumption evidence comes from `rules::consume_packet`, for published and unpublished drain samples alike.
- **Monitor:** finite summaries only, as section A's `Monitor`. Per value: `applied`, `evidence` and `leftovers` as `Ident` (value, channel, segment, epoch, sample). Also: publication progress per epoch, dispositions, command terminals, deadline observations, the drain-ended flag per chain, witness bits. It records facts and checks nothing: the oracles are part 2b. In 2a, `World::safety`, `transition` and `progress` return `Ok(())`, with a one-line comment naming part 2b. `incomplete` is real in 2a: an actor still waiting (for example R in `recv`) at the epilogue end gives `Incomplete` naming the actor (E1-65).
- **Key:** the exact behaviour state, in `Ord` types, with no hash and no `HashMap`. `World` is destructured without `..` in `key`, so a new field fails to compile until the key decides it. The trace and the diagnostic counters (`recomputes`, `discarded`) stay out of the key.
- **Construction:** a plain constructor taking the parameters the cases set: streams and their cells, horizon, epilogue, queue capacity, DSP FIFO capacity, recv timeout, drain window, and the environment slots. No case scripts: those are part 3.
- **Tests of 2a** (section C names): `harness_key_separates_each_behaviour_field`, `harness_key_ignores_trace`, `harness_key_ignores_diagnostic_counters`, `harness_per_actor_fifo_order`, `harness_first_tick_produces_sample_0`, `harness_failed_reservation_leaves_no_send`, `harness_take_cancelled_sends_done_dropped`, `harness_recv_timeout_loop_at_end_is_incomplete`, `harness_deadline_beyond_last_input_is_observed`. Each builds a small world in the test (a few lines of setup, reusing a fixture where one fits). The tests that check "in every visited state of case N" are part 3.
- **Mutation rows of 2a:**
  - Planned: E1-29, E1-42, E1-62, E1-63, E1-65.
  - E1-35, E1-37 and E1-66 target `R.result` and `S.check` in the plan, but their named killers are pure E1a tests. If `R.result` only calls `Cell::device_result` and `S.check` only calls `rules::settings_check`, the decision has one home that E1a rows already mutate (check `mutations.json`). Then do not add these three rows; say so in the report (AGENTS.md §9 maintainability item 5).
  - The rest (E1-22, 23, 41, 44, 47, 48, 49, 52–57, 64) come with part 3, where their killing case tests exist. Leave each anchor point simple, so those rows can be written later without production scaffolding.
  - Rust `old` anchors are token sequences that match exactly once (`python3 plan/maintenance/tools/rules.py --base HEAD`). Each row must break its named test. Reason it out; do not run mutations.
- **Not in 2a:** oracles (2b), case scripts and measurement (3). No change to E1a or part-1 code unless something blocks you. If something does, change as little as possible and report it.
- **Size:** checkpoint about 1,000 lines including tests. Well past it, stop at a coherent point with the tree green and report what remains. Never compress code or drop planned tests to fit. If the plan is contradictory or unimplementable, implement its safest reading and report it.

## How to build and test (owner: never put build or test load on this PC)
- Every cargo build, test or clippy runs on sim02 through `python3 plan/maintenance/tools/remote/rdev.py -- cargo …` from the repository root, one at a time, in the foreground. Examples: `… -- cargo test -p ezsdr-radio-uhd --lib brain` and `… -- cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings`. Never run cargo build, test or clippy locally.
- The only local cargo command is `cargo fmt --all`, before reporting. Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r`.
- Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout. Test temp files, if any, go on /tmp.

## Self-review (one round, before reporting)
From the repository root, in the foreground: `timeout 2400 opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free --agent plan --auto "<checklist>"`. If it prints nothing for 3 minutes, kill it by PID and relaunch once. If the free model refuses for quota, use `opencode-go/deepseek-v4.1-flash`.

Checklist to pass to the reviewer: read 45-experiment §3.1, §3.2 and §5, test plan sections A and C, `brain/world.rs` and the diff, then report:
1. Every §3.2 event is present as one atomic step, with the plan's enabling condition and effect. Name each missing, merged or extra event.
2. Every decision the world makes that already lives in `core.rs`, `cell.rs`, `rules.rs` or `model.rs` (duplication).
3. The key: every behaviour field in it, the trace and the counters out of it.
4. Each listed test observes what section C says, and each 2a mutation row breaks it.
5. The monitor records the three kinds of fact apart, with `Ident`s.

Read-only, no mutation runs. Fix what it finds, or reject a finding with its reason in your report.

## Report (return this, at most 30 lines)
- Files and line counts (body / tests).
- The exact rdev commands run and their counts, and the clippy result.
- The rows added and the test each breaks, and the rows not added with the reason.
- The `Event` variants, names only.
- The self-review findings and their disposition.
- Every deviation from the plan, and every place you found it contradictory.
- What remains, if you stopped at the checkpoint.
