# #99 — after a finite burst is abandoned, receipt and dispatch disagree on whether a booking joins: test plan (v2, direction B)

Implementer's test plan, written before any code (AGENTS §9 steps 0 and 2), amended after the owner's direction (B) and Fable's design review. Base: worktree `/home/komatsu/local_work/Ez-SDRv4-e3`, branch `work/n99` at `cc8c3cb` (main). The first plan was written on `44eb9b2`; `git diff --stat 44eb9b2 cc8c3cb` shows two plan files only, so the code and every `path:line` of the first plan still hold (VERIFIED). **Nothing in the worktree was edited, formatted or built** (`git status` clean at the end). Every probe ran in out-of-tree copies, `/tmp/n99-probe` and `/tmp/n99-base` (`rsync -a --no-times --exclude .cargo --exclude target`, timestamps refreshed, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`), deleted at the end. Kept outside the repository, in `~/.cache/ezsdr-fixes/2026-11-12-n99/probe/`: `reference-sketch2-production.diff` (the production change, 70 changed lines), `reference-sketch2-tests.diff` (the edits to existing tests and the pure tests), `n99.rs.txt` (the rig tests U1–U11 as they ran), `n99-rows.json` and `n99-mut.py` (the rows and the debug-copy loop of §5.4), and the first plan's probes. The first version of this plan is `test-plan-v1.md` beside it.

Units: samples are indices on the stream's SampleClock. The UHD frozen rig (`switch_rig_with`, `tx.rs:6680`) runs 1 MS/s, 200 root ticks a sample, window `W` = 10 000 samples, device lead 2 000 samples, `g` = 15; `J(e)` = `instant(e)` − 5 ms. "Record" is `Bookings` (`crates/ezsdr-radio/src/staging.rs`), "chain" its `Chain`, "device chain" `DeviceChain` (the executor's actual burst state). Claims are VERIFIED (I ran it, or an existing test asserts it) or INFERRED (read from code only).

## Changes after Sol's review (sol-plan.md, PASS WITH CHANGES; all three minors adopted by the orchestrator, 2026-11-13)

- **m1, the halt exception is narrowed.** The guard (`TxList::halted()`) reads a halt *unread in the list*, so it covers exactly "a finite abandon does not rewrite the record a halt already ended"; once the owner has read the halt, a new finite booking and its abandon end that booking's join as any other's. RM-15 and UR-23 say it so (§5.6's "an abandon that follows a Stop … changes nothing" is replaced by "does not rewrite what a Stop, Provider::stop or a device loss already ended in the record"). P9 (`txbook_a_finite_abandon_after_a_halt_changes_nothing`) gained the second half: a booking received after the Stop, abandoned with no halt unread, ends its own join and leaves the Stop's chain as it was. Repeat abandons are untouched.
- **m2, KL-39's front.** The workaround names the retained RM-15 front: a finite burst's end plus four, or for a standalone burst of 32 samples or fewer its padded end plus `g`, or a joined-behind burst's own front; example: one sample at 10 000 holds up to 10 019 at 1 MS/s. §5.6's "four samples beyond the end" for every case is withdrawn.
- **m3, a composition test.** `n99_p8_successive_abandons_leave_each_footprint_and_the_last_piece_alone_moves_or_leaves`: A(40)+B(100)+C(10); a move of C while it is joined is refused (D24); abandon A, then B; the verdicts are ` ERR@0 ok@165` (A's 44 and B's 144 held, C's own front 165); a drop of C gives ` ERR@0 ok@144`; `move_untimed(140, 140, Some(10))` gives `Some(144)` and ` ERR@0 ok@169`; a booking at B's end is refused.
- `end_finite(e)` without `k` is kept (Sol: the end identifies the chain without prunable join points). The implementer's local mutant loop (`n99-mut.py`) is not gate evidence: the rows ran through `remote_mutate.py` (`impl-mut.log`, `impl-mut2.log`).

## Changes after the owner's direction and Fable's review

What differs from `test-plan-v1.md`, in the order of the brief; the body below is the plan to implement.

1. **Direction and risk.** Owner: direction B is the plan (the abandon of a finite booking is an owner's close for the record; KL-39's footprint kept; a booking at the abandoned burst's end is refused with UR-21). §3's options table is kept as the record of the rejected options only. **B+ (a grow-only front for a late abandon, P4) is not included**: P4 stays as P3 does, judged by its late policy at dispatch. **The dispatch stat is not included.** The risk class is **risky** (v1 said "not risky"; F1 is a lifecycle fact about a halt and an abandon): steps 2 and 3 of §9 apply, and §5.7 stages the work in three sub-steps.
2. **F1, the guard (§4.2).** *Chosen: Fable's check, in the book's own list, plus one read in the Provider's section for a lost device.* `TxBook::abandon` writes nothing for a finite booking while a `Halt` is unread in its list (`TxList::halted()`, 4 lines), and the Provider passes no end when `core.is_lost()` is read inside that same section (a loss pushes no halt: `core.rs:659-664`). Evidence (probed on the reference sketch; §4.2 and Appendix A): (a) **Fable's exact pure sequence is already closed by the way F2 finds the chain** (`e < c.front`: a chain a halt cut has its front below the abandoned booking's end), so the guard is not what protects *that* sequence; (b) a halted chain whose front stays above `e` (the whole chain ends below the Stop's bound) is revived without the guard, in pure form (F1b, F1c) and on the rig: a Stop received while the owner waits in a send, a Stop unread when a report abandons, and a loss (R1, R2: a booking at 12 014 is accepted after the Stop and refused after the abandon); (c) the rig reaches (b) only when the owner's call outlasts the window (`SEND_TIMEOUT` is 1 s, `tx.rs:132`) or the owner lags; in normal timing the abandoned booking ends within a chunk of the Stop's bound (INFERRED). **Rejected:** `join == false` as the test (INFERRED: a chain `remove_head` left has `join == false` and pieces behind it, and the abandon of its first piece must still frame them); a `halted` flag on `Chain` (a second representation of what the list's unread halt says); clamping the abandon's fronts to the halt's (a raise to the pieces' own fronts is right when no halt came); writing at `complete` (misses the report-driven abandons). **What the guard must not change** is listed in §4.3 and pinned by tests.
3. **F2, `end_finite` keyed by the abandoned piece.** `Bookings::end_finite(e)` finds the chain that holds the abandoned piece's last sample `e − 1` *below its front* (`c.start < e && e <= c.end && e < c.front`), not by `c.end == e || joined.contains(e)`, and splits behind the piece at `e` itself. **Signature note for the orchestrator:** the brief and Fable write `end_finite(k, e)`; I kept their semantics (keyed by the piece, not by the join list) and dropped `k`: the waiting device burst's abandon has no open booking and so no `k`; `e − 1` names the same chain; and the piece boundary is `e` by contiguity (a piece joins only at the end of its chain), whether or not a receipt's `prune` dropped that join point from the list. Fable's two sequences are pure tests (P5: the prune; P6: the stale halted chain) and the first is a rig test (U9). Adding `k` back is one parameter if the orchestrator wants it.
4. **S9 corrected, and every sequence re-derived** (§5.1 gives each expected result as the amended sketch produced it, with today's in brackets). S9: after an abandon, a **Stop between the abandoned booking's start and a framed piece's start now leaves the region `[t, B)`** (RM-16: the piece is a standalone booking the Stop cancels; before, it was a joined piece of the abandoned chain and left none): Stop at 11 ms with A at 10 000 and B at 12 000: receipts `E11900..=21004 .21005..=25099` (today `E11900..=12013 .12014..=25099`). "RM-16's one-section rule covers the abandon" is not claimed (§3.3).
5. **Hardware.** The abandons that hardware produces promptly are the reports (UR-28: underflow, underflow in packet, sequence error, sequence error in burst: `tx.rs:1129-1141`) and a send that returns `Err` at once (`tx.rs:662-674`); they are the primary rig cases (U2's first five rows, U8, U9; the `FailSend` fault for the error). A short send comes back only after `SEND_TIMEOUT` = 1 s (`tx.rs:601, 651`: the note "a send did not complete within 1 s"; UHD's behaviour on a full queue is INFERRED), so its abandon lands a second of device time after its plan: U7 and R1 model that wait with a 0.5 s device-time jump inside the send, which is where F1 lives. No hardware test changes (§5.3); no bench value is adopted.
6. **Text** (§5.6): RM-15, UR-23, KL-39 and the KL-38 cross-reference, each sentence quoted.
7. **Rows** (§5.4): 17 new rows with Rust token anchors, 8 existing rows re-anchored (and one test name retargeted). The set was run **in the debug copy outside the repository** with `n99-mut.py` (`anchor.apply`, then the killer test): 25 of 25 killed, after the one fix the loop found (P7 gained an assertion). That is a check of the rows, not the gate's run, which is `remote_mutate.py`; after `cargo fmt --all` all 419 rows of the two files still match exactly once.
8. **Withdrawn from v1:** "not risky"; the "no booking at `e`: the chain takes no join" wording (Fable's replaces it); the old P4's repeat case (no finite abandon can end inside a live repeat's span); the oracle helper of U1 is optional (§5.3).
9. **Decided by the orchestrator's brief and the plan, listed for the owner at landing:** S9's region (user-visible; an application of B); the guard; `end_finite`'s signature; the three sub-steps. User-visible overall: B at `e` refused with UR-21; S9's region; KL-39, RM-15 and UR-23 as §5.6 quotes them.
10. **Not verified (INFERRED):** that F1's revival is reachable in production only when the owner's call outlasts the window or the owner lags (`SEND_TIMEOUT`, host stalls), and not in normal timing; UHD's behaviour when a send times out on a full queue (that it returns the count sent after `SEND_TIMEOUT`); that a chain `remove_head` left would be skipped wrongly by a `join == false` test; that the two mutants §5.4 leaves unrowed are unobservable by a receipt; the overlap of an abandoned part's footprint with the rest's first four samples being harmless to every reader of the chains (read, and the suite passes). Everything else in the plan was run.

## 0. Summary (for the orchestrator)

- **Reproduced** at `cc8c3cb`, every number of the issue confirmed (§1; `e3b_c6_f1_and_f4_as_they_are` passes there and pins the issue's sequence as it is). It is not specific to a short send: any abandon of a *finite* booking leaves the record's chain joinable (short send, failed send, stalled send, underflow report, sequence-error report, a held-tail send that comes up short, an expired timeline). The record learns nothing from a finite abandon (`TxBook::abandon` returns at once for a non-repeat, `staging.rs:942-944`) while `close_unjoined` and `halt` write `join = false`.
- **Cause, one sentence:** `Chain.join` is the record's only representation of "the device burst can still be continued at `e`", and every event that ends that possibility writes it (the owner's close, a Stop, a loss, a refused configure, a drop at dispatch) except the abandon of a finite booking.
- **Plan: direction B (owner).** At the abandon of a finite booking that ends at `e`, the record (1) stops the chain taking joins, keeping its footprint `[.., e + 4)` as KL-39 says today, and (2) turns the pieces already joined behind it into a chain of their own, by the rule `remove_head` already has (extracted as `Chain::left_without_head`). It does neither while a halt waits in the list or the device is lost (F1). The chain is found by the samples it holds below its front (F2). About 45 lines of production code in `staging.rs` and 4 in `tx.rs`; Mock unchanged (`grep -rn abandon crates/ezsdr-mock-radio/src` finds nothing).
- **User-visible (the owner decides, AGENTS §9):** (a) a booking at the exact end of an abandoned finite burst is refused at receipt with `UR-21: the burst overlaps an accepted booking` (today: accepted as a join, sent alone), and a booking behind a piece accepted before the abandon is accepted or refused by that piece's real front (today: at the joined prediction, then dropped at dispatch); (b) a Stop that comes after the abandon and cancels a piece the abandon framed leaves the region `[t, B)` (S9); (c) KL-39's text, RM-15 and UR-23 (§5.6). Everything accepted *before* the abandon is known keeps its answer (§3.3).
- **Not fixed by B, same family:** after a *late* abandon (cursor within `g` of the burst's end) the record's front (end + 4) is below the device's (cursor + 2 + g); by today's design (`ur_23_a_booking_at_the_records_abandon_front_waits_for_the_owner_front`) a booking inside the owner's front is judged by its late policy at dispatch. Not B+ (owner).
- **Risk class: risky** (item 1 above): Sol reviews this plan before code. Sub-steps §5.7.


## 1. Reproduction on the current tree (`cc8c3cb`; the code is `44eb9b2`'s)

`cargo test -p ezsdr-radio-uhd --lib e3b_c6` passes: `e3b_c6_f1_and_f4_as_they_are` (`crates/ezsdr-radio-uhd/src/provider/tx/tests/e3b.rs:986-1031`) already pins the issue's sequence as it is. My own probe `n99_q1` (cache dir) prints it, verbatim (1 MS/s, `switch_rig_with([ramp 2 000, ramp 10, ramp 4], [ShortSend(0)])`):

```
A (2 000 @ 10 000) abandoned: sends [(500, sob), (2, eob)], device front Some(10517), no event
   record: Chain { start 10000, end 12000, joined [], front 12004, join: true }
B (10 @ 12 000): accept(12000,10) = Ok; booked: no event;  accept(12013,1) Err, accept(12014,1) Ok  (front 12014)
C (4, DropAndFlag @ 12 014): booked: no event
after dispatch: sends [.. (10, sob, eob)] device front Some(12025); events ["radio.TIME_ERROR \"drop\""]
   record: Chain { start 10000, end 12010, joined [12000], front 12014, join: true }
```

**The issue's sequence and outputs are confirmed exactly.** Corrections and additions (all VERIFIED by probes in the cache dir):

1. **Any abandon of a finite booking does it, not only a short send.** The same B and C outcome with `FailSend(0)` (device front 11 017), `StalledSend(0)` (sends `[(0, sob)]`, no close, front None), an `Underflow` report and a `SeqError` report (both `TX_UNDERFLOW`, front 11 017, probe Q4: `accept(12000,10)` is `Ok` before the fix). The `TailShort` abandon (Q12) leaves the chain joinable the same way, and the owner's expired-timeline abandon too (Q10c: A fully sent with its held tail, B held at `e`, device time jumps past `e`: B is left in A's chain with the prediction 12 014 while the device front is 12 017 and B will start its own device burst).
2. **C's fate depends on its late policy**: `DropAndFlag` → dropped at dispatch with `TIME_ERROR "drop"`; `SendAsapAndFlag` → moved to 12 025 with `TIME_ERROR "send_asap"` (Q9a); an untimed C is placed by the dispatch move (INFERRED, `dispatch_start` `tx.rs:950-964`).
3. **A long B shows nothing** (Q2: B of 200 at 12 000 is accepted as a join, sent `(199, sob), (2, eob)`, device front 12 204 = the record's `end + 4`): the mismatch is visible only when B is at most 32 samples, because only then the standalone close's front (`end + g`, rule A) differs from the joined piece's (`end + 4`).
4. **The usual route is a booking made before the abandon, not after** (Q3, Q11): bursts booked back to back ahead of time (KL-13's own workaround) are all accepted before A fails. B is then a legitimate join at receipt; after the abandon it is sent standalone, and any later booking accepted at B's predicted front is dropped (Q3 prints the same `TIME_ERROR "drop"`). The issue's "B accepted after the abandon" is the rarer, interactive case.
5. Not observed on hardware (the issue says so). The hardware trigger is an underflow or sequence report (UR-28) or a send failure; the FakeDevice's faults are the only evidence (INFERRED for hardware).

Re-run on `cc8c3cb` for this version: `cargo test -p ezsdr-radio-uhd --lib e3b_c6` passes in `/tmp/n99-base` (VERIFIED), and the eleven new rig tests of §5.3, run there with only `mod n99;` and the helpers' `pub(super)` added, fail ten and pass one (U7 guards the fix against a regression and so passes today): U1–U6 and U8–U11 fail.


## 2. Every path where the record's prediction and the device chain's state can disagree at receipt

"Receipt" reads the record only (`control.rs:517-583`, `accept` at `:581`; comment `core.rs:399`). "Dispatch" is `dispatch_start` (`tx.rs:937-1000`, late policy against the device chain's front) and `step`'s joins (`tx.rs:403-406, 432-462`). Rows are the events that change the device chain; the E3b records' F1–F8 are reused where they apply.

| # | Event | What receipt predicts (record) | What dispatch does (device chain) | What the user sees | Status |
|---|---|---|---|---|---|
| **P1** | **Abandon of a finite booking** (short/failed/stalled send `tx.rs:645-676`; tail send short `:610-611`; underflow/seq report `:1129-1140`; open timeline expired `:398-402`; waiting device burst expired `:419-424`) | chain keeps `join = true`, front `end + 4`; a booking at `e` joins; pieces already joined keep the joined front | device burst closed (two zeros), front `cursor + 2 + g`; a piece starts a new device burst: short ⇒ closed burst fronting `end + g` | **B at `e`: accepted, sent alone**; a short B's front is `g` past its end, not 4, so **C accepted at B's predicted front is dropped (`DropAndFlag`) or moved (`SendAsap`)**; no refusal anywhere | VERIFIED §1; **this is #99** |
| P2 | Abandon of a **joined piece** (the abandoned booking is not the head) | the chain's `join`/`end` are the last piece's; a booking at the last end joins | same as P1 for the pieces behind it | Q7: A, B (2 000), C (10) joined, B abandoned: `accept(14010)` `Ok` (joins C), D at C's predicted front 14 014 accepted and dropped | VERIFIED (Q7) |
| P3 | Abandon of a **repeat** (F2) | repeat ends at the cursor, front `cursor + 5` (`staging.rs:1382-1384`); a repeat never joins | device front `cursor + 2 + g` | a booking in `[cursor + 5, cursor + 2 + g)` accepted, late at dispatch (`TIME_ERROR send_asap`/`drop`); no join involved | VERIFIED, **pinned by design** (`ur_23_a_booking_at_the_records_abandon_front_waits_for_the_owner_front`, `tests/o1.rs:821`) |
| P4 | **F9, new:** abandon of a finite booking *late* (cursor within `g` of its end; Q9b: `FailSend(1)`, cursor 11 999, device front 12 016) | footprint to `end + 4` = 12 004 | device front 12 016 | B at 12 004 accepted at receipt, `TIME_ERROR "drop"` at dispatch (Q9b). The other direction (cursor far before the end: record refuses what the device takes) is F3/KL-39 | VERIFIED; same family as P3, by-design pattern; **not fixed by B** |
| P5 | The owner's close of the open device burst at `e` (`close_unjoined`, `tx.rs:470-481`) | record closes in the same section as the check (`staging.rs:979-981, 1108-1113`): `join = false` | the close is sent after | agree | VERIFIED (RM-16 last sentence; `e1_q14_*`) |
| P6 | A close that **fails or comes up short** (F6, `DeviceEnd::Uncertain`), then reopen and T3 flush | already closed in the record by P5 | device front None until the flush (`flush_front`, `tx.rs:815-818`) | a booking inside the flush front accepted and judged late at dispatch | VERIFIED, pinned (F6) |
| P7 | **Stop** (`halt`, `staging.rs:1214-1238`) | every kept chain `join = false`, front capped at the Stop bound `B`, region `[t, B)` | owner closes at the cursor (`stop_now` → `end_open`), front `cursor + 4/5` | by design the window bound is stricter than the cursor | VERIFIED, by design (F7) |
| P8 | **Loss** / `Provider::stop` | same `halt` at the loss sample | chain closed (`DeviceChain::lose`) | agree | VERIFIED (`staging.rs:1225`) |
| P9 | **Reopen succeeds / fails / interrupted** | unchanged | success: front from the flush (P6); failure: the stream's clock ends, the open booking and the held ones are cancelled with `ClockEnded` (`tx.rs:831-846`); later bookings are refused by the clock check | no join mismatch | INFERRED for the clock-ended refusal (not probed) |
| P10 | **Refused configure / drop at dispatch / SendAsap move** | `drop_piece` → `remove_head` writes the record at the event: the pieces behind a head that leaves are re-framed, a surviving short head closes (`staging.rs:1451-1482`, RM-15's exception; KL-38) | same | agree after the event | VERIFIED (existing tests, `rm_15_a_short_head_left_alone_*`). **This is the precedent B reuses** |
| P11 | **Direct close of a long burst's last send** (F1) and the window margin (F5) | `end + 4` / refuses starts below the horizon | `end + 3` / the owner may hand nothing yet | the record is stricter than the device | VERIFIED, pinned, conservative; out of scope |
| P12 | Receipt **between** the device result and the record's write (`complete_send` section → the abandon's section, or during the device call) | predicts from the state before the abandon | — | accepted as a join legitimately; see §3.3 | VERIFIED (Q13) |
| **P13** | **A Stop, `Provider::stop` or a loss that comes before the abandon's section** (F1: the owner is in a send that outlasts `SEND_TIMEOUT`, or a report abandons before the list's halt is read) | the Stop wrote the record under the lock at receipt (`control.rs:315-322` → `core.rs:437-441`; `core.rs:645-665` for the other two) and pushed its halt in the same section; a loss pushes none | the owner reads the halt later and ends the open booking | with an abandon that writes the record: a chain the halt cut is split and fronted again (a booking the Stop left acceptable is refused) | VERIFIED on the rig (R1, R2) and pure (F1b, F1c); closed by the guard (§4.2) |
| **P14** | **A receipt's `prune` drops a chain's join points; a Stop leaves a chain with a stale `end`** (F2) | `prune` keeps only the newest join point past device time; a halted chain keeps its `end` and `joined` and has its front at the Stop's bound | the abandoned booking's chain is the one that holds its samples | an abandon found by the join list or by `c.end == e` misses the chain, or finds the stale one (a booking received after the Stop, ending where the stale chain ends) | VERIFIED pure (P5, P6) and on the rig (U9); closed by the way the chain is found (§4.1) |

What `DeviceChain` could tell receipt, and cannot: `DeviceChain` holds one open device burst's `next`, tail and front. For a chain whose bookings have not been sent it is closed and says nothing; a join at `e` is a prediction about the future, so receipt **cannot** read "can the device burst be continued at `e`" from it. Only an *event* (the abandon) makes the prediction false, and the book learns that event in one section, `TxBook::abandon`. That is the argument for writing the record at the event rather than reading the chain at receipt.

## 3. Design options (decided: B, by the owner on 2026-11-13; the table is the record of the rejected ones)

All options keep the settled constraints (AGENTS §3: modules talk through Kernel contracts, the Mock enforces the envelope, core validates and does not optimise) and the E3b decision (owner 2026-11-12, shape S: receipt reads the record; "the device burst's actual state decides whether a join is possible" is #99's own wording of the fix).

### 3.1 The options and their contracts

Sequence used throughout (§1): A finite 2 000 at 10 000, abandoned at ~10 500; B 10 samples at 12 000 (A's end `e`); C 4 samples `DropAndFlag` at 12 014. "Known" = the receipt comes after the abandon section.

| | **A. Status quo + a KNOWN_LIMITATIONS entry** | **B. The abandon is an owner's close for the record (recommended)** | **C. Receipt reads live state (shape L for the join)** | **D. B + the abandon writes the device's actual front (C6: trim)** | **E. The abandon cancels the pieces behind it** |
|---|---|---|---|---|---|
| Rule | none | at the abandon of a finite booking ending at `e`: the chain takes no join at `e`; pieces joined behind it become a chain of their own (RM-15's head-leaves rule) | `accept`/`joinable` ask the book whether the chain's last piece is still held, open or continued by the device burst | B, and the abandoned chain's front becomes the device's (`cursor + 2 + g`), shrinking KL-39's footprint | the abandon drops every piece joined behind it (as it drops a repeat's switch, S4/A1) |
| **B at `e`, abandon known** | accepted, sent alone | **refused**: `UR-21: the burst overlaps an accepted booking` (inside A's footprint `[10 000, 12 004)`, KL-39) | refused (no live piece) | accepted as a standalone burst, front rule A: short `end + g` | accepted (but see "pieces") |
| **C after B, abandon known** | accepted, dropped (`DropAndFlag`) / moved (`SendAsap`) | B refused, so C is a plain booking: accepted at 12 014 and sent (Q1: `(4, sob, eob)`, front 12 033) | same as B (B refused) | refused at receipt if inside B's real front 12 025 | accepted |
| **B accepted before the abandon, C after it is known** (the usual route) | C accepted, dropped | **C refused** (B re-framed: front 12 025; Q3) | C accepted at the predicted front, dropped (the pieces keep the joined front) | refused | B cancelled (UR-22: "future held bursts keep their reservations", `design/18` line 258) |
| **C accepted before the abandon** | dropped/moved at dispatch | dropped/moved at dispatch (residual, §3.3; Q11) | same | same | cancelled with its piece |
| Fits RM-13/15/16 | contradicts RM-15's footprint (a booking at `e` is inside A's footprint and "joins" nothing) | RM-13's "a burst whose region meets an accepted one's is refused by RM-15"; RM-15's footprint sentence (a finite booking UR-22 abandoned keeps it) stays true; the abandon's record write is in the one section `abandon()` already takes (§3.3); RM-16's close rule (the check and the close are one step under the lock) is not extended to it | RM-15 states acceptance as a function of the record; adding live state makes "decided when received, in receipt order" depend on thread progress | changes RM-15's footprint sentence (a contract change KL-39 reserves to the owner, owner 2026-11-09) | contradicts UR-22 (`design/18:258`) and note 49 §5.4.4 item 2 (decided: pieces are not cancelled, they are judged at their own issue) |
| UR-21 / UR-22 | none | refusal text unchanged; UR-22 unchanged (the abandon closes under UR-22 as before) | text unchanged | UR-22's abandon front becomes the record's | UR-22 changes |
| Front rule A (short ≤ 32: `end + max(3, 15 µs)`, else `end + 3`) | the prediction `end + 4` for a joined piece is wrong when B is sent alone | pieces left behind use `predicted_front(.., joined = false)`: rule A for a short head (`end(padded) + g`), `end + 4` (held-tail shape, `HELD_TAIL_FRONT`) for a long one, as RM-15 says for a head and `remove_head` already computes | unchanged | same as B | n/a |
| Mock | no split (never abandons) | **no split, no change**: `grep -n abandon crates/ezsdr-mock-radio/src/*.rs` finds nothing (VERIFIED); M5 (`design/09:272`, note 49 §3 M5) says the Mock's sends do not fail. The new record function has no Mock caller; `Bookings` is shared, so the Mock's receipts are unchanged | needs a Mock stub for the live read | same as B | n/a |
| Cost (code) | one KL entry | (as amended, §4.1) `staging.rs`: `Bookings::end_finite` (17 lines), `Chain::left_without_head` extracted from `remove_head` (no new logic), `TxList::halted` (4), one `finite_end` argument on `TxBook::abandon`; `tx.rs`: compute it in the existing `abandon()` read and filter it by `is_lost()` (4 lines). Reference sketch: `probe/reference-sketch2-production.diff`; the first sketch's `probe/reference-sketch.diff` is superseded | `Bookings::accept`, `joinable`, `first_free`, `place`, `region_move` all gain a live-state parameter; the book's list, held map, open booking and device chain become inputs of a pure rule; the control receipt and the Mock both need it | B plus one line (`front = max(front, close_front(cursor, 2, gap))`, grow-only) or a shrink (needs the owner) | large |
| Makes impossible later | nothing; the mismatch stays | nothing: it is the first half of C6/D (D writes the front in the same section); a Mock fault schedule (M5) stays addable, the new function is pure and Provider-neutral | blocks the pure-record differential harness (VH): the Mock's receipt and the Module's would no longer be one function of one record | — | per-piece cancellation reports |
| Rejected / recommended | rejected: a known wrong answer with a documented workaround of "leave a gap"; the mechanism has 16 issues (`failure-mechanisms.md:9`) and note 49 asks for the record to be written at events | **recommended** | rejected (§3.2) | **deferred to the owner's C6 decision**; B loses nothing by waiting | rejected |

Variant **B+** (grow-only front at the abandon, closing P4/F9): the record's abandoned front is `max(predicted, close_front(cursor, CLOSE_SAMPLES, gap))`. One line in `end_finite`'s caller (the abandon already computes the same sum, `tx.rs:883`); user-visible only for abandons within `g` of the burst's end. I do not include it: it is a second behaviour (a front), F2 deliberately keeps the same pattern for repeats, and the owner has not seen it. **Decision (orchestrator, 2026-11-13): not included**; P4 stays as P3.

### 3.2 Why not C (receipt reads the live state)

1. A join at `e` is accepted while the chain's bookings are not yet sent (the normal case: booked 5 ms ahead). Receipt has nothing to read then: the device chain is closed, the open booking is another one. "Live" would have to mean "the last piece is still in `list.unread`, `list.held`, `open`, or the device burst continues at `e`", four structures in two locks' worth of state read inside the pure `accept`. That is a second representation of what `Chain.join` already says.
2. It does not fix the pieces already joined: their predicted front (`end + 4`) is still wrong after the abandon is known, so P1's C is still accepted and dropped. Only writing the record at the event re-frames them.
3. It makes `Bookings::accept` impure for the Mock too, and the Mock has no such state (VERIFIED, above).

### 3.3 The race: a receipt that comes before the abandon is known

The abandon is known to the book at two instants: the device call returns (`complete_send` section) and `abandon`'s record section (`tx.rs:882-883`), with the owner's open read between them (`tx.rs:879-881`). A receipt can land (a) while the device call is in flight, (b) between the completion and the record section.

Probe Q13 (`probe("tx_send_planned")` arming `at_nth_lock(streams, 3)`: the third streams lock after the plan is the abandon's record section; 4 is `end_open`'s): a B received at lock 3 is accepted as a join (no event) and the abandon section then re-frames it (record chains `{A: 10000..12000, front 12004, join false}`, `{B: 12000..12010, front 12025, join false}`); a B received at lock 4 is refused.

What each option does with a receipt before the abandon is known:
- **A:** accepted; sent alone; C dropped.
- **B:** B is accepted legitimately (the device burst *could* have been continued when it was received). The abandon section re-frames it. A C received **after** that section is refused or accepted against B's real front (Q3). A C received **before** it (also accepted at B's predicted front 12 014) stays accepted: **it is dropped or moved at dispatch** by its late policy (Q11: `TIME_ERROR "drop"`). This residual is unavoidable and consistent with RM-14/RM-15 ("one delayed by host lateness or a recovery at dispatch is judged by its late policy there"; `design/07` line 167): a receipt is decided when received, in receipt order, and is never revoked. I record it in the KL-39 text.
- **C:** the same, and the pieces are never re-framed (§3.2).
- **D:** as B.
- **E:** C's booking would be cancelled with B only if C were B's piece.

The window is made no wider by B (the record write is in the section `abandon()` already takes). Folding it into `TxBook::complete`'s `Settle::Abandon` would shorten it by one section but reaches only the send-driven abandons (the report-driven ones, `tx.rs:400, 422, 1140`, never pass through `complete`): not worth a second place. I do not propose it.

**A halt in the window (added after Fable's review).** The abandon is also late relative to a *Stop*, a `Provider::stop` or a loss that comes while the owner is in a send (`SEND_TIMEOUT` is 1 s): their record write is already in. §4.2 states the rule that keeps the abandon from undoing it (a halt unread in the list, or a lost device, makes the finite abandon write nothing), the probes that chose it, and what it must not change. A Stop that comes *after* the abandon's section is the ordinary case: the record holds the framed chains and the Stop halts them (S9).

## 4. The plan in detail (direction B)

### 4.1 The production change

Reference sketch: `probe/reference-sketch2-production.diff` (formatted with the repository's `rustfmt.toml`; `cargo clippy -p ezsdr-radio -p ezsdr-radio-uhd --all-targets` and `cargo fmt --all --check` are clean in the copy). Three pieces.

```rust
// staging.rs, Chain: the arms `remove_head` has today, extracted (no new logic; remove_head calls it)
fn left_without_head(&mut self, gap: Option<i64>) {
    if self.closed_short() { self.join = false; self.front = predicted_front(.., self.start, Some(self.end - self.start), false).unwrap_or(self.front); }
    else if !self.joined.is_empty() { self.join = false; /* front = max(front, every piece's own standalone front) */ }
}

// staging.rs, Bookings: the owner abandons the finite booking that ends at `e`
pub fn end_finite(&mut self, e: i64) {
    let gap = self.gap;
    let Some(c) = self.chains.iter_mut().find(|c| c.start < e && e <= c.end && e < c.front) else { return };
    let joinable = std::mem::replace(&mut c.join, false);
    if c.end == e { return; }                                                  // nothing joined behind it: only the join ends
    let mut behind = c.joined.split_off(c.joined.partition_point(|&j| j < e)); // the pieces from `e` on, accepted before the abandon was known
    behind.retain(|&j| j != e);                                                // `e` is the first piece's start, not a join point of the rest
    let abandoned = c.last();                                                  // the newest piece that starts before `e`
    let mut rest = Chain { start: e, end: c.end, joined: behind, front: c.front, join: joinable };
    c.end = e;
    c.front = predicted_front(&PACKETS, gap, abandoned, Some(e - abandoned), abandoned != c.start).unwrap_or(c.front);
    rest.left_without_head(gap);
    self.chains.push(rest);
}

// staging.rs, TxList and TxBook
pub fn halted(&self) -> bool { self.unread.iter().any(|arrival| matches!(arrival, Arrival::Halt(_))) }
pub fn abandon(&mut self, repeat: bool, id: .., cursor_k: i64, now_k: i64, bound: i64, finite_end: Option<i64>) -> usize {
    if !repeat {
        if let Some(e) = finite_end.filter(|_| !self.list.halted()) { self.record.end_finite(e); }
        return 0;
    }
    ..
}
```

`tx.rs`, `Tx::abandon` (the one place the six production callers reach, §6): the existing read of the open booking returns a third member, and the existing record section filters it:

```rust
let (repeat, id, finite_end) = self
    .with_book(|book| book.open.as_ref().map(|open| (open.booking.repeat, open.booking.settings.action(), open.booking.record_len().map(|len| open.booking.k + len))))
    .unwrap_or_else(|| (self.with_held(|map| map.get(&cursor).is_some_and(|held| held.switch)), None, Some(cursor)));   // no open booking: the waiting device burst, which ends at the cursor
..
let cancelled = self.with_book(|book| book.abandon(repeat, id, cursor, now, clock.instant(close_front(cursor, CLOSE_SAMPLES, CLOSE_GAP)), finite_end.filter(|_| !self.core.is_lost())));
```

Why the chain is found as it is (F2): a receipt's `prune` keeps only the newest join point past device time, and a chain a halt cut keeps its `end` and `joined`; `c.end == e || joined.contains(e)` misses the first and can find the second. The abandoned piece's last sample `e − 1` lies below the front of the chain that holds it (a live chain's front is at least its last piece's end plus three), and below no stale chain's front, which a halt left at its bound, below the first sample of any booking received after it. Spans of live chains are disjoint (the split gives the abandoned part `[.., e)` and the rest `[e, ..)`; a footprint may overlap its neighbour's span by four samples, which no reader of the chains needs to be disjoint: `chain_front`, `held_until` and `may_be_handed` take a maximum or any, `move_head` finds the head by its start; VERIFIED by reading, and by the suite).

### 4.2 F1: the guard, and the evidence it was chosen on

Facts (VERIFIED by reading, and by the probes): a Stop writes the record and pushes its halt in one section under the streams lock (`control.rs:315-322` → `core.rs:437-441`; `Provider::stop` the same at `core.rs:659-665`); a loss writes the record under the lock and then stores `lost` (`core.rs:1061-1070`) and pushes no halt; the owner reads the list one entry per section (`tx.rs:270-279`) and ends the open booking when it reads a halt (`stop_now`, `tx.rs:1024`), so **after a halt is read no abandon reaches the record** (it needs a cursor: the open booking's or the device burst's, both gone); a halt is therefore *unread* exactly in the window in which an abandon can follow a halt's record write: while the owner is inside a send that began before the Stop (the plan's section, the device call, the result's section: `tx.rs:582-645`), or reads a report first (`pass` reads the reports before the list: `tx.rs:217-221`).

| shape | what the abandon does without the guard | probed |
|---|---|---|
| Fable's: A 30 000 @ 10 000 + B 10 @ 40 000 joined; `halt(10 100, 20 105)`; abandon at 40 000 | **nothing**: the chain's front is 20 105, `e` = 40 000 is not below it, so F2's way of finding the chain skips it (record `==` the halted one) | pure `n99_pure_f1`, "same-as-halted: true" |
| F1b: A 2 000 @ 10 000 + B 10 @ 12 000 joined; `halt(10 100, 20 105)`; abandon at 12 000 | splits: A `{.., front 12 004}`, B `{12 000.., front 12 025}`: a booking at 12 014 is accepted after the Stop and refused after the abandon (`ok@12014` → `ok@12025`) | pure `n99_pure_f1` |
| F1c: A, B 2 000 @ 12 000, C 10 @ 14 000 joined; the same halt; abandon at 14 000 | `ok@14014` → `ok@14025` | pure `n99_pure_f1` |
| R1: A 2 000 @ 10 000 + B 10 @ 12 000 joined; the first send takes 0.5 s of device time; a Stop comes in it; `ShortSend(0)` or `FailSend(0)` | after the Stop `E11990..=12013 .12014..`; after the abandon, unguarded, `E11990..=12024 .12025..` | rig `n99_r1` (printed: `[Chain{10000..12010, joined [12000], front 12014, join false}]` then `[{10000..12000, 12004}, {12000..12010, 12025}]`) |
| R2: the same A and B, device time 11.5 ms before the owner runs; a Stop unread **or** a device loss; an underflow report | same: unguarded `.12025`; guarded `.12014` | rig `n99_r2` (Stop, loss, and "no halt" as the control: `.12025` with or without the guard) |
| TxBook: the F1b record with `list.halt(Stop)` | unguarded: record changed; guarded: `==` the record before | pure `n99_pure_txbook_guard` (`N99_NOGUARD`) |

*Reachability.* In normal timing the owner hands samples only up to about `now + W` and a Stop's bound is `t + W + 5`, so the abandoned booking's end is within a chunk of the bound or past it, and F2's `e < c.front` already skips the halted chain (INFERRED from the pure and rig results above; a rig run in normal timing, a Stop at the seam of the 11 ms send, left the record as the Stop left it, but with the guard on, so it does not separate the guard from F2's lookup). The guard matters when the owner's call outlasts the window, which `SEND_TIMEOUT` = 1 s allows, or the owner lags. It is cheap (4 + 1 lines), exact where F2's predicate is timing-dependent, and states the invariant "an abandon after a halt changes nothing" as one rule.

*Chosen:* `TxList::halted()` + the Provider's `is_lost()` filter, both read in the abandon's record section, so no Stop or loss can slip between the check and the write (the Stop's section and the loss's section take the same lock and set their halt, or `lost`, before releasing it). *Alternatives rejected* (item 2 of the changes): the `join == false` test; a flag on `Chain`; clamping; writing at `complete`.

### 4.3 What the change must not change (each has an existing or new test)

- A repeat's abandon and its switch (`e2a_80_*`, `o1_uhd_t7a*`, `ur_21_s4_*`: `TxBook::abandon`'s repeat path is untouched; N47S5-153 retargets the branch).
- KL-39's footprint: a booking at 10 505 on a finite burst abandoned at 10 500 is refused, one at the footprint's end accepted (`e2a_kl39_*`, `o1_uhd_t7b_*`, P1, U2).
- The abandon's close and fronts (the device column of `e3b_c1`), the Provider's events at the abandon (U2: no event beyond today's).
- A halt's own record writes (`halt`, `stop_played`, `lose`) and the owner's reading of the list; an abandon that follows a Stop or a loss (U7, U8, `txbook_a_finite_abandon_after_a_halt_changes_nothing`).
- A booking received after a Stop and abandoned ends *its* chain, and the stale chain is as it was (P6).
- `remove_head`'s behaviour (the rows N47S5-140/-141/-145/-146 and P4: one rule, two callers).
- MockRadio: never abandons (M5, `design/09:272`: its sends do not fail); `Bookings` is shared, so its receipts are unchanged (`grep -rn "abandon\|end_finite" crates/ezsdr-mock-radio/src` finds nothing).

### 4.4 Open points for the implementer

`end_finite`'s `c.end == e` branch writes `join = false` itself rather than calling `close(e)`: `close` finds a chain by `join && end == e` (a chain that ended through a halt is the case it must skip), so it is not the same lookup; the one repeated fact is "the owner's end of a join writes `join = false`" (§5.5). The `left_without_head` extraction keeps `remove_head`'s behaviour exactly (VERIFIED: every `staging` test passes with only the call-site argument added).

## 5. Test plan

### 5.1 Sequences and expected results (1 MS/s frozen rig; every literal was produced by the amended sketch; today's value in brackets)

"Receipt" is `receipt_runs` over a one-sample booking: `E` refused for overlap, `.` accepted. Sends are `(samples, sob, eob)`. Today's values come from the unchanged tree (the ten failing rig tests of §1 print them) or, where the probe's `N99_OFF` switch gave them, from the sketch with `end_finite` off.

| ID | Sequence | Expected after the change (today in brackets) |
|---|---|---|
| S1 | the issue's: A 2 000 @ 10 000, `ShortSend(0)`; B 10 @ 12 000; C 4 `DropAndFlag` @ 12 014 | sends `[(500,sob), (2,eob)]`, device front 10 517, no event; receipt at 1 ms `E11990..=12003 .12004..=12099` (`E11990..=11999 .12000..=12000 E12001..=12003 .12004..`). B: one `radio.COMMAND_REJECTED "UR-21: the burst overlaps an accepted booking"` (accepted, joined). C accepted (no event), sent `(4,sob,eob)`, device front 12 033, **no TIME_ERROR** (`TIME_ERROR "drop"`, front 12 025). |
| S2 | the same A abandoned six ways (U2): `ShortSend(0)`; `FailSend(0)`; `StalledSend(0)`; no fault + an `Underflow` report (tick 10 200) after 1 ms; the same with a `SeqError` report (no tick); no fault, 12 ms of turns (the open burst's timeline expires) | sends/fronts/events: `[(500 sob),(2 eob)]` / 10 517 / none; `[(500 sob),(2 eob)]` / 11 017 / `COMMAND_REJECTED "fake: the send failed after half its samples"`; `[(0 sob)]` / none / none; `[(1000 sob),(2 eob)]` / 11 017 / `TX_UNDERFLOW`; the same; `[(1000 sob),(999),(2 eob)]` / 12 004 / none. In each: `accept(12000,10)` `Err` (`Ok`), `accept(12003,1)` `Err`, `accept(12004,1)` `Ok` |
| S3 | long B (200) @ 12 000 after the abandon, then a 200-sample booking at 12 004 | B refused `UR-21: overlaps` (accepted, sent `(199 sob),(2 eob)`, front 12 204); the booking at 12 004 accepted, no event |
| S4 | A, B booked together (B joins at receipt, no event); `ShortSend(0)` abandons A; C 4 @ 12 014 | after the abandon `E11990..=12024 .12025..=12099` (`E11990..=12009 .12010..=12010 E12011..=12013 .12014..`): B's region is `[12 000, 12 025)`; C refused at receipt, one `UR-21: overlaps` (accepted, dropped at dispatch); B sent `(10,sob,eob)`, front 12 025, no TIME_ERROR |
| S4b | A, B, C all booked before the abandon | unchanged (the residual, §3.3): sends `[(500 sob),(2 eob),(10 sob eob)]`, `TIME_ERROR "drop"` for C, front 12 025 |
| S5 | A 2 000 @ 10 000, B 2 000 @ 12 000, C 10 @ 14 000 joined; `ShortSend(2)` abandons B (4 ms) | sends `[(1000 sob),(1000),(1000),(2 eob)]`, front 13 017; receipt `E13990..=14024 .14025..=14099` (`E13990..=14009 .14010..=14010 E14011..=14013 .14014..`): A and B stay one chain without a join at 14 000, C is its own chain fronted 14 025; `accept(14010,4)` `Err` (`Ok`); D 4 @ 14 014 refused `UR-21: overlaps` (accepted, dropped); C sent `(10 sob eob)`, front 14 025 |
| S6 | tail short: A 2 000 sent with its held tail, B 2 000 @ 12 000 booked after, `ShortSend(2)` takes none of the tail | sends `[(1000 sob),(999),(0),(2 eob)]`, front 12 017; B abandoned unsent; `accept(14000,10)` `Err` (`Ok`), `accept(14004,10)` `Ok` |
| S7 | waiting device burst expires with B held: A, B 10 @ 12 000 booked; after 2 ms `E11990..=12009 .12010..=12010 E12011..=12013 .12014..` (one chain); device time jumps to 12.5 ms, one step | sends `[(1000 sob),(1000),(2 eob)]`, front 12 017; receipt `E11990..=12024 .12025..=12099` (unchanged one-chain answer): A's chain ends at 12 000, B's front is its own; this is the abandon with **no open booking** (the `Some(cursor)` end) |
| S8 | receipt at the seam: B received before the abandon's record section / after it (`probe("tx_send_planned")` → `at_nth_lock(streams, 3 / 4)`) | lock 3: B accepted (no event), then framed: `E11990..=12024 .12025..`; lock 4: B refused, `E11990..=12003 .12004..` |
| S9 | a Stop after the abandon (A abandoned at ~0 ms, B @ 12 000 held): at 11 ms (between A's start and B's) / 13 ms (both started) / 5 ms (before A) | 11 ms: region `[11 000, 21 005)`: `E11900..=21004 .21005..=25099` (`E11900..=12013 .12014..=25099`, no region); 13 ms: no cut, no region: chains as S4 (`.12025`; today `.12014`); 5 ms: both chains cancelled, region `[5 000, 15 005)`, as today |
| S10 | an abandon with no chain that holds `e − 1` (pure P1): `e` = 999, 42, 0, −5 on a chain 0..40; and none that a booking before it holds (P7) | the record is unchanged (`==` its clone) |
| S11 (F1) | A 2 000 + B 10 joined; the first send takes 0.5 s; a Stop, or a device loss, comes in it; the send is short or fails | the record the halt left stands: `E11990..=12013 .12014..=12099`; sends `[(500 sob),(2 eob)]` (after a loss `[(500 sob)]`: no close) (guard off: `E11990..=12024 .12025..`) |
| S12 (F1) | A open at 11 000, B booked, device time to 11.5 ms before the owner runs; a Stop received and **unread** / a loss / nothing; an underflow report | Stop, loss: `E11990..=12013 .12014..=12099`; nothing: `E11990..=12024 .12025..=12099` (guard off: the first two `.12025`) |
| S13 (F2) | A 2 000, B 2 000, C 10 @ 14 000 joined; 1 ms of turns; device time to 12.5 ms; a receipt (D 4 @ 30 000) prunes B's join point; an underflow report | `E11990..=14024 .14025..=14099` (`E11990..=14013 .14014..`); `accept(14010,4)` `Err` (`Ok`) |

### 5.2 Pure tests (`crates/ezsdr-radio/tests/staging.rs`; no clock, no thread)

Chains are built with `armed()` (gap 15) and `chain_of` (each piece accepted and booked in order); assertions are verdict runs over a sample range (`verdicts`, as C1's `receipt_runs`), never private fields. All run on the sketch (VERIFIED, 9 tests).

| ID | Test | Asserts | Fails if (rows) |
|---|---|---|---|
| P1 | `n99_p1_an_abandoned_finite_chain_takes_no_join_and_keeps_its_footprint` | head 40: before, `accept(40,10)` `Ok`; after `end_finite(40)`: ` ERR@39 ok@44`; head 40 + piece 10, `end_finite(50)`: ` ERR@49 ok@54`; no chain holds `e − 1` (`e` = 999, 42, 0, −5): record `==` its clone | the join stays (N99-04); `e <= c.end` missing (N99-02); the no-piece branch builds an empty rest chain whose front (end + 19) refuses a booking at 44 (N99-05) |
| P2 | `n99_p2_the_pieces_behind_the_abandoned_booking_are_a_chain_of_their_own` | head 40 + 10 + 4, `end_finite(40)`: ` ERR@40 ok@69`; head 40 + piece 100: `accept(140,10)` `Ok` and ` ERR@40 ok@140 ERR@141 ok@144`; head 40 + piece 10: ` ERR@40 ok@65`, a booking at 50 refused; head 40 + 10 + 100, `end_finite(50)`: `accept(150,10)` `Ok`, `accept(50,10)` `Err` | pieces forgotten (N99-10); a long piece loses its join (N99-11); the rest's front is the joined one (N99-09); the first piece's start left in `joined` (N99-06) |
| P3 | `n99_p3_the_abandoned_footprint_outlives_the_pieces_behind_it` | `end_finite(40)` on head 40 + 10, then `drop_piece(40, Some(10))`: ` ERR@40 ok@44`; head 40 + 10 + 4, `end_finite(50)`, `drop_piece(50, Some(4))`: ` ERR@50 ok@54` (the joined piece's end + 4, not a standalone 65) | the abandoned part's front not set (N99-08); the piece taken for a head (N99-07) |
| P4 | `n99_p4_a_head_that_leaves_and_an_abandoned_one_leave_the_same_chain_behind_them` | for 5 shapes, the verdicts from 44 on after `drop_piece(0, Some(40))` equal those after `end_finite(40)`, and so does a join at 150 | the two callers of `left_without_head` diverge (N99-09; N47S5-140/-141/-145/-146 for `remove_head`) |
| P5 | `n99_p5_a_receipt_s_prune_does_not_hide_the_abandoned_booking_s_chain` | A 2 000 @ 10 000, B 2 000 @ 12 000, C 10 @ 14 000; `prune(12001)`; `end_finite(12000)`: ` ERR@11990 ok@14025`, `accept(14010,4)` and `accept(12000,4)` `Err` | the chain found by `joined.contains(e)` (F2); `e <= c.end` (N99-02) |
| P6 | `n99_p6_a_chain_a_stop_cut_is_not_the_abandoned_booking_s_chain` | 3 × 100 from 0, `halt(50, 70)`, then a booking of 220 from 80 (ends 300 too): before, `accept(300,10)` `Ok` (a join); after `end_finite(300)`: `Err`, and the verdicts over 0..79 as before | `e < c.front` missing (N99-03: the stale chain, first in the list, is found) |
| P7 | `n99_p7_a_booking_before_the_abandoned_one_does_not_hold_its_chain` | a chain of 100 at 5 000 booked first, then a head 0..40: `end_finite(40)`: `accept(5100,10)` `Ok` (it still joins), `accept(40,10)` `Err`, ` ok@44` over 44..4 900 | `c.start < e` missing (N99-01: the chain at 5 000 is split at 40; its footprint then holds `[40, 5 104)`) |
| P8 | `txbook_a_finite_abandon_ends_its_chain_and_changes_nothing_else` (replaces `txbook_a_finite_abandon_changes_nothing`, `staging.rs:1601`) | `abandon(false, .., Some(40))` leaves `record == record.clone().end_finite(40)` (so the repeat behind it and its pending switch are as they were), `playing` and the settled switches unchanged, a booking at 40 refused and one at 44 accepted; `None` leaves the record `==` | the call is dropped (N99-13); the finite branch is not taken (N47S5-153) |
| P9 | `txbook_a_finite_abandon_after_a_halt_changes_nothing` | head 100 + piece 10, `halt(50, 400)`: ` ERR@100 ok@114`; with `list.halt(Stop)` unread the abandon leaves the record `==`; without it ` ERR@100 ok@125`; the halt read → `halted()` false | the guard (N99-12); `halted()` reads the wrong entries (N99-14) |

### 5.3 UHD rig tests (new file `crates/ezsdr-radio-uhd/src/provider/tx/tests/n99.rs`, `mod n99;`; single-threaded, frozen clock, no timing dependence)

Helpers of `e3b.rs` made `pub(super)`: `dev_front`, `dev_open`, `evs`, `accepted`, `receipt_runs`, `at_nth_lock` (6 test-only lines). The record is read as a receipt reads it, never through a field. All eleven ran on the sketch (VERIFIED); on the unchanged tree U1–U6 and U8–U11 fail and U7 passes (it guards the fix).

| ID | Test | Asserts and how | Fails if |
|---|---|---|---|
| U1 | `n99_u1_a_booking_at_the_end_of_an_abandoned_burst_is_refused` | S1: sends, front, no event; the receipt runs; B → one UR-21 `COMMAND_REJECTED`; C accepted, sent `(4 sob eob)`, front 12 033, no event after 20 ms (this is the F4 half of `e3b_c6`, moved) | the abandon leaves the join (N99-13, N99-15); B accepted; C dropped |
| U2 | `n99_u2_every_way_a_finite_burst_is_abandoned_ends_its_chain` | S2: six cases, each a `(sends, front, events)` literal and the receipts at 12 000 / 12 003 / 12 004 | one abandon path (`send`'s three arms, `report`'s two codes, `step`'s expiry) not reaching the record |
| U3 | `n99_u3_a_piece_accepted_before_the_abandon_is_a_chain_of_its_own` | S4 and S4b: the receipts, C refused (S4) or dropped with `TIME_ERROR "drop"` (S4b), B sent `(10 sob eob)`, front 12 025 | the pieces keep the joined front (N99-09, N99-10); S4b pins the residual so it cannot vanish silently |
| U4 | `n99_u4_the_abandoned_middle_piece_leaves_the_pieces_before_and_behind_it` | S5 | the split at `e` loses A's span or C's own front (N99-06..11) |
| U5 | `n99_u5_a_tail_that_comes_up_short_abandons_the_piece_that_waits_for_it` | S6 | the tail path's abandon (`tx.rs:611`) reaching the record with the wrong end (N99-15) |
| U6 | `n99_u6_the_waiting_device_burst_that_expires_re_frames_the_piece_held_behind_it` | S7: the abandon with no open booking | the `Some(cursor)` end (N99-16) |
| U7 | `n99_u7_a_stop_or_a_loss_that_comes_during_the_send_is_not_undone_by_its_abandon` | S11: three cases (Stop + `ShortSend`, Stop + `FailSend`, loss + `ShortSend`); the probe at `tx_send_planned` jumps device time 0.5 s and books the Stop (control parked in a `thread_local!`: the probe's closure is `Send + Sync`) or calls `device_lost`; receipts and sends | the guard (N99-12) and the loss filter (N99-17) |
| U8 | `n99_u8_a_halt_the_owner_has_not_read_is_not_undone_by_a_report_driven_abandon` | S12: Stop unread / loss / none, the underflow report called directly | the same two; the control case pins that the abandon does frame B when no halt came |
| U9 | `n99_u9_a_receipt_between_the_sends_and_the_abandon_does_not_hide_the_abandoned_chain` | S13 | F2's chain lookup; a join-list lookup |
| U10 | `n99_u10_a_receipt_at_the_seam_is_decided_by_the_record_it_finds` | S8: lock 3 (accepted, then framed) and lock 4 (refused) | a record write moved out of the abandon's section (the seam count changes without notice) |
| U11 | `n99_u11_what_the_abandon_leaves_to_a_later_booking_and_a_later_stop` | S3 and S9 (the 11 ms Stop) | an over-wide split; S9's region lost |

Optional (the implementer's call, 6 lines): an `o::abandoned(&Region) -> Region` in `crates/ezsdr-acceptance/tests/oracle/mod.rs` that marks the region non-joining, and U1 asserting `o::admit` equals the receipt's verdict: an independent check of S1 written from the new RM-15 sentence. The oracle's `Region` has no such field today; the helper would set `len` to the short bound.

**Existing tests that change (typed edits, none silent; VERIFIED: on the amended sketch these two are the only failures of `cargo test --no-fail-fast -p ezsdr-radio-uhd -p ezsdr-radio -p ezsdr-mock-radio -p ezsdr-acceptance`):**
- `e3b_c1_the_front_after_each_close`, the row "1 MS/s: a finite burst abandoned by a short send" (`e3b.rs:276-282`): the receipt line `E5000..=5199 .5200..=5200 E5201..=5203 .5204..=5239` becomes `E5000..=5203 .5204..=5239` (the failure prints exactly that; the device lines are unchanged).
- `e3b_c6_f1_and_f4_as_they_are`: its F4 half (`e3b.rs:1003-1030`) moves into U1; the test keeps F1 and is renamed `e3b_c6_f1_as_it_is` (nothing references the old name but `plan/phase8/reviews/e3b-test-plan.md`, a record).
- `txbook_a_finite_abandon_changes_nothing` → P8; the 7 `book.abandon(..)` call sites in `tests/staging.rs` (1525, 1550, 1567, 1581, 1595, 1611, 2602) gain `, None`.

No hardware test changes (VERIFIED: `grep -rn abandon crates/ezsdr-radio-uhd/tests` finds only an unrelated comment, `infra.rs:50`; the B8 and H2 hardware tests drive the raw device and compute nothing from RM-15's join rule); no bench value is adopted, so no sweep (AGENTS §9: a receipt rule with no measured number).

### 5.4 Mutation rows (Rust token anchors; run in a debug copy outside the repository: 25 of 25 killed)

New rows (ids `N99-nn`; each `old` matches exactly once after `cargo fmt --all`; `new` compiles):

| Row | File | `old` | `new` | Killer |
|---|---|---|---|---|
| N99-01 | staging.rs | `c.start < e && e <= c.end && e < c.front` | `e <= c.end && e < c.front` | `n99_p7` |
| N99-02 | staging.rs | the same | `c.start < e && e < c.front` | `n99_p1` |
| N99-03 | staging.rs | the same | `c.start < e && e <= c.end` | `n99_p6` |
| N99-04 | staging.rs | `let joinable = std::mem::replace(&mut c.join, false);` | `let joinable = c.join;` | `n99_p1` |
| N99-05 | staging.rs | `if c.end == e { return; }` | `if c.end == e && false { return; }` | `n99_p1` |
| N99-06 | staging.rs | `behind.retain(\|&j\| j != e);` | `let _ = &behind;` | `n99_p2` |
| N99-07 | staging.rs | `Some(e - abandoned), abandoned != c.start)` | `Some(e - abandoned), abandoned == c.start)` | `n99_p3` |
| N99-08 | staging.rs | `c.front = predicted_front(&PACKETS, gap, abandoned, Some(e - abandoned), abandoned != c.start).unwrap_or(c.front);` | `let _ = abandoned;` | `n99_p3` |
| N99-09 | staging.rs | `rest.left_without_head(gap);` | `let _ = gap;` | `n99_p2` |
| N99-10 | staging.rs | `self.chains.push(rest);` | `let _ = rest;` | `n99_p2` |
| N99-11 | staging.rs | `front: c.front, join: joinable }` | `front: c.front, join: false }` | `n99_p2` |
| N99-12 | staging.rs | `finite_end.filter(\|_\| !self.list.halted())` | `finite_end` | `txbook_a_finite_abandon_after_a_halt` |
| N99-13 | staging.rs | `self.record.end_finite(e);` | `let _ = e;` | `txbook_a_finite_abandon_ends_its_chain` |
| N99-14 | staging.rs | `matches!(arrival, Arrival::Halt(_))` | `matches!(arrival, Arrival::Booking(_))` | `txbook_a_finite_abandon_after_a_halt` |
| N99-15 | tx.rs | `.map(\|len\| open.booking.k + len)` | `.map(\|len\| open.booking.k)` | `n99_u1` (and U10, U11) |
| N99-16 | tx.rs | `None, Some(cursor)));` | `None, None));` | `n99_u6` |
| N99-17 | tx.rs | `finite_end.filter(\|_\| !self.core.is_lost())` | `finite_end` | `n99_u7` |

(N99-01..03 are one anchor text with three `new`s: the `old` matches once in the file, and the three rows differ in `new` only.) Not rowed on purpose (INFERRED unobservable by a receipt): `c.start < e` → `<=` (a chain that starts at `e` is the rest, which sits after its neighbour in the list); `partition_point(|&j| j < e)` → `<=` (leaves `e` in the abandoned part's `joined`, which no receipt reads).

Re-anchored, same program change, same killer (8; found by matching every row of the six lists against the sketch, and each killed in the debug copy): the text moves from `c.` to `self.` in `left_without_head` for **N47S5-140** (`if self.closed_short() {` → `… && self.end - self.start <= 0 {`), **N47S5-141** (`self.front = predicted_front(&PACKETS, gap, self.start, Some(self.end - self.start), false).unwrap_or(self.front);` → `let _ = self.end - self.start;`), **N47S5-145** (`self.front = self.front.max(pieces);` → `let _ = pieces;`), **N47S5-146** (`self.join = false; let starts` → `let starts`); the three-member tuple of `Tx::abandon` for **N47S5-151** (`.unwrap_or_else(|| (self.with_held(|map| map.get(&cursor).is_some_and(|held| held.switch)), None, Some(cursor)));` → `.unwrap_or_else(|| (false, None, Some(cursor)));`), **E2A2B-13** (`.map(|open| (open.booking.repeat, open.booking.settings.action(), open.booking.record_len().map(|len| open.booking.k + len)))` → `(true, …)`) and **E1-U13** (the `with_book(|book| book.abandon(.., finite_end.filter(|_| !self.core.is_lost())))` text with the `ProviderStop` halt added); **N47S5-153** (`old` `if !repeat { return 0; }` becomes `if !repeat {`, `new` `if false && !repeat {`: the finite branch is never taken, so a finite abandon reaches `Bookings::abandon` as a repeat's does; its `filter` is retargeted to `txbook_a_finite_abandon_ends_its_chain`). Withdrawn: none; `rules.py` finds every row exactly once after the change (VERIFIED on the formatted sketch: 419 rows of the two files).

### 5.5 Maintainability (AGENTS §9)

1. **Duplication:** the surviving-head rule exists once, in `Chain::left_without_head` (extracted, not copied; two callers, P4). The one repeated fact is "an owner's end of a join writes `join = false`" (`close`, `end_finite`'s no-piece branch), whose lookups differ (§4.4).
2. **State:** no new field and no new representation. `Chain.join` is written by one more event; `TxList::halted()` reads the list's own unread halts rather than a second flag; `DeviceChain` is untouched.
3. **Tests:** pure decisions at the pure level (P1–P9); rig tests only for wiring, ordering and the seams (U1–U11). C1 is edited; C6's F4 half moves, not copies. U2 and U7 are tables in a loop, not eleven copies of a setup.
4. **Leftovers:** the F4 half of C6; `txbook_a_finite_abandon_changes_nothing`; N47S5-153's old meaning.
5. **Rows:** smallest anchors; the two non-rowed mutants are named.

### 5.6 Text and records

The sentences below are what the commit writes. `rules.py --base cc8c3cb` must pass (a rule is normative text only: no rationale, history or issue numbers; they go in the specs' *Changes* tables and the decision record).

**`design/07-radio-model.md`, RM-15** (lines 165 and 167; Fable's corrections: the exception is no longer "the one", "as if the abandoned booking had left" is dropped because the footprint stays, "the abandoned booking's chain" replaces "the chain", and an abandon after a Stop changes nothing).

Line 165, the first sentence, from

> The one exception is a booking whose configure the device refuses: it leaves the record, and a surviving head of at most 32 samples is closed at its standalone front, so no later booking joins it.

to

> The exceptions are a booking whose configure the device refuses and a finite booking that UR-22 abandons (its footprint, below). A booking whose configure is refused leaves the record, and a surviving head of at most 32 samples is closed at its standalone front, so no later booking joins it.

Line 167, the clause "a finite booking that UR-22 abandoned keeps it, a repeat that UR-22 abandoned keeps it up to its cursor plus five," from

> a finite booking that UR-22 abandoned keeps it, a repeat that UR-22 abandoned keeps it up to its cursor plus five,

to

> a finite booking that UR-22 abandoned keeps it, and the abandoned booking's chain takes no join from then on (a booking at its end is refused, as it meets the footprint); the pieces joined behind it, accepted before the abandon was known, are a chain of their own, framed as the pieces behind a head that leaves are (above), so a long piece alone behind it still takes a join; an abandon that follows a `Stop`, `Provider::stop` or a device loss changes nothing; a repeat that UR-22 abandoned keeps it up to its cursor plus five,

*Test:* line, appended to "the footprint an abandoned finite booking keeps by `o1_uhd_t7b_…` and `e2a_kl39_…`": ", the end of its join and the pieces behind it by `n99_p1_an_abandoned_finite_chain_takes_no_join_and_keeps_its_footprint`, `n99_p2_the_pieces_behind_the_abandoned_booking_are_a_chain_of_their_own`, `n99_p6_a_chain_a_stop_cut_is_not_the_abandoned_booking_s_chain`, `txbook_a_finite_abandon_after_a_halt_changes_nothing`, `n99_u1_a_booking_at_the_end_of_an_abandoned_burst_is_refused`, `n99_u3_a_piece_accepted_before_the_abandon_is_a_chain_of_its_own` and `n99_u7_a_stop_or_a_loss_that_comes_during_the_send_is_not_undone_by_its_abandon`".

**`design/18-uhd-radio.md`, UR-23** (line 266; UR-22 stays true: its "future held bursts keep their reservations and undergo UR-21 at dispatch" is what a piece accepted before the abandon does). After "…one whose send failed or came up short is abandoned there too." add:

> The abandon ends the burst's chain in the record in the same section (RM-15): a booking at its end is refused at receipt with `UR-21: the burst overlaps an accepted booking`, and a burst already held behind it is sent as a new device burst, not as its continuation.

*Test:* line, appended: `n99_u2_every_way_a_finite_burst_is_abandoned_ends_its_chain`, `n99_u6_the_waiting_device_burst_that_expires_re_frames_the_piece_held_behind_it`.

**`KNOWN_LIMITATIONS.md`, KL-39** (same commit, AGENTS §2). Symptom, from "…although the burst it overlaps stopped sounding earlier." to:

> **Symptom:** a timed burst is refused at receipt with `radio.COMMAND_REJECTED` (`UR-21: the burst overlaps an accepted booking`), although the burst it overlaps stopped sounding earlier. A burst booked at the exact end of an abandoned finite burst, which would have continued it, is refused the same way.

When:

> **When:** the UHD Module abandoned a burst (UR-22: an underflow, a sequence error, a failed or short send, a continuation past its time), and the new burst starts inside what the abandoned burst still holds: a finite burst keeps its booked region and four samples past its end (its front, RM-15) and no burst continues it; a repeat keeps its region up to the cursor it was abandoned at plus five samples (RM-15), which can be up to the in-flight window (`radio.timing.tx_in_flight_ns`) ahead of device time. A burst accepted before the abandon was known keeps its answer: one booked at the abandoned burst's end is sent as a new device burst with its own front (a short one: `g` samples past its padded end, KL-36), and a burst booked at the front it was accepted at, or inside the device's front after a late abandon (the cursor plus two plus `g`), is judged by its late policy at dispatch (RM-14: `drop` flags `TIME_ERROR`). A `Stop` that comes after the abandon and cancels such a piece leaves the region `[t, B)` (RM-16). MockRadio has no transmit fault model, so it never abandons a burst.

Workaround (Fable: "its end plus four samples" is false behind a joined piece and after a late abandon; "the abandoned burst's booked end" must change):

> **Workaround:** book the burst untimed (it is placed after the held region, RM-15), or timed at or after the abandoned burst's front: its end plus four samples, or, when a burst was joined behind it, that burst's own front (its end plus four, or `g` past its padded end if it is short); after a late abandon, at or after the device's front. Or send a `Stop` for the stream first, which cuts the held region at its bound (RM-16) but also cancels every burst booked after the Stop's instant.

Evidence: the tests of §5 (`n99_u1_…`, `n99_u3_…`, `n99_u11_…`) are added beside `o1_uhd_t7b_…`, `e2a_kl39_…` and `o1_uhd_t7a2_…`; the `Stop` workaround stays "read from the code, not tested (INFERRED)" except the region it leaves, which U11 tests. Status: "Ceiling (owner, 2026-11-09: kept as it is; owner, 2026-11-13: the join at the abandoned burst's end ends with the abandon, and the footprint stays; shrinking the region to what was sent is the contract change C6 of design note 49, made only if the owner later approves it)". The title stays.

**KL-38 cross-reference** (the same framing, one sentence at the end of its *When*): "The same framing applies to the bursts accepted behind a burst that is later abandoned (KL-39)."

**Records:** note 49's Changes row (F4 / #99 closed by the record write; §5.4.4 item 2's "the finite booking keeps its range" stays true); E3b's dispositions: the F4 pointer; `plan/maintenance/failure-mechanisms.md`: #99 belongs to `planned-state-read-as-actual`, and F1 (an abandon written from a view that predates a halt) and F2 (a lookup by a list the receipt prunes) are findings of the orchestrator's count. No version bump, no `SCHEMA_CHANGELOG` entry (pre-freeze rule); no schema changes. Round trip: `design/` text is reviewed as the implementing diff.

### 5.7 Sub-steps (each leaves the tree green; one local commit each on `work/n99`; the step's start commit `cc8c3cb` is `--base` for `gate.py` and `rules.py`)

1. **Refactor, no behaviour change:** extract `Chain::left_without_head`; re-anchor N47S5-140/-141/-145/-146; every test unedited (VERIFIED on the sketch: all `staging` tests pass). The four rows run at this commit.
2. **The record:** `Bookings::end_finite`, `TxList::halted`, `TxBook::abandon(.., finite_end)` with `Tx::abandon` passing `None` (behaviour unchanged), the 7 call sites, P1–P9 (P8 replaces the old test), rows N99-01…14, N47S5-153 retargeted. Pure tests first, failing on the tree before the code.
3. **The Provider and the text:** `Tx::abandon` computes `finite_end` (the open booking's `k + record_len()`, or `Some(cursor)` with no open booking) and filters it with `is_lost()`; U1–U11 (U1 first, failing on the tree after sub-step 2: it fails today, §1), the C1 and C6 edits and the helpers' visibility; RM-15, UR-23, KL-39 and KL-38; rows N99-15…17 and N47S5-151, E1-U13, E2A2B-13 re-anchored; the records of §5.6.

Self-review checklist (step 5, once per sub-step, narrowed to its diff): every S-row of §5.1 has its test and the test observes what the row says; every "every / only / all" claim searched (`grep -n "abandon(" crates/ezsdr-radio-uhd/src/provider/tx.rs` lists the six production callers, `send` ×3 (611, 655, 669), `step` ×2 (400, 422), `report` ×1 (1140), all reaching `Tx::abandon`'s single record section at 883; the guard's "no abandon follows a read halt" argument of §4.2 re-read against `stop_now`); `design/` text made false (§5.6); rows match once after `cargo fmt --all`; `cargo clippy` clean; N99-01…03's shared anchor.

## 6. Searches (so each "not found" is rerunnable)

- `grep -n "abandon(" crates/ezsdr-radio-uhd/src/provider/tx.rs` (production lines before `#[cfg(test)]` at 1194): callers at 400, 422, 611, 655, 669, 1140, the definition at 870; the one call in the tests is `tx.rs:3695` (`Tx::abandon`, whose signature does not change).
- `grep -rn "abandon\|end_finite" crates/ezsdr-mock-radio/src`: no output (VERIFIED).
- `grep -rn abandon crates/ezsdr-radio-uhd/tests` (hardware and fake): only `infra.rs:50`, an unrelated comment.
- `grep -n "book.abandon(" crates/ezsdr-radio/tests/staging.rs`: 7 sites (1525, 1550, 1567, 1581, 1595, 1611, 2602).
- Callers of `TxBook::abandon` in `crates/`: `tx.rs:883` and the 7 test sites; `Bookings::abandon` (the repeat's) is called by `TxBook::abandon` and the tests only.
- Rows: a script over `plan/*/tools/mutations.json` with `anchor.apply` against the sketch's `staging.rs` and `tx.rs` (`probe/n99-mut.py` and `n99-rows.json`): the 8 rows of §5.4 stop matching exactly once on the unformatted and formatted sketch before re-anchoring; after it, 419 of 419 match.
- `git diff --stat 44eb9b2 cc8c3cb`: two plan files, no code.
- Read: `staging.rs` 91-143, 274-333, 686-1010, 1040-1238, 1348-1390, 1416-1482; `tx.rs` 217-283, 360-481, 544-708, 733-792, 870-902, 937-1000, 1024-1033, 1112-1160; `control.rs` 290-324; `core.rs` 425-460, 606-665, 1050-1075; `design/07` RM-13, RM-15, RM-16; `design/18` UR-21…UR-23; `design/09:272`; KL-36…KL-39; note 49 §3 R3, §5.4.4; `reviews/e3b-test-plan.md`, `e3b-dispositions.md`; AGENTS §1–§9; `fable-review.md`, `design-decisions.md`.

## Appendix A. Probe outputs (baseline = `cc8c3cb`; "sketch" = the amended reference sketch of §4.1)

Everything below ran in `/tmp/n99-probe` (the sketch plus `println!` probes; the debug-only switches `N99_NOGUARD` and `N99_OFF` that turned the guard and `end_finite` off are not in the sketch and not in the diffs). The first plan's Q1–Q13 (baseline, `probe/n99_uhd_probes.rs.txt`) are unchanged; Q1–Q13 were re-run on the amended sketch and every line of §5.1 comes from that run.

**Pure, Bookings level (`n99_pure_f1`, `n99_pure_f2`):**
- F1 (Fable's): `[Chain{10000..40010, joined [40000], front 40014, join true}]`; after `halt(10100, 20105)`: `front 20105, join false`, verdicts ` ERR@19000 ok@20105`; after `end_finite(40000)`: **unchanged** (`same-as-halted: true`): `40000 < front 20105` fails, the chain is not found.
- F1b: A 2 000 + B 10 joined, `halt(10100, 20105)`: `{10000..12010, joined [12000], front 12014, join false}`, ` ERR@10000 ok@12014`; after `end_finite(12000)`: `{10000..12000, front 12004}, {12000..12010, front 12025}`, ` ERR@10000 ok@12025` (the raise F1 is about).
- F1c: A, B 2 000 @ 12 000, C 10 @ 14 000: `ok@14014` → `ok@14025`.
- F2 prune: before `{10000..14010, joined [12000, 14000], front 14014, join true}`; `prune(12001)` → `joined [14000]`; `end_finite(12000)` → `{10000..12000, front 12004, join false}, {12000..14010, joined [14000], front 14025, join false}`, ` ERR@12000 ok@14025`, `accept(14010,4)` `Err`.
- F2 stale chain: `S = {0..300, joined [100, 200], front 70, join false}` after `halt(50, 70)`; X 220 @ 80 `{80..300, front 304, join true}`; `end_finite(300)` → S unchanged, X `join false`; `accept(300,10)` `Err`.
- PT1…PT8 of the first plan hold on the amended sketch (head 40 abandoned: `{0..40, front 44, join false}`, ERR@39..43, ok@44; head 40 + 10 + 4: `{0..40, 44}` + `{40..54, joined [50], front 69, join false}`; head 40 + 100: `{40..140, front 144, join true}`; head 40 + 10: `{40..50, front 65, join false}`).
- TxBook (`n99_pure_txbook_guard`): the F1b record with an unread `Halt`: record `==` before (guard on); with the guard off: the two split chains. With no halt: the two split chains.

**Rig (`n99_r1`, `n99_r2`, `n99_s9`, and the Q probes):**
- R1, guard on / off: A 2 000 @ 10 000, B 10 @ 12 000 booked before; the send at 0 ms takes 0.5 s (device time → 100 000 000 ticks) and a Stop is received in it (`now 100000000`). After the Stop: `[Chain{10000..12010, joined [12000], front 12014, join false}]`, region none. After the abandon: guard on, the same; guard off, `[{10000..12000, front 12004}, {12000..12010, front 12025}]`, `accept(12014..12024)` `Err`. `FailSend(0)`: the same. No B: `{10000..12000, front 12004, join false}` both ways.
- R2 (A open at 11 000 after 1 ms, B booked, device time to 11.5 ms, then the halt, then an `Underflow` report): Stop unread: guard on `E..12013 .12014`; off `.12025`; device lost: the same; no halt: `.12025` both ways.
- S9 (`n99_s9`): at 11 ms with `end_finite` off: `[{10000..12010, joined [12000], front 12014, join false}]`, region none, `accept(12014)` `Ok`; with it on: `[{10000..12000, front 12004}]`, region `(11000, 21005)`, `accept(12014..21000)` `Err`, `accept(25000)` `Ok`. At 13 ms: no change from the Stop (both chains start before t). At 5 ms: both chains cancelled, region `(5000, 15005)`, both ways.
- Normal timing (first R1, A 12 000 @ 10 000, B joined, Stop at the seam of the 11 ms send): the Stop's bound is 21 005, the abandoned booking ends at 22 000: the record after the abandon is the record the Stop left (F2's `e < c.front`).
- Q-probes on the amended sketch: Q1 (3 faults): `[after A abandoned] record {10000..12000, front 12004, join false}`, B refused, C sent `(4 sob eob)` front 12 033, no event; Q2: long B refused, `accept(12203,1)`/`12204` `Ok`; Q3: `{10000..12000, 12004}, {12000..12010, 12025}`, C refused; Q4: underflow/SeqError: `[(1000 sob),(2 eob)]`, front 11 017, `TX_UNDERFLOW`; expired timeline: `[(1000 sob),(999),(2 eob)]`, front 12 004; Q5 (`ShortSend(0..3)`): the last piece's chain is split or its join ended; Q7: `{10000..14000, joined [12000], front 14004, join false}, {14000..14010, front 14025, join false}`; Q8 (jump to 11.5 ms): `[(1000 sob),(2 eob)]`, front 11 017; Q9b (a late abandon, `FailSend(1)`): B @ 12 004 accepted and dropped at dispatch (P4, not fixed); Q10b `ShortSend(1..4)`: `ShortSend(1)` → `{10000..12000, 12004}, {12000..14010, joined [14000], front 14025}`; `ShortSend(3)` (C's own send) → one chain, `join false`, no split (the abandoned piece ends the chain); Q10c: `{10000..12000, 12004}, {12000..12010, 12025}`; Q11: `TIME_ERROR "drop"`, front 12 025; Q12: `{10000..14000, joined [12000], front 14004, join false}`; Q13: lock 3: B accepted at the seam then framed, lock 4: refused.

**Mutation rows in the debug copy (`n99-mut.py`):** N99-01…17 and the eight re-anchored rows: every one `killed` by the test the §5.4 table names (N99-01 survived the first P7 and was killed after P7 gained `verdicts(&ended, 44, 4_900) == " ok@44"`).

**Suites on the amended sketch (formatted, clippy clean):** `ezsdr-radio-uhd` lib 616 ok (the two expected failures of the unedited tests gone with their edits), integration (fake) 433 ok; `ezsdr-radio` all ok (staging 174); `ezsdr-mock-radio` all ok; `ezsdr-acceptance` all ok (parity 70, 109 s).
- owner 2026-11-13: #99 direction B (the abandon is an owner's close for the record; KL-39 footprint kept; B at e refused UR-21). Fable 5.2 review: sound with changes (F1, F2, the text corrections), adopted by the orchestrator (`fable-review.md`, `design-decisions.md`).
