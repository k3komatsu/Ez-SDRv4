# Step F (2026-11-10): the front after a closed device burst is time-based; transmit rates start at 1 MS/s (#81, #82)

**Risky**: a booking rule both Providers call, the boundary between FakeDevice and the X300, the transmit rate grid, and user-visible limitations. The direction was decided by the owner from the bench of 2026-11-10 ([bench-results.md](../bench-results.md) "2026-11-10"), so AGENTS §9 step 0's design draft was skipped and Sol's test-plan review also attacked the rule.

People and order of work:
- Implementer: Claude Sonnet subagents (owner, 2026-11-10; self-review waived). One wrote the test plan and F1; a fresh one did F2 and the fix rounds.
- Reviews:
  - Sol on the test plan (FAIL), then an Opus re-check of the amended plan (FAIL, one major, closed in the plan's Revision 3 without another re-check);
  - a cheap Haiku review of each sub-step;
  - the full mutation run before the code reviews;
  - an Opus review at effort max (FAIL), which the owner asked for before Sol;
  - Sol and Fable 5.2 on the code, with the same prompt (both PASS WITH MINORS). The owner asked for Fable 5.2, run headless through `claude -p --model claude-fable-5-2`, as a comparison.
- Sub-steps on a local work branch: F1 (the record and the grid), F2 (uhd-tx, FakeDevice, hardware tests, bench RF text).

Records: brief [stepF-brief.md](stepF-brief.md), test plan [stepF-test-plan.md](stepF-test-plan.md) (Revisions 1–3), renames [stepF-renames.txt](stepF-renames.txt), gate `stepF-gate.json` (added after the gate).

## The decisions (owner, 2026-11-10)
- After a standalone closed device burst of at most 32 transmitted samples, the next safe start is the closing send's exclusive end + `g(rate) = max(3, ⌈15 µs × rate⌉)`; after any other close, + 3. One rule for every profile and rate. Rejected: a per-interpolation table, and a guard only at interpolation ≥ 80 (owner: the case arises only for bursts of ≤ 32 samples; a simple rule beats many corner cases).
- Transmit rates on `x3xx` are at least 1 MS/s (decimation ≤ 200), both Providers; receive unchanged (KL-40). A floor at 0.5 MS/s was rejected (only coarse data).
- Bench transmit may go up to 200 MS/s on the cabled loopback (the owner's coax and attenuators are rated to 18 GHz). The bench RF envelope was widened to ±100 MHz and **reverted the same day** to ±1 MHz per transmit channel: RM-19 checks only the transmit centre, so the rate needs no wider band, and the narrow band is the automatic guard against a wrong retune (Fable 5.2's note; Opus had made the same point at the plan re-check, and the orchestrator fixed only the wording then).

## What changed
- `ezsdr_radio::staging`: `closed_short_gap(root_hz, ticks_num, ticks_den)` (integer arithmetic) and `CLOSE_GAP` = 3, with `HELD_TAIL_FRONT` = 4 and `STOP_FRONT` = 5. `Bookings` carries its stream's gap; it has no `Default`; it is armed where the transmit clock is registered (UHD `Control::new`, Mock `prepare`); an unarmed record refuses a short head. `stop_sample` and the handed bound use `STOP_FRONT`.
- `x3xx::tx_rates` (decimation ≤ 200, ascending), `Description.tx_rates`, `Direction::grid`; transmit coercion and capabilities use it; UR-13 and raw measurement keep the 639-value device grid.
- uhd-tx: a closed short head fronts at end + g, any other close at end + 3, a two-zero close at `close_start` + 5, the flush at + g, the abandon at cursor + 2 + g (stricter than the record's cursor + 5; a booking there is judged by its late policy at dispatch, KL-39). One helper `core::stream_gap` serves the owner and the record.
- FakeDevice: guard end + g × ratio after a closed burst of ≤ 32 samples (every sample the host sent in it, refused or dropped included), end + 3 × ratio after any longer close, at every ratio; its own copy of the values.
- Hardware tests take the transmit rate (1, 2, 2.857 142 857, 10, 100, 200 MS/s), use time-based leads, and include a 100/200 MS/s long close whose shape (held tail plus one zero, B three samples after) is checked. `raw_tx` allows up to 200 MS/s.
- Text: RM-8, RM-15, RM-16, RM-29, MR-3, MR-38's row, UR-22, UR-23, UR-33, KL-13, KL-36, KL-38, KL-39, new KL-40, `plan/phase7/bench.md` and `plan/phase8/bench.md` RF safety.
- Mutation rows N49F-01…N49F-27 added; about 45 rows re-anchored or re-filtered.

## Findings
| Source | Findings | Disposition |
|---|---|---|
| Sol, test plan | FAIL: M1 the transmit grid was specified descending (`Grid::Values` is ascending); M2 raw hardware tests cannot exceed 2 MS/s (`raw_tx`), N-H1's lead was a sample count (100 µs at 200 MS/s); minors: non-integral clock ratio, recovery at several rates, re-arm, two streams, three mutation mappings, a 200 MS/s rig's front, text (KL-13, RM-16 B−5, stale "none named") | all adopted; M2 led to the owner's 200 MS/s decision |
| Opus re-check, plan | FAIL: widening the envelope put the Rust B7 retune (hz + 100 MHz) inside the band; minors: KL-13 gap 4 not 3, an abandon geometry the fake cannot build, two missing rows, a bundled row, RM-19 checks only the centre | all adopted (Revision 3) |
| Haiku, F1 | P1 two rows' filters named renamed tests; P2/P3 a missing row, a join case, per-probe identity, an unarmed rig, KL wording | adopted except: a UHD variant of a row (N49F-18 exists), the Python test name, compile-forced hardware edits, a device-side KL line (F2) |
| Mutation, F1 rows | N49B-15 compile error, N47S5-14 survived (its last probe was refused both ways under the wider front) | fixed (probe at 146) |
| Haiku, F2 | P1 the B7 retune to 2.4 GHz lay inside the CBX's widened band (union check, RM-19); P2 size-class wording, stale KL-13 status, a dead constant, bench text | adopted (the P1 case later moot: envelope reverted) |
| Full mutation run 1 | 8 rows: one compile error (`Bookings` has no `Default`), one stale filter, six survivors whose probes the wider fronts no longer separated | fixed from the Opus review's counterexamples |
| Opus (max), code | FAIL: M1 the same 7 rows; M2 the Module-path T2 test never ran 100/200 MS/s (a false link premise) and its calibration burst was too short there; residual risk: + 3 after a long close is zero margin at 200 MS/s with a 20 000-sample first send; minors: the abandon-front booking test, "never looser" below 1 MS/s, stale text, two duplicated decisions, an overclaiming test name | all adopted; a 100/200 MS/s long-close bench test added |
| Sol, code | PASS WITH MINORS: the two-stream parity test accepted wrong refusals (consecutive ids only); the long-close test did not check its close shape; the Python B7 at 2.4 GHz inside the widened CBX band | all adopted (the third moot after the revert) |
| Fable 5.2, code | PASS WITH MINORS: the bench procedure still stated stage 5's values; UR-22's "margin, not a measurement" false at 200 MS/s; an assertion that observed nothing; a stale MR-38 row; note: the envelope widening was unnecessary and weakened the guard | adopted; the owner reverted the widening |
| Orchestrator | `hw_b8_burst_one_sample_after_a_burst` still expected end + 2 to play after a 30 000-sample burst (now refused; the front is end + 4); its fake rehearsal booked only end + 1 | fixed (end + 3 refused, end + 4 plays; the rehearsal books end + 1, + 3, + 4); an audit of every hardware test found no other stale expectation |

Comparison of the two code reviewers on the same prompt (owner's request): Sol took about 5 minutes and found three test-observation gaps; Fable 5.2 took about 20 minutes and found five minors plus notes, reaching spec text, the bench procedure and the RF decision. They overlapped on one finding.

Mechanism: the step's cause is a measurement gap (stage B's + 10 was a 1 MS/s value applied to every rate; `bench-results.md` "2026-11-10" corrects the 2026-11-09 entry), not a design mechanism; no `mech:` label is assigned. The envelope widening is an orchestrator error in a recommendation to the owner (a wrong premise, occupancy versus centre), caught by review.
