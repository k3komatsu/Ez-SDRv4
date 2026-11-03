**PASS — the reported P2 is closed. No new P1/P2/P3 found in this narrow re-check.**

Paths below are relative to `crates/ezsdr-radio-uhd/src/brain/`.
- `threads.rs:194–202,617–625`: completion is recorded after `run_*` returns, including final sends and sender deregistration. S cannot join while its discarder remains active.
- The non-event wait preserves forced replay: only queued commands require `SDiscard`; terminate with an empty queue returns without another permit. No new hang found.
- `observed.rs:110–115`: `s_ended`/`rx_ended` measure thread completion rather than processing-loop exit; pending discards and final sends remain observable, not hidden.
- `stop_tests.rs:660–679` covers the reported sequence. Read-only anchor checks found exactly one match each for E4-117, E2-38 and E2-40.

Not checked: build/test/mutation execution, hardware, broader E4 changes, or either forbidden holdout directory. No files or git state changed.
