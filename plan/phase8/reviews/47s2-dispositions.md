<!-- rulings-s2-plan.md -->
# Stage 2: orchestrator rulings on the implementer's plan questions (2026-11-04)

1. **`FromNone` for an idle stream with channels stays** in stage 2 (a `cold` change of an idle stream, e.g. `sample_rate_hz`, opens it). Only "a channel enters an empty stream" is membership and goes now. Stages 3–4 reshape the idle-stream path.
2. **Python:** the client refuses `.stream = …` before sending (the attribute can never succeed); the Kernel's admission refusal is pinned through the low-level `sdr.set` and in Rust.
3. **`ur_24_*`/`ur_25_*` tests that used a runtime `stream` update only as a stimulus:** retarget to a stimulus that still exists (a timed frequency/gain update, a rate change) when the behaviour they pin stays reachable in stage 2; delete only the tests whose meaning is the membership interplay itself. Reason: the timed-release code lives until stage 3, and its mutation rows must keep a killer (AGENTS §9: remove coverage only when the failure case is unreachable or covered elsewhere). Stage 3 deletes them with the code.
4. **The extra dead code** (`held_elsewhere`, `needed_before`, `move_source_busy`, `await_release`, `release_unneeded`, `release_first`, `Holder::Owner { seq }`'s `seq`): remove in this stage, each with the search showing no remaining caller.
5. **Mutation rows:** decide row by row by anchor (as the plan says); a row whose anchor stays but whose killer is deleted gets a new killer, never deletion.

<!-- dispositions-s2-plan.md -->
# Stage 2 plan review: dispositions (orchestrator, 2026-11-04)

Sol: PLAN NEEDS CHANGES (`sol-s2-plan-final.md`), 2 major, 5 minor. All adopted.

| # | Severity | Finding | Disposition |
|---|---|---|---|
| 1 | major | retained-path retargets not ready: a TX rate change after a burst/Stop does not reproduce the deleted enable/disable stimulus (TX keeps its clock, stays Active); Appendix A marks retained tests D | adopted: before code, resolve every retained-path test and row individually: the reachable prefix (RX Stop/owner completion, TX configure-refusal/owner completion…) and the asserted holder, phase, generation, report correlation; replace name-based D classifications with per-test decisions; Appendix B becomes a per-row table |
| 2 | major | S3 expects closes after loss, which loss safety forbids | adopted: split normal stop (`close_streams`) from loss (no device operation after loss, no cleanup close, sections completed, claims returned through the real owner/control exits — not a manual `give_back`) |
| 3 | minor | U5's `Arc<Barrier>` is an F8 recovery barrier, not membership | adopted: keep it and the four recovery orderings; only adapt `Holder::Owner` |
| 4 | minor | short-lead retarget has no TX channel; sample count at 2 Msps wrong | adopted: TX c0 assigned initially; keep 1 Msps or recompute (−49 ms = −98 000 samples at 2 Msps) |
| 5 | minor | Python `sdr.set` raises `Rejected` | adopted: catch `Rejected` and inspect its entry (or `submit`), close before reading the Manifest |
| 6 | minor | UR-34 is a bench obligation, not membership | adopted: keep UR-34; the `ur_34_*` membership tests belong to UR-25 |
| 7 | minor | S2 equality-only checks miss routing | adopted: `{0,2}` layout, channel-distinct samples, order, stopped-stream silence, peer continuity |

<!-- rulings-s2a-plan.md -->
# Stage 2a: orchestrator rulings on the implementer's plan questions (2026-11-04)

1. **`FromNone` goes in 2a.** After 2a it is reachable only after a refused TX change, a case stage 4 redesigns anyway; keeping it costs ~16 retargets and ~16 rows on code about to go. The idle TX owner then configures at its segment start. Pin that new behaviour with one sequence (TX rate change refused by the device → a corrected TX change → the next burst is configured and sent; MockRadio and FakeDevice agree), and amend the UR/MR text that describes from none. Sol reviews it as a TX behaviour change inside 2a.
2. **`timeline::covering` (RM-36) goes in 2a** (its callers are all receive).
3. **KNOWN_LIMITATIONS KL-3 (timed receive retune part) and KL-11 are edited in the same change** (AGENTS §2).
4. **`layout_instant`'s `start` parameter and `command_instant`'s `cold` flag**: remove when dead, each with the search.
5. **The 31 CHECK/SPLIT items** are read and decided in phase 2 before their code changes; the plan's tables are updated with the decision.

<!-- dispositions-s2a-plan.md -->
# Stage 2a plan review: dispositions (orchestrator, 2026-11-04)

Sol: PLAN NEEDS CHANGES (`sol-s2a-plan-final.md`), 3 major, 4 minor. All adopted. Ruling 1 of `rulings-s2a-plan.md` is revised (item 1 below).

| # | Severity | Finding | Disposition |
|---|---|---|---|
| 1 | major | ruling 1 (`FromNone` deleted) not incorporated or tested | adopted, and the ruling is reshaped per Sol's size note: **2a-1** = RX classless + RX deletion, `FromNone` kept; **2a-2** = `FromNone` deletion with the new TX retry sequence (device refuses TX rate change → owner idle → corrected change → next burst: refusal-clock end, corrected clock registered, payload sent, claims returned, configure before payload; both Providers through real booking/admission; T3/F8 tests on their surviving open/recovery routes). Two code reviews (after 2a-1 and after 2a-2), **one gate** at the end, so `FromNone` tests are not retargeted in 2a-1 only to be deleted in 2a-2. The plan gets a 2a-2 section now |
| 2 | major | name/direction heuristic deletes retained behaviour (`mr_21_overrun_shape`, `ur_12_every_advertised_rate_is_accepted`, clipping) | adopted: every DELETE is decided by the test's subject after reading it; keep and trim tests whose subject survives; a row is deleted only when its anchored code is deleted, not because incidental context vanished |
| 3 | major | S6 initial-value retargets cannot reach first-open / rate-set comparison / Mock refused-start | adopted: name reachable setups; keep UHD owner-configure refusal/retry and peer isolation; retire unreachable first-open cases with evidence, keep arm/recovery comparison; decide Mock's refused-start modelling explicitly |
| 4 | minor | `ready` scenario needs a new mechanism | adopted: deferred to stage 3 (note 47 permits); P1402 and S7's new mechanism dropped; current start-lead tests kept |
| 5 | minor | short-lead target not pre-origin; ended-clock replaced by run-state rejection | adopted: a genuinely pre-origin target with recomputed assertions; ended clock via a refused TX change while Running, or a focused Kernel fixture |
| 6 | minor | `ColdConfig.configured` left without a reader | adopted: remove marker/accessor and its tests/rows; keep the snapshot/projection lock and `Streams`' record |
| 7 | minor | KL-7/9/32/34, UR-23/36, H4 text incomplete | adopted: inventory and amend, keeping limitation ids and historical evidence |

Process: the implementer amends the plan and stops; Sol re-checks the corrected dispositions before any bulk deletion (Sol's size note: freeze dispositions first).

<!-- dispositions-s2a-plan2.md -->
# Stage 2a plan re-check: dispositions (orchestrator, 2026-11-04)

Re-check reviewer (Opus subagent, `opus-s2a-plan2-final.md`; its 60 direct hits `opus-s2a-miss3.txt`): PLAN NEEDS CHANGES, 3 major, 3 minor. All adopted.

| # | Severity | Finding | Disposition |
|---|---|---|---|
| M1 | major | ~120 tests missing from Appendix A (RX timed updates, `Ma1::tune`, deleted-helper callers, RX `Kind::Cold` units, Python, class tables, `--features uhd` bench tests) | adopted, with a **method change**: the name/regex inventory is replaced by the compiler and the test run. 2a-1 is implemented in the tree; every test that no longer compiles or fails is listed with its subject and decided by subject (KEEP-TX-KEY, KEEP-INITIAL, rewrite, DELETE only when the behaviour it pins is gone); the re-check's list is the minimum that list must contain |
| M2 | major | wrong DELETEs whose subject survives (mock_radio.rs:3434/3475/3499/3570, 1971; mock_layout.rs:2598; control.rs:3246; fake.rs:3889, 8051, 2987, 1425; v58.rs:567); 153 "KEEP + NEW KILLER" rows name no killer | adopted: each listed test as the re-check decides; every kept row names its killer test, probed out of tree |
| M3 | major | KEEPs that cannot pass (control.rs:2734, timeline.rs:1951, mock_layout.rs:3560, rx.rs:2115/1484, rx.rs:3940/3960/3991/4011/4346) | adopted as the re-check decides (rewrite as refused `start_rx` / Stop then `Kind::Start`; delete covering- and debt-only tests, moving surviving assertions to named tests) |
| n1 | minor | from-none tests outside the 29; KEEP-TX-KEY setups using `channels_from_none` | adopted: ignore list extended; named TX setups (pending segment of a booked TX rate change) |
| n2 | minor | S10: T3 not on the owner route; reasons differ between Providers; the UHD test must wait for the refusal; duplicate Mock tests; `ready` = booking time until stage 4 | adopted: drop the T3 clause (or add a refused-open variant), normalise reasons as `parity.rs:916-923`, wait for the refusal event before the corrected change, name the existing Mock tests as killers, record the start-lead risk as INFERRED |
| n3 | minor | KL-24's workaround refused; KL-33 obsolete | adopted |
| — | note | leftovers: plan:163 `ready`; control.rs:7327 `is_configured()` | adopted |

<!-- dispositions-s2-a.md -->
# Stage 2 part (a): dispositions of the re-check (orchestrator, 2026-11-04)

Re-check reviewer (Opus, `opus-s2-a-recheck.md`): NEEDS CHANGES, 3 major, 6 minor, notes. All adopted.

| # | Sev | Finding | Disposition |
|---|---|---|---|
| M1 | major | H4 deleted whole; note 47 keeps TX headroom/load | restore h4.rs except the dispatch proxy; probe asks rx/tx c0 at 10 MS/s at connect, repeat without timed updates, records headroom and CPU/thread load; restore the 12 unit tests and the 2 `tx_start_lead_*` units that pass with the flip; table rows |
| M2 | major | only pins of surviving behaviour deleted: refusal halt of a later stream alone / successful change clears the halt (fake.rs@HEAD:7564, 8218), start lead over a long receive call (fake.rs@HEAD:3889), fault between clocks (mock_radio.rs@HEAD:1971), D14 action targets (mock_layout.rs@HEAD:570) | REWRITE-INITIAL with the stimuli the re-check gives (Stop then `start_rx`; WrongRate fault; 390 625 S/s, block 65 536 at six phases; fault in the Stop gap; D14 target refusals and per-stream Stop/`start_rx`) |
| M3 | major | uhd-control VH differential deleted | restore `control/differential.rs`, filtered like the Mock side (Stop/StartRx/loss); `rm_26_the_ceiling…` stays deleted |
| m1 | minor | `vh_8` TX rows dropped on a reason that does not reproduce | keep TX rows; compare origins relative to each rig's arm (or lattice + (direction, stream, sequence, channels)) |
| m2 | minor | REFUSAL tests do not check why | assert `ezsdr.update_class`, the failure reason (`radio.rf_envelope` for connect refusals), the log entry, a control case for v58.rs:1027 |
| m3 | minor | tests passing because their stimulus is refused (mock_layout.rs:1001, 1074; fake.rs:3218; fake.rs:6282 first assertion) | the receive one rewritten with Stop→`start_rx` asserting `rejected()` empty; the others go to part (b) with their code |
| m4 | minor | bench tests recording refusals as data; failing bench tests; MA-1 missing from `?` | rewrite with initial values (`tx_only`, initial 50 MS/s with a Stop as the cut, a pending segment by `start_rx`); park the rest explicitly with a one-line reason, including `hw_ma1_phase_after_a_pre_start_tune` |
| m5 | minor | lost halves (overflow companion of fake.rs@HEAD:8826–8834; tail of `mr_21_overrun_shape`) | restore with Stop-based stimuli |
| m6 | minor | table errors; stale comments/text; unused `_PARAMS` converters | fix the table; clean the stale text; remove unused converters |
| notes | — | start-lead unmeasured until stage 3; generator still emits Cold/Timed; rx.rs:587 mislabels a start_rx refusal; part (b) must keep Mock value history, rx/differential layer 3, control.rs:5430 D14 twin; stage-3 `ready` scenario of fake.rs@HEAD:2783 | part (c) records the start-lead gap; part (b) removes Cold/Timed from the generator and rebalances it; rx.rs:587 and the `ready` scenario go into the stage-3 brief; the keep list goes into the part (b) brief |

<!-- dispositions-s2-a-fix.md -->
# Dispositions: Opus re-check of the stage 2 part (a) fix round (opus-s2-a-fix-recheck.md)

Verdict NEEDS CHANGES, minor only -> no further re-review (AGENTS §9). Fixes go first in part (b) (brief-s2-b.md item 0).

| # | Finding | Disposition |
|---|---|---|
| 1 | M2(c) half vacuous: mock_radio.rs:1928 reads `domain` before the new clock registers | Adopt: read after `step(20_000_000)`, assert two clock records, first-block assertions on the new clock; fix the "10 ms" comment (1.01 ms) |
| 2 | m3 hand-off names the wrong test; `mr_34_a_layout_deterministic_transmit_entry_keeps_phase_zero` (mock_layout.rs:1123) is vacuous | Adopt: delete it (a transmit entry has no Run-time form until stage 4); fix the rev-4 row |
| 3 | m2 missed common/mod.rs:873 (B7 rehearsal accepts any Rejected) | Adopt: assert `ezsdr.update_class` |
| 4 | hw_56 lost its subject (plain restart) | Adopt the rewrite: Stop, `start_rx` (e2 = its rx_segment origin), a second Stop ~10 ms later, then the current drain and recovery, keeping the e1/e2 placement field. Compile-only (no bench now) |
| 5 | Nit fake.rs:5806 drains before the Stop | Adopt: drain just before `start_rx` |
| Q3 | m1 lattice check misses a wrong initial origin before T0; `% 100` looser than stream 1's 200 | Adopt the tightening: `origin == lattice(arm, n) - T0` per rig (UHD: timing row "arm".at; Mock: arm = 0, as vh_8 does) |
| Q2 | `tx_start_lead_ns` governs nothing reachable | Keep the key; stage 4 (TX settings on idle streams) decides. Part (b) does not delete it |
| N1 | h4.rs staged D + untracked | Orchestrator `git add`s it at commit |
| N2 | `ProbeDevice::matching_timed_apply` only used by its own test | Part (b) deletes it with that test |
| N3 | design/18 UR-34 cites hw_h4_dispatch | Part (c) |

<!-- dispositions-s2-code.md -->
# Dispositions: Sol code review of note 47 stage 2 (sol-s2-code-final.md)

| # | Finding | Disposition |
|---|---|---|
| 1 (P2) | UHD no longer ends a TX clock whose plan is cut at its origin (loss or `Provider::stop` at S ≤ O); HEAD's `Streams::reconcile` did, and it was the only production TX `clocks.end` | Adopt (overrides the orchestrator's acceptance in orch-review-s2.md: the end of a clock is on the must-survive list, and the fix is small). Restore a single-clock reconciliation in `Streams` that ends the TX clock at its segment's cut instant when the plan acquires a cut, at booking (covering loss paths that bypass the owner). Tests on the controlled clock: S < O and S = O end the clock at O (`ended_at`), a stop after O leaves it open; a row for the end |
| 2 (P3) | Specs still describe deleted machinery: design/09 MR-15 (cold transmit change), MR-34 (timed-tune constants), test-table rows at 09:135, 204, 325, 347, 364 and 18:530, 567 describe generated membership changes, timed updates and release observations | Adopt: remove the obsolete clauses, rewrite those rows from today's assertions; history stays in Changes tables. Also sweep the rest of 07/09/18 test tables for the same (c2 said it fixed only the obvious ones) |
| note | Multi-pending-fault ordering mismatch stays outside the generator (Loss has at most one non-loss fault) | Pre-existing; record as an open item (issue) after the gate |

