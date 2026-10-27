STATUS: done
(The implementer's final message, saved by the orchestrator.)

## Built
- R1 (provider/control.rs): a receive frequency goes to the device timed only if a planned non-refused segment holds the channel and covers e (origin <= e, no cut or instant(cut) > e). Rows, live, handed_at, queue slot, projection unchanged; gain and transmit unchanged. Deferred value: the destination's configure applies it; if already configured, give applies it untimed. Destination = earliest non-refused non-empty segment beginning after e, re-read at each release; only its own mark decides; a segment with no configuration (first segment) counts as configured. ColdConfig has an AtomicBool mark set by configure while holding `updates`; uhd-control's enable-from-none sets it after its own configure succeeds.
- R5 (device.rs, FakeDevice): DDC/DUC FIFO per front end, consumed by device time over intervals samples actually flow, at radio rate, one entry per tick; sample values from the change history. DDC holds 32; the 33rd timed write fails, as does any untimed frequency write while full. DUC overflow abstract, INFERRED. Timed sends keep their time only until a send without one. pub dsp_pending, dsp_log, freq_at. Radio queue applies only gains.
- Probes P4–P7 in tests/hardware.rs (not run). UhdDevice::tune_auto in src/uhd.rs (probe-only, not in the Device trait) returns RF and DSP of the same AUTO/AUTO tune.
- Docs: spec 18 §2 evidence, UR-24, UR-25, UR-33, *Test:* lines, Changes row; design 33 D9 rules 1–4 (original text kept); note 39 and research corrections.

## Changes from the plan
M1 uses the real await_release gate, source value via freq_at. 1c, 4b on Direct; 8 and 10 as Spec Runs; 6 and 9 unit tests calling the device. M5a weaker (cut-at-origin segment not reached). freq_constant added (per-block history; a 200 MS/s rehearsal went past the 120 s watchdog; now 31–59 s vs 44 s at HEAD). Extra test ur_33_a_stream_that_starts_now_takes_a_due_command_at_its_first_sample.

## Existing tests changed
control.rs: ur_24_a_timed_update_reaches_a_later_stream_s_pending_segment (untimed apply), ur_25_the_switch_applies_the_configuration_in_effect_at_e2 and ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch (no timed, one redundant untimed), ur_34_a_timed_update_crossing_the_move_… (FIFO). device.rs: ur_33_a_timed_command_follows_its_front_end_across_a_spec, ur_33_a_command_due_while_its_front_end_was_out_never_applies (gain). fake.rs: ur_24_stop_cancels_held_commands (gain).

## Tests written
Receive (control.rs): 1a/1b, 2a/2b, 4, 4c, 5, M1–M6 (M4a/M4b, M5a/M5b), 12d, 12c, series 6/7. Fake (device.rs): R5a–m, a "starts now" test, per-chain FIFOs. Transmit: S2 Ceiling ur_25_a_transmit_channel_that_leaves_and_returns_keeps_its_old_timed_value (control.rs); F3' Ceiling and between-bursts as Spec Runs in fake.rs with 1c and 4b. Not written: 2c, 3d, 12b, 15, 14; handed_at > S for a value not handed is not checked.

## Mutation
New P965–P989; updated J326, P214, P215, P219, P409; P977 filter changed after surviving. Final remote run maintenance,phase7 sim02:4,sim03:4: 58/58 (P424 pre-existing equivalent counted). No rows for: mark published outside the lock, Holder::Owner as mark, handed_at None, released.push dropped.

## Checks
cargo test --workspace 1915 passed; clippy clean; docker clippy clean (too_many_arguments in timed_chunks fixed); rules.py 0; diff --check clean. VH-8 passes unchanged (gains only, does not exercise R1). 20× for ur_24|ur_25|ur_29|ur_33|ur_34 lib tests, fake rehearsal, four new fake tests: green.

## Self-review (Go DeepSeek; free hit its limit): P1 0, P2 8, P3 13
Fixed: fake drain cursor off-by-one; series 7 control test (P989); M4a/M4b wait for a started signal; M5a rearranged; 12d earlier-stop case; P4 prints unconsumed; P6 continuation at the next sample; spec wording. Rejected: late receive start flow (rx_start late branch never sets next); from-none mark outside the lock (one thread configures and releases; spec says so); P4 safety assert kept; weak observations kept.

## Left
Mark behaviour across move chains rests on reading held_elsewhere / needed_before (INFERRED). Not verified: FIFO capacity and persistence across close (bench P4); real FPGA image; P5 reproducing F7 in one process; whether P4-B changes the DSP value (prints INCONCLUSIVE if not). Note 42 §5 still lists UR-22, which D9a does not change.

# Fix round 1 (after Sol FAIL 4 majors; Opus 1 major (MA-1, owner), 5 minors)
- Sol 1: set_subdev_spec ends every RX flow under the old map before clearing streamers; ur_33_replacing_the_subdev_spec_stops_the_receive_flow; P990; P226 old updated.
- Sol 2 (P5): movement only in tone-carrying windows from 1 ms after burst start; priming send timed 400 ms after burst start; prints old header timestamp, e, header ≥ e, INCONCLUSIVE otherwise.
- Sol 3: ColdConfig::configure calls probe("mark_publishing") before the store; ur_25_the_mark_is_published_while_the_projection_lock_is_held; P991 (unlock before store); ClockMutex test-only is_held / waiters; M4a/M4b wait until the thread reached the lock; P992 (lock at give removed → M4b).
- Sol 4: design/18 transmit paragraph, Changes row, design 33 D9 rule 1: F3′ only for instants after the burst's first send. Note 42 not edited.
- mi-1 UR-24 "gives the device no timed command for the frequency"; mi-2 configure after the cut; mi-3 gain; mi-5 ur_24_a_refused_untimed_release_is_a_refusal_like_a_timed_one, P993.
- Checks: 1918 passed; clippy host/docker clean; rules.py 0; diff --check; remote maintenance,phase7 63/63; 20× lib ur_24|25|29|33|34, fake rehearsal, fake D9a tests: 40 runs green.
- M6 / first-segment behaviour unchanged pending the owner's MA-1 decision.

# Fix round 2 (after Sol re-review: 1 major, P5 threshold)
- tests/common/tone.rs: classify_tone, tone_verdict (pure). Signal window (25 samples) when lag-1 phase coherence > max(0.5, 2 × floor); floor from pre-burst windows; only inside the burst from 1 ms after start. Move = 3 consecutive windows > 25 kHz off base. INCONCLUSIVE when < 10 floor windows or fewer than half the windows on each side of e carry the tone; P5 also when header < e.
- P5, P6, P7 use the classifier; P4 prints tone-window count, noise coherence and threshold; P2 unchanged.
- Test the_tone_classifier_finds_an_attenuated_shift_in_noise (tests/fake.rs; 0.008-amplitude 100→150 kHz tone over noise: early shift at 2 ms, on-time at 20 ms, no shift, noise only, no pre-burst windows).
- Rows P994–P997, 4/4 killed. Workspace 1919 passed; clippy host/docker; rules.py; diff --check.
