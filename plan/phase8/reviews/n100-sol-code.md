# #100 — Sol's code review (2026-11-12, openai/gpt-6.1-sol, read-only)

My failure scenarios are:
- Replace an unsent repeat at its origin; no predecessor burst or false cancellation should be recorded.
- Receive a later switch exactly at the open repeat’s next sample, after it has emitted a block; `first` is then false.
- Admit a finite booking after a Stop, or process a loss at the same boundary; neither caller of `open_held` may overwrite a live booking.
- Check that replacement preserves switch settlement and overlap footprints, in release as well as debug builds.

I’m now comparing these with the fix and regression assertions.
The fix misses the later-switch case: after one repeat block completes, a switch received exactly at `open.next` reaches `open_held` with `first == false`, so the debug assertion remains reachable. UHD explicitly handles this case through `join_open`; the Mock’s new guard handles only an unsent predecessor. I’m checking the release-build consequence and the test coverage.
## Verdict: FAIL

### 1. Major — the fix leaves the same panic reachable after the first block
**VERIFIED by source trace.** `crates/ezsdr-mock-radio/src/lib.rs:954–957`

Minimal sequence using the existing harness:

```rust
let mut h = tx_harness("ideal");
h.arm_start(0).unwrap();
let d = tx_domain(&h);
h.actions.push(tx_action(d, 2_000, 2_000, true, LatePolicy::SendAsapAndFlag));
h.step(0).unwrap();
h.step(4_000_000).unwrap(); // emits [2000, 4000); open.next = 4000, first = false
h.actions.push(switch_action(d, 4_000, 1_000));
h.step(4_000_000).unwrap(); // still panics
```

The switch is accepted at boundary 4000 (`lib.rs:1719–1735`). Its held start equals `open.next`, so `segment::extent` returns `None` (`crates/ezsdr-radio/src/segment.rs:98–101`). Consequently, `emit_tx_until` cannot finish the predecessor. `advance` nevertheless calls `open_held` (`lib.rs:1995–2024`), whose new guard skips the predecessor because `first == false`. `TxBook::take` then hits the original assertion (`crates/ezsdr-radio/src/staging.rs:722`).

**Release consequence:** `take` overwrites the booking, but not the tracker or device model. The replacement’s first SOB produces a tracker discontinuity and rejection (`lib.rs:1859–1878`); the still-open device model also rejects it (`lib.rs:1892–1908`). The switch can already be recorded as `applied` before that emission (`lib.rs:2038–2044`).

**Required:** handle the already-emitting predecessor at this boundary, including tracker/device closure, and add this sequence as a regression. Simply removing the `first` condition would suppress the debug panic but leave the release failure.

## Other requested checks

- **Other bookings — VERIFIED:** ordinary finite bookings cannot overlap the repeat’s region (`staging.rs:1078–1086`); a finite predecessor normally ends through `ends_waveform` and `complete` (`segment.rs:78–79`, `staging.rs:795–797`). Stop/loss processing explicitly ends the open booking (`lib.rs:1187`). The other caller, `transmit_until_cut`, calls `open_held` only after checking that no booking remains open (`lib.rs:925–935`). The problematic caller is `advance`.

- **`DeviceEnd::Keep` — VERIFIED for the narrow unsent case:** it changes neither the record nor device chain (`staging.rs:633,734–739`). With no emitted predecessor block, there is no tracker record or device-model burst to close. Pending switch, `carried`, `halted`, and overlap footprints therefore retain their receipt/settlement semantics. The channel segment is already replaced at the same key (`lib.rs:1767`); cutting it would incorrectly cut the replacement.

- **UHD comparison — VERIFIED, but incomplete:** UHD prevents `take` with an open booking: its open-booking branch returns before `tx.rs:459`. It explicitly handles **both** unsent and already-emitting predecessors through `join_open` (`crates/ezsdr-radio-uhd/src/provider/tx.rs:403–405,1002–1009`), including tracker closure (`tx.rs:751–755`). The Mock fix copies only the unsent case. UHD also uses `Started::AtFirstIssue` (`provider/core.rs:415`), so an unissued repeat cannot accept this origin switch (`staging.rs:1252–1253`).

- **`x3xx` exclusion — VERIFIED for this sequence:** its 10 ms window plus 2 ms lead place the switch boundary beyond receipt (`crates/ezsdr-radio/src/x3xx.rs:411`, `crates/ezsdr-mock-radio/src/profile.rs:47`, `lib.rs:1719`). Unlike `ideal`, it cannot newly insert the switch at the current emitted cursor.

- **Test and mutations — VERIFIED by inspection:** the new test (`tests/mock_radio.rs:3484–3505`) covers only unsent predecessors, held and open. N100-1 is killed by the internal debug assertion—not a functional assertion; with debug assertions disabled, that deletion preserves the tested overwrite behavior. N100-2 fails the waveform-dependent sample/wrap assertions. The supplied `mut.log:5–9` records both killed. Neither row covers the major above.

- **Design and maintainability:** MR-42’s new sentence (`design/09-mock-radio.md:225`) correctly describes the narrow origin case. No additional contract change is needed; the remaining sequence violates existing behavior rather than requiring weaker wording. The change reuses existing transitions and fixtures without unnecessary abstraction; its defect is incomplete lifecycle handling, not code volume.

No files changed; no builds, tests, or mutations executed during this review. The major finding is source-traced, not runtime-reproduced.
