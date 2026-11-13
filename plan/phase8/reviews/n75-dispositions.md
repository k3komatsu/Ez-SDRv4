# #75 — review record and dispositions (2026-11-13)

A transmit recovery reopen confirms its flush (T3): an attempt counts only when its send was whole and every channel position ACKed with a tick at or after max(pre-send device time, the old burst's padded end + 2 ticks); up to three attempts on fresh streamers inside one reopen; Stop and loss checked at every boundary; the pre-send time read retried inside the attempt's 100 ms; after the third unconfirmed attempt the stream's clock ends for the Run with one `COMMAND_REJECTED` (`tx_reopen`). The open at `arm` gets the same rule behind `OPEN_FLUSH_CONFIRMED`, off until the 2026-11-15 open-flush sweep. Design note: [n75-options.md](n75-options.md) (owner decisions at its end); Fable's design review: [n75-fable-design.md](n75-fable-design.md). Risk: risky (a lifecycle; the boundary between the fake and the hardware). Branch `work/n75` from `4327a66`, sub-steps A–C and fix round 1, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n75-briefs.md](n75-briefs.md)); test plan [n75-test-plan.md](n75-test-plan.md).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Fable, design | — | — | adopted F1, F4, F5, F6 and its text list |
| Sol, test plan | 5 | 3 | — |
| Opus, re-check 1 (plan) | 2 new (N1, N2/M5) + M5 open | 4 | — |
| Opus, re-check 2 (plan v3) | 0 | 6 + 1 hardware note | — |
| Sol, code | 2 | 2 | — (an interruption check missing after a successful read; a second report-recording path without events) |
| Opus, re-check 3 (fix round 1) | 0 | 2 notes | — |
| Owner, fix round 2 | — | — | the failing time read waits for a read, a Stop, a loss or a 2 s cap (B), superseding N3 |

## Dispositions (all adopted; the decision log follows)

## Sol's plan review (sol-plan.md, FAIL: five majors, three minors) — orchestrator (2026-11-13 ~12:00)
- M1: the owner's "one COMMAND_REJECTED" is the recovery failure's event (one, whatever the number of attempts). The held bursts the ended stream cancels keep the existing stream-end accounting (state exactly what today's `Reopen::Failed` path records and emits for the open and held bookings, VERIFIED with path:line; if it emits a COMMAND_REJECTED per booking, keep that and say so — the orchestrator lists it for the owner at landing). P1b asserts exactly one recovery-failure event plus the existing cancellation accounting, by kind.
- M2: switch off = today's arm and mid-run open exactly: the live streamer kept, today's events and acceptance; tests for missing ACK, zero/short send and failed send at arm with a subsequent usable send, and the mid-run caller.
- M3: commit C5d: an ACK older than the pre-send device time rejected, equal/newer boundary, tickless cannot confirm, the stale ACK retained in the raw record; mixed channel positions; failure or loss of the new pre-send time read.
- M4: gate between attempts outside `device_gate`; publish the loss there before another attempt.
- M5: gates before open and after open / before T3; ordinary TX Stop during recovery with bookings on both sides of its receipt (cancellation order, recovery preserved), claim released, no later attempt.
- Minors adopted: P1a's target later than the retries (expired-target behaviour stays in P3; distinguish the rig's `ManualTimeAuthority` from device-host time); N75-08 dropped or re-aimed; the report-gate row needs an observable blocked-reader checkpoint across the retry boundary; record-boundary tests pin pending → applied/cancelled across the longer recovery and receipt-side reservation checks around an abandoned finite chain and Stop (n99.rs pattern).
- (M1 interpretation, owner 2026-11-13 ~11:45) Confirmed: the 'one COMMAND_REJECTED' applies to the retry attempts (one recovery-failure event); the per-booking tx_burst events of the ended stream (today's Reopen::Failed accounting) stay.

## Opus re-check of the amended plan (2026-11-13 ~12:30): M1–M4 closed; M5 and the P1a/clock minor not closed; new N1–N6 — orchestrator
- N1 (major): the rigs' `core.now()` is an independent ManualTimeAuthority; move it to the device's time inside the `t3_between` probe (as `n99_u7`, n99.rs:225-228) or after the reopen; read elapsed device time from `device.time_now()`; fix P1a, P3, P4.
- N2 / M5: a Stop at G2 (inside `open_device_locked`, after check B): expect one more `tx_open`, no T3 send and `Err`; drop check B (check C covers it) or kill N75-05b with a Stop at `stop_before("tx_close")` of attempt 2 — choose and say why.
- N3 (decided, to keep no new user-visible cause): a failed pre-send time read is retried inside the attempt's 100 ms with a 1 ms back-off (as control.rs:646-660 tolerates); only a read still failing at the attempt's deadline ends the attempt.
- N4: pin the legacy arm: a C11 case with a stale and a tickless scripted ACK at `arm` (accepted today, still accepted with the switch off); anchor the strict rows in strict-only text (P1174, N75-06/07, N48S1-09 checked).
- N5: C5d's negative cases drop the fake's own ACK (`DropAckOfSend(0)` / `DropBurstAck`).
- N6: the floor is max(pre-send device time, the old device front); state the residual if any.

## Opus re-check of plan v3 (2026-11-13 ~12:30): CLOSED (N1–N6), six minors, no new major. Decided by the orchestrator:
- Minor 1 (P1a's last step sends nothing: booked 500 ms ahead, recovery_rig window 200 ms): adopted — move the Kernel clock to ≥ target − W and assert exactly one `at=<target>` send.
- Minor 2 (e3b "reopen succeeds" golden changes anyway): adopted — the plan says so; the check is "new front > old front".
- Minor 3 (KL-44 "retries add nothing to the bound" true on the fake only; a UHD time read holds the control mutex): adopted — KL-44 states the bound as about 3 × (open + 100 ms + one time read), the read's own UHD timeout INFERRED.
- Minor 5 (`assert_no_flush_rows` also matches legacy `arm` rows): adopted — filter to rows that carry `attempt`.
- Minor 6 (G2b relies on the manual clock advancing while the test thread blocks): adopted — G2b runs 20 times; if it cannot be made to pass by construction, fall back to the direct rig and say so.
- Hardware note (a T3 queued behind a still-playing good burst loses attempt 1 with floor = old front; likely common with a 10 ms window): adopted now — the old-front term of the floor becomes the old burst's **padded end + 1 tick** (rejects that burst's own ACK at end + 1, accepts the T3's genuine ACK at ≥ end + 4 samples), §10 item 6; N75-18's `old` text follows. The failed-EOB extra term ((a)) is not adopted (residual non-blocking per the re-check).
- The re-check's condition on "costs one retry" (old front leading device time by < ~100 ms; recovery_rig's 200 ms window can exceed it): stated in §0.2 (ii) and the §5.1 "cannot tell" rows; with the padded-end floor it applies only to the failed-EOB case.
- Minor 4 (persistent time-read failure ends recovery Failed at ~300 ms with "could not be reopened", before control's 1 s loss rule): user-visible cause text → asked of the owner 2026-11-13 ~12:35, implemented as recommended (keep: the reopen fails, KL-44 names the cause) until the answer.

## Sub-step A report (12:51), decided by the orchestrator
- Deviation 1 accepted: old-front floor term = instant(front − 3) + 2 ticks (the burst's own ACK sits at padded end + 1 and the compare is >=; "+1" would accept it). For a closed short burst it can cost a retry, never a false confirmation.
- Deviations 2–4 accepted (Frozen clock holds in exact tests; e3b lock-section golden (true,7)→(true,9); extra rows N75-21..23 and P1a (v)).
- (13:50) Sub-step C accepted as reported (UR-13/KL-12 at the flip; note 44 wording; failure-mechanisms row; §6 test row). One unexplained fake failure in one of ~10 runs: name lost; to be watched at the gate.

## Sol code review (2026-11-13 13:52): FAIL, 2 majors, 2 minors — all adopted (orchestrator)
- M1: check Stop/loss before each pre-send time read, after a successful read (before the T3 send), and before each back-off retry; tests: Stop during a successful read (send must not happen, Interrupted), Stop during back-off (no further read).
- M2: reports stashed in an attempt go through the normal report→event handling (tx.rs:1121–1140 path: `radio.TX_UNDERFLOW { cause: lost }` for SeqError, underflow events, etc.) at the retry boundary, attributed to the old generation and never correlated with the replacement streamer's bursts; tests assert the events, not only the raw rows; N75-11 (or a new row) must die on dropping the event.
- Minor 3: the transient-read test sets its fault window relative to a device-time origin read after `Frozen` is installed (no absolute 100–103 ms).
- Minor 4: KL-44's bound includes the closes (initial, between attempts, final) — "about 3 × (close + open + 100 ms + one time read)", durations INFERRED.
- The failed-EOB foreign-ACK residual stays as decided (re-check: non-blocking).

## Owner decision (2026-11-13 ~15:00): minor 4 = B (the orchestrator's revised recommendation)
- A failed pre-send time read does not count against the attempt: the recovery keeps re-reading (1 ms back-off) until a read succeeds, a Stop, or the device is declared lost (uhd-control's 1 s rule, control.rs `FAILING_FOR`, which a successful read restarts); a cap of 2 s of continuous read failure ends the reopen `Failed` (cause `flush time read failed: …`) in case the loss rule never fires. The 100 ms attempt window then starts after a successful read. Reason: the device's own rule tolerates 1 s of failing reads; recovery must not be stricter (a transient glitch must not end the transmit stream for the Run).
- (15:16, orchestrator) The 2 s cap counts continuous read failure (a successful read restarts it, as uhd-control's loss rule does); reaching it ends the reopen Failed, so the total failing-read wait is at most 2 s. Supersedes N3's 'retried inside the attempt's 100 ms'.

## Gate

PASS ([n75-gate.json](n75-gate.json)) on sim02:8 and sim03:8: 2710 tests, 667 UHD, `hw-build`, Python 121 × 2, e2e 14/14, mutation 2580/2581 (the known survivor `phase7/G09`).
