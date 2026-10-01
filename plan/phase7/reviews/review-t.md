# Review T — re-review of Review S's fixes and UR-7's reopen (`git diff 1153d43 8ebf593` on `worktree-phase7-impl`)

Reviewer: Claude Opus, resuming the Review N–S session, 2026-10-01. I changed no repository files. The scratch copy was `/tmp/claude-1000/-home-komatsu-works-Ez-SDRv4/d82454b2-b1d7-4f3c-a624-72d8e684572b/scratchpad/review-t/copy`, made with `rsync -a --no-times` and touched before each build session. I compared every file under `crates/` against HEAD `a0e1ae5`, at the start and after the mutations: it matched. Every build ran in `ezsdr-v4-dev:uhd4.10` with `CARGO_TARGET_DIR=/cargo-target/review`, and UHD 4.10's source was read in the image. No USRP was used.

Brief: [`../prompts/review-t.txt`](../prompts/review-t.txt). The report as returned.

## VERDICT

**No blockers. Every Review S finding is closed in the code. One part (`reclaim`) has not been seen working on the bench.**

**The new loss rule.** A dead link that UHD reports only as `op_timeout` (UHD error 47) now ends the Run. On the bench, through the switch, it did so 3.0 s after the unplug (bench-results part 10), and the fake test pins it.

**The device's drop.** `UhdDevice::drop` no longer frees a device it has not marked lost when the device does not answer.

**UR-7's reopen.** It is sound:
- the first radio and device are dropped, and the device freed on its live link, before the second open;
- no root clock is left in the `ClockRegistry`, because `DeviceAuthority::new` fails in `set_sources`, before it allocates or registers its root;
- the prefix contract is a shared constant.

**Test results.**

| Run | Result |
|---|---|
| Fake suite, stable | 129/129 |
| Fake suite, 1.85.0 | 129/129 |
| `--features uhd` | 128/128 |
| Server `protocol` tests, stable and 1.85.0 | 37/37 each |
| B33–B38 | All killed, each by the test the JSON names |

**Open, all P2.** A narrow false-loss path in the new rule; `KEPT` and `reclaim` edge cases; and `reclaim`'s unobserved effect.

## BLOCKERS

None.

## NONBLOCKING

- **NB-T1 (P2) — failing `ref_locked` reads alone can declare a live device lost.**
  - In `control.rs` `check_device`, a successful time read resets `failing_since` only when `ref_locked` also succeeded (`Ok(_) if failed.is_none()`).
  - When the reference is monitored, a `ref_locked` read that keeps failing (any error) while the time reads succeed therefore makes `DEVICE_LOST` after 1 s.
  - A device whose time reads succeed is not gone. On the X300 both reads use the same control path, so the case is unlikely (INFERRED).
  - Fix: base the 1 s rule on the time read alone, or reset on any successful time read, and keep a `ref_locked` failure as `ref_locked_failed` timing only.
  - The reset itself is unpinned (TG-T1).
- **NB-T2 (P2) — the rest of the false-loss question checks out.**
  - **A slow read under load** succeeds, so it does not count.
  - **A cold switch or reconfiguration** takes the `control` mutex, which serializes with the read and causes no error.
  - **A time read behind a timed command:** the Module releases timed commands at most 3 ms ahead (UR-24), so a read queued behind one waits at most that.
  - **The rule while uhd-control is itself blocked:** the rule runs on uhd-control, counting from the start of the first failing read, so a single long blocked read followed by a second failure fires correctly. That is the bench's 3.0 s.
  - **A board without the sensor:** `ref_locked` returns `Ok(None)`, not an error.
- **NB-T3 (P2) — `reclaim` holds `KEPT`'s lock across `answers()`** (`uhd.rs` `reclaim`). On a dead link each kept device's read waits out UHD's timeouts: part 10's 14 timeouts while the open retried every 2 s.
  - While it waits, any `UhdDevice::drop` that keeps a device blocks on the same lock.
  - Fix: take the matching entries out of the list, read them unlocked, and put back those that do not answer.
- **NB-T4 (P2) — `reclaim` can free a kept device while another live `UhdDevice` of the same `args` exists.**
  - `reclaim` runs on every `open(args)`. If a live device of those `args` exists at that moment, the kept one's teardown writes the shared X300's radio registers (`deinit`) and releases its claim (`~x300_impl`: claim time and source 0) under the live one. INFERRED disruption.
  - Not reachable in `ezsdr-server` (one Session per process; the reopen loop drops the first device before it reopens). Reachable from a test process or embedding code that opens twice.
  - Fix: count the live devices per `args`, and reclaim only when none lives.
- **NB-T5 (P2) — `UhdDevice::drop` can block for UHD's control timeouts on a dead link.** Usually that is the serving thread at `RunHandle::finish`, which is acceptable. It is also any thread that drops the last `Arc`, including `uhd-clock` after its detach. That one is harmless: the thread is already detached.
  - A device whose time read fails for another reason is kept rather than freed: a leak, conservative, and right.
- **NB-T6 (P2) — `KEPT`'s entries are not dropped at exit, and that is right.** `KEPT` is a Rust `static`, never dropped; its handles stay in UHD's function-static map, which UHD tears down at exit, as UR-29 states. Rust never frees the kept streamers, which is right: freeing them would call into a graph UHD's teardown may already have destroyed.
- **NB-T7 (P2) — `KEPT` and the threads still holding a device.** A kept device's streamers cannot be held by live Provider threads: those threads hold `Arc<Core>`, and through it the device, so `UhdDevice::drop` (which moves the device into `KEPT`) runs only after the last of them has gone. When `reclaim` drops the kept streamers (after setting `lost = false`), nothing else holds them, so they are freed before `uhd_usrp_free`, keeping the order `UhdDevice::drop` relies on.
- **NB-T8 (P2) — `reclaim` on a kept device whose UHD task loop exited.** The loop that exited in part 9's log is the X300's firmware-control task: "x300 fw communication failure", the claimer.
  - `answers()` reads the time over the RFNoC control endpoint, whose receive worker was still alive in part 9's gdb thread list, so the read can succeed after a replug (INFERRED).
  - If it never answers, the device stays kept until exit. Through the switch that exit was normal (part 10).

## TEST_GAPS

- **TG-T1 (P2)** — the 1 s rule's reset on a successful read is unpinned: removing it (mutation T-A) survives the whole `ur_` suite (117 passed). A regression would turn two transient failures far apart into a false `DEVICE_LOST`. Fix: a fake fault that fails one time read, then recovers; assert no `DEVICE_LOST` after a second failure more than 1 s later. `FakeFault::TimeReadFails` is unused by any test and persistent, so it cannot do this as it stands.
- **TG-T2 (P2) — `reclaim` and the drop's `answers()` have no fake and were not observed on the bench.**
  - Part 10 says so; it cannot show whether the reopen freed the kept device first.
  - Fix: expose a count of kept devices (for example `ezsdr_radio_uhd::uhd_kept_count()`), and have `hw_b9_unplug_and_reopen` print it before and after the reopen. It should go 1 → 0 when `reclaim` frees the device.
- **TG-T3 (P2)** — the transmit-only latency is measured once through the switch (3.0 s), and Review S's direct-cable figure (5.8–6.0 s) predates the rule. A rerun on the direct cable with the rule in place would complete NB-S1's re-measurement, which §19 says the owner asked for.

## PREVIOUS_FINDINGS

| Review S finding | Status | Evidence |
|---|---|---|
| S-B1 (P1) undetected dead link freed | **CLOSED** | Reads failing for 1 s make the device lost (`failing_since`, `FAILING_FOR`); `UhdDevice::drop` keeps an unmarked device that does not answer. B35 killed by `ur_29_a_dead_link_that_fails_without_lost_is_a_lost_device` (`FakeFault::Unreachable`, failing as error 47). Bench through the switch: `DEVICE_LOST` with `UHD error 47: RfnocError: OpTimeout`, 3.0 s, process exits normally. Residual NB-T1. |
| NB-S1 (P2) latency attribution | **CLOSED** | Part 9 and UR-29 separate the event from the Run's end; the event's UTC printed from the Manifest's relation (`lost_utc`). Re-measurement on the direct cable: TG-T3. |
| NB-S2 (P2) kept device on reopen | **CLOSED** in code | `KEPT` with streamers; `reclaim(args)` before `uhd_usrp_make`, freeing kept devices that answer. Bench: reopen in one process and a normal exit. `reclaim`'s effect not observed (TG-T2); NB-T3, NB-T4. |
| NB-S3 (P2) device kept after a panic | **CLOSED** | Covered by `reclaim`. |
| NB-S4 (P2) `submit` waiting on a dead link | **CLOSED** | Stated in UR-29. |
| NB-S5 (P2) streamer frees | **CLOSED** | Recorded. |
| NB-S6 (P2) `mark_lost` race | **CLOSED** | Recorded. |
| TG-S1 (P1) no dead-link fake | **CLOSED** | `FakeFault::Unreachable`; B35. |
| TG-S2 (P2) bench through a switch, and reopen | **CLOSED** | Part 10: carrier kept, transmit-only by the new rule, receive by the silence rule 1.10 s, reopen in one process. |
| TG-S3 (P2) server loss test | **CLOSED** | `ea_07_a_lost_device_ends_the_session_with_its_manifest`. The next-`Connect` part does not apply: one Session per process. |
