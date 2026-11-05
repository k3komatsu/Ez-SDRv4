# Sol review of design note 48, first draft (2026-11-05)

`openai/gpt-6.1-sol`, read-only, HEAD `d131909`. Verbatim final answer.

## Independent list

Before reading note 48, I expected these to lose their reason: timed-setting queues/FIFOs, release/debt/restore machinery, setting-driven TX clock switches, and—**after stage 5**—general preemption.
I expected these to remain dangerous to remove: RX cut/restart readiness and refusal handling; TX chunk/window bounds, adjacent-burst continuity, two-sample closes, safe fronts, T3, report correlation, generations and recovery.

Reviewed HEAD `d131909`. Below, **U** = `crates/ezsdr-radio-uhd/src`, **R** = `crates/ezsdr-radio/src`, **M** = `crates/ezsdr-mock-radio/src`. **VERIFIED** means traced in code/source; hardware causation remains **INFERRED** where stated.

## Per-row review

### A — removal

- **A1 — CHANGE; removal justified, retained subset wrong. VERIFIED.**
  The sole production `apply` call passes `None` (`U/provider/core.rs:1023–1031`); timed queue/FIFO insertion is confined to `Some(at)` (`U/device.rs:1420–1462`).
  Keep untimed configuration, delays, coercion/read-back/refusal tests and frequency history (`U/device.rs:1464–1519,1148–1155`). But **`Dsp.flows` is not needed by `sample_at`**: its consumer is timed-FIFO `drain` (`:1047–1075`). Remove flow bookkeeping with that FIFO; preserve TX stale-header updates separately (`:1980–1992`).
  Do not authorize deletion of “most of 1,300 lines” by location: that range also tests live stream/report/close behaviour. Preserve P5/D70 packet-state coverage; separate obsolete timed-tune assertions. Amend UR-2/UR-33 and forwarding test Devices.

- **A2 — CHANGE; delete key/event, replace the queue argument. VERIFIED.**
  Runtime accounting/emission is absent: remaining references publish/compare capabilities (`R/channels.rs:470`, `R/measurement.rs:367`, `R/lib.rs:110,215`).
  The measurement explicitly counts **high-level timed `Device::apply` calls, not FIFO entries** (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1477–1479`). “Seven” is not a bound on RX starts or TX packets.
  UHD writes RX start registers through the control transport, but start time waits in the **separate RX command FIFO**; STOP bypasses and clears it ([UHD host source](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/radio_control_impl.cpp#L1116-L1133), [FPGA source](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v#L157-L199)). TX timestamps travel with data, not through the timed-setting queue.
  Thus delete the obsolete capability, not because “remaining commands cannot reach seven.” Preserve start/stop sequencing and back-pressure behaviour; update RM-6/10/11/**20/35**, MR-3, UR-9/34, measurement fixtures, payload registration and schemas together.

- **A3 — OK. VERIFIED.**
  `halted()` only returns `State.halted`; its callers are timeline tests (`R/timeline.rs:249–255,428,470`; `crates/ezsdr-radio/tests/timeline.rs:469,513,540,677`).
  Delete the query/field, **not refusal coverage**: retain assertions on the plan, `streaming()`, refusal omission and prune equivalence (`R/timeline.rs:215–221,263–266,488–512`). Halt remains normative.

- **A4 — OK. VERIFIED.**
  Production acquires the DSP gates only around open/reopen; the timed-write competitor is test code (`U/provider/core.rs:1065–1069,1186–1199,1380–1381,1597–1598`).
  Remove those gates, but retain report gates, generation isolation, flush acknowledgement/deadline and unrelated-report preservation (`:1072–1181`).

- **A5 — CHANGE; distinguish dead APIs from live failure state. VERIFIED.**
  `Channels::without` and `rx_start_now` have no production caller; removing them is sound (`R/timeline.rs:51–53`; `U/device.rs:1563–1565`; production RX calls at `U/provider/rx.rs:287,385,405`).
  `Recovery::Halted` is **written on a reachable failure**, although only tests read that variant (`U/provider/tx.rs:180–183,805–855`). It can be collapsed because the failure also clears `clock`, but retain the failing-reopen test: subsequent queued/new payload must not send. Do not delete that behavioural assertion with its accessor.

- **A6 — OK, with precise wording. VERIFIED.**
  The cited stale comments exist (`U/provider/control.rs:352–356`, `U/provider/core.rs:438–439`, `M/lib.rs:1779–1798`).
  `switch_cut` still denotes a reachable termination cut, so **rename/explain it**, not delete its checks (`U/provider/tx.rs:512–525`). Historical Changes tables remain history; withdrawn rule IDs remain stubs.

### B — simplification

- **B1 — CHANGE; approve the direction, strengthen the contract and acceptance test.**
  **VERIFIED:** chunk sizing uses sample cursors/window arithmetic, not metadata timestamps (`U/provider/tx.rs:516–554`). Correlation remembers **device-burst starts**, not chunks (`:355–403,613,632,664–665`). Neither requires timed continuations.
  T3 is independent (`U/provider/core.rs:1102–1112`); safe fronts count actual closing samples (`U/provider/tx.rs:691,725–727,868–869`); #70 requires a two-sample EOB, not a timestamp on every send (`:576–594,740–758`).
  UHD fragments a send and advances timestamps **only when metadata has one** ([host source](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/tx_streamer_impl.hpp#L289-L317)). Untimed continuations clear DUC timestamp propagation ([FPGA source](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L396-L425)); radio playback continues without checking each timestamp while active ([radio source](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L337-L379)). P7 observed continuous signal without the timed retune (`plan/phase8/bench-results.md:224–227`).
  Specify **first send of each device burst**, not each Kernel burst: adjacent Kernel bursts and held-tail sends stay untimed. Closing continuations stay untimed and ≥2 samples.
  **INFERRED:** this fixes #78. Timed metadata does not itself prove why starvation began. A deliberately delayed continuation can reproduce the packet-drop cascade, but cannot prove the Module’s observed starvation cause.
  Keep underflow abandonment: late untimed samples must not silently resume on a shifted timeline (`U/provider/tx.rs:1131–1139`). The first untimed continuation can still initially carry DUC time (`U/device.rs:614–642`).
  Name tests for chunk/window boundaries, repeat wraps, adjacent bursts, short/error sends, per-channel ACK correlation, safe restart, T3 and underflow abandonment. Amend UR-22/23/33 and relevant limitations; verify #78 on hardware before calling it fixed.

- **B2 — OK. VERIFIED.**
  T3 protects stale timestamp state across open/reopen, including a previous timed one-shot; untimed continuations do not make every preceding burst untimed (`U/device.rs:583–596,1942–1959,2086–2091`).
  Retain P5 A/C and flush ACK/deadline/generation tests. Do not merge this with obsolete DSP-command draining.

- **B3 — CHANGE; remove it, but correct the evidence and complete the scope. VERIFIED.**
  The live delay is exactly the successor readiness term (`R/timeline.rs:533–540`), supplied by both Providers (`U/provider/control.rs:155–165`; `M/lib.rs:1439–1447`).
  Remove **only release-window time**: retain old-rate receive-call duration, delivery allowance, start lead, lattice rounding, configure completion warnings and missed-start handling.
  Also update RM-20/25/35, MR-3, UR-3 timing prose/UR-9/25/34, measurement comparison and fixtures—not merely RM-6/schema.
  The “3 ms candidate passed” bench is **not proof of a 3 ms post-removal margin**: its actual remaining lead was 6.7–9.4 ms, partly because the old floor still applied (`plan/phase8/bench-results.md:296`).
  Retarget `rm_25_a_receive_successor_adds_release_window_before_call_and_lead` to assert the retained terms exactly; rerun differential/VH and hardware restart candidates after removal.

- **B4 — OK, deferred into stage 4. VERIFIED today; future representation requires design.**
  TX replay ignores Stop/Start and only terminates the existing clock (`R/timeline.rs:456–486`); current multi-clock machinery is unnecessary (`U/provider/tx.rs:330–342`; `M/lib.rs:1381–1407`).
  Preserve booking-time registry termination for loss/stop before origin (`U/provider/core.rs:669–679`), cancellation and Manifest history.
  Stage 4 should have **one current clock**, not “only one registered/historical clock.” Old domains/records must remain usable for delayed reports and completed bursts. Include configure refusal → corrected values → retry before choosing the representation.

- **B5 — OK. VERIFIED.**
  The three constructors mean orphaned-clock or failed-reopen cancellation (`U/provider/tx.rs:348,828,844`), not cold changes.
  Rename without changing `cancellation_reports`, event counts or cancellation records (`R/lib.rs:119–133`; `U/provider/tx.rs:1015–1029`).

### C — retain

- **C1 — OK. VERIFIED.**
  Startup, restart, first sample, stop failure and loss remain reachable (`U/provider/rx.rs:126–140,265–319,385,503,539`).
  The uncertain `abandon_unowned` case is specifically **successful `rx_start`, then owner-thread spawn failure** (`U/provider/mod.rs:635–650`), not an unidentified operation. Keep cleanup/no-post-loss-call tests.

- **C2 — OK. VERIFIED.**
  Adjacent bursts, Stop, finite endings and failed closes still use tail/front/recovery (`U/provider/tx.rs:444–480,543–575,715–855`). Preserve T2, safe fronts and F8; A5 only simplifies redundant halt representation.

- **C3 — OK. VERIFIED.**
  Correlation/generation and strict in-flight bounds remain required (`U/provider/tx.rs:355–403,418–419,516–532,1052–1057`).
  The window also bounds Stop payload and will protect the stage-5 repeat-switch boundary. Remove neither it nor Mock’s Stop model with preemption.

- **C4 — CHANGE; retain the behaviours, qualify blanket reachability.**
  **VERIFIED:** snapshots, staging, refusal, prune/made and lead warnings have production consumers (`U/provider/control.rs:254,314–324`; `U/provider/core.rs:527,589–606`; `U/provider/rx.rs:330,354–385,423`).
  But `owner_step`’s changed-set reopen and same-command `Nothing` branches are not demonstrated reachable today: active RX streamers open at arm, channel sets never change, and each accepted restart has a new sequence (`U/provider/mod.rs:555–560`; `U/provider/core.rs:614–625`; `U/provider/rx.rs:362`).
  Mark those subcases **INFERRED/unreachable pending trace**, rather than treating every branch of the old ownership mechanism as justified. Keep configure/refusal/loss ordering; do not force a larger ownership rewrite here.

### §3.4 — CHANGE

General preemption is **still reachable today** (`U/provider/tx.rs:431–457,957–1012`); stage 5 is the right removal boundary.
However, overlap rejection does **not** make `dispatch_start`, `later_bound`, or `late_policy_front_outcome` obsolete:
a valid reservation can wait until its lead expires, or start before the preceding close’s safe front (`:479–485,904–945`).
Deleting them permits late/safely-spaced bookings to be sent at invalid times. Preserve `ur_21_first_dispatch_rechecks_all_late_policies` and post-EOB safe-start tests.
Likewise split Mock’s preemption front from its still-needed Stop/safe-front use (`M/lib.rs:1342–1376`). TX Stop’s redundant planner notification can be addressed with B4, preserving owner notification and records.

## Missing items

1. **Timed `rx_stop` argument. VERIFIED:** production always passes `None` (`U/provider/rx.rs:287,405`); X3x0 ignores timed STOP. Remove the trait option; keep any timed raw bench probe outside the production trait. Update UR-2/33.
2. **Always-empty Mock `applied` section. VERIFIED:** created empty (`M/lib.rs:389`), with no staged-value writer. Decide its removal explicitly, including readers/spec/record tests; do not confuse staged values with device read-back.
3. **Fake flow scaffolding after FIFO removal. VERIFIED:** `open_flow`, `close_flow`, `end_rx_flow`, flow cursor and timed-history flags largely serve `drain`, not loopback frequency history (`U/device.rs:558–579,1028–1084`). Include this cleanup while retaining header-state tracking.
4. **Constant TX parameter histories in Mock. VERIFIED today:** `ChannelModel` stores three Timelines (`M/channel.rs:56–62`), but production `.set()` calls update only RX (`M/lib.rs:990–1009`). Inventory them, but decide their final form with stage 4; RX histories remain necessary.
5. **Old owner decision branches/marks:** inventory separately as above under C4. Their original changed-membership purpose is gone; retaining the entire abstraction needs a current caller argument.

## Staging and §5 answers

1. **B4: put it inside note 47 stage 4.** Design one current-clock lifecycle once; retain historical registry/report identity. Reject a separate S3 followed immediately by another clock rewrite.
2. **A2: delete.** Its timed-apply unit no longer describes a supported operation. Document the separate RX command FIFO and TX data path instead of claiming all timing shares a depth-seven queue.
3. **B3: delete.** Preserve the other readiness terms and parity; do not simultaneously reduce profile start lead. Repeat the candidate bench against the new floor.

**§4 recommendation:** S1 first, with an independently reviewed test plan and a narrow hardware confirmation before claiming #78 solved; then S2 with explicit retained-test/mutation mappings; then stage 4 including B4, followed by stage 5. Keep startup-transient modelling separate.
The tables currently promise preservation mostly by topic. Before implementation, name the surviving tests and replacements—especially for the broad A1 deletion.

**Verdict: NOTE NEEDS CHANGES.** The simplification direction is sound; A1’s retained scaffolding, A2’s queue proof, B1’s acceptance claim, and §3.4’s removal scope need correction.

Not checked: no builds, tests or bench runs; no files changed. #78’s cause and fix remain unverified on hardware.
