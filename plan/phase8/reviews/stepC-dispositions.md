# Step C (2026-11-09): three load-sensitive tests no longer depend on host wall-clock timing (#83)

Not risky (tests only, plus one observability row the owner chose). Implementer: a Claude Haiku 5.5 subagent. Reviews: Sol (owner's request: "一応SolにReviewさせて"), then an Opus re-check. Brief with the owner's decisions: [stepC-brief.md](stepC-brief.md). Gate: [stepC-gate.json](stepC-gate.json) (PASS).

## What depended on wall time and the fix
- `rx_start_lead_group_runs_on_the_fake`, `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements`: a wall-clock FakeDevice's 5 ms configure delay, a Rx SampleClock read before uhd-rx registered it, a 2 s wall wait. Now a manual clock and `advance_until` bounded by an absolute device-time deadline (and ending when the Run is cleaned up). 20/20 under a 48-hog load (the old code failed once each under the same load).
- `s15_parity_untimed_booking_after_stop`: the UHD Stop's sample was taken from the submitted tick, and the repeat's receipt instant depends on the test thread. Owner decision (a): uhd-control records the applied transmit Stop as a Manifest timing row `tx_stop_applied` {stream, at, bound} (UR-30) — a genuine observability record. Owner decision (B) after Sol's P2: the parity test asserts only the UHD Stop region (10 003..10 004 samples from that row), the Mock's exact gap, and that the repeat starts no earlier than the bound; the exact placement (receipt inside the region → the bound; after → the ordinary start of its receipt) is pinned per Provider by manual-clock tests (`ur_21_an_untimed_repeat_after_a_stop_starts_at_the_bound_or_its_ordinary_start`, `mr_16_untimed_repeat_after_stop_starts_at_the_stop_region_end`, and the pure `place_untimed`/`prune` tests). 20/20 under load and 20/20 with a 15 ms host sleep injected before the repeat (the old exact check failed with it).
- Rejected: widening the UHD band by 1 ms; receipt timing rows in both Providers; a device-time interval around the push; routing through the Kernel.

## Findings
| Source | Findings | Disposition |
|---|---|---|
| Sol, code | 1 × P2 (the parity verdict still depended on the test thread between the Stop and the repeat's receipt), 1 × P3 (`advance_until` bounded a count, not device time) | P3 fixed; P2 by owner decision (B) |
| Opus re-check | PASS WITH MINORS: m1 the Mock gap can be exact; m2 a comment overstated the Mock's pins; m3 `advance_until` could hang after a manual-clock Run ended | all fixed |

Mutation rows: new C83-1 (the timing row's bound), C83-2 (untimed placement inside a Stop region); re-anchored N47S4-17c, N47S5-24; N47B-19, N47B-36, P824 killed by the rewritten tests.

Mechanism: `wall-clock-test-oracle` (#83), the tests #60 did not move.

## Tooling note
Sol (OpenCode2, `--standalone`) took more than 4 minutes to print its first line three times in a row: OpenCode snapshots the working tree at start-up. A short prompt from `/tmp` answered at once. The "about 4 minutes at 0 bytes → stop and retry" rule in handoff §5 is too short for this repository; look at `~/.local/share/opencode/log/opencode.log` before stopping a run.
