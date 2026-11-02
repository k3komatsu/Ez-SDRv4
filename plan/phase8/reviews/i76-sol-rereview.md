# Sol 6.1 — i76-rereview (issue #76), report section

## REVISE

**P2 — The new H4 test still has an unprotected submission window**  
`crates/ezsdr-radio-uhd/tests/fake.rs:9308–9316`; `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:468–503`.

- **Counterexample:** read `first`, then suspend the test thread while Provider participants run/wait until device time passes `first_e`. Resume submission. The target is now late: admission may reject it, or `book_timed` relocates it (`src/provider/control.rs:641–654`). Lookup by the original `e` finds no apply; the new test fails without any mutation.
- `fake()` selects a manual clock but does not register the test thread. M5/M6 therefore leave precisely the earlier counterexample open. Delaying between the two updates exposes another window.
- **Fix:** protect target selection through booking with controlled timing, releasing the clock for waits. Merely using `ClockMode::Manual` is insufficient. P1139/P1142/P1154 still have a scheduling-sensitive filter.

### Other requested checks

- Earlier findings **1, 2 and 5 are closed**: recovery and receive workers register before releasing `Frozen`; returned freezes protect subsequent observations. Exact timestamps and `DropAndFlag` are restored.
- Earlier finding **4 is closed**: the extracted H4 functions preserve the old inline behavior. The fixtures exercise earliest-end selection, later successful-apply exclusion, and successful row mapping.
- `released()` correctly overlaps participant ownership: the worker registers before the old freeze drops, and the replacement freeze registers before the worker exits. No busy-participant time leak or deadlock was found in its current callers.
- **Apply lookup:** `advance_to(e)` is not an all-threads-idle barrier. For an on-time booking on the active RX stream, however, control’s 1 ms polling deadline falls within the 3 ms release window; M4 prevents advancing to `e` before that participant runs. The remaining problem is submission timing, not that lookup ordering.
- P1316 fails on selecting 400 instead of 40; P1317 fails on `"unavailable"` instead of `"mapped"`. P1151 checks the exclusive end boundary. No maintainability blocker found in the helper or fixtures.

Not checked: tests and mutations were not rerun; mutation verdicts above are code-derived. No files changed; the prohibited directory was not read.
