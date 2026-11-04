# Sol test-plan review: note 47 stage 3 (receive values change again, on a stopped stream)

You are the independent reviewer (read-only). Repository `~/local_work/Ez-SDRv4`, HEAD `d8f23af` (stage 2 committed: every radio key classless, the Run-time change machinery deleted). Read `AGENTS.md` §9 (step 3). Do not edit, format, commit or run mutation tools; if you build, from a copy outside the tree with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`.

- Design: `plan/phase8/47-no-change-while-streaming.md` §1, §2.1 (items 2, 4), §2.2, §2.3, §2.4, §2.5, §3 (Q4, Q6), §4 stage 3 and stage 2's 「段階 3・4 に残したこと」.
- In `~/.cache/ezsdr-fixes/2026-11-04-n46/`: the brief `brief-s3.md`, the plan `test-plan-s3.md` (rev 1), the orchestrator's rulings on its §0 questions `rulings-s3-plan.md`.
- Stage 2 records in the tree: `plan/phase8/reviews/47s2-review.md`, `47s2-sol-code.md` (your code review), `47s2-test-plan.md`.

**First write your own failure scenarios** from the step's purpose, today's code (UHD `crates/ezsdr-radio-uhd/src/provider/{control,core,rx}.rs`, MockRadio `crates/ezsdr-mock-radio/src/lib.rs`, `crates/ezsdr-radio/src/timeline.rs`, the Kernel admission and pipeline `crates/ezsdr-kernel/src/coordinator/`, the server `crates/ezsdr-server/src/`, Python `python/ezsdr/session.py`) and primary sources (UHD 4.10 for an untimed retune with the receive streamer stopped), and only then compare them with the brief's S1–S11 and the plan. Then review:
1. Does each planned test observe what it claims, and would the wrong behaviour named fail it? Are the mutation rows the smallest that break the intended behaviour, and does each name a killing test?
2. Lifecycle and order: the "streaming" fact (D2: booking time), staging across `Stop`, loss, `Provider::stop`, a refused start, a cancelled start, two streams, the owner's configure snapshot and its configure-once comparison, the start-lead rule with a slow configure (D4), O2's timing (is every Provider refusal for the submitted Action really present when submit returns, for the device-paced Run and the stepped MockRadio Session? H6).
3. The fake/hardware boundary: what the fake models of an untimed configure with the streamer stopped versus UHD 4.10; the VERIFIED/INFERRED marks of §1.
4. Scope: missing or extra against the brief and note 47; the D1 shared `Staging` type (one rule, not two copies); the generator's `Value` class and its oracle (D10).
5. The rulings: any you would overturn, with a counterexample.

Report in your final message: verdict (PLAN PASS / PLAN NEEDS CHANGES), findings major/minor with path:line, for each major a minimal operation sequence and the observable wrong outcome, a suggested fix. At most 100 lines.
