# Review H — the reviewer's report

Adversarial dynamic review of Phase 6 by Claude Opus (AGENTS.md §8), brief [`../prompts/review-h.txt`](../prompts/review-h.txt), run on 2026-09-26 against `d977ad8`…`474082b` in scratch copies outside the repository. Reproduced as delivered, lightly shortened; the triage and the fixes are in [`../implementation-notes.md`](../implementation-notes.md), "Review H".

**Verdict: CHANGES_REQUIRED.** 5 P0, 6 P1, 12 P2. Everything that existed passed (671 Rust tests on 1.85.0 and stable, Clippy clean, 12/12 Python on 3.9 and 3.13, 35/35 listed mutations killed); the defects are where the tests did not reach, and all 14 of the reviewer's own mutations survived.

## P0

**P0-1 — RS-25a's environment rule binds `sim.channel`, `sim.seed` and `sim.faults`.** Spec 17 rejected "environment equal in full" because a sweep over `sim.channel` must be possible, claiming "no admission check reads" it — false: `ezsdr_sim::register` registers `SeedCheck`, `FaultsCheck` and `ChannelCheck` (`crates/ezsdr-sim/src/lib.rs:97-99`). Probe: a child with the coupling gain changed is `Rejected: … section sim.channel is not its parent's, which a registered check reads`; `sim.seed = 7` likewise. §58 #14's environment variation is refused inside a Session; `kf_03_rs_25a_refusals` only uses the doubles. *Fix:* bind only the sections whose check runs at `CheckStage::Runtime` (today `radio.rf_envelope`); fix spec 17's rejected alternative; add a `kf_03` case for a Validate-only section.

**P0-2 — `Rx.capture` can return another request's artifact.** After `capture(1_000_000, timeout=0.001)` raised `CaptureTimeout`, the next `capture(100)` returned shape `(1000000,)` (the timed-out request stays queued, HD-10); after a raw `submit(sink.capture 777)`, `capture(100)` returned `(777,)`. EA-17's "the next CAPTURE_WRITTEN … is always its own" is false. *Fix:* the Sink numbers the requests it receives and puts the number in `CAPTURE_WRITTEN` and `REQUEST_REJECTED`; `capture` waits for its own number.

**P0-3 — `run_child` input sizes that overflow crash the server, and the Session gets no Manifest.** `inputs: [18446744073709551615, 1]`: exit 101, "attempt to add with overflow" (`lib.rs:241`); the Session directory is empty. *Fix:* checked sum; a mismatch is `protocol`.

**P0-4 — The server exits without the Session's Manifest** when the Run ends during `connect`'s advance to T0 (a `sim.faults` `device_lost` at 0: `RunEnded`, directory empty; the Run is dropped and the server does not even exit), when a reply write fails (a reader that stops: "Broken pipe", directory empty), and on any panic. *Fix:* `impl Drop for Server` finishing a live Run and writing `manifest.json`; in `connect`, finish, write, reply `ended` and exit on an advance error.

**P0-5 — `wait_for`'s already-delivered path skips KC-36's lease check and KC-29's Ended-first rule.** After `Stop {}` (`CleanedUp`) `wait_for` returns `Ok(Some(0))` where `advance_to` returns `Err(Ended)`; with an expired Detached Lease it returns `Ok(Some(0))` and the Run stays `Running`. *Fix:* run `advance_to`'s prologue before the already-delivered check.

## P1

**P1-1 — A request that fails to decode kills the Session.** `submit({"kind":"set_paramter",…})` → `ProtocolError`, the server exits (status 2), `close()` raises, `manifest` is `None`, the written Manifest says `client_disconnect`. EA-5 limits exits to framing errors. *Fix:* parse the header to a JSON value, read `body_bytes` and the body, then decode the request; a decode failure is a non-fatal `protocol` error.

**P1-2 — KC-37a takes the child's host clock and admission checks from the caller's Assembly.** The parent's Lease copy's `expires_at_host` is read on the child's (fresh) `SystemHostClock`, so the parent's TTL does not bound the child (probe: parent TTL 1 000 ms expired during `drive`; the child ended `Completed`); and the copied envelope is enforced only if the caller registers its check. *Fix:* give the child the parent's `host_clock` and `checks`; test expiry during a child.

**P1-3 — An early `wait_for` leaves its no-op at the horizon, adding a round later.** `wait_for(…, 8000)` returning at 5000, then `advance_to(10000)`, steps at `[0, 3000, 5000, 8000, 9000, 10000]` where `advance_to(5000)` gives `[0, 3000, 5000, 9000, 10000]`. *Fix:* cancel it on an early return; say so in KC-29b.

**P1-4 — A capture spanning a SampleClock change comes back as one uniform array** (two continuity maps, 1 and 2 Msps, returned as `(20000,)`). *Fix:* `samples()` refuses more than one domain.

**P1-5 — The Kernel tests prove less than KC-29b and KC-37a:** H01 first match → last, H02 empty `kinds` does not advance, H03 the parent's post-child lease check removed, H04 `ezsdr.children` records `seq: 0`, H05 KC-37a's `check_entry` removed — all survive. *Fix:* the missing cases.

**P1-6 — The frontend tests prove less than EA-5, EA-13, EA-14, EA-16, EA-17:** H06 input sizes unchecked, H07 a targeted `Stop` taken as the child's end, H08 `read` accepting a URI that extends a reported one, H09 a bad frame after `connect`, H10 `capture` waiting from event 0, H11 `REQUEST_REJECTED` not raising, H12 `repeat` always resetting channels, H13 `samples` accepting mixed channel counts, H14 `capture` dropping `at` — all survive. *Fix:* a case for each.

## P2

1. EA-3 says `kernel: "<ezsdr-kernel version>"`; the code sends `kernel_api: "4.0.0"`.
2. §57's snippet verbatim captures 2 045 samples of silence first; the carrier sleeps past the repeat's start — say so in §8.
3. A `run_child` whose `duration_ns` overflows the ticks is admitted and runs nothing; refuse it first.
4. HD-16: an artifact whose metadata write then fails gets no `CAPTURE_WRITTEN`.
5. EA-17's timeout is re-armed for each skipped foreign event.
6. `Session.close` after the server exited raises and leaves `manifest` `None` though `manifest.json` exists.
7. `v58_10`'s forbidden list lacks `sim.faults`.
8. `design/03` §10 still defers "the Python Spec builder and its source hash (Phase 6)".
9. Appendix A and `mutations.json` disagree on F05's description and F21's test.
10. `body_bytes` has no limit; record it as a ceiling for Phase 7's listener.
11. `samples()` assumes cf32 whatever the artifact's datatype (INFERRED future risk).
12. `NotSession` is mapped to "submit is available only on a Session" (wrong for `run_child`; unreachable).

## Holes, scope, governance

All five holes confirmed against `d977ad8`. No scope creep; the one reading the plan missed is §58 #14 inside a Session (P0-1). MA-3, PO-4, PO-8, PO-11, GY-2 (116 NEW / 292; no Kernel schema change), EA-18 and GY-5 hold; the three new schemas are in `SCHEMA_CHANGELOG.md`; the Vision is unedited.

## Decisions

S1 supported (§62's trust zones, v3's `SimpleMockClient`, §35's `--retry` VERIFIED; the toolchain argument INFERRED) — keep. S2 sound (P1-1 is the framing's). S3 right mechanism; the payload must identify the request (P0-2). S4 sound; cancel the stale callback (P1-3). S5 sound for Simulation; the child takes the parent's clock and checks (P1-2). S6 works; its explicit-profile companion is blocked by P0-1. S7 sound (the default has no `radio.rf_envelope`; acceptable while simulated). S8, S9, S10 (verified on 3.9.6 and 3.13.15), S11 keep. A1–A8 sound; A2 decodes after the body (P1-1), A4 needs a traversal test (H08), A5 must write on every exit (P0-4).

## Mutations and tests

The list: 35/35 killed. The reviewer's own: H01–H14, all survived. `cargo +1.85.0 test --workspace` 671, stable 671, Clippy clean, Python 12/12 on 3.13.15 and 3.9.6, `check_links.py` 433 links.
