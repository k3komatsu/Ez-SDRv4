My independent failure scenarios are:
- Registration before Authority initialization, or a channel/join wait afterward, freezes the clock while waiting for work that needs it to advance.
- A driver wake tied with an owner wake leaves their execution order unspecified; queue pushes may straddle one poll even though device time stays fixed.
- Drop/unwind keeps the driver registered through Authority teardown, or releases the drop-test evidence too early.
- Two rigs confuse registrations or teardown state.
- A mutation appears killed only because a wall bound expires or one scheduling order wins.

I’ll now compare these with the plan, focusing on T4’s receipt race.
The retained failing binary’s `Direct::start` calls raw `TimeAuthority::wait_until`, bypassing the sketch’s trailing 1 µs wait. It matches the M12 probe: that was the last mutant built before the binary was copied, and the probe script restores source without rebuilding.

This gives T4 a concrete tie path without implicating stage 1a’s clock. It also makes the plan’s optional test of `start()`’s own wait important.
W13 has a second problem: after `HoldAfterApply` signals entry, its owner stays busy waiting for release. The proposed `until_received` may still be asleep on device time, so it cannot read the signal until the owner’s 10 s timeout releases the hold. I ran the sketch test: it passed in 13.11 s.

That can move `Provider::stop` after configuration, defeating the ordering the test is meant to pin. The plan needs a handshake that observes entry while the owner is still held, with a marker if that setup times out.
## Verdict: **FAIL**

`P` below means `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1c/`.

### Major 1 — W13’s rewrite can destroy the ordering being tested
**VERIFIED.** `P/test-plan.md:13,32,111`; `fake.rs:6104–6108,6172–6177`; `P/probe/sketch-1c.diff:134–163`.

Minimal sequence:
1. Driver enters `until_received` and sleeps on device time.
2. RX owner signals entry into `apply`, then blocks awaiting release.
3. That busy participant freezes device time; the driver cannot finish its sleep.
4. The owner’s **10 s timeout** releases configuration; only afterward can the driver observe entry and call `Provider::stop`.

**Wrong outcome:** the test passes without establishing Stop **during** configuration—the ordering required by P473 (`plan/maintenance/tools/mutations.json:5871–5878`). I ran the sketch test: **PASS in 13.11 s**, corroborating this path.

Require an entry/release handshake observable while the owner remains held, an explicit ordering assertion, and a marker on setup timeout. Merely rerunning P473 is insufficient.

### Major 2 — T4’s “unexplained race” binary contains M12’s bypass
**VERIFIED.** `P/test-plan.md:111,131,161`; `P/probe/mutprobe.py:23–41`; `P/plan.log:5617–5627`.

Disassembly of `/tmp/opencode/fake-sketch3-bin` shows `Direct::start` calling raw `TimeAuthority::wait_until`, **not** `Direct::wait_until`. Its trailing 1 µs is missing. The probe restored source without rebuilding, then copied the latest executable after M12.

Minimal sequence:
1. Raw start wait and control poll finish at T.
2. Driver pushes A; control receives A and sleeps until T+1 ms.
3. Driver, still registered, sleeps in real time and pushes B.
4. B is received at T+1 ms.

**Wrong outcome:** receipts differ by one poll. I reproduced **3 failures / 24**, exactly 200,000 ticks apart.

This is a sketch-artifact defect, **not evidence of a stage-1a clock defect**: M4 preserves time, not execution order among participants at one instant (`host.rs:233–254`; `control.rs:234–250`). Make the optional `start()` duration assertion and M12 coverage mandatory; rebuild a verified baseline before collecting evidence.

### Major 3 — Adding 1 µs does not exclude every final-deadline tie
**INFERRED counterexample; not executed.** `P/test-plan.md:11,100–102,165`.

From an armed rig, spawn control at S, then call `settle(999 µs)`. Its two sleeps finish at **S+1 ms**, exactly control’s next poll. Push A, pause in real time, push B: the same scheduling-dependent split remains possible.

T3 tests poll-aligned **input** ticks, not a trailing microsecond that itself lands on a poll. Add this boundary case; either constrain and document supported phases or establish ordering at the final wake. Another fixed epsilon alone is not a general solution.

No edits, builds, formatting, mutation runs, or remote execution performed. P473’s revised kill path remains unverified.
