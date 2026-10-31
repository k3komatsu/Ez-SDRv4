# Stage L1 — review record (2026-10-31)

Implementer GPT-6 Luna fast (#max): test plan, the code run with one free-DeepSeek self-review round, two fix rounds. Independent review Sol 6.1 (test plan, code, one re-review); orchestrator Claude Opus. Design: [note 44](../44-device-chain-state.md) §3 (owner decisions of 2026-10-30 in §9).

| Source | Stage | Severity | Finding | Mechanism | Disposition |
|---|---|---|---|---|---|
| Sol | test plan | P2 ×5 | begin/restart vs eligibility publication not gated; `left`/`latest` not followed through chain reuse; the mixed RX/RX/TX head-of-line case optional; delayed apply rejection unobserved; the "raw" P4 test still went through Control | test plan (not a design mechanism) | all adopted before code |
| orchestrator | test plan | note ×2 | one named test per case; how each RX 5 retarget is put between two release passes | test plan | adopted before code |
| DeepSeek (self-review) | code | P1, P2, P3 | added tests without mutation rows; a stale overview claim; UR-24's general sentence implied an unbegun segment receives the value | P1 test power; P2, P3 — | fixed before the report |
| orchestrator | code | major, note | `abandon_unowned` was test-only: an RX owner whose spawn failed left its chain `Armed` (same as Sol M2); transition preconditions are `assert!` in production builds too | M — (a note 44 transition not wired); note — | M with Sol M2; the note kept: a call site that meets a wrong state is the defect (Sol M1), not the assertion |
| Sol | code | major ×3, minor ×3 | M1 a cut-free LateCommand restart after a handoff published loss called `rx_start` and then panicked on `Lost`; M2 = the orchestrator's major; M3 a blocked head returned before the due-record fold, so an earlier handed command's `issued` row waited behind it; m4 the delayed-refusal test never delayed; m5 U40 retired for a rename, its path still live; m6 a `!planned.refused` predicate RM-36 makes unreachable, with four equivalent rows | M1 `mech:stream-lifecycle-scattered`; M2 —; M3 — (the new hold rule's record fold); m4–m6 test power / dead code | fix round 1 (P1290–P1294, U40 restored, the predicate and P1271/P1272/P1277/P1278 removed) |
| Sol | fix 1 | major, minor | M a receive turn could pass its early loss check, wait for the chain lock while Control's handoff published loss, then call `rx_recv`; m a corrected fixture built a second owner whose `begin` failed on an unrelated configure | M `mech:stream-lifecycle-scattered` (the orchestrator rated it minor: the window existed before L1 and closes fully only with a lock held across a blocking call; narrowed by reading loss in the lock that already decides `one_packet`); m test | fix round 2 (P1295); no Sol re-review (minor) |
| full gate | fix 2 | 2 compile errors, 5 survivors | existing rows on code whose meaning L1 changed (the step's runs covered only touched rows): J69, P970 no longer compiled; P965, P1035, P1230, P456, phase7 B01 survived | test power / dead code | fix round 3: P1230's `completed` check and P456's post-configure loss check deleted as decided by note 44's `eligible` and `begin`'s loss recheck under the chain lock (rows retired); J69, P970 retargeted; P965, P1035 pointed at the tests that observe them; B01's test asserts the receive timeout — 5/5 killed |
| full gate | fix 3 | 1 survivor | phase7 B02 (a stream silent past its cut keeps its stop record): fix round 3's rewrite of the shared test for B01 no longer reached it | test power | fix round 4: its own manual-clock test `ur_25_a_silent_stream_past_its_cut_records_its_stop`; B01, B02 2/2 killed |
| full gate | fix 4 | — | — | — | workspace 2155, uhd 580 + 5 (lib 567 after fix 4), clippy (host, `--features uhd`), hardware build, Python 3.14/3.9 57 each, rules 0; mutation 1986/1987 (G09 known; B02 rerun after fix 4) |

Both `stream-lifecycle-scattered` findings were a device loss Control publishes through a handoff and an owner path that called the device without reading it; both were fixed by reading the loss in the chain lock the path already takes, which is note 44's rule (the chain's `Lost` is a transition) rather than a new flag. Hardware is not yet rerun (one bench after L2, note 44 §9).

## Sol, test plan

VERDICT: REVISE

### Independent failure scenarios

- Control checks eligibility while the owner begins, restarts, stops, or finishes: a value reaches the wrong segment, twice, or after ownership ends.
- An unbegun segment blocks the held queue: unrelated RX/TX commands overtake it, disappear, or remain stuck after the head becomes eligible.
- A failed drain leaves device state across stop/restart, close/open, or a channel move; software forgets that state or blocks restoration forever.
- Consumption removes the only restoration history, or reopening resets `latest`, causing the wrong frequency to survive Provider stop.
- Loss or owner-spawn failure leaves an armed chain without a completing owner, or cleanup calls the device after loss.
- A delayed handoff fails: its provisional debt or logical record incorrectly survives rejection.

The plan covers most stable states and correctly removes the assumption that an unbegun successor can own handed debt. The remaining gaps concern the sequences and observations needed to prove that simplification safe.

### Findings

### 1. P2 — Concurrency coverage does not yet pin restart and eligibility publication

**Plan:** `l1-test-plan.md:28–36,48–56,124`.

RX 2 promises gated begin/release orders, and the retained tests cover handover versus stop/completion. RX 4, however, tests release **before or after** restart, not during it. The current restart calls `rx_start` outside the DSP lock (`crates/ezsdr-radio-uhd/src/provider/rx.rs:643–645`); accidentally retaining that ordering could pass those serial cases.

**Change:** Specify the gate and blocking assertion for:
- successful begin after device start but before eligibility/`Streams::started` publication;
- restart versus timed apply, in both lock orders;
- the first-sample transition, proving it preserves eligibility and segment identity.

Use chain-lock waiter handshakes, not merely threads launched together. State which lock each gate holds and assert the forbidden call cannot enter until release. Retain the existing stop/completion contention tests. This distinguishes actual serialization from tests of two stable snapshots.

### 2. P2 — `left` and `latest` are not tested through chain reuse

**Plan:** `l1-test-plan.md:78–106`.

RX 6 checks transfer to `left`; RX 8 checks that `left` suppresses restoration. Neither specifies the positive continuation: later samples consume the leftover and remove the blocker. Likewise, “latest remains after consumption” does not test retention through finish and close/open.

A refactor that clears both collections on reopen—or never removes `left`—can satisfy the listed isolated assertions while losing restoration history or suppressing restoration indefinitely.

**Change:** Add one production-path lifecycle:
1. Hand a value, force bounded recovery failure, and finish with one reported leftover.
2. Close/open and begin another segment.
3. Show the fake’s stale value survives, then is consumed by qualifying samples.
4. Assert software tracking reconciles that consumption without a second `rx_drain` report.
5. Stop and verify restoration uses the correct retained history.

Include the channel-move variant, where the leftover remains recorded on the old stream’s chain. Inspect the device before another configure masks the outcome. This follows note 44 §3.4/§3.7; it does not demand isolation beyond KL-9.

### 3. P2 — Cross-stream and TX head-of-line coverage is optional

**Plan:** `l1-test-plan.md:30,70–74`.

“Where the fixture supports both directions” and “where practical, TX” permit omitting an explicitly accepted observable change: one delayed RX owner also delays commands for an otherwise ready stream and TX.

**Change:** Make one mixed-queue case mandatory: blocked RX frequency, later frequency for a second already-running RX stream, then a TX update. Advance beyond their instants while holding the first owner before begin. Assert:
- no later device apply and no premature `issued` row;
- release resumes after begin, in `(e, sequence)` order;
- delayed rows retain their original `at`;
- the second stream’s physical update is late despite its owner starting on time.

Also exercise refusal/removal of the blocked head and show the following commands are reconsidered. Existing `release()` ordering (`provider/control.rs:684–721`) makes this a concrete integration boundary, not just a collection-order test.

### 4. P2 — Delayed apply rejection lacks its observable regression

**Plan:** `l1-test-plan.md:48–56`; note 44 §3.8, “apply rejection.”

RX 4 asserts successful delayed handoff. RX 4a covers loss. RX 1 promises transition-result coverage, but does not specify the ordinary handoff rejection’s Control-level consequences.

**Change:** Hold a frequency until after `e`, begin its segment, then refuse its timed apply without loss. Assert:
- provisional debt is removed and `latest` is unchanged;
- one `COMMAND_REJECTED`, with no frequency-update applied row;
- the refused value does not become configuration of record;
- existing projection behavior is preserved;
- a subsequent command can proceed on the next release pass.

A pure transition test cannot observe the record/configuration consequences implemented around `give()` (`provider/control.rs:868–891`).

### 5. P2 — The retained “raw” characterization still depends on production handoff eligibility

**Plan:** `l1-test-plan.md:142`.

`ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment` is not wholly raw: it obtains V1 through `timed_at`/Control before manually stopping/restarting the device (`provider/control.rs:7278–7296`). Its setup opens a streamer but does not adopt/start an eligible RX chain. Under L1, its first pending-DDC assertion no longer follows.

**Change:** Explicitly migrate this setup. For raw characterization, inject V1 through the raw timed device apply, preserving the stale-overwrite assertions and clearly separating it from production eligibility. Alternatively, establish a real eligible owner before handing V1 and explicitly bypass owner draining afterward. Do not weaken the pending/stale-value assertions merely to retain the test name.

### Evidence limits

UHD 4.10 `ddc.v:118–142` verifies transfer-dependent consumption; `radio_rx_core.v:171–183,524–527` verifies STOP bypasses timed queuing. Bench P4 A/C records persistence across stop/reopen. These do not establish late-tune physical phase or a universal drain-time guarantee.

Read-only review: no files changed, builds, tests, mutations, or hardware operations run. The findings concern test-plan completeness, not demonstrated defects in an L1 implementation.

## Sol, code

VERDICT: FAIL

### Independent scenarios

- A timed handoff reports loss while RX has a cut-free `LateCommand` result in flight.
- Initial `rx_start` succeeds, but spawning its owner fails.
- An earlier command is handed successfully; a later, unbegun RX segment blocks release while the earlier command becomes due.
- A frequency remains held past `e`, then its delayed apply is refused.
- Failed drain leaves `left` across reopen or a channel move; concurrent shutdown restores only safe channels.

### Major findings

### 1. Loss can be followed by a restart call and an owner panic

**Locations:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:633–637`; `crates/ezsdr-radio-uhd/src/provider/core.rs:181–182,225–228`.

A timed handoff returning loss sets the chain to `Lost`. Meanwhile, RX can already have a cut-free `LateCommand` result from its in-flight receive. Its restart acquires the chain lock but does not recheck loss.

Crucially, `self.core.device.rx_start(...)` is evaluated **before** `restart_result` checks its prior state. Consequently, this sequence invokes another device operation and then panics because `Lost` is not an allowed restart state—even if `rx_start` itself returns a loss error.

The new contention tests exercise successful apply/restart orders; the handoff-loss test calls `try_end` afterward, so neither covers this combination.

**Fix:** Recheck global loss and the chain’s terminal state after acquiring the restart lock, before invoking the device. Route loss through normal finalization. Add a gated handoff-loss versus LateCommand regression asserting no restart call, no panic, and one report per outstanding entry.

### 2. Owner-spawn failure never performs the planned abandonment transition

**Locations:** `crates/ezsdr-radio-uhd/src/provider/mod.rs:690–695,715–719,775–778`; `crates/ezsdr-radio-uhd/src/provider/core.rs:303–309`.

`Rx::new` adopts the successful initial start as `Armed`. If `spawn` fails, its closure—and the RX owner—is dropped. The error path calls Provider stop, but there is no owner thread to finish that chain.

`abandon_unowned` exists only under `#[cfg(test)]` and has no production caller. Cleanup therefore encounters an ownerless `Armed` chain: the new invariant panics in test builds, while production closes without recording the designed exception.

The test at `provider/rx.rs:1987–1999` manually abandons an owner constructed **without** an initial stream. It does not exercise this failure path.

**Fix:** Implement and invoke the abandonment transition on actual RX spawn failure, preserving terminal loss. Add a Provider-level injected spawn-failure test after successful initial start, through stop and cleanup.

### 3. A blocked command suppresses records for earlier successful handoffs

**Location:** `crates/ezsdr-radio-uhd/src/provider/control.rs:737–739,747–765`.

`Handed::Blocked` returns from `release`, skipping the fold that records already-released commands whose instants have arrived.

Example:

1. Command A is handed before its instant.
2. Later command B targets an unbegun covering RX segment.
3. Time passes A’s instant.
4. Every release pass encounters B and returns before recording A.

A’s `issued` row and configuration-of-record fold can remain postponed indefinitely, although A already reached the device. The accepted head-of-line limitation delays commands **behind** B, not records for earlier successful commands. The no-channel blocking branch correctly uses `break` instead.

**Fix:** Stop handing the suffix but continue the existing due-record fold, retaining its end/loss checks. Add a manual-clock test with a successfully handed prefix and a persistently blocked RX head.

### Minor findings

### 4. The “delayed refusal” test never exercises delayed delivery

**Location:** `crates/ezsdr-radio-uhd/src/provider/control.rs:8323–8363`.

The named delayed-refusal test starts an eligible chain immediately, then refuses apply at `refused_e − release_window`. It never holds that command past `e` or begins its covering segment afterward. Thus it tests ordinary early refusal, not the mandatory amended-plan sequence.

**Fix:** First observe the command blocked while unbegun, advance beyond `e`, begin its segment, then refuse apply. Preserve the rollback, event, record, projection, and next-pass assertions; verify the refusal call actually occurs after `e`.

### 5. U40 was retired for a textual rename, not an unreachable behavior

**Locations:** `plan/phase7/tools/mutations.json` (deleted U40); `crates/ezsdr-radio-uhd/src/provider/rx.rs:494–499`; `crates/ezsdr-radio-uhd/src/provider/core.rs:1728–1739`.

U40 checked acceptance of a switch-time rate mismatch. That behavior remains reachable through `owner_configure` and its configure-error return. Removing `owner_configure_with_abort` removed the callback, not rate-mismatch handling.

**Fix:** Retarget U40 to the current `owner_configure` call and retain its differential killing test. Correct the report’s retirement rationale.

### 6. Proven-unreachable RX predicates remain with four equivalent mutations

**Locations:** `crates/ezsdr-radio-uhd/src/provider/control.rs:863–866`; `plan/maintenance/tools/mutations.json:15862–15943`.

P1271/P1272/P1277/P1278 all remove the same `!planned.refused` predicate and are declared equivalent because refused RX segments never appear in this plan. That explanation agrees with `timeline.rs:1013–1015,1034–1040`.

Keeping the predicate and four equivalent rows preserves dead machinery rather than completing the requested simplification.

**Fix:** Remove this redundant RX-only predicate and retire those rows with the established invariant as evidence. Preserve refusal/retarget tests with meaningful mutations.

### Other conclusions

The obsolete settlement callback and per-exit successor recovery are removed. The inspected handoff path checks covering-segment eligibility under the chain lock, and successful begin/restart publication retains that lock. The spec amendments document late delivery, head-of-line delay, and phase uncertainty without promoting FakeDevice observations into hardware-phase guarantees.

Read-only review only: no edits, builds, tests, mutations, or hardware operations. Reported gate results were not independently rerun; the lifecycle and recording findings above remain blockers.

## Sol, fix round 1

The fixture does not panic: `begin()` fails configuration first because the initial segment has no saved configuration. The timed-refusal assertions still exercise the named behavior, but those added owner lines create an unrelated rejection.

A separate loss gap remains: the ordinary/drain receive path can wait for Control’s chain lock, acquire it after handoff loss, and still call `rx_recv` without rechecking loss.
VERDICT: FAIL

### Major 1 — Receive can still call the device after handoff loss

**Location:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:208–235`

**Problem:** The normal receive path—including drain reads and `one_packet` reads—checks global loss before its later chain-lock acquisitions, but does not recheck afterward.

**Evidence:** RX can pass line 208, then wait for a chain lock while Control’s timed apply fails. Control sets the chain to `Lost` (`core.rs:225–228`) and publishes global loss. RX then acquires the chain in `recv_len`, `recv_timeout`, or the `one_packet` check, ignores `Lost`, and invokes `rx_recv` at line 234. This is a new device call, not a receive already in flight. The native implementation reaches `uhd_rx_streamer_recv` without checking loss (`crates/ezsdr-radio-uhd/src/uhd.rs:1033–1045`).

**Fix:** Order receive dispatch against handoff loss, checking both global loss and terminal chain state after acquiring the relevant synchronization. Route loss through normal finalization. Add a gated regression that pauses RX after its early loss check, publishes handoff loss, then verifies no receive call follows, no panic occurs, and outstanding entries are reported once.

The new begin/restart checks themselves are correct. Their gated regressions exercise the targeted failures and would reject the old code. Stop and flush transitions retain the chain lock, preventing the same chain’s handoff from publishing `Lost` between those transitions.

### Minor 2 — Corrected refusal fixture creates an unrelated rejection

**Location:** `crates/ezsdr-radio-uhd/src/provider/control.rs:4463–4465`

**Problem:** `start_rx_chain` already starts and publishes stream 0. Constructing another owner without an adopted stream and calling `begin()` does not establish that owner successfully.

**Evidence:** The initial segment has no saved configuration. `owner_configure` therefore reaches `Planned::configure`’s “segment has no configuration” error (`control.rs:218–222`). `begin()` reports that failure and returns `true`; the assertion mistakes that return for successful startup. The subsequent timed-refusal assertions still test the named behavior, but the fixture adds an unrelated rejection.

**Fix:** Remove the extra owner construction and `begin()` assertion. Keep the correctly targeted `start_rx_chain` and existing refusal/debt assertions.

### Notes — Other adopted findings closed

- **Spawn failure:** Production abandonment runs after failed RX spawn, preserves terminal `Lost`, and has a Provider-level stop/cleanup regression (`provider/mod.rs:699–705,1078–1122`; `provider/core.rs:303–308`). No other production construction path drops an adopted RX owner unrun.
- **Blocked-head recording:** `Handed::Blocked => break` reaches the existing guarded due-record fold. The manual-clock regression checks both the earlier issued row and configuration-of-record update while the head remains held (`provider/control.rs:740–766,7954–7985`).
- **Delayed refusal:** The revised test holds the command, advances past `e`, begins the segment, and then refuses its timed apply (`provider/control.rs:8423–8449`).
- **U40 / dead predicate:** U40 targets the live configuration-error path. Removing the covering-plan refusal predicate agrees with RX plan exclusion in `crates/ezsdr-radio/src/timeline.rs:1013–1015,1034–1040`.

Read-only review; no edits, builds, tests, mutations, or hardware operations. Reported gate results were not independently rerun.

