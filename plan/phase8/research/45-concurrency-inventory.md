# 設計メモ 45 の材料：UHD Module の並行処理の棚卸し（Sol 6.1，2026-10-31，main d52f127）

設計メモ [45](../45-uhd-decision-core.md) の §2 の元．Sol 6.1 が読むだけで調べた事実（path:line と UHD 4.10 の出典つき）．orchestrator は数字の一部を確かめた．

INVENTORY: At `d52f127`, the UHD Module combines per-stream owner threads, shared timeline and device-chain state, message channels, and several layers of device-call serialization.

Paths below are relative to `crates/ezsdr-radio-uhd/src/`, unless prefixed with `crates/` or `plan/`. Historical closure statements describe the review records, not newly executed verification.

## 1. Threads

| Thread | Loop, decisions and device calls | Ending |
|---|---|---|
| `uhd-rx-s{k}`, one per declared RX stream | Polls shutdown messages and the shared plan; begins/configures segments, receives samples, decides cuts, drains pending DDC writes, stops and restores frequency. Idle sleep: 1 ms. Normal `rx_recv` requests use a 100 ms timeout, reduced near cuts/deadlines. Other calls include open/close, apply/read-back, timed/immediate start and untimed stop. (`provider/mod.rs:678–710`; `provider/rx.rs:160–244,265–451,481–595`) | Shutdown/disconnection requests termination; loss finalizes the current segment and leaves the owner idle until shutdown. (`provider/rx.rs:160–210,247–259`) |
| `uhd-tx-s{k}`, one per declared TX stream | Requests real-time priority, drains burst/Stop/shutdown messages, refreshes its plan, polls reports, and sends one unit of work. Idle sleep: 200 μs. Payload/EOB sends have a 1 s timeout; ordinary async reads use zero timeout. Reconfiguration/recovery can open, close, apply and perform a flush. (`provider/mod.rs:713–719`; `provider/tx.rs:211–297,543–623,761–843`) | Shutdown/disconnection cancels bursts, closes an open burst and reads final ACKs for 100 ms of device-host-clock time, using 10 ms reads. Loss suppresses ordinary work; shutdown remains necessary. (`provider/tx.rs:251–265,1255–1272`) |
| `uhd-control` | Books each Action before receiving the next, releases timed commands, polls every 1 ms and checks reference/time every 500 ms. It can apply settings, open/configure an idle stream, and run TX initialization flushes. Control calls have no caller-supplied timeout; UHD may block internally. (`provider/mod.rs:723–724`; `provider/control.rs:39–40,363–382,826–902,1145–1191,1375–1402`) | `streams.stop` triggers settlement and plan recording. Loss prevents further bookings/releases/checks but does not itself exit this loop. (`provider/control.rs:363–388,692–695`) |
| `uhd-clock`, Authority-owned | Reanchors device time every 100 ms through `Device::time_now`; discards excessively wide reads. Waits through `HostClock`. (`authority.rs:101–139,452–494`) | Stop flag plus notification, or failed `Weak` upgrade. Drop waits 1 s, then detaches a blocked thread. (`authority.rs:465–493,530–548`) |

Provider threads run through `catch_unwind`; an owner panic publishes device loss, whereas a control-thread panic only records rejection. Each join has a **separate 1 s bound**. Stop books the end first, joins control, shuts down/joins all TX owners, then RX owners. Any detached Provider thread causes cleanup to retain all streamers. (`provider/mod.rs:398–440,736–791`)

`HostClock` itself starts no autonomous scheduler thread: `spawn` registers participants and launches the supplied body. (`host.rs:391–403`)

## 2. Shared state and locks

**Every `Core` field** is accounted for below. Its declaration is `provider/core.rs:1032–1076`.

| Fields | Synchronization and users |
|---|---|
| `device` | Immutable `Arc<dyn Device>` shared by coordinator, control, RX/TX owners and Authority. The trait is `Send + Sync`. (`device.rs:153–156`) |
| `device_gate` | Standard `Mutex<()>`; open/close/rate-read admission and any thread publishing loss. (`provider/core.rs:1175–1184,1252–1271,1314–1323,1560–1607`) |
| `rx_chain` | Per-stream `ClockMutex<RxChain>`; control hands frequencies/plans restoration; RX owns start/restart/consume/stop/finish transitions. Contains device phase, segment identity, cut/termination intent, debt, leftovers, latest values and restoration state. (`provider/core.rs:129–150`; `provider/control.rs:390–418,850–899`; `provider/rx.rs:265–451`) |
| `tx_dsp_gates` | Per-device-channel `ClockMutex<()>`; control timed TX applies versus opener/recovery flush. Multiple gates are sorted and deduplicated before acquisition. (`provider/core.rs:1389–1394,1512–1528`; `provider/control.rs:831–832`) |
| `tx_report_gates` | Per-stream `ClockMutex<()>`; serializes initialization/recovery and every ordinary/shutdown async reader. (`provider/core.rs:1401–1402,1514–1528`; `provider/tx.rs:1160–1168`) |
| `tx_report_stash` | Per-stream standard mutex over report deque; flush preserves unrelated/late reports, owner consumes them, successful open clears them. (`provider/core.rs:1405–1410,1462–1480,1572–1574`) |
| `rec` | Standard mutex over `Records`: applied rows, bursts, async reports, rejection/timing rows and stats. All workers write; cleanup copies the sections. (`provider/core.rs:993–1028,1210–1219`; `provider/mod.rs:443–454`) |
| `streams` | Standard mutex over instance timelines, sequence/end/stop/loss/version and per-stream state. Control books commands; owners publish progress/refusal/completion/device records; stop/loss publishers book ends. (`provider/core.rs:342–368,499–525,743–799`) |
| `held` | Standard mutex over `(ClockDomainId,start)` reservations; control inserts, TX removes. (`provider/control.rs:1350–1369`; `provider/tx.rs:378–380`) |
| `rates` | Standard mutex over cached per-direction rate-check outcomes; coordinator/control/owners access through shared open/check helpers. Device read occurs outside this mutex. (`provider/core.rs:1296–1332`) |
| `start_up` | `AtomicI64`; arm writes with Release, start and burst booking read with Acquire. (`provider/mod.rs:587,645`; `provider/control.rs:1350`) |
| `lost` | `AtomicBool`; loss publication writes Release after booking the end; workers read Acquire. (`provider/core.rs:1206–1208,1258–1271`) |
| `id`, test-only `rx_id`/`tx_id`, `mcr`, `root`, `description`, `block_len` | Immutable construction-time values. (`provider/core.rs:1044–1062,1118–1145`) |
| `time`, `clocks`, `events`, `overflow`, `inputs`, `links` | Immutable shared handles/collections; interior operations go through their respective contracts. RX publishes through links and pre-resolved overflow handles. (`provider/core.rs:1052–1059`; `provider/rx.rs:735–815`) |
| test-only `probes` | Standard mutex over callbacks; lookup clones the callback and releases the registry lock before invoking it. (`provider/core.rs:1161–1172`) |

Other cross-thread state:

- `ColdConfig`: immutable base, `ClockMutex` updates and SeqCst `configured` atomic, shared through `Arc` between control and owners. (`provider/control.rs:94–184`)
- `UhdRadio`’s prepared/running/stopped/cleaned/detached fields are coordinator-owned lifecycle state, not worker-shared mutable fields; `Running` retains channel senders and join handles. Process-wide serial registration uses one standard mutex. (`provider/mod.rs:99–146,183–205`)
- `DeviceTime`: anchor/schedule mutexes, monotonic `last` atomic and read-counter atomics; Authority stop uses `Arc<Mutex<bool>>`, notification and a first-anchor channel. Callbacks execute outside the schedule mutex. (`authority.rs:28–30,84–98,189–225,452–505`)
- Native boundary: control mutex, mutex-protected RX/TX maps, `Arc` streamer lifetimes, per-stream call mutexes, TX pointer-storage mutex and loss atomic. FakeDevice instead protects its model with one standard mutex and releases it around clock waits. (`uhd.rs:319–336,458–474,1033–1045,1113–1150`; `device.rs:813–825,1786–1843,2072–2148`)
- `HostClock`: standard mutex/condvar signal; manual-state mutex containing busy participants, deadlines, waits and FIFO clock-lock handoffs; atomic lock-ID allocation. (`host.rs:65–114,435–480`)

**Lock order.** Documented instance order is `updates → streams → rec`, with `streams` never crossing device calls. Actual additional nesting includes `updates → TX-DSP gate` or `RX-chain → streams`; TX open/recovery takes DSP gates → report gate → device gate/native locks. Loss takes device gate → streams. Restoration scans other chains individually before reacquiring its own. The test checker detects taking updates/device-loss entry while holding instance locks, but explicitly does **not** detect `rec → streams`. (`provider/core.rs:28–68,342–348,1252–1260,1514–1536`; `provider/control.rs:831–856`; `provider/rx.rs:421–439`)

**Locks crossing calls/waits:**

- `ColdConfig.updates` crosses all configure applies; control holds projection locks across timed applies. (`provider/control.rs:161–184,831–901`)
- RX-chain crosses start/restart/stop, flush receive waits and restoration apply; normal receive releases it before blocking. (`provider/rx.rs:271–350,429–439,513–540,642–647,234–242`)
- TX DSP/report gates cross open/flush sends, async waits and retry sleeps, bounded by the flush’s 100 ms deadline. (`provider/core.rs:1431–1518`)
- Device gate crosses open/close/rates; native control crosses control calls, and RX stream-command acquisition waits for the streamer call mutex while holding control. Stream call locks cross RX receive, TX send and async waits; TX pointer storage also crosses send. (`provider/core.rs:1314–1323,1560–1607`; `uhd.rs:601–616,903–990,1033–1045,1124–1150`)
- Host condvar waits release their state mutex; logical clock-lock ownership persists until guard drop. (`host.rs:201–219,435–480,586–591`)

## 3. Cross-thread interactions

- **Actions → control → TX:** control receives admitted Actions, reserves burst identity and sends `Held` through per-stream unbounded `mpsc`; TX owns waveform ordering, tiling, dispatch and reservation release. (`provider/mod.rs:670–676`; `provider/control.rs:369–374,1353–1370`; `provider/tx.rs:300–309,378–380`)
- **Control → RX timed handoff:** projection locks plus chain lock ensure a covering segment is Armed/Streaming; accepted timed frequency creates chain-owned debt. An unbegun covering segment returns `Blocked`, retaining the head and later commands. (`provider/control.rs:737–740,826–899`; `provider/core.rs:152–156,211–230`)
- **Any failure observer → all workers:** device gate serializes loss against open/close admission; shared end is booked before `lost` becomes visible, then native loss is marked. Ordinary calls retain check-to-call windows; RX’s narrowed window is explicitly recorded. (`provider/core.rs:1249–1278`; `plan/maintenance/failure-mechanisms.md:62`)
- **Coordinator → workers:** stop/end publication is one streams section, followed by joins and shutdown messages. (`provider/mod.rs:746–770`)
- **Control ↔ owner claim:** `Holder::{Free,Owner,Release,FromNone}` excludes conflicting ownership; from-none waits for `reading` to clear. Shutdown reads intentionally bypass holder checks. (`provider/core.rs:414–455,577–656`; `provider/control.rs:959–980`)
- **Streamer replacement → report correlation:** shared generation changes on successful open; TX clears pending correlations and TimeError deduplication when it observes a new generation. (`provider/core.rs:893–924`; `provider/tx.rs:1134–1144`)
- **Segment/channel moves:** both timelines change together; owners close/release old resources and wait for recorded ownership, unfinished predecessor segments and same-move source configuration. The plan is reread after waits. (`provider/control.rs:1030–1058`; `provider/core.rs:855–885,1625–1740`)
- **Stop versus timed rows:** release checks end publication while holding streams before recording rows; finish settles held/given commands against original `S` and handover time. (`provider/control.rs:743–766,905–947`)

## 4. UHD 4.10 thread-safety facts

- **Explicit guarantee:** `send`, `recv` and `recv_async_msg` may execute on different threads. Concurrent `send` calls on one streamer, or concurrent `recv` calls on one streamer, require application serialization. This requires serialized access, **not permanent thread affinity**. [UHD general.dox:222–246](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/general.dox#L222-L246)
- **Slow path:** UHD documents simultaneous settings changes as safe but potentially leaving settings uncertain, recommending at most one settings-manipulation thread context. This is not a blanket atomicity guarantee for `multi_usrp` operation sequences. [general.dox:241–246](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/general.dox#L241-L246)
- **Command-time bracket is explicitly non-atomic.** Set-time, settings changes and clear-time are separate operations; full command queues block UHD. [timed_cmd.dox:71–115](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/timed_cmd.dox#L71-L115)
- **Implementation is selectively locked:** RFNoC `multi_usrp` uses `_graph_mutex` for stream creation and rate/frequency setters, but `set_command_time` has no encompassing graph lock. [multi_usrp_rfnoc.cpp:327–333](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L327-L333), [964–987](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L964-L987), [1314–1367](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1314-L1367)
- **Receive versus settings:** ordinary concurrent streaming/control is consistent with the general documentation; **INFERRED, not verified as an unrestricted guarantee**, especially for structural/rate changes. Streamer rate setters directly modify receive-path state. [rx_streamer_impl.hpp:237–246](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/rx_streamer_impl.hpp#L237-L246)
- **Receive versus `issue_stream_cmd`:** RFNoC stores stop/overrun flags atomically and posts command actions, supporting a separate command path; I found no explicit unrestricted concurrency promise for this pair in the inspected API documentation. This repository serializes them using the same RX call mutex. [rfnoc_rx_streamer.cpp:131–149](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_rx_streamer.cpp#L131-L149), [header:139–142](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/rfnoc/rfnoc_rx_streamer.hpp#L139-L142); `uhd.rs:601–616,1033–1037`.
- Despite UHD permitting split send/report threads, this implementation uses the **same TX call mutex** for both. (`uhd.rs:1124,1145`)

## 5. Real-time paths and budgets

**RX, per receive result:** native buffer allocation/zeroing and pointer-vector construction; metadata extraction; chain debt consumption; lattice/overlap/cut decisions; delivered-floor publication; gap/event construction; sample copy into HostPool; immutable block construction, link publication and mutex-protected statistics. Debt consumption currently walks individual samples and searches pending entries. (`uhd.rs:1033–1067`; `provider/core.rs:266–285`; `provider/rx.rs:681–815`)

**TX, per send:** plan/report pass, time/window/cut decisions, waveform slicing, fixed-capacity channel pointers/tail/padding, native metadata allocation/free and conversion/send, accepted-count handling, BurstTracker and stats. Short repeats are tiled on message receipt, not every send. (`provider/tx.rs:43–55,276–297,432–714`; `uhd.rs:1113–1140`)

Budgets:

- Profile: RX/TX start leads 50 ms, timed release window 3 ms, TX in-flight window 10 ms; device lead 2 ms and delivery allowance 3 ms. (`crates/ezsdr-radio/src/x3xx.rs:397–402`; `profile.rs:9–23`)
- TX chunk is `window_samples/4`, capped at 20,000; at 200 MS/s that cap represents **100 μs of samples**. The owner waits 200 μs when no work can proceed. Held final samples close when only device lead remains. (`provider/tx.rs:211–214,495–501,543–581`)
- RX’s block production interval is `block_len/rate`; default 2,000 samples gives **10 μs at 200 MS/s**. These calculated intervals are not measured scheduling guarantees. (`profile.rs:14–15`; `provider/rx.rs:563–578`)
- RX drain bound is latest pending instant plus a call, versus cut plus a block, then delivery allowance. Deadline reads use `one_packet`; UHD otherwise reapplies timeout per packet rather than bounding the whole call. (`provider/rx.rs:835–857`; [UHD implementation:139–182](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/rx_streamer_impl.hpp#L139-L182))
- Hardware validation of the 10 ms TX window remains INFERRED; wake-delay and held-tail bounds remain bench items. (`handoff.md:20,33–35`)

**INFERRED delay insertion points:** a hop before receive postpones draining buffers; a hop before TX send consumes queued headroom; a hop before timed apply consumes release lead. Existing chain/projection/native-control contention already lies at these boundaries. (`provider/rx.rs:227–243`; `provider/tx.rs:444–446,543–623`; `provider/control.rs:700–722,831–884`)

## 6. Production test hooks

Named probes are inert outside test builds; callbacks may gate execution. (`provider/core.rs:1161–1172`)

- **Provider:** `fail_next_rx_spawn`; `start_stream` pauses between initial starts; cleanup chain-reusability assertion. (`provider/mod.rs:195–196,399–403,708–710,788–789`)
- **Core locking:** thread-local lock-depth/address tracking, `assert_line_free`, claim-drop/give-back/read-pair assertions. (`provider/core.rs:28–68,448–455,631–653`)
- **Core probes:** `device_gate_waiting`—loss/open admission contention; `rates`, `rates_before_gate`, `rates_returned`—rate-read windows; `apply`—between-channel loss; `open`, `open_returned`—device-call/record publication; `release_claimed`—loss after release claim; `wait_release`—resource wait; `configured`—configuration return before record. (`provider/core.rs:1181,1300–1325,1342,1561–1576,1640,1679,1730`)
- **Control:** `mark_publishing` plus `updates_held/updates_waiters` exposes projection/configured publication; `before_timed_apply` exposes selection-to-call; `wait_reading`, `reading_wait`, `reading_done` expose report-read ordering; `cold_booking` exposes claim-through-booking. (`provider/control.rs:125–180,835,964–979,1224`)
- **RX:** `rx_after_early_loss_check`, `rx_before_finish`, `rx_restore_before_chain_scan`, `refused_booking`; cursor accessor observes an owner held before begin. (`provider/rx.rs:212,332,424,469–473,555`)
- **TX:** `tx_reopen_claimed`, `refused_booking`; recovery accessor exposes transition state. (`provider/tx.rs:135–136,805,1120`)
- **Core test helpers:** DSP waiter count, direct flush entry, RX reusable assertions, stream-zero clock registration and `assert_tx_open_flush_barrier`. (`provider/core.rs:1396–1399,1421–1428,1610–1623,1743–1792`)
- **FakeDevice injections:** `slow_next_read`; `eob_error`; `gate_next_payload_eob_send`/`tx_send_gate` holds payload SOB/EOB before error evaluation. `set_time` and `read_back` expose controlled device state. (`device.rs:818–825,895–902,1042–1061,1420–1425,1923–1933`)

`test_support.rs`, differential modules and generator import are test-only; ordinary `FakeFault` and `with_rx_stall` facilities are not `cfg(test)` hooks. (`provider/mod.rs:9–14,361–366`; `device.rs:1938–1948`)

## 7. Race/interleaving history

| Finding(s); threads | Mechanism and recorded disposition |
|---|---|
| Phase 7 K N-P1-2; control/TX | Moving preemption frontier overtaken by send. Decision moved to owner-side dispatch; current path rechecks late policy. (`plan/phase7/reviews/review-k.md:40–43`; `provider/tx.rs:508–516`) |
| Phase 7 L P1-2; control/RX/TX | Shared error-string overwrite. Closed for handle calls with per-handle errors under corresponding locks. (`plan/phase7/reviews/review-l.md:40–43`; `review-m.md:65`) |
| Phase 7 Q D-1; clock/process exit | Last device release versus UHD static registry teardown. Weak ownership plus bounded join closed normal teardown; detach/process-exit residual explicitly retained. (`plan/phase7/reviews/review-r.md:11–15,40–43`) |
| Spec 22 refusal round trip; control/RX | Duplicated planned state and partial correlation: five findings; one shared timeline replaced round-trip copies. (`plan/maintenance/failure-mechanisms.md:22`) |
| 1b-4c O1 and three re-review majors; control/owners | Booking versus prune, and stale owner device caches. Atomic booking section restored; caches replaced by shared device record and claims in 4d. (`plan/phase8/reviews/1b4c-opus.md:7–30,82–105`) |
| 1b-4d-1a, three correlation findings; control/TX | Control replaced streamer while owner retained old report correlations. Shared generation plus centralized synchronization closed them. (`plan/maintenance/failure-mechanisms.md:34`) |
| 1b-4d-2; stop/RX and startup/owners | Far-future start outlives join: **#67 open**. Partial startup loss publication fixed before stopping already-started owners. (`plan/maintenance/failure-mechanisms.md:40`) |
| 1b-4e-2 self-review; control/RX/TX | Open/close versus loss; recursive streams-lock acquisition while reporting loss. Device gate added; reporting moved after releasing streams. Closed. (`plan/maintenance/failure-mechanisms.md:42`; `plan/phase8/reviews/1b4e2-sol.md:88–90`) |
| 1b-4e-2 major 2; control/move owners | From-none overtook unopened predecessor and blocked its progress. Shared `needed_before` admission predicate; gated close/open-order regression. Closed. (`plan/phase8/reviews/1b4e2-sol.md:33–50,119–123`) |
| 38a, four majors; end publisher/control | Release fold and end settlement chose competing final rows. Unified handover-at-`S` rule and locked end check. Closed; physical acceptance time remains unobservable. (`plan/phase8/reviews/38a-opus.md:52,61–66`) |
| D9b-1a M1/report bookkeeping, recovery findings; opener/control/TX | Flush ACK identity, report-triggered send before recovery, Stop erasing recovery, generation-surviving deduplication. Reopen/gates, tick/channel correlation and recovery state fixed recorded cases. (`plan/phase8/reviews/d9b1a-sol.md:10–18,268–278`) |
| D9b-2, six lifecycle findings; control/RX | Handoff versus stop/completion; ownerless successor debt; settlement flag missed later obligations. Lock-through-completion and per-obligation settlement, then L1’s eligible-owner handoff rule. (`plan/maintenance/failure-mechanisms.md:60`; `plan/phase8/reviews/d9b2-sol.md:9–15`) |
| L1 M1 and fix-round receive finding; control/RX | Handoff loss followed by restart/receive. Loss reread under chain lock; restart closed, receive window **narrowed, not eliminated**. Spawn-failure abandonment also wired. (`plan/phase8/reviews/l1-sol.md:10–17`) |
| #10/#68/#69, later regressions; test/worker scheduling | `wall-clock-test-oracle`: manual participants, gates and controlled-clock unit tests replaced timing verdicts; documented exceptions remain. (`plan/maintenance/failure-mechanisms.md:19,54`) |

Related Kernel races—not UHD-owned state—include callback-before-wake-handle publication, submit versus FreezeDispatch, and cleanup versus a delayed step. (`plan/phase7/reviews/review-k.md:35–38`; `review-j.md:53`; `review-l.md:77–78`)

## 8. Test-clock capabilities and limits

M1–M7 provide registered-before-start participants, frozen time while any participant is busy, deadline stops, busy-at-notify/handoff, real-speed-only advancement when all idle, nonparticipant exclusion and non-shortenable sleeps. (`host.rs:6–31`)

Tests can deterministically hold a participant busy, order a deadline/notification/clock-lock handoff, pause before state-lock acquisition or after wake, and drive owners against separately controlled Core/FakeDevice time. (`host.rs:305–313,435–480,595–648`; `provider/test_support.rs:146–179`)

They **cannot obtain deterministic runnable-thread ordering merely by selecting Manual mode**: the OS still schedules spawned bodies; standard mutex/channel waits count as busy, and can freeze time needed by another participant. Plain test threads do not stop time. UHD-internal threads/device execution are not registered participants. These are implementation limits, not an exhaustive interleaving scheduler. (`host.rs:8–20,394–400`; `plan/phase8/reviews/37c-report.md:201`)

## 9. MockRadio comparison

MockRadio declares `Driving { stepped: true }`; its mutable rules execute through `step(&mut self, until)`, consuming Actions then advancing the model. Its stream timelines reside together in `timeline::Lines`; it does not create UHD-style owner threads. (`crates/ezsdr-mock-radio/src/lib.rs:73–95,469,2722–2745`)

Simulation time is explicit: pending callbacks are ordered by `(tick,sequence)`; `next_wakeup` jumps `now` to the earliest tick and invokes callbacks outside the state mutex, capped at 1,000 per call. External `wait_until` uses a condvar. (`crates/ezsdr-sim-engine/src/lib.rs:23–38,99–104,234–253`)

Shared decisions include `timeline::Lines`, lattice/start leads, timed/layout instants, preemption-bound helpers, covering-segment selection and report payload rules. Mock does **not** share native streamer ownership, flushes or report-queue lifetime. (`crates/ezsdr-mock-radio/src/lib.rs:1014–1018,1693–1710,1927–1949`; `provider/control.rs:296–303,808`; `provider/rx.rs:772`; `plan/maintenance/failure-mechanisms.md:10,50`)

## 10. Sizes

Method: physical lines, including comments/blanks; split at the first trailing `#[cfg(test)] mod tests` or differential-module declaration. **Production-region counts include embedded test hooks/helpers**, so they are not exact release-compiled LOC. Entire test-support/differential files count as tests. `#[test]` counts are literal attributes, not generated cases.

| File | Production region | Test region | `#[test]` | Split evidence |
|---|---:|---:|---:|---|
| `provider/mod.rs` | 796 | 457 | 13 | `:797–798` |
| `provider/core.rs` | 1,793 | 1,570 | 55 | `:1794–1795` |
| `provider/control.rs` | 1,406 | 7,057 | 201 | `:1407–1411` |
| `provider/rx.rs` | 866 | 3,323 | 71 | `:867–871` |
| `provider/tx.rs` | 1,279 | 5,658 | 145 | `:1280` |
| `provider/mode.rs` | 100 | 0 | 0 | `:1–100` |
| `provider/test_support.rs` | 0 | 900 | 0 | `provider/mod.rs:9–10` |
| `provider/control/differential.rs` | 0 | 576 | 2 | `provider/control.rs:1407–1408` |
| `provider/rx/differential.rs` | 0 | 861 | 1 | `provider/rx.rs:867–868` |
| **Provider total** | **6,240** | **20,402** | **488** | Above boundaries |
| `device.rs` | 2,191 | 1,286 | 48 | `:2192` |

No files changed; no cargo, tests, mutations or hardware runs performed. The anchor tree was not read; unrestricted UHD receive/control concurrency and measured latency guarantees remain unverified.
