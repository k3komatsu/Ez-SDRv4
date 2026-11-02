# E2b results: the speed of the new runtime against today's owner on FakeDevice

Note 45 §7's speed criterion, measured as `brief-e2b.md` defines it, as amended by its fix briefs. The criterion: p99(new) − p99(today) < 100 µs, p99(new) ≤ 300 µs, loss(new) ≤ loss(today), and every update accounted. Runs were on sim03 (32 cores, load average 16–18). The raw JSON stays outside the repository in `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-results*/`.

## Verdict (round 3, `e2b-results-3/`; loss recomputed from the reader in round 4)
| Condition | Owner | p50 | p99 | p99.9 | Failures | Loss | Verdict |
|---|---|---|---|---|---|---|---|
| 1 MS/s × 2,000, spaced (5 ms) | today | ~540 µs | 1,050 µs | ~1,058 µs | 0 | 0 | |
| | new | ~67 µs | 79 µs | ~92 µs | 0 | 0 | **PASS** |
| 1 MS/s × 2,000, bursty (4 × 100 µs per 10 ms) | today | ~550 µs | 1,076 µs | ~1,088 µs | 0 | 0 | |
| | new | ~60 µs | 82 µs | ~108 µs | 0 | 0 | **PASS** |
| 50 MS/s × 500 (100,000 blocks/s), spaced | today | ~530 µs | 1,045 µs | ~1,060 µs | 0 | receive loss | clean settings |
| | new | ~75 µs | 1.67 s | ~2 s | 1,136–2,873 per repetition | not above today's | **FAIL** |
| 50 MS/s × 500, bursty | today | ~550 µs | 1,080 µs | ~1,100 µs | 0 | receive loss | clean settings |
| | new | ~65 µs | 4.65 s | ~5 s | 2,197–4,094 per repetition | not above today's | **FAIL** |

The 50 MS/s × 500 condition is supported: both owners pass the rate trial at 99,999.98 blocks/s with no updates. 200 MS/s × 2,000 is not supported by either owner on FakeDevice.

## Why the high-rate condition fails (rounds 3 and 4, `45e2b-results-4-readme.md`)
- **VERIFIED in code:** a successful timed call keeps its in-flight slot until R observes the value's consumption in samples (`brain/core.rs:629-674`). Today's command slot expires at device time (`provider/control.rs:659-666,688-689`). A host receive backlog of B ms therefore keeps about B / 5 values in flight (spaced), and at the bound of 4 the next values wait and are applied late.
- **Measured:**
  - Bound hits occur only in seconds with a backlog above 1 ms.
  - The runs stay mostly clean for about 20 s, then the backlog stays at 20–100 ms.
  - At bound 16 (FakeDevice's command queue, `device.rs:413`), the failures fall by about 4.6× spaced and about 1.6× bursty, but they do not disappear. That run is a diagnostic, not evidence for the design.
- **The cause of the backlog (direction VERIFIED, share INFERRED):** FakeDevice scans its DSP history once per block (`device.rs:548-553`), so the scan grows with the number of applied updates. With 10,000 entries preloaded, R saturates at 54.7k blocks/s, taking about 18 µs per block against about 9 µs empty. Today's owner is exposed to the same scan: it loses receive data, but it keeps its settings on time, because its command slots do not wait for the receive path.

## What the rounds removed from the measured runtime (adapter cost, not the design)
| Round | Removed | Effect |
|---|---|---|
| fix 1 A | per-sample scratch reconstruction, a growing `consumed` history cloned per block, per-block tick vectors | 10k → 100k blocks/s in a 10 s run |
| fix 2 | a broadcast on one shared scheduler condvar per acquisition and publication; an unbounded per-call log in `CheckingDevice`; extra cell acquisitions | no-update 50 s trial 72.9k → 99.0k blocks/s; brain, S, clock and timer context switches 72k/s → 0; bursty p99 316–403 µs → 82 µs |

## Core defects and questions found by the measurement
- **Fixed:** a held value at the in-flight bound was never reserved again after a slot freed (`u14b`, row E2-66). The DST never put more than four values on one chain, so it could not see this.
- **Pinned, open for M1:** a held value retried after its instant has passed is reserved and sent late, as a late release is today (`u14c`).
- **Design question for the owner (note 45 §4.5):** the in-flight bound counts a value until its sample evidence, which ties settings capacity to host receive progress. Production would need device capacity (freed at device time) kept apart from disposition (which waits for evidence).

## Fault runs (device loss in the middle of the table)
- **Today's owner:** 260 updates not applied (1 `Lost`, 259 not applied). The stop returned ok.
- **New runtime:** 359 not applied. The brain `Terminated`, the executors exited, the failure flag was set, there was one lost terminal and no panics.
