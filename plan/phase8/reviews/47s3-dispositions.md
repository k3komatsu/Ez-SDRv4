<!-- rulings-s3-plan.md -->
# Orchestrator rulings on test-plan-s3.md rev 1 §0 questions (before Sol's plan review)

1. D8 (§58 #16 receive form = antenna outside `antenna_ports`, refused at admission; the frequency-band form waits for stage 4): accepted. No new receive frequency band in RM-19.
2. S8 wording: accepted. The Kernel keeps the generic RS-52 text; the Python `Error`, RM-27's row note and KL-35 name the bench condition.
3. D2 (streaming is a booking-time fact; after a refused start the user sends `Stop` before a corrected value): accepted; document in EA-16, KL-35 and the refusal text.
4. D3 (a channel in no stream, and an update carrying `at`, are refused): accepted.
5. D4 (a slow configure is a late start, reported with LATE_COMMAND and GAP_BEFORE; the origin is not moved by the configure): accepted. It satisfies note 47's requirement that the configure time never silently eats the start lead. MockRadio's configure takes no time, so it never reports this; record it as a parity limit in the plan.
6. D7 (`Submitted.refused`; Ended instead of a reply when the Run is no longer Running): accepted.

<!-- dispositions-s3-plan.md -->
# Dispositions: Sol's review of test-plan-s3.md rev 1 (sol-s3-plan-final.md)

| # | Finding | Disposition |
|---|---|---|
| M1 | A Spec `schedule` of Stop → Set → Start cannot stage: scheduled updates carry `at` (event.rs:929–933), which D3 refuses, and timed schedule entries are dispatched together at startup while Stop enters the runtime agenda (pipeline.rs:337–344, 747–787) | Adopt: the Kernel boundary stays; in this stage a scheduled receive value update is refused (D3) — test and document that (B3 rewritten to assert the refusal; EA/RM text and KL-35 say values change through Session actions only). Successful scheduled staging is out of scope |
| M2 | D4 as written lets the configure silently consume part of the lead (configure ends at b+25 ms for an origin at b+50 ms with a 50 ms lead: no report) | Adopt, overturning D4 as written: at the owner's configure completion, if completion + `rx_start_lead_ns` > the planned origin, the Provider reports it explicitly (an event with the stream, the origin, the completion and the lead left — reuse `radio.LATE_COMMAND`'s shape if it fits, otherwise say why) and still starts at the planned origin (the clock-origin contract and Mock parity stay). Tests: the partial-consumption case (report, origin unchanged) and the configure-past-origin case (today's missed start). MockRadio's configure takes no time: a parity limit, recorded |
| M3 | No test of `Rx.start` stopping on a Provider refusal of an admitted value | Adopt: real-Mock sequence (streaming stream → `rx.start(frequency={0: f2, 1: f3})`): `Rejected` with the event and the first Action's id; no second update, no `start_rx` sent. Keep the Kernel-rejection S9 case |
| m1 | The configure-once observation bypasses `Owner::Nothing` (late-start retry goes straight to `rx_start`) | Adopt: call `owner_configure` twice for the same segment (one configure), then a new command/snapshot (another); keep existing killers until this one is shown to reach `owner_step` |
| m2 | O2's early-return fallback not tested with an interrupted wait | Adopt: gated Provider — dispatch, pause before booking finishes, freeze/end, release; no successful `Submitted`; and submit cannot complete before the gate opens |
| m3 | Some mutation rows impossible or with unproven killers (N3-10, Mock values at booking vs origin, the event-kind filter) | Adopt: compiling single-behaviour replacements with distinct killing observations; drop equivalent ones |
| m4 | H2 cites a superseded INFERRED label (note 39 header marks stale overwrite VERIFIED; `ddc_block_control.cpp:650–660`); the held-open acceptance gate is rate-specific | Adopt: correct H2; state separately what retune bench verification remains; read-back does not imply RF settling |

<!-- rulings-s3-plan2.md -->
# Orchestrator rulings on test-plan-s3.md rev 2 questions 7 and 8

7. **M2's threshold.** RM-25 / spec 07 line 81 define `rx_start_lead_ns` as the lead "from the instant its Provider is ready for it to a new segment's first sample". With the origin planned at booking (`ready: now`) and the configure run afterwards in the owner, comparing against the full start lead fires on every restart. Ruling: **the receive start lead counts from the booking and must cover the start's configure** (spec 07's definition of `rx_start_lead_ns` changes accordingly for receive restarts; the restored receive start-lead bench group measures restart-with-configure, so the profile value includes it). The Provider **reports** when the configure leaves less than the minimum timed-command lead before the planned origin (`origin − completion < min_timed_command_lead_ns`), i.e. when the stream command can no longer be issued with the device's own lead; the origin never moves. Reasons: keeping the origin fixed at booking keeps the plan identical in MockRadio and the UHD Module and needs no re-planning after booking (the machinery stage 2 deleted); the report still makes a configure that eats the lead visible before the device reports a miss. Rejected: moving the origin to completion + lead (HEAD's form; re-plans after booking and makes the plan depend on the host's timing); the full-lead threshold (fires on every restart).
8. **Event kind.** Accept a new `radio.START_LEAD_SHORT` (Warning, `continue`) with the stream, the planned origin, the configure's completion and the lead left; `LATE_COMMAND`'s `applied` and its `mark_artifact` reaction do not fit an on-time start. Schema regenerated, `schema_freeze` green.

<!-- dispositions-s3-plan2.md -->
# Dispositions: Opus re-check of test-plan-s3.md rev 2 (opus-s3-plan2-recheck.md)

| # | Finding | Disposition |
|---|---|---|
| R7 | Rev 2 still uses the full-lead threshold; ruling 7 needs plan changes 1–7 | Adopt all seven: `lead_left` with `min_timed_command_lead_ns` ("the declared command lead, RM-6"); boundary tests (== no report, −1 report; D = 25 ms no report kills a full-lead mutant); partial test numbers from the lattice origin (46/45/25 ms); `SlowApply` via the existing `probe(&core, "apply", …)` hook moving both clocks; the loose fake test asserts SHORT or LATE ≥ 1; payload `min_lead_ns`; rows N3-35 `<`→`<=` and a "threshold = start lead" row; the bench group fails on LATE or SHORT. Wording: spec 07:81 keeps "from the instant its Provider is ready" and adds "and covers the start's configure" (ruling 7's "counts from the booking" is corrected: after a cut, ready is the later of the booking and cut + window + one receive call + allowance) |
| D2 | Major: an arrival-order streaming flag disagrees with the timeline — `Stop` → `start_rx(at = now+1 s)` → `Stop` → set f2 is accepted while the start still begins at +1 s with f1 | Adopt the recommended fix: "streaming" is the line's final state after replay (`on`), not a separate flag; the sequence becomes a test; generated `StartRx` gets an `at`. Rejected: a Stop that cancels pending starts (changes RM-21 and parity) |
| m2 gap | `ThreadedProvider` has no hold-after-take knob | Adopt: the plan names the new test knob |

<!-- dispositions-s3-code.md -->
# Dispositions: Sol code review of note 47 stage 3 (sol-s3-code-final.md)

| # | Finding | Disposition |
|---|---|---|
| 1 (P2) | A surviving earlier start is skipped: stop → `start_rx(at=+1 s)` (uhd-rx queues it) → `start_rx(at=+200 ms)`; replay keeps the earlier start, uhd-rx's cursor is past it, capture receives nothing though the line is on | Adopt as a defect, fixed by **refusing** rather than reconciling (orchestrator, note 47's reduction): a `start_rx` on a stream that is streaming after replay (`Line::streaming`, which includes a booked start not yet begun) is refused by both Providers with `COMMAND_REJECTED` (kind `command`, the Action's id, a reason naming the rule); the line is unchanged. So a booked start can never be revised to an earlier origin. RM-21/MR/UR text and EA-16 say it ("send `Stop` after the start has begun, then `start_rx`"). Regression test: Sol's sequence on the fake (second start refused, the +1 s start delivers). Restore timed starts (`at`) in differential layer 3 and the generator, now that no revision exists. Rejected: reconciling queued-segment identity and the begin cursor with the surviving plan (adds re-planning after booking, the machinery stage 2 deleted) |
| 2 (P2) | Negative channel numbers index from the end in `session.py:365–374` | Adopt: reject negative (and non-int) keys before indexing; test asserts nothing submitted |
| 3 (P2) | An immediate booking refusal (lattice overflow at `at.ticks = i64::MAX`) carries no Action id, so `Submitted.refused` misses it | Adopt: pass the dispatched id into every immediate booking failure (`core.rs:530–533`, `control.rs:315–316` and any other `command_rejected(None, …)` reachable from a submitted Action's booking — search them); extend the overflow test through `Control::book` and the submit reply (`Rejected` with the id) |
| 4 (P3) | The receive start-lead bench group is not restored | Disposition: named follow-up, not this step — the bench step (note 47 work order ③) restores and runs it before any profile `rx_start_lead_ns` is changed; until then hardware may report `START_LEAD_SHORT`. Recorded in note 47 §4 stage 3 and KL-35 |
| 5 (P3) | S7's tests narrower than planned (LateCommand injected; no SHORT→LATE; no first-block gap/lost observation) | Adopt: the deterministic SHORT→LATE case on the fake with the fake producing the late command, observing the first block's gap and lost count; record any remaining omission in the plan |
| 6 (P3) | Spec edits: RM-21 still says the Kernel refuses every scheduled radio value; MR-34's old validate-refusal citation; SB-30 says runtime checks see no radio updates; EA-16 lost unrelated rows | Adopt: correct those; restore EA-16's unrelated rows from HEAD |

