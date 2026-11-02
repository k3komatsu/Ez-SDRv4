## Verdict: REVISE

Paths below are relative to `crates/ezsdr-radio-uhd/src/`, unless stated otherwise. Mutation kills are assessed by inspection, not execution.

| Item | Assessment and evidence |
|---|---|
| **P1-A** | **Partly closed.** Ownership-safe restart and second-panic recovery fix the original deadlock/escape (`brain/threads.rs:340–396`). The minimal path introduces an I14 race; see P1 below. |
| **P2-B** | **Not closed as specified.** E2-44 causes premature issuance, but changes the shared trace builder too; S10b fails before its pre-acknowledgement assertion. See P2 below. |
| **P2-C** | **Closed.** Acceptance is injected after computation, before the runtime’s write; evidence must contain `(1, 8)` (`brain/thread_tests.rs:890–910`). The runtime retries using `unchanged_since` (`brain/threads.rs:703–720`), including acceptance-bearing `in_flight` (`brain/cell.rs:340–346`). |
| **D / E2-39** | **Closed by inspection.** Its unique anchor removes recovery’s `run_ops`; forced S5c then cannot replay the required guard operations (`brain/thread_tests.rs:866–873`). No scheduling race is needed. |
| **Minimal-path ordering** | `F` precedes `mark_lost` (`brain/threads.rs:375–386`), but is not serialized with lifecycle admission. |
| **Executor termination** | S and every R receive termination (`brain/threads.rs:393–395`; `brain/rules.rs:516–518`); S’s discarder ends on `brain_gone` (`brain/threads.rs:454–459`). New S5c/S5d assert routing and brain exit, then abort—not actual executor completion (`brain/thread_tests.rs:851–882`). K/timer still require explicit stop. |
| **Once-only recording** | Does not suppress another actual `mark_lost`: every publication still calls it (`brain/shared.rs:290–298`), and `CheckingDevice` logs every call (`brain/model_device.rs:334–336,426–427`). `sink.order` is intentionally not a count of reports. |
| **`before_write` hook** | Test-only through `#[cfg(test)] mod brain` (`lib.rs:8–9`). Defaults to `None` (`brain/threads.rs:89`); ordinary free runs gain mutex overhead but no injected state change (`714–717`). |
| **Anchor uniqueness** | `anchor.find` found exactly one match each for E2-10, E2-37, E2-39, and E2-42–46. |
| **Row semantics / killers** | E2-10/37/39/42/43/45/46 match their named regressions and have deterministic failure paths. E2-44 also deterministically fails S10b, but not at the required assertion. |

### Findings

**P1 — Minimal recovery permits a lifecycle call after `F`.**
- Minimal sequence: inject panics at operations `[0, 1]` → recovery owns G → `guard_minimal` releases G → R acquires G and reads `F == false` → brain sets F and calls `mark_lost` → R performs `rx_close`.
- Observable wrong outcome: a handle closes after F, violating I14 (`brain/oracle.rs:375–380`).
- Evidence: release-before-F at `brain/threads.rs:375–376`; R checks F only at admission (`587–595`) and later calls close (`664–665`).
- Keep F’s publication serialized under G, including the minimal fallback. S5d currently parks other actors and misses this interleaving.

**P2 — E2-44 does not validate S10b’s pre-acknowledgement assertion.**
- The mutant issues Start while processing PlanCancel. `recheck` records Start as outstanding before considering Plan, so Plan waits (`brain/core.rs:724–730,842–850`).
- S10b builds its trace using that same mutated core (`brain/thread_tests.rs:202–214,818–826`). Its command is consequently Start, not Plan; Start produces only one boundary, whereas the script demands two `RSend`s (`822`; `brain/rules.rs:152–159,306–314`).
- The trace-builder assertion fails before runtime creation and the pre-ack assertion (`brain/thread_tests.rs:827–830`). Make that assertion reachable under the mutant.

Not checked: test/mutation execution, full gate, hardware, or the prohibited holdout directory. No files, formatting, or git state changed.
