# E2a test plan — the orchestrator's answers to §1 and §7 (before Sol's review)

- D1 / Q3 accepted: E2a-2a keeps `step`'s sections as they are and splits only `send` (plan / device calls / `complete`). `tx_step_guarded`, `e1_q2b`, `e1_q14` and N47S5-57 stay. Condition: the plan's claim that `held`, `open` and `chain` are written only by the uhd-tx thread is the premise; a reviewer who finds a receipt-side writer refutes it.
- D2 / Q1 accepted: the started rule is a property of the Provider, given to `TxBook::new` (`Started::AtAcceptance` for the Mock, `Started::AtFirstIssue` for UHD); `receive_switch` takes no `cursor`/`started` argument (A3).
- D3 accepted (S19 / R16): a second switch on a switch whose own payload uhd-tx has not issued is refused.
- D4, D5 accepted: §2.3 is the abandon's rule; P10's table against today's code is the check, and any cell that differs other than through #80's `retain` is a finding.
- D6, D7, D8, D9, D10 / Q6, D11, D12, D13 accepted.
- Q2: refusal text `"UR-38: the repeat the switch replaces has not started on the device"`; KL-41 (the next free number; KL-40 is the last).
- Q4: the F7 `emit` check lands in E2a-2a. Each existing emit under a lock that it finds is reported as a finding and fixed in E2a-2a.
- Q5: the implementer names a candidate killer for P650 by reading; the orchestrator runs it through `remote_mutate.py`; if the row survives, the implementer adds an assertion to an existing `late_by_ns` test or the row is deleted with its reason.
- Q7: new file `crates/ezsdr-radio-uhd/src/provider/tx/tests/e2a.rs`.
