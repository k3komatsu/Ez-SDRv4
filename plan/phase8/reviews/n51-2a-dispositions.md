# Note 51 stage 2a — review record and dispositions (2026-11-13)

Every UHD integration Run goes through a test wrapper `Run` (crates/ezsdr-radio-uhd/tests/common/run.rs; included by `#[path]` in crates/ezsdr-acceptance/tests/uhd.rs) that judges how a Run ended at each method return that can settle it (creation, `submit`, `advance_to`, `wait_for`, `run_until_end`, `finish`, `Drop`): a real-time bound — KC-12a or KC-21a in the termination's reason or any `also` entry, RS-8a by a timed-out cleanup step, a Provider join `did not join within` — writes the `EZSDR-INFRA:` marker and fails; an end the test did not declare fails. Declarations: `Expect` (`Refused`, `Policy(kind)`, `Any` only in three observation-only bench loops), `join_is_subject`, `rx_join_issue_67` (a `uhd-rx-*` join only, citing #67). `Drop` finishes a live Run, also while unwinding (marker only, guarded). Fixtures 4, 4b, 6, 7, 8 and five e2e cases (9 → 14) show each bound reaches the runner as a bound; a source scan (crates/ezsdr-acceptance/tests/governance.rs) rejects any reference to `connect` / `start_spec_run` or a `RunHandle` outside the wrapper, `Expect::Any` outside its three loops, and time reads in tests outside an allow list; `server.handle` goes through one fn; delay injection (`EZSDR_RIG_DELAY_MS`/`_AT`, crates/ezsdr-radio-uhd/tests/common/infra.rs) and a sweep ([n51-2a-sweep.md](n51-2a-sweep.md)): 15 tests change verdict under injected delays, the input of stages 2b/2c. Risk: risky (the boundary between a fake and what it stands for: what a test verdict means). Branch `work/n51-2a` from `42f5031`, rebased onto `e449397` and `cc8c3cb`, eight sub-steps, one fix round, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n51-2a-briefs.md](n51-2a-briefs.md)); test plan [n51-2a-test-plan.md](n51-2a-test-plan.md).

## Process

- Test plan with out-of-tree probes; Sol's plan review ([n51-2a-sol-plan.md](n51-2a-sol-plan.md)): FAIL, three majors, three minors, all adopted (briefs, "plan-decisions").
- Sub-steps, each a separate run and a local commit: 1 the wrapper and its pure judgement (rows 01–07); 2 the switch of every UHD integration Run (full lists: 12 rows `ERROR` from markers written while a failing test's Run was torn down, settled: `spec_run_ended` tears down with the #67 declaration, `ur_06` uses it, N92-1 narrowed with N51-2A-21 `bound`, R15/P880/J317/J318 `bound`); 3–4 the acceptance and protocol tests (2351/2352); 5 fixtures and e2e (14/14; 2351/2352); 6–7 the scan and the injection (2365/2366); 8 the text. The only row not killed in each full run: `phase7/G09`.
- Sol's code review ([n51-2a-sol-code.md](n51-2a-sol-code.md)): FAIL, one major, two minors. Fix round 1 (42 rows, all killed). Opus re-check ([n51-2a-recheck.md](n51-2a-recheck.md)): closed, one minor, two notes.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 3 | 3 | `wall-clock-test-oracle` (M1 a join bound exempted before T0; M2 a method-return check satisfiable by `Drop`; M3 a bound cut in the background) — plan defects |
| Sol, code | 1 | 2 | `wall-clock-test-oracle` (M: a KC-21a in `termination.also` behind a declared Policy end passed); minors: unexpected markers captured, a function-value alias missed by the scan |
| Opus, re-check | 0 | 1 | — (a row pinning only the pure `captures` helper) |

These are the mechanism note 51 is the design answer to; they were fixed inside the wrapper, not beside it.

## Dispositions

**Sol, test plan (FAIL)** — all adopted: M1 a join is a bound in every Run (the #67 tests declare `rx_join_issue_67`); M2 each check test catches the panic of the one call; M3 a read-only return cannot see a new end (`CleanedUp` is entered only inside a settling call), so the guarantee is stated at the settling methods and `Drop`; minors: the scan's forms, the fixtures' observed-entry handshake with a marked setup bound, fixture 8 kept; `Expect::Any` only in named observation-only loops.

**Sol, code (FAIL)** — all adopted, fix round 1: `judge_end` takes the whole termination section and reads every `also` entry for KC-12a / KC-21a before any declaration (rows N51-2A-40, -41); `capture_markers(expected, ..)` keeps only the expected bound's marker and the no-bound tests capture nothing (N51-2A-42); the scan rejects any reference to `connect` / `start_spec_run` (call, import, function value) outside `Run::` and `fn` (N51-2A-26/-27/-28 retargeted). The background interleaving of the major is not driven end to end (KC-21a's budget is a fixed 5 s of wall time); the pure test covers the decision.

**Opus, re-check** — closed. Minor (N51-2A-42 pins the pure helper only): recorded, not fixed — no test can make a second bound appear in a passing join test without a Kernel change, and the four bounded tests assert exactly one marker. Note (`support/mod.rs`'s `session_run_with` is outside the scan; pre-existing, no instance): to stage 2b. Note (tests README wording): fixed by the orchestrator.

## Decided by the orchestrator

- Full lists not run at sub-step 1 (dead code and seven rows); they ran at every later sub-step.
- `run_stage` (measurement/multichannel.rs) declares `Expect::Any`: an observation-only throughput stage records a Policy end (`run_ended_early`), as before 2a; bounds still fail.
- P880, R15, J317, J318, N51-2A-21 `"kill": "bound"` (their only kill is a bound; P880's goes with #67).
- `Drop` finishes a live Run while unwinding, guarded by `catch_unwind` (`finish()` panics only on a broken Kernel invariant).
- The three allow-listed time-reading tests that drive a Run stay allow-listed in 2a; they and the sweep's 15 tests go to stages 2b/2c.
- `rig_delay` lives once in `common/infra.rs` (protocol.rs includes it); the scan's size (a 410-line lexer) accepted.

## Gate

PASS ([n51-2a-gate.json](n51-2a-gate.json)) on sim02:8 and sim03:8: 2577 tests, 623 UHD, `hw-build` (the `hardware` binary with the wrapper), Python 121 × 2, e2e 14/14, mutation 2376/2377 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, E2A2A-10, J317, J318, N51-2A-21, N51A-14, N51A-15, N51B-08, N51C-06, P880, `phase7/G16`, `phase7/R15`, no `ERROR`, no timing re-runs.
