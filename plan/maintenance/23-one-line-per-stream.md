# Maintenance note 23 — One timeline per stream on the UHD Module

| Field | Value |
|---|---|
| Status | **Proposed**, awaiting the owner. A design note under AGENTS.md §6's three-strikes rule, written by the orchestrator (Claude Opus) on 2026-10-08 after spec 22 step 3. No code changes until accepted. |
| Mechanism | The refusal round trip between uhd-control and uhd-rx: `planned-state-read-as-actual` and `correlation-by-partial-key` ([failure-mechanisms.md](failure-mechanisms.md)). |
| Scope | `crates/ezsdr-radio-uhd/src/provider/{control,rx,tx,mod}.rs` and VH-8's layer 3. No change to `ezsdr_radio::timeline`, to MockRadio or to any rule in `design/`. |

## The bugs

Five bugs of one mechanism in two days, all in how a receive plan reaches uhd-rx. All were found by review or by layer 3, not by users:

1. Spec 22 step 2, review B1: after the device refused a segment's rate, uhd-rx began the next segment where the old plan had placed it, and the stream went silent.
2. Step 3, fix 2: the wait for a replanned segment ended on a plan made before uhd-control knew of the refusal (seed 167).
3. Step 3, fix 3: a refused segment counted as begun, so an earlier replanned segment was skipped (seed 317).
4. Step 3 review, B1: uhd-control's half of the fix-2 protocol was tested by nothing.
5. Step 3 fix round: a loss ended the stream at a later cut from a plan that predated the loss (seed 118).

Step 3's fix 4, where an orderly stop cut nothing until the plan arrived, is the same shape on `Provider::stop`.

## The cause

Each receive stream's planned state exists twice:
- uhd-control holds the `Line`, with its items and its plan;
- uhd-rx holds the last plan uhd-control sent it.

Messages keep the two in step (`RxCmd::Plan`, `RxCmd::Cut`, `streams.refused`). Sometimes uhd-rx learns of an event first: a configuration the device refuses, a loss, or `Provider::stop`. From then until uhd-control's next plan arrives, uhd-rx acts on a copy it knows is out of date.

Each fix closed one such window by adding one more correlation field:
- the refused list in `RxCmd::Plan`;
- the `known` set;
- the `refused` wait;
- `stopping`;
- `lost_at`;
- `refusals()` at `finish`.

The next kind of event will open the next window.

## The question

Can each stream have one copy of its planned state? MockRadio already works that way: one `Line` per direction, booked on receipt, read by the step loop.

## Recommendation: A — one shared `Line` per direction, booked by whichever thread learns the event

- **Shared state.** Each direction's `Line` sits behind a mutex that uhd-control, uhd-rx and uhd-tx share.
- **Who books what.**
  - uhd-control books the commands, as now.
  - uhd-rx books what only it learns: it marks a refused segment's item `refused`, and it books a loss it detects as `End`, with `Item.delivered` set to the first sample not yet delivered. The field exists for exactly this (VH-3).
  - `Provider::stop` books `End` directly.
- **How uhd-rx reads it.** At each turn, uhd-rx takes two things under the lock: the next segment and the current segment's cut, both from the one plan. It holds the lock only to book and to read, never across a device call.
- **What goes.**
  - `RxCmd::Plan`, `RxCmd::Cut` and their transmit counterpart;
  - the refused list, the `known` set and the refusal wait;
  - `streams.refused` and uhd-control's `refusals()` poll;
  - the loss and stop fallbacks in uhd-rx.

  The delivered floor stays with uhd-rx, which alone knows what it delivered. It reaches the plan as `Item.delivered`.
- **What it gives.**
  - A whole class of windows goes away: a plan can no longer be older than an event its reader knows of.
  - The UHD Module takes MockRadio's structure, which serves parity (`radio-rule-implemented-twice`).
  - Code is deleted. INFERRED: 150–250 lines across `rx.rs` and `control.rs`.
- **Costs and risks.**
  - One lock acquisition per uhd-rx turn. INFERRED to cost microseconds against a receive call of milliseconds; the bench measures it.
  - Every booking replans from the start of the Run until #63 prunes the items. The lock makes that cost visible on the receive thread, so #63 should land first or alongside.
  - Layer 3 changes from handing plans to booking items into the shared `Line`. Its oracle, the timeline, does not change, and the plan latency stays as a turn delay before uhd-rx reads.
  - The mutation rows that pin the message protocol retire and name their replacements: J18, J24, J27 and J28, and possibly J22's successors.

## Rejected

- **B. uhd-rx owns the receive `Line`; uhd-control sends items, not plans.** uhd-control needs the receive plan when it books: `coerce` over the projected configuration, `LATE_COMMAND`, timed updates on the channels in force at their instant (#61), and the recorded plan row. It would need a copy again, or a query round trip.
- **C. Version numbers on plans.** uhd-rx would act only on a plan whose version covers every event it reported. This is fix 2 generalised: it keeps two copies and the waiting, and each new kind of event needs its own handshake.
- **D. Keep the protocol, now that it is pinned.** That means five bugs of one mechanism, found only because layer 3 models a plan latency of 0–2 ms. Production has other latencies: a 100 ms receive call, a slow control call. The rule asks for this note precisely to stop that.

## If accepted

A decision record and its implementation would form one step, with:
- an Opus implementer and a separate Opus reviewer;
- layers 1–3 as the gate;
- the touched mutation rows during the fix rounds and the full lists once at the end (AGENTS.md §7).

#63, pruning `Line.items`, goes with it.
