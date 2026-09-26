# Phase 6 — implementation notes

What was run, in order, and what it showed. The plan is [`00-overview.md`](00-overview.md); spec 16 is [`16-easy-api.md`](16-easy-api.md) and spec 17 is [`17-amendments.md`](17-amendments.md).

## Baseline (2026-09-26, `d977ad8`)

`cargo +stable test --workspace`: 644 passed, 0 failed (Phase 5's count).

## The prototype

Before the plan, §57's snippet was written as a Rust test against `RunHandle`, the way a client would drive it, in a scratch copy with the real Modules (MockRadio `x310-like`, the capture Sink, the Simulation Engine, a 0 dB loopback coupling). What it showed is the evidence column of `00-overview.md` §2:

| Step | Result |
|---|---|
| `connect` | `Running` at instant 0, `start_instant()` 2 000 000 000: the devices start at T0 = 2 s (MR-11), so a client must advance there before its first call |
| `radio.start_repeat` straight after `connect` | `Rejected`: "SC-23: local:mock/tx has no running transmit SampleClock" — the implicit Spec requests zero transmit channels (SB-22c), so a client sets `radio.tx.channels` first |
| §3's second snippet: `radio.rx.frequency_hz` 2.45 GHz, `radio.rx.sample_rate_hz` 20 Msps, `radio.rx.gain_db` 20, then `radio.tx.channels` 1 and `radio.tx.sample_rate_hz` 20 Msps | all admitted; the repeat then admitted with `at` coerced to T0 + 2 ms (RS-19) |
| `sink.capture` of 100 000 samples, then `advance_to` in 1 ms steps, watching the capture directory | the `.sigmf-meta` appeared at 2 005 000 000 ns; no `RunHandle` method said so (hole 1); the artifact (`rec_0`, 800 000 bytes) reached the Manifest only at `finish` |
| `RunChild` | `Rejected` with `ezsdr.run_child`, "RS-25a: child Runs are Phase 6's" (hole 3) |
| the Manifest | the inputs recorded as `mem:<hash>`: a finished Session keeps no waveform bytes (replay's precondition, `00-overview.md` §3) |

Holes 2 (no `wait_for`), 4 (RS-25a's environment) and 5 (SB-14's field) are from reading `coordinator/mod.rs`, `stepping.rs`, RS-25a against SB-29, and `manifest.rs` against SB-14.

## Commits

| Step | Commit | What | Tests (stable) |
|---|---|---|---:|
| 0 | `be6285a` | `00-overview.md`, specs 16 and 17 | 644 |
| 1 | `5f42d3b` | KF-1…KF-4: `events`, `wait_for`, `run_child`, `ezsdr.children`, KC-37's reason; `design/03`, `04`, `06` text | 652 |
| 2 | `4635d91` | VD-1: `sink` 1.1.0, the capture Sink 1.2.0, the payload schema, `design/10` text; the rig's profiles and determinism projection | 653 |
| 3 | `adee35e` | spec 16's server: `ezsdr-server`, `schemas/server/`, the acceptance rig on the server's catalogue, the Reactor-in-a-child-Run carrier; the governance lists | 671 |
| 4 | `7433ffa` | spec 16's Python package, its tests and examples; `v58_10` over the Python sources | 671 (+ 12 Python) |
| 5 | `a8f7979`, `474082b` | the mutation list and tool, the notes, the exit tables, Review H's brief | 671 (+ 12) |
| Review H | `79c7655`, `69883ef`, `2b607a2` | the fixes (`79c7655` is the work in progress, committed by a forked agent), `Rx.request` / `Rx.result`, mutations to 60 | 682 (+ 19) |
| Review I | `ecca745` | the fixes, mutations to 70, `AGENTS.md` and `handoff.md` | 684 (+ 21) |

Each step was verified before its commit on 1.85.0 and on stable (and Step 4's Python suite on 3.9 and 3.13).

## Tests changed under GY-3

| Test | Change | Forced by |
|---|---|---|
| `hd_06_vocabulary` (`sink_vocabulary.rs`) | version 1.1.0; two event kinds, the second `sink.CAPTURE_WRITTEN` | HD-6 (VD-1) |
| `hd_07_descriptor`, `hd_15_a_capture_is_a_sigmf_recording` (`sink_capture.rs`) | version 1.2.0, `sink ^1.1.0`, the implementation hash, `core:recorder` | HD-7 (VD-1) |
| `hd_11_stop_for_own_target_finishes_the_capture` | the one event the Sink now emits is the partial capture's `CAPTURE_WRITTEN`, where it asserted none | HD-16 (VD-1) |
| `hd_14_a_bad_capture_value_is_an_event_not_a_failure` | counts the three `REQUEST_REJECTED` among the events, beside the served request's `CAPTURE_WRITTEN` | HD-16 (VD-1) |
| `hd_14_schema_freeze` | `schemas/sink/capture_written_payload.v1.json` added | HD-16 (VD-1) |
| `rig::determinism_projection` (used by the `v58_03_*` carriers) | also strips the `uri` inside a `CAPTURE_WRITTEN` event's artifact, as it strips the artifact's own: the URI holds the Run id | HD-16 (VD-1); found when `v58_03_reactor_decisions_reproduce_with_their_seed` failed on it |
| every acceptance profile (`rig.rs`) | names the capture Sink 1.2.0 (HD-8 refuses another version) | HD-7 |
| `rig::assemble` | delegates to `ezsdr_server::assemble` (EA-7), which refuses a binding no Module of the catalogue builds instead of ignoring it | EA-7 |
| `ma_03_no_module_crate_depends_on_another`, `po_11_no_hashmap_and_no_wall_clock_in_simulation_code` | `ezsdr-server` in their lists (not a Module, so not in MA-3's Module set); `ezsdr-acceptance` depends on it | spec 16 §1, `00-overview.md` §5 |
| `v58_10_experiments_name_no_mock_type` | also scans `python/ezsdr/*.py` and `python/examples/*.py` | GY-6, EA-18 |

No expected value of a Phase 1–5 test moved.

## Findings while implementing

- **The loopback is rotated.** The first `ea_09_the_default_profile_loops_back` compared the capture with the waveform byte for byte and failed: `x310-like` draws each channel's LO phase at `prepare` (MR-34), so the loopback returns the waveform times one unit phasor. That is the profile working as specified. The carriers find the offset by magnitude and check one phasor fits every sample (`00-overview.md` §7); under `ideal` the phasor is exactly 1 (`test_v58_01_the_same_script_under_another_profile`).
- **The RF envelope judges a transmitting channel.** `test_v58_16_…` first retuned the transmitter to 2.6 GHz with no transmit channel enabled and was admitted: `radio.rf_envelope` judges what may radiate, as the Rust carrier `v58_16_runtime_retune_outside_the_rf_envelope_is_rejected` already shows by enabling a channel first. The test enables one.
- **A capture starts before the repeat.** `examples/minimal.py`'s capture starts at the Session's current instant, and the repeat 2 ms later (RS-19's lead, `x310-like`), so the first 2 045 samples are silence — what a timed device does. The Python carriers `sleep` past the repeat's start before they capture.
- **Clippy.** `large_enum_variant` on the reply types: the two Manifests in replies are boxed.
- **A mutation must not hang.** `ea_14_run_child` first asserted the refusal of a child with neither a duration nor a scheduled stop before the child with a scheduled stop; a mutation inverting the check then ran a child forever. The stopping child comes first now, so the inverted check fails fast (F25).

## Verification

| Check | Result |
|---|---|
| `cargo +stable test --workspace` | 684 passed, 0 failed (671 before Reviews H and I) |
| `cargo +1.85.0 test --workspace` | 684 passed, 0 failed |
| `python3 -m unittest discover -s python/tests` (GY-7) | 21 passed on Python 3.13.15 (numpy 2.5.3) and 3.9.6 (numpy 1.24.2), with `-W error::ResourceWarning` (12 before Reviews H and I) |
| `#[ignore]` in `crates/` | none |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean |
| `kernel_surface` `ov_23b` | 116 NEW / 292 public items (unchanged: the three methods are not items, GY-2) |
| `schema_freeze` | passes; no Kernel schema changed; `sink/capture_written_payload`, `server/request_frame` and `server/reply_frame` added (SCHEMA_CHANGELOG.md, Phase 6 entry) |
| Kernel direct dependencies | `schemars`, `serde`, `serde_json`, `sha2` |
| `Cargo.lock` | one workspace member added (`ezsdr-server 0.1.0`); no external package |
| mutations (spec 17 Appendix A, `tools/mutations.json`) | see "Mutations" |
| `check_links.py` | every link resolves |

## Mutations

`python3 plan/phase6/tools/mutate.py plan/phase6/tools/mutations.json <scratch>` with `EZSDR_PYTHON` set to an interpreter with numpy. Phase 5's tool, extended: a mutation with a `python` field runs that unittest id against the scratch copy's `ezsdr-server`, built after the mutation, so a Rust mutation can be killed by a Python carrier (F27) and a Python mutation by its own (P01–P08).

Run of 2026-09-26 on the tree of Step 4: **35 of 35 killed** — F01–F27 (Kernel, Sink, server) and P01–P08 (the Python package). Every baseline passed first.

One mutation was designed and dropped: the Python capture accepting an event from another recorder (`if event["source"] != source: continue` removed) is equivalent under every profile Phase 6 has, because a radio's receive port feeds one recorder; a profile with two recorders on two radios would distinguish it, and none exists yet.

## Review H

Adversarial dynamic review by Opus, brief [`prompts/review-h.txt`](prompts/review-h.txt), report [`reviews/review-h.md`](reviews/review-h.md). Verdict **CHANGES_REQUIRED**: 5 P0, 6 P1, 12 P2. Everything that existed passed; the defects were where no test reached, and all 14 of the reviewer's own mutations survived. Every finding was taken.

| Finding | Fix | Test (and mutation) |
|---|---|---|
| P0-1 RS-25a bound `sim.channel`, `sim.seed`, `sim.faults`, refusing §54's sweeps and §58 #14 inside a Session; spec 17's rejected alternative claimed no check reads `sim.channel` | The rule binds only the sections of checks that run at the runtime stage (`AdmissionCheckRegistry::runtime_sections`, crate-private) — the checks that judge what a running Session may do, `radio.rf_envelope` among them. RS-25a, KC-37a and spec 17's rejected alternative rewritten | `kf_03_rs_25a_refusals` gains a validate-only section (G07) |
| P0-2 `Rx.capture` could return another request's samples (a timed-out request, a raw `submit`) | The capture Sink numbers every capture request it receives; `CAPTURE_WRITTEN` and `REQUEST_REJECTED` carry `request`; `Session.submit` counts the captures it has admitted per recorder, and `capture` waits for its own number (HD-16, EA-17) | `hd_16_every_capture_request_is_numbered` (G10), `test_ea_17_each_capture_gets_its_own_samples` (P09, P15) |
| P0-3 input sizes that overflow crashed the server | Checked sum; a mismatch is `protocol` | `ea_14_refusals_before_a_child_runs` (G11) |
| P0-4 no Manifest when the Run ended on its way to T0, when a reply could not be written, on a panic | `impl Drop for Server` finishes a live Session and writes `manifest.json`; `connect` lets a Run that ended on its way to T0 fall through to the `ended` path, which writes it | `ea_15_every_exit_writes_the_manifest` (G14), `ea_10_a_run_that_ends_on_its_way_to_t0_writes_its_manifest` (G15) |
| P0-5 `wait_for`'s already-delivered answer skipped KC-29's Ended-first rule and KC-36's lease check | KC-29's prologue first | `kf_02_wait_for_answers_ended_first` (G05) |
| P1-1 a request that failed to decode ended the Session | Frame first (a JSON object, `body_bytes`, the body), decode second; a decode failure costs only that request (EA-5) | `ea_05_a_request_that_does_not_decode_costs_only_itself` (G13) |
| P1-2 the child took its host clock and checks from the caller's Assembly | `run_child` gives the child the parent's host clock and checks (KC-37a) | `kf_03_a_lease_that_expires_during_a_child_ends_the_child_first` (G08, G03), `kf_03_the_parents_checks_judge_the_child` (G09) |
| P1-3 an early `wait_for` left its no-op at the horizon | Cancelled on an early return (KC-29b) | `kf_02_wait_for_returns_the_first_match_and_withdraws_its_horizon` (G06, G01) |
| P1-4 a capture across a SampleClock change came back as one array | `samples()` refuses more than one domain (EA-17) | `test_ea_17_a_capture_across_a_rate_change_is_refused` (P12) |
| P1-5 H01–H05 | the cases above, plus `kf_02_wait_for_with_no_kinds_is_advance_to` (G02) and `kf_03_children_are_recorded_in_order` (G04). H05 (KC-37a's `check_entry` removed) is equivalent: for `RunChild`, `check_entry` checks only that the entry's time is on the local node, which a Session's `now` always is; the call stays, as in `submit`, for the day `RunChild` carries more | — |
| P1-6 H06–H14 | `ea_14_refusals_before_a_child_runs` (H06 as G11, H07's targeted `Stop`), `ea_13_*` gains `<uri>/../x` and `<uri>x` (G12), `ea_05_a_request_that_…` (H09 as G13), the Python cases (H10 as P09, H11 as P10, H12 as P11, H14 as P13). H13 (mixed channel counts) is equivalent: a capture across an `rx.channels` change is reachable (Review I), but each channel count comes with its own SampleClock, so the domain check refuses the artifact first; H07 as a mutation is killed only by the tool's timeout (the child runs forever), so the targeted-`Stop` case is a test without a listed mutation | — |
| P2-1 EA-3's field name | EA-3 says `kernel_api` | — |
| P2-2 §57's silence | `00-overview.md` §8 says why the carrier sleeps | — |
| P2-3 a duration that does not fit the child's clock | refused before the Kernel (EA-14) | `ea_14_refusals_before_a_child_runs` (G16) |
| P2-4 an artifact whose metadata write fails | HD-16 says it is not announced and the Sink's error fails the Run | — |
| P2-5 the capture timeout re-armed per skipped event | `wait_for` takes `until`, its reply `horizon`; `capture` keeps its first deadline | `ea_12_time_and_events` (G17) |
| P2-6 `close` after the server exited | falls back to the `manifest.json` every exit writes | `test_ea_16_close_after_the_server_exited` (P14) |
| P2-7 `v58_10` lacked `sim.faults` | added | — |
| P2-8 `design/03` §10 | now an owner decision before the freeze (KF-4) | — |
| P2-9 Appendix A's F05 and F21 rows | corrected | — |
| P2-10 no limit on `body_bytes` | recorded as a ceiling in the server (`ponytail:`) for Phase 7's listener | — |
| P2-11 `samples()` assumes cf32 | recorded as a ceiling in EA-17 | — |
| P2-12 `NotSession`'s message | generic | — |

The list after Review H: 60 mutations (F01–F27, P01–P16, G01–G17). A run over the first 59 killed 58: F25 only by the 900 s timeout, because its filter also matched the refusals test, whose targeted-`Stop` child then ran forever (the test is now `ea_14_refusals_before_a_child_runs`), and G01 survived, because no wait began with two matches already delivered (`kf_02_wait_for_returns_the_first_match_…` now ends with one). Re-run: F25, G01, G11, G16 and P16 killed. **60 of 60 killed.**

### Added at the owner's request during Review H: `Rx.request` / `Rx.result`

Asked whether two `capture` calls in a row lose samples: they do. `capture` returns once its artifact is written, which is known only when the block holding its last sample arrives, and the next request starts at the instant it is admitted (at 1 Msps on `x310-like`, `capture(1000)` twice gave samples 0–1 000 and 2 000–3 000). Two requests submitted before either is answered are served one after another from the end of the one before (HD-10) and are contiguous (4 000–5 000 and 5 000–6 000). The owner asked for the split: `Rx.request(n, at)` submits and returns a `CaptureRequest` carrying the recorder's request number (HD-16); `Rx.result(request, timeout)` waits for that number's artifact; `capture` is `result(request(n, at), timeout)`. EA-16's table has the two rows; `test_ea_17_requests_made_ahead_capture_contiguous_samples` takes the results in reverse order and checks one continuous stretch of the waveform across the boundary (mutation P16). v3 had the same split (`receiveRequestOnly` / `receiveResponseOnly`, `v3/client/ezsdr.py:253`, `:262`).

## Review I

The re-review of Review H's fixes (Opus), brief [`prompts/review-i.txt`](prompts/review-i.txt), report [`reviews/review-i.md`](reviews/review-i.md). Verdict **CHANGES_REQUIRED**: Review H's P0-2 not closed (P0-A), 3 P1, 7 P2; every other Review H finding closed; 60/60 listed mutations killed. Every finding was taken.

| Finding | Fix | Test (and mutation) |
|---|---|---|
| P0-A a capture routed to the recorder by another target path, or a `SetParameter` of `sink.capture_samples`, took a number `Session.submit` did not count | `Session.submit` counts what the Kernel routes (RS-14): a `sink.capture` for the only recorder when one is bound, else for the one it names; a `SetParameter` of `sink.capture_samples` on `sink/<recorder>`; EA-17 rewritten, its ceiling gone | `test_ea_17_each_capture_gets_its_own_samples` gains both routes (P19, P20) |
| P1-B an undecodable first frame did not end the handshake | before `hello`, a decode failure replies `protocol` and exits (EA-3) | `ea_03_handshake` (G18) |
| P1-C a capture across a gap came back as one array | `samples()` refuses a map with a gap, a channel gap or more than one valid stretch on a channel (EA-17) | `test_ea_17_a_capture_across_a_gap_is_refused` (P21) |
| P1-D N05, N06 | a Kernel-rejected capture takes no number; a foreign announcement inside the timeout does not restart it | the same test (P17); `test_ea_17_a_capture_keeps_its_first_deadline` (P18) |
| P2-1 a discarded request was never answered | the Sink answers each with `REQUEST_REJECTED` and its number (HD-11, HD-16) | `hd_16_a_discarded_request_is_answered` (G19) |
| P2-2 the `wait_for` fields' descriptions | each stands alone and says "exactly one"; the schema cannot say it without `oneOf`, which the frame types do not generate | — |
| P2-3 no prepare-only check in the test | `kf_03_rs_25a_refusals` gains one | G22 |
| P2-4 the `i64::MAX / 2` bound | its reason in EA-14 and the code; a duration just over it refused | `ea_14_refusals_before_a_child_runs` (G21) |
| P2-5 `ea_12` compared `horizon` only where it equals `now` | a wait that returns early is checked to echo its far horizon | `ea_12_time_and_events` |
| P2-6 stale "Checked by" lines | EA-17, KC-29b and KC-37a list every test | — |
| P2-7 `finish` lost the Manifest when it could not be written | returned with no `path` (EA-15) | `ea_15_a_manifest_that_cannot_be_written_is_still_returned` (G20) |

The reviewer's H09a and H10 are equivalent after the fixes (`Drop` disconnects; the request number filters older events), and N11 (`<=` in the cancellation) is equivalent (cancelling a callback that already fired returns false). The fixes are small and each is tested and mutation-guarded, so, by the owner's rule, there is no further review.

After Review I's fixes: **70 of 70 mutations killed** (F01–F27, P01–P21, G01–G22). G21 (the whole tick range admitted) is killed only by the tool's timeout, because the admitted duration then runs for 5·10¹⁸ ns; the refusal test itself is fast. Workspace 684 on 1.85.0 and stable, Clippy clean, Python 21 on 3.9.6 and 3.13.15, `ov_23b` 116 NEW / 292, every link resolved.
