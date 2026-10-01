# Phase 7 — Native UHD Provider: overview and plan

| Field | Value |
|---|---|
| Status | **Accepted at Gate X** (owner, 2026-10-01: "終わったらmainへmergeしてください．そしてGate Xを受理します"; §11) and merged into `main`; the bench session and Reviews L–T in [`design-notes.md`](design-notes.md) §11–§20 and [`bench-results.md`](bench-results.md). **Accepted at Gate P** (owner, 2026-09-27, every decision as recommended, §11); **implemented** on 2026-09-27 (the owner: "Phase 7を実装してください"; 00-overview §9 steps 1–6, [`design-notes.md`](design-notes.md) §6), not yet reviewed (Review L); earlier the same day: "実装はしないで". Designed first: The owner asked on 2026-09-27 for Phase 7 to be designed and implemented up to the point where hardware is needed ("それではPhase7の設計と実装を進めてください．実機がいる直前まで進めてください"), then, during the work, to stop at the design and to design it in detail ("設計だけで止めてください．ただし，設計は詳しく設計してください"). No code was written: an implementation begun for KG-4 was reverted before anything was committed. The design was reviewed by Opus: Review J (CHANGES_REQUIRED: 4 P0, 11 P1, 23 P2) led to a broad revision — the time rules for a paced Authority, the data thread's wake, Lease and admission rules, the transmit path's preemption and command queue, enabling a stream on the device — and Review K re-reviewed it (CHANGES_REQUIRED: 1 new P0 — the revision had broken enabling transmit at runtime —, 5 P1, 14 P2; one Review J test gap still open), and its findings were fixed in turn (§9, [`reviews/`](reviews)); the triage is in [`design-notes.md`](design-notes.md). When the owner asks for the implementation, it follows §9's steps 1–6, then the bench session of [`bench.md`](bench.md), then Gate X. |
| Phase | Vision §67 Phase 7: "Native UHD Provider". Predecessor: Phase 6 (Python Easy API; accepted at Gate X 2026-09-27). Successor: Phase 8 (Mock → X310 parity test). |
| Scope | A Module `ezsdr.radio.uhd` that is a Radio Provider and a device-paced Time Authority for a USRP X310 with UBX daughterboards (or, since 2026-09-30, one OBX or one CBX: spec 18's `x310-obx` and `x310-cbx`), built on UHD's own C API; the Kernel changes that let the coordinator drive a device-paced Run (the UHD spike's findings K1–K13 and Phase 2's MA-8 risk); the Vocabulary and Module amendments those need; the server and Python changes that make the Phase 6 snippets run against a USRP by changing the profile only; and a bench procedure for the first hardware session. Everything except that session runs without hardware. |
| Not in scope | §3 lists it. In one line: no RealtimeEmulation, no second USRP in one Run, no USRP2 profile, no RFNoC Replay or DDC/DUC capabilities, no remote listener, no Mock profile value changes (Phase 8). |
| Language | English, like Phases 1–6. |
| Location | Spec 18, [`design/18-uhd-radio.md`](../../design/18-uhd-radio.md), is the new Module's spec; it moved there from this directory at Step X (2026-10-01), as specs 11, 14 and 16 did. Spec 19, [`19-amendments.md`](19-amendments.md), holds the Kernel amendments KG-1…KG-14 and the Vocabulary, Module and frontend amendments VE-1…VE-6; their text reaches `design/` in the commit that implements each (GZ-7), and the file stays here as the record. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Documents:

```text
plan/phase7/
  00-overview.md     this file: what exists, the holes, scope, decisions, crates, governance, tests,
                     traceability, sequencing, exit criteria, decision log
  (18-uhd-radio.md   spec 18, moved to design/ at Step X): the Module ezsdr.radio.uhd 0.1.0 (UR-1…UR-35), its device boundary,
                     its profile x310-ubx 0.1.0, its Authority, its test double and its bench tests
  19-amendments.md   spec 19: Kernel amendments KG-1…KG-14; Vocabulary, Module and frontend
                     amendments VE-1…VE-6; Appendix A, the mutations the implementation must record
  bench.md           the first hardware session: set-up, RF safety, the ordered steps B0…B9, what each
                     shows and where its result is recorded
  vision-issues.md   the Vision edits the specs need, to be applied at Step X with the owner's approval
  design-notes.md    what the design was checked against, and Review J's findings and their triage
  prompts/, reviews/ the review brief and the reviewer's report
```

---

## 1. What Phase 7 has to prove, and what already exists

Vision §59: "Existing experiments written for MockRadio should run against a real USRP by changing the BindingProfile rather than changing application logic. If large semantic changes are required when moving from Mock to UHD, the abstraction boundary must be reconsidered." §35: USRP support uses "a narrow native C++ bridge to UHD" that exposes typed POD data, translates exceptions into explicit status, converts UHD metadata and asynchronous errors into typed RuntimeEvents, and runs in-process, with a vanished device becoming `DEVICE_LOST`. §15: "In a Hardware or HIL Run [the Time Authority] is the device timekeeper, which publishes a relation to host monotonic." §32 and MA-30: in HardwareInLoop and Hardware no Provider is stepped, and Executors and Sinks run on threads.

The UHD spike of 2026-09-26 (branch `spike/uhd`, findings in [`plan/spikes/2026-09-26-uhd.md`](../spikes/2026-09-26-uhd.md)) was Phase 7's prototype. It ran the acceptance crate's own Specs against a UHD Provider and a device-paced Authority, on a wall-clock fake device and against libuhd 4.10 on this Mac with no device attached, through the Phase 3 coordinator with one Kernel line changed. What passed unchanged:

| Piece | Where | State before Phase 7 |
|---|---|---|
| Class derivation: `Pacing::Device` + `ezsdr.rf_path: cabled` → HardwareInLoop, and the `ezsdr.time.class` cross-check | MA-41, `plan/compile.rs:328` | VERIFIED by the spike; tested by `ma_41_execution_class_table` |
| Validate, admission and matching over a Radio Model tree | SB-6…SB-46 | VERIFIED by the spike with MockRadio's tree |
| T0 on a device root, `SpecTime` resolution in device ticks | KC-15, KC-16 | VERIFIED by the spike |
| Burst target conversion onto the transmit clock | SC-23a, SC-23b | VERIFIED by the spike |
| Capture Sink, host Link, continuity, an overflow recorded as an `overflow_restart` gap with its lost count | HD-10, SC-30 | VERIFIED by the spike on the fake device |
| `BurstTracker` records, `mark_artifact` via the Policy, a Manifest with fidelity `real` | SC-28, RS-30, MA-42 | VERIFIED by the spike |
| A Session: timed capture (v61_02's shape) and `start_repeat` + `capture` (§57's shape) | KC-28, RS-19 | VERIFIED by the spike, with K5's workaround |
| The UHD C API from Rust: find, make, error strings | `uhd.h` | VERIFIED on this Mac: `uhd_usrp_make` returns an error code and `uhd_get_last_error` its text when no device answers |

What does not exist: a UHD Provider or a device-paced Authority in `main`, a coordinator that runs any class but Simulation, and the answers to the spike's Kernel questions. §2 lists every hole with its evidence.

## 2. The holes, with evidence

VERIFIED means reproduced (by the spike's fake-device runs, a probe, or reading code and headers); INFERRED means expected but checkable only on hardware (AGENTS.md §6).

| # | Hole | Evidence | Disposition |
|---|---|---|---|
| 1 | **KC-2 refuses every class but Simulation.** | `coordinator/pipeline.rs:372-380` fails the plan with "KC-2: Phase 2 runs the Simulation class only"; spike K1 changed exactly this line and no existing test failed, because `kc_02_a_wall_paced_authority_is_refused` pins only WallPaced | KG-1 |
| 2 | **Nothing drives a device-paced Run's Sinks between client calls.** MA-30's table puts Executors and Sinks on threads in HardwareInLoop and Hardware, but the coordinator steps every instance only inside a round, and rounds run only inside `RunHandle` calls | Spike K8, VERIFIED: an overflow Run recorded `link_drops_seen: 298` after its capture, blocks the rx thread published while no call was running; `coordinator/stepping.rs:119` `round` is called only from `run_loop`, `submit`, the start and the cleanup drain | KG-2 |
| 3 | **A fatal event in a device-paced Run waits for the client.** KC-31 applies the Policy after a round, so a `DEVICE_LOST` or `CLOCK_LOST` emitted while no call runs reaches the Policy, and the radios stop, only at the client's next call | follows from row 2 (`stepping.rs:217` `drain_and_react` runs only inside a round); a Python client computing for minutes between calls leaves a transmitter on | KG-3 |
| 4 | **Session admission races a Provider that is not stepped.** KC-21's round after dispatch is what lets MockRadio apply a `cold` change before the next Action is admitted; a hardware Provider applies it on its own thread, later | Spike K5, VERIFIED: after `SetParameter(radio.tx.channels = 1)`, `start_repeat` was refused with "SC-23: usrp/tx has no running transmit SampleClock" depending on the race; the spike's workaround was `advance_to(+20 ms)` | KG-4 |
| 5 | **RS-19's lead is counted from admission, while MA-10 defines it from receipt.** | Spike K6, VERIFIED: every untimed Session transmission on the fake device was `TIME_ERROR send_asap`, 1.83 ms of its 2 ms lead gone before the Provider saw it; MA-10: "the least lead this instance needs between receiving a timed Action and that Action's instant"; RS-19 adds it to the current instant at admission | KG-8, VE-2 (RM-14) |
| 6 | **A device-paced Spec Run ends `Completed` when the agenda is empty,** because `next_wakeup` then returns `None` although the device streams | Spike K2, VERIFIED: the Run completed right after the first block; `stepping.rs:352` requests `Completed` on `None` for a Spec Run | KG-5 |
| 7 | **MA-8 is not enforced.** A `prepare` or `arm` that hangs on I/O hangs the Run before any cleanup deadline applies | Phase 2's named Gate X risk; `plan/phase4/00-overview.md` §3 assigns it to Phase 7 ("the first Module whose calls can block on I/O is the UHD Provider"); `pipeline.rs:763-771` and `:842-850` call `prepare` and `arm` directly | KG-6 |
| 8 | **Transmit and receive sample grids disagree.** The transmit SampleClock starts at `arm` (MR-9), the receive one at T0 (MR-11); when `T0 − arm` is not a whole number of samples, a burst at "T0 + n samples" moves to the next transmit sample | Spike K3, VERIFIED on the fake device (83 ticks = 415 ns at 200 MHz and 1 Msps); reachable in Simulation and pinned as a ceiling by `k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample` (`crates/ezsdr-acceptance/tests/v58.rs:635`); the owner put its fix in Phase 7 (Phase 4 Gate X) | KG-7, VE-2 (RM-25), VE-4 |
| 9 | **TM-16c's "callbacks observe `now()` equal to their fire time" is false for a device-paced Authority,** whose time keeps moving while a callback runs | Spike K12; TM-16a1 already says a device-class Authority reads the real clock; `design/01-time-model.md:173` also states "`cancel` reports whether the callback was still pending" twice | KG-9 |
| 10 | **A Provider that is not stepped cannot report a lost device.** KC-30 turns a `DeviceLost` error into `DEVICE_LOST` only when a Module call returns it; a hardware Provider discovers the loss on its receive thread, where no call returns | found while writing spec 18: `stepping.rs:198-205` and `pipeline.rs:1686` emit `DEVICE_LOST` only for a returned error; the spike logged its receive errors to a section and never ended the Run | KG-10 |
| 11 | **No hardware Run records its root's relations,** to UTC (which TM-18 requires of every Run but a Simulation one, KA-16) and to `host.monotonic` (which TM-18 and §15 require the device timekeeper to publish) | `coordinator/ending.rs:258` writes `relations: Vec::new()` for every class; no Kernel API lets an Authority publish a relation (`grep ClockRelation crates/ezsdr-kernel/src` finds only the type, the schema and the Manifest field) | KG-11 |
| 12 | **A child Run of a hardware Session would reopen its parent's device.** KC-37a builds the child from a fresh Assembly; on a USRP the parent's Provider holds the device | KC-37a's ceiling ("the first hardware Provider decides whether `run_child` quiesces the parent's streams (Phase 7)"); `crates/ezsdr-server/src/lib.rs` builds a child's Assembly with `assemble`, which would open the same `args` again | KG-12 |
| 13 | **The radio Vocabulary cannot say what UHD reports.** `TimeErrorOutcome` has no value for "the Provider sent the burst on time by its clock and the device reported it late"; `TX_UNDERFLOW`, `ALIGNMENT_ERROR` and `CLOCK_LOST` are declared kinds with no payload type | Spike K11 (it mapped the device's late report to `refused`); RM-11's payload table lists five kinds; `crates/ezsdr-radio/src/lib.rs` `payloads` has no type for the other three | VE-3 |
| 14 | **RM-8's coercion exists only inside MockRadio,** so the only static description of an X310 in the repository is the Mock profile, and a second radio Provider would copy it or depend on MockRadio | Spike K13; the spike built its tree and `coerce` from a `MockRadio` (a Module depending on a Module, which MA-3 forbids and `ma_03_no_module_crate_depends_on_another` would refuse in `main`); `crates/ezsdr-mock-radio/src/coerce.rs` is RM-5, RM-7 and RM-8 over a profile | VE-1 |
| 15 | **No UHD Provider and no device-paced Authority** | Vision §67; the spike's code is on `spike/uhd`, which is never merged | spec 18 |
| 16 | **The server cannot build a UHD Run,** and `connect()` without a profile always means the simulated default | `crates/ezsdr-server/src/catalogue.rs`: the Authority must be `ezsdr.sim-engine`, and any other Module is "no Module … in this server"; Phase 6 decision S7's ceiling: "Phase 7's lab server makes its own profile the default" | VE-5 |
| 17 | **On a wall-paced Session a client has no instant to start a capture from.** A `capture` without `at` starts at admission, a client round trip after the `sleep` before it | `plan/phase6/00-overview.md` §3, "Capture timing on a wall-paced class", assigned to Phase 7; `python/ezsdr/session.py` `sleep` returns `None` although the `advanced` reply carries `now` | VE-6 |
| 18 | **UC-3 forbids what a device does on a `cold` change.** It says a restarted stream's new SampleClock starts at the instant the old one ends; a USRP stops the stream, reconfigures and restarts it, which takes time | `design/05-module-api.md` UC-3; MockRadio restarts at one instant (MR-18); a UHD rate change needs a stream stop, a DDC reconfiguration and a timed start, whose least spacing is INFERRED (spec 18 UR-25's 50 ms, bench B8) | KG-13 |
| 19 | **A Detached Lease expires only inside a client call.** KC-36 checks the Lease "at every `RunHandle` call and after every round", the Lease is a field of `RunHandle`, and in a device-paced class rounds run only inside calls: a detached client that never calls again leaves a transmitter on past its TTL | `crates/ezsdr-kernel/src/coordinator/mod.rs` (`RunHandle { lease, … }`); RS-22's own counterexample, "a detached transmitter alive forever"; found by Review J (P1-3). The server hides it today by finishing on end of input (`crates/ezsdr-server/src/lib.rs:170-176`) | KG-2 (KC-36) |

Spike findings that need no amendment, and why:

| Finding | Why no amendment |
|---|---|
| K4 (a Module holding Provider and Authority must take the Authority role under its radio binding, or a Session fails) | SB-22c already says so: "a Module holding both Provider and Authority that `authority` names is a resource in a Session … the Authority rides on it under both Run kinds". Spec 18 UR-6 requires `authority` to name the radio's own binding |
| K7 (`ActionReceiver` is pull-only; a hardware Provider polls) | Kept (decision T6): KG-4 bounds the wait and KG-8 makes the poll period part of the declared lead; a blocking receive is additive later |
| K9 (an abandoned cleanup step loses the Provider's Manifest sections) | KC-44's behaviour is right (a held slot cannot be read); KG-6 bounds `prepare` and `arm`, and spec 18 UR-16 bounds every join inside the Provider's own `stop`, so that the RS-8a deadline is not reached by a healthy device |
| K10 (a hardware Provider's events come from its own threads; the hot and control paths are indistinguishable) | RM-11 already requires `RX_OVERFLOW` on the hot path (Phase 4, VC-1), and `EventSink::emit` is callable from any thread (`crates/ezsdr-kernel/src/event.rs:503`, a mutex-guarded ring). Spec 18 UR-20 emits `RX_OVERFLOW` on the hot path from the receive thread and every other kind on the control path from the thread that decided it |
| K13 (the device may apply a value other than the claimed one) | KC-27's ceiling stands. Spec 18 UR-12 refuses a sample rate the device does not apply exactly (a SampleClock must be exact), and records every applied frequency and gain beside the claim in the Provider's `applied` section |
| SC-31a's per-channel `ALIGNMENT` has no UHD producer | VERIFIED by Phase 4 Review D in UHD's streamer source: an alignment failure yields no samples or only aligned ones, never a partial channel set. Spec 18 UR-19 maps it to a whole-stream gap with `ALIGNMENT_ERROR`; SC-31a keeps its flag for a future Provider that can lose one channel |

## 3. Scope

### In scope

1. **Spec 18** — the Module `ezsdr.radio.uhd` 0.1.0 (crate `crates/ezsdr-radio-uhd`): a Radio Provider and a device-paced Authority for one USRP per Run; the device boundary `Device` with two implementations, the UHD C API behind the cargo feature `uhd` and the wall-clock `FakeDevice` test double; the profile `x310-ubx` 0.1.0; the bench tests, compiled only with `uhd`.
2. **KG-1…KG-14** — the Kernel amendments of spec 19 (holes 1–12, 18 and 19).
3. **VE-1…VE-4** — `radio` 1.3.0 (the shared device description, RM-25's grid rule, RM-14's device lead, the payloads of hole 13) and `ezsdr.radio.mock` 1.3.0 (holes 8, 13, 14).
4. **VE-5, VE-6** — the server's catalogue gains the UHD Module behind its own `uhd` feature, and `EZSDR_PROFILE` names the default profile (hole 16); the Python `sleep` and `wait_until` return the instant the Run stands at (hole 17).
5. **The carriers of §8**, all runnable without hardware, and the bench procedure of `bench.md` with its hardware tests.

### Out of scope, with the phase that owns each

| Item | Why not now | Owner |
|---|---|---|
| RealtimeEmulation (a wall-paced Simulation Engine) | KG-1 admits the device-paced classes only. A wall-paced Engine is a Simulation Engine change with no Phase 7 consumer; its first consumer is an Executor with real deadlines | Phase 10 (KC-2 keeps refusing it) |
| A second USRP in one Run, and v3 behaviour 4 on hardware (10 MHz + PPS aligned start, PPS source armed first; v61_04 on a USRP) | The second device's root must relate to the Authority's root (TM-18's first sentence: each device publishes a relation to `host.monotonic`), and claiming them equal because they share a PPS is a coherence claim the owning Provider must declare and the bench must measure. One device is enough for §59's claim | after Phase 8, owner decision (§11) |
| A USRP2 / N2x0 profile | The bench has a USRP2 whose daughterboard is not recorded anywhere in the repository; a profile needs its tuning range and gain table. Bench step B9 records its `pp_string` so that the profile can be written | when the daughterboard is known (§11) |
| RFNoC Replay for `tx.repeat`, DDC/DUC and device FFT capabilities, `extensions.uhd.rfnoc.*` | §35 names them; the host-side repeat of UR-22 already satisfies RM-13 and v3 behaviour 1, and nothing in §58 needs device memory | a later phase, when a Spec needs a waveform longer than host streaming sustains |
| A remote listener, server-owned profiles and authentication; the `body_bytes` limit | Phase 6 assigned them to Phase 7 "with the first lab server holding a device" (S9). The Phase 7 bench runs the client on the PC that holds the USRP, so no Phase 7 test crosses a machine boundary, and a listener is a trust boundary (§62) that deserves its own review. The protocol is a byte stream, so a listener is additive | a later phase, owner decision (§11) |
| Device refusals raised as Python exceptions | The Phase 6 design question stands unchanged: a refusal arrives as an event after admission, on hardware as in Simulation; `sdr.events()` shows it | when asked |
| Session replay and the artifact store | Phase 6 Gate X: after Phase 7, in the frontend | after Phase 7 |
| The v4.0 freeze items (`Endpoint::EventIn` / `EventOut`, optional `ParamDecl.update_class`, the Manifest's `spec.source`) | Freeze prerequisites, not Phase 7's; nothing here depends on them | before the v4.0 freeze |
| Per-Island threads with affinity and real-time policy | KG-1 refuses an Island that declares either in a device-paced class; KG-2's one data thread steps Executors and Sinks in MA-30's order. No Phase 7 Run has an Executor (the native Executor refuses every class but Simulation, NX-4) | Phase 10 |
| A transmit port and link-fed transmission | RM-2 reserves `tx`; the UHD Provider transmits `TxBurst`s only, as MockRadio does | Phase 10 |
| Changing the Mock `x310-like` values | §59: profile values change only from a measurement. Where spec 18's `x310-ubx` differs (the command lead), the difference is recorded as a Phase 8 input | Phase 8 |
| A blocking `ActionReceiver` receive (K7) | Decision T6 | when a measurement shows the poll matters |
| The ClockRelation of a second device's root (TM-18's first sentence, for a Provider that owns a timekeeper other than the Authority's) | With one device the Authority's relations are the Run's (KG-11); a second timekeeper's relation matters when a second device joins the Run | with multi-device |

A request to add any of these during Phase 7 is scope creep and is refused (AGENTS.md §6).

### Phase 8 inputs

Where the design makes the UHD Provider differ from MockRadio `x310-like` 1.1.0, the difference is recorded for Phase 8's parity test instead of being "fixed" in either profile now (§59: profile values change only from a measurement):

| Difference | MockRadio `x310-like` 1.1.0 | UHD `x310-ubx` 0.1.0 | Effect | Measured at |
|---|---|---|---|---|
| Declared command lead | 2 ms | 5 ms (2 ms device lead + 3 ms delivery allowance) | a Spec's lead constraint between 2 and 5 ms validates on MockRadio and is refused on the X310 (Review J, P2-18) | B8 |
| Preempting a running burst | at `now + 2 ms` | not before the in-flight window's end (10 ms) | a Spec preempting a repeat with less lead passes on MockRadio and is late on the X310 (spec 19 VE-2, RM-15) | B8 |
| Restart of a stream on a `cold` change | at once (restart lead 0) | 50 ms later (a stream enabled from 0 channels starts at once on both) | the new SampleClock starts later on the X310, and a burst sent right after a transmit rate change is moved to the new clock's origin with `TIME_ERROR { send_asap }` on the X310 and not on MockRadio (spec 19 VE-2, RM-15; Review K) | B8 |
| Repeat constraints | 256 Mi samples, even length (Replay) | 4 Gi samples, any length (host) | MockRadio is the stricter | — |
| Applied frequency | exactly the grid value | within 0.047 Hz of it (DSP resolution) | recorded in `applied`, K13 | B3 |


## 4. Cross-cutting decisions

Each row is open to reversal at Gate P; a reversal is recorded in §11.

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| T1 | How Phase 7 is delivered | Designed in detail and reviewed (Review J) before any code, with a Gate P, then implemented step by step (§9) up to the bench, then the bench session | Implementing under delegation as in Phases 4–6 (the owner stopped it: "設計だけで止めてください"); implementing and benching in one go (the bench needs the owner at a Linux PC) | The owner may reverse any row at Gate P |
| T2 | The bridge to UHD | **UHD's own C API** (`uhd.h`, `libuhd`), declared by hand in one Rust module: about 45 functions, each checked against the UHD 4.10 headers | Our own C++ shim built with `cc` (a C++ toolchain in every build and a new external crate, PO-4, to redo what `UHD_SAFE_C` already does: `uhd/error.h:89-104` catches `uhd::exception`, `boost::exception`, `std::exception` and `...` in every C entry point and returns a `uhd_error` with the text in `uhd_get_last_error`); `bindgen` (a new external crate and libclang at build time, for 45 declarations); a third-party UHD crate (an external package, PO-4, whose maintenance this repository cannot vouch for); `dlopen` at run time (the `libloading` crate, and still `unsafe`) | A C++ shim is additive behind the same `Device` trait if UHD's C API lacks a call a later phase needs (RFNoC graphs, §35's Replay) |
| T3 | Where `unsafe` lives | One module, `crates/ezsdr-radio-uhd/src/uhd.rs`, compiled only with the crate's feature `uhd`; the crate root says `#![deny(unsafe_code)]` and that module alone `#[allow(unsafe_code)]` (GZ-3) | A separate `-sys` crate outside `crates/` (splits one Module over two crates and puts it outside every governance scan); `forbid` everywhere (a C call cannot be made without `unsafe`) | none |
| T4 | Testing a hardware Provider without hardware | A `Device` trait with two implementations: `UhdDevice` (feature `uhd`) and `FakeDevice`, a wall-clock device double compiled always, constructed only by Rust code; no BindingProfile can select it (GZ-9). The fake keeps the device's queues and drift in the ways the tests depend on — an in-order transmit queue that drops a late start-of-burst, an in-order back-pressuring command queue, a loopback only between equal frequencies, a configurable drift — because Review J showed a looser fake passing tests that hardware would fail (spec 18 UR-33) | The spike's `args = "fake"` (a document could then bind a fake device and write a HardwareInLoop Manifest with fidelity `real`); a cargo feature for the fake (workspace feature unification turns it on in `cargo build --workspace` anyway, so the flag hides nothing) | The bench tests run the same test bodies against `UhdDevice` (UR-34) |
| T5 | Who drives a device-paced Run's Executors and Sinks | **One data thread per Run** (KG-2), stepping every Executor and Sink once per pass, in MA-30's order, as long as some instance progressed, and parking 200 µs otherwise; the control thread keeps the agenda, admission, the Lease and cleanup | A thread per instance (MA-30's table read literally: order between instances becomes a race, and no Phase 7 Run has more than one Sink and no Executor); stepping in client calls (spike K8); `step_until_quiescent` on the data thread (the device's clock moves on between passes, so repetition drains a stream, and at a high block rate a Sink progresses on every pass and MA-30's cap, a guard against a livelock at one instant, would end a healthy Run) | Per-Island threads with affinity and RT policy in Phase 10 |
| T6 | Session admission against a Provider that is not stepped (K5, K7) | **MA-14b + KC-21a + KC-24a:** a threaded instance has finished with every Action it took when it next calls `recv()`; after dispatching on the control path in a device-paced class, the coordinator waits until every instance it dispatched to has finished, at most `DEFAULT_HOST_BUDGET_NS`; one admission lock serializes admission with dispatch and with the freeze, now that two threads admit. `recv` stays pull-only | A synchronous dispatch callback on `Provider` (a role-trait change for what the queue can tell, and a Provider thread may be inside a device call); a fixed delay after each call (the spike's 20 ms: slow, and still a race); retrying admission (hides the race) | A blocking `recv_timeout` on `ActionReceiver` is additive if the poll period ever matters (K7) |
| T7 | Where a command lead is counted from (K6) | **From dispatch:** `min_command_lead` (MA-10) is the least lead between the coordinator dispatching a timed Action and its instant, including the time the instance takes to receive it; RM-14's lateness test uses the device's own lead, which is the declared lead less the delivery allowance the Provider declares (KG-8, VE-2); a burst that would preempt a running one is judged against what the device already holds as well (RM-15 as VE-2 amends it) | The Kernel adding an estimate of each Provider's delivery latency (the Kernel would guess a Module's behaviour); stamping each Action with its admission instant (a Kernel Action schema change for what one number per Provider carries) | Phase 8 measures the device lead and the allowance on the bench (`bench.md` B8) |
| T8 | Transmit and receive grids (K3) | **KC-15 puts T0 on every declared SampleClock's grid** (the least common multiple of their ratios' numerators), and **RM-25 makes every radio Provider register each SampleClock at an origin on its root's lattice** (a multiple of the ratio's numerator): the receive clock at T0, the transmit clock at the first lattice instant at or after `arm`, a clock restarted by a `cold` change at the first lattice instant at or after its effective instant (KG-7, VE-2, VE-4) | A Kernel refusal of off-lattice origins (TM-13b fixes a receive origin at a device's first sample, which a Kernel rule must not second-guess); moving the receive origin onto the transmit grid (the spike's workaround: T0 is then no longer the first sample, which v3 behaviour 3 needs); registering the transmit clock at T0 (KC-17 admits scheduled bursts before `start`, and SC-23 needs a running transmit clock) | Two clocks of different rates share only the instants on both lattices, which is the physics |
| T9 | An end requested off the control path in a device-paced class | **The radios stop at once** (KG-3): the data thread itself performs RS-6 steps 1 and 2 (freeze dispatch, stop every Provider) after the pass in which it requested the end, and wakes the control loop; the rest of cleanup, and the Manifest, follow at the client's next call, and the steps already done are not repeated (they are recorded in `done`, KC-39) | Waiting for the client's next call (an unbounded delay with a transmitter on); the whole cleanup on the data thread (the Session log, the Lease and the Manifest are control-path state behind `&mut RunHandle`); a separate stopper thread (the first draft: one thread and one join more — Review J, P2-9) | A supervisor that also writes the Manifest without a client call is a later change |
| T10 | Where MA-8 is enforced | **In device-paced classes only** (KG-6): `prepare` and `arm` run on a worker thread joined with `DEFAULT_HOST_BUDGET_NS` | Every class (a wall-clock timeout in the Simulation class is a nondeterministic outcome, which PO-11 exists to exclude); no class (Phase 2's named risk, and UHD calls can block for seconds) | — |
| T11 | Where RM-8's coercion code lives | **In the `radio` Vocabulary** (`ezsdr_radio::device`, VE-1): a `DeviceDescription` builds RM-2's tree and computes RM-5's defaults, RM-7's refusal and RM-8's coercion; MockRadio and the UHD Provider each supply a description | A copy in the UHD crate (two implementations of one Vocabulary rule, which Phase 8 would then compare as well); the UHD crate depending on MockRadio (MA-3) | none |
| T12 | The UHD profile and versions | Profile `x310-ubx` 0.1.0 and Module 0.1.0: the X310 values of MR-3 (VERIFIED or INFERRED as MR-3 marks them), except the command lead (5 ms: the 2 ms device lead plus a 3 ms delivery allowance, INFERRED); 0.x because nothing is measured yet | 1.0.0 now (a version that claims a verified envelope); reading limits from the device in `coerce` (MA-11: pure, no hardware) | 1.0.0 after the bench, with values from Phase 8's measurement |
| T13 | Devices per Run and the Authority | One UHD device per Run; the Authority role is taken by the radio's own binding (SB-22c), and both roles share one opened device | Several devices now (out of scope, §3); an Authority binding of its own (K4: a Session then derives a second resource from it) | Multi-device after Phase 8 |
| T14 | A child Run of a device-paced Session | **Refused** (KG-12), logged as a rejected entry with the reason | Letting the child reopen the device (UHD refuses or the two fight over one device); quiescing the parent's streams for the child (a design with no Phase 7 consumer) | A later phase that needs §54's sweep on hardware designs the hand-over |
| T15 | The default profile of a lab server | `Config.default_profile`, which the binary sets from `EZSDR_PROFILE` (a path to a BindingProfile document), is used when `connect` names no profile (VE-5); the built-in simulated default otherwise; the library reads no environment variable | A `--profile` flag (the Python package spawns the server and would have to pass it, and EA-18 keeps binding content out of Python); a default in Python (EA-18) | Server-owned profiles for remote clients come with the remote listener |
| T16 | The server's UHD build | A cargo feature `uhd` on `ezsdr-server`, off by default, that turns on `ezsdr-radio-uhd/uhd`; without it the catalogue knows the Module and refuses to open a device with a message naming the feature | Always linking libuhd (every build and CI would need UHD installed, which Vision §13 says the software path must not); a second server binary (the same catalogue twice) | none |
| T17 | TM-18 for a device-paced Run | The Authority measures its root's relations to `host.monotonic` and to `utc` and publishes them through a provided method `Authority::relations()` (default none); the coordinator records them (KG-11) | The coordinator measuring them from `now()` and the host clock (TM-18 puts the measurement with the timekeeper, and only the Authority knows the error of its own `now()`); a Provider section (TM-18 names the Manifest's clock record); a UTC relation alone (TM-18's and §15's relation to `host.monotonic` would still have no carrier) | Relations of other device roots, by their Providers, with multi-device |
| T18 | When the bench is part of Phase 7 | **Gate X needs bench steps B0–B8** (the acceptance experiments and §57 on an X310, changing only the profile), not the envelope measurement table, which is Phase 8's | Gate X without hardware (a "Native UHD Provider" that never touched a USRP, against §59); Gate X only after Phase 8's measurement (merges two phases) | owner decision at Gate P (§11) |

## 5. Crate layout

One crate is added. No external Rust package is added (PO-4): `ezsdr-radio-uhd` uses `ezsdr-kernel`, `ezsdr-radio`, `ezsdr-hostmem` and `serde_json`, and links `libuhd` through its build script only when its feature `uhd` is on, finding it with `pkg-config` (GZ-5).

| Crate / package | Tier | Change |
|---|---|---|
| `ezsdr-kernel` | Kernel | KG-1…KG-14: `coordinator/paced.rs` (new, private: the data thread with its early stop and Lease check, the wake, the bounded call, the finish wait), `coordinator/state.rs` (the queue's finish count, the admission lock, the wake handle, the Lease deadline), `stepping.rs`, `pipeline.rs`, `ending.rs`, `module_api.rs` (`Authority::relations`, a provided method: no new item); design text; `tests/kernel_surface.rs` (`ClockRelation` in the `ma6_documents!` list, MA-46); tests and doubles (`tests/support/run_doubles.rs`: `WallAuthority`, `ThreadedProvider`) |
| `ezsdr-radio` | Vocabulary | 1.3.0: `device` (moved from MockRadio: `Grid`, `DeviceDescription`, tree, defaults, coerce), RM-25, RM-14's device lead, four payload types and their schemas |
| `ezsdr-mock-radio` | Module | 1.3.0: uses `ezsdr_radio::device`; its transmit and `cold` origins on the lattice (MR-9, MR-18); `radio ^1.3.0` |
| `ezsdr-radio-uhd` | Module (new) | spec 18: `lib.rs` (descriptor, `open`), `profile.rs`, `device.rs` (`Device`, `FakeDevice`), `uhd.rs` (feature `uhd`: the C API and `UhdDevice`), `authority.rs`, `provider/` (`mod.rs`, `rx.rs`, `tx.rs`, `control.rs`), `build.rs`; tests `tests/fake.rs`, `tests/uhd_api.rs`, `tests/hardware.rs`. Its dev-dependencies go beyond PO-8's Kernel-only list: `ezsdr-sink`, `ezsdr-sink-capture` and `ezsdr-link-host`, because `tests/fake.rs` assembles a Run as the server's catalogue does (workspace crates only; `governance.rs` enforces the list; Review L, deviation 4) |
| `ezsdr-server` | frontend | 0.2.0: the catalogue gains `ezsdr.radio.uhd` and its Authority; feature `uhd`; `EZSDR_PROFILE` |
| `ezsdr-acceptance` | tests | `tests/uhd.rs`: the §59 rehearsal on `FakeDevice`; the governance lists (MA-3, PO-2, PO-11) and `gz_08_hardware_tests_are_gated`; `v58_10` also scans `python/examples/bench_loopback.py`, which names no Module or profile (EA-18); the K3 ceiling test replaced by `kg_07_a_burst_at_t0_plus_n_samples_is_exact` |
| `python/` | frontend | `sleep` and `wait_until` return the `TimePoint`; `examples/bench_loopback.py` (bench step B7) |
| the others | — | unchanged |

## 6. Governance

OV-1…OV-23b, PO-1…PO-12, GV-1…GV-6, GW-1…GW-5, GX-1…GX-6 and GY-1…GY-7 bind Phase 7. The rules below add what it needs.

- **GZ-1** Phase 7 rule ids are `KG-n` (Kernel amendments), `VE-n` (Vocabulary, Module and frontend amendments), `UR-n` (spec 18) and `GZ-n`, protected by OV-1. An amended rule keeps its id (PO-1); a new rule in an existing series takes the next free number or a letter suffix (`KC-21a`, `KC-46`). *Process obligation.*
- **GZ-2** The Kernel changes only as KG-1…KG-14 say. The public surface gains one provided method on an existing trait (`Authority::relations`) and no item; no Kernel schema changes (`ClockRelation` has one since Phase 1). *Checked by `kernel_surface` (`ov_23b`: 116 NEW / 292, unchanged) and `schema_freeze`. `ma_06_role_signatures_name_only_documents_and_handles` refuses the new signature until `time::ClockRelation` joins its closed `ma6_documents!` list, which KG-11 does together with MA-46's list (Review J's probe, P1-9).*
- **GZ-3** PO-2 is amended: every crate forbids unsafe code except `ezsdr-radio-uhd`, whose `src/lib.rs` says `#![deny(unsafe_code)]` and whose only `unsafe` is in `src/uhd.rs`, a module compiled only with the feature `uhd` and marked `#[allow(unsafe_code)]`. *Checked by `po_02_every_crate_forbids_unsafe_code`, amended to require exactly this of that one crate (the `deny` line, the module's `cfg` and `allow`, and no `unsafe` token in any other file of the crate).*
- **GZ-4** PO-11 is amended: output-affecting code spawns no thread and reads no wall clock except `run_cleanup`'s step threads, **`coordinator/paced.rs`**, which runs only in HardwareInLoop and Hardware (KG-1, KG-2, KG-3, KG-6), and **`ezsdr-radio-uhd`**, which is hardware code. *Checked by `po_11_no_hashmap_and_no_wall_clock_in_simulation_code`, amended: `paced.rs` is exempt by name, and `kg_02_the_simulation_class_starts_no_thread` asserts that a Simulation Run never reaches it.*
- **GZ-5** No external package is added (PO-4). The default build of every crate needs no UHD installation; `ezsdr-radio-uhd/build.rs` does nothing unless `CARGO_FEATURE_UHD` is set, and then links `uhd` from `UHD_LIB_DIR`, else `pkg-config --variable=libdir uhd`, and fails with a message naming both when neither finds it. *Checked by `po_04_the_lock_gains_no_external_package` and by the default `cargo test --workspace`, which must pass on a machine without libuhd.*
- **GZ-6** Every new or amended rule with code has a test that fails when the rule's code is disabled, shown by a recorded mutation (spec 19 Appendix A, spec 18 §8), as PO-12 requires. *Process obligation; exit criterion 6.*
- **GZ-7** Spec 19's text reaches `design/` in the commit that implements it; spec 18 moves to `design/18-uhd-radio.md` at Step X; the Vision is not edited before Gate X (OV-6). *Process obligation.*
- **GZ-8** Hardware tests live only in `crates/ezsdr-radio-uhd/tests/hardware.rs`, behind `#![cfg(feature = "uhd")]`, each `#[ignore = "needs a USRP: see plan/phase7/bench.md"]` and reading the device from `EZSDR_UHD_ARGS`; they never run in `cargo test --workspace`. *Checked by `gz_08_hardware_tests_are_gated` (acceptance): a text scan of the file for the `cfg` line and an `ignore` on every `#[test]`.*
- **GZ-9** `FakeDevice` is constructed only by Rust code: no selector value, profile or environment section selects it, and `ezsdr_radio_uhd::open(args)` never returns one. *Checked by `ur_33_no_document_selects_the_fake_device`.*
- **GZ-10** The bench results are recorded in `plan/phase7/bench-results.md` (created at the bench session) with the commit, the UHD version, the device's `pp_string` and each step's outcome, and `plan/spikes/2026-09-26-uhd.md`'s table is filled from it. *Process obligation; exit criterion 9.*

## 7. Test strategy

- **Kernel** (`crates/ezsdr-kernel/tests/coordinator.rs`, `module_api.rs`, `time_model.rs`): the device-paced paths with two new doubles in `tests/support/run_doubles.rs` — `WallAuthority`, a `Pacing::Device` Authority on the host's monotonic clock (a 1 GHz root; `next_wakeup` sleeps until the earliest callback, waking early when one is scheduled, as spec 18 UR-7 does for a device), and `ThreadedProvider`, a non-stepped Provider with its own thread that drains its queue every millisecond (optionally sleeping in each Action, to exercise KC-21a's wait), publishes a block on its link every few milliseconds, and can emit `DEVICE_LOST` from its thread. Every KG rule has a named test in spec 19. The Simulation-class tests stay as they are; where one changes (KG-7's T0 rounding), `design-notes.md` records which and why (GY-3's rule applied to Phase 7).
- **Vocabulary and MockRadio**: `radio_model.rs` for the moved `device` module (the same cases MockRadio's `mr_06_*` covered, now against a description), the new payloads and schemas; `mock_radio.rs` for the lattice origins.
- **The UHD Module** (`crates/ezsdr-radio-uhd/tests/fake.rs`): every UR rule through the real coordinator with `FakeDevice` — the Authority, prepare's refusals, T0 start, receive blocks and gaps, overflow, alignment, late start, transmit bursts and repeat, the late policies, the command queue, `cold` changes, `Stop`, the reference monitor, a lost device, the Manifest sections. `FakeDevice` is scriptable (UR-33): it can stall, overflow, misalign, drop out, lose its reference and report transmit errors on command.
- **The UHD C API** (`crates/ezsdr-radio-uhd/tests/uhd_api.rs`, feature `uhd`, runs without a device): `find` returns without error; `open("addr=192.0.2.1")` (a TEST-NET address, RFC 5737) returns `Err` carrying UHD's text; the struct sizes the declarations assume equal the header's (`size_of` checks).
- **End to end** (`crates/ezsdr-acceptance/tests/uhd.rs`): the §59 rehearsal — the acceptance crate's own Specs and Sessions (`experiments::receive`, `transmit`, `with_timed_capture`, §57's loopback, v61_01/02/03's shapes) under a profile that differs from the Mock one only in the radio binding, the Authority and `ezsdr.rf_path`, run on `FakeDevice`; each asserts what its Mock counterpart asserts that a wall-clock device can reproduce (the termination, the capture's first sample and continuity, the burst records, the loopback content), and none asserts determinism.
- **Server** (`crates/ezsdr-server/tests/protocol.rs`): a profile naming `ezsdr.radio.uhd` without the `uhd` feature is `refused` naming the feature; `Config.default_profile` is used when `connect` names no profile, and the binary sets it from `EZSDR_PROFILE`; a `run_child` in a device-paced Session comes back as a rejected entry. The device-paced Session through the protocol is exercised by an in-process test embedding whose `Config.open_device` returns a `FakeDevice` (VE-5), so the catalogue's own assembly path runs.
- **Python**: `test_sleep_returns_the_instant` against the simulated default; the device-paced Python path is the bench's (B7).
- **Hardware** (`tests/hardware.rs`, `#[ignore]`, feature `uhd`): the same bodies as `tests/fake.rs`'s rehearsal cases, run on `UhdDevice`, plus the probes of `bench.md`.
- **Mutations**: spec 19 Appendix A and spec 18 §8 list them (GZ-6), run with Phase 6's `plan/phase6/tools/mutate.py` copied to `plan/phase7/tools/`.
- **Suite time**: every coordinator-driven UHD test on `FakeDevice` waits at least the profile's 2 s start-up (UR-15), so `tests/fake.rs`'s ~60 tests take about two minutes of wall time on one thread and well under one with cargo's parallel test threads; `kg_06_*` take about 10 s each (one host budget and one RS-8a deadline). Wall-clock assertions carry a factor-of-two margin and read counts while the Run runs, never exact timings.
- **The Kernel's token ban**: OV-23a fails the Kernel on the token `uhd` anywhere in its source, comments included (the spike hit it); the Kernel's text and code say "device-paced" and never name the Module.

## 8. Traceability

### Vision → Phase 7

| Vision | Test | Phase 7 carrier | Remaining |
|---|---|---|---|
| §59 | Mock experiments run on a USRP by changing the profile | `uhd_59_*` (acceptance, `FakeDevice`); bench B3–B7 (`hardware.rs`, X310) | the envelope measurement and the parity failure rule: Phase 8 |
| §58 #1 | Binding substitution | `uhd_59_one_spec_mock_and_uhd_profiles`: one Spec document under the Mock profile and the UHD profile, byte-identical Spec | on hardware: B3 |
| §58 #4 | RuntimeEvents flow through the same path used by hardware | `ur_20_rx_overflow_is_emitted_on_the_hot_path` (the UHD Provider's receive thread emits RM-24's bytes; the Manifest's payload decodes with `from_payload`) | — (now true end to end) |
| §58 #6 | An injected overflow shows the same gap as a UHD overflow | `ur_18_an_overrun_is_rm_17s_gap` (`FakeDevice` overrun: `GAP_BEFORE | RESTARTED`, `lost` = time jump, `RX_OVERFLOW`) beside the Mock's `v58_06_*` | on hardware: B5 |
| §58 #11 | The same typed `TIME_ERROR` as hardware | `ur_21_late_policies_on_the_device_lead`, `ur_28_the_device_reports_a_late_burst` (`late_at_device`) | on hardware: B8 |
| §58 #15 | TX bursts closed; repeat does not underflow at the wrap | `ur_22_repeat_is_continuous_across_the_wrap`, `ur_23_a_burst_ends_at_the_next_bursts_start` | on hardware: B6 |
| §58 #16 | Session Actions admitted on hardware alike | `uhd_59_a_session_retune_outside_the_rf_envelope_is_rejected` | on hardware: B7 |
| §61 v3 behaviours 1–3 | repeat across the wrap; capture at a sample index; timed TX and RX start | `uhd_61_01_*`, `uhd_61_02_*`, `uhd_61_03_*` (acceptance, `FakeDevice`) | on hardware: B4, B6 |
| §61 v3 behaviour 4 | several devices on 10 MHz + PPS | — | after Phase 8 (§3) |
| §35 | a narrow bridge; typed status; async errors as events; `DEVICE_LOST`; in-process | spec 18 UR-2…UR-4, UR-28, UR-29; `ur_29_a_lost_device_aborts_the_run` | — |
| §15, TM-16a1, TM-18 | the device timekeeper; its relation to UTC | `ur_07_*` (the Authority), `kg_11_a_device_paced_run_records_its_root_s_relations` | — |
| §32, MA-30 | Executors and Sinks on threads in HIL and Hardware | `kg_02_*` | per-Island threads: Phase 10 |
| §13 rule 2 | timing checked at the owning layer, Mock and hardware alike | `ur_21_*`, `ur_24_*` (the UHD Provider enforces its own lead and queue depth) | — |

### Earlier deferrals → Phase 7

| Item | Where it was deferred | Phase 7 disposition |
|---|---|---|
| Spike K1 | `plan/spikes/2026-09-26-uhd.md` | KG-1 |
| Spike K2, K5, K6, K8, K11 | Phase 4 §3 (Gate X) | KG-5, KG-4, KG-8 with VE-2, KG-2, VE-3 |
| Spike K3 | Phase 4 §3 (owner: Phase 7) | KG-7, VE-2, VE-4; the ceiling test is replaced by `kg_07_a_burst_at_t0_plus_n_samples_is_exact` |
| Spike K4, K7, K9, K10, K12, K13 | spike findings | §2's second table; K12 is KG-9 |
| MA-8 enforcement | Phase 2 Gate X named risk; Phase 4 §3 | KG-6 |
| `ALIGNMENT` producer (SC-31a); alignment and clock-loss fault kinds | Phase 4 §3 | UR-19 (no per-channel producer on UHD; whole-stream gap and `ALIGNMENT_ERROR`), UR-27 (`CLOCK_LOST`) |
| A wall-paced Run and K8 | Phase 6 §3 | KG-2, KG-5 |
| Capture timing on a wall-paced class | Phase 6 §3 | VE-6 |
| `run_child` on a wall-paced class (S5's ceiling) | Phase 6 §4, KC-37a | KG-12 |
| A remote listener, server-owned profiles, authentication, `body_bytes` (S9) | Phase 6 §3 | out of scope again (§3, owner decision §11) |
| Device refusals as Python exceptions | Phase 6 §3 | out of scope (§3) |
| The lab server's default profile (S7's ceiling) | Phase 6 §4 | VE-5 (`EZSDR_PROFILE`) |
| TM-20 (UHD seconds + fraction converted with explicit rounding at the Provider boundary) | spec 01, "tested in Phase 7" | UR-3: `UhdDevice` converts ticks to `(full_secs, frac_secs)` and back with the rules stated there; `ur_03_time_specs_round_trip` |
| RS-32's hot path for a hardware Provider (RM-11 "the Phase 7 Provider's concern") | spec 07 | UR-20 |
| Phase 2 P3-1: UC-5 `atomic_realtime`'s definite carrier "conditional on the UHD Provider being first" | Phase 2 Gate X | not carried: no UHD key is `atomic_realtime` (RM-4 declares none); stays with the first key that is |

## 9. Sequencing

| Step | What | Check |
|---|---|---|
| 0 | This file, specs 18 and 19, `bench.md`, `vision-issues.md` | links resolve; **Review J** (Opus) of the design, its fixes, and **Review K**, the re-review of the fixes (the owner's rule: a large fix is reviewed again); `design-notes.md` |
| P | **Gate P** (owner): the decisions of §4 and §11's open rows, specs 18 and 19 | — |
| 1 | KG-1…KG-14 (Kernel code, `design/` text, the two doubles) | workspace tests on 1.85.0 and stable; Clippy; `kernel_surface` 116 / 292; `schema_freeze` |
| 2 | VE-1…VE-4 (`radio` 1.3.0, MockRadio 1.3.0, the profiles that name them) | the same; the radio schema freeze; the K3 ceiling test replaced |
| 3 | Spec 18 without the feature: the crate, `FakeDevice`, the Authority, the Provider; `tests/fake.rs` | the same; `cargo test -p ezsdr-radio-uhd` |
| 4 | Spec 18's `uhd.rs` and `build.rs`; `tests/uhd_api.rs`; `tests/hardware.rs` compiled | `cargo test -p ezsdr-radio-uhd --features uhd` on this Mac (libuhd 4.10, no device); `cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` |
| 5 | VE-5, VE-6; `tests/uhd.rs` (acceptance) | the workspace tests; the Python suite (GY-7); `cargo build -p ezsdr-server --features uhd` |
| 6 | Appendix A's and spec 18 §8's mutations | each killed |
| — | **Review L** (Opus): the implementation, as Reviews H and I were for Phase 6 | findings triaged, recorded in §11 |
| B | **The bench session** (`bench.md`, owner, Linux PC with the X310) | B0–B8 pass; `bench-results.md` (GZ-10) |
| 7 | Exit tables, Vision issues collected, `handoff.md` | **Gate X** (owner) |
| X | Spec 18 moved to `design/`; Vision issues applied | links and `v3/` paths recheck |

## 10. Exit criteria

1. Specs 18 and 19 accepted at Gate P and ratified at Gate X, and every decision row — T1–T18 here, U1–U14 of spec 18, and the reviews' — has a verdict in §11.
2. Every Phase 7 rule — KG-1…KG-14, VE-1…VE-6, UR-1…UR-35, GZ-1…GZ-10 and each amended rule — has an OV-3 disposition in `plan/phase7/exit-review/`, read from test bodies (PO-10), with no `GAP` and no `UNCERTAIN`.
3. `cargo test --workspace` passes on Rust 1.85.0 and on stable with no `#[ignore]` and **without libuhd installed**; `cargo +stable clippy --workspace --all-targets -- -D warnings` passes.
4. `cargo test -p ezsdr-radio-uhd --features uhd` and its Clippy pass where libuhd 4.x is installed (this Mac, and the bench PC at B1).
5. Every carrier of §8 exists and passes; the Python suite passes on Python 3.9 and on a current Python (GY-7).
6. `kernel_surface` (116 NEW / 292), `schema_freeze`, every Vocabulary freeze test and the server's pass; every mutation of spec 19 Appendix A and spec 18 §8 is killed (GZ-6).
7. The Kernel's direct dependencies are still exactly four; `Cargo.lock` gains no external package (PO-4, GZ-5).
8. Every link in `design/` and `plan/` resolves, and the `v3/` path check of `handoff.md` §1 passes.
9. Bench steps B0–B8 pass on an X310 + one OBX (profile `x310-obx`) and are recorded (GZ-10) — T18, accepted at Gate P for an X310 + UBX; the owner moved the bench to one CBX in loopback and then to one OBX on 2026-09-30 ([`design-notes.md`](design-notes.md) §9, §10).

## 11. Decision log

| Decision | Gate | Verdict | Note |
|---|---|---|---|
| Design only, in detail; implementation after Gate P (T1) | — | **the owner's instruction** | 2026-09-27: "設計だけで止めてください．ただし，設計は詳しく設計してください", given during the work; the implementation begun for KG-4 was reverted uncommitted |
| Review J (design review, Opus): CHANGES_REQUIRED, 4 P0, 11 P1, 23 P2; every finding fixed in the documents or answered (`design-notes.md` §4) | — | **closed** | 2026-09-27; the fixes change the time rules, the concurrency rules and the transmit path, so they are re-reviewed (the owner's rule) |
| Review K (re-review of Review J's fixes, Opus): CHANGES_REQUIRED, 1 new P0, 5 P1, 14 P2, Review J's G09 test gap still open; all fixed or answered (`design-notes.md` §5) | — | **closed** | 2026-09-27; see §5 of `design-notes.md` for why no further review was run |
| T1–T18, specs 18 and 19 (with U1–U14) | P | **accepted** | owner, 2026-09-27, as recommended ("すべて推奨で受理します") |
| Multi-device and v61_04 on hardware: after Phase 8 (§3). Recommendation: after Phase 8, with TM-18's device-to-host relations | P | **after Phase 8, with the device-to-host relations** | owner, 2026-09-27, as recommended ("すべて推奨で受理します") |
| The USRP2 profile: when its daughterboard is known (§3). Recommendation: B9 records the `pp_string`; the profile follows | P | **after B9's `pp_string`** | owner, 2026-09-27, as recommended ("すべて推奨で受理します") |
| The remote listener, server-owned profiles, authentication and `body_bytes`, which Phase 6 assigned to Phase 7 (§3). Recommendation: a later phase of their own, before remote users need them | P | **a later phase of their own** | owner, 2026-09-27, as recommended ("すべて推奨で受理します") |
| Whether Gate X needs the bench (T18). Recommendation: yes, B0–B8 | P | **yes, B0–B8** | owner, 2026-09-27, as recommended ("すべて推奨で受理します") |
| Implementation | — | **not now** | owner, 2026-09-27: "実装はしないで"; §9's steps 1–6 wait for the owner's request |
| Implementation and its reviews: Reviews L–T (Opus), each finding fixed, answered or recorded (`design-notes.md` §7, §8, §12–§20) | — | **closed** | 2026-09-28 to 2026-10-01; the last, Review T, with no blockers and its P2s fixed (`1d3f382`) |
| The bench session (B0–B9; the owner at the X300, partly remote) | — | **done** | 2026-09-30 to 2026-10-01 on an X300 + one OBX (`bench-results.md`, Sessions 1–2, parts 1–11): B0–B8 pass (criterion 9); B9 run on the direct cable and through an L2 switch; the design changes it caused — F1–F3 (§11), F4 (§17), UR-7's lock refusal and reopen (§18) — each decided by the owner first |
| Gate X: specs 18 and 19 ratified, the implementation accepted | X | **accepted** | owner, 2026-10-01: "終わったらmainへmergeしてください．そしてGate Xを受理します". At acceptance, checked in the bench session: criteria 3, 4 and 6 (workspace 885 passed on stable and 1.85.0, `kernel_surface` and `schema_freeze` among them; `--features uhd` 171; clippy clean; mutations 160 rows, every live row killed when last run — the 63 on the files Reviews S and T changed rerun at the end, U07 and U43 rewritten for the new code) and 9 (B0–B8 pass); criterion 1's verdicts are this log and design-notes §11–§20; checked at acceptance on the bench PC: criterion 5 on a current Python only (`python3 -m unittest discover -s python/tests` against the debug server, Python 3.14.4: 23 passed; no Python 3.9 on that machine), 7 (`Cargo.lock`'s 27 registry packages identical to `main`'s; the Kernel's dependencies `serde`, `serde_json`, `schemars`, `sha2`), and 8's links (465 in `design/`, `plan/`, AGENTS.md and handoff.md, none broken); **not met or not checked: criterion 2 — the OV-3 disposition tables in `plan/phase7/exit-review/` were not produced —, criterion 5 on Python 3.9, and criterion 8's `v3/` paths (this clone has no `master` and no `v3/` worktree)** — accepted as they stand, recorded here; Step X below |
| Step X: spec 18 to `design/18-uhd-radio.md`, the three Vision issues applied (§35, §15, §32), the revision history row; issue 1 refined for the bench's F4 | X | **done** | owner, 2026-10-01: "StepXは実施" |
