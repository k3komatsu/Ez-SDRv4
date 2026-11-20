# #126 — a `Block` link on a radio rx port: one meaning in both Providers — phase A test plan (v2)

Written by the implementer (Claude Code, Sonnet 5.5) on `work/n126` (from `main` 5bc555d3), **reading only**: no edit, no build, no run, no git write. v1 is kept as `test-plan-v1.md`. Inputs: the brief, issue #126 (the two owner decisions, 2026-11-20), `plan/phase8/54-mock-vs-x310.md` row RX4, AGENTS §2/§9, `sol.md` (Sol's review of v1: FAIL, 5 majors + minors, all adopted), `plan-decisions.md` (the orchestrator's binding answers Q1–Q11, 2026-11-20). A claim is VERIFIED only where a `path:line` is cited; otherwise INFERRED. Nothing here was run.

## Changes from v1 (finding / decision → where)

| Source | Change | Section |
|---|---|---|
| Sol M1 + decision Q1 | `RX_OVERFLOW` reports the **device loss only**, stamped at its own first lost sample (D3 reversed); the carrying block's `lost` is the whole jump; the stat is the discards. Mock keeps `PendingReport.first` and gains the device intervals; UHD keeps a carried loss report. Mixed-loss tests (U4, MK5a–e, P8-5) recomputed; MK5c pins a fault **after** discards; MK5d's tail is 20 000; the parity helper, the RM-17 draft, the stamp mutation (N126-M10) follow | D3, §2.2, §2.3, §4.6, §4.10, §5, §7 |
| Sol M2 | The `jumped \|\| own` shortcut is dropped (and with it decision Q9's `jumped`; Q9's `read` stays). Each report is qualified **against its own device read**; the flags and the device loss it yields are **carried across `Full`** (`Carry`). Sol's sequence (unannounced jump, Overrun report, contiguous accept) is U5b | D8, §2.3, §4.6 |
| Sol M3 | U3c drives the owner through `Shutdown(Abort)` and asserts plan cut, applied stop and clock end agree; the design fixes the rollback/`refresh_intent` path: the abort floor is read from `Streams.delivered` (one source), and a floor read inside the publish window pins the block (`floor_reads`) | §2.3 (window rule), §4.5 U3b/U3c, §7 SC-20a |
| Sol M4 | P8 rig booted **before** the action window (`Started::booted`), row-specific preconditions (s5 has no `[0,2000)` prefix), poll margin and observed cut stated; infra verdict only on independent drift evidence | §4.10 |
| Sol M5 | FM10 becomes a `LockSpy`, turn-driven receive test that observes returned sample spans and per-turn stats (not `FakeDevice::calls()`, which records no `rx_recv`); the stall mutation has a manual-clock observation point | §4.11, §5 |
| Sol minor: K | K tests: a second Provider without a `Block` output in the same Run; an artifact span covering the event for the default mark; K2 no longer claimed to prove registration (the fallback row keeps the kind) | §3 |
| Sol minor: lifecycle, mutation audit | Segment change on **one** `Rx` owner and a refusal on `rx1` (U2, U12, MK9); N108-98 killer → MK5a; N108-96's overflow-tail branch → MK6c; N126-M04 no longer spins | §4.4, §4.7, §4.12, §5 |
| Sol minor: spec | RM-38 in `ezsdr-radio` cited by both Providers' rules; MR-27's tail formula, MR-28 and UR-20's event lists, UR-19's overflow wording, SC-18's `GAP_BEFORE`-alone sentence; every amended rule has its indented `*Test:*` line | §7 |
| Q2 | payload `{}`, the stream node names the port | D4 |
| Q3 | accepted; the stat may exceed the cut under abort and is asserted per Provider; the rule text says so | D5, §4.5, §7 |
| Q4 | `Full` is not in `link_drops_seen`; UR-17 says so | D6, §7 |
| Q5 | both Providers already have all stats keys from construction (Mock `lib.rs:421`, UHD `Records::with_limit`), so the new key is added in both and the goldens updated | D7, §8 |
| Q6 | narrow registration + the negative test kept | §3 |
| Q7 | no Run-level acceptance test | §3 |
| Q8 | literal UR-17 second-stop rule kept (now over the carried loss report too) | §4.6 U7 |
| Q10 | no KNOWN_LIMITATIONS entry | §8 |
| Q11 | the abort window is accepted and written in SC-20a/UR-17, with a test that makes it agree | §4.5 U3c, §7 |

Conventions. `x3xx` at 1 MS/s, block length 2 000 (`profile.block_len()` and UHD's `radio.rx.block_len` default), `G` = `ezsdr_radio::x3xx::timing().overflow_restart_gap_ns` in samples (140 000 at 1 MS/s; no test states the number). `a` = first sample of a discarded stretch, `kf`/`kg` as MR-21. "Accepted block" = a block a link answered `Accepted` (or a drop outcome), i.e. not `Full`. Rigs: **MK** = `mock_radio.rs` `Harness` (ManualTimeAuthority, single thread, the test steps and drains); **MC** = `mock_channel` rig; **U** = unit test in `rx.rs` (`rig_with_links`, `Rx::receive` fed synthetic `RxRecv`, as `rx.rs:1555` does; **UL** = U with `LockSpy`, manual host and `Rx::turn()`, as `rx.rs:3390`); **P8** = the VH-8 parity pair (`crates/ezsdr-acceptance/tests/parity.rs`); **K** = Kernel coordinator test.

---

## 0. What the plan decides

Internal decisions (AGENTS §9 "What the owner decides"), each tied to the binding answer where there is one.

| # | Decision | Why |
|---|---|---|
| D1 | The rule is one new Radio Model rule, **RM-38** (design/07, after RM-37); MR-19, UR-12, UR-17, MR-7 step 7 **cite it and add their mechanism**. The shared mechanism is a new module `crates/ezsdr-radio/src/rx_link.rs` (§2.1) that both Providers call. | AGENTS §2 "a topic is written once"; the #125 precedent (`staging.rs:1716`). RM-17 cites MR-19 twice today: the rule written in a Provider spec. |
| D2 | A discard is **not a device report**: it adds `GAP_BEFORE` and its span to the next accepted block's jump and nothing else (no `RESTARTED`, `SEQ_DISCONTINUITY`, `ALIGNMENT`, restart gap, head, `RX_OVERFLOW`). | Owner decision 2026-11-20. |
| D3 | **Q1.** A device report's `RX_OVERFLOW` (and `ALIGNMENT_ERROR`) is the **device's loss only**: `lost` = the samples the device itself lost in the reports the block carries (a discard excluded), stamped at the **first sample of that device loss** (as today). The carrying block's header `lost` is the whole jump (device loss + discards). `rx_link_discarded_samples` is the discards. Law, for every accepted block: `first = prev_end + lost`; `Σ event lost over the jump + discarded in the jump = lost`. Needs the device's pending loss kept apart from the discard count in both Providers (§2.2, §2.3). | Binding answer Q1; Sol M1. |
| D4 | The per-segment event is `ezsdr.LINK_BACKPRESSURE` (Kernel kind, continue + mark artifact, warning), `emit_control`, source the stream node `<device>/rx/s<k>`, time = the first sample of the segment's **first discarded block** in the segment's SampleClock (registered at the first block, accepted or not, RM-25), payload `{}`. | Brief; Q2. |
| D5 | `rx_link_discarded_samples` is added **when a block is discarded** (its length after any clip at the cut), never retracted. An orderly `Stop` inside a stretch therefore counts exactly to its cut on both Providers; under an `abort` it may exceed the cut and depends on the abort's timing, so it is asserted **per Provider**, never equal across them. | Q3 (accepted deviation from "up to the cut"). |
| D6 | A `Full` is not in UHD's `link_drops_seen`; `rx_blocks`/`rx_samples` count accepted blocks only on both Providers. | Q4. |
| D7 | `rx_link_discarded_samples` is present from construction in both stats sections (both Providers already keep every key from construction). | Q5. |
| D8 | UHD gets the Mock's two cursors and its carry: `expected` = the first sample no accepted block carries (abort floor, tail, the accepted block's `lost`); `read` = the end of the last block the device returned, accepted or discarded (overlap trim, the device-jump test, `recv_len`); `Carry` = the flags, device loss report and alignment report that discarded blocks yielded and no accepted block has carried yet. **Every block is judged against `read` exactly as today and consumes the reports pending at it**, so a report is qualified by *its own* device read and never credited with an earlier gap. | Sol M2; Q9's `read` kept, its `jumped` replaced by `Carry`. |
| D9 | Kernel registration is **narrow**: `(node, LINK_BACKPRESSURE)` for every node of a Provider's tree **only if** a `Block` StreamOut is attached to that Provider (§3). | Q6. |

---

## 1. Facts the plan relies on

VERIFIED (code at 5bc555d3):
- Mock `Full` today: `ezsdr-mock-radio/src/lib.rs:600-615` (restart gap `k_g = max(b, a + gap)`, `GAP_BEFORE | RESTARTED`, `rx.note(next, Overrun)`, `rx.planned = None`, `head_until = next_sample + rx.head`, `undelivered` unmoved); `publish_rx` returns `full` after publishing to every link (`:703`) and emits the report only `if !full` (`:705-713`); the accepted branch sets `undelivered = stop` and counts `rx_blocks`/`rx_samples` (`:616-626`). The clock is registered inside `publish_rx` before any link answers (`rx_register`, `:632`; MR-25).
- Mock admission: `lib.rs:2223-2227` ("MR-7: multiple rx links cannot use Block back-pressure", `:2225`). Tail: `rx_tail` `lib.rs:1334-1343` (`lost = cut − undelivered`, stamp at `undelivered`, no event without a pending report). Abort floor: `lib.rs:1239` (`delivered = undelivered`). `PendingReport { first, cause }` `lib.rs:127-173`: `first` is read only at `lib.rs:711` (the block-carried event stamp).
- Mock: `rx.note(kf, cause)` sits **inside** `if lost > 0` in `apply_rx_fault` (`lib.rs:778-781`): a Mock report always has a device loss; a zero-loss overrun (the `ideal` profile's gap 0) makes no report, flag or event. The invariant `first = undelivered + lost` for an accepted block holds because a fault moves `next` to `kg` and adds `kg − kf` to `rx.lost`.
- UHD refusal: `provider/mod.rs:513`; publish `provider/rx.rs:726-745` (events emitted `:704-722` **before** the publish; `stream.pending` taken `:680`; `stream.expected = k + len` `:701`; `rx_blocks`/`rx_samples` for every outcome `:744-745`; non-`Accepted` counted in `link_drops_seen` `:741`); `streams.delivered(...)` (the RM-16 floor) is written **before** the publish (`:648-653`, "a Stop booked after them cuts after the block"). Overlap trim `:655-669` and `lost = k − expected` `:679` read `expected`; `recv_len` reads it (`:508`); `follow`'s `done` (`:194`), `command(Abort)` (`:271`), `refresh_intent` (`:769-772`: `request_cut(instant(cut.max(expected)))` and, under an abort, `request_cut(instant(expected))`), `emit_tail` (`:382`), `planned_end` (`:376`) read it as the first undelivered sample. The chain keeps the **smaller** requested cut (`core.rs:199-201`).
- A UHD jump with no report is `SEQ_DISCONTINUITY` (`rx.rs:686-689`); a missed start adds `GAP_BEFORE` only; `loss_causes` excludes Alignment and MissedStart (`:783-794`); a gapless block with an `Alignment` report still emits `ALIGNMENT_ERROR { lost: 0 }` (`:709-717`, `lost_n = lost.unwrap_or(0)`).
- `FakeDevice::calls()` records **no** `rx_recv` entries; `LockSpy` records each receive request, mode and timeout (`test_support.rs:605-640`, `rx_requests()`, `rx_packet_modes()`, `rx_timeouts()`) and scripts answers (`script_rx_recv`). The fake reports a "behind" overflow when the reader is more than `OVERRUN_BEHIND` (100 ms) behind (`device.rs:~1817`, `now − end > behind`). Turn-driven manual-clock receive tests exist: `rx.rs:3390` (`rx.turn()`, `rx.host.advance_for_test`, `time.advance_to`).
- `Started` (P8) advances the UHD to T0 + 5 ms and settles before the test body runs (`parity.rs:811-835`): an action submitted afterwards cannot cut before the sample the UHD has already read.
- Kernel: the collector's pair table is built in `create_collector` (`pipeline.rs:463-540`): Provider nodes × the Module's vocabulary kinds + `DEVICE_LOST` at the root; the Kernel's five kinds only under `EventSource::Kernel`. An unplanned `(source, kind)` merges into the fallback row **but keeps its kind** (`event.rs:551-569`): the event and its policy reaction work without the pair; what is wrong without it is RS-33's accounting. `create_links` runs before `create_collector` (`pipeline.rs:314`, `:464`), so the attached links are known; `attached_links` is consumed at `prepare` (`:592`).
- `rs_28_policy_defaults_table` pins `LINK_BACKPRESSURE` = continue + mark artifact (`run_session.rs:441`). No Provider emits it from a Provider node today.
- Both parity rigs plan `(every stream node) × kinds.kinds()` (`parity.rs:247-249`, `:386-390`): **the Provider-level tests cannot see the Kernel registration**; the K tests are its only check.
- Every rig's `MemLink` is a private copy (`mock_radio.rs:164`, `mock_layout.rs:~140`, `mock_channel.rs:~95` with a `limit` knob, `parity.rs:168`, `rx.rs:825` `link()`, `rx/differential.rs:~630`). The plan adds a scripted outcome only where needed (§4.1).

INFERRED: that a consumer of a `Block` link on a radio rx port is an Executor/Processor component (SC-21 forbids `Block` into a Sink, `stream_contract.rs:429`); that `Streams::first_undelivered` is read only by bookings (`Provider::stop`, a loss) — the implementer greps the callers before adding the `floor_reads` counter (§2.3); that `HostClock::manual().sleep` moves the manual clock (needed by the stall mutation, §5).

---

## 2. The shared rule and what each Provider keeps

### 2.1 `crates/ezsdr-radio/src/rx_link.rs` (new; `pub mod rx_link;` in `lib.rs`)

```
pub const REASON: &str = "RM-38: a Block link must be its receive port's only link";
pub const DISCARDED_STAT: &str = "rx_link_discarded_samples";
pub fn refusal(policies: &[BackPressure]) -> Option<&'static str>   // Some(REASON) iff len > 1 && any Block
#[derive(Default)] pub struct Discards { raised: bool }               // one per receive segment
impl Discards { pub fn refuse(&mut self, len: u64) -> Refused }       // Refused { samples, raise_event }
pub fn event_payload() -> serde_json::Value                            // {}
pub const GAP_FLAGS: BlockFlags = BlockFlags::GAP_BEFORE;              // what a discard adds
/// A device loss report (RM-17): merge keeps the smallest first sample, sums the loss, overrun wins over sequence.
pub struct LossReport { pub first: i64, pub lost: u64, pub cause: RxOverflowCause }
impl LossReport { pub fn merge(&mut self, other: &LossReport); pub fn payload(&self) -> Option<RxOverflowPayload> }
```
`Discards` is the whole per-segment memory; the count and the flags are stateless; `LossReport` is the one merge rule both Providers use for the **block-carried** event (the tail's lost is each Provider's, §2.2/§2.3). Both Providers hold one `Discards` per receive segment (`Rx` for the Mock, `Stream` for UHD, both rebuilt per segment: `lib.rs:1396`, `rx.rs:70`).

### 2.2 Mock keeps (all in `lib.rs`)
`next`/`undelivered` (already two cursors) and `rx.flags`/`rx.lost` (the carry). A `Full` does `rx.next = stop; rx.planned = None; rx.flags |= GAP_FLAGS; rx.lost = Some(lost.unwrap_or(0) + len)`; `undelivered`, `head_until` and `rx.report` untouched; the event call (`emit_event("rx/s<k>", EventKind::LINK_BACKPRESSURE, Warning, event_payload(), TimePoint::new(domain, first))` when `raise_event`); `add_stat(DISCARDED_STAT, samples)`. Removed: the restart-gap arithmetic, the `rx.note` at a discard, the `head_until` restart. **D3**: `PendingReport` keeps `first` (the stamp) and gains the device intervals `(kf, kg)` of its faults (a `Vec` bounded by the Run's fault list, which is finite; the old "nothing grows" comment was about refusals): the block-carried event is `payload(cause, Σ(kg − kf))` at `first`; the **tail** (`rx_tail`) becomes `Σ max(0, min(kg, cut) − kf)` stamped at `first` (it was `cut − undelivered` stamped at `undelivered`; they agree whenever no discard precedes the first fault). Admission: `rx_link::refusal` replaces `lib.rs:2223-2227`.

### 2.3 UHD keeps (all in `provider/rx.rs`, `mod.rs`)
`Stream` gains `read: i64`, `carry: Carry { flags: BlockFlags, overflow: Option<LossReport>, alignment: Option<(i64, u64)> }` and `discards: Discards`. `samples()` per block with `len > 0` after the trim:
1. Trim against `read` (not `expected`); `own = k − read`; `stale_overrun = false` (the device returned samples, as today); the clock is registered (RM-25) as today.
2. **Judge the block against `read`, as today, and consume `pending`**: `own_flags = own > 0 ? GAP_BEFORE | (SEQ_DISCONTINUITY if pending is empty) | cause flags of pending : NONE`; `own_loss = own > 0 && loss_causes(pending) ≠ ∅ ? LossReport { first: read, lost: own, cause } : None`; `own_align = Alignment ∈ pending ? (read, own) : None` (a gapless block keeps today's `{lost 0}`). With `read == expected` and an empty carry this is today's rule exactly.
3. Header: `lost = k − expected` (whole jump), `flags = (GAP_BEFORE if lost > 0) | carry.flags | own_flags`; device loss = `carry.overflow` merged with `own_loss`; alignment merged likewise. Publish. Then:
   - **`Full`** (a discard): `stat(DISCARDED_STAT, len)`; `if discards.refuse(len).raise_event { emit LINK_BACKPRESSURE at TimePoint(clock.domain, k) }`; `carry.flags |= own_flags`, `carry.overflow`/`carry.alignment` merged with the block's; `read = k + len`; `expected` unmoved; no `rx_blocks`/`rx_samples`, no `link_drops_seen`; the floor rollback (below).
   - **any other outcome**: emit `RX_OVERFLOW { cause, lost = device loss }` at `(domain, device loss first)` and `ALIGNMENT_ERROR { lost }` at its first (now **after** the publish; they were before), clear `carry`, `read = expected = k + len`, count as today (`link_drops_seen` only for `DroppedOldest`/`DroppedNewest`).
4. `ended` handling as today.
5. **Tail and second stop** read `carry` too: the causes are `carry.overflow.cause` ∪ `loss_causes(pending)`; the tail's device loss is `carry.overflow.lost + (pending has a loss cause ? max(0, cut − read) : 0)` stamped at `carry.overflow.first` or, with no carry, at `read`; `end_cut` and the second-stop condition (Q8, the literal rule) treat a carried Overrun as "a loss report pending". With an empty carry and `read == expected` this is today's `cut − expected` at `expected`.

**Floor rollback and the publish window (Sol M3).** The booking floor (`Item.delivered`) is `Streams.delivered`, written before the publish (`:648-653`). Plan: `Streams` keeps a `floor_reads` counter bumped by `first_undelivered` (the bookings' read; uhd-rx's own reads use an uncounted accessor). After a `Full`, under the streams lock: if `floor_reads` is unchanged since the optimistic write, write `delivered` back to `expected` (no booking saw it); if it changed, **a booking read the block's end as the floor** (Q11): keep `delivered = k + len`. `refresh_intent` and `command(Abort)` take the abort floor from `Streams.delivered` (one source), not from `stream.expected`, so the abort request, the plan's booked cut and the clock end are the same value in both cases (the chain keeps the smaller request, so a request below the booked cut would otherwise win). `expected`, `read` and `carry` stay as the discard left them; the stream then ends on the next block (`len = 0`, `ended`) or at once (`done`), and the tail rule of step 5 covers a carried loss.

`mod.rs:513` loses the `Block` refusal; the per-port policy list goes through `rx_link::refusal`.

State argument (AGENTS §9 maintainability 2): `read`, `expected` and `Carry` can disagree **only while a stretch is pending**, by design. Invariants asserted in U1/U5 on the `Stream` fields: `read ≥ expected`; after an accepted block `read == expected` and `carry` is empty; a stream with `read > expected` has either a carry or no device jump yet; each broken by a mutation row.

The Mock twin needs no per-report qualification: its faults put flags, `lost` and the report into the carry at once and always have a loss (§1). The U4s/U5b counterexamples are UHD-only (a device `Overflow` with a contiguous next packet; an unannounced jump).

---

## 3. Kernel: how `LINK_BACKPRESSURE` from a Provider is registered (INFERRED design, verified against code)

Change (`coordinator/pipeline.rs` `create_collector`, beside `device_lost`): for each Provider instance in `self.shared.providers`, if `self.attached_links` holds, for that instance's fragment, a `StreamOut` link with policy `Block`, insert `(Node { node }, ezsdr.LINK_BACKPRESSURE)` for every `node` of `instance.tree.walk()`. The implementer resolves the key (the fragment id `prepare` uses at `:592`) and the Provider↔fragment map in the loop's `slot`. A Provider emits through its root-bound view (RS-32 admits any node below the root): no view change.

Spec: **KC-8** pair list gains "…; for each Provider instance one of whose attached `StreamOut` links is `Block`, every node of its tree × `LINK_BACKPRESSURE` (SC-20a, RM-38)"; **RS-27**'s "`LINK_BACKPRESSURE` from a DataLink policy (SC-20a)" becomes "`LINK_BACKPRESSURE`, which a producer emits where a `Block` link is full and its device cannot wait (SC-20a, RM-38)". RS-33 unchanged. Drafts in §7.

Rig: `two_link_docs()` (`coordinator.rs:2258`: graph link `radio.rx → c1.in`, `from` a resource port, the consumer a component on the Executor `exec`, not a Sink; the same port also carries the Sink feed, which the double Provider does not mind: RM-38's one-link rule is the Mock's and UHD's, not the Kernel's) with `"policy": "block"`, extended with a **second Provider** resource (`distinct_resource_docs(&["left","right"])` pattern, `coordinator.rs:3615`) whose output has no `Block` link. `SteppedProvider` gains `emitting_at(node_suffix, kind, severity, at)` (a small double change) to emit from a child node.

Tests (K, beside `kc_08_*`):
- **K1** `kc_08_a_provider_with_a_block_link_has_a_link_backpressure_row_at_every_node`: the left Provider emits `ezsdr.LINK_BACKPRESSURE` from its child node. Asserts on the Manifest counter rows: `(<left>/<child>, LINK_BACKPRESSURE)` exists with count 1 and the `unforeseen` row is 0. Fails today (no such row; the count is in `unforeseen`). Row N126-K01.
- **K1n** (same Run) the right Provider, which has no `Block` link, has **no** `LINK_BACKPRESSURE` row under any of its nodes; with only drop-class links no `Node`-source `LINK_BACKPRESSURE` row exists at all (the `Kernel` source's planned row aside). Fails a registration over every Provider (D9's wide form). Row N126-K02 (drop the `Block` condition).
- **K2** `spec["policies"]["failure"]["ezsdr.LINK_BACKPRESSURE"] = "stop"`: a control-path event (not dropped, unlike `rs_36_a_dropped_stop_body_stops_the_run_orderly`) ends the Run `Stopped { Policy { … } }` orderly. This pins the **reaction by kind**; it is **not** a registration test (the fallback row preserves the kind, `event.rs:551-569`).
- **K3** the default reaction marks the artifact: an artifact span (the rig's `rec` Sink output, opened before the event's time and closed after) records a mark of kind `ezsdr.LINK_BACKPRESSURE` at the event's time in the Manifest (`rs_30_*` shape); no `stop`. Fails if the default reaction or the artifact marking is not applied to a Provider-emitted event.
Timing: none (ManualTimeAuthority). No Run-level acceptance test (Q7).

---

## 4. Tests by sequence

### 4.1 Rigs the plan adds

- **MK**: no new double. `Harness`'s capacity-1 `Block` `MemLink` models a consumer that has stopped taking blocks (the test drains by hand; no wall clock). The first-block-refused case pre-fills the link with a dummy block before `arm_start` instead of capacity 0, so the link can open; capacity 0 stays for the "never accepts" cases.
- **U / UL / P8**: a `ScriptedLink` (policy `Block`): `publish` answers `Full` iff the block's `first_sample_time` (sample index in its segment clock) lies in configured `[from, to)` ranges, else `Accepted`, and records every refused block's `(first, len)` for the test's own invariant; an optional hook runs before it answers (U3c). By sample index, not call count or time, so both Providers see the same script whatever their block boundaries. In `provider/test_support.rs` (U/UL) and as a `MemLink` variant in `parity.rs` (`MockRig`/`UhdRig` take the link through a new constructor beside `with_ports`; the existing ones stay).
- **P8 `Started::booted`** (new): `arm` and `start` but **no** advance to T0 + 5 ms, so a Stop or a script installed afterwards lands before the UHD has read the samples it concerns (Sol M4). The existing `Started` is unchanged.
- **Helper** `assert_gap_law(blocks)` (one per rig file): for consecutive accepted blocks of a segment `n.first == p.first + p.len + n.lost.unwrap_or(0)` and `GAP_BEFORE` iff `lost > 0` (SC-13); and `assert_loss_law(blocks, events, discarded)`: per jump, `Σ RX_OVERFLOW lost + discarded == header lost` and each event's stamp is its own first lost sample (≥ the jump's start). The parity helper `assert_rx_overflow_output` (`parity.rs:881-900`: `payload.lost == header.lost`, `event_at == first − lost`) keeps those equalities **only when `discarded == 0`** and gains a `discarded` argument that applies the loss law instead.

### 4.2 Sequence 1 — a stretch → one gap block, no `RESTARTED`, no `RX_OVERFLOW`, one event, stats

| Test | Rig | Set-up | Asserts (observed by) | Wrong behaviour that fails it |
|---|---|---|---|---|
| **MK1** `mr_19_a_full_link_discards_and_the_next_block_carries_the_gap` (replaces `mr_19_backpressure_is_an_overrun`) | MK, `x3xx`, capacity-1 `Block` | step to block 0 `[0,2000)` (accepted, link full); step over `[2000,4000)`, `[4000,6000)` (discarded); drain; step to `[6000,8000)` | `link.receive()`: block 0; then `(first 6000, len 2000, flags == GAP_BEFORE exactly, lost Some(4000))`; `rx_overflows(&harness)` empty; exactly one `ezsdr.LINK_BACKPRESSURE`, source `mock/rx/s0`, time `(segment domain, 2000)`, warning; stats: `rx_blocks 2`, `rx_samples 4000`, `rx_link_discarded_samples 4000`; `assert_gap_law` | **Old MR-19 fails** on flags (`GAP_BEFORE|RESTARTED`), `lost` (`G`), the next block's position (`2000+G`), the `RX_OVERFLOW`, the stat |
| **MK1b** `mr_19_a_discard_ignores_the_restart_gap_and_the_head` (replaces the zero-gap test) | MK, `ideal` and `x3xx` | one discarded block, then accepted | `lost == Some(2000)` on **both** profiles; `first == 4000` | `k_g = max(b, a+gap)` (T37); a gap added on `x3xx` only |
| **MK1c** fractional root (`Rational::new(1_000_000_001, 1)`, the old test's second half) | MK | as MK1 | `lost == Some(block_len)`, `first == 2·block_len`, no `+1` fractional-gap sample | the restart arithmetic surviving on a fractional root |
| **MC1** `s4a_a_discard_does_not_restart_the_head` (replaces `s4a_a_back_pressure_restart_begins_with_the_head`, `s4a.rs:327`) | MC (`radio.link.limit`, whose double must report `policy() == Block`, §8) | loopback burst, link limited to 1, discard one block | the block after the discard: `flags == GAP_BEFORE`; its samples **from its first sample** are on-time signal (`assert_on_time`, no zero head) | S4A-41's old behaviour (`head_until = next_sample + head`) |
| **U1** `ur_12_a_full_block_link_discards_and_the_next_block_carries_the_gap` | U, `ScriptedLink` Full on `[2000,6000)` then `[8000,10000)`; blocks at 0…10000 via `rx.receive(samples(clock.instant(k), 2000))` | accepted `(0,2000,NONE,None)`, `(6000,2000,GAP_BEFORE,Some(4000))`, `(10000,2000,GAP_BEFORE,Some(2000))`; no `RX_OVERFLOW`/`ALIGNMENT_ERROR`; one `LINK_BACKPRESSURE` (source `usrp/rx/s0`, time `(domain,2000)`); stats `rx_blocks 3`, `rx_samples 6000`, `rx_link_discarded_samples 6000`, `link_drops_seen 0`, `rx_overflows 0`; after each discard `read == k+len`, `expected` unmoved, `carry` empty (no report); after an accepted block `read == expected` | `expected` advanced at a discard; flags/`pending` consumed wrongly; per-block event; `RESTARTED`/`SEQ_DISCONTINUITY` on a pure discard; `rx_blocks` counted for `Full` |

### 4.3 Sequence 2 — a second stretch in one segment
Inside MK1 (leave the accepted block in the link, discard `[8000,12000)`, drain, accept `[12000,14000)`: `lost 4000`; `LINK_BACKPRESSURE` count stays 1; stat 8000) and U1 (one event, stat 6000). Fails: an event per stretch, block or call (N126-M06, N126-U05).

### 4.4 Sequence 3 — a new segment (Stop, `start_rx`) with a stretch
- **MK3** `mr_19_each_receive_segment_raises_its_own_event`: first segment as MK1; `Stop` + `start_rx("mock/rx/s0")` (as `mock_radio.rs:2581`), link left full so the successor's first block is discarded. `LINK_BACKPRESSURE` count 2; the second's time is `(second domain, 0)` (registered at its refused first block) and its domain differs from the first's; stat sums both. Fails a once-per-Run flag (structural: `Rx` is rebuilt per segment; no row).
- **U2** `ur_17_a_segment_change_on_one_receive_owner_raises_a_second_event` (Sol minor): **one `Rx` owner**, two planned segments (`plan(&core, …)` as `ur_26_*`): the first ends by a cut (`turn()` finishes it), the same owner's `begin()` starts the second; both stretches discard. Asserts: two events, two domains, the second at `(domain2, 0)`; `stream.discards`/`read`/`carry` are fresh in the second (`read == 0`, empty carry before its first block); the stats add. Fails state kept on the owner instead of the `Stream` (a once-per-owner flag, a carry that survives the segment: e.g. a carried Overrun reaching the successor's first block).
- **MK9 / U12** a refusal on **rx1** (Sol minor): MK9 (`mock_layout.rs` two-port rig, `Block` on `rx1` only, link full) and U12 (`rig_with_links_on(1, …)`, `test_support.rs:125`): the event's source is `mock/rx/s1` / `usrp/rx/s1` and no `rx/s0` event exists; the other stream's blocks stay gapless and eventless. Fails a hard-coded `s0` source or `k`.

### 4.5 Sequence 4 — a stretch pending when the segment ends

| Test | Rig | Set-up | Asserts | Wrong behaviour that fails it |
|---|---|---|---|---|
| **MK4a** `mr_19_a_stop_inside_a_stretch_makes_no_event_and_counts_to_the_cut` (replaces `…emits_the_tail_from_the_refused_block`) | MK, capacity-1 | block 0 accepted, link full, `Stop` for `mock/rx/s0` at sample 30 000; a second run with the Stop at 29 500 | no `RX_OVERFLOW` (before and after the cut); one `LINK_BACKPRESSURE` at `(domain,2000)`; clock `ended_at == T0 + 30 ms` (29 500 in the second); stat `28000` (`27500`: the last block clipped at the cut); only block 0 delivered; `rx_blocks 1` | **Old MR-19 fails**: the tail `{overrun, 28000}`. A stat counted before the clip fails the 29 500 run |
| **MK4b** `mr_19_an_abort_in_a_stretch_cuts_at_its_first_sample` (the old `…an_abort_after_a_refusal_loses_nothing_before_its_cut`, extended) | MK | block 0 accepted, 2 blocks discarded, `stop(Abort)` | clock ends at `a = 2000` (`2_002_000_000`); no tail; `rx_blocks 1`; new: one `LINK_BACKPRESSURE`, stat `4000` — **a deviation from the brief's "up to the cut" accepted as Q3** (cut 2 000; the discards before the abort are not retracted) | `undelivered = stop` in the discard branch (N108-52 retargeted); an abort cut at `next` |
| **U3a** `ur_18_a_stop_inside_a_stretch_makes_no_event_and_counts_to_the_cut` | U | plan cut at sample 9 000; A at 0, F at 2000, 4000, 6000, 8000 (the last clipped to `[8000,9000)`) | no `RX_OVERFLOW`; stat `7000`; the stream ends (`try_end`) with the clock ended at the cut; no tail; with a cut at 5 000 and `read == 4 000` after the discards, `rx.recv_len() == 1000` | a tail from the discard; the clip applied after counting; the stream never ending because `expected` never reaches the cut (`ended` must come from the block reaching the cut); `recv_len` from `expected` (N126-U10) |
| **U3b** `ur_24_an_abort_in_a_stretch_cuts_at_its_first_undelivered_sample` | U (the owner driven through `RxCmd::Shutdown(Abort)`) | A at 0, F at 2000, 4000; then the shutdown | `Streams.delivered == 2000` **after** the discards (the optimistic write rolled back); the plan's cut, the applied `rx_stop` row and the clock end are all `clock.instant(2000)`; no event | the rollback missing (the abort would cut at 6000, after the stretch) (N126-U03) |
| **U3c** `ur_24_an_abort_booked_while_a_block_is_discarded_ends_the_stream_at_that_block_s_end` (Q11's window) | U, `ScriptedLink` whose `publish` runs a hook that books `Provider::stop(Abort)` into the shared plan (the booking `Provider::stop` makes: it reads `first_undelivered`, bumping `floor_reads`) **before** answering `Full`; then the owner is driven through `RxCmd::Shutdown(Abort)` and its loop to the end | A at 0 (`[0,2000)`); F `[2000,4000)` with the abort booked inside its publish | **all three agree at 4 000**: the plan's booked cut (`planned(key)[0].segment.cut == Some(4000)`), the applied `rx_stop` row (`at == clock.instant(4000)`), and the clock's end (`clock.instant(4000)`); no `RX_OVERFLOW`; stat `2000`; the stream ends once, no second end. Contrast U3b (abort booked after the `Full` returned): all three at 2 000 | the rollback re-writing `delivered = expected` regardless of `floor_reads` (the booked cut 4000, the abort request 2000: the chain takes the smaller, the clock ends at 2000 while the plan says 4000); `refresh_intent`/`command` reading `stream.expected` (N126-U03b) |
| **P8-4** `vh_8_a_stop_or_abort_in_a_stretch_cuts_both_providers_alike` | P8 with `Started::booted`, `ScriptedLink` Full on `[4000,∞)` | orderly: step both to relative 12 ms, submit the Stop, run to 40 ms; abort: abort at 20 ms | orderly: each Provider's clock ends at **its own** cut row (as `vh_8_an_overrun_cut_before_its_restart…`, `parity.rs:1010-1018`: the cut is per-Provider, the UHD's is the applied Stop); that cut ≥ 12 000 + the poll margin (one 2 000-sample call + 3 ms allowance + 1 ms poll ≈ 6 ms at 1 MS/s: assert ≥ 12 000 + 6 000); no `RX_OVERFLOW`; one `LINK_BACKPRESSURE` each; per Provider `rx_link_discarded_samples == own cut − 4000` (== Σ the link's refused lengths); abort: both cut at 4 000, no event, stat == Σ refused per Provider (**not** equal across Providers) | the discriminators above on both Providers |

### 4.6 Sequence 5 — flags, reports and device loss across a discarded block (Q1: header = whole jump, event = device loss)

Mock (MK5, rewriting `mr_19_a_fault_report_survives_a_refused_publish` and its neighbours; `x3xx`, capacity-1):
- **MK5a** overrun at 1 ms **before** the discards: block 0 `[0,1000)` accepted; restart block `[1000+G, 3000+G)` discarded; drain; accepted `[3000+G,5000+G)`: `flags == GAP_BEFORE|RESTARTED`, header `lost == Some(G + 2000)`; **one `RX_OVERFLOW {overrun, G}` stamped `(domain, 1000)`** (device loss only, own first sample); one `LINK_BACKPRESSURE` at `(domain, 1000+G)`; stat `2000`; fault row `G`. The report waits for the accepted block (the refused publish consumes none: N108-50/98 killer). `assert_loss_law`.
- **MK5b** sequence error at 1 ms: block `[3000,5000)` discarded; next `[5000,7000)`: `GAP_BEFORE|SEQ_DISCONTINUITY`, header `lost 4000`, one `RX_OVERFLOW {sequence, 2000}` at 1000; stat 2000.
- **MK5c** `mr_19_a_fault_after_discards_reports_the_device_loss_only`: block 0 `[0,2000)` accepted (link full); discards `[2000,10000)` (8 000); `rx_overflow` at 10 ms (`kf = 10000`, `kg = 10000+G`); drain; accepted `[10000+G, …)`: header `lost == Some(8000 + G)`, `GAP_BEFORE|RESTARTED`; **`RX_OVERFLOW {overrun, G}` stamped `(domain, 10000)`** (not 2 000, not `8000+G`); `LINK_BACKPRESSURE` at 2000; stat `8000`; fault row `G`. A variant leaves the link full across the restart block too (discard `[10000+G, 12000+G)`, accepted at `12000+G`): header `lost == 8000 + G + 2000`, event `{overrun, G}` at 10000, stat `10000`. Fails: the event's `lost` equal to the header's (N126-M09/M13), the stamp at the jump start, the stat counting `lost`.
- **MK5d** `mr_19_a_fault_inside_a_stretch_then_a_stop_reports_the_device_loss_to_the_cut` (the old "…then a stop counts the stretch once"): the MK5c set-up, then a `Stop` at 30 ms inside the fault's gap: **one tail `{overrun, 20000}`** (= `30000 − kf`: the device loss to the cut, discards excluded; was 28 000) stamped `(domain, 10000)`; one `LINK_BACKPRESSURE`; stat 8000. Fails: the tail measured from `undelivered` (28 000 at 2 000) (N108-31's family, retargeted).
- **MK5e** `mr_37_pending_reports_keep_the_first_sample_and_the_causes_not_a_list`: (1) capacity-0 link, three stretches, `Stop` at 300 ms: no `RX_OVERFLOW`, stat `300 000`, one event at `(domain, 0)`; (2) two faults in both orders around one discarded restart block: sequence first (loses 2 000, then the overrun's `139 000`: device loss `141 000`), discarded `[142000,144000)`, accepted at 144000: header `lost 143000`, **`{overrun, 141000}` at 1000**, stat 2000; overrun first (`140 000` then sequence `2 000`: device loss `142 000`), discarded `[143000,145000)`, accepted at 145000: header `lost 144000`, `{overrun, 142000}` at 1000, stat 2000. The implementer rechecks the numbers against the test's own timeline comment.
- **P8-5** `vh_8_a_discarded_restart_block_is_one_jump_on_both_providers`: `rx_fault_pair` (overrun at 0) with `ScriptedLink` Full for the block that **begins at `G`** (script `[G, G+1)`: after the report the UHD's next call is one packet, UR-17, so its length is not assumed); per Provider: header `lost == G + Σ refused`, `RX_OVERFLOW {overrun, G}` stamped at sample 0, flags `GAP|RESTARTED`, one `LINK_BACKPRESSURE`, stat == Σ refused. Compare the **laws** across Providers (event lost `G` and stamp 0 equal; the refused length per Provider). Precondition (row-specific): the refused list holds exactly one block whose `first == G`.

UHD (all U; blocks via `rx.receive`; assertions on blocks, events, `Stream` fields; a report is qualified by its own read):

| id | feed | accepted block / events |
|---|---|---|
| **U4a** Overrun | `A[0,1000)`, `Overflow` report, `F[5000,6000)` (device jump 4 000: own), `A[6000,7000)` | after `F`: `pending` empty (consumed), `carry.flags == GAP_BEFORE|RESTARTED`, `carry.overflow == (first 1000, lost 4000, overrun)`, **no event yet**, `read 6000`, `expected 1000`. `A`: `(GAP_BEFORE|RESTARTED, Some(5000))`; one `RX_OVERFLOW {overrun, **4000**}` at `(domain, 1000)`; `LINK_BACKPRESSURE` at `(domain, 5000)`; stat `1000` |
| **U4b** Sequence | same with a sequence report | `GAP_BEFORE|SEQ_DISCONTINUITY`, `Some(5000)`; `RX_OVERFLOW {sequence, 4000}` |
| **U4c** Alignment | same | `GAP_BEFORE|ALIGNMENT`, `Some(5000)`; one `ALIGNMENT_ERROR {lost 4000}` at 1000; no `RX_OVERFLOW` |
| **U4d** MissedStart (`LateCommand`, restart) | same | `GAP_BEFORE` alone; no event beyond the `LATE_COMMAND` the report emitted |
| **U4e** Overrun + Alignment | same | both flags; one of each, losses 4 000 |
| **U4s** a report **spent on a gapless discarded block** | `A[0,1000)`, zero-loss `Overflow`, `F[1000,2000)` (own 0), `A[2000,3000)`; also with `Alignment`; also with the report arriving **after** `F` (`F`, report, `A`) | the report is consumed at its own read (`pending` empty after `F`/`A`); `A` is `(GAP_BEFORE exactly, Some(1000))`; **no `RX_OVERFLOW`**, no `RESTARTED`; `LINK_BACKPRESSURE` once; the end makes **no** second stop; with `Alignment`: `ALIGNMENT_ERROR {lost 0}` (today's gapless value), **not** the discard's 1 000 |
| **U5** an unannounced jump inside a stretch | `A[0,1000)`, `F[1000,2000)`, `F[3000,4000)` (device jump 1 000, no report), `A[4000,5000)` | `GAP_BEFORE|SEQ_DISCONTINUITY`, `lost 3000`, no `RX_OVERFLOW`; contrast `A[0,1000)`, `F[1000,2000)`, `A[2000,3000)` → `GAP_BEFORE` exactly, `lost 1000` |
| **U5b** (Sol's sequence) a later report must not take the earlier gap | `A[0,1000)`, `F[3000,4000)` after an unannounced jump, an `Overflow` report, contiguous `A[4000,5000)` | `GAP_BEFORE|SEQ_DISCONTINUITY`, `lost 3000`, **zero `RX_OVERFLOW`**, no `RESTARTED` (the Overrun report is qualified by `A`'s own read: gapless, spent). Fails `jumped \|\| own` (it would give `RESTARTED` and an event and lose the SEQ flag) (N126-U06c) |
| **U6** overlap inside a stretch | `F[1000,2000)`, then the same `[1000,2000)` again (`Repeat` shape) | `rx_overlapping +1`, second copy dropped, stat `1000` (not 2 000). Fails trimming against `expected` |
| **U7** second stop (Q8, the literal rule) | a discarded block that **carries** an Overrun loss report (`F` with a device jump), then the end | `carry.overflow.cause == Overrun` counts as "an overrun reported since the last published block": the end stops twice and writes `rx_stop_overrun_clear` (`LockSpy::seen("rx_stop")`, `ur_26_a_second_stop_follows_only_an_unanswered_overrun_report`'s helper); in **U4s** (spent on a gapless block) it stops once. One line flips both if the orchestrator changes the rule |
| **U7t** tail with a carry | `A[0,1000)`, Overrun, `F[5000,6000)`, plan cut at 9 000, the stream ends before any accepted block | one tail `{overrun, 4000 + (9000 − 6000) = 7000}` stamped `(domain, 1000)`; the clock ends at 9 000 (a loss report pending keeps the plan's cut, UR-18) |

Wrong behaviour overall: events or `pending` consumed at the refused block and lost (the old order, `rx.rs:680, 704-722`: the Overrun/Alignment rows would then emit at the discard or lose their flags); the event's `lost` equal to the header's; a report credited with an earlier gap (U5b); the carry kept across a segment (U2). Mutation rows N126-U02/U02b/U04/U06/U06b/U06c/U12.

Parity for flags, `lost`, events of sequences 1, 2, 4, 5: **P8-1** (§4.10).

### 4.7 Sequence 6 — a segment whose first block is refused
- **MK6** `mr_19_a_refused_first_block_registers_the_clock_and_the_first_accepted_block_counts_from_the_origin`: link pre-filled so block `[0,2000)` is refused; the clock record exists (`mock_radio.rs:2480`); `LINK_BACKPRESSURE` at `(that domain, 0)`; drain; next block `[2000,4000)`: `(GAP_BEFORE, Some(2000))` (SC-13: a first block's `lost` is its index); stat 2000. With capacity 0 and a `Stop` at 20 000: no `RX_OVERFLOW`, stat `20000`, the clock ends at the cut. Old fails: tail `{overrun, 20000}`.
- **MK6c** `mr_19_a_tail_after_a_refused_first_block_is_timed_in_the_registered_clock` (Sol minor: N108-96's overflow-tail branch): capacity 0 (first block refused, clock registered), an `rx_overflow` fault at 5 ms (inside the stretch), `Stop` at 20 ms inside the fault's gap: one tail `{overrun, 20000 − 5000 = 15000}` stamped `(registered domain, 5000)` — in the segment's clock, not the primary root. Fails N108-96 (tail stamped in the root even when a refused first block registered the clock).
- **MK6b** `mr_19_an_abort_at_a_refused_first_block_ends_its_clock_at_the_origin` (kept): case 1 unchanged; case 2's tail `{overrun, 8000}` (`mock_radio.rs:2596`) becomes "no `RX_OVERFLOW`; stat 8000; two `LINK_BACKPRESSURE` (segment 1 at 2000, segment 2 at (domain 2, 0))".
- **U8** first block refused: `F[0,2000)` registers the clock (`core.clocks` record present), `A[2000,4000)`: `(GAP_BEFORE exactly, Some(2000))`, event at `(domain,0)`. Fails: the clock registered only on acceptance; SEQ added to the first-block gap.

### 4.8 Sequence 7 — admission, one reason text
- **R7** (`crates/ezsdr-radio/tests/rx_link.rs`, pure): `rm_38_a_block_link_must_be_its_ports_only_link` over `[]`, `[Block]`, `[DropOldest, DropNewest, DropOldest]`, `[Block, DropOldest]`, `[DropOldest, Block]`, `[Block, Block]` → `None, None, None, Some(REASON) ×3`; `rm_38_the_event_is_owed_once_per_segment`; `rm_38_the_count_is_the_samples_refused`; `rm_38_loss_reports_merge_to_the_first_sample_the_sum_and_the_overrun` (the shared `LossReport::merge`). Rows N126-R01 (`len > 1` → `> 0`), R02 (`any` → `all`), R03 (`raised` never set), R04 (merge keeps the later first sample).
- **MK7** `mr_07_prepare_cases` (`mock_radio.rs:780`) and `mr_7_each_receive_port_takes_its_own_links` (`mock_layout.rs:726`): a sole `Block` admitted; `Block + DropOldest` on one port refused with `message == "RM-38: a Block link must be its receive port's only link"` (the literal in the test, not `REASON`); two ports with one `Block` each admitted. Old fails the **reason text**.
- **U9** `ur_12_prepare_refusals` loses its `Block` case (`fake.rs:1631`); new `ur_12_a_block_link_must_be_its_ports_only_link`: single `Block` → `Ok` (**the old UR-12 refusal fails this**, `mod.rs:513`), `[Block, DropOldest]` → `Err` with the same exact string, two layout ports one `Block` each → `Ok`. The old `contains("Block")` assertion is not reused (the new reason also satisfies it). Row N126-U01.
- **P8-7** `vh_8_a_block_link_is_admitted_or_refused_alike`: both Providers, the three cases, equal `message` strings (extends `vh_8_a_refusal_is_the_same_key_and_reason`, `parity.rs:621`).

### 4.9 Sequence 8 — a drop-class link (regression; one change to say)
Behaviour of a drop-class port is unchanged **except the order of two things on every port, drop-class included**: a device report's `RX_OVERFLOW`/`ALIGNMENT_ERROR` is emitted *after* the publish (it was before, `rx.rs:704-722` vs `:726`), so a consumer thread can see the block a moment before its event enters the collector; the collector stamps events, it does not order them against blocks, and the sequential U/P8 tests read both after `receive`/`settle`. Links that act inside `publish` were audited: `tests/common/measurement.rs:2905` `HeaderRecording` and `tests/common/s9bench.rs:64` `Recording` record the header and forward `inner.publish` (a drop-class link, never `Full`) and depend on no event/block order; `rx/differential.rs:~635` and `parity.rs:168` `MemLink`s likewise. **Still right.** Existing regression tests: the Mock's `DropOldest/DropNewest` receive tests (`mock_radio.rs:857…`, `mr_21_*` with `DropOldest, 8`), UHD's `ur_17_blocks_reach_every_link`. Added **U10** `ur_17_a_drop_class_link_s_outcomes_are_counted_and_discard_nothing`: a `ScriptedLink` answering `DroppedOldest`/`DroppedNewest` for chosen blocks: `link_drops_seen` counts them, `rx_blocks` counts every block, `rx_link_discarded_samples == 0`, no `LINK_BACKPRESSURE`, `expected` advances. Fails: a drop outcome treated as `Full`.

### 4.10 Sequence 9 — both Providers, one table
**P8-1** `vh_8_a_full_block_link_discards_alike_on_both_providers` (`parity.rs`; `Started::booted` **before** the action window (script installed at construction, Stop submitted later); selector `block_len` 2 000, `x3xx`, 1 MS/s; one `const CASES`, both Providers checked against the **same** row; thread-driven, repeat 20 runs):

| case | Full ranges (samples) | expected, both Providers | `LINK_BACKPRESSURE` | `rx_link_discarded_samples` | row-specific precondition |
|---|---|---|---|---|---|
| s1 | `[4000,8000)` | accepted `(8000,2000,GAP_BEFORE,Some(4000))` after `(0,…)`, `(2000,…)` | 1 at `(domain,4000)` | 4000 | the first UHD block is `[0,2000)` |
| s2 | `[4000,8000)`, `[14000,16000)` | the above, then `(16000,2000,GAP_BEFORE,Some(2000))` | 1 | 6000 | same |
| s4 | `[4000,∞)`, orderly Stop submitted at relative 12 ms | no accepted block after 4000 | 1 | **own cut − 4000** per Provider; cut ≥ 12 000 + 6 000 | blocks `[0,2000)`, `[2000,4000)` precede the stretch |
| s5 | overrun at 0 (`rx_fault_pair`), Full for the block that begins at `G` | `GAP_BEFORE|RESTARTED`, header `lost == G + Σ refused`; **`RX_OVERFLOW {overrun, G}` at sample 0** | 1 at `(domain, G)` | Σ the link's refused lengths, per Provider | **no `[0,2000)` prefix** (the report fires before any sample); the refused list is exactly one block with `first == G` |
| s6 | `[0,2000)` | `(2000,2000,GAP_BEFORE,Some(2000))` | 1 at `(domain,0)` | 2000 | the first UHD block starts at 0 and is 2 000 long |

Common asserts: `assert_gap_law` and `assert_loss_law` on each Provider's blocks and events; **no `RX_OVERFLOW` except in s5**; each Provider's stat equals Σ **its own link's** refused lengths and the *laws* (not literals) are compared across Providers in s4 and s5; `rx_overflows == 0` on the UHD stats except s5's injected one. A failed precondition fails the test **with its own message**; the `infra::timing` verdict (as `parity.rs:1062` does for T0 drift) is raised only on independent drift evidence (the UHD's T0 against a probe), never from a precondition. Observation: `rx_links[0].take_blocks()`, `events()`, the two stats sections. Timing: the UHD half runs uhd-rx on a thread against the manual clock (`wait_until` + `settle`); asserts are on content and counts, not instants.

### 4.11 Sequence 10 — the discard itself induces no device overflow (UHD)
**UL10** `ur_17_a_long_stretch_makes_no_device_overflow` (Sol M5; `LockSpy` on a manual `FakeDevice`, `rx.host = HostClock::manual()`, `Rx::turn()` driven while `time.advance_to` and `rx.host.advance_for_test` step one block (2 ms) per turn, as `rx.rs:3390`; **not** `FakeDevice::calls()`, which records no `rx_recv`): `ScriptedLink` Full on `[4000, 404000)` — 400 ms of device time, four times `OVERRUN_BEHIND` — then accepting. After **every** turn (so a failure surfaces before teardown): `stats.rx_overflows == 0`, `stream.pending` has no Overrun, `spy.rx_packet_modes().last() != Some(true)`. At the end: the link's refused list is contiguous from 4 000 and Σ == 400 000 (the **returned sample spans**: the reader read every sample), `spy.rx_requests().len() ≥ 200`, the first accepted block after the stretch is `(404000, …, GAP_BEFORE exactly, Some(400000))`, `rx_link_discarded_samples == 400000`, no `RX_OVERFLOW`, no `RESTARTED`. Wrong behaviour: a Provider that holds or stalls on a full link (the manual clock runs on, the fake reports the reader behind ⇒ `Overflow` ⇒ the per-turn assertion fires). Row N126-U11 (a 150 ms manual-clock stall inserted in the `Full` branch; INFERRED that the manual host's clock moves — the implementer confirms, else the mutant advances it explicitly with `advance_for_test`). Timing: manual device time only; 400 ms is device time, not wall.

### 4.12 Cases the code shows beyond the brief
- **Fault inside / after a stretch** (MK5c, MK5d, U4, P8-5); **`LateCommand` inside a stretch** (U4d); **retune/`start_rx` while discarding** is sequence 3 (a new segment, U2/MK3).
- **`continue` Policy / override**: the Providers emit the event once either way; the reaction is the Kernel's (K2, K3).
- **Device loss inside a stretch** (`DEVICE_LOST`, MR-20/UR-29): **MK8** `mr_20_a_loss_inside_a_stretch_keeps_the_count_and_makes_no_tail` and **U11**: `device_lost` at 6 ms during a stretch from 2 000: clock ends at the loss, stat `Σ` up to the loss, no `RX_OVERFLOW` without a report, one `DEVICE_LOST`. Fails a tail invented from the stretch.
- **A `Full` from a non-`Block` link** (a link whose `policy()` lies): both Providers treat any `Full` as a discard; the `mock_channel` double (`limit`, policy `DropOldest` while answering `Full`) is fixed to say `Block` (§8).
- **Several receive streams**: MK9/U12 (§4.4).
- **Reader performance while discarding** (INFERRED, not tested): the discarded block is built (pool fill, planar copy) before the publish answers, the cost of an accepted one; no pre-check exists in `DataLink`. A risk, not a test.

---

## 5. Mutation rows (id prefix `N126-`)

Rust token anchors, smallest unique text; **anchors for code that does not exist yet come with the code** (the implementer writes the smallest uniquely matching token sequence and runs `anchor.py`/`rules.py`; none invented here). Rows run only through `remote_mutate.py`; every new row is one token-level change, and a mutant that can spin (a cursor that does not advance) is written as an off-by-one instead.

Existing rows touched:

| existing row | verdict |
|---|---|
| T37 | **delete** (anchor goes with the restart arithmetic); N126-M02 replaces it |
| S4A-41 | **delete**; N126-M11 is its inverse, killer MC1 |
| N108-50 | **keep**, killer MK5a (a report pending across the refused publish) |
| N108-98 | **keep, killer → MK5a/MK5c** (v1's MK1 had no pending report, Sol) |
| N108-51, N108-97 | **delete** (anchor `rx.note(next, …)` goes); N126-M05 re-introduces a note at a discard |
| N108-52, N108-53 | **retarget** to the new `rx.lost = …` line: `rx.undelivered = stop` (killer MK4b), `rx.undelivered = next` (killer MK4a) |
| N108-96 (tail stamped in the primary root) | **keep, killer → MK6c** (the overflow-tail branch; v1's MK6 only exercised the event stamp) |
| N108-99 | **keep** (MK6b case 1 unchanged) |
| N108-153 (`report.first = report.first.min(first)`) | **keep** (D3 keeps `first`), killer `mr_37_pending_reports…` (2) and MK5c |
| N108-154/155, N108-27/28/31/33, F44, J08 | keep; N108-31 (`Some((cut - undelivered).max(0) as u64)`) is the **old tail formula**: **retarget** to the new `Σ max(0, min(kg,cut) − kf)` text, killer MK5d |
| P475 | keep, re-anchor if the publish loop moves |
| N108-20/21/22/24/35/35b/35d/41c/85/86/88/89, U47, U48, U25, J13, J45, R08, N67F-01…06 | keep, **re-run all**; re-anchor those whose text moved (U48 `std::mem::take(&mut stream.pending)` is now `consume at the block's own read`); N108-21 (`at_or_after(end) - stream.expected`) and N108-41c (`stream.pending.contains(&Pending::Overrun)`) are **retargeted** to the carry-aware tail/second-stop text |
| P258 | keep; killer set gains U3b/U3c |
| U09 | keep |

New rows (effect → killer):

| id | effect | killer |
|---|---|---|
| N126-R01 / R02 / R03 / R04 | `len > 1`→`> 0`; `any`→`all`; `raised` never set; merge keeps the later first | `rm_38_*` |
| N126-M01 | Mock `Full` branch adds `\| BlockFlags::RESTARTED` | MK1, P8-1 |
| N126-M02 | `rx.next = stop` → `stop.max(next + gap)` (restart gap back) | MK1, MK1b |
| N126-M03 | drop `+ len` from `rx.lost` | MK1 |
| N126-M04 | `rx.next = stop` → `rx.next = stop - 1` (an overlapping next block; **not** `next`, which spins the publish loop) | MK1 (`assert_gap_law`) |
| N126-M05 | call `rx.note(…)` at a discard | MK1 (no `RX_OVERFLOW`) |
| N126-M06 | `refuse` replaced so every discard raises the event | MK1 second stretch, MK4a |
| N126-M08 | `rx_blocks` also counted at a discard | MK1 |
| N126-M09 | stat `+= lost` instead of `+= len` | MK5c (stat 8000 vs header lost), MK5a |
| N126-M10 | block-carried event `lost` = header `lost` instead of `Σ(kg − kf)` | MK5a/MK5c |
| N126-M11 | `head_until` restarted at a discard | MC1 |
| N126-M12 | stat counted before the clip at the cut | MK4a (29 500) |
| N126-M13 | block-carried event stamped at the jump start instead of `report.first` | MK5c |
| N126-U01 | restore `if link.policy() == BackPressure::Block { return Err(…) }` in `mod.rs` | U9 |
| N126-U02 / U02b | `pending.clear()` skipped at a `Full` (a report not consumed at its own read) / consumed **without** carrying its flags and loss | U4a, U4s, U5b |
| N126-U03 | delete the `Streams.delivered` rollback call | U3b |
| N126-U03b | rollback unconditional (ignores `floor_reads`) / the abort request reads `stream.expected` | U3c |
| N126-U04 | in the `Full` branch also call `emit_overflow(…)` with the discarded block's payload | U4a |
| N126-U05 | `if refused.raise_event {` → `if true {` | U1 second stretch |
| N126-U06 / U06b / U06c | `SEQ_DISCONTINUITY` added when `own == 0`; `carry.flags` not merged at a `Full`; report qualified against `expected` instead of `read` (the earlier gap credited to a later report) | U5, U4a, U5b |
| N126-U07 | overlap trim against `expected` | U6 |
| N126-U08 | `read = k + len` deleted in the `Full` branch | U1, U5, U6 |
| N126-U09 | `rx_blocks`/`rx_samples`/`link_drops_seen` at a `Full` | U1 |
| N126-U10 | `recv_len` reads `stream.expected` instead of `stream.read` | U3a |
| N126-U11 | insert a 150 ms manual-clock stall in the `Full` branch | UL10 (per-turn `rx_overflows == 0`) |
| N126-U12 | the carry kept across a segment (not rebuilt in `Stream::new`) | U2 |
| N126-U13 | tail ignores `carry.overflow` | U7t |
| N126-K01 / K02 | no `LINK_BACKPRESSURE` pairs / pairs for every Provider (the `Block` condition dropped) | K1 / K1n |

---

## 6. Order of work (proposal)

1. **S1** `ezsdr-radio::rx_link` + `rm_38_*` + Kernel registration (K tests) + RM-38/SC-20a/KC-8/RS-27 text. 2. **S2** Mock (MK*, MC1, `mr_07`/`mr_7` strings, device intervals and the new tail) + MR-19/MR-7/MR-25/MR-27/MR-28/MR-37/M7 text + rows. 3. **S3** UHD (`mod.rs` admission, `samples()` restructure, `read`/`Carry`, `floor_reads`, the abort floor from `Streams.delivered`, stats key and goldens, U*/UL10) + UR-12/UR-17/UR-18/UR-19/UR-20/UR-30 text + rows. 4. **S4** P8 table and helper, the retargeted-row audit, `rules.py`, `plan/phase8/54-mock-vs-x310.md` RX4 closed, failure-mechanisms ledger. Each step leaves the tree green; S3 is the risky one (a lifecycle/queue change and the Mock/hardware boundary).

---

## 7. Spec text (draft, rule format: normative text, one topic, ending in an indented `*Test:*`)

**design/07 — new RM-38** (after RM-37):

> - **RM-38** A `Block` link on a radio receive port is admitted when it is the port's only link; otherwise `prepare` is `Rejected` with `RM-38: a Block link must be its receive port's only link`. While that link answers `Full` the Provider keeps reading the device and discards what it reads: the device does not overflow and is not restarted. The next block the link accepts carries `GAP_BEFORE` and `lost` equal to its time jump, the discarded samples included (SC-13); a discard adds no `RESTARTED`, `SEQ_DISCONTINUITY` or `ALIGNMENT`, no restart gap and no head, and is no report: it makes no `RX_OVERFLOW`, and a device report's event (RM-17) states the device's loss only, not the discarded samples. The Provider emits one `ezsdr.LINK_BACKPRESSURE` per receive segment, from the stream's node, stamped at the first sample of the segment's first discarded block, and adds each discarded block's length (its clipped length at a cut) to `stats.rx_link_discarded_samples`; under `abort` the count may exceed the cut. Both Providers call `ezsdr_radio::rx_link` for the admission, the count and the once-per-segment rule.
>   *Test:* `rm_38_a_block_link_must_be_its_ports_only_link`, `rm_38_the_event_is_owed_once_per_segment`, `rm_38_the_count_is_the_samples_refused`, `rm_38_loss_reports_merge_to_the_first_sample_the_sum_and_the_overrun`, `vh_8_a_full_block_link_discards_alike_on_both_providers`, `vh_8_a_block_link_is_admitted_or_refused_alike`.

**design/07 RM-17** — drop "`r = a`, the refused block's first sample, for a Mock `Block` link (MR-19), whose next block begins at `max(b, a + gap)` …" (keep `r = f` for an injected fault and the USRP clause); drop the sentence "A block a `Block` link refused (MR-19) is an overrun report at its first sample and follows this rule like any report."; in the third paragraph: "`radio.RX_OVERFLOW` … `lost` the **device loss** of the reports that block carries (RM-38: samples a link discarded are not in it), at the first sample of that loss; or, when a cut ends the segment first, at the segment's end, `lost` the device loss up to the cut (a `Stop`'s cut survives a later abort)"; "(the first report's `k_f`)" is kept as the stamp. Test line: remove `mr_19_backpressure_is_an_overrun`, `mr_19_repeated_refusals_…`, `mr_19_a_fault_report_survives_a_refused_publish`, `mr_19_a_stop_before_acceptance_emits_the_tail_from_the_refused_block`, `mr_19_a_fault_inside_a_refusal_s_gap_then_a_stop_counts_the_stretch_once`, `mr_19_a_refused_first_block_registers_the_clock_and_its_tail_is_timed_in_it`; add `mr_19_a_full_link_discards_and_the_next_block_carries_the_gap`, `mr_19_a_fault_after_discards_reports_the_device_loss_only`, `mr_19_a_fault_inside_a_stretch_then_a_stop_reports_the_device_loss_to_the_cut`, `mr_19_a_tail_after_a_refused_first_block_is_timed_in_the_registered_clock`, `ur_18_a_report_spent_on_a_gapless_discarded_block_makes_no_event` (U4s), `ur_18_a_later_report_does_not_take_an_earlier_gap` (U5b).

**design/02 SC-20a** — replace "or, where its device cannot wait for a consumer (UR-12), it records the refused block's loss as its device's overrun, an RM-17 report at the refused block's first sample (MR-19), and continues; a cut at or before that sample loses nothing before the cut." with "or, where its device cannot wait for a consumer, it discards what the device delivers while the link is full, raises `LINK_BACKPRESSURE` once per segment and carries the discarded span in the next block it publishes (RM-38); a cut under `abort` is the first sample no accepted block carries or, where the abort was booked while a block was being published and discarded, the end of that block (UR-17)." *Test:* `sc_20a_full_is_not_a_silent_drop` (the retry clause), `mr_19_an_abort_in_a_stretch_cuts_at_its_first_sample` (Mock), `ur_24_an_abort_in_a_stretch_cuts_at_its_first_undelivered_sample` and `ur_24_an_abort_booked_while_a_block_is_discarded_ends_the_stream_at_that_block_s_end` (UHD), `rm_38_*`, `kc_08_a_provider_with_a_block_link_has_a_link_backpressure_row_at_every_node`. **SC-18**: "A discard (SC-20a, RM-38) appears as the next block carrying `GAP_BEFORE` with `lost` the discarded span, plus the flags of any device report that block carries (RM-17), and no `RESTARTED` or `SEQ_DISCONTINUITY` of its own." *Test* (SC-18): add `mr_19_a_full_link_discards_and_the_next_block_carries_the_gap`. §11 deferred: unchanged (Providers send `{}`).

**design/09** — **MR-19** (rewrite, citing RM-38): "A publish that returns `Full` is RM-38's discard. The Mock reads on: the block is not delivered and not counted in `rx_blocks` or `rx_samples`; the segment's next block begins at its end `b`; the first sample no accepted block carries stays where it was (RM-16), so a cut under `abort` there loses nothing and a `Stop` inside the stretch counts the blocks up to its cut. The pending `lost` takes `b − a` and the pending flags `GAP_BEFORE`, whatever a fault left pending (MR-21, MR-22); a fault's report is not consumed and its event states the fault's own loss (`Σ (k_g − k_f)`) at its first sample. The restart gap (MR-21) and the head (MR-35) do not apply. A segment whose first block was refused has a clock (MR-25), in which RM-38's event is stamped." *Test:* the MK/MC tests of §4. **MR-7 step 7**: "Among one port's links, `ezsdr_radio::rx_link::refusal` decides (RM-38; the refusal reads `RM-38: a Block link must be its receive port's only link`)." *Test* (`mr_07_prepare_cases`): "a `Block` link beside another on one port refused with RM-38's reason; a sole `Block` admitted". **MR-25**: "(`k_f` of a fault, `a` of a refused block)" → "(`k_f` of a fault, `a` the first sample of a discarded stretch)"; "(MR-19)" → "(RM-38)". **MR-27**: "the `radio.RX_OVERFLOW` after a cut inside the gap says the cut's sample less the first sample not delivered (RM-17)" → "…says the device's loss up to the cut, from the first report's first lost sample (RM-17), a discard excluded"; the stats list gains `rx_link_discarded_samples`; the `*Test:*` adds `mr_19_a_full_link_discards_and_the_next_block_carries_the_gap` (the key from construction). **MR-28**: "Every event the Mock emits is RM-10's … except `RX_OVERFLOW`, which MR-37 puts on the hot path" gains "and `ezsdr.LINK_BACKPRESSURE` (RM-38), a Kernel kind emitted through `emit_control` from the stream's node with payload `{}`"; *Test* adds `mr_19_a_full_link_discards_and_the_next_block_carries_the_gap`. **MR-37**: "A report — a fault's or a refused block's (MR-19) —" → "A report — a fault's —"; "A report's overflow, a fault's or a refused block's, is emitted" → "A report's overflow is emitted"; the Test sentence naming `mr_19_backpressure_is_an_overrun` and `mr_19_a_fault_report_survives_a_refused_publish` → `mr_19_a_fault_after_discards_reports_the_device_loss_only`. **M7**: "Backpressure | A discard, the exact gap in the stream (RM-38) | An overrun's restart (the device does not restart); retrying forever (a device cannot wait for its host) | none". Test-table rows `:312-317` and the `mr_21_overrun_shape` row rewritten to the MK tests.

**design/18** — **UR-12**: "or one whose policy is `Block` (a device cannot wait for a consumer; it overflows); several links on one port are allowed" → "or a link set `ezsdr_radio::rx_link::refusal` refuses (RM-38: a `Block` link must be its port's only link); several drop-class links on one port are allowed". *Test:* `ur_12_prepare_refusals` (the `Block` case removed), `ur_12_a_block_link_must_be_its_ports_only_link`; table row `:557` likewise. **UR-17 publish paragraph (`:238`)**: "… to every link of its port (SC-11); a publish answering `DroppedOldest` or `DroppedNewest` is counted (`stats.link_drops_seen`); one answering `Full` is RM-38's discard: the block is not counted in `rx_blocks` or `rx_samples`, `expected` stays, `read` moves to its end, the block's own flags and device loss are carried to the next block a link accepts, and its events wait for that block. Each block is judged against `read` as today (a report spent on a gapless block is spent, a jump with no report is `SEQ_DISCONTINUITY`), so a report never takes an earlier gap. An abort booked while a block is being published and discarded cuts at that block's end." *Test* adds U1, U3a–c, U4a–e, U4s, U5, U5b, U6, U7, UL10. **UR-18**: "the next block carries `GAP_BEFORE | RESTARTED` with `lost` its whole time jump" keeps the header `lost`; "The reports before one jump block produce RM-17's single `RX_OVERFLOW`" → "…whose `lost` is the device loss (RM-38)"; the tail sentence gains "the device loss up to the cut". **UR-19**: "The `RX_OVERFLOW` reports before that block produce at most one event with its whole `lost`" → "…with the device loss of those reports (RM-38 excludes discarded samples)"; "`radio.ALIGNMENT_ERROR { lost }`" states the device's loss of the block that read the report. **UR-20**: "every other event is emitted with `emit_control` … with RM-11's source, time and payload" gains "except `ezsdr.LINK_BACKPRESSURE` (RM-38), a Kernel kind with source the stream's node and payload `{}`". **UR-30** stats list (and its table row, `Records::with_limit`) gains `rx_link_discarded_samples`; *Test* `ur_30_the_sections_are_written` adds the key. Changes-table rows in 02, 04, 06, 07, 09, 18.

**design/04 RS-27** and **design/06 KC-8**: §3. KC-8 *Test* adds `kc_08_a_provider_with_a_block_link_has_a_link_backpressure_row_at_every_node`, `kc_08_a_provider_without_a_block_link_has_no_link_backpressure_row`; RS-27 *Test* adds `kc_08_a_link_backpressure_event_marks_the_open_artifact` (K3) and K2's name.

`rules.py --base 5bc555d3` must pass; RM-38 is a new id; MR-19 is rewritten, not appended.

---

## 8. Audit of the existing tests (AGENTS §9 step 1 table)

`grep -rn "BackPressure::Block" crates/*/tests crates/*/src` plus the tests the mutation rows name (T37, S4A-41, N108-50…99, P475); every hit was read:

| test (file:line) | expectation came from | verdict |
|---|---|---|
| `mock_radio.rs:2247 mr_19_backpressure_is_an_overrun` (incl. the fractional-root half `:2274`) | MR-19 overrun: `RESTARTED`, `lost G`, event with the block | **fixed** → MK1, MK1c |
| `:2288 …a_zero_gap_still_loses_the_dropped_block` | `k_g = max(b, a+gap)` | **obsolete** → MK1b |
| `:2314 …repeated_refusals_are_one_overrun…` | each refusal adds `G` | **fixed** → MK1 second stretch |
| `:2345 …a_fault_report_survives_a_refused_publish` | refusal adds a restart gap, one event with `2G` | **fixed** → MK5a, MK5b |
| `:2380 …a_stop_before_acceptance_emits_the_tail_from_the_refused_block` | tail from the refused block | **fixed** → MK4a |
| `:2410 …an_abort_after_a_refusal_loses_nothing_before_its_cut` | abort cuts at `a` | **still right** for the cut, extended → MK4b |
| `:2435 …a_fault_inside_a_refusal_s_gap_then_a_stop_counts_the_stretch_once` | tail `{overrun 28000}` at 2000 | **fixed** (Q1: `{overrun 20000}` at 10000) → MK5d; accepted-block half → MK5c |
| `:2468 …a_refused_first_block_registers_the_clock_and_its_tail_is_timed_in_it` | tail 20 000 from the origin | **fixed** → MK6, MK6c |
| `:2490 mr_37_pending_reports_keep_the_first_sample_and_the_causes_not_a_list` | `G` per refusal | **fixed** → MK5e |
| `:2550 …an_abort_at_a_refused_first_block_ends_its_clock_at_the_origin` | clock/row end; case 2 tail | case 1 **still right**, case 2 **fixed** → MK6b |
| `mock_radio.rs:3060 mr_21_overrun_shape` (the `blocked` harness) | `4G + 1000` over three refusals | **fixed**: replaced by MK5c; the rest of the test untouched |
| `mock_radio.rs:780 mr_07_prepare_cases` | refusal text "multiple rx links cannot use Block" | **fixed** (exact RM-38 text; sole `Block` admitted) |
| `mock_layout.rs:726 mr_7_each_receive_port_takes_its_own_links` | same text | **fixed** (string); two-ports-admitted half **still right** |
| `mock_channel/s4a.rs:327 s4a_a_back_pressure_restart_begins_with_the_head` (`radio.link.limit`, `mock_channel.rs:95-117`) | back-pressure restart has a head | **fixed** → MC1; the `limit` link reports `DropOldest` while answering `Full`: **fixed** to `Block` |
| `mock_radio.rs:164`, `mock_layout.rs:~140` `MemLink` (`Block ⇒ Full`) | link double | **still right** |
| `stream_contract.rs:300-380` `sc_20/sc_20a_full_is_not_a_silent_drop` (`RetryingProducer`) | SC-20a's retry clause | **still right**; only the spec's Test line changes |
| `stream_contract.rs:429`, `spec_binding.rs:1259/1778/4899`, `module_api.rs:*`, `link_host.rs:62-85`, `kernel/tests/support/*`, `sink-capture` | `Block` into a Sink refused (SC-21), `HostLink` `Full` | **still right** (not a radio port) |
| `fake.rs:1631 ur_12_prepare_refusals` (`Block` case) | UR-12 refusal | **fixed** (moved; `contains("Block")` not kept) |
| `fake.rs:3593 ur_30_the_sections_are_written` (stats keys) | the UR-30 key list | **fixed** (add the key) |
| `tx/tests/e2b.rs:784…` (16 JSON stats literals), `tx/tests/e3.rs:258-306` (41 occurrences of the key list in `stats link_drops_seen=0 rx_before_origin=0 …` strings), the Mock's `lib.rs:421` initial object | golden `stats` objects | **fixed** (D7/Q5: add the key after `rx_errors`, alphabetical; Mock initial object) |
| `tests/common/s9bench.rs:392-812` | reads stats **by key** | **still right** |
| `tests/common/measurement.rs:2905`, `s9bench.rs:64` recording links | record then forward | **still right** (§4.9) |
| `parity.rs` `MemLink::new()` users; `parity.rs:881 assert_rx_overflow_output` | drop-class; `payload.lost == header.lost`, `event_at == first − lost` | users **still right**; the helper **fixed**: the equalities hold only with `discarded == 0`, else the loss law (Q1) |
| `rx/differential.rs` layer 3 (`rm_26_the_timeline_against_uhd_rx`) | drop-class | **still right**; its generator has no full-link op and gets none |
| mutation rows T37, S4A-41, N108-51/97 | see §5 | **delete**; N108-31/21/41c **retarget**; N108-50/98/52/53/96/99, P475, U48 keep/re-anchor |
| `mock_radio.rs` `mr_21_*`, `mr_22_*`, `mr_25_*`, `mr_20_*` (faults, `DropOldest`) and `vh_8_*` | device reports, no `Block` | **still right**; they are the regression for the carried-report change (no discard: event lost == header lost, stamp == `first − lost`) |

KNOWN_LIMITATIONS: no entry changes and none is added (Q10: owner 2026-11-20, discarding while full is the documented meaning of `Block` there). KL-47 ("on MockRadio and FakeDevice, every restart after an overrun") stays true (a discard no longer restarts). KL-4/KL-8 are about device overflows. Other records at landing: `plan/phase8/54-mock-vs-x310.md` RX4 closed, `plan/maintenance/failure-mechanisms.md` (`radio-rule-implemented-twice`; the carry is also `pending-provenance-collapsed`'s shape, counted there only if a bug of that form is found), handoff line.

---

## 9. What the plan could not close

- **The `floor_reads` counter** (§2.3) rests on `first_undelivered` being read only by bookings; the implementer greps the callers first. If another reader exists, the counter counts only the booking paths (`Provider::stop`, a loss).
- **The stall mutation (N126-U11)** needs the manual `HostClock::sleep` to move the clock (INFERRED, `rx.rs:3413` uses `advance_for_test` explicitly); the plan names the fallback.
- **Step instants** of MK1/MK4/MK5 (which `harness.step(..)` lands on which block boundary) and the numbers of MK5e (2) are taken from the old tests' comments; the implementer checks each with a run.
- **P8 block shapes**: the plan asserts row-specific preconditions instead of assuming the fake's 2 000-sample blocks and the post-overrun packet length; whether the fake delivers `[0,2000)` first at `block_len` 2 000 is checked by the precondition, not known.
- **K rig**: whether `distinct_resource_docs` and `two_link_docs` combine into one Run with a second Provider (the rig shape is INFERRED from `coordinator.rs:2258` and `:3615`); the implementer builds it, the assertions are fixed.
- **Not tested**: the reader's cost while discarding (§4.12).
