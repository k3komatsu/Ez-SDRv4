# Brief for Sol: review the E2a test plan (note-45 experiment), read-only

You are the independent reviewer (AGENTS.md §9 step 3) of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, format nothing, build nothing, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.** Paths below are absolute, or relative to the repository.

## Order of work (it matters)
1. **Before you read the brief or the test plan,** write your own failure scenarios for E2a from:
   - its purpose: note 45 `plan/phase8/45-uhd-decision-core.md` §4.2–§4.7, §5 and §7 ("再現", "同期");
   - the plan `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md` §1–§5 and its §10 last row (the E2 split and why FakeDevice is not changed);
   - the existing code: `crates/ezsdr-radio-uhd/src/brain/` (E1, committed), `src/device.rs` (the `Device` trait, FakeDevice) and `src/uhd.rs` (the native lock).

   A scenario is a minimal operation sequence (pseudocode) and the observable wrong outcome it leads to. Focus on:
   - where real threads, a `Device`-trait wrapper of the model (`ModelDevice`), trace forcing or the observation record could pass while the wrapper is wrong;
   - where the comparison of DST and threads could be vacuous;
   - where conformance against FakeDevice could miss a divergence that matters to E1's verdicts.
2. Then read the brief `~/.cache/ezsdr-fixes/2026-11-01-n45/brief-e2a.md` and the implementer's plan `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan-e2a.md` (its §12 holds the orchestrator's answers to the open questions; review them too). For each of your scenarios, say whether the plan catches it: which test, and whether its assertion observes the behaviour. Then check each of the plan's tests:
   - Does its assertion observe what it claims?
   - Does it name the wrong behaviour that would fail it?
   - Would its mutation row break it?
3. Also check:
   - the `Event` → thread-point table: can a forced replay pass while the threads use a coarser or different protocol than the DST?
   - the `Observed` fields: is anything that E1's oracles care about missing?
   - the logic moved out of `World`: is any decision left duplicated?
   - the scope against the brief: missing and extra;
   - the maintainability of the plan (AGENTS.md §9 Maintainability);
   - the size estimate.

## Output
- A verdict: PASS, or REVISE.
- Findings P1/P2/P3. Each has `path:line` where it applies and, for a P1, a minimal sequence with its observable wrong outcome.
- At most 120 lines.
- End with "Not checked:".
