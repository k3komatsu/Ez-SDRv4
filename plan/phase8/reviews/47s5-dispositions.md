# Note 47 stage 5 — overlapping bookings refused at receipt, the repeat switch, preemption removed, untimed bursts: dispositions (2026-11-07)

Risky (a queue, deadlines, a lifecycle, the boundary between FakeDevice/MockRadio and the X3x0).

Order of work:
1. Brief, the implementer's test plan (Haiku 5.5 `anthropic/claude-haiku-5-5#high`, OpenCode2), and the orchestrator's rulings.
2. Sol plan review (PLAN NEEDS CHANGES), then rulings and Opus re-checks of plan rev 3 and rev 4. The second re-check gated 5b-2a on its major A.
3. Sub-steps 5a, 5b-1, 5b-2a, 5b-2b, 5b-3, 5c and 5c-2, then 5d-M.
   - The owner then decided that later work runs as one local commit per sub-step on `work/n47s5`, with a cheap self-review of each sub-step and Sol once at the end (AGENTS §9 step 5).
   - The earlier sub-steps' commits were rebuilt from OpenCode's worktree snapshots.
4. Sub-steps 5d-U, 5e, 5f, then 5g, 5h, 5i and 5j.
   - 5g, 5h and 5j come from owner decisions made during implementation.
   - 5i settled a misread side finding.
5. Ten sub-step self-reviews (free DeepSeek, then Go DeepSeek once the free quota ran out), 57 findings, and four self-review fix rounds.
6. Sol code review (FAIL, 4 × P2), the Sol fix round, then four Opus re-check rounds (FAIL, FAIL, FAIL, PASS WITH MINORS) with fix rounds 5–8, then the gate.

Records:
- `47s5-brief.md` holds all briefs.
- `47s5-test-plan.md` is rev 35; its §8 has deviations D1–D80 and the per-round records.
- `47s5-sol-plan.md`, `47s5-sol-code.md`, `47s5-opus-rechecks.md`, `47s5-selfreview.md`, `47s5-gate.json` and `47s5-renames.txt`.

## What stage 5 does

- **One receipt-time booking record** (`ezsdr_radio::staging::Bookings`, replacing stage 4's `Busy`). MockRadio and the UHD Module only call it. Invariant (RM-15): a booking accepted at receipt plays at its (possibly policy-moved) start on the Module unless the host is late, or an earlier burst's configure is refused (KL-38).
- **What the record decides:**
  - overlaps, in either receipt order;
  - the device-burst framing: chains whose head has more than 32 samples, and fronts of `start+5` for 1–3 samples, `end+1` for 4–32 and the chain's `end+2`;
  - the held-tail close at `J(e)` (the UHD owner closes under the streams lock);
  - the Stop region `[t, B)`, which applies only when a cancelled booking starts in it;
  - the hand-over horizon: a booking that starts before one the Provider may already have handed is refused (KL-37);
  - the one placement rule `place` (SendAsap and untimed);
  - the repeat switch's pending boundary.
- **Preemption is deleted.** A dispatch-time move that meets another booking is refused. `core.held` is removed.
- **The repeat switch** (RM-37, MR-42, UR-38):
  - It is `start_repeat` carrying the metadata mark `radio.repeat_operation = "switch"` (owner: no separate verb), and Python `Tx.switch(x)` sends it.
  - The boundary is chosen at receipt.
  - A switch the owner has handed past follows its late policy (owner): SendAsap moves it to the next boundary with `TIME_ERROR{send_asap}`.
  - A Stop, a loss, `Provider::stop`, F8, or a repeat the Provider abandons cancels a switch not yet reached.
  - Records go in the `switches` section.
- A second plain repeat on a playing stream is refused.
- **Untimed bursts** (owner):
  - The Kernel carries `Dispatched.untimed` on every Action, and an untimed action has no `requested_at`.
  - An untimed TxBurst starts at the first sample its stream can take, past the lead, a Stop region, accepted bookings and the horizon. It reports no `TIME_ERROR`; it is refused only when an open repeat holds the stream.
  - A timed SendAsap meeting only a Stop region moves with `TIME_ERROR{send_asap}`.
- **Python:** `repeat; stop; repeat` transmits.
- **#79:** every divergence has a parity test that passes on both Providers.
- **Specs:** RS-19 and RS-51 (design/04), MA-14 (05), KC-28 (06), SC-23a (02), RM-12–16, RM-21 and RM-37 (07), MR-16/17/25/27/32/42 (09), EA-16 (16), and UR-21/22/23/30/36/38 (18). KNOWN_LIMITATIONS: KL-13, KL-15, KL-36, plus the new KL-37 and KL-38.

## Owner decisions during the stage (`owner-decisions.md`)

| Date | Decision |
|---|---|
| 2026-11-07 | The in-flight window stays for finite bursts (before the stage; note 47 §4.5). |
| 2026-11-07 | Sub-step self-review and local commits on a work branch, with Sol at the end (AGENTS §9). |
| 2026-11-07 | D60: no `switch_repeat` verb. The switch is the mark on `start_repeat`, and the Kernel is unchanged. |
| 2026-11-07 | S12: a switch handed past its boundary follows its late policy instead of being refused (5g). |
| 2026-11-07 | B-first: a booking before one the Provider may already have handed is refused at receipt (5h). |
| 2026-11-07 | F36 then untimed: an untimed burst starts when its stream can take it, with no TIME_ERROR. The Kernel carries "untimed" to the Provider (5j). The owner asked whether the problem came from the design; it did (see Mechanisms). |
| 2026-11-07 | Adopted: (1) "untimed" is a general Kernel property; (2) one table of the record's rules; (3) refusals are checked against user-facing promises in briefs; (4) RM-15's principle. Device state → the late policy decides; a conflict with another accepted booking → refusal; untimed → the first free sample. |
| 2026-11-07 | After the stage, take inventory of the time and booking model, including removing features users rarely need (the owner decides each one). The actor split is evaluated for simplification and separation of responsibility, not speed, in the owner's two-level form: a controller keeps the mutable schedule and commits to a one-way FIFO executor. |
| 2026-11-07 | A design phase (AGENTS §9 step 0) runs before long-lasting stages. An Opus subagent drafts at effort `max`, the owner settles the direction first, and Sol then reviews and proposes alternatives. The orchestrator may edit records while an implementer runs. |

## Sol plan review (PLAN NEEDS CHANGES; 9 × P2, 1 × P3 group; all adopted, `rulings-sol-plan.md`)

- The record decides the device-burst framing and the UHD owner follows it: chains, fronts, and the close instant (Sol 1, 2).
- A dispatch move into another booking is refused (Sol 3).
- The switch's cursor is exclusive (Sol 4). There is one logical pending rule for the switch (Sol 5). The other findings and the P3 group are folded in.

## Opus re-checks of the plan (rev 3: majors M1–M4; rev 4: major A and minors m1–m5; all adopted, `rulings-recheck.md`)

- **M1:** the close at `J(e)` happens under the lock.
- **M2:** halt's region.
- **M3:** the switch's continuation branch.
- **M4:** `move_start` takes the length.
- **Major A:** the Stop region applies only for a cancelled booking that starts before `stop_k`.

## Sub-step self-reviews (10 reviews, 57 findings F1–F57; `47s5-selfreview.md`)

Severity and outcome:
- By severity: 0 P1, 15 P2, 42 P3.
- 51 fixed or folded in.
- Rejected with evidence:
  - F12: its geometry is refused at receipt.
  - F20: unreachable from receipt; release refuses instead of panicking.
  - F31: `accept` sees no stale switch.
- F36 became the owner's untimed decision.

Real behaviour defects found:
- F10: a pruned head moved untracked.
- F21: a refused switch overwrote `playing`.
- F30: the Mock never settled an applied switch.
- F36: `repeat; stop; repeat` was refused.
- F39: a moved switch's record was not marked late.

## Sol code review (FAIL; 4 × P2; all adopted, `rulings-sol-code.md`)

| # | Finding | Disposition |
|---|---|---|
| S1 | `Chain.last` kept only the newest joined piece, so the owner closed before an earlier accepted join | every join point is kept (`joined`), and passed ones are pruned |
| S2 | a configure-refused head left invalid framing for the surviving pieces | after re-checks: UR-22 kept (no open first send under 32); a short surviving head that is alone is closed; retained pieces keep their regions; pieces joined behind a closed short head follow their late policy (KL-38) |
| S3 | the Mock settled a switch before an earlier loss in the same step | switches are settled in replay order |
| S4 | an underflow abandon left the repeat booked, and a switch pending forever | the record ends the repeat at the cursor (`stop_at(…, 0)`); an abandon cancels switches not yet handed, including one joined but not yet opened; a late-dropped repeat leaves the record; a finite abandon keeps the switch |

## Opus re-checks of the Sol fixes (four rounds)

1. **FAIL.** The S2 fix left a 2–32-sample first send open, which breaks UR-22. The fake hid this by clearing same-tick packets. The S4 stop point was one window late.
2. **FAIL.** The S2 front shrank while pieces remained. The S4 cursor at the boundary was settled `applied`.
3. **FAIL.** An abandon in the gap between `join_open` and the switch's open left the repeat booked.
4. **PASS WITH MINORS.** Three host-late record cases were fixed in round 8 without a re-review (minor only).

## Gate

`47s5-gate.json` is the passing run (run 3). Two runs failed before it:

1. **Run 1:** rules (21 problems, run without `--renames`); the tree changed during the run (the orchestrator edited records); 3 mutation rows did not compile and 16 survived.
   - Gate fix round 1 settled all 19 rows: re-anchored, retargeted to the test that sees them, strengthened, or deleted as equivalent with evidence. P653 and phase3 M78 were deleted (test plan rev 34 §8).
2. **Run 2:** everything passed except two problems.
   - rules: one broken relative link in `47s5-selfreview.md`, which the orchestrator fixed.
   - Mutation row N47S5-143 survived. The untimed-switch test's receipt bound sat about 40 samples short of a 1 000-sample grid point, so under load it could land on the grid, where the mutant gives the same boundary.
   - Gate fix round 2 changed the test to a 1 024-sample period, so the bound lands about 400 samples into the period. Probe: the mutant is killed 10/10 unloaded and 8/8 under load (test plan rev 35).
3. **Run 3:** PASS.

## Counts and mechanisms

- Sol plan: 9 P2, 1 P3 group.
- Opus plan re-checks: 4 majors (M1–M4) and 1 major (A), plus 10 minors.
- Self-review: 0 P1, 15 P2, 42 P3.
- Sol code: 4 P2.
- Opus fix re-checks: 6 majors over three FAIL rounds, then minors.
- Owner decisions mid-implementation: 4. These were the switch verb, S12, B-first and untimed; each came from a brief ruling made without alternatives or a check against the user contract.

Mechanisms (`plan/maintenance/failure-mechanisms.md`):
- **`planned-state-read-as-actual`** covers the receipt-time record (a prediction of the device) disagreeing with the owner's actual lifecycle. Instances: Sol S1, S2 and S4; re-check rounds 1–3 (5 majors); self-review F10, F21 and F30; Sol S3 (replay order).
  - This mechanism was already past three strikes. It is answered by the time and booking inventory the owner ordered for after the stage, where the controller-commits-to-executor split removes the second copy of the state.
- **New, `refusal-where-late-policy-decides`:** the orchestrator's rulings refused a booking that device state, not a conflict, kept from its start. Instances: S12 (owner) and the Stop region (F36, self-review then owner). Answered by RM-15's principle sentence and the design phase (AGENTS §9 step 0).
- **`wall-clock-test-oracle`:** the S4b parity margin (F28) and the repeat-after-stop Python test (bounded assertions).
- **Tooling:** the watchdog misread rows whose `new` text is a bare literal as applied mutants whenever their anchor moved. Fixed by making those rows specific (fix round 4).
