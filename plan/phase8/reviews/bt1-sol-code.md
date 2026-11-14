The RF gates and per-command attenuator reminders are present. I’m checking whether C6 distinguishes an invalid head transmission from the intended late-tail sequence, and whether any mutation “kills” come from incidental panics rather than verdict assertions.
## Verdict: FAIL

### Major

1. **C6 can PASS a damaged head transmission as the intended held-tail experiment.**
   - **VERIFIED:** `crates/ezsdr-radio-uhd/tests/common/mt3_tail.rs:63–102` ignores `SeqError`/`SeqErrorInBurst`, including tickless reports. It accepts an Underflow up to one chunk before the head ends and checks only the last send’s handoff margin.
   - **INFERRED minimal sequence:** Hand the head on time → lose its final transport packet → Underflow within that accepted final-chunk range → hand the delayed close → receive sequence error, then TimeError, without ACK. The verdict returns **PASS**, although the head starved because of packet loss, not the held tail.
   - **VERIFIED source support:** UHD emits transport sequence errors **without timestamps** (`/root/tmp/uhd-4.10.0.0/host/lib/include/uhdlib/rfnoc/chdr_tx_data_xport.hpp:369–374`, inside the specified image).
   - Make sequence-error trials **INCONCLUSIVE**, as C4 already does (`tests/common/mt3_cascade.rs:131–132`), and add a pure verdict test plus mutation row.

### Minor

2. **BT1-53 is killed by an incidental panic, not its intended verdict assertion.**
   - **VERIFIED:** `plan/maintenance/tools/mutations.json:26048` removes the missing-transition reason. An all-dropped grid then reaches `lower.expect("straddled")` in `tests/common/mt3_two.rs:139`, panicking before the test checks INCONCLUSIVE.
   - Retarget the mutation so the assertion—not an unwrap—detects the wrong verdict.

3. **C1’s measurement call pattern changed.**
   - **VERIFIED:** The base implementation read device time after every send. `tests/common/mt3.rs:269–272,395–396` now reads only after the last send for C1.
   - Its burst shape and verdict remain unchanged, but its control-call load and scheduling are not unchanged. Preserve C1’s original pattern or explicitly audit this deviation.

**Other checks:** No production files changed. RF frequency/gain/amplitude checks, C5’s reused two-channel gate, and per-command owner attenuator reminders are present. Rehearsals share hardware bodies; C4 explicitly tests both sides of the fake’s boundary.

No files changed; no formatter, builds, hardware tests, or mutation tools ran. Mutation outcomes were reviewed statically, not independently reproduced.
