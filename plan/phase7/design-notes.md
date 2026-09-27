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
