# e2b-results-4 (fix round 3, 2026-11-03)

All runs on sim03 through `rdev.py`, one at a time, release build. Host load was 16-18 on 32 cores in every run (`*.uptime.txt`): other users share
the host, which matters below. Raw lines are `E2B_BOUND`, `E2B_BOUND_UPDATES` (per-update table) and `E2B_TRIAL`.

## 1. Paired bound diagnostic (`bound-diag.txt`, `run-bound.sh`)
New runtime only; 50 MS/s x 500; 10,000 updates; bound 4 (design) and bound 16, ABBA, spaced then bursty. DIAGNOSTIC: not a verdict, not evidence for the
original design. Bound 16 is the fake's command queue (`FakeConfig::command_queue` 16, `device.rs:413`; checked at `device.rs:1429`, a full queue
back-pressures the apply); the DSP FIFO holds 32 (`device.rs:501`, `1437`) so the queue binds first.

Per-second samples (`bound_diag.samples`) hold the fake backlog (end and 10 ms-reading maximum), the chain's in-flight occupancy (end, max), the
second's retirements and bound rejections. The per-update table gives apply entry and return, the application tick, the retirement.

| kind | bound | failures | p99 acceptance | overflows | seconds with backlog max > 1 ms | of those with a bound hit | bound-hit seconds without an excursion |
|---|---|---|---|---|---|---|---|
| spaced | 4 | 4359 | 592.6 ms | 33 | 29 of 50 | 27 | 0 |
| spaced | 16 | 941 | 22.9 ms | 41 | 31 of 50 | 27 | 0 |
| spaced | 16 | 616 | 21.2 ms | 35 | 25 of 50 | 18 | 0 |
| spaced | 4 | 2734 | 357.6 ms | 25 | 28 of 50 | 22 | 0 |
| bursty | 4 | 4996 | 2938.7 ms | 15 | 15 of 25 | 14 | 1 |
| bursty | 16 | 2266 | 192.3 ms | 20 | 17 of 25 | 13 | 0 |
| bursty | 16 | 2778 | 190.2 ms | 16 | 16 of 25 | 13 | 0 |
| bursty | 4 | 2916 | 1007.1 ms | 9 | 15 of 25 | 12 | 0 |

(Computed from `bound_diag.coincidence_1ms`; failures are all `NotInDsp` plus a few `IngressLate` in the bursty runs.)

What the samples say, plainly:
- Bound hits happen only in seconds with a backlog excursion (one exception in eight runs). Excursions without bound hits also occur.
- The runs are mostly clean for roughly their first 20 s (backlog about 0.06 ms; one spaced bound-16 run already reaches 40.8 ms in second 20), then collapse and stay collapsed: backlog 20-100 ms (100 ms is the fake's
  overrun limit, hence the overflows), with the block rate at 75-90 % of nominal. The onset is at about 4,000 updates applied (spaced, 200 per second)
  and earlier in the bursty runs (400 per second), i.e. when the DSP history is a few thousand entries: see section 2.
- In-flight occupancy tracks the backlog: with bound 16, occupancy is about backlog / 5 ms (35 ms -> 7, 90 ms -> 16), the update spacing. A value is
  retired when R observes its consumption, so a backlog of B ms keeps about B / 5 values in flight: INFERRED-then-seen consistent with the core's
  retirement rule (`core.rs:~626-671`); bound 4 is therefore reached at a 20 ms backlog and bound 16 at 80 ms.
- A larger bound lowers the failures, about 4.6x spaced but only about 1.6x bursty (pooled 7,912 -> 5,044) (bound 4: 2,734-4,359 spaced, 2,916-4,996 bursty; bound 16: 616-941 and 2,266-2,778), and does not remove
  them, because the backlog itself reaches 100 ms. Whatever causes the backlog is the first cause; the bound converts it to lateness.

## 2. FakeDevice scan cost (`scan-h*.txt`, `run-scan.sh`)
New runtime, no updates, 50 MS/s x 500, 50 s, after H timed values were applied straight to the fake (instants far in the past; the fake itself is
unchanged). The fake scans the chain's DSP history per block (`device.rs:548-553`, `freq_constant`).

| H (preloaded) | blocks/s steady | overflows | max backlog | R thread CPU (ticks/s, 100 = one core) |
|---|---|---|---|---|
| 0 | 90,328 | 52 | 97 ms | 83 |
| 2,000 | 87,636 | 61 | 95 ms | 89 |
| 10,000 | 54,679 | 236 | 98 ms | 100 (saturated) |
| 0 (again) | 93,785 | 28 | 83 ms | 81 |

- The host was noisy: even H = 0 lost blocks in episodes of several seconds (blocks/s 65-80 k) in both H = 0 runs, whereas the same condition passed
  at 99,999.98 blocks/s in `e2b-results-3/trial-new.txt` and the first 20 s of every diagnostic run above. So the H = 0 numbers show the host's
  noise floor on this day, not a property of the runtime.
- Against that floor, H = 10,000 is clearly worse: R is saturated (100 ticks/s) and delivers 18 us per block against about 9 us at H = 0 (CPU / rate),
  i.e. the scan roughly doubles R's per-block time. H = 2,000 is within the noise. A linear fit through the two points (9 us + H x 0.9 ns) would cross
  the 10 us budget near H = 1,100, earlier than the observed onset (about 4,000 entries), so the fit is too crude to locate the onset; the scan's
  contribution is VERIFIED in direction, its share of the collapse INFERRED (host noise and held-value retries are not excluded; Sol, sol-e2b-fix3.md).
- FakeDevice was not changed; the preload applies timed values to the fake with `Some(0)` (it clamps them to its time), waiting out its FIFO.

## 3. Loss numbers (B3) (`loss-recomputed.txt`)
The verdict's missing samples are now the sink reader's gaps (leading, in between, trailing), each counted once; publication drops stay separate. In
all 32 saved verdict runs the reader's gaps equal the published stream's and no trailing gap occurs, so no loss number changes and **no loss comparison
flips**; the four verdicts were not rerun. The high condition fails on failures regardless.

## 4. B1 booking identity
The new runtime's `S` tags each apply with the command id (`Timing::tag`), so a request's booking and effective instant are the owner's own
(`ingress.booking_identity` in each run, `owner_booking_effective_e` in the per-update lines). Today's owner exposes no booking identity without a
production change (`Control::book_timed` keeps its seq internal); there a request is paired with an apply by its instant, which the output labels
INFERRED.


## Orchestrator note (2026-11-03)
Today's owner is exposed to the same scan through the same fake receive path. It keeps its settings on time (zero settings failures), not lossless reception: its command slots expire at device time, independent of R (`src/provider/control.rs:659-666,688-689`; Sol, VERIFIED). The new runtime's slots wait for sample evidence (`brain/core.rs:629-674`), so the same receive backlog becomes settings lateness.
