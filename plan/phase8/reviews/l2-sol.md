# Stage L2 — review record (2026-10-31)

Implementer GPT-6 Luna fast (#max): test plan, the code run with one free-DeepSeek self-review round, one fix round. Independent review Sol 6.1 (test plan, code); orchestrator Claude Opus. Design: [note 44](../44-device-chain-state.md) §4 (owner decisions of 2026-10-30 in §9). L2 is behaviour-preserving: the transmit owner's `continues_at`, `tail`, `front` and `reopen_needed` became one private `TxChain { burst, front, recovery }` per streamer, changed only by named transitions; nothing moved into `Core`.

| Source | Stage | Severity | Finding | Mechanism | Disposition |
|---|---|---|---|---|---|
| Sol | test plan | P1 | the barrier row kept a device-burst cursor and tail under `ReopenNeeded`, which both failed-close routes clear today; the front-admission row started from an already-open cursor, which bypasses admission | test plan (would have pinned a behaviour change) | adopted before code |
| Sol | test plan | P2 ×3 | an unacknowledged or short T3 left out; `Provider::stop` covered only by a pure transition call; Stop/loss covered only before the owner claims the streamer | test plan | adopted before code |
| orchestrator | test plan | note | the `Halted → Clean` reset tied to successful segment adoption, with refused and interrupted switches; the cold change does not replace the stored front | test plan | adopted before code |
| DeepSeek (self-review) | code | P2, P3 ×several | production reopen failure not asserted to reach `Halted`; segment dispatch, cross-domain front, ACK-gated front advance and burst open/continue without rows | test power | fixed before the report (P1310–P1314) |
| Sol | code | minor | the payload-bearing EOB send's error branch chose `lose()`/`mark_reopen_needed()` by `error.lost`, where main used `!core.is_lost()`: a loss published while the send was outstanding was overwritten with `ReopenNeeded` | — (a behaviour-preservation slip) | fix round 1: main's condition restored, a gated test, P1315; the other four recovery decisions compared with main and unchanged; no Sol re-review (minor) |
| Sol | code | note | pre-existing: an unconfirmed T3 (no ACK, or a short or failed send without loss) still ends recovery as `Clean`, so payload follows a flush the DUC may not have taken (INFERRED from UHD 4.10 `axi_rate_change.v`, `radio_tx_core.v`) | `mech:failure-collapsed-into-value` (seventh bug: a design note before its fix) | kept and pinned in L2 (`ur_22_owner_keeps_clean_recovery_after_an_unacknowledged_t3`); issue #75, fixed in D9b-1b (owner, 2026-10-31) |
| full gate | fix 1 | — | — | — | workspace 2163, uhd 588 + 5, clippy (host, `--features uhd`), hardware build, Python 3.14/3.9 57 each, rules 0; mutation 2006/2007 (G09 known) |

## Sol, test plan

VERDICT: REVISE

## Independent scenarios

Written before reading the test plan:

- A continuation waits for buffer space, retaining its tail; later dispatch sends that tail exactly once without another device SOB.
- A short/failed payload or closing send abandons the right cursor, preserves the previous safe front when closure is uncertain, and blocks subsequent payload until recovery.
- Ordinary Stop preserves recovery; terminal shutdown ends it through the actual owner path.
- Failed reopen settles the current segment, while a valid later segment can start. Refused or interrupted switches must not accidentally enable payload.
- Stop/loss arrives during reopen or its acknowledgment read; recovery, reservations and report ownership remain consistent.
- A front persists independently of an open burst, is ignored on a different clock domain, and is replaced only when advancement records that domain.

Paths below abbreviate `crates/ezsdr-radio-uhd/` as `UHD/`. Plan citations refer to the supplied `l2-test-plan.md`.

## Findings

### 1. P1 — The recovery-barrier matrix row preserves state that production clears

**Where:** Plan line 20; `UHD/src/provider/tx.rs:577–600,669–683,805–826`.

The row calls `DeviceBurst::Open` with cursor/tail and `ReopenNeeded` reachable, then requires those device-burst facts to remain available. Today, both failed-close routes clear `continues_at` and `tail`: `end_open()` takes them before closing; `abandon()` clears them before closing.

The report-triggered failure case instead reaches **logically Closed + ReopenNeeded + a new Kernel `open { first: true }`**. The uncertain hardware state is represented by recovery—not by retaining the abandoned tail. Preserving or replaying it would change behavior.

**Change the plan:** Replace this row with that reachable combination. Assert the abandoned tail is absent, the new Kernel cursor is unchanged at the barrier, and no payload precedes replacement/T3. Describe `Closed` here as “no retained device-burst cursor,” not proof that hardware accepted EOB.

Also correct line 21’s test setup: front admission happens before creating `Open`, in `step()` → `dispatch_start()` (`tx.rs:437–466,852–863`). An already-created Kernel `open` goes directly to `send()`, bypassing that decision. Start this case with a held burst, not an already-open cursor.

### 2. P2 — Recovery outcomes omit unsuccessful, nonterminal T3 completion

**Where:** Plan lines 33–36; `UHD/src/provider/core.rs:1439–1509,1543–1552`; `UHD/src/provider/tx.rs:734–752`.

The proposed recovery table distinguishes successful reopen/T3 from open or rate-validation failure. There is another current outcome: T3 can have a short/failed non-lost send or missing ACK, record `tx_flush`, and still return a non-stopped result. `open_tx_device_locked()` does not reject merely because `acknowledged` is false; the owner subsequently clears `reopen_needed`. Only an acknowledged flush with a tick advances the front.

Thus “Clean” cannot silently acquire the stronger meaning “T3 was acknowledged.” That would be a behavior change.

**Change the plan:** Add owner-level recovery cases for missing ACK and non-lost T3 send failure/short send. Assert today’s resulting recovery state, unchanged front, rejection record, generation/correlation handling and subsequent dispatch behavior. Existing Core flush tests establish the flush result, but not its integration into the new owner transition. Record any concern about this existing behavior separately; do not fix it in L2.

### 3. P2 — A pure `provider_stop` transition test does not protect shutdown wiring

**Where:** Plan lines 38–41; `UHD/src/provider/tx.rs:1174–1191`.

The plan explicitly leaves terminal shutdown’s recovery clearing covered only by calling the transition directly. That test still passes if implementation forgets to call it from `shutdown()`, or places it before a failing shutdown EOB that sets recovery again.

Today shutdown first calls `stop_now()`, then clears recovery.

**Change the plan:** Add an integrated owner shutdown case with recovery already pending, plus a case where shutdown’s close fails. Drive the ACK window with the manual host clock. Assert recovery is no longer pending, no reopen occurs, reservations are settled, and the permitted shutdown report reads remain. The test must fail when the production clearing call is deleted or moved before `stop_now()`.

### 4. P2 — Stop/loss coverage tests entry conditions, not in-flight recovery

**Where:** Plan lines 39–41,49; `UHD/src/provider/tx.rs:725–750`; `UHD/src/provider/core.rs:1455–1499`.

`ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss` publishes Stop/loss **before** calling `pass()`. It does not cover publication after the owner claims the streamer or while T3’s report read is outstanding. Those paths suppress `halt_failed_reopen()` when shutdown/loss has already won.

There is no separate report thread writing these four fields today: `reports()` and `report()` execute on the TX owner. The real concurrency is external stop/loss publication and control-path ownership against that owner.

**Change the plan:** Add deterministic gated sequences for Stop and loss during owner recovery, including the final ACK return. Assert claim release, no erroneous `Halted`/refusal cancellation, no experiment payload, and no newly started device operation after observed loss. Reuse the existing gate pattern in `core.rs:2188–2212`, but exercise `Tx::pass()` so the new recovery transition is checked. Replace the blanket ban on `HostClock::spawn` with a ban on wall-clock verdicts; registered manual-clock participants are appropriate.

## Hardware evidence and preservation checks

UHD 4.10 primary sources support idle-only radio timestamp checking, DUC timeline persistence, empty-EOB padding, and partial-send behavior. P5–P7/D70 support the documented measured cases—not universal recovery safety or timed-command FIFO draining. The existing FakeDevice differential test checks model outcomes, not equivalent RF-tone verdicts.

The front-domain replacement rule matches `remember_front()` today. Cold switching itself does **not** replace the stored front; dispatch filters it by domain. The later-segment recovery test is useful, provided its reset is tied to successful segment adoption.

No files changed; no builds, tests, mutations or hardware operations ran. Production hardware equivalence remains unverified.

## Sol, code

VERDICT: FAIL

## Scenarios reviewed

- First sends, adjacent continuations, held-tail waits, preemption and successful EOB.
- Short/failed payload and EOB sends, including concurrent device loss.
- Report-triggered abandonment → recovery barrier → replacement/T3 → payload.
- Reopen refusal, `Halted`, and successful/refused/interrupted segment adoption.
- Ordinary Stop, shutdown, and Stop/loss during claimed recovery and final ACK return.
- Missing/partial T3 acknowledgments, failed flush sends, generation changes and report correlations.

## Minor finding

### 1. A non-lost EOB error can reintroduce recovery after device loss

**Where:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:655–662`.

**Problem:** This replacement does not preserve the former condition:

- HEAD, `tx.rs:599`: `reopen_needed = !self.core.is_lost()`.
- Current, `tx.rs:662`: chooses between `lose()` and `mark_reopen_needed()` using **`error.lost`**.

**Sequence:** A payload-bearing EOB send is outstanding; another thread publishes device loss; the send returns a non-lost error. `abandon()` observes the published loss and calls `chain.lose()` at `:894–895`. The subsequent branch then overwrites that result with `ReopenNeeded`.

HEAD leaves recovery not pending. The replacement leaves it pending despite having observed loss. In production, the lost-device branch of `run()` (`:259–261`) skips subsequent passes, so this need not immediately clear.

**Impact:** An internal lifecycle regression, not evidence of an additional device call or unsafe payload. Nevertheless, it contradicts L2’s behavior-preservation requirement and its loss transition.

**Fix:** Preserve HEAD’s `self.core.is_lost()` condition in this EOB-error branch. Add a deterministic gated send-return test publishing external loss before returning a non-lost error; assert closed burst, no pending recovery, unchanged front and no subsequent device calls. Mutate that condition to `error.lost` to pin the distinction.

## Notes

### 2. The remaining transition mapping looks consistent

**Where:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:95–177,693–710,734–756,793–903,1036–1038,1107–1112,1255–1271`.

- `DeviceBurst` now explicitly represents an active device burst as well as a continuation waiting between Kernel bursts. Its readers account for that representation.
- I found no reachable production path violating the new open/continue assertions.
- Production readers distinguish only `ReopenNeeded`; `Halted` otherwise behaves like HEAD’s “not pending.” Its reset occurs after successful segment adoption.
- Removing the saved/restored front around failed payload EOB is valid: `abandon(..., close_open_burst=false)` does not advance it.
- State changes remain behind named `TxChain` transitions. No receive-path, Core-state, schema, option or dependency change was found.

No newly false `design/` text was identified beyond finding 1’s loss-lifecycle mismatch with UR-22 (`design/18-uhd-radio.md:247`).

### 3. Planned tests and mutation coverage are substantially present

**Where:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:1329–1388,1698–1769,1773–2070`; `plan/maintenance/tools/mutations.json`, P1296–P1314.

The added tests cover transition state, both abandoned-close routes, incomplete T3 outcomes, segment adoption, integrated shutdown and four gated recovery interruptions. The matrix is correctly described as state-only; retained owner tests supply observable send behavior.

No existing test function or mutation row was removed. I checked all TX anchors in both modified mutation lists: each matches exactly once. Retargeted mutations retain meaningful failure modes; P1194 now directly tests recovery clearing rather than only the return value.

The new concurrency tests use gates and manual-clock participants. I found no new behavioral verdict based on elapsed wall time; timeout guards remain subject to severe-load false failures.

### 4. Unacknowledged T3 becoming `Clean` is real, pre-existing, and potentially serious

**Where:** `crates/ezsdr-radio-uhd/src/provider/core.rs:1439–1509,1543–1552`; HEAD `provider/tx.rs:734–752`; current `provider/tx.rs:806–829`.

This behavior exists in HEAD: unsuccessful nonterminal T3 can return `Ok(flush)`, and the owner clears recovery regardless of `flush.acknowledged`. L2 correctly preserves and tests it. **It is not counted against L2.**

Distinguish two cases:

- A missing ACK may mean successful hardware completion with a lost report.
- A short/failed send may mean the two-sample EOB never reached the DUC. Waiting 100 ms does not establish closure.

UHD 4.10 [`axi_rate_change.v:396–433`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L396-L433) retains its timeline until the actual burst boundary, and untimed input clears timestamp propagation. The [radio state machine](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L337-L426) checks timestamps from idle and acknowledges an emitted EOB. Consequently, an incomplete flush can leave subsequent payload incorrectly timed or DSP consumption disabled. That consequence is source-grounded inference, not a bench result.

A separate fix should make incomplete recovery explicitly unsuccessful—bounded retry or segment halt/refusal—rather than silently permitting payload. Missing-ACK policy needs an explicit decision. Successful T3 still does **not** drain pending timed-command FIFO entries, as note 39 §6.6 and the earlier review establish.

**Not checked:** No edits, builds, tests, mutations or hardware operations ran. Execution results in the implementation report were not independently reproduced.
