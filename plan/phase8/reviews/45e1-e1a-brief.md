# Brief: implement stage E1a of the design-note-45 receive-slice experiment

You are the implementer (AGENTS.md §9 step 4). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Contract
- Experiment plan (draft 4): `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md` — scope, actors, events, oracles, pure-level rows U1–U18.
- Test plan (approved by Sol): `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md` — section A (module sketch) and section B (pure-level tests) are this stage; sections C–E are stage E1b, not now. Mutation rows of section F that target E1a code come with this stage.
- Design: `plan/phase8/45-uhd-decision-core.md`; semantics: note 44 §3, note 39 §6.6–§6.8.

## Scope of E1a
- `crates/ezsdr-radio-uhd/src/brain/` with `mod.rs`, `core.rs` (Brain, Input, Command, Op, Disposition, Terminal, `Brain::step`, conditional ops), `cell.rs` (Cell, phases, in-flight values with the five states, revision, the four procedures and the Stopping transition), `rules.rs` (the executors' pure rules; consumption through the production `RxChain::consume`), `model.rs` (the resumable device model; each function names the FakeDevice function it mirrors). `lib.rs` gets `#[cfg(test)] mod brain;`; `provider/mod.rs` makes `core` `pub(crate)` (the only production edit).
- Every test of section B (60), named as the plan names them, in the module they test. Tests are pure: no threads, no clock, no sleep.
- The section-F rows whose target is E1a code, appended to `plan/maintenance/tools/mutations.json` with ids `E1-NN` as planned; Rust `old` anchors are token sequences that must match exactly once (`python3 plan/maintenance/tools/rules.py --base HEAD` checks it).
- Not in E1a: the DST harness, oracles, cases, toy models (E1b). Do not wire anything into the Provider, uhd-control, uhd-rx or FakeDevice.
- Size: about 1,500 lines is a checkpoint, not a target. If E1a goes well past it, stop at a coherent point, keep the tree compiling and green, and report what remains. Never compress code or drop planned tests to fit.
- If the plan turns out contradictory or unimplementable at some point, do not resolve it silently: implement the plan's reading that is safest, mark it in the report, and continue.

## How to build and test (owner: never put build or test load on this PC)
- Every cargo build, test or clippy runs on sim02 through `python3 plan/maintenance/tools/remote/rdev.py -- cargo …` from the repository root, e.g. `python3 plan/maintenance/tools/remote/rdev.py -- cargo test -p ezsdr-radio-uhd --lib brain`, `… -- cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings`, and before reporting `… -- cargo test --workspace` once. Never run cargo build/test/clippy locally. Run one rdev command at a time and wait for it in the foreground.
- Formatting is the only local cargo command allowed (it compiles nothing): `cargo fmt --all` before reporting.
- Never run `mutate.py` or `remote_mutate.py` (the orchestrator runs the rows at the gate). Never git commit, push, stash, reset or checkout.
- Test temp files, if any, go on /tmp.

## Self-review (one round, before reporting)
From the repository root run, in the foreground (it takes minutes):
`timeout 2400 opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free --agent plan --auto "<checklist>"`
(if the free model refuses for quota, use `opencode-go/deepseek-v4.1-flash`). The checklist you pass: read the experiment plan, the test plan sections A, B, F and the diff (`git diff` and the new files under crates/ezsdr-radio-uhd/src/brain/); report (1) every U-row test of section B present and observing what the plan says, and each E1a mutation row breaking a named test; (2) scope against the plan, missing and extra; (3) for each "every / only / all" claim the code makes, a search of the positions it covers; (4) maintainability per AGENTS.md §9 items 1–5 (duplication with provider/core.rs and device.rs, state kept with its owner, leftovers); do not run mutation; read-only. Fix what it finds, or reject a finding with its reason in your report.

## Report (return this)
At most 30 lines: files and line counts (body / tests), tests passing on sim02 (counts, the exact rdev commands run), clippy result, rows added, self-review findings and their disposition, every place you deviated from the plan or found it contradictory, what remains if you stopped at the checkpoint.
