# Brief: test plan for the design-note-45 receive-slice experiment, stages E1a + E1b

You are the implementer for this step (AGENTS.md §9 step 2). This brief asks for a **test plan only**: write no code, run no cargo, change no file in the repository.

Repository: ~/local_work/Ez-SDRv4 (branch main). Read AGENTS.md first (§9 is the workflow). Do not cd into v3/. Do not read anything under `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/` (a holdout set the evaluation uses; reading it invalidates the experiment).

Risk: risky (a new decision core, cell procedures, executor and device models: queues, deadlines, identities, lifecycles, and the boundary between a fake and the hardware).

Read:
1. `plan/phase8/45-uhd-decision-core.md` (design note 45, all of it).
2. `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md` (the experiment plan: scope, stages, actors, events, cases, oracles, pure-level sequences U1–U15). This is your contract.
3. `plan/phase8/44-device-chain-state.md` §3 and `plan/phase8/39-dsp-timed-commands.md` §6.6–§6.8 (the semantics the slice builds on).
4. Code to reuse or mirror: `crates/ezsdr-radio-uhd/src/provider/core.rs` lines 86–372 (`RxEntry`, `SegmentKey`, `RxDevice`, `RxChain`), `crates/ezsdr-radio-uhd/src/device.rs` (`RxStreamer`, `Dsp`, `DSP_FIFO`, `open_flow`/`close_flow`, `start_rx`, `drain`, `settle`, `rx_recv`, `rx_stop`, `apply`).

Write `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md` with:

A. **Module sketch** (no code bodies): the files under `crates/ezsdr-radio-uhd/src/brain/`, the main types and their fields (Brain, Input, Command, Cell and its phase, value state per note 45 §4.4, executor states, the device model, the harness state), the cell procedures as method signatures, and which existing types you reuse. Keep it to what the tests need to name.

B. **Pure-level tests** for each of U1–U15: test name (`u01_...`), the setup, the operation sequence, what it asserts, how the assertion observes the behaviour (which value/field/command/log), and the wrong behaviour that would fail it.

C. **Harness tests**: for the harness itself (DFS visits every enabled event, visited-set hashing covers every field that affects behaviour, cycle detection, budget overrun counts as failure, trace replay is deterministic, shrinking keeps the failure) — each with name, assertion and the wrong behaviour it catches. Include at least one test where a deliberately broken toy model must be caught, so the harness's power is shown on something known.

D. **Cases 1–6 and the seeded configuration**: for each, the exact script (actors' initial state, environment inputs with their earliest/latest times, parameters), which invariants and progress properties apply, the reduced non-ignored variant, and an estimate of the state count (say how you estimated). If you think a case as written cannot meet the 10-minute budget, say so and why.

E. **Oracles**: for each of I1–I11 and P1–P5, how the harness evaluates it (which state it reads, at which events), and a wrong core behaviour that it must catch.

F. **Mutation rows** you plan to add with the code (they come with the code, not now): for each, the file, the behaviour mutated, and the test that should kill it. Rust rows use token anchors (AGENTS.md §7). Rows go in `plan/maintenance/tools/mutations.json`.

G. **Open questions**: anything in the experiment plan that is ambiguous, contradictory or that you think is wrong, with your proposal. Do not silently resolve a contradiction.

Rules: one assertion or two per test; repeated setup goes in helpers; name every test after its row (u01_, case1_, harness_, inv_i1_ …). No wall-clock waits anywhere in E1 (it is single-threaded). All later builds and tests run only on sim02 through `python3 plan/maintenance/tools/remote/rdev.py -- cargo ...` (owner: no build load on this PC); write the plan with that in mind (e.g. the exhaustive runs as `#[ignore]` tests run there with `--ignored`).

Return: the path of the test plan and a summary of at most 20 lines (counts of tests per section, the open questions in one line each).
