
Paths: **K**=`crates/ezsdr-kernel`; **R**=`crates/ezsdr-radio`; **M**=`crates/ezsdr-mock-radio`; **U**=`crates/ezsdr-radio-uhd`; **A**=`crates/ezsdr-acceptance`.

### 1. Reachability
- **Session `SetParameter`, explicit `UpdateParameter`, and Module-origin updates:** classless resource keys fail before coercion/checks/dispatch (`K/src/coordinator/admission.rs:106–130`). Keep Provider-side rejection for direct harnesses: `M/src/lib.rs:1984–1996`, `U/src/provider/control.rs:428–450`.
- **Spec schedule:** radio updates fail validation, not merely runtime admission (`K/src/plan/validation.rs:351–361`).
- **Vocabulary aliases cannot bypass this:** `CompileRule::UpdateParameter` produces the same pending update (`K/src/session.rs:381–397`). H4’s alias therefore also stops working.
- **Parameter smuggling does not work today:** radio verbs are burst/Command compilations (`R/src/lib.rs:225–229`); Mock requires empty `start_rx` parameters and burst metadata (`M/src/lib.rs:1681–1684,1745`); UHD likewise (`U/src/provider/control.rs:425–426,493–511`).
- **Children:** a child receives a separate Assembly and runs `start_spec`; its initial `requires` may differ without changing the parent. Device-paced children are refused (`K/src/coordinator/pipeline.rs:1053–1058,1083–1104`).
- **Mock stepping/UHD control:** stepping consumes dispatched actions; it is no alternate settings entrance (`M/src/lib.rs:2872–2878`). UHD’s timed/cold booking entrances are the guarded update branches above.
- **Still reachable:** runtime **configuration execution** at `start_rx`, segment refusal, Stop/cut, recovery, clock operations, and raw `Device::apply` bench calls. Those are not runtime parameter-update loopholes. For example, `start_rx` snapshots configuration and `Rx::begin` configures it (`U/src/provider/control.rs:503–509`; `U/src/provider/rx.rs:523–543`).

### 2. Additional fallout and honest replacements
- **§58 #16:** classless rejection proves “rejected/logged/not dispatched,” but does **not** exercise the RF/performance-envelope checks. Record that temporary gap; retain initial-envelope tests and restore runtime-envelope coverage in stages 3–4. The actual acceptance wording is `design/vision/10-implementation-path-and-testing.md:73`; affected tests include `A/tests/v58.rs:521–579` and `A/tests/uhd.rs:184–191`.
- **KC/RS:** most Kernel tests already use `test.gain`/`test.grid`, despite naming the resource `radio`; do not delete them (`K/tests/coordinator.rs:1182–1289`; `K/tests/run_session.rs:968,1002`). Real-radio examples in acceptance need replacements, not withdrawal of Kernel rules.
- **Short-lead/ended-clock tests:** preserve their subjects using an explicit past burst target and a genuinely ended-clock fixture; initial membership alone does not reproduce either sequence (`A/tests/v58.rs:296–327,541–563`).
- **Server/protocol:** admitted-update examples break (`crates/ezsdr-server/tests/protocol.rs:1056–1060,1498`). Use an existing capture/burst action for generic submit/log tests. The new named-selector test specifically expects an admitted radio gain update; change it to assert the numbered **rejected** key/log until restoration (`protocol.rs:657–683`).
- **Simulation:** pure channel gain/delay/frequency-gate coverage survives (`crates/ezsdr-sim/tests/sim_channel.rs:189–210`). Initial settings honestly replace static gain/frequency fixtures. They do **not** replace timed-retune/sample-boundary semantics: remove those tests with their withdrawn clauses (`M/tests/mock_channel.rs:823–908`; `A/tests/v58.rs:1037–1113`).
- **Keep channel histories:** delayed samples still require historical settings when stages 3–4 return; do not flatten this into one mutable current value (`M/src/channel.rs:102–116`; note 47:62).
- **Python:** attribute/coercion tests and the bench example need initial `ezsdr.Stream` values (`python/tests/test_easy_api.py:142–172`; `python/examples/bench_loopback.py:25–40`). Stop/start and multi-clock capture tests must retain their lifecycle/reader assertions, not disappear wholesale (`test_easy_api.py:546–579`).
- **Bench drivers:** `plan/phase8/tools/ssh-drop-driver.py:64–65` already uses initial TX settings and survives. `run-record.sh` launches measurement consumers, so those consumers must be repaired too.
- **Measurement:** initial membership/rates cheaply replace `enable_tx`, `disable_rx`, companion setup (`U/tests/common/measurement.rs:2670–2690`; `…/measurement/multichannel.rs:926–946`). Live-rate-transition campaigns are **not** equivalent to fresh Runs: defer/redesign their conditions and classifiers (`measurement.rs:2920–2967,3130–3153`).
- **H4:** its control-dispatch proxy fundamentally measures timed gain release, not a generic admission call (`…/measurement/h4.rs:15–41,474–516`). A test Vocabulary key is **not** an honest substitute for production uhd-control latency. Retire/redesign that observation; retain separable TX-headroom/load measurements and label their changed scope.
- Update affected **KNOWN_LIMITATIONS** entries in the same changes, especially timed-retune/membership ceilings (`KNOWN_LIMITATIONS.md:31–34,106–123`); absence of users does not waive that record.

### 3. What must survive deletion
- **RxChain segment identity and lifecycle**, cut/Stop, uncertain/lost state, and first-sample transition (`U/src/provider/core.rs:101–145,180–237,263–273`). Debt/latest/restore specific to timed handoff may go.
- **Owner-side configuration/snapshot and refusal halt**, including claim return and peer isolation (`U/src/provider/core.rs:1733–1792`; `U/src/provider/rx.rs:523–587`). `start_rx` still calls these today.
- **Read-back refusal and loss-between-channels:** retain shared `Core::configure`, exact rate/antenna checks, and rate-set comparison (`U/src/provider/core.rs:1355–1410`). Retarget disappearing killers to initial/open/start paths.
- **Readiness/start lead:** retain cut + old-rate receive-call + allowance + lead + lattice rules (`R/src/timeline.rs:1020–1038`), and missed-start retry (`U/src/provider/rx.rs:651–667`).
- The **configure-completion `ready` timestamp** currently lives inside the deletable cold path (`U/src/provider/control.rs:1175–1180,1215`). Its implementation may disappear temporarily, but preserve the requirement and test scenario for stage 3; otherwise configuration latency silently consumes the promised start lead.
- **Clock registration/end ordering**, TX booking-time registration capability, claims/read exclusion, generation/report correlation, T3 and F8 remain obligations—not obsolete because their present killer starts with membership (`U/src/provider/rx.rs:545–567,755–756`; `U/src/provider/core.rs:1803–1809`; `U/src/provider/tx.rs:1786–1850`).

### 4. Can 2a/2b each be green?
- **Yes, conditionally; they need not land together.** 2a makes RX keys classless and removes RX-only behavior across both Providers, tests, generators, Python and specs; TX changes remain supported.
- Shared `FromNone`, timed queue/floor, move containers and profile terms must remain while TX still uses them. For example, `book_cold` is direction-generic and its floor is instance-wide (`U/src/provider/control.rs:989–1012,1122–1154`). Delete shared machinery only in 2b after its final callers disappear.
- Thus “green at each step” is feasible, **not established by the paragraph**; mixed-direction tests/rows need explicit dispositions.

### 5. J33 / RS-19
- **Faithful—and already implemented.** `SteppedProvider.with_arm_origin_after(5_000_000)` registers a future-origin TX clock; tests assert ASAP coercion to 5 ms and dispatched sample tick 0, with and without Provider lead (`K/tests/coordinator.rs:1391–1424`).
- **J32 already mutates exactly J33’s statement** using that Kernel killer (`plan/maintenance/tools/mutations.json:668–685`). Consolidate the duplicate J33 obligation into J32 rather than inventing another Provider. Restore radio-specific integration coverage when new clock transitions return; review J34 similarly (`mutations.json:688–695`).

### 6. Cost and recommendation
- **Choose corrected re-staging:** removes obsolete concurrency machinery before new staging; costs temporary feature/coverage gaps, external fixture repairs, and careful retained-path killers. The 113/63 counts are not all avoidable.
- **Reject membership-only 2a/2b-with-retargets:** preserves current features, but pays extensive retargeting now and another deletion/review later.
- **Reject merging removal plus restoration:** avoids the temporary gap but combines two Provider lifecycles and new clock/retry semantics into the largest, hardest-to-review step.

No files changed; no builds, mutation runs or hardware checks performed. Green-step feasibility and the eventual staged hardware behavior remain unverified.
