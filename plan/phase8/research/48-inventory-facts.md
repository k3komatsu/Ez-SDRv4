# Inventory of mechanisms whose reason (Run-time change / timed setting / preemption) may be gone — FACTS ONLY

Tree: `~/local_work/Ez-SDRv4`, HEAD 9dab08a, working tree as found (no edit, no build, no cargo). Date 2026-11-05.
Scope: `crates/ezsdr-radio-uhd/src`, `crates/ezsdr-radio/src`, `crates/ezsdr-mock-radio/src`, `design/07-*`, `09-*`, `18-*` (+ their `tests/` as consumers).
Conventions: "prod" = non-`#[cfg(test)]` code reached by `ezsdr-server`/Session; "test-only" = callers only under `#[cfg(test)]` or `tests/`. Lines at or after the first `#[cfg(test)] mod tests` of a file (tx.rs:1193, rx.rs:708, core.rs:1369, device.rs:2119, control.rs:507, mod.rs:743) are tests and were excluded from "prod" reachability.
"Why added" = the first commit that adds the string (`git log -S'<id>' --format='%h %ad %s' -- crates design | tail -1`) or the comment / design-note / spec rule cited in the source. VERIFIED = I traced the caller chain by grep and reading; UNSURE = not traced to the end.

## 0. Already gone in HEAD (negative results; nothing to inventory)

- `timed_instant`, `covering`, `eligib*`, `debt`, `drain_timed`, `held_tune`, `pending_tune`, `flush_unconsumed`, `restore` as code: 0 hits in the three `src` dirs. Search: `grep -rEn 'timed_instant|covering|eligib|debt|held_tune|pending_tune|flush_unconsumed|restore' crates/ezsdr-radio-uhd/src crates/ezsdr-radio/src crates/ezsdr-mock-radio/src` (the `covering`/`eligib`/`restore` hits that exist are in `tests/` or other crates).
- States `FromNone`, `Release`, channel move: gone (`Holder` has only `Free`, `Owner`, core.rs:325-336). One stale comment still names `FromNone` (§6).
- Spec stubs: UR-24 (`18-uhd-radio.md:267`), RM-34 (`07-radio-model.md:330`), RM-36 (`:349`), MR-40 (`09-mock-radio.md:219`) are one-line "Withdrawn (note 47 stage 2)"; ezsdr-sim `cold_change_refused` withdrawn (`design/08-simulation.md:104`).
- Update classes: only receive `frequency_hz`/`gain_db`/`antenna` have one (`cold`); every transmit key and every rate is classless (`crates/ezsdr-radio/src/lib.rs:184-193`). `HardwareTimed` outside kernel appears only in tests (`ezsdr-radio-uhd/tests/fake.rs:5400-5405`, `ezsdr-mock-radio/tests/mock_radio.rs:1720,1741`).
- **Transmit lines hold at most one segment.** `Control::book` books only `Kind::Stop` (control.rs:280) on a Tx line; `Streams::end` books `Kind::End` (core.rs:562); `Kind::Start` is booked only for Rx (control.rs:324). In the planner replay `Kind::Stop | Kind::Start` do nothing for Tx (`timeline.rs:477`); the only Tx cut is a loss/`Provider::stop` at or before the clock's origin (`timeline.rs:483-495`, `State::finish` → `end_at_origin`). Mock books only `Kind::End` on Tx (`mock-radio/src/lib.rs:1177`). This bears on §2.7 and §6.

---------------------------------------------------------------------------------------------------

## 1. Timed settings applied to a running stream (timed tune/gain/rate, `set_command_time`, DDC/DUC FIFO, release window)

Production fact first. The ONLY non-test call of `Device::apply` is `provider/core.rs:1031` `self.device.apply(dir, *chan, settings, None)` (untimed, in `Core::configure`). Search: `grep -rn --include='*.rs' -E 'apply\(' crates/ezsdr-radio-uhd/src | grep -vE 'fn apply|inner.apply'` → outside tests only core.rs:1031. Non-test `rx_stop` calls pass `None` (rx.rs:287, 405). `grep -rn 'rx_start_now' crates/ezsdr-radio-uhd/src` → no prod caller (trait decl device.rs:193, impls device.rs:1563 / uhd.rs:1001, test wrappers).

| # | Item (def) | What | Why added | Uses | Reachable today |
|---|---|---|---|---|---|
| 1.1 | `Device::apply(.., at: Option<i64>)` timed branch: trait `device.rs:186`; `UhdDevice::apply` uhd.rs:887-950 (`uhd_usrp_set_command_time` uhd.rs:534,567,896; clear 549,582,935); `FakeDevice::apply` device.rs:1407-1520 | timed write queued by the device (command-time bracket) | `git log -S'uhd_usrp_set_command_time'` 27065e7 2026-09-27 "feat(phase7): steps 3-4, the Module ezsdr.radio.uhd 0.1.0"; spec UR-2 | prod passes `None` only (core.rs:1031). `Some(..)`: device.rs tests 2342-3177, core.rs:1381,1598,1783, rx.rs:859, `tests/hardware.rs:573,616,839,3147`, `tests/common/measurement.rs:1278` (`measure_command_queue_depth` 1436-1470) | `at: Some` NOT reachable from any Session operation (VERIFIED by the grep above); reached only from tests and the measurement harness |
| 1.2 | `UhdDevice::tune_dsp` (uhd.rs:523), `tune_auto` (uhd.rs:~558) | bench probes of DDC/DUC shift, timed or not | `git log -S'fn tune_dsp'` 3109743 2026-10-24 "test: design note 39 probes"; `tune_auto` a9b0a26 2026-10-28 "stage D9a" | only `tests/hardware.rs:1011-1069` | test-only (VERIFIED: `grep -rn 'tune_dsp\|tune_auto' crates`) |
| 1.3 | `FakeDevice` timed command queue: `Fake.commands`/`last_command` (device.rs:~752-756), `settle` (1028), `FakeConfig::command_queue` (380), back-pressure in `apply`; spec UR-33 "configuration" paragraph (`18-uhd-radio.md:~335`) | in-order timed command queue, blocks when full | `git log -S'command_queue: usize'` 27065e7 (Phase 7 steps 3-4) | fed only by `apply(at: Some)` (1.1) | test-only (follows 1.1) |
| 1.3a | `FakeDevice` DSP FIFO: `DSP_FIFO=32` (device.rs:501), `struct Dsp` (507: `fifo`,`log`,`flows`,`cursor`,`tx_header`,`tx_header_stale`), `dsp_of` 532, `freq_of` 539, `freq_constant` 548, `open_flow` 558, `close_flow` 568, `drain` 1047, `end_rx_flow` 1079, observers `dsp_pending/dsp_log/freq_at` 1089-1110; spec UR-33 "DSP" paragraph (`18-uhd-radio.md:~336-338`) | model of the DDC/DUC timed-frequency FIFO taken by passing samples | `git log -S'DSP_FIFO'` a9b0a26 2026-10-28 "stage D9a — receive retunes reach the DDC only while samples pass it" | `fifo` is pushed only by timed `apply`. `log` also records untimed frequency writes and is read by `freq_of`/`freq_constant`/`sample_at` (device.rs:1148) to produce receive samples | FIFO part test-only; `log`/`flows` part prod (VERIFIED: `sample_at` is in the `rx_recv` path) |
| 1.3b | `FakeFault::TxBlocks` (device.rs:292, used 1873; blocks only a TIMED tx_send) | scripted hang of timed sends | 27065e7 | `tests/fake.rs:2029` | test-only; every prod tx_send is timed except the T3 flush (§2) |
| 1.3c | `FakeFault::RxApplyFails` (device.rs:363, 1478); doc says "uhd-control's configuration of an enable from no stream" | first receive `apply` refused once | `git log -S'RxApplyFails'` f487877 2026-10-19 "stage 1b-4c" | rx.rs:2124, 2574; `tests/fake.rs:912,1313,1513,7131` | the refused-apply path is real (owner's configure at a start); doc text stale (§6) |
| 1.4 | `timeline::Stream.release_window` (`ezsdr-radio/src/timeline.rs:104`), read in `State::begin` ready term (`timeline.rs:538`: `cut + release_window + ceil(call) + allowance`) | after a receive cut, readiness of the next segment counts the release window | `git log -S'release_window_ns'` 30082bd 2026-10-24 "design note 38 stage 38a — MockRadio and the UHD Module share one timed-command rule (#62)"; spec RM-25 (`07-radio-model.md:257`) | set: control.rs:156,164 (Rx only), `mock-radio/src/lib.rs:1446`; tests: radio/tests/timeline.rs:32-99, mock tests/timeline.rs:193-197, both differentials | reached on Stop then `start_rx` (a previous cut exists): VERIFIED in `begin` (`self.previous` set by `cut`). Its stated reason (a timed command still in the release window at the cut) has no producer now. 47 §2.1 item 4 says only this term is to be removed |
| 1.5 | `TimingEnvelope.release_window_ns`, `keys::RELEASE_WINDOW_NS`, `check_start_lead` (RM-6: `rx_start_lead_ns >= release_window_ns`) (`ezsdr-radio/src/lib.rs:57,181,545,561-566`; x3xx.rs:385 = 3 ms; channels.rs:~359 ideal 0; schema `schemas/radio/envelope.v1.json`) | envelope capability / measurement key | 30082bd (38a) | `check_start_lead`: channels.rs:373, `provider/mod.rs:346`, tests; measurement.rs:307,368 (LowerBound); `provider/mod.rs:908-931` (candidate test) | published and validated at every Run (prod, VERIFIED); only consumer with timing effect is 1.4 |
| 1.6 | `Item.ready`, `Item.delivered`, `Kind::End{abort}` | readiness / delivered floor / skip of items after an end | spec 22; `87bb265` 2026-10-08 "put MockRadio and the UHD Module on one stream timeline" | `Streams::book` core.rs:527, `Streams::end` 562; `replay` skip `ready >= ended` (timeline.rs:454) | prod VERIFIED (End, Stop, start_rx) |
| 1.7 | `Line::made` (timeline.rs:244) | the segment as the items before `(t, seq)` have made it | `git log -S'fn made('` 87bb265 | core.rs:527 (Rx End `delivered`), `mock-radio/src/lib.rs:689,1226`, mock tests/timeline.rs:268 | prod VERIFIED (Rx `End` booking; Mock `rx_runs`/`rx_settle`) |
| 1.8 | `Line::refuse` (timeline.rs:215), `Lines::refuse` 331, `Streams::refuse` core.rs:542, `Item.refused`, `State.refused_running` (timeline.rs:435-440) | a start whose configuration the device refuses halts the stream | `refused_running`: 266633b 2026-10-28 "stage 43a — a refused segment no longer advances later origins" | rx.rs:423 (`refuse_segment`) | prod: a device refusal at `Rx::begin` (owner_configure / `rx_start` error) VERIFIED |
| 1.9 | `Line::halted(t)` (timeline.rs:251) and `State.halted` (428; written 470; read 255) | "a refusal has halted the stream" as a query | `git log -S'pub fn halted'` 84df09e 2026-10-09 "fix: a halted direction has no channel for a timed update; Pps only with a locked reference" | `grep -rn '\.halted(' crates \| grep -v raw.halted`: ONLY `crates/ezsdr-radio/tests/timeline.rs:469,513,540,677` | NOT reachable in prod: no non-test caller (VERIFIED). Its original consumer (a timed update to a halted direction) is gone |
| 1.10 | `Line::prune` (timeline.rs:228), `Lines::prune_stream` 340, `Streams::prune` core.rs:589, `State.start` folding; #63 | keep the plan bounded by folding old items | `git log -S'pub fn prune'` 7f68017 2026-10-09 "refactor: keep each stream's plan once on the UHD Module (note 23)"; RM-25 last para | rx.rs:330 (`finish_locked`, each Rx segment end) | prod VERIFIED. Not tied to timed updates: `start_rx` may carry a future `at` and a later `Stop` can take effect before it |
| 1.11 | `ColdConfig` (control.rs:41-80), `Planned.settings` (control.rs:83-96), `StreamState.configs` (core.rs:~358), `Streams::planned` (core.rs:597) | per-segment configuration snapshot shared by control and owner | `git log -S'struct ColdConfig'` a71ed96 2026-10-03 "fix: preserve later timed updates across cold switches" (name from the cold-switch era); now the `start_rx` snapshot (control.rs:37-40 comment, RM-21/UR-26) | written control.rs:323 (`ColdConfig::of`); read by `Planned::configure` → `Core::owner_configure` (core.rs:1308) | prod VERIFIED; `ColdConfig::new` is `#[cfg(test)]` (control.rs:48) |
| 1.12 | `Core.tx_dsp_gates`, `Core::lock_tx_dsp` (core.rs:735,819,1065-1070); comment core.rs:734 "A timed TX DSP write and the open-time flush share each channel's gate (D9b-1a)" | per-channel gate between a timed DSP write and the open-time flush | `git log -S'lock_tx_dsp'` 80dd2f6 2026-10-30 "stage D9b-1a — every transmit send timed, two-sample EOB, flush at open, design note 39 §6.6" | `grep -rn 'lock_tx_dsp\|tx_dsp_gates' crates/ezsdr-radio-uhd/src`: core.rs:1097 (`#[cfg(test)] tx_flush`), 1188 (`open_tx_device`), 1198 (`reopen_tx_streamer`); the "timed DSP write" side exists only in tests core.rs:1380,1597 | taken by prod openers; no prod code contends for it (VERIFIED by the grep) |
| 1.13 | Fake DUC header model: `Duc`, `DucPacket`, `Radio` (device.rs:602-735), `Dsp.tx_header/tx_header_stale`, `stale_tx_header` 583, `clear_tx_header` 591, `Duc.eob_lost` | models X3x0 DUC: first timed header supplies time, untimed continuation clears `has_time`, one-sample EOB loses its ACK and the next timed header is ignored, stale header advances the FIFO | `git log -S'struct Duc'`, `eob_lost`, `tx_header_stale`: 80dd2f6 (D9b-1a); gh #70 "A one-sample device burst on X3x0 makes the next TX burst lose its time", #72 "A timed TX frequency change inside a running burst is not applied on X3x0 (untimed continuation sends)" | `Duc::packet` runs in every fake tx_send (device.rs:1959); header staleness set at tx_open/close (1831, 2090, 2105). The untimed branch (`at == None`) is hit in prod only by the T3 flush (core.rs:1112) | the model is exercised by every send; sub-cases that need a timed FIFO entry are test-only. `eob_lost` is reachable by a one-sample device burst (#70) — UNSURE whether prod ever sends one |
| 1.14 | `min_timed_command_lead_ns`, `ProviderInstance.min_command_lead`, `DEVICE_LEAD_NS`/`DELIVERY_ALLOWANCE_NS` (`profile.rs:8-21`), `timeline::lead_left` (386), `START_LEAD_SHORT` | Kernel admission lead for every timed Action; RM-14 device lead; start-lead-short warning | `min_command_lead` f0d72af 2026-09-25 (Phase 2); `lead_left`/`START_LEAD_SHORT` first added by d27718a 2026-11-05 "note 47 stage 3" | rx.rs:375 (`Rx::begin`); UHD only (`grep START_LEAD_SHORT crates/ezsdr-mock-radio/src` → none) | prod VERIFIED; not a Run-time-change mechanism, listed to mark the boundary |

---------------------------------------------------------------------------------------------------

## 2. Transmit sends where every chunk carries a time; device burst state; closing send >= 2 samples

Untimed-send search: `grep -rn --include='*.rs' 'tx_send(' crates/ezsdr-radio-uhd/src` outside tests → four prod call sites: tx.rs:563 (held-tail send, `at=Some(clock.instant(next-1))`), tx.rs:594 (payload, `at=Some(clock.instant(open.next))`; comment tx.rs:593 "T1: every payload-bearing packet names its first sample, even inside a device burst"), tx.rs:758 (`close_device_burst`, `at=Some`), core.rs:1112 (`tx_flush_locked`, `at=None`, sob+eob, two zeros). So untimed sends in prod = only the T3 flush. Spec UR-22 (`18-uhd-radio.md:247`): "Every payload-bearing `Device::tx_send` names the first sample's instant ... The only sample-bearing untimed send is T3's two-zero SOB/EOB flush".

| # | Item (def) | What | Why added | Uses | Reachable today |
|---|---|---|---|---|---|
| 2.1 | "every chunk timed" rule: `Tx::send` (tx.rs:563-594), UR-22, fake DUC split (1.13) | every send names its first sample so the DUC takes a timed FREQUENCY at the right sample (#72) | `git log -S'padded_eob'` 80dd2f6 2026-10-30 "stage D9b-1a — every transmit send timed, two-sample EOB, flush at open, design note 39 §6.6"; 46 F2 text: "UR-22 がすべての送信を時刻つきにしたのは #72 のため" | every send | prod VERIFIED. The time also pins the first sample for chunking (`TX_SEND_CAP`) and report correlation (`remember_burst(target)`), so removing it is not decided by this fact alone |
| 2.2 | T3 flush at open/recovery: `Core::tx_flush_locked` (core.rs:1102), `open_tx_device` 1186, `reopen_tx_streamer` 1195, `open_tx_device_locked` 1215, `TxFlushResult`, `tx_report_stash`/`stash_tx_report`/`take_stashed_tx_report`/`lock_tx_reports`/`tx_report_gates` (core.rs:~734-770, 1072-1090) | untimed two-zero SOB/EOB clears stale DUC header; reads per-channel BurstAck within 100 ms; unrelated reports stashed | 80dd2f6 (D9b-1a); UR-22 para 3 (`18-uhd-radio.md:249`) | `open_streamer` (core.rs:1262, from `arm`/`owner_configure`), `Tx::reopen_streamer` (tx.rs:782); `Core::tx_flush` (1093) is `#[cfg(test)]` | prod at every transmit streamer open (arm) VERIFIED; recovery use in 2.5 |
| 2.3 | `TxChain` (tx.rs:111-193): `burst: DeviceBurst{Closed,Open{next,tail}}`, `front`, `recovery: Recovery{Clean,ReopenNeeded,Halted}` and methods | one owner for device-burst, safe-start front and recovery per streamer | `git log -S'TxChain'` 695e1f4 2026-10-31 "stage L2 — transmit device chain state as one TxChain per streamer, design note 44 §4" | `Tx::step/send/end_open/close_device_burst/abandon/reopen_streamer/reports/shutdown` (tx.rs:290-1180) | burst/front: prod, every send VERIFIED. `ReopenNeeded`: set at tx.rs:622,640,765,775 (EOB/close send failed or short) — fault path only. `Recovery::Halted`: written tx.rs:182 (`reopen_failed`), reset by `provider_stop`/`lose`; sole reader is `#[cfg(test)] fn recovery()` (tx.rs:145) — `grep -n 'Recovery::Halted' tx.rs` shows no prod read (VERIFIED) |
| 2.4 | `Tail` (tx.rs:79-90), last-sample hold-back (`hold`, `padded_eob`, `device_count`, `open_ended`; tx.rs:~540-575), `Tx::close_device_burst` (742), `end_open` (715) | closing EOB send has >= 2 samples per channel; last sample held until no burst can still be booked at the next sample (#70; UR-22 "A device burst closes only with an EOB send of at least two samples") | `git log -S'struct Tail'` 21128b9 2026-10-29 "stage TXS — 200 MS/s transmit, bench 2026-10-29, design note 39 §6"; `padded_eob` 80dd2f6 | every finite burst and every device-burst close | prod VERIFIED. Spec rows `07-radio-model.md:470`, UR-22, MR-17/MR-25 ("two-sample reservation margin") |
| 2.5 | `Tx::reopen_streamer` (tx.rs:782-840), `halt_failed_reopen` (841-857), `Core::reopen_tx_streamer`, `Tx::pass` reopen first (293), `send` early return (513) | after an uncertain EOB close: close streamer, reopen, T3, continue held bursts; failure halts the stream (RM-25); 46 F8 (a) keeps it | `fn reopen_streamer`, `halt_failed_reopen`: 80dd2f6 | `pass` → `reopen_streamer` only when `chain.needs_reopen()` | reachable only from an EOB/close send failure or short accept (tx.rs:622,640,765,775) VERIFIED |
| 2.6 | Report correlation: `PendingReport`, `Tx.unacked`, `Tx.late_reported`, `remember_burst`, `reported_targets`, `time_error_target`, `target_pending`, `sync_streamer`, `Tx.streamer` generation, `StreamState.generation` (tx.rs:73,205-224,355-409,1052-1060; core.rs:~350-362) | per-channel BurstAck / TimeError retire pending bursts; generation drops correlations of a replaced streamer | `PendingReport` d9e02a7 2026-10-04 "fix: correlate transmit reports per channel"; `late_reported` 80dd2f6; `sync_streamer` a52863c 2026-10-19 (1b-4d-1a) | `Tx::report`/`reports` (UR-28) | prod VERIFIED; generation moves only on a reopen (2.5) |
| 2.7 | `Tx.plan: Vec<Planned>`, `Tx.version`, `Tx.at`, `Tx.cut`, `plan_section` 330, `retarget` 341, `end_clock` 1034, `cut_k`/"switch_cut" in `step` (tx.rs:411-505) and param `switch_cut` of `send` (512; passed `cut_k` at 442, 503), `Streams.version` | follow a multi-segment transmit clock plan (clock switch at a `cold` change, UR-25) | `switch_cut` 27065e7 2026-09-27 (Phase 7); UR-25 | `Tx::pass` → `plan_section` every turn | the Tx plan has at most one segment (§0). `cut` is Some only when a loss/`Provider::stop` is at or before the clock origin (VERIFIED, `State::finish`). So `end_clock`, `at += 1` and the cut branches run only for a stop/loss at or before the first transmit instant |
| 2.8 | `cancel_unstarted` (tx.rs:346), `CancelReason::ColdChange` at tx.rs:348; `command` (tx.rs:312-327, "Booked on a clock that ended at its origin before this thread took the burst (UR-25)") | cancels a burst booked on a clock that ended | see 4.3 | `command` when `self.clock` is None or its domain differs | reached only after `end_clock` or `halt_failed_reopen` (clock set None) VERIFIED |
| 2.9 | `Held::tile_repeat` (tx.rs:40), `TX_SEND_CAP`, `chunk` in `send` | chunks a repeat for 200 MS/s | 21128b9 (stage TXS) | every repeat | prod; unrelated to Run-time change (listed because it sits in tx.rs beside #70 code) |
| 2.10 | Mock mirror: `DeviceModel` (MR-24, `mock-radio/src/device.rs`), `TxFront`, `front`, `stop_cut` (lib.rs:259, 211-213), `handed_end`/`handed_end_before`/`merge_front` (276-290), `tx_handed_front` (1342), `remember_tx_front` (1374), `HeldBurst.order`/`OpenBurst.order`, `next_order` | Mock models the UHD two-sample safe front and the payload handed within `tx_in_flight_ns` | `git log -S'tx_handed_front'` 7043a29 2026-10-26 "fix: design note 40 — the preemption bound follows the in-flight window on Mock and UHD" (message: "Four majors of one mechanism (the Mock inferring UHD's device-burst framing)") | MR-17, MR-25; `handle_tx_burst` 1538, `stop_tx` 1110 | prod VERIFIED (every `send`/`Stop`) |

---------------------------------------------------------------------------------------------------

## 3. Receive device chain state (`RxChain`, L1 of plan/phase8/44-device-chain-state.md)

`RxChain` = `core.rs:117-228` (`RxDevice{Off,Armed{at},Streaming,Uncertain,Lost}`, `RxIntent{cut,end_now,terminate}`, `SegmentKey`); one `ClockMutex<RxChain>` per stream (`Core.rx_chain`, core.rs:731). Added `git log -S'struct RxChain'` ec3c304 2026-10-31 "stage L1 — receive device chain state with one owner per DDC value, design note 44 §3". The L1 reason (one owner per DDC value: `debt`, `left`, `latest`, `restore`, `Flushing`) is no longer in the struct (stage 2); the device-state part remains.

| Transition | def | Prod callers | Reachable from | Verdict |
|---|---|---|---|---|
| `adopt_start` Off→Armed | core.rs:139 | rx.rs:138 (`Rx::new`, first segment started by `Provider::start` → `rx_start` mod.rs:635) | connect / Run start | VERIFIED |
| `begin_result` Off\|Uncertain→Armed/Lost | core.rs:145 | rx.rs:385 (`Rx::begin`, a segment after a `start_rx`) | Stop then `start_rx`; prior `Uncertain` only after a failed `rx_stop` (non-lost) | VERIFIED |
| `restart_result` Armed\|Streaming→Armed | core.rs:162 | rx.rs:503 (`RxRecv::LateCommand`: device missed the timed start) | late start at T0 or at a `start_rx` origin | VERIFIED (UR-17; not a Run-time change) |
| `first_sample` Armed→Streaming | core.rs:178 | rx.rs:539 | every receive | VERIFIED |
| `stop_result` → Off/Uncertain/Lost | core.rs:196 | rx.rs:287 (`try_end`), rx.rs:405 (clock-declare failure) | Stop, Provider::stop, abort | VERIFIED |
| `request_cut`/`request_end_now`/`request_terminate` and getters | core.rs:130-137,184-195 | rx.rs:233,252-260,468,528,694-697 | Stop, abort, silence, late command | VERIFIED |
| `lose`/`finish`/`abandon_unowned` | core.rs:210,214,221 | rx.rs:136,275,297,311,319; mod.rs:650 | loss, Provider::stop, failed start | VERIFIED; `abandon_unowned` only at mod.rs:650 (UNSURE which operation exactly: a stream's start fails after another began) |
| `RxDevice::Uncertain` | core.rs:204,223 | entered only by `stop_result(Err, !lost)` and `abandon_unowned` | failed `rx_stop` / failed start | fault path only |

Related receive items:
- `Core::assert_rx_chain_reusable` / `assert_rx_chains_reusable` (core.rs:1287-1305): `#[cfg(test)]` L1 invariant, no prod code.
- `Core::owner_configure` (core.rs:1308), `Streams::owner_step` (614), `Owner{Nothing,Configure,Open}` (core.rs:309), `DeviceSet`, `record_open/record_opened/record_refused/record_configured` (632-668), `StreamState.set/configured/generation`; `git log -S'fn owner_step'` a52863c 2026-10-19 "stage 1b-4d-1a — one record per stream of what the device holds" (UR-36). Reachable: `Open` at the first segment and after a failed open; `Configure` at every `start_rx` after Stop (VERIFIED: `configured != segment.by`); `Nothing` when set and mark match. With channel sets fixed for the Run (F4a) a changed-set `Open` cannot occur; `Open` remains for failed opens and `record_refused`.
- `Streams::claim_owner/give_back/begin_read/end_read/begin_shutdown_read`, `Holder{Free,Owner}`, `Claims{holder,reading}`, `Claim` drop-panic (core.rs:309-345, 411-470); `git log -S'enum Holder'` 2af9afd 2026-10-22 "stage 1b-6a — a UHD stream's device holder changes only through Streams' transitions". Callers: `claim_owner` core.rs:1308 (`owner_configure`), tx.rs:~800 (`reopen_streamer`); `begin_read` tx.rs:300,1066; `begin_shutdown_read` tx.rs:1178. `begin_shutdown_read`'s doc (core.rs:438-439) says "a detached uhd-control left in `FromNone`" — no such state exists (stale).
- `Rx.from`, `Rx::begin`'s `find(origin >= self.from)`, `Rx.last_stop`, `Stream.not_before` (rx.rs:~70-110, 336-440): successive segments after a Stop; reachable via Stop + `start_rx`.

---------------------------------------------------------------------------------------------------

## 4. Transmit preemption / interruption / repeat replacement

Search: `grep -rniE 'preempt|interrupt' crates/ezsdr-radio-uhd/src crates/ezsdr-radio/src crates/ezsdr-mock-radio/src` (non-test): `Tx::preempt` and comments only (no `interrupt` definition). `grep -rn 'switch_repeat\|repeat_operation' crates` → no hit (note 47 stage 5 is future).

| # | Item (def) | What | Why added | Uses | Reachable today |
|---|---|---|---|---|---|
| 4.1 | `Tx::preempt` (tx.rs:957-1013), call sites tx.rs:433 (open burst) and tx.rs:456 (held tail), `Tx::dispatch_start` (905), target-ordered `held` map | when a held burst starts at or before the open burst's next un-handed sample, decide it against that cursor (late policy), end the open burst there (continuing the device burst if adjacent) | `git log -S'fn preempt'` 27065e7 2026-09-27 (Phase 7 steps 3-4); `fn dispatch_start` 40dbb0d 2026-10-03; RM-15 "as VE-2 amends it" (tx.rs:955), UR-21 "Preemption" (`18-uhd-radio.md:244`) | `Tx::step` | prod VERIFIED: `Control::book_burst` does not refuse an overlap (UR-21 line 241: "A start at or before the open burst's next sample is not refused here: uhd-tx's preemption ... decides it"); `send`+`send` (second start inside the first) or `start_repeat` then `send` reach it |
| 4.2 | `timeline::later_bound` (357), `late_policy_front_outcome` (362), `decided_at` (352) | decision against max(ordinary bound, handed front) | `git log -S'late_policy_front_outcome'` 7043a29 2026-10-26 "design note 40 — the preemption bound follows the in-flight window on Mock and UHD"; `decided_at` 87bb265 | tx.rs:905-1013, control.rs `book_burst` (~413), mock lib.rs:1596-1605 | prod VERIFIED |
| 4.3 | `CancelReason{Stop,ProviderStop,Loss,ColdChange}` (`ezsdr-radio/src/lib.rs:119-128`), `cancellation_reports` (131: true only for `ColdChange`), tx.rs `report_cancelled_burst` 1015, `stop_now` 1024, mock `record_cancelled_burst` 1101 | `COMMAND_REJECTED` for a cancelled held burst only when the cause is `ColdChange` | `git log -S'enum CancelReason'` 5ffbf2d 2026-10-23 "stage 1b-7a — a held burst's cancellation is reported as one vocabulary rule decides"; the `ColdChange` string first 5eb61dd 2026-09-26 (Phase 3) | `ColdChange` constructed only at tx.rs:348 (`cancel_unstarted`), 828, 844 (failed-reopen halt); none in Mock (`grep -n CancelReason crates/ezsdr-mock-radio/src/lib.rs`: Stop 1483, Loss 1849, ProviderStop 2246). Stop/ProviderStop/Loss never report | the name is stale (no cold change cancels anything); the report is reached only via the clock-ended / failed-reopen paths (2.5, 2.8) VERIFIED |
| 4.4 | Mock stop-through-window: `stop_tx` handover branch (mock lib.rs:1110-1155: `handover`, `horizon`, `stop_cut`, `remember_tx_front`), MR-25 | Stop plays out payload already handed within `tx_in_flight_ns` | 7043a29 (note 40) | `Action::Stop` (1483), `stop_at` (1156) | prod VERIFIED |
| 4.5 | Mock `handle_tx_burst` refusal "burst begins at or before the open burst's next sample" (lib.rs:~1635), `emit_tx_until(.., extra_cut)` (1670), held bursts as cuts of the open burst (`schedule_wakeup` 777-840) | Mock's RM-15 "a burst ends at the start of the next burst" | MR-15/MR-16 (`09-mock-radio.md:135,138`) | MR-15 | prod VERIFIED |
| 4.6 | `TxCmd::Stop`/`Tx::stop_now` (tx.rs:323,1024); `Control::book` Stop booking (control.rs:280 books `Kind::Stop` on a Tx line AND sends `TxCmd::Stop`) | transmit Stop ends bursts, not the clock | RM-16; UR-26 | Stop | prod VERIFIED. The Tx `Kind::Stop` item has no planner effect (§0) but `Streams::book` → `changed` bumps `Streams.version` (core.rs:669-683), so every `Tx` re-reads the plan |
| 4.7 | `Tx.late_reported`, `report_send_asap`, `time_error` (tx.rs:884-897) | one TIME_ERROR per moved burst across hand-overs | 7043a29 ("a moved burst reports TIME_ERROR once") | `dispatch_start`, `preempt` | prod VERIFIED |

---------------------------------------------------------------------------------------------------

## 5. Queue-depth accounting (UR-24), in-flight window, restart lead for cold changes, `Line::halted`

| # | Item | Facts |
|---|---|---|
| 5.1 | `radio.timing.command_queue_depth` (`TimingEnvelope.command_queue_depth` lib.rs:543; key lib.rs:55,179; x3xx.rs:384 = 16; ideal channels.rs:359 = u32::MAX; description channels.rs:470; measurement.rs:367) | Declared, validated (UpperBound, `07-radio-model.md:338`), recorded in the Manifest and `schemas/radio/envelope.v1.json`. Consumed by NO runtime queue accounting: `grep -rn 'command_queue_depth' crates/*/src` → definitions, description, measurement and the candidate test only (lib.rs:543; provider/mod.rs:908,926; x3xx.rs:384; channels.rs:359,470; measurement.rs:367). Origin `git log -S'command_queue_depth'` 1dfd39f 2026-09-21 init. Measurement harness: `tests/common/measurement.rs:1420-1470` |
| 5.2 | `radio.COMMAND_QUEUE_FULL` + `CommandQueueFullPayload` (lib.rs:110,215,802,843; schema file; spec rows `07-radio-model.md:119,123,133`) | Defined, `Reaction::Abort`; spec 07:119 "reserved: no key has the update class `hardware_timed` (RM-21), so no Provider emits it". `grep -rn 'COMMAND_QUEUE_FULL' crates/*/src` → lib.rs:110,215 only (never emitted). Test refs: radio/tests/radio_model.rs:173, `tests/common/measurement.rs:4102`. `git log -S'COMMAND_QUEUE_FULL'` 6bf67fb 2026-09-25 Phase 2. Never emitted (VERIFIED) |
| 5.3 | `tx_in_flight_ns` (`TimingEnvelope.tx_in_flight_ns` lib.rs:554; x3xx.rs:389 = 10 ms; ideal 0) | Prod consumers: `Tx::step` window_cut (tx.rs:418), `Tx::send` chunk (516), Mock `stop_tx` 1118, `tx_handed_front` 1346/1365. `git log -S'tx_in_flight_ns'` 7043a29 2026-10-26 (note 40). Purpose in source: bounds how far ahead samples are handed to the device so preemption/Stop decide against an un-handed cursor (tx.rs file header; commit message); also paces chunking (`chunk = window/n/4`) and bounds Stop latency (MR-25). Reached by every send/Stop (prod VERIFIED) |
| 5.4 | restart lead for cold changes | no such constant left. `grep -rniE '\bwarm\b' crates/*/src` → none; `cold` → only radio lib.rs:126,187-193 and control/differential.rs:214 (test) |
| 5.5 | `update_parameter` labels | `command_rejected(Some(id), "update_parameter", ..)`: control.rs:251,257; mock lib.rs:1766-1778; prefixes "UR-26: RM-21:" / "MR-18: RM-21:" — the stage-3 staging refusals, live |
| 5.6 | `Line::halted` | see 1.9: test-only (radio/tests/timeline.rs:469,513,540,677); `State.halted` has no other reader |
| 5.7 | `Stream.release_window`, `RELEASE_WINDOW_NS`, RM-6 | see 1.4-1.5 |
| 5.8 | `FakeConfig::apply_delay` (device.rs:374, wall time of an untimed `apply`) | used by untimed `apply` (prod via `Core::configure`) — live |

---------------------------------------------------------------------------------------------------

## 6. Other code citing #70/#71/#72, notes 39/44, "Run-time change"; stale comments

Searches: `grep -rnE '#7[012]\b' crates/ezsdr-radio-uhd/src crates/ezsdr-radio/src crates/ezsdr-mock-radio/src design/07* design/09* design/18*` → NO hit (issues are cited only in plan/ docs: `plan/phase8/46-streaming-feature-ledger.md:57,65,83,84,161`, `handoff.md:15`, `plan/maintenance/failure-mechanisms.md:20`). `grep -rnE 'note 39|note 44|design note 39|design note 44'` → `uhd.rs:523`, `provider/tx.rs:223`, `design/07-radio-model.md:470`, `design/09-mock-radio.md:427`, `design/18-uhd-radio.md:682,683,714,715,716` (Changes rows). `grep -rniE 'run-time change'` in src → `ezsdr-mock-radio/src/channel.rs:155`; design 07:204, 07:408, 09:200.

Issues (gh, open): #70 "[P2] A one-sample device burst on X3x0 makes the next TX burst lose its time"; #71 "[P3] Many timed transmit retunes with no burst running can apply at the wrong times (DUC FIFO full)"; #72 "[P2] A timed TX frequency change inside a running burst is not applied on X3x0 (untimed continuation sends)". Mechanism label `mech:device-state-outlives-call` ("#70, #72", `plan/maintenance/failure-mechanisms.md:20`). Handoff: #72 "closes with note 47" (`handoff.md:15`); #70 "no symptom on the Module path, recheck after #78".

Stale comments / leftover scaffolding in `src` (text refers to something that no longer exists):
- `provider/control.rs:352,354,356`: empty banners `// ---- UR-24`, `// ---- UR-25`, `// ---- UR-21` (the first two hold no code).
- `provider/core.rs:438-439`: "a detached uhd-control left in `FromNone`".
- RM-34 cited (channel move, withdrawn): `core.rs:252`; `mock-radio/src/lib.rs:61,83,321,326,1889,2057,2202`; `ezsdr-radio/src/timeline.rs:15,46,51,76`.
- `ezsdr-radio/src/timeline.rs:46,51`: `Channels::with` (prod caller channels.rs:513) and `Channels::without` (no non-test caller; test rx.rs:1707 only).
- `ezsdr-radio/src/lib.rs:126`: `CancelReason::ColdChange` doc "A `cold` change cancelled the burst on its old transmit clock"; `lib.rs:545-546` documents release window as "How far ahead of its instant a timed command is handed to the device".
- `provider/tx.rs:493` comment "The pending switch will cancel this old-clock reservation"; param name `switch_cut` (tx.rs:512).
- `mock-radio/src/lib.rs:1789-1796`: comment about "A timed update received at `now` takes effect no earlier than `now + min_timed_command_lead_ns` (MR-18's `earliest`)"; header comment of `advance` lists "held bursts and updates" (candidates at 1781-1815 are Fault, RxBegin, Tune, HeldBurst only).
- `mock-radio/src/channel.rs:155`: "stays although no Run-time change reaches it now".
- `device.rs:361-362`: `FakeFault::RxApplyFails` doc "uhd-control's configuration of an enable from no stream".
- Mock multi-clock transmit: `TxStream.clocks: Vec<TxClock>`, `cur`, `tx_gen`, `TxClock.by/ended`, `tx_reconcile` (mock lib.rs:225-232, 1327-1340, 1381-1412), Tx `record_layout_segment` (1391, 2131): with Tx lines holding only `Kind::End` the plan has <= 1 segment (§0), so `clocks.len() <= 1`. `git log -S'fn tx_reconcile'` 87bb265 2026-10-08. VERIFIED (`book_tx` has one caller, lib.rs:1177).
- UHD multi-clock transmit: `StreamState.clocks: Vec<Clock>` ("none or one"), `Streams::changed` ending the clock at the plan cut (core.rs:669-683): same fact.

Other prod paths still on, listed for completeness:
- `Control::ceil_root` (control.rs:337), `timeline::command_instant` (timeline.rs:279, control.rs:~321, mock 1511), `timeline::lattice` (393, used by `LateCommand` restart rx.rs:~492), `Staging` (`ezsdr-radio/src/staging.rs`, new in stage 3): all live.

---------------------------------------------------------------------------------------------------

## 7. Spec rule IDs that describe these mechanisms (file:line of the rule's first line)

| Rule | file:line | One line |
|---|---|---|
| UR-2 | `18-uhd-radio.md:71` (trait text 72-104) | `Device` trait: `apply(.., at: Option<i64>)`, `rx_stop(.., at: Option<i64>)`, `tx_send(.., at: Option<i64>, ..)`; `Applied` "read back after an untimed change" |
| UR-13 | `18-uhd-radio.md:212` | `arm` opens streamers; T3 flush "not an experiment buffer" |
| UR-17 | `18-uhd-radio.md:223` | uhd-rx plan, cut, untimed stop |
| UR-21 | `18-uhd-radio.md:241` (Preemption 243-244) | TxBurst refusals; "A start at or before the open burst's next sample is not refused here"; first-dispatch recheck |
| UR-22 | `18-uhd-radio.md:247` (248-250) | every payload send timed; T3 flush untimed; closing EOB >= 2 samples; safe start; recovery |
| UR-23 | `18-uhd-radio.md:251` (252-262) | uhd-tx plan, in-flight window, device bursts |
| UR-24 | `18-uhd-radio.md:267` | WITHDRAWN stub (timed release of `hardware_timed` updates) |
| UR-25 | `18-uhd-radio.md:269` | one timeline line per stream, booked by control/rx/loss/stop |
| UR-26 | `18-uhd-radio.md:291` | staging of rx values; refusal; `start_rx` snapshot |
| UR-28 | `18-uhd-radio.md:303` | async report correlation; `tx_flush: true` ACK does not retire |
| UR-33 | `18-uhd-radio.md:328` (329-346) | FakeDevice: timed command queue, DDC 32 / DUC FIFO, DUC header, one-sample EOB loss, faults `TxBlocks`, `RxApplyFails` |
| UR-36 | `18-uhd-radio.md:280` | per-stream record of what the device holds (set, generation, last configured command) |
| UR-37 | `18-uhd-radio.md:288` | owner makes no device call for a refused start |
| U13 row | `18-uhd-radio.md:404` | "Timed commands — Withdrawn (note 47 stage 2)" decision row |
| Changes rows | `18-uhd-radio.md:682,683,714,715,716` | D9b-1a, D9b-2, L1 history |
| RM-6 | `07-radio-model.md:89` | TimingEnvelope (queue depth, release window, in-flight); `rx_start_lead >= release_window` |
| RM-10/11 | `07-radio-model.md:108,119,123,125,133` | event kinds; `COMMAND_QUEUE_FULL` "reserved ... no Provider emits it"; its payload row |
| RM-12 | `07-radio-model.md:147` | verbs; `start_rx` |
| RM-14 | `07-radio-model.md:162` | late policy, device lead, decision instant |
| RM-15 | `07-radio-model.md:166` (168) | held bursts in target order; a burst ends at the start of the next; handed-front paragraph |
| RM-16 | `07-radio-model.md:170` (174-175) | Stop / cut / handed payload within the window |
| RM-20 | `07-radio-model.md:208` | envelope document (eight timing members) |
| RM-21 | `07-radio-model.md:237` (238-241) | `cold` receive keys, staging, streaming |
| RM-25 | `07-radio-model.md:257` (258-267) | lattice, start rule incl. "After a cut, readiness includes the release window", halt, prune |
| RM-27 | `07-radio-model.md:274-289` (class column 278-280) | key table; classes |
| RM-34 | `07-radio-model.md:330` | WITHDRAWN stub (channel move) |
| RM-36 | `07-radio-model.md:349` | WITHDRAWN stub (`covering`) |
| table rows | `07-radio-model.md:66,72,73,337,338` | `hardware_time`, `command_queue_depth`, `release_window_ns`, UpperBound/LowerBound lists |
| MR-15 | `09-mock-radio.md:135` | tx blocks; block ends at the next burst's start |
| MR-16 | `09-mock-radio.md:138` | TxBurst refusals incl. "at or before the open burst's next sample" |
| MR-17 | `09-mock-radio.md:145` (146) | decision vs conservative in-flight front, two-sample margin |
| MR-18 | `09-mock-radio.md:150` | staging of rx values |
| MR-24 | `09-mock-radio.md:170` | DeviceModel framing check |
| MR-25 | `09-mock-radio.md:172` (174) | Stop payload cut within the window; retained front |
| MR-33 | `09-mock-radio.md:200` | gain/frequency timelines; "no transmit key changes during a Run" |
| MR-34 | `09-mock-radio.md:203-207` | LO phase; untimed tune at each `start_rx` origin; "No channel takes a timed tune" |
| MR-3 rows | `09-mock-radio.md:53,74` | `release_window_ns` 3 ms; `command_queue_depth` 4294967295 |
| MR-40 | `09-mock-radio.md:219` | WITHDRAWN stub (channel move) |

---------------------------------------------------------------------------------------------------

## 8. Test-only consumers (to size a deletion)

- `crates/ezsdr-radio-uhd/src/provider/rx/differential.rs` (998 lines) and `provider/control/differential.rs` (275): `#[cfg(test)] mod differential` (rx.rs:709, control.rs:507); build `timeline::Stream { release_window, .. }` and a fake with timed apply (`rx/differential.rs:96,657`).
- `crates/ezsdr-radio-uhd/tests/common/probe_device.rs` (624: `fail_timed_apply`, `before_timed_apply`, `fail_restoring_apply`, `record_timed_apply`), `tests/common/measurement.rs` (5595; timed-apply queue trials 1229-1331, `measure_command_queue_depth` 1436), `tests/hardware.rs` (queue / DSP probes ~560-620, 829-840, 1011-1070, 3147), `tests/common/d9.rs`, `tests/fake.rs` (8003).
- `ezsdr-radio-uhd/src/device.rs` tests 2119-3420 (~1300 lines) are mostly FakeDevice timed-apply / DSP / DUC unit tests (`device.apply(.., Some(..))` at 2342-3177).
- `crates/ezsdr-radio/tests/timeline.rs`, `ezsdr-mock-radio/tests/timeline.rs`: planner cases incl. `Line::halted`, `prune`, `refuse`, `made`.

## 9. Commands run (for rerun)

- `grep -rn --include='*.rs' -E 'apply\(' crates/ezsdr-radio-uhd/src | grep -vE 'fn apply|inner.apply'`
- `grep -rn --include='*.rs' 'tx_send(' crates/ezsdr-radio-uhd/src`
- `grep -rn --include='*.rs' -E 'rx_stop\(|rx_start_now|\.rx_start\(' crates/ezsdr-radio-uhd/src`
- `grep -rn --include='*.rs' -E 'lock_tx_dsp|tx_dsp_gates' crates/ezsdr-radio-uhd/src`
- `grep -rn --include='*.rs' -E '\.halted\(|\.made\(|\.prune\(|prune_stream|lead_left\(|check_start_lead' crates`
- `grep -rn --include='*.rs' 'CancelReason::' crates` ; `grep -rn --include='*.rs' 'cancellation_reports' crates`
- `grep -n 'Recovery::' crates/ezsdr-radio-uhd/src/provider/tx.rs`
- `grep -rn 'COMMAND_QUEUE_FULL\|command_queue_depth' crates/*/src`
- `grep -rnE '#7[012]\b' crates/ezsdr-radio-uhd/src crates/ezsdr-radio/src crates/ezsdr-mock-radio/src design/07* design/09* design/18*`
- `git log -S'<identifier>' --format='%h %ad %s' --date=short -- crates design | tail -1` for each "why added".
