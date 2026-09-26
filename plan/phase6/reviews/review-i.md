# Review I — the reviewer's report

Re-review of Review H's fixes (commits `79c7655`, `69883ef`, `2b607a2`) by Claude Opus (AGENTS.md §8), brief [`../prompts/review-i.txt`](../prompts/review-i.txt), 2026-09-26, in scratch copies outside the repository. Reproduced as delivered, lightly shortened; the triage is in [`../implementation-notes.md`](../implementation-notes.md), "Review I".

**Verdict: CHANGES_REQUIRED.** 1 new P0 (Review H's P0-2 not closed), 3 P1, 7 P2. The suite passes (682 Rust tests on 1.85.0 and stable, Clippy clean, 19/19 Python on 3.9 and 3.13), and all 60 listed mutations are killed.

## Review H's findings

P0-1 closed: §58 #14's variation works in a child through the server (`sim.seed`, a coupling gain, a `device_lost` fault each admitted and run); the RF envelope cannot be left (a derived child tuned outside it fails `validate`; dropping or widening it is refused by RS-25a); an added section is judged by the parent's checks. **P0-2 not closed** (P0-A). P0-3 closed. P0-4 closed through the binary (a fault at 0, a closed reader, all eight framing exits, EOF). P0-5 closed. P1-1 closed (every `body_bytes` shape framed or refused; an undecodable frame's body consumed), but see P1-B. P1-2 closed. P1-3 closed (the engine cancels by `(ticks, seq)`; a Run ending in the matching round is not cancelled; N11 `<=` is equivalent). P1-4 closed for a rate change, not for a gap (P1-C). P1-5 closed; H05's equivalence argument is sound. P1-6 closed; H07 is killable by a timeout (not "untestable"), H09a and H10 are now equivalent, and H13 is reachable (a capture across `rx.channels` 1→2) but equivalent because each count has its own domain — the notes' "unreachable" is wrong. P2-1…P2-12 closed (P2-3's `i64::MAX / 2` bound unexplained and untested; P2-5's Python half untested).

## New findings

**P0-A — `Rx.capture` still returns another request's artifact.** With one recorder bound the Kernel routes any `sink.capture` to it whatever its target path (RS-14), and a `SetParameter` of `sink.capture_samples` on `sink/rec` reaches it too; `Session.submit` counted by the target's path, so `submit(sink.capture, target radio/rx)` followed by `capture(100)` returned `(777,)`, and `set("sink/rec", "sink.capture_samples", 555)` then `capture(100)` returned `(555,)`. EA-17's "the count of `sink.capture` entries the Session admitted on that recorder" is false for both. *Fix:* count what the Kernel routes: an admitted `sink.capture` for the only recorder when one is bound, else for the one its target names; an admitted `set_parameter` of `sink.capture_samples` on `sink/<recorder>` too; test both.

**P1-B — an undecodable first frame no longer ends the handshake.** `{"request":{"op":"nope"}}` then `hello`, `connect`: connected. EA-3: any first request other than `hello` gets `protocol` and an exit. *Fix:* on a decode failure before `hello`, reply `protocol` and exit; add the case to `ea_03_handshake`.

**P1-C — a capture across a gap is one array that hides the jump.** With an `rx_overflow` fault at 2 ms, `capture(5000)` returned `(5000,)` whose samples 1 999 and 2 000 are 50 ms apart (one map, a 50 000-tick `overflow_restart` gap, two valid segments). §23: gaps are flags plus time jumps. *Fix:* `samples()` refuses a map with `gaps` or `channel_gaps`, or more than one valid segment per channel; test with an `rx_overflow` fault.

**P1-D — two Python halves untested.** N05 (count Kernel-rejected captures too; reachable with `params: {}`) and N06 (re-arm the timeout per skipped event, P2-5's client half) survive. *Fix:* a Kernel-rejected capture before a capture; a foreign event inside a short timeout that must still time out at the first deadline.

## P2

1. A `Stop` for `sink/rec` discards queued requests with no event, so `result()` waits out its whole timeout.
2. `request_frame`'s `within_ns` description reads "…; or else" (one doc comment split across two fields), and "exactly one" is not in the schema.
3. N01 survives: RS-25a frees "prepare-stage" sections, but no test has a prepare-only check.
4. N09 survives: the `ticks <= i64::MAX / 2` bound has no stated reason and no test.
5. N10 survives the server's tests (Python kills it): `ea_12` compares the echoed `horizon` only where it equals `now`.
6. The "Checked by" lines of EA-17 and KC-29b list only the pre-Review H tests.
7. When `manifest.json` cannot be written, `finish` replies `io` without the Manifest.

## Mutations and tests

The list: 60/60 killed, none by timeout. Review H's re-run: H01–H04, H06, H08, H09b, H11, H12, H14 killed; H07 killed by a 300 s timeout; H05, H09a, H10, H13 survive, equivalent. New: N02, N03, N04, N07, N08 killed; N01, N05, N06, N09, N10 (server tests only), N11 (equivalent) survive. `cargo +1.85.0 test --workspace` 682, stable 682, Clippy clean, Python 19/19 on 3.13 and 3.9, `check_links.py` 437 links, `ov_23b` 116 NEW / 292.
