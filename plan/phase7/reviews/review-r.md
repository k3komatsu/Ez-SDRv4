# Review R — re-review of the fixes to Review Q (`git diff 167c7cb bd8151c` on `worktree-phase7-impl`)

Reviewer: Claude Opus, resuming the Review N/O/P/Q session, 2026-10-01. I changed no repository files. The scratch copy was `/tmp/claude-1000/-home-komatsu-works-Ez-SDRv4/d82454b2-b1d7-4f3c-a624-72d8e684572b/scratchpad/review-r/copy`: the tree at `00f453d`, made with `rsync -a --no-times` and touched before each build session. No other copy was built during this review. Every build ran in `ezsdr-v4-dev:uhd4.10` with `CARGO_TARGET_DIR=/cargo-target/review`, one at a time, and the copy was restored to match the tree after each mutation. No USRP was used.

Brief: [`../prompts/review-r.txt`](../prompts/review-r.txt). The report as returned.

## VERDICT

**No blockers. Every Review Q finding is closed except TG-Q4, which stays open as accepted. The fix introduced no regression I could find.**

**`uhd-clock` and `drop`.** The new loop is correct: no wake-up is lost, spurious wakes are harmless, nothing deadlocks, and the last device handle no longer falls on `uhd-clock`.
- **No lost wake-up:** `drop` sets the flag under the mutex and notifies; `wait_timeout_while` tests the predicate under that mutex before it waits.
- **Spurious wakes:** re-checked by `wait_timeout_while` itself.
- **No deadlock:** the stop mutex is used by nothing else. `drop` holds no lock while it polls, and `next_wakeup`'s naps use a different condvar and mutex.
- **No drop on `uhd-clock`:** the thread holds only a `Weak<DeviceTime>`, never the `DeviceAuthority`. While `drop` waits, the Authority's own `time` field is still alive, so the thread's upgraded `Arc` is never the last one. The last device handle therefore drops on the dropping thread.

**The fake's reports.** They now come in order, and a `BurstAck` only once its end has played. That is the device's behaviour (`radio_tx_core.v`), not stricter: it differs only in leaving out the transport delay. The Provider's `unacked` pairing and UR-26's 100 ms wait at `Provider::stop` both hold under it: at a stop, the last device burst ends within the in-flight window, so its acknowledgement is due in about 10 ms.

**Test results.**

| Run | Result |
|---|---|
| Fake suite, stable | 127/127, two runs |
| Fake suite, 1.85.0 | 127/127, two runs |
| `--features uhd` | 126/126 |
| Server `protocol` tests, stable and 1.85.0 | 33/33 each |
| U35, B20, B22–B30 | All killed, each by the test the JSON names |
| My extra: `drop` without `notify` (R-1) | Killed by `ur_07_no_device_read_follows_the_authority_s_drop` (its < 50 ms bound) |

**Open, P2.**
- The new branch for a failed held-sample send has no test (R-2 survives), and the fake cannot make a one-sample send fail (TG-R1).
- Two residuals of D-1 that are inherent (NB-R1).

## BLOCKERS

None.

## NONBLOCKING

- **NB-R1 (P2) — residuals of D-1 the fix does not, and cannot, remove.**
  - **The 1 s detach.** If a read is blocked in UHD past `CLOCK_JOIN`, `drop` detaches the thread. When that read returns, the thread drops its upgraded `Arc<DeviceTime>`. If the Authority's fields have been dropped by then, that `Arc` can be the last, so `UhdDevice::drop` (`uhd_usrp_free`) runs on `uhd-clock`. If that happens while the process exits, it is D-1's race again. It needs a device gone at exit; acceptable as recorded.
  - **UR-16's detached Provider threads** hold the device past the Session by design.
  - **`ea_07_the_device_is_released_when_the_session_ends`** passes on the old code too, as §15 says. It pins that no reference survives the Session; it is not a test of D-1's race.
- **NB-R2 (P2) — NB-Q1's end-of-burst after an error that sent nothing.**
  - When the failing first send took nothing, `abandon(true)`'s empty end-of-burst becomes, in UHD, one zero sample without start-of-burst or time (`tx_streamer_impl.hpp:266–276`). The X300 transmits it as an untimed one-sample burst at once and acknowledges it, and that `BurstAck` pops the start just pushed, so the pairing holds.
  - The cost is one zero sample at an arbitrary instant: the transmit chain is switched on for one sample. INFERRED negligible.
  - It is the right trade against leaving a begun burst open.
- **NB-R3 (P2) — `FailSend` on a one-sample send returns `Ok(0)`, not an error** (`device.rs`). `took = n / 2 = 0` makes `FailSend(k)` take the `StalledSend` branch (`short && took == 0 → Ok(0)`). UR-33 describes `FailSend` as half taken, then an error. Fix: return the error whenever `fail` is set, even with nothing taken.

## TEST_GAPS

- **TG-R1 (P2)** — the `Err` branch of the held-sample send ahead of a continuation (`tx.rs`, NB-Q2's `Err(error) => … abandon(true)`) is unpinned: removing its `abandon` (mutation R-2) survives the whole `ur_` suite (115 passed). NB-R3 is why: the fake cannot fail a one-sample send. Fix: NB-R3's change, then a `FailSend` case of `ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned`.
- **Carried over (P2):** TG-Q4 (`ShortSend`'s dropped end-of-burst, near-equivalent); NB-5, NB-6, TG-4, TG-5 as §15 records.

## PREVIOUS_FINDINGS

| Review Q finding | Status | Evidence |
|---|---|---|
| D-1 (P2) `uhd-clock` outliving the Authority | **CLOSED** | Condvar wait, flag checked before each read, `drop` wakes and joins (1 s bound). B23 and B24 killed by `ur_07_the_last_device_handle_is_not_dropped_on_uhd_clock` and `ur_07_no_device_read_follows_the_authority_s_drop` (my probes); R-1 killed. Residuals NB-R1. |
| NB-Q1 (P2) a first send that fails | **CLOSED** | Taken as begun: start pushed, `abandon(true)`. B25 and B26 killed (`FailSend`). Cost NB-R2. |
| NB-Q2 (P2) held-sample send unchecked | **CLOSED** | Checked; `Ok(0)` path pinned (B28 killed via `StalledSend`). `Err` path unpinned (TG-R1). |
| NB-Q3 (P2) `ShortSend` wording | **CLOSED** | UR-33 states it cuts at half. |
| TG-Q1 (P2) `unacked` after a first send cut short | **CLOSED** | `ur_22_a_first_send_cut_short_keeps_the_reports_paired`; B27 killed; the fake's ordered, end-timed reports (B30 killed). |
| TG-Q2 (P2) a continuation's first send cut short | **CLOSED** | `StalledSend`; B29 killed by `ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst`. |
| TG-Q3 (P2) the packet term | **CLOSED** | The phase sweep asserts UR-25's floor; B22 restored and killed. |
| TG-Q4 (P2) `ShortSend`'s dropped end-of-burst | **OPEN** (accepted as near-equivalent) | |
