# Review S — F4's fix and B9 (`git diff 7dd0d11 0c2ae4d` on `worktree-phase7-impl`)

Reviewer: Claude Opus, resuming the Review N–R session, 2026-10-01. I changed no repository files. The scratch copy was `/tmp/claude-1000/-home-komatsu-works-Ez-SDRv4/d82454b2-b1d7-4f3c-a624-72d8e684572b/scratchpad/review-s/copy`, made with `rsync -a --no-times` and touched before each build session. I verified it file by file against HEAD `da0d4ea`. The working tree gained uncommitted changes during the review (`authority.rs`, `device.rs`, `fake.rs`, `protocol.rs`: the reference-PLL message), which are not in the copy and not reviewed. Every build ran in `ezsdr-v4-dev:uhd4.10` with `CARGO_TARGET_DIR=/cargo-target/review`. UHD 4.10's source was read in the image. No USRP was used.

Brief: [`../prompts/review-s.txt`](../prompts/review-s.txt). The report as returned.

## VERDICT

**Not ready yet: one P1 finding.**

**What the fix gets right.** For a device the Provider has declared lost:
- `mark_lost` runs before `DEVICE_LOST`.
- `UhdDevice` and its streamers then free nothing, along every path that can free them: `close_streams`, the last `Arc` from `DeviceTime`, UR-16's detached threads, `rx_open`/`tx_open` dropping an old streamer.
- `streamer_lifecycle` keeps its own flag and has no device behind it, so it is unaffected.

**Test results.**

| Run | Result |
|---|---|
| Fake suite, stable | 127/127 |
| Fake suite, 1.85.0 | 127/127 |
| `--features uhd` | 126/126 |
| Server `protocol` tests, stable and 1.85.0 | 33/33 each |
| B33 | Killed |

**The P1 (S-B1).** The fix covers only a loss the Provider has detected. A dead link it has not yet detected is still freed, inside `finish`. That is F4's abort again, while the cable is out and the Run ends before `DEVICE_LOST`. In transmit-only and transmit Runs, detection depends on the operating system eventually reporting `ENETUNREACH`: UHD's own RFNoC control timeouts are not mapped to a lost device. So on a link whose host carrier stays up (a switch in between, or a host that keeps the route), such a Run never gets `DEVICE_LOST`.

**The accepted latency.**
- **Receive:** `DEVICE_LOST` itself came about 1.1 s after the unplug, meeting bench.md. The 3–4 s recorded is the Run's end as the test saw it: its 1 s poll plus the stop's 1 s join of a blocked uhd-control.
- **Transmit-only:** the 6.0 s is the event itself, and it is exactly where S-B1 shows.

## BLOCKERS

### S-B1 (P1) — a dead link the Provider has not marked lost is still freed; in transmit-only and transmit Runs it may never be marked

**Location.**
- `uhd.rs` `check_with`: `lost = matches!(code, UHD_ERROR_IO | UHD_ERROR_USB | UHD_ERROR_OS) || (streaming && code == UHD_ERROR_RUNTIME)`.
- `control.rs` `check_device`: a non-lost `time_now` error is only `timing { time_read_failed }`.
- `UhdDevice::drop`: frees unless `mark_lost` was called.
- Spec 18 UR-29.

**Evidence: the error mapping (VERIFIED in UHD 4.10).**
- The RFNoC control path times out with `uhd::op_timeout` (the logs' `Control operation timed out waiting for ACK`).
- `op_timeout` derives from `rfnoc_error` (`exception.hpp:229–250`), which `error_from_uhd_exception` does not map (`lib/error_c.cpp:18–35`), so the C API returns `UHD_ERROR_EXCEPT` (47). The Module counts 47 as not lost.
- Only an `io_error` (30), such as `send error on socket: Network is unreachable`, is lost.

**Evidence: the bench (INFERRED from it).**
- The transmit-only `DEVICE_LOST` message is exactly that `ENETUNREACH` (`UHD error 30`).
- The 500 ms reads before it must have failed with something not counted. `ENETUNREACH` arises when the host's network stack drops the route after the carrier loss (INFERRED: NetworkManager or an equivalent).
- The Manifest's `timing` (`time_read_failed` rows) can confirm this. Bench part 9 did not print it.

**Consequences.**
1. With a switch between the host and the X300 (the host keeps carrier when the cable is pulled at the X300), or a host that keeps the route, a transmit-only, idle or transmitting Run gets no `DEVICE_LOST` at all. Transmit sends merely come up short (UR-22's abandon, not a loss).
2. When the client stops such a Run, `finish` drops the last `Arc<dyn Device>` and `uhd_usrp_free` aborts the process in `x300_radio_control_impl::deinit` (VERIFIED path, part 9): the Manifest and the client's reply are lost, which is F4.
3. The same happens in any Run that ends inside the detection window: ~1 s for receive, ≥ 6 s for transmit-only, for example a client stopping as soon as it sees the device stop answering.

**Fix (one recommendation, two parts).**
1. **Detect.** Add a time-read rule to UR-29: a device time read (or `ref_locked` read) that has failed with any error for ≥ 1 s on consecutive reads is a lost device. This mirrors the receive stream's 1 s silence rule and does not depend on how UHD or the OS classify the failure.
2. **Do not free an unmarked dead device.** In `UhdDevice::drop`, when not marked lost, read the device time once first (`uhd_usrp_get_time_now`, whose C wrapper catches the exception), and skip `close_streams` and `uhd_usrp_free` if it fails.
   - On a live link this costs one read (~0.1 ms). On a dead one it costs UHD's timeouts (seconds), once, at the end.
   - A link dying between the read and the free remains, but only in that narrow interval.

Add a fake case for (1) (TG-S1) and B9 with a switch, or with the host's carrier kept, for both.

Rejected alternatives:
- Mapping `UHD_ERROR_EXCEPT` to lost: too broad (any unmapped exception).
- Matching UHD's message text: brittle.

## NONBLOCKING

- **NB-S1 (P2) — the latency figures attribute the time to the wrong thing.**
  - **Receive** (`hw_b9_unplug`, part 9 after the fix): the `DEVICE_LOST` event's tick is 4 235 276 494, i.e. 21.176 s after the time was set. The time was set (in `DeviceAuthority::new`, inside `spec_run`) before the start print at UTC …361.774, so the event came at or before UTC …382.950: at most 1.10 s after the carrier dropped (…381.853). The silence rule bounds it below at ~1.0 s. So the event met bench.md's "about 1 s".
  - The 3–4 s recorded (`not running at …385.864`) is the Run's end as `until_not_running` saw it:
    - `advance_to(after(run, 1 s))`: up to 1 s of polling;
    - the Policy's stop: `Provider::stop` gave up joining uhd-control after 1 s (`rejected: did not join within 1 s; left detached`), uhd-control being blocked inside UHD;
    - then `cleanup`.
  - **Transmit-only:** tick 6 239 997 161 = 31.200 s; the time zero precedes the print by ≥ the 2 s start-up lead (`past_t0`). So the event came about 5.8–6.0 s after the unplug: the latency is the event's, and it is S-B1.
  - Fix: bench-results part 9, design-notes §17 and UR-29 should separate the event's latency from the Run's end. Each can be computed exactly from the Manifest's root→UTC `ClockRelation` rather than from the print.
  - Acceptance: the receive figure is acceptable as measured. The transmit-only figure should not be accepted as a number, since it is the OS's route timeout, unbounded on other hosts.
- **NB-S2 (P2) — the residual "a kept device may stop the same process from opening it again" is probably stated the wrong way round.**
  - The X300's claim is per process hash (`x300_claim.cpp:26–52`: the same process is `CLAIMED_BY_US`, so `try_to_claim` succeeds).
  - The kept object's claimer task is the task loop that exits on the error ("An unexpected exception was caught in a task loop"), so its claim lapses.
  - `ezsdr-server` opens a new `UhdDevice` on every `Connect` (`catalogue.rs:118–121`). After the cable is back, the next `Connect` therefore most likely succeeds, and makes a second `multi_usrp` for the same X300 while the kept one lives (INFERRED).
  - The likelier cost is at exit: the kept object's `deinit()` writes through its old control transport, which the new open has re-routed. Those writes time out and throw, so the process may abort at exit even with the link up (INFERRED).
  - **Cheaper remedy within the Module:**
    1. Keep a process-wide list of kept devices in `uhd.rs`, by `args`.
    2. In `UhdDevice::open(args)`, before `uhd_usrp_make`, probe each kept device with the same `args` (one time read).
    3. If it answers, free it then: its `deinit()` succeeds on a live link. This ends both the leak and the abort at exit.
    4. If it does not answer, open as now.
  - UR-29 should state the residual as found by the bench: reopen after replug in one process, then exit.
- **NB-S3 (P2) — a reachable device kept after a panic** (UR-16's panicking thread counts as a loss) has NB-S2's same-process reopen and exit cost, and NB-S2's remedy covers it.
- **NB-S4 (P2) — nothing else is hidden by accepting the latency.**
  - **A Policy acting late with the transmitter on:** an X300 cut off from its host underruns as soon as its buffer empties, and `radio_tx_core` goes idle on underrun (`radio_tx_core.v:359–370`). The RF stops within milliseconds whatever the Module's latency (INFERRED from the FPGA source).
  - **A client left waiting:** while uhd-control sits inside a UHD call that waits out its timeouts, a Session's `submit` (KC-21a) waits with it, for seconds, until `DEVICE_LOST` stops the Run. Worth a sentence in UR-29.
  - **A Run that never stops:** S-B1.
- **NB-S5 (P2) — a streamer's free does not touch the device in UHD 4.10.** `~rfnoc_rx_streamer` and `~rfnoc_tx_streamer` call the graph's `disconnect`, which removes the node logically ("TODO: Physically disconnect", `rfnoc_graph.cpp:396–420`); `uhd_rx_streamer_free` only deletes the handle (`usrp_c.cpp:97–101`). Leaking a lost device's streamers is therefore conservative, not required, and harmless. VERIFIED from source; the transport destructors are INFERRED to close sockets only.
- **NB-S6 (P2) — `mark_lost` cannot race a free of the device.** `Core` holds an `Arc` to the device, so `UhdDevice::drop` cannot run during `device_lost`. A streamer drop already past its check when the mark lands frees a streamer, which is harmless by NB-S5.

## TEST_GAPS

- **TG-S1 (P1)** — the fake has no dead link that fails without `lost: true`. `FakeFault::Lost` makes every call fail lost at once, which the real link never does first; UHD gives `op_timeout`, code 47. Fix: add `FakeFault::Unreachable(at)` (every call waits out its timeout and fails with `lost: false`), and test that a transmit-only Session reaches `DEVICE_LOST` within S-B1's bound. It fails today, which shows S-B1.
- **TG-S2 (P2)** — `UhdDevice`'s side of F4 (the skipped frees) has no test short of the bench. Accept that B9 checks it, and add B9 with a switch or the host's carrier kept, and B9 with a replug and reopen in the same process, then exit (NB-S2).
- **TG-S3 (P2)** — no server test covers a loss. Add a `protocol` test with `FakeFault::Lost`: the Session's Manifest is written, the client gets its reply, and the next `Connect` succeeds.
- **The tests and the mutation kill what they claim.** B33 (the `mark_lost` call removed) is killed by `ur_29_a_lost_device_aborts_the_run`. The four loss tests assert one `mark_lost`, before any `close_streams`.
