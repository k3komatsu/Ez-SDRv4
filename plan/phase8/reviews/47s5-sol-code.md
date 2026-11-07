# Note 47 stage 5 — Sol code review (openai/gpt-6.1-sol, read-only, 1bc67d3)

# Scenarios considered

- Multiple contiguous bookings arriving between the owner’s command drain and tail-close check.
- A refused or moved chain head leaving short joined pieces behind.
- Stops before/after hand-over, repeated Stops, and bookings in the drain region.
- Switch installation below/equal/past its boundary; cancellation and subsequent switches.
- Underflow/recovery followed by another switch.
- Untimed placement, dispatch-time movement, Kernel dispatch paths, replay representation, and required schema fields.

All paths below are relative to `sol-tree`. Findings are statically traced counterexamples, not executed probes.

## P2 — must fix

### 1. A third joined piece makes the owner forget the first continuation
**`crates/ezsdr-radio/src/staging.rs:230,239`**, **`crates/ezsdr-radio-uhd/src/provider/tx.rs:492–496,538–546`**

`Chain.last` remembers only the newest joined piece. `continuation_open(e)` therefore misses earlier accepted joins.

Minimal sequence, at 1 MS/s:
1. A occupies `[20000,20040)`; the owner sends its body and holds its tail.
2. After the owner drains commands, control accepts B `[20040,20080)` and C `[20080,20120)`. Their commands have not been drained yet.
3. Before `J(20040)`, the owner checks the tail at `20040`.

The record now has `last=20080`, `end=20120`, so `continuation_open(20040)` returns false. The owner closes A. When B reaches the owner, its accepted start lies before the resulting device front; B is dropped or moved despite ample lead and no host lateness. MockRadio plays B.

**Fix:** retain enough joined-boundary information to recognize every accepted continuation awaiting hand-over.

**Test gap:** `tx.rs:2833` and mutation `N47S5-57` cover only one joined piece. Extend that sequence with C accepted before B’s command is drained.

### 2. Removing a refused head leaves an invalid continuation chain
**`crates/ezsdr-radio/src/staging.rs:465–470`**, **`crates/ezsdr-radio-uhd/src/provider/tx.rs:600–602,637–657`**

`remove_head` changes only the chain’s start. It retains the old joinability and framing, although the next piece may now be a short standalone device burst.

Minimal sequence:
1. Book A(40), B(10), C(4), contiguous and early, all accepted.
2. A’s configure fails once without device loss. A is removed.
3. B’s retry succeeds well before its deadline.
4. B starts a new device burst. Its short first send closes it.
5. C starts exactly at B’s payload end.

C was accepted as a continuation, but B’s standalone close requires an extra sample. C is dropped or moved at dispatch without host lateness.

The recorded host-lateness consequence for removing a head does not cover this configure-refusal sequence.

**Fix:** reframe the surviving pieces when a head is removed, or make the owner preserve the framing the record promised.

**Test gap:** the configure-refusal test at `tx.rs:6038` removes a standalone four-sample booking; it does not exercise surviving joins.

### 3. MockRadio applies a switch before processing an earlier loss
**`crates/ezsdr-mock-radio/src/lib.rs:1915–1923,1981–1990`**

`advance(u)` settles switches at the final step time before replaying faults earlier than `u`.

Minimal sequence using the existing ideal-profile cancellation fixture:
1. A repeat plays; accept a switch at sample 5000, boundary 6000.
2. Device loss is scheduled at sample 5500.
3. Step directly past sample 6000.

The initial `busy.advance(u)` settles the switch as `applied`. Replay then reaches the loss at 5500 and cancels its waveform before it starts. The final row nevertheless says `applied`. Stepping first to 5500 instead produces `cancelled`.

**Fix:** advance switch settlement in replay order, not ahead of earlier faults.

**Test gap:** `mock_radio.rs:1705–1714` steps exactly to the loss. Add the same sequence with one step spanning both loss and boundary, asserting identical cancellation and no new waveform.

### 4. A cursorless switch refusal leaves its row pending forever
**`crates/ezsdr-radio-uhd/src/provider/tx.rs:350–356,958–979,1203–1210`**

Underflow abandons the open repeat, but leaves its open booking and `playing` identity intact. A subsequent switch can therefore pass receipt validation.

Minimal sequence:
1. A repeat is playing.
2. An underflow report causes `abandon`; the device burst closes and `self.open` disappears.
3. Submit switch S.
4. Receipt accepts S and writes its pending row.
5. `install` finds neither an open cursor nor `chain.next()` and returns false before calling `Bookings::install`.

S is reported rejected, but its row stays `pending` indefinitely. Subsequent switches encounter that stale pending switch. The record and owner disagree about both the repeat and S.

**Fix:** reconcile the booking’s repeat lifecycle when abandonment ends it, and ensure every owner-side switch refusal settles the pending entry.

**Test gap:** underflow tests end the damaged burst, while switch-refusal tests supply a cursor. Add underflow → accepted/refused switch → settlement → next switch.

## P1 / P3

No additional findings reported.

## Verdict: **FAIL**

The shared rule still permits record/owner disagreements and an on-time accepted continuation to be lost.

Static review only: no files changed, builds, tests, mutation runs, or hardware runs.
