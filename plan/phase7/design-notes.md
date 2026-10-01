# Phase 7 — design notes

What the design of [`00-overview.md`](00-overview.md), [spec 18](18-uhd-radio.md) and [spec 19](19-amendments.md) was checked against, what writing it found, and what Review J found and how each finding was answered. The design stopped short of code at first (the owner's instruction, 2026-09-27: "設計だけで止めてください．ただし，設計は詳しく設計してください"); the implementation followed the same day, and §6 records where it departs from the text.

## 1. The state the design starts from

- `main` at `7cc6147` (Phase 6 Step X), clean. `cargo test --workspace` on it: 684 passed, 0 failed — after `find crates schemas python Cargo.toml Cargo.lock -type f -exec touch {} +`. The first run reported 21 failures in the path-reading tests (`kernel_surface`, `governance`, `v58_10`): the repository had been moved to `~/work/Ez-SDRv4` with its files' timestamps kept, and cargo reused test binaries compiled at the old path, whose `CARGO_MANIFEST_DIR` no longer exists (AGENTS.md §7's mtime rule, met in the main tree rather than a copy). Nothing in the code was wrong. The implementation must start with the same `touch`.
- A first implementation step (KG-4's queue counters, in `coordinator/state.rs`, `admission.rs`, `ending.rs`, `pipeline.rs`) was begun and reverted with `git checkout` when the owner asked for the design only; nothing was committed.

## 2. What the design was checked against

| Claim | How it was checked |
|---|---|
| UHD's C API translates every exception into a status | read `/opt/homebrew/include/uhd/error.h:89-104` (UHD 4.10.0, `pkg-config --modversion uhd`): `UHD_SAFE_C` catches `uhd::exception`, `boost::exception`, `std::exception`, `...` |
| The receive and asynchronous metadata codes | `uhd/types/metadata.h:80-92`, `:232-244` |
| A late timed command is activated on arrival; a full command queue back-pressures | `uhd/usrp/multi_usrp.hpp`, the comment of `set_command_time` |
| `set_time_unknown_pps` takes at most 2 s; set the time after the clock source | `multi_usrp.hpp`, the comment of `set_time_unknown_pps` |
| `ref_locked` can be read from C | `uhd/usrp/usrp.h:448` (`uhd_usrp_get_mboard_sensor`), `uhd/types/sensors.h:121` (`uhd_sensor_value_to_bool`) |
| KC-2 refuses every class but Simulation | `coordinator/pipeline.rs:372-380` |
| A round runs only inside `RunHandle` calls | `coordinator/stepping.rs:119` (`round`), called from `run_loop`, `submit` (`pipeline.rs:1374`), the start (`pipeline.rs` end of `pipeline()`) and `drain_cleanup` |
| `None` from `next_wakeup` completes a Spec Run | `stepping.rs:352` |
| `prepare` and `arm` are called directly; `collect_prepare` locks every slot | `pipeline.rs:763-771`, `:842-850`; `with_inputs`, `pipeline.rs:1131-1137` |
| T0 is `now + lead` with no rounding | `pipeline.rs:867-892` (`set_start_instant`) |
| `relations` is always empty | `coordinator/ending.rs:258` |
| The Kernel's queue has no notion of a finished Action | `coordinator/state.rs:30-37` |
| A Module may emit `DEVICE_LOST` for its own source root | `event.rs:544-551` (`emit_control` checks the registered pair, not the owner); KC-8 registers `(source root, DEVICE_LOST)` for every instance |
| `EventSink::emit` is callable from any thread | `event.rs:503-541` (a mutex-guarded ring) |
| `ClockRegistry::end` accepts a future instant | `time/domain.rs:302-326` (it checks the root and a second end only) |
| PO-11 forbids threads and the wall clock in the coordinator; PO-2 forbids `unsafe` in every crate | `crates/ezsdr-acceptance/tests/governance.rs` (`po_11_…`, `po_02_…`) |
| SB-22c already settles spike K4 | `design/03-spec-and-binding.md:180` |
| MockRadio's values and their sources | `design/09-mock-radio.md` MR-3 and §2; `crates/ezsdr-mock-radio/src/profile.rs` |
| RM-8's coercion lives only in MockRadio | `crates/ezsdr-mock-radio/src/coerce.rs`; `grep -rn "fn coerce" crates` |
| The spike's code and findings | `git show origin/spike/uhd:spike/uhd/src/{ffi,device,authority,provider}.rs`; `plan/spikes/2026-09-26-uhd.md` |

## 3. What writing the design found

Beyond the spike's findings and the items earlier phases handed over, writing the rules found four holes and one defect of an earlier draft:

1. **A Provider thread could not report a lost device** (overview §2, hole 10): KC-30 only turns a returned `DeviceLost` into the event. → KG-10 (MA-9a).
2. **No hardware Run could record TM-18's relations** (hole 11): the Manifest's `relations` is always empty and no interface carries a relation. → KG-11, which first returned only the UTC relation and was widened to both relations TM-18 and Vision §15 ask the device timekeeper for.
3. **UC-3 forbids what a device does on a `cold` change** (hole 18). → KG-13.
4. **UC-6 had K6's defect as well as RS-14**: a `hardware_timed` update's lateness was judged against the whole lead, delivery included. → KG-8 amends UC-6 beside MA-10.
5. **KG-6's first draft deadlocked**: after abandoning a `prepare` that did not return, it went on to `collect_prepare`, which locks every instance's slot (`with_inputs`) and would wait for the abandoned call forever. → KC-12a now fails the stage without calling it.

Two consequences of hardware that the Radio Model did not state were written down rather than hidden: a transmitter cannot recall samples it has handed to the device, so a stopped burst ends within a stated in-flight window (RM-16 amendment, UR-23); and a Session `cold` change must be booked before its device switch (UR-25), which `ClockRegistry::end`'s acceptance of a future instant allows.

## 4. Review J

Brief: [`prompts/review-j.txt`](prompts/review-j.txt). Report: [`reviews/review-j.md`](reviews/review-j.md). Claude Opus, read-only, 2026-09-27, with two probes in a scratch copy: KG-7's T0 rounding moves no existing test's T0 except the K3 ceiling test's (683 of 684 pass, the 684th being the ceiling that KG-7 replaces), and `Authority::relations` fails `ma_06` until `ClockRelation` joins its list. It confirmed all 18 hole-evidence cells and all 66 C functions and 4 structs of UR-3.

**Verdict: CHANGES_REQUIRED** — 4 P0, 11 P1, 23 P2. Every finding was accepted; none was rejected. Each disposition names where the documents now carry the fix.

| Finding | Disposition |
|---|---|
| **P0-1** The wake at `now`, `advance_to` and UR-7 schedule at or before a moving `now`, which TM-16c's `InPast` forbids | KG-9 amends TM-16c: a paced Authority refuses only an instant before its last fired one, and an accepted instant already passed fires at the next `next_wakeup`; `WallAuthority` (§0), UR-7; tests `tm_16c_a_paced_authority_accepts_an_instant_already_passed`, `ur_07_schedule_accepts_an_instant_already_passed`; mutations G33, U33 |
| **P0-2** A 10 ms in-flight window makes "a burst ends at the next burst's start" impossible for a preemption inside it; RM-15 unamended; the test and the fake hide it | VE-2 amends RM-15 (the preemption bound: the later of `now + device lead` and the first unhanded sample, with the burst's `LatePolicy`); UR-21, UR-23; the fake's transmit queue drops a late start-of-burst (UR-33); tests `ur_21_a_burst_inside_the_in_flight_window_is_late`, `ur_23_a_burst_ends_at_the_next_bursts_start` at 20 ms; the difference from MockRadio is a Phase 8 input (overview §3); decision U7 revised |
| **P0-3** Enabling a stream at runtime configures nothing on the device | "Configuring a direction" defined once in UR-12 and required on enabling or raising a count in UR-25; the fake loops back only between equal frequencies (UR-33); tests `ur_25_enabling_tx_applies_the_configuration`; B1 prints the transmit settings found; decision U14; mutation U39 |
| **P0-4 a** TM-13b vs UR-17's missed start | KG-14 amends TM-13b (the origin stays at T0; the first block `GAP_BEFORE` with `lost` its index) |
| **P0-4 b** TM-13e vs RM-25 and UR-25 | KG-14 amends TM-13e (the lattice; KG-13's restart instant) |
| **P0-4 c** RM-16's cancellation vs UHD's unrecallable commands | UR-24 holds timed commands host-side (P1-6), so a `Stop` cancels what is held; VE-2 amends RM-16 for the commands already released within the release window; test `ur_24_stop_cancels_held_commands` |
| **P0-4 d** MockRadio and the UHD Provider followed two cold-change rules | VE-2's RM-25 states one rule (`e₁` on the old lattice, `e₂` on the new lattice at or after `e₁` plus the restart lead); VE-4's MR-18 is its zero-lead case, with a test case where `e₁ ≠ e₂`; UR-25 cites it; mutation V13 |
| **P1-1** Lost wakeup: step 1 cancels a pending wake without clearing its flag, and nothing says a `cancel` wakes a waiting `next_wakeup` | KC-46a keeps the wake handle apart and step 1 cancels and clears it; KC-46b's wake is unconditional; MA-29 as KG-3 amends it (a paced `cancel` wakes the wait); UR-7, `WallAuthority`; tests `kg_03_a_cancelled_horizon_wakes_the_control_loop`, `ur_07_cancel_wakes_a_waiting_next_wakeup`; mutations G31, U34 |
| **P1-2** Admission and dispatch no longer serialized; KC-21a's "no control call is running" false | KC-24a, the admission lock (held from admission through dispatch, by a Module's submission and by the freeze; never across a round or the wait); KC-21a's sentence corrected; test `kg_04_no_action_is_dispatched_after_the_freeze`; mutation G29 |
| **P1-3** A Detached Lease's expiry is not enforced while no call runs | KC-36 as KG-2 amends it (the deadline mirrored in `Shared`, checked by the data thread, acted on by KC-46b); overview hole 19; test `kg_02_a_detached_lease_expires_while_no_call_runs`; mutation G30 |
| **P1-4** The Authority is not re-anchored while no call runs | UR-7's `uhd-clock` re-anchors every 100 ms; the fake has a drift (UR-33); test `ur_07_now_tracks_the_device_while_no_call_runs`; decision U4 revised; mutation U35 |
| **P1-5** VE-6's promise ("captures from the instant the sleep ended") is false on hardware | VE-6 rewritten: EA-17 states when a capture starts at `at`; `status` carries `root_rate` and `Session.after(seconds)` names an instant ahead; server tests on the fake device (`ea_17_*`) and B7 records the INFERRED case |
| **P1-6** Timed commands issued at receipt meet UHD's in-order, back-pressuring queue | UR-24's holding and release (3 ms release window; a command before one already released is applied late), queue depth counted over held and released commands; the fake's command queue (UR-33); B8 measures the device queue's capacity; test `ur_24_a_far_future_update_does_not_delay_a_nearer_one`; decision U13; mutation U37 |
| **P1-7** No thread named for UR-25's device switch | UR-14 (uhd-control books and never waits for a device instant), UR-25 (uhd-rx and uhd-tx perform the switches; UR-12's exact-rate rule at the switch), UR-3 (each streamer's owning thread, reopen included; the async reports moved into uhd-tx); tests `ur_14_the_control_thread_never_waits_for_a_device_instant`, `ur_25_a_rate_the_device_does_not_apply_at_the_switch_is_rejected`; decision U2 revised; mutations U40, U41 |
| **P1-8** `cleanup` may free a streamer a detached thread is still inside | UR-16 and UR-26: no `close_streams` for a streamer a detached thread owns; UR-3's `Arc`-held handles; test `ur_16_cleanup_keeps_the_streamer_of_a_detached_thread`; mutation U42 |
| **P1-9** KG-11 fails `ma_06` and makes MA-46's list false | KG-11 amends MA-46's list and `ma6_documents!`; GZ-2 records it |
| **P1-10** A failed instance is re-stepped on every pass; `DEVICE_LOST` repeats | KC-46: a failed instance is not stepped again in the Run; KC-46b: under `abort` the data thread stops stepping; tests `kg_02_a_failed_sink_is_not_stepped_again`, `kg_03_an_abort_stops_the_data_thread`; mutations G32, G35 |
| **P1-11** Two thread-id tests fail on a correct implementation; the capacity-4 test is flaky | the thread-id tests restricted to steps before `finish`; the drain test uses capacity 64 and counts read while the Run runs |
| P2-1 `ServerConfig::assembler` vs `Config.open_device` | overview §7 now says `Config.open_device` |
| P2-2 `EZSDR_PROFILE` read in the library races in-process tests | `Config.default_profile`, set by the binary; tests `ea_09_the_configured_default_profile_is_used`, `ea_09_the_binary_reads_ezsdr_profile` |
| P2-3 fired wake handles accumulate | the wake handle kept apart from `scheduled` (KC-46a) |
| P2-4 KG-7's move of T0 unbounded in the text | KC-15 states the bound (less than `L` root ticks) |
| P2-5 the reason against `step_until_quiescent` overstated | KC-46 and T5 give the reason as time moving between passes, and the cap at high block rates |
| P2-6 KC-29's "no end request left pending" now false | KC-29 amended in KG-3 |
| P2-7 MA-20's "nothing after `until`" | MA-20 amended in KG-2 |
| P2-8 KC-21a's early exit needs a notification or a poll | KC-21a re-checks every 10 ms; `clear()` notifies |
| P2-9 the stopper thread is unnecessary | removed: the data thread performs steps 1–2 itself (KG-3, T9) |
| P2-10 the reaction latency is bounded by the slowest step | stated in KC-46 and KC-46b |
| P2-11 `ClockReference::Time` has no producer | dropped; additive when a Provider monitors its PPS (VE-3) |
| P2-12 a transmit-only Run never finds a lost device; `RUNTIME` is generic | UR-29: uhd-control's time read every 500 ms, `RUNTIME` counted only on streaming calls; B9 covers both; test `ur_29_a_lost_device_is_found_while_idle`; mutation U43 |
| P2-13 `N` from a float division | UR-12: `N = round(mcr / rate)`, verified exactly |
| P2-14 a trim can leave an empty block | UR-17: dropped; test `ur_17_an_overlap_trimmed_to_nothing_is_dropped` |
| P2-15 the burst record's host view | UR-30 says so |
| P2-16 the command-time bracket is shared state | UR-3's control-call mutex |
| P2-17 a client dying during a long wait | a ceiling of VE-5 |
| P2-18 the lead difference between the profiles | overview §3, "Phase 8 inputs" |
| P2-19 the UTC exclusion only in `method` | TM-18 and UR-8 state it |
| P2-20 the rpath | removed (UR-35); B0 says how a `/usr/local` build runs |
| P2-21 OV-23a's token ban | overview §7 |
| P2-22 bench: the USRP2 probe, the Session profile, B5's stall | B9's own probe, the full Session profile written out, B5's stall lengthened and the buffer sizes recorded |
| P2-23 the suite's wall time | overview §7 |
| TEST_GAPS: G07, G09, G12, G16, G18, U13, U16, U19, U22 | G07 → `kg_02_a_sink_that_always_progresses_does_not_livelock`; G09 → `kg_02_the_first_delivered_stopping_event_is_the_termination_cause`; G12 → `kg_03_step_3_runs_a_final_round_over_the_sinks`; G16's mechanism named (`mutate.py`'s timeout); G18 withdrawn as equivalent (KG-5's note); U13 with the fake's 20 ms `apply`; U16 through the fake's restart record; U19 with a 20 ms margin; U22 → `ur_25_a_rate_change_admits_a_burst_on_the_new_clock`; the missing tests of the list added (the Hardware class: `kg_01_the_hardware_class_reaches_running`, G34) |

## 5. Review K

Brief: [`prompts/review-k.txt`](prompts/review-k.txt). Report: [`reviews/review-k.md`](reviews/review-k.md). The same Opus reviewer, resuming its Review J session with the fix delta, 2026-09-27, read-only (one probe: a Python `Fraction` computation).

**Verdict: CHANGES_REQUIRED** — 1 new P0, 5 new P1, 14 new P2. Of Review J's findings every P0, P1 and P2 was closed; one test gap (G09) was still open. The new P0 was a regression of the Review J revision: making every `cold` change wait a restart lead broke enabling transmit at runtime, the path of every Phase 6 snippet. Every finding was accepted.

| Finding | Disposition |
|---|---|
| **N-P0-1** Enabling a stream from 0 channels now waits the restart lead on a thread that is not running; the next burst is admitted before its clock's origin; no rule covers it | UC-3 (KG-13) and RM-25 (VE-2): a change from 0 channels needs no restart lead; UR-25: uhd-control opens the streamer and configures the direction itself while booking, registers the clock after the configuration, and hands the streamer to its owner, which UR-15 now starts idle for both directions; RM-15 and UR-21: a burst never starts before its clock's origin (a later target is late under its `LatePolicy`); tests `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time` (transmission at the admitted instant, no `TIME_ERROR`), `ur_21_a_burst_before_its_clock_s_origin_is_late`; the X310's restart after a rate change is a Phase 8 input; mutations U45, U46 |
| **N-P1-1** A check-then-store race in the pending wake can suppress every later wake | KC-46a: pending wakes counted by generation, stored before `schedule`, cleared by the callback only for its own generation; MA-29: a paced Authority runs callbacks without a lock `schedule` or `cancel` takes; test `kg_02_every_delivery_wakes_a_waiting_call`; mutation G36 |
| **N-P1-2** The preemption bound races uhd-tx's hand-over | RM-15 and UR-21: a preempting burst is decided again by uhd-tx when it takes it, against the first sample only uhd-tx advances; the test's outcome no longer depends on timing; mutation U44 (its kill stays probable, not certain, and is recorded so) |
| **N-P1-3** `Session.after`'s test contradicts its exact arithmetic, and V11 survives | EA-16: the decimal the caller wrote (`Fraction(repr(seconds))`); the test adds 1.5 ns on a 1 GHz root, whose ceiling (2) and floor (1) differ, so V11 dies |
| **N-P1-4** `kg_04_no_action_is_dispatched_after_the_freeze` asserts what a correct implementation violates, and kills G29 only by chance | rewritten as a deterministic test: a gating runtime admission check holds the admission inside the lock while the data thread tries to freeze, and a Provider that drains after `stop` catches an Action pushed after the clear; an `Admitted` entry cleared by the freeze is correct and no longer counted against the implementation |
| **N-P1-5** `ur_25_enabling_tx_applies_the_configuration` required the opposite order to UR-25's | UR-25's from-zero order is now configure, then register, which the test asserts |
| N-P2-1 G06 named a renamed test | corrected |
| N-P2-2 the Lease deadline's unit | KC-36: the host clock's monotonic milliseconds, as `Lease` keeps it |
| N-P2-3 MA-29's "returns the earliest instant still scheduled" | reworded: it re-evaluates, waiting on or returning `None` |
| N-P2-4 the data thread inside a Provider's `stop` steps nothing | stated as the trade-off in KC-46b |
| N-P2-5 the control-call mutex skews an anchor's bracket | UR-3: the brackets are taken inside the mutex; UR-7: a bracket wider than 1 ms is discarded |
| N-P2-6 a 20 µs tolerance is flaky | 100 µs |
| N-P2-7 TM-13e and UC-3 worded differently from RM-25 | both restated in RM-25's terms (`e₁`, `e₂`), generically in the Kernel's text |
| N-P2-8 a command already in the release window at booking | UR-14: released before the next `recv()` |
| N-P2-9 which configuration "configuring" applies | UR-25: the one in effect at `e₂`; later held commands stay held |
| N-P2-10 a missed start at `e₂` | UR-17: handled as at T0 |
| N-P2-11 a latent lock cycle with a Provider that submits from its own thread | a ceiling of KC-24a (no Phase 7 Provider submits) |
| N-P2-12 timed stream commands outside the release discipline | UR-24: they go through the same list; UR-3: under the control-call mutex; B8 checks the shared queue (INFERRED) |
| N-P2-13 the MockRadio test's instant precedes T0 under `x310-like` | the test names the `ideal` profile |
| N-P2-14 the overview's server bullet | `Config.default_profile` |
| TEST_GAPS: G09, G31, G29, V11, U39, the preemption test, the from-zero test, the wake race, the tolerance, G33 | G09: the termination-cause test now drains on both threads at once; G31 withdrawn as unobservable (KC-46a no longer asks step 1 to clear the generation); G29, V11, U39, the preemption test, the from-zero test and the tolerance as above; the wake race's test added; G33 names the UHD Authority as the rule's real-code carrier |

**No further review.** The owner's rule is to review again after a large fix and to stop after a small one. Review J's fixes were large (the time rules, the concurrency rules, the transmit path) and were re-reviewed. Review K's are small: each is the fix the reviewer recommended, confined to the sentences it names — the from-zero case of three rules (UC-3, RM-25, UR-25) restored to the first draft's immediate start with the configuration Review J asked for, one lower bound in RM-15 and UR-21, a generation counter in KC-46a, one decision moved from uhd-control to uhd-tx, and test and wording corrections. The from-zero path, the one place the Review J revision regressed, was checked again by walking Python `tx.repeat`'s two calls through KC-24, KC-21a, UR-25 and UR-21 against the fixed text; nothing else in the design changed. The implementation's own review (Review L, overview §9) will read the whole design again against running code.

## 6. The implementation

The owner asked for it on 2026-09-27 ("Phase 7を実装してください"). It followed §9's steps 1–6 in four commits on the branch `worktree-phase7-impl` (steps 1–2, 3–4, 5, 6); Review L, the bench session and Gate X are the owner's next steps. What the code does differently from the text, and why, is below; each row is a question for Review L, not a settled change of the design.

### 6.1 Deviations from the specs

Review L judged each row (§7, DEVIATIONS); the specs now say what the code does where it asked for a spec change.

| Where | The spec says | The code does | Why |
|---|---|---|---|
| KG-8 (`ProviderInstance::min_command_lead`) | the field's documentation carries KG-8's sentence | the rustdoc is unchanged and the sentence is a `//` comment beside it | the rustdoc is part of the Kernel's generated schema, which `schema_freeze` holds byte for byte (GZ-2: no Kernel schema changes in Phase 7) |
| Spec 19's test locations | the KG tests in the Kernel's `tests/coordinator.rs`-style files | `crates/ezsdr-kernel/tests/device_paced.rs` with the doubles in `tests/support/paced.rs`; RM-26's tests in `crates/ezsdr-radio/tests/device.rs` | one file per rule family; the names are the spec's |
| VE-4 and MockRadio's `mr_32_*` tests | — | two `mr_32` tests expect the post-change burst at index 2, not 3 | the new transmit origin moved onto its lattice (RM-25), which moves the first burst index; the tests' instants are the spec's |
| PO-8's dev-dependency allow-list | the Kernel only | `ezsdr-radio-uhd` may also use `ezsdr-sink`, `ezsdr-sink-capture` and `ezsdr-link-host` in its tests | `tests/fake.rs` assembles a Run as the server's catalogue does (spec 18 §6); workspace crates only (`governance.rs` records it) |
| UR-24, the timed stream commands | a stop at `e₁`, a start at `e₂` and a restart go through uhd-control's release list | uhd-rx issues them itself, under the control-call mutex, when it performs the switch or the restart | *ponytail*: the list would need a message path from the owning thread back to uhd-control; whether the device's queue orders stream commands with tunes is INFERRED and B8 measures it. If B8 shows a retune queued behind a stream command, route them through the list |
| UR-3, the anchor's brackets | taken inside the control-call mutex | `DeviceAuthority::read` brackets `Device::time_now`, which takes the mutex inside | the `Device` trait has no "bracketed read"; UR-7's rule (a bracket wider than 1 ms is discarded) drops a read that waited for the mutex |
| UR-7, failed time reads | counted | counted in `DeviceAuthority::failed_reads()`, not written to the Manifest | an Authority has no Manifest section in the Kernel's API (MA-29); the bench prints it |
| UR-32, the receive buffer | reused | `UhdDevice::rx_recv` allocates per call (`ponytail:` comment) | the trait returns owned samples; Phase 8's copy-regression benchmark settles it |
| UR-21, a burst at the open burst's next sample | refused on uhd-control | decided by uhd-tx's preemption (RM-15 as VE-2 amends it), like any burst inside the open one | one decision point, the one Review K's N-P1-2 moved to uhd-tx |
| UR-34, `with_rx_stall(after, …)` | `after` the start | `after` the first received block | the start is two seconds before the first block (UR-15's start-up), so "after the start" would stall before any sample flowed |
| Spec 18 §6, `tests/fake.rs` | the tests through the coordinator | also a `Direct` harness that drives the Provider without a coordinator, for `ur_24_hardware_timed_updates` and `ur_25_a_cold_change_the_envelope_refuses_changes_nothing` | the coordinator's admission (KC-24) refuses an out-of-envelope change, and moves a late instant, before the Provider sees it; the harness reaches the Provider's own checks |
| EA-14 (VE-5), `catalogue::empty_assembly` | an Assembly with no Module object | no Provider, Sink, Executor or Link, and no device; its Authority is a Simulation Engine | `Assembly.authority` is not optional; the Kernel refuses the child before reading it (KG-12) |
| EA-7 (VE-5), `assemble` | — | takes `open_device` as a fourth argument; `Config::new(runs_dir)` builds a Config with nothing else set | the acceptance rig and a test embedding pass their own opener; `Config` gained two fields |
| The server and acceptance tests on the fake device | — | their profiles use a 2 s start lead | UR-15's start-up latency of the `x310-ubx` profile; each such test waits 2 s of wall time |
| Spec 19's text as applied to `design/` (an Opus subagent applied it) | RM-1: "four payload types" | three payload types and one outcome value | what VE-3 adds; the other findings of that pass — RM-26 given its rule prefix, RM-11's source rule silent on the new kinds, design/14 NX-4's KC-2 reason now stale, KC-20 and KC-45 made consistent — are Review L's to confirm |

### 6.2 The mutations (GZ-6)

`plan/phase7/tools/mutate.py` is Phase 6's tool with `.claude` left out of the copy and two additions: a mutation may name a second edit in the same file (`old2`, `new2`), which G09 needs (the `delivered` lock taken after the reactions); and a timeout kills the test's whole process group — Phase 6's `subprocess.run` killed only cargo, and G16's hung mutant kept spinning through the rest of the first run. `plan/phase7/tools/mutations.json` holds Appendix A's 34 live rows (G18 and G31 are withdrawn) and spec 18 §8's 46.

Where a row could not be written as the table words it:

- **G11, G12, G29** live in `ending.rs`, `stepping.rs` and `ending.rs` (the table names other files): `StopTx`'s record, the final round of `drain_cleanup`, and `FreezeDispatch` are there.
- **G36**: "the handle is stored after `schedule` returns" is read as the pending generation being stored then (the handle itself is only `FreezeDispatch`'s to cancel, and is stored after `schedule` in the implementation too).
- **U07**: the clock-before-time order lives in `uhd.rs`'s two C calls, which only the bench reaches; the mutation reverses the Authority's order of `set_sources` and `set_time_zero`, which the killing test checks.
- **U18**: held bursts are keyed by their start, so "arrival order" is written as taking the last key first, which for the test's two bursts (the later target booked first) is their arrival order.
- **U22**: publishing the new clock at the switch would take an edit in two files; the mutation withholds it from the transmit path at receipt, which the killing test fails at the same assertion (the burst on the new clock is refused before the switch).
- **U44**: the preemption is not decided again on uhd-tx (uhd-control's booking stands). It is killed deterministically here, where the spec expected only a probable kill.

**Results** (2026-09-27; the whole list once, then the rows it did not kill again after the fixes below): **91 of 93 killed**; G16 by `mutate.py`'s 900 s timeout, as Appendix A intends.

The first run left 13 rows not killed. Each was either a mutation that did not say what its row says, or a test that could not see the change:

| Row | Why it survived the first run | Fix |
|---|---|---|
| G07 | the mutation looped one instance with no cap and no `STEP_LIVELOCK`, which the test does not look for; the row names the Kernel's `step_until_quiescent` | the mutation calls `step_until_quiescent` for the Sink, as the row says: killed |
| G29 | a push after the freeze is cleared again by the control thread's own RS-6 step 1 about 100 µs later, and `draining_after_stop` polled every 1 ms | the double spins (`yield_now`) instead: killed |
| G36 | the mutation was half of the row (the callback's unconditional clear alone never loses a wake) | the row's whole mutant, the generation also stored after `schedule` returns: **still survives**, below |
| V01 | RM-8's tie case goes through the `Step` grid, and the mutation was in the `Values` arm | the mutation rounds the `Step` grid's tie up: killed |
| V08 | the mutation did not compile (a guard made the `match` non-exhaustive) | the default profile replaced by `None`: killed |
| U05, U34 | a missed wake delays the waiter by at most one 20 ms nap, inside the tests' 200 ms and 100 ms bounds | both tests measure the median lateness of 20 tries (under 4 ms and 3 ms; the mutants give about 13 ms): killed |
| U21 | `ur_24_hardware_timed_updates` counted the `LATE_COMMAND` but not where the late update was applied | it checks the applied instant against the event's `applied` and the device lead: killed |
| U22 | `ur_25_a_rate_change_admits_a_burst_on_the_new_clock` checked the Kernel's admission only | it waits past the switch and checks the burst went out on the new clock with no `COMMAND_REJECTED`: killed |
| U32 | the test compared the capture's end with `Provider::stop`'s instant, before uhd-rx's own stop | it checks the 1 ms tail after uhd-rx's `rx_stop`: killed |
| U37 | through the coordinator the nearer update reaches the Provider first, so the far one never held it | the test pushes the far one first through the `Direct` harness: killed |
| U41 | the test timed the second call, and the mutant blocks the first | it times both calls against 30 ms (the switch is 50 ms away): killed |

**Two rows survive: G09 and G36.** Both are races between the data thread and the control thread whose window is a few microseconds: G09's between `drain` and the `delivered` lock in two concurrent `drain_and_react` calls, G36's between `schedule` returning and the generation being stored (the control thread must run the no-op before the data thread stores). The tests (`kg_02_the_first_delivered_stopping_event_is_the_termination_cause`, 50 Sessions; `kg_02_every_delivery_wakes_a_waiting_call`, 50 wakes) exercise the paths but do not win those races on this Mac. A deterministic kill needs a pause point inside the Kernel (a `testing`-feature hook between the two steps), which would be new Kernel code for a test; left to Review L to decide.

## 7. Review L

Brief: [`prompts/review-l.txt`](prompts/review-l.txt). Report: [`reviews/review-l.md`](reviews/review-l.md). An Opus subagent, 2026-09-28, read-only, probes in scratch copies.

**Verdict: CHANGES_REQUIRED** — 3 P0, 8 P1, 19 P2. The Kernel half held; the UHD half had three defects the bench would have hit first. Every finding was accepted.

| Finding | Disposition |
|---|---|
| **P0-1** `rx_open`/`tx_open` freed the streamer they stored (`..stream` copied a `Drop` value) | `RxStream::make`/`TxStream::make` own the handles from creation and are moved into the `Arc`; `uhd_api_a_streamer_is_freed_once` runs the same path without a device (the copying version aborts it with SIGTRAP, checked on a probe copy) |
| **P0-2** UR-12's exact-arithmetic rule refused 487 of UR-9's 512 rates | UR-12 amended to MockRadio's rule; `decimation` replaces `exact_decimation`; `ur_12_every_advertised_rate_is_accepted` (all 512, 200 MHz/3 through `prepare`, 200 MHz/7 through a cold switch); mutation R11 |
| **P0-3** a receive stream enabled from 0 ignored its instant | `lattice(e.max(now + 50 ms), n)`; UR-25 amended; `ur_25_rx_channels_from_zero_starts_at_its_instant` (through the `Direct` harness, which gained links and the clock registry); R04 |
| **P1-1** the timed receive stop was issued at booking | uhd-rx holds it until `e₁` is a restart lead away, then issues it and records it in `applied`; UR-24 reworded (the stream commands stay off uhd-control's list, INFERRED ordering left to B8); `ur_25_a_timed_receive_stop_is_released_a_restart_lead_ahead`; R12 |
| **P1-2** UHD's global error string | per-handle texts (`uhd_usrp_last_error` under the control mutex, the streamers' own); the global one only for handle-less calls; UR-3 amended |
| **P1-3** no untimed fallback at `e₁`; reopen on every switch; new streamer before the old one released | all three fixed; `FakeFault::IgnoresTimedStop`; `ur_25_a_device_that_ignores_the_timed_stop_is_stopped_at_e1` (both with and without the fault: one `rx_open`, an untimed stop only when the timed one was ignored, a capture with no gap); R13, R14 |
| **P1-4** a preempted burst could end without end-of-burst | `end_open(true)` always; `FakeDevice::unended_bursts()` counts a timed start-of-burst inside an un-ended burst; asserted zero in `ur_23_a_burst_ends_at_the_next_bursts_start` and `ur_21_a_burst_inside_the_in_flight_window_is_late`; R06 |
| **P1-5** KC-21a's "every Action" unguarded | `kg_04_every_action_of_one_call_is_finished` (two schedule entries at T0, 30 ms each); R01 |
| **P1-6** G36 killable without a Kernel hook | `WallAuthority::firing_before_schedule_returns` and `kg_02_a_wake_that_fires_before_schedule_returns_still_clears`; G36's row now names it |
| **P1-7** eight obligations with no killing test | L07: `ur_24_the_device_queue_order_is_the_effective_order` (R03); L08: P0-3's test (R04); L09: `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` (R05); L11: `ur_25_a_cold_transmit_change_cancels_the_held_bursts` (R07); L12: `ur_26_an_abort_publishes_nothing_after_the_stop_instant` (R08), which also needed NONBLOCKING 9's fix; L14: `ur_29_a_silent_stream_is_a_lost_device` bounds `DEVICE_LOST` to 1–2 s after the silence (R10); L05: `FakeFault::SlowTimeRead`, `DeviceAuthority::discarded_reads()`, `ur_07_a_read_bracketed_wider_than_1_ms_is_discarded` (R02); L13: `ea_07_the_uhd_authority_takes_the_binding_s_sources` (R09). L04 (a Module's submission under KC-24a's lock) is left untested: no Phase 7 Module submits |
| **P1-8** `rehearsal_b7_session_loopback` flaky under load | on the fake the check is spike K6's regression — no `late_at_device` and no move of more than one device lead; the hardware run keeps "no `TIME_ERROR` at all" |
| N-1 wake handle stored after `schedule` | stored under a lock held across `schedule` (KC-46a amended) |
| N-2 the pass checks `done` before the slot lock | not changed: re-checking under the slot lock would take `done` inside a slot, the reverse of RestoreBaseline's order, and the case needs RS-8a to have abandoned step 3; recorded as a ceiling |
| N-3 KC-46b's unconditional wake is equivalent | kept, and KC-46b says why it is redundant |
| N-4 ignored results | `clear_command_time`'s result checked; `channels()` and the metadata getters left (a ceiling: a failed getter reads as 0) |
| N-5 `BROKEN_CHAIN` as lost | not lost; UR-29 amended |
| N-6 UR-24's depth per update | one per channel; UR-24 amended |
| N-7 `host.monotonic` floors | refused as `Inexact` (TM-16c) |
| N-8 a from-zero transmit enable with a passed `at` | `LATE_COMMAND`; UR-25 amended |
| N-9 the receive tail counted from uhd-rx's shutdown | uhd-rx is told the stop instant as `stop` begins (`RxCmd::Cut`), and a stop that arrives during a receive wait cuts that block; UR-26 amended; R15, R16 |
| N-10 a confusing message | reworded (`"UR-12: <rate> S/s is no decimation 1…512 of <mcr> Hz"`) |
| N-11 cleanup closes both streamers or neither | a ceiling (conservative; the `Device` trait closes both) |
| N-12 the fake never sends a partial count | a ceiling of the fake; the partial path is reached through `TxBlocks` |
| N-13 U07 guards another claim | recorded (§6.2); the order inside `uhd.rs::set_sources` waits for the bench |
| N-14 a test name in KG-9 | `tm_16_authority_order_and_now` |
| N-15 NX-4's stale reason | design/14 reworded |
| N-16 the 6 s bound | 3 s, as UR-16 says |
| N-17 a blank line | removed |
| N-18 unused declarations | used by P1-2 |
| N-19 the per-nap re-anchor untested | recorded: near-equivalent while uhd-clock re-anchors every 100 ms |
| DEVIATIONS 1–15 | the spec text now says what the code does: KG-8's comment (spec 19), the `mr_32` note (VE-4), the dev-dependencies (overview §5), UR-3's brackets, UR-7's counters, UR-21's refusal list, UR-24's stream commands, UR-32 deferred by name, UR-34's first block, EA-7's `Config::new` (design/16) |
| G09 | kept as not killable by a black-box test (the reviewer's reading: only Kernel code runs in the window); no Kernel hook |

**Mutations after the fixes.** `mutations.json` gained Review L's rows R01…R16, each killed by the test its fix added (named in the table above). The whole list of 109 ran once: **107 killed**, then R08 (its first text was equivalent — under abort the immediate untimed stop hides the cut — and now makes abort behave as orderly) and R16 (its test's 2 ms blocks made the straddling block a matter of chance; the test now uses 20 ms blocks) were rewritten and killed in two further runs each. G36 is killed by its new test. **G09 was killed in this run** after surviving the two before: it remains a race the tests only sometimes win, not killable deterministically without a Kernel hook, which Review L advised against.

A process note: a probe built in a scratch copy with the shared target directory left mutated artifacts that a later build of this worktree reused (AGENTS.md §7's mtime rule, met again) — three runs of `ur_26_an_abort_publishes_nothing_after_the_stop_instant` failed on the mutant's code until `find … -exec touch {} +`. Every run recorded here followed a `touch`.

## 8. Review M

Brief: [`prompts/review-m.txt`](prompts/review-m.txt). Report: [`reviews/review-m.md`](reviews/review-m.md). The same Opus reviewer, resuming its Review L session with the fix commit, 2026-09-28, read-only.

**Verdict: CHANGES_REQUIRED** — 0 P0, 2 P1, 8 P2 (new). Every Review L P0 and P1 closed (P1-7's L04 an accepted ceiling), every R01…R16 and G36 killed; the new findings came from the fixes' own new paths. All were accepted.

| Finding | Disposition |
|---|---|
| **P1-A** a timed stop handed over inside the device lead is late, and uhd-rx took the device's `LATE_COMMAND` for a missed start and restarted the stream | `stop_at` issues a timed stop only when the cut is a device lead ahead, else the untimed fallback; a `LateCommand` while a cut is set stops the stream untimed and never restarts; the fake reports `LateCommand` for a timed `rx_stop` in its past; `ur_26_an_orderly_stop_does_not_restart_the_stream` (20 ms blocks: no `rx_start` after the stop, no `LATE_COMMAND`, no late stop issued); UR-17, UR-26, UR-33 amended; R17, R18 |
| **P1-B** `ur_21_late_policies_on_the_device_lead` flaky under 1.85.0 | the on-time case 20 ms ahead; the device lead's boundary is B8's to measure; the spec's test row says so |
| (found while checking P1-B) `rehearsal_b7_session_loopback` failed once in 10 runs under 1.85.0: a `send_asap` move of 2.09 ms, past Review L's one-device-lead bound | on the fake the bound is the whole 5 ms lead (spike K6 itself is `ur_21_an_untimed_send_is_on_time_after_its_delivery`'s); then 15 runs of the fake binary on 1.85.0 and 5 on stable without a failure |
| N-1 a switch after a Cut moves the cut later | a switch is ignored once `stop` has begun, and never moves an earlier cut later; UR-25 amended (untested: the window between `Cut` and uhd-control's join needs a Module-level race; recorded) |
| N-2 the lock held across `schedule` | KC-46a says a paced `schedule` may block only briefly and that a wake scheduled after RS-6 step 1 is a harmless no-op (M05 is equivalent in effect, as N-3's reasoning says) |
| N-3 old samples reaching the new stream | a block whose first tick precedes the stream's origin is dropped and counted (`rx_before_origin`); UR-17 amended (untested on the fake, which stops at once) |
| N-4 enabling a stream that is still draining | refused with `COMMAND_REJECTED` until its `e₁`; `ur_25_enabling_a_draining_stream_is_refused`; R20 |
| N-5 global error text on shared objects | a ceiling (metadata and sensor-value calls, whose failure is rare) |
| N-6 the orderly half of N-9 untested | `ur_26_orderly_stop_delivers_the_tail_abort_does_not` bounds the end at the stop instant + 1 ms + one sample; R19 |
| N-7 the per-channel depth untested | `ur_24_the_queue_counts_a_command_per_channel` (two channels, the 9th update refused); R21 |
| N-8 the `Inexact` host instant untested | recorded: no caller schedules a `host.monotonic` instant on a paced Authority (the reviewer checked `advance_to`, the agenda, the wake and the Providers) |

**Mutations.** `mutations.json` gained R17…R21 (114 rows). Every row whose code or killing test changed in this round ran again — U24, U25, U26, U32, U37, U40, U41, U43, U45, R03…R08, R10, R12…R21 and G36: **27 of 27 killed**. G09 stays the one race the tests win only sometimes (it survived Review M's run and was killed in the run before).

**No further review.** The owner's rule: re-review after a large fix, stop after a small one. Review L's fixes were large and were re-reviewed; Review M's are small and local — two branch conditions in uhd-rx's stop path and one in its switch, a refusal in uhd-control, a drop of stale blocks, the fake's late stop, a test's margin, and spec sentences — each the fix the reviewer recommended, each with its test.

## 9. The one-CBX bench

The owner, 2026-09-30, after asking what the bench needs and whether one CBX on the X310 would do: "はい，UBX2枚での試験ではなくCBX1枚でループバックで可能なように内容を修正してください". The bench is now an X310 with one CBX (or CBX-120) in slot A, its TX/RX cabled to its RX2 through ≥ 30 dB.

**What one CBX breaks in `x310-ubx`.** Three things, each checked against UHD 4.10.0.0's source (`host/lib/usrp/dboard/`, VERIFIED): the CBX tunes 1.2–6 GHz (`db_sbx_common.hpp`, `cbx_freq_range(1200e6, 6.0e9)`) and UHD clips a tune to that range widened by the DSP's reach, without a warning (only `tune_result.clipped_rf_freq` and the read-back show it) — so `x310-ubx`'s default 1 GHz, which the bench envelope allowed at 999–1001 MHz, would have transmitted at about 1.18 GHz (1.14 GHz on a CBX-120); one board is one usable channel each way (UHD still reports 2 + 2, channel 1 its unknown board for the empty slot B), against `x310-ubx`'s 0…2; and a timed tune does not fix the CBX's LO phase (`sync_phase` is the UBX's, and since UHD 4.9 the OBX's, only). The gain (0…31.5 dB in 0.5 dB steps) and the antennas are the same.

**Decision: a second profile, `x310-cbx` 0.1.0, not a looser `x310-ubx`.** Spec 18 UR-9 amended: `x310-ubx`'s values with channels 0…1, frequency 1.2…6 GHz, and a default frequency of 2.45 GHz (`CBX_DEFAULT_FREQUENCY_HZ`; RM-5's 1 GHz is outside the range; 2.45 GHz is in the ISM band). UR-5 amended to refuse a profile whose channels the device lacks or whose front ends UHD names otherwise (`Device::front_end`, UHD's `uhd_usrp_get_{rx,tx}_subdev_name`, two more C calls in UR-3): the root cause of the 1.2 GHz case is a profile that does not describe the hardware, and the check is where every binding passes. Rejected: running the bench under `x310-ubx` at an in-range frequency (the profile would still claim 10 MHz and two channels to every Spec, a Mock looser than the device, §59); a frequency override for the tests (the default frequency is what a Session that sets none tunes to, so the envelope must follow the profile, not a test knob); an environment variable naming the profile for the tests (tried: it leaked into every fake test's binding; the tests now take the profile from the front end the device names, `profile_of`).

**Tests.** `ur_05_the_profile_must_be_the_device_s_front_ends`, `ur_09_x310_cbx_is_one_channel_from_1_2_ghz_defaulting_to_2_45_ghz`, and the rehearsals of B3, B4, B6 and B7 on a one-CBX fake (`rehearsal_*_on_one_cbx`, at 2.45 GHz). `FakeConfig` gained `channels` and `front_end`. Mutations: U01's text follows the new check; R22 (front-end check removed), R23 (channel check removed), R24 (the 1 GHz default kept), R25 (the 10 MHz lower bound kept): U01, U02 and R22–R25 killed (118 rows). `cargo test --workspace` 845 passed on stable and 1.85.0; clippy clean with and without `uhd`; `uhd_api` 4 passed against libuhd 4.10.

**What it leaves (ceilings).**
- `x310-cbx`'s `radio.phase_behavior_on_retune` stays `random_unless_timed_tune`, which overstates a CBX: RM-9 has no value for a phase no timed tune fixes. A `radio` Vocabulary addition (`random`) is Phase 8's; nothing in Phase 7 reads the capability.
- Gate X's criterion 9 (00-overview §10) now reads "on an X310 + one CBX". `x310-ubx` stays implemented and unmeasured; B8's numbers are the CBX's (a tune's lead and how many tunes the queue holds depend on the synthesizer).
- Phase 8's parity compares MockRadio's `x310-like` (the UBX values) with the measured device; with a CBX bench it needs a Mock profile with the CBX's values, or a UBX bench. Phase 8 decides.
- UR-12 records a frequency read-back that differs from the claim but does not refuse it, as it refuses a rate. With UR-5's check, no profile claims a frequency its front end clips; a refusal on the read-back would be defence in depth, not added now.
- Bench findings go to §11, not here.

**Review.** An Opus review of `09f11b4` (read-only, UHD 4.10.0.0's source): PASS_WITH_RISK, 0 P0, 0 P1, 5 P2, all accepted. It VERIFIED that a real X310 with one CBX and slot B empty passes the check (2 + 2 channels, channel 0 `CBX…`) and that `x310-ubx` is refused there. P2-1: the clip is not to 1.2 GHz and not logged (the texts above corrected). P2-2: the one-CBX fake had one channel where the device has two (the fake now names each channel's front end; `one_cbx` is a CBX-120 plus an unknown board; a UBX + CBX case added; R26 checking every device channel and R27 checking channel 0 only, both killed, 120 rows). P2-3: `hw_b1_probe` now fails unless the profile is `x310-cbx`. P2-4: UHD `strncpy`s into the string buffers, so a full buffer had no NUL; every read passes one byte less. P2-5: bench.md's B9 row named `x310-ubx`. The fixes are small and local, so no re-review.

## 10. The one-OBX bench

Asked whether an OBX would be better than the CBX, I recommended it; the owner, 2026-09-30: "了解ですOBXに切り替えてください". The count was not given; the profile assumes one board (one channel each way), which a second OBX in slot B also satisfies.

**Why the OBX** (UHD 4.10.0.0's `db_obx.cpp`, `db_obx.hpp`, `obx/obx_expert.cpp`, VERIFIED): 10 MHz–8.4 GHz (`obx_freq_range`), so RM-5's 1 GHz and the bench's 999–1001 MHz envelopes stand; 160 MHz of bandwidth; the gain and antennas the UBX has; two MAX2871 LOs each way with the timed LO phase sync the UBX has (`_sync_phase`), so `random_unless_timed_tune` is true of it and §9's phase ceiling does not reach the bench; and the same kind of synthesizer as the UBX, so the lead and queue numbers B8 measures are closer to what `x310-ubx` would show than a CBX's (INFERRED: the OBX's tune path, a CPLD and UHD's expert framework, is its own). Against it: UHD has driven it only since 4.9 (June 2025; `db_obx.cpp` is absent from v4.8.0.0), fewer users than the CBX; the Docker image's 4.10 has it.

**Decision: a third profile, `x310-obx` 0.1.0**, `x310-ubx` with channels 0…1 and 10 MHz–8.4 GHz; `x310-cbx` kept (implemented, tested, a fallback if the OBX's young driver fails the bench). Rejected: accepting OBX front ends under `x310-ubx` (its name says UBX and it claims two channels; a one-OBX device reports slot B's unknown board on channel 1, which UR-5 refuses anyway). `x310(…)` takes the upper frequency too; `profile_of` knows `OBX`; `one_obx` fakes an OBX plus slot B's unknown board; `hw_b1_probe` fails unless the profile is `x310-obx`; the rehearsals of B3, B4, B6 and B7 run on the one-OBX fake (B3 also on the one-CBX fake); bench.md, spec 18 UR-5, UR-9, UR-34 and Gate X's criterion 9 name the OBX.

**Tests and mutations.** `ur_09_x310_obx_is_one_channel_from_10_mhz_to_8_4_ghz_defaulting_to_1_ghz`; `ur_05_the_profile_must_be_the_device_s_front_ends` gained the OBX cases. R28 (`x310-obx`'s range capped at 6 GHz) and R29 (`x310-obx` naming UBX front ends) added, R25's text follows `x310(…)`'s new argument: U01, U02 and R22–R29 killed (122 rows). `cargo test --workspace` 847 passed on stable and 1.85.0; clippy clean with and without `uhd`; `uhd_api` 4 passed. The change copies `x310-cbx`'s shape, which the Opus review of §9 read, so it had no review of its own (the owner's rule: small and local).

**What it leaves.** `x310-ubx` stays unmeasured, as in §9. Phase 8's parity needs a Mock profile with the OBX's values (MockRadio's `x310-like` is the UBX's: 6 GHz, two channels) or a UBX bench; Phase 8 decides.

## 11. The bench

What the bench (plan/phase7/bench-results.md, session 2 part 2, 2026-09-30; X300 + one OBX, UHD 4.10) found that the specs got wrong. Recorded before any change, for the owner (handoff.md §4, "前提と規則": a design change is recorded here and asked, not made at the bench). The findings were recorded before any change; the owner's decision and the change are at the end of this section, Review N's re-examination in §12.

### F1 — the X3x0 ignores the time of a receive stop (UR-25)

**Found.** `hw_b8_raw_rx_timed_stop`: a continuous stream given `rx_stop(Some(now + lead))` ends where the stop was issued (−0.01…−0.04 ms) at every lead from 1 to 500 ms. VERIFIED in UHD 4.10's FPGA source: `radio_rx_core.v` takes STOP outside the command FIFO (line 171) and "timed STOP commands are not supported" (line 526); the host does send the time (`radio_control_impl.cpp:1116–1131`). Spec 18 UR-25 says INFERRED that the X3x0 honours it, with a fallback for a device that does not stop — not for one that stops early.

**What it breaks.** `release_stop` (`rx.rs:213–225`) hands the device the timed stop of a `cold` receive change up to the restart lead (50 ms) before `e1`; the device stops at once. `hw_b8_cold_change_capture`: the old clock's capture ends at sample 161 817 instead of `e1`'s 211 625 — **49.8 ms of samples lost with no gap, no flag and no event**, before a clock whose `ended_at` says they exist. A Session's `rx.sample_rate`, `rx.channels` (a count change) or any `cold` receive change does this, on every X3x0 with an RFNoC FPGA (INFERRED beyond the X300: the same `radio_rx_core.v` serves the RFNoC radios). `stop_at` (`rx.rs:233–251`) would do the same for a cut ≥ 2 ms away; no caller makes one today (the orderly stop's cut is 1 ms), so it is latent. `FakeDevice` honours a timed stop (`device.rs:673–674`), and its `IgnoresTimedStop` fault keeps streaming: it has no mode that stops early, so no test could see this (Vision §13, §59: a double looser than the device is a defect).

**Recommendation.** Never hand an X3x0 a timed stop for a continuous stream: uhd-rx keeps the cut and stops the stream untimed on the first sample at or after it — UR-25's fallback becomes its only path, for `release_stop` and `stop_at` alike. The samples past the cut are already discarded by the cut (RM-16), and the untimed stop lands 0.2–3.9 ms after the cut on this bench (B3, B5, B8's orderly stops), inside the restart lead. `FakeDevice` gains the device's behaviour — a timed stop stops at once — as its default, with the current honouring behaviour as the fault, so that `ur_25_*` tests the path the bench takes. Spec 18 UR-25's INFERRED sentence becomes VERIFIED-negative with these citations.

Rejected: (a) issuing the timed stop just before `e1` (at the device lead): the loss shrinks to ~2 ms but stays, unflagged; (b) a finite `NUM_SAMPS_AND_DONE` command for the samples left to `e1`: the FIFO holds it behind the running continuous command, which never finishes (VERIFIED from `radio_rx_core.v`: `ST_RUNNING` ends a continuous command only on `cmd_stop` or an overrun, line 570, and the FIFO pops only in `ST_STOP`; Review N, N5; not measured) — a stop that could not be issued safely; (c) flagging the missing tail as a gap: the samples are lost for no reason the device imposes.

### F2 — a `cold` receive restart is late: uhd-rx waits for a 100 ms timeout (UR-25)

**Found.** In `hw_b8_cold_change_capture` the switch ran 0.58 ms after `e2`, the timed start came back `LATE_COMMAND`, and UR-17 restarted 150.8 ms after `e2`. INFERRED from the timing (it agrees to the tick): after F1's early stop no sample comes, and uhd-rx ends the old stream only on a sample past the cut or on an `rx_recv` timeout (`RECV_TIMEOUT` 100 ms, `rx.rs:85`) with `now ≥ cut`; the first timeout came 100 ms after the stop. Had the device stopped at `e1`, the wait would still end ~50 ms after `e2`: `RECV_TIMEOUT` exceeds the restart lead. The device itself restarts on its tick at 1 ms lead, even with a new streamer (`rx_open` ~1 ms; `hw_b8_raw_restart_lead`): the restart lead is uhd-rx's need, not the device's.

**Recommendation.** F1's change removes the cause: the stream runs past `e1`, the first sample past the cut ends it, and the switch starts about one block after `e1`, ~48 ms before `e2`. In addition uhd-rx's end-of-stream check should not depend on the timeout: when a cut is set, `rx_recv`'s timeout is bounded by the time to the cut plus a block (so a stream that has stopped is found within a block of the cut). Spec 18 §3's restart lead (50 ms) can stay: the bench shows margin once the loop no longer waits.

Rejected: lowering `RECV_TIMEOUT` everywhere (it also paces UR-29's 1 s "yielded nothing" check and the idle loop; a bound only while a cut is pending is the smaller change); raising the restart lead above 100 ms (hides the wait, costs every `cold` change 100 ms).

### F3 — a timed start-of-burst at the previous burst's end is late at the device (UR-23)

**Found.** `hw_b8_raw_burst_gap`: a burst ending with end-of-burst at tick T and the next with a timed start-of-burst at T is reported `TimeError` and not played, 4 of 4; at T + 1 sample and later, never. `hw_b8_preemption` hits it through the Module: at a 20 ms lead the repeat is cut with `eob` at 121 809, the burst's start, and the device reports the burst late (`late_at_device`, 15 ns) and drops it. Spec 18 UR-23 prescribes exactly this transition ("the current one is sent up to the sample before the next start, its last buffer with end-of-burst … the next starts with start-of-burst and its time spec"); RM-15 lets a burst start where the previous ends. `FakeDevice` accepts it: it refuses a timed start only before its cursor (`t < st.tx_cursor`, `device.rs:836`), not at it.

**Recommendation.** When the next held burst starts exactly at the open burst's next sample, uhd-tx does not end the open burst on the device: it sends the next burst's samples in the same device burst (no end-of-burst, no new start-of-burst between them), while the two Kernel bursts keep their own records (`BurstRecord` is per Kernel burst; RM-16). A start one or more samples later keeps UR-23's end-of-burst / timed start-of-burst, which the device takes. `FakeDevice` refuses a timed start-of-burst at its cursor as the X300 does. INFERRED: that UHD sends two `tx_send` calls without end-of-burst between them as one continuous device burst whatever their time specs — the second call's time spec must then be omitted (`at = None`), which is what uhd-tx does inside a burst already (`tx.rs:226–227`: the time spec only on the burst's first buffer).

Rejected: (a) moving the next start one sample later (moves a timed start the client asked for: a `TIME_ERROR` the client did not cause); (b) ending the open burst one sample early (drops a sample of the open burst's waveform, silently); (c) refusing a burst that starts at the open burst's end (RM-15 allows it; the Mock accepts it: a Session that works on the Mock would fail on the X310).

### The decision and the change

The owner, 2026-09-30: "F1-F3は推奨で" — the three recommendations above. One departure from F1's text: its fake kept the honouring behaviour as a fault; the change removed it instead, as no device the Module drives behaves that way (Review N, N5).

**Tests first** (bench.md "If a step fails": a test on `FakeDevice` that reproduces the bench's failure before the fix). `FakeDevice` was made as strict as the X300 in the three ways the bench showed it was looser: any `rx_stop`, timed or not, stops the stream at once (the `rx_stop_at` state and the `IgnoresTimedStop` fault are gone: no device the Module drives behaves that way); an `rx_recv` on a stopped stream waits its whole timeout, as UHD's does (it returned after 10 ms, which hid F2); a timed start-of-burst at the end of the samples already queued is `TimeError` (`t <= tx_cursor`, was `<`). Against the unchanged Module, six tests then failed on exactly the bench's findings: `ur_25_a_cold_receive_change_delivers_every_sample_before_e1` (new: the old clock's capture ended at sample 64 000 for an `e₁` of 112 280, 48 ms early, as the bench's 49.8 ms), `ur_25_a_stream_silent_before_e1_switches_before_e2` (new: the switch at 500 395 376 for an `e₂` of 491 000 000), `ur_25_the_old_stream_is_stopped_untimed_at_e1` and `ur_25_the_receive_stop_is_recorded_when_it_is_issued` (the two tests that asserted the timed stop, rewritten to assert its absence: `ur_25_a_device_that_ignores_the_timed_stop_is_stopped_at_e1` and `ur_25_a_timed_receive_stop_is_released_a_restart_lead_ahead` are gone), and the existing `ur_23_a_burst_ends_at_the_next_bursts_start` and `ur_21_a_burst_inside_the_in_flight_window_is_late` (the device's `TimeError` at the gap-0 start).

**The change.** uhd-rx (`rx.rs`): `Stream::held_stop` and `release_stop` are gone; a `cold` switch and `stop_at` only set the cut; `stop_at_cut` stops the stream untimed when a block reaches the cut (`k + len >= cut_k`: a block ending exactly at the cut stops it too, which the timed stop had covered before) or when the stream is silent past it, and records `rx_stop` in `applied` with the cut and the instant it was issued; `recv_timeout` bounds the wait by the cut plus one block (1 ms…100 ms). uhd-tx (`tx.rs`): `continues_at` marks a device burst left open for the burst that starts at its next sample; `send` separates the device's start-of-burst and end-of-burst from the Kernel's (`device_sob`, `device_eob`), so that a burst held at the open one's end, or preempting it at its next sample, continues the device burst without either and without a time spec; the Kernel's `START_OF_BURST`/`END_OF_BURST` blocks and the two `BurstRecord`s are as before; `unacked` counts device bursts; `end_open(true)` closes a device burst left open for a continuation that did not come, and `step` does so when the next held burst is not the continuation. Spec 18 UR-17, UR-23, UR-24, UR-25, UR-26, UR-33, the INFERRED list and §6's table follow. No profile value, no spec 18 §3 number, no Kernel or Vocabulary change.

**Checks.** `cargo test --workspace` 851 passed on stable and 1.85.0 (849, less the two removed tests, plus four); `--features uhd`: fake 101 (102 without the feature), `uhd_api` 4, hardware 26 ignored; clippy `-D warnings` clean with and without `uhd`. Mutations: R12 (a timed stop at `e₁` again), R13 (the untimed stop at the cut removed) and R14 rewritten for the new code, R06 rewritten as a continued device burst given a timed start-of-burst, R17 withdrawn (its code is gone), U17's text follows `step`; B01 (the wait unbounded), B02 (the silent stream's stop and record), B03 (end-of-burst at the next burst's start), B04 (a preempted burst ended before its continuation) added. Every row on `rx.rs`, `tx.rs` and `device.rs` ran again, 31 rows: **29 killed; R18 and B02 survived.** B02 survived because `ur_25_a_stream_silent_before_e1_switches_before_e2` did not look at the stop: it now asserts the untimed stop before the start at `e₂` and its `applied` row, and B02 (with B01) is killed. **R18 is withdrawn:** it disabled the `LateCommand`-while-cut guard, whose case was a late timed stop, which no longer exists; the guard stays for a late start of a stream already being cut, a case no test reaches (INFERRED rare: a UR-17 restart pending when a stop or switch arrives). 124 rows.

**On the bench** (bench-results.md, session 2 part 3): `hw_b8_cold_change_capture` now delivers the old clock's samples to `e₁` and starts the new clock at `e₂` with no `LATE_COMMAND`; `hw_b8_preemption`'s 20 ms burst is played, with no `TIME_ERROR`; B1–B8, B9's long Runs and B7 (Python) pass. **§11 F1–F3 are closed.**

### What is not a design change

The other B8 numbers are Phase 8 inputs and change nothing now (spec 18 §3, bench.md): the device lead is 0.3–0.5 ms (Module 2 ms); a timed OBX retune is clean at ≥ 2 ms lead (so the Module's 2 ms is about the least for retunes); seven timed OBX tunes fit the queue (profiles' `command_queue_depth` 16 — the Mock's envelope is looser than the device: Phase 8's parity will fail it, as §59 intends); the in-flight window can be ≥ 0.5 ms at 2 Msps (10 ms); the transmit ends ≤ 11.4 ms after a Stop's submission; the loop delay is 35–44 samples depending on the rate (`tx_path_delay_samples` 45 is one number); 200 Msps receive is sustained. UHD's overrun restart is 50 ms after detection, VERIFIED (`radio_control_impl.cpp:48, 117`, `OVERRUN_RESTART_DELAY = 0.05`), as MR-3 says.

## 12. Review N

Opus reviewed `bc0db98` (brief [`prompts/review-n.txt`](prompts/review-n.txt), report [`reviews/review-n.md`](reviews/review-n.md)): correct on the path the bench measured, not yet safe outside it — three P1 blockers, two P1 test gaps. The owner, 2026-09-30: "推奨で直して再レビューをしてください". Every finding below was taken as the review recommends unless the row says otherwise; tests first, as for §11.

**The fake first (T1, T2).** `FakeDevice` gained the receive link the bench showed and it lacked: `FakeConfig::rx_latency` (a packet reaches `rx_recv` that long after its last sample) and `rx_packet` (samples per packet; defaults 0 and a whole request, so every earlier test runs as before); a per-packet wait, with a call cut short by a packet that does not come returning what it has and the next call returning `Timeout` at once (UHD's error cache, `rx_streamer_impl.hpp:139–203`, read in the image); an untimed stop that still delivers what was produced before it, ahead of the next stream's samples (the bench: one or two blocks); and a transmit burst that runs out of samples before its end-of-burst reported as `Underflow`. Its §11 strictness is now pinned on the device itself: `ur_33_a_timed_receive_stop_stops_the_stream_at_once`, `ur_33_a_stopped_stream_s_tail_comes_before_the_next_stream`, `ur_33_a_stopped_stream_s_recv_waits_its_whole_timeout`, `ur_33_a_recv_cut_short_by_a_packet_s_timeout_is_followed_by_a_timeout_at_once`, `ur_33_a_timed_start_at_the_previous_burst_s_end_is_late` (with N2: the late report's tick is at or after the start), `ur_33_a_burst_that_runs_dry_underflows`. Against the unchanged Module the three blockers' tests then failed: `ur_25_a_long_block_at_a_low_rate_stops_the_stream_at_e1` (B1: an old-rate sample on the new clock, `rec_0: map 1 sample 148171 is 0.54039, its clock's ramp 0.4467926`, and a `LATE_COMMAND`), `ur_25_a_slow_link_at_a_low_rate_delivers_the_old_clock_to_e1` (B2: the old clock ended at 82 000 for `e₁` 82 660 and at 88 000 for 89 111), `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it` (B3: the device's `TimeError`).

| Finding | Disposition |
|---|---|
| B1 (P1) | Fixed as recommended: `recv_len` asks for no more samples than up to a pending cut, so the last block ends at `e₁` and the stop follows one delivery after it whatever `block_len`; each new stream drops (`rx_before_origin`) blocks before the instant the previous stream's untimed stop was issued (`Stream::not_before`), which `ur_25_a_late_switch_drops_the_old_stream_s_tail` exercises with a 60 ms link (the switch late, its tail past the new origin, none of it published). |
| B2 (P1) | Fixed, with one addition: a `Timeout` ends a stream past its cut only once the time is past the cut plus a block plus the delivery allowance (3 ms) **and** no sample has come for as long (`quiet_span`); the wait is bounded by the same instant. The review's rule alone ended the 60 ms link's stream 57 ms early (the late-switch test found it): a stream still delivering, however late, is waited for. |
| B3 (P1) | Fixed as recommended: a Kernel burst's end is never the device burst's at once; `continues_at` holds the device burst open for a burst booked at its next sample until that sample is a device lead (2 ms) away, or another burst is next, and then an empty end-of-burst ends it before it runs dry. `ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it` and `ur_23_a_burst_alone_is_ended_before_it_runs_dry` cover the closing paths (T3). |
| N1 (P2, INFERRED) | Recorded in spec 18 UR-23 and for Phase 8 (T6): the gap the X300 needs may be in radio ticks (`radio_tx_core.v`'s `ST_IDLE` → `ST_TIME_CHECK`), so 1–3 samples at 200, 100, 66.7 Msps; `hw_b8_raw_burst_gap` at those rates is Phase 8's. |
| N2 (P2) | Fixed: the fake's late report carries a tick at or after the late start. |
| N3 (P2) | Open: `unacked` per device burst is right but untested (a test needs a device-late burst after a continued pair, which UR-21 keeps the Module from sending). |
| N4 (P2) | Fixed: an abort's stop is recorded in `applied` and marks the stream stopped, so the tail reaching the cut stops and records nothing again (`ur_26_an_abort_publishes_nothing_after_the_stop_instant` now asserts one `rx_stop` and one row). |
| N5 (P2) | Fixed: spec 18 UR-17 and UR-25; §11's lead paragraph; the note on F1's fake; F1's alternative (b) VERIFIED. |
| N6 | Nothing to do (the marks check out). |
| N7 (P2) | Both fixed: UR-29's silence check runs until a pending cut, not only without one (`ur_29_a_stream_silent_before_a_far_cut_is_a_lost_device`, as UR-29's "while it should be streaming" already says); `abandon` ends an open device burst with an empty end-of-burst (untested: the fake has no failing send short of a lost device). |
| T1, T2 (P1) | Done (above). |
| T3 (P2) | Done in part: the Stop and the lone-burst paths are tested, and the mutations B09, B10 kill; the `cold` transmit switch during a held continuation is not. |
| T4 (P2) | Done: `FakeDevice::min_recv_timeout`; the silent-stream test and every capture across `cold` changes assert ≥ 1 ms; mutation B12 is killed by the 60 ms link's (the one where the wait reaches its floor). |
| T5 (P2) | Open: R18's guard is still reached by no test (the late-switch test's `LATE_COMMAND` is the new stream's, with no cut). |
| T6 (P2) | For Phase 8: B2 at 390 625 S/s is run at this session's bench (`hw_b8_cold_change_capture_low_rate`); N1's gap at 200 and 100 Msps is Phase 8's. |

**Checks.** `cargo test --workspace` 864 passed on stable and 1.85.0; `--features uhd` 118 passed, hardware 26 ignored; clippy `-D warnings` clean with and without `uhd`. Mutations: U17, R13, B01, B02, B03 rewritten for the new code, B05–B13 added (133 rows); the 39 rows on the three changed files rerun: **36 killed; B07, B08 and B12 survived.** B07 (a `Timeout` at the cut ends the stream whatever the silence) and B12 (the 1 ms floor removed) are reached only by a link slower than the wait's allowance, so both now name `ur_25_a_late_switch_drops_the_old_stream_s_tail`, and every capture test asserts the floor. B08 was not the pre-fix code: it restored the device end-of-burst but left `continues_at` set, so the next burst went out without start-of-burst and the fake did not see it; with its second edit (`continues_at` only without a device end-of-burst: `bc0db98` exactly) it is killed. Rerun: B05–B08 and B12 killed. 133 rows.

**On the bench** (bench-results.md, session 2 part 4, at `aaf1e00`): the low-rate captures (390 625 S/s, the default block and 65 536) keep the old clock to `e₁` and start the new at `e₂`, and drop the old tail; the burst booked at a sent burst's end is played; B1–B8, B9's long Runs and B7 (Python) pass.

**R-1 — residual, for the owner and the re-review (INFERRED).** B1's fix bounds each receive request issued once a cut is pending, but not the one uhd-rx is already inside when the change is booked; that call can last a whole block (168 ms at 390 625 S/s and `block_len` 65 536). On the bench the untimed stop came 25.5 ms after `e₁` in that case (0.4–1.9 ms with the default block). A block longer than about the restart lead less the delivery can therefore still make the switch miss `e₂` when the booking lands early in a call: a `LATE_COMMAND` and UR-17's restart, reported, with no old sample on the new clock (`not_before`). Two ways to close it, both beyond Review N's recommendation and so not taken without the owner: (a) book `e₁` at least one block after the restart lead (`e ≥ now + 50 ms + block`: UR-25's rule changes for every `cold` change, by one block — 2 ms at the defaults); (b) have uhd-rx ask for at most a few ms of samples per call and assemble `block_len` blocks itself (UR-17's block is no longer one `rx_recv`). I recommend (a): one line in `control.rs` and UR-25, no change to how blocks are made.

**Review O** ([`reviews/review-o.md`](reviews/review-o.md), brief [`prompts/review-o.txt`](prompts/review-o.txt)): B1, B2, B3 at gap 0, T1 and T2 closed; two P1s open. **O-B1** — the deferred device end-of-burst is an empty send, which UHD pads to one zero sample, so the device burst ends one sample late and a burst at gap 1 is dropped — is VERIFIED on the bench (`hw_b8_raw_empty_eob_gap`, bench-results.md session 2 part 4: gap 1 late 4 of 4 after an empty end-of-burst, played 4 of 4 after end-of-burst on the last data buffer). **O-B2** confirms R-1 on the fake (2 of 6 booking phases late) and endorses (a) with the bound `e ≥ now + restart lead + one old-rate block + the delivery allowance`. Both wait for the owner.

## 13. Review O

The re-review ([`reviews/review-o.md`](reviews/review-o.md)): Review N's B1, B2 (with the `quiet_span` departure justified), B3 at gap 0, T1 and T2 closed; two P1s new. Before deciding O-B1, the owner asked whether one held sample is worth its complexity and what a zero sample breaks, then: "推測を実機で確かめてください" — `hw_b8_raw_empty_eob_gap` VERIFIED it (§12, bench-results.md part 4). Then: "それでは二つとも推奨で直してください". Asked mid-way whether O-B2's bound holds from 100 kS/s to 200 Msps: it is not a fixed time but the old stream's block at its rate; the question showed that the block term must be the longer of `block_len` and UHD's packet (`max_num_samps`, now `Device::rx_packet_samples`) — a 100-sample block at 390 625 S/s is 0.26 ms, its 1 996-sample packet 5.1 ms — and that the fixed 50 ms and 3 ms are measured at 1 Msps only, so the extremes are checked on the bench (`hw_b8_cold_change_timing_at_the_extremes`, below). 390 625 S/s is the X300's lowest rate (200 MHz / 512); 100 kS/s is not offered.

**Tests first.** `FakeDevice` sends an empty end-of-burst as one zero sample, as UHD does (T-a), takes no cached timeout (N-1: UHD's `rx_streamer_impl.hpp:25–47` caches none — §12's text was wrong), and puts receive packets on the stream's own grid (N-2). Against the unchanged Module, `ur_23_a_burst_one_sample_after_a_burst_is_played` failed (B at A's end + 1 sample: `TimeError`, booked early or late) and `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time` failed (phase 0: switched 8 ms after `e₂`).

| Finding | Disposition |
|---|---|
| O-B1 (P1) | Fixed as recommended, with one refinement: uhd-tx holds back the last sample of a Kernel burst's final buffer (`Tx::tail`) while a burst could still be booked at its end, and sends it with end-of-burst at the deadline (or when another burst is next, at `Stop`, a `cold` switch, `Provider::stop`, `abandon`), or without it ahead of a continuation's first buffer; a final buffer sent within the deadline carries end-of-burst itself, so most bursts hold nothing (the refinement), and a one-sample burst is not held. An empty end-of-burst remains only for a repeat stopped mid-waveform (nothing to hold). `ur_23_two_bursts_back_to_back_loop_back_whole` checks on the exact loopback that the held sample lands where it belongs. |
| O-B2 (P1) | Fixed as recommended and corrected by the owner's question: a receive `cold` change while a receive stream runs puts `e` at least `now + 50 ms + d + 3 ms`, `d` the old stream's longer of `block_len` and `rx_packet_samples()` at its rate; UR-25's `LATE_COMMAND` threshold moves with it. Transmit changes are unchanged. |
| N-1 (P2) | Fixed: the fake's cache removed; the `ur_33` test now asserts a partial return and a normal next wait; spec 18 UR-25, UR-33 corrected. |
| N-2 (P2) | Fixed: packets on the stream's grid from its start, a stopped stream's last packet ending at the stop. |
| N-3 (P2) | Fixed: `not_before` is the instant after `rx_stop` returns plus a device lead. |
| N-4 (P2) | Fixed: a burst held at the continuation point but at or after `e₁` is not a continuation; the device burst is ended (untested: a waveform must end exactly at `e₁`). |
| N-5 (P2, INFERRED) | Recorded: every held burst end relies on uhd-tx sending the held sample within ~1.5 ms of its deadline; the bench showed no underflow (part 4, 5). |
| N-6 (P2) | Open: `quiet_span` still uses `block_len`; with O-B2 the call in progress ends before `e₁`, so a silent stream with a long block ends later than needed but no sample is lost. |
| N-7 (P2) | Open: the one intermittent 1.85.0 failure was not reproduced here (4 runs). |
| N-8 (P2) | Fixed: `blocks_until` fails after 5 s. |
| T-a, T-b (P1) | Done (above). |
| T-c, T-d, T-e (P2) | Open, as recorded (the allowance unpinned; `abandon` untested; N3, T5, the `cold` switch during a held continuation). |

**Checks.** `cargo test --workspace` 868 passed on stable and 1.85.0; `--features uhd` 122 passed, hardware 29 ignored; clippy `-D warnings` clean with and without `uhd`. Mutations: B03, B08 rewritten for the new code; B14 (an empty end-of-burst instead of the held sample), B15 (O-B2's term removed), B16 (the held sample not sent ahead of the continuation), B17 (the fake's padding removed) added (137 rows); every row on `rx.rs`, `tx.rs`, `device.rs` and `control.rs` rerun, 63 rows: **59 killed; U13, B05, B14 and B16 survived.** B05 (requests not bounded by the cut) no longer shows at the fixed phase of `ur_25_a_long_block…` once O-B2 moved `e₁` past the call in progress; it now names the phase sweep and is killed. B14 (an empty end-of-burst instead of the held sample) zeroes a burst's last sample without any error report; it now names `rehearsal_b6_txrx_and_repeat`, whose exact loopback sees it. B16 (the held sample not sent ahead of a continuation) is reached only when the continuation is booked after the last buffer went out; `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it` now asserts that exactly the 31 000 samples were handed over, and kills it. **U13** (uhd-control drains its queue before booking) survives at `08a5b90` and at `9010c5b` as well, before this session's changes: pre-existing, not introduced here, left open for the re-review and the owner (its test, `ur_14_actions_are_finished_before_the_next_recv`, submits one Action, which the mutation books the same way). 137 rows.

**On the bench** (bench-results.md, session 2 part 5): O-B2 holds at both ends of the rates — 200 Msps, and 390 625 S/s with a 100-sample block and with a 65 536-sample one, four booking phases each, the switch always 44.8–49.8 ms before `e₂` and no `LATE_COMMAND`; O-B1's burst one sample after a burst is played, booked early or late; B1–B8, B9's long Runs and B7 (Python) pass.
