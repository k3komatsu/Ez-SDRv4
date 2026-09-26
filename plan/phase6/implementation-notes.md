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
| `cargo +stable test --workspace` | 671 passed, 0 failed |
| `cargo +1.85.0 test --workspace` | 671 passed, 0 failed |
| `python3 -m unittest discover -s python/tests` (GY-7) | 12 passed on Python 3.13.15 (numpy 2.5.3) and 3.9.6 (numpy 1.24.2), with `-W error::ResourceWarning` on 3.13 |
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
