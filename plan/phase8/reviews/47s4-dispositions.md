# Note 47 stage 4 — transmit values on an empty stream, and B4: dispositions (2026-11-06/07)

Risky (a lifecycle, the transmit clock, the boundary between FakeDevice/MockRadio and the X3x0). Order: brief → implementer's test plan (Haiku 5.5 `anthropic/claude-haiku-5-5#high`, OpenCode2) → orchestrator's rulings P1–P10 → Sol plan review → rulings 1–8 → plan rev 2 → Opus re-check of the plan → rulings on it → plan rev 3 and part (a) → part (b) in sub-steps b1–b6 (the implementer stopped at green points as its context filled) with two self-review rounds (after b1 on free DeepSeek, after b5 on Go DeepSeek once the free quota ran out) → Sol code review (FAIL) → fix round 1 → Opus re-check (PASS) → the orchestrator's minor fixes → gate (mutation 1794/1797) → gate fix round (two rows) → those rows rerun. Records: `47s4-brief.md`, `47s4-test-plan.md` (rev 11; §8 holds deviations D-1–D-48 and the per-round records), `47s4-sol-plan.md`, `47s4-sol-code.md`, `47s4-opus-rechecks.md`, `47s4-gate.json`, `47s4-renames.txt`.

## What stage 4 does

- **Part (a), B4 (note 48)**: a transmit stream has one SampleClock for the Run, registered at `arm` (UR-23). The UHD owner's multi-section plan (`Tx.plan`, `version`, `plan_section`, `retarget`, the mid-stream cut arithmetic in `send`, `refuse_first_send`) and MockRadio's clock list (`clocks`, `cur`, `tx_gen`, `tx_reconcile`) are gone; the only transmit cut is at the origin (an End booked at or before it ends the registry clock there, both Providers; after the origin the registry clock is never ended). No behaviour change.
- **Part (b)**: a transmit channel's `frequency_hz`, `gain_db` and `antenna` are `cold` again; `sample_rate_hz` stays classless (the bench condition). A value is staged only when its stream is not busy at receipt — the shared fold `ezsdr_radio::staging::Busy` (booked ends, a repeat open until a `Stop`, after a `Stop` at most one in-flight window), which both Providers only call (RM-21). A burst's booking takes the snapshot (values, generation, ActionId). The UHD owner applies a newer generation untimed, `rate: None`, in a configure turn of its own, only with no open device burst, after its own last closed device burst's end and outside the recovery; then the burst's late policy decides; the record's `configured` mark holds the generation (arm `Some(0)`, a failed configure `None`, a loss clears it, published only if not lost). A refused configure refuses that burst with its id; the stream does not halt; the burst stays busy to its booked end. MockRadio applies the values at the burst's exact first-sample instant, with history (B6). `radio.timing.tx_start_lead_ns` is removed (schema regenerated). Python: `repeat(x, at, *, frequency=, gain=, antenna=)`, the transmit setters, `sample_rate` raising `Error` with the bench condition. §58 #16 has a transmit form.
- Specs: design/07 (RM-15, RM-19, RM-20, RM-21, RM-25, RM-27), design/09 (MR-18, MR-32–MR-35), design/16 (EA-16), design/18 (UR-9, UR-12, UR-22, UR-23, UR-25, UR-26, UR-36, §3), Vision §27 and §58 #16, KNOWN_LIMITATIONS (KL-11, KL-35, the floors entry).

## Decisions

| # | By | Decision |
|---|---|---|
| R1 | orchestrator | The transmit rate stays refused, so a burst with new values registers no new clock (frequency, gain and antenna do not change sample timing). Note 47 §4 stage 4's open question (a refused configure ends the new clock and the Kernel then refuses the corrected burst) does not arise; it returns with the stage that accepts a transmit rate change. B4 is part (a). |
| R2, P2, P4 | orchestrator | "Not streaming" for transmit is decided at receipt from the booking record (never from hand-over state — #79's mechanism), one scalar per stream, written once in `ezsdr_radio`; after a `Stop` the bound is RM-16's `s + tx_in_flight_ns` (the tightest bound knowable at receipt, re-check O-1). |
| P1 | orchestrator | No Ceiling entry for the Module's configure time: a burst taking a new value is booked at least the device lead (2 ms) after the value; the 15–42 µs configure fits; a descheduled host is S8 (late policy). |
| P3 | orchestrator | The booking record is not updated from the owner's dispatch-time moves; the owner guard keeps the air safe instead. |
| P5, ruling 7 | orchestrator | The device's values are the record's existing `configured` mark, as a generation; equality is by generation (an accepted value equal to the device's still configures once, between bursts). |
| P6–P10 | orchestrator | `repeat` only in Python (no Python `send`); a configure refusal is asynchronous (note 47 §2.4); `tx_start_lead_ns` removed; the two-transmit-stream fixture checked; N47S4-A1 dropped. |
| O-3 | orchestrator | On the re-check's argument, the mid-stream transmit cut code was unreachable (the only cut is at the origin) and is deleted with `refuse_first_send`. |
| m4 | orchestrator | The between-burst configure passes no rate (the bench's T5 covered frequency and gain only); the rate read-back check stays. |
| Sol plan 1 | orchestrator | Where MockRadio and the Module accept different bookings (#79), their `Busy` and their answer to a later value differ; recorded under #79 (stage 5), no known-divergence test. |

## Sol plan review (PLAN NEEDS CHANGES; 4 × P2, 4 × P3; all adopted)

| # | Sev. | Finding | Disposition |
|---|---|---|---|
| 1 | P2 | `Busy`'s inputs differ where the Providers accept different bookings | recorded under #79 (above) |
| 2 | P2 | the owner-guard tests omit the cases where the guard matters | tests (a) moved at dispatch, (b) one sample padded to four, (c) a Stop's held tail at e |
| 3 | P2 | S7's one-shot fault fires at prepare and cannot show a partial configure | a persistent, value-keyed refusal armed after prepare; the two-channel partial case |
| 4 | P2 | Stop, loss and `Provider::stop` during a configure unspecified | probes between channels and before publication; publication only if not lost |
| 5 | P3 | B4's removal inventory incomplete | every user of cut/plan/version given a disposition |
| 6 | P3 | rows with no demonstrated killer | re-anchored on the real admission check; mark asserted `None` |
| 7 | P3 | equal values vs generation | generation-based, deliberately (ruling 7) |
| 8 | P3 | Python validation and spec texts missing | added |

## Opus re-check of the plan (GO WITH LISTED CHANGES; 4 majors, 10 minors; all adopted)

M1 prepare's mark erased by arm's streamer open (an apply before every first burst) → arm publishes `Some(0)`. M2 a `Stop` arriving during a configure did not stop the burst (commands are read at the loop's top) → the configure is a turn of its own. M3 the one-shot fault cannot be refused twice → persistent. M4 B4 would have deleted MockRadio's transmit clock end (in `tx_reconcile`, no test or row pinned it) → kept as a one-clock function with a test and a row. Minors m1–m10 folded in (O-2's placement in `pass`, concrete rows, the receive callers of the removed lead, `rate: None`, units, the ActionId on `Held`, no log-format change, m9's exact boundary, the abort case).

## Implementation

- Part (b) took six sub-steps (b1–b6); each ended green. One run stopped at once, mistaking its own process for a second implementer; the brief then said so and it was relaunched.
- Findings during implementation: **D-19** (b3) — a burst with a newer generation booked at an open device burst's next sample continued it and played the old values; continuation now takes only a burst whose generation the device holds. The second self-review (b5) found a reopen after a loss publishing the mark (fixed, test and row in b6) and five stale spec texts (fixed); its other P2s were the then-unbuilt tests, built in b6; one item (`handoff.md`) was the orchestrator's, fixed at the gate.

## Sol code review (FAIL; 1 × P2, 2 × P3; all adopted)

| # | Sev. | Finding | Disposition |
|---|---|---|---|
| 1 | P2 | MockRadio: a burst starting at a fractional root instant radiated its first sample with the old values (values keyed at the rounded-up tick) | values keyed at the exact instant, held bursts open at the floor tick; fractional-start test (failed before) and rows N47S4-54/-55 |
| 2 | P3 | fail → fix → resend not tested through the Kernel; the resent burst's send not asserted | `FakeFault::TxApplyRefused` public; Session test through admission, same clock, the corrected burst sent; row N47S4-56 |
| 3 | P3 | UR-12 and UR-26 contradict the between-burst configure | rewritten, Changes row |

## Opus re-check after fix round 1 (PASS; 4 minors)

All three closed; the implementer's stated limit of the floor-tick rule (fewer than one root tick per sample) is unreachable (the Mock runs only under the Simulation Engine's 1 GHz root and `ideal` rates are at most 1e9). Minors: m1 UR-12 said only the full configuration reads the rate back (the code checks it on every configure) and m2 a stale test comment — fixed by the orchestrator; m3 (the Session test does not assert the refused value reached the device) and m4 (two steps at one tick give another MR-16 reason text) — recorded, not changed.

## Gate

`47s4-gate.json`: every step PASS (tests 1881, UHD 409, Python 3.14 and 3.9 102 each); mutation 1794/1797: G09 (known survivor), phase7/R07 BASELINE FAILED (its filter named a test part (a) renamed; `rules.py --renames` does not check a `filter`), phase3/M52 survived (fix round 1's floor-tick open moved its old killer off the branch; the branch stays reachable, retargeted to `mr_25_a_stop_transmits_every_held_burst_before_it`). Both rows fixed and rerun alone on sim02: 2/2 killed. Only those two rows' filters changed after the gate.

## Counts

Sol plan 8 (4 P2, 4 P3); Opus plan re-check 4 majors, 10 minors; self-review two rounds (after b1, free DeepSeek: 1 P1 — the unbuilt part, a stop-point record — 3 P2, 3 P3, all fixed; after b5, Go DeepSeek: 10 P2, 7 P3); Sol code 3 (1 P2, 2 P3); Opus fix re-check 0 majors, 4 minors; gate 2 rows. Mechanisms (`plan/maintenance/failure-mechanisms.md`): `planned-state-read-as-actual` — the reopen after a loss publishing the mark (self-review) and plan M1; `device-state-outlives-call` — D-19 (a continuation assuming the device holds the burst's values); `stream-lifecycle-scattered` — plan M2 (a Stop during the configure); `radio-rule-implemented-twice` — Sol plan 1, #79's existing count. Sol code 1 is a MockRadio modelling defect (rounding an instant; none). The gate's R07 is a tooling gap (a renamed test in a row's `filter`).
