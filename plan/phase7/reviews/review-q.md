# Review Q — re-review of the fixes to Review P (`git diff 9597e02 ea63b3a` on `worktree-phase7-impl`)

Reviewer: Claude Opus, resuming the Review N/O/P session, 2026-10-01. I changed no repository files. The scratch copy was `/tmp/claude-1000/-home-komatsu-works-Ez-SDRv4/d82454b2-b1d7-4f3c-a624-72d8e684572b/scratchpad/review-q/copy`: the tree at `4aa3497`, made with `rsync -a --no-times` and touched before each build session. No other copy was built during this review. My D-1 probe (`probe_q.rs`) is kept beside the copy. Every build ran in `ezsdr-v4-dev:uhd4.10` with `CARGO_TARGET_DIR=/cargo-target/review`, one at a time, and the copy was restored to match the tree after each probe. No USRP was used.

Brief: [`../prompts/review-q.txt`](../prompts/review-q.txt). The report as returned.

## VERDICT

**No blockers. Review P's findings are closed or recorded as open, and the fix introduced no regression I could find.**

**What was fixed.**
- **`abandon`:** it now closes every device burst that has begun with an empty buffer, straight after what the device took. Covered: a first buffer cut short, a later one, a continuation whose held sample went ahead, a repeat, a lost device (no call).
- **The held sample:** it is stored only after its send succeeded.
- **Pairing:** pushing the start to `unacked` after a first send cut short pairs it with the `BURST_ACK` that the closing end-of-burst brings.

**The fake.** `FakeFault::ShortSend` models UHD's `send` returning short. In UHD 4.10 (`tx_streamer_impl.hpp:266–345`) a packet that times out ends the call with the count sent so far, and end-of-burst goes only on the final fragment. The fault is not stricter than the device anywhere a test depends on.

**TG-3.** The equivalence argument holds for whether the switch is on time, on the X310 profiles. The term's presence can still be pinned cheaply (TG-Q3).

**Test results.**

| Run | Result |
|---|---|
| Fake suite, stable | 121/121, two runs |
| Fake suite, 1.85.0 | 121/121, two runs |
| `--features uhd` | 120/120 |
| B18, B19, B20, B21, U13 | All killed, each by the test the JSON names |

**D-1.**
- §14's INFERRED cause is right in mechanism. I can name the exact UHD state it races: the C API's static registry of open devices.
- I reproduced the mechanism on the fake: the last device handle is dropped on `uhd-clock`, 3 of 3 runs.
- The candidate fix (wake and join the thread in `drop`) is sound and enough for `hw_b2_authority`, but not in general (see below).

**Open, all P2.** Two of the new `abandon` paths are unpinned (TG-Q1, TG-Q2), plus the remaining items below.

## BLOCKERS

None.

## NONBLOCKING

### D-1 (P2 for the Module; the hardware suite's exit is the risk) — the cause, a better statement of it, and the candidate fix

**The cause (VERIFIED in UHD 4.10's source).**
- UHD's C API keeps every open `multi_usrp` in a function-static map: `usrp_c.cpp:80–84`, `typedef std::map<size_t, usrp_ptr> usrp_ptrs; UHD_SINGLETON_FCN(usrp_ptrs, get_usrp_ptrs)`.
- Every call indexes that map (`#define USRP(h_ptr) (get_usrp_ptrs()[h_ptr->usrp_index].ptr)`).
- `uhd_usrp_free` erases from it under a static mutex (`:236–249`).
- When the test harness's main thread exits the process, `exit()` runs the static destructors: that map (and any `multi_usrp` still in it) and the mutexes.

**The race (INFERRED).** If `uhd-clock` is inside a UHD call at that moment, the static teardown runs concurrently with it. That call is either `uhd_usrp_get_time_now` (indexing the map) or `uhd_usrp_free` (erasing the same entry, then deleting the handle). Either is a use-after-free or double free, consistent with `double free or corruption (out)`.

**How the device ends up on `uhd-clock`.**
1. In `hw_b2_authority` the test thread drops `time`, `authority` and `device`.
2. Normally `DeviceTime`'s last `Arc` goes with `authority`, and `device` is dropped last on the test thread, so `uhd_usrp_free` completes before the test returns.
3. But if `uhd-clock` is inside `reanchor` at that moment, it holds an upgraded `Arc<DeviceTime>` (`authority.rs:286–287`), which holds `Arc<dyn Device>`.
4. The last `UhdDevice::drop`, and so `uhd_usrp_free` (`uhd.rs:435–440`), then runs on `uhd-clock` after the test has returned, while the harness exits.

**Evidence (fake).** `probe_q_the_last_device_drop_runs_on_uhd_clock` wraps `FakeDevice` in a `Device` that records the thread of its `Drop` and makes `uhd-clock`'s read take 60 ms. It drops in `hw_b2`'s order while that read is in progress: the last handle is dropped on `uhd-clock`, 3 of 3.

**Rate.** Per run the chance is about (the read's duration)/100 ms (`REANCHOR`). That is well under 1 % for a sub-millisecond read. One abort in ~20 runs is a single event, so I cannot confirm the read is slower than that on the bench. INFERRED.

**A second path, outside B2.**
- The loop sleeps first and then upgrades, checking the stop flag only before the next sleep (`authority.rs:284–289`).
- So whenever another `Arc<DeviceTime>` outlives the Authority (the Kernel's `TimeAuthority` handle, a Provider's `core.time`), `uhd-clock` reads the device once more, up to 100 ms after `drop`.
- Probe `probe_q_a_device_read_after_the_authority_is_dropped`: 1 read after the drop.

**The candidate fix is sound.** Waking and joining the thread in `drop` means no device call outlives the Authority, and the last device drop can no longer fall on `uhd-clock`. Needed with it:
- **Wake promptly:** replace `sleep(REANCHOR)` with a condvar or `park_timeout` that `drop` signals.
- **Check the flag first:** check the stop flag after waking and before `upgrade`.
- **Bound the join:** a read on a lost device can block inside UHD. After a deadline, detach the thread and record that in `timing`, rather than hang `drop`.

**It is enough for `hw_b2_authority`**: every other holder is on the test thread and is dropped before it returns.

**It is not enough in general (INFERRED).** Any thread that can hold the last `Arc<dyn Device>` at process exit can still race `exit()` the same way:
- UR-16's detached, wedged uhd-rx or uhd-tx thread (via `Arc<Core>`);
- the Kernel's or a Provider's `TimeAuthority` clone dropped on another thread.

Also make the hardware suite (and `ezsdr-server`) drop the device on the main thread, after the threads are joined, before it exits. A `FakeDevice` test like my first probe can pin the joined `drop`: no `Drop` on `uhd-clock`, and no read after `drop`.

### Other findings

- **NB-Q1 (P2, INFERRED) — a first send that fails, rather than coming up short.** In `tx.rs` `send`, `Err` with `device_sob` calls `abandon(was_open)`, which is `false` for a first buffer, and pushes nothing to `unacked`. If UHD had sent packets before it returned the error, two things follow:
  - the device burst is left without end-of-burst;
  - its later `BurstAck` or `TimeError` pops the next burst's `unacked` entry: a mis-paired `TIME_ERROR` target.

  When `uhd_tx_streamer_send` returns an error rather than a short count after sending packets is not known (a lost device is handled: no call). Fix: treat `Err` on a first buffer like a short send with an unknown count — close with an empty end-of-burst and push the start. A stray end-of-burst outside a burst costs one zero sample at most.
- **NB-Q2 (P2) — the continuation's held sample is sent unchecked** (`let _ = tx_send(&tail, …)`). If that one-sample send fails or is short, the device burst lacks the previous burst's last sample and the continuation plays one sample early. Rare; the next send would most likely fail too. Fix: check it, and `abandon(true)` on failure.
- **NB-Q3 (P2) — ShortSend cuts at half the request, not at a packet boundary.** UHD's short counts are whole packets. Harmless for the tests. Noted for UR-33's accuracy.

## TEST_GAPS

- **TG-Q1 (P2)** — pushing a cut-short first send to `unacked` is unpinned: removing it (mutation Q-1) survives the whole `ur_` suite (109 passed). Fix: after `ur_22`'s first-buffer case, send a second burst that the device reports late, and assert the `TIME_ERROR` target is the second burst's.
- **TG-Q2 (P2)** — a continuation's first send cut short, after its held sample went ahead, is unpinned: `was_open = !first` (mutation Q-2) survives. Fix: a `ShortSend` case in `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it` asserting one closing end-of-burst and `unended_bursts() == 0`.
- **TG-Q3 (P2) — the packet term can be pinned by its value, not its effect.** B22 / Q-3 (`block_len` only) survives, as §14 says.
  - The equivalence argument holds for the switch. On the bench, `rx_packet_samples()` read 1 996 at every rate (MTU 9000; INFERRED smaller at other MTUs), so a packet is ≤ 5.1 ms at 390 625 S/s. The call in progress therefore ends ≥ ~44 ms before `e₁` either way, and the stop and the switch come out the same.
  - The term only matters for packets near 95–100 ms, which the ≤ 100 ms receive timeout cuts first: §14's 65 536-sample attempt shows that.
  - It can still be pinned against UR-25's stated floor. A fake case with `block_len` 100 and `rx_packet` 1 996 at 390 625 S/s that asserts `e₁ − booking ≥ 50 ms + max(block, packet)·N + 3 ms` (58.1 ms, against B22's 53.3 ms), as `hw_b8_cold_change_timing_at_the_extremes` does, would kill B22. Optional, since the term is a margin.
- **TG-Q4 (P2)** — `ShortSend`'s dropped end-of-burst is unpinned (mutation Q-4, keeping it, survives). The Module's closing end-of-burst makes the outcome the same, so this is close to equivalent.
- **Carried over (P2):** TG-4 (N-3's margin, N-4's guard), TG-5 (T-c, T-d, T-e).

## PREVIOUS_FINDINGS

| Review P finding | Status | Evidence |
|---|---|---|
| NB-1 (P2) held sample after a failed send | **CLOSED** | Tail stored after success; `abandon(device_open)` closes with an empty buffer; B18, B19 killed by `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took`; `ShortSend` faithful to `tx_streamer_impl.hpp`. Residuals NB-Q1, NB-Q2, TG-Q1, TG-Q2. |
| NB-2 (P2) U13's weak test | **CLOSED** | `ur_14_actions_are_finished_before_the_next_recv` rewritten to the probe; U13 killed. |
| NB-3 (P2) one-sample burst not held | **CLOSED** (kept as specified) | UR-23 states the cost; B21 killed by `ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time`. |
| NB-4 (P2) bound wording | **CLOSED** | `control.rs` comment and UR-25. |
| NB-5 (P2) TimingEnvelope lacks the receive `cold` lead | **OPEN** | Recorded for Phase 8's parity. |
| NB-6 (P2) packet grid across an overflow | **OPEN** | Recorded. |
| NB-7 (P2) test comment | **CLOSED** | Comment corrected. |
| TG-1 (P2) held sample's position | **CLOSED** | B20 killed (sample-by-sample comparison of what the device transmitted). |
| TG-2 (P2) one-sample burst booked ahead | **CLOSED** | B21 killed. |
| TG-3 (P2) the packet term | **CLOSED** as equivalent for the switch | Argument verified above; pin by value is optional (TG-Q3). |
| TG-4 (P2) | **OPEN** | |
| TG-5 (P2) | **OPEN** | |
