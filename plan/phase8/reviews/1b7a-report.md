STATUS: done

# Phase 8 1b-7a — implementation report

Implemented the held-burst cancellation rule from design note 36 decisions 1–2. `ezsdr_radio::CancelReason` and `cancellation_reports` are the shared policy: Stop, ProviderStop and Loss record only; ColdChange records and emits `radio.COMMAND_REJECTED`. The UHD Module's Stop now matches MockRadio. Provider-specific reason strings are unchanged.

## Plan revisions

- Adopted Sol's two majors: current/skipped-clock cold changes and loss assert reservation release; added parity coverage that stopping TX s1 leaves s0 transmitting.
- Corrected T14a: UHD already reports `UR-25: its transmit clock was never started`; it is a regression test, not a pre-implementation failure.
- Enumerated cancellation paths and added ordered booking barriers. Cancellation completion is observed through the owner record after join; fixed sleeps are not test oracles.
- T15b replays a later burst on the surviving clock; owner-level tests inspect the cancelled burst's exact shared reservation without racing UHD's asynchronous release.

## Changed files

Diff counts are additions/deletions. The other changes are confined to 1b-7a; no schema, Kernel type, version, changelog or D2/RX_OVERFLOW change.

| Path | Lines | Change |
|---|---:|---|
| `crates/ezsdr-radio/src/lib.rs` | +35 / −0 | Cancellation enum, policy function and its four-row unit test. |
| `crates/ezsdr-mock-radio/src/lib.rs` | +133 / −16 | Bool arguments replaced by reasons; one reporting helper; Stop/ProviderStop/Loss/ColdChange routing; reservation unit tests. |
| `crates/ezsdr-radio-uhd/src/provider/tx.rs` | +133 / −18 | `stop_now` takes a reason; one reporting helper; Stop, shutdown and both cold-change paths routed through it; reservation tests. |
| `crates/ezsdr-acceptance/tests/parity.rs` | +609 / −9 | Shared waveform/input and loss harness; 10 parity tests for T1–T4, T13–T15c with event, rejected-row, TX-block/burst and FakeDevice observations. |
| `crates/ezsdr-radio-uhd/tests/fake.rs` | +24 / −10 | Removed tautological booking assertion; asserted record-only device Stop and exact cold-change event/row outcomes. |
| `design/07-radio-model.md` | +3 / −1 | RM-16 cancellation rule, Test line and Changes row. |
| `design/18-uhd-radio.md` | +7 / −3 | UR-23 tests and UR-26 record-only Stop rule, Test line and Changes row. |
| `plan/maintenance/tools/mutations.json` | +129 / −9 | Retargeted F27; added P561–P572. |
| `plan/phase7/tools/mutations.json` | +2 / −2 | Retargeted R07 to the UHD report helper. |

MR-25 already accurately describes MockRadio recording cancelled bursts without implying an event, so it was not changed.

## Cancellation-site audit

- **MockRadio:** `stop_tx` and `cancel_later` receive the reason for stream/device Stop, Provider::stop, loss or cold change. The skipped-future-clock path in `tx_switch` passes ColdChange. All record/event selection goes through `record_cancelled_burst`, which calls `ezsdr_radio::cancellation_reports`.
- **UHD:** `TxCmd::Stop` passes Stop; shutdown passes ProviderStop; `do_switch` and `adopt_later` pass ColdChange. All record/event selection goes through `report_cancelled_burst` and the shared predicate.
- No cancellation call site chooses report policy or calls `reject_action_at`/`command_rejected` directly. Receipt refusals, late-policy drops and underflow handling remain out of scope and unchanged. `Stop` of one TX stream remains isolated.

## Tests and checks

- Added parity tests: `vh_8_tx_stream_stop_cancels_held_bursts`, `vh_8_device_stop_cancels_held_bursts`, `vh_8_provider_stop_cancels_held_bursts`, `vh_8_cold_change_cancels_held_bursts`, `vh_8_loss_cancels_held_bursts_without_rejection`, `vh_8_cold_change_cancels_a_burst_on_a_skipped_future_clock`, `vh_8_tx_stop_cancels_a_burst_on_a_future_clock`, `vh_8_rx_stop_does_not_cancel_held_tx`, `vh_8_tx_stop_allows_a_later_burst` and `vh_8_tx_stream_stop_leaves_the_other_stream_held`.
- Added the enum table unit test, Mock owner/shared-reservation tests and UHD current-clock, skipped-clock, Stop and loss-shutdown reservation tests. The UHD release-wait Stop test now expects no `COMMAND_REJECTED`; the fake device-Stop test verifies its action booking barrier, cancellation row, no event and no TX send.
- **Determinism:** all 10 new parity tests, both changed fake tests and five changed/new UHD TX unit tests passed in 20 consecutive runs each at `--test-threads=8`; the two Mock cancellation unit tests and Vocabulary table unit test also passed 20 runs each.
- **Rust tests:** `timeout 300 env RUST_TEST_THREADS=8 cargo test -p ezsdr-radio -p ezsdr-mock-radio -p ezsdr-radio-uhd -p ezsdr-acceptance` passed. A full-workspace run at `RUST_TEST_THREADS=4` had one existing timing-sensitive FakeDevice failure, `ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned`; an isolated rerun passed. Other full-suite runs also exposed wall-clock-sensitive FakeDevice tests under load; no unrelated tests were changed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Docker UHD 4.10 clippy with `--features uhd --all-targets`: passed; `cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`: 271 library tests and 5 API tests passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems.

## Mutation results

- Retargeted **F27** (Mock loss cancellation source) and **R07** (UHD cold-change reporting site); added **P561–P572** for both policy table outcomes and cancellation reasons at the call sites.
- The requested grep over all `plan/**/mutations.json` old/new strings (`stop_now|stop_tx|cancel_later|emit|emit_command_rejected|command_rejected|reject_action_at|record_rejected_action`) found 29 rows: 24 existing rows plus five new keyword matches. Unrelated rows were retained. For the 14 selected rows, every `old` matched once and every filter named an existing test.
- `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:3,sim03:3 F27 P561 P562 P563 P564 P565 P566 P567 P568 P569 P570 P571 P572 R07`: **14/14 killed**. P568 initially survived; its unit observation was strengthened to assert the event, the row was retargeted to the report decision, and the fix run killed it.
- Full mutation lists were not run; those remain the orchestrator's gate.

## Independent review and remaining holes

DeepSeek review rounds 1 and 2 both found **P1 none**. Adopted findings added reservation assertions, TX-stream isolation, T13 reason checks, positive Mock barrier observations, proof the RX Stop was accepted, Mock TX-block stats, corrected T14a wording, source-site auditing, and restored the prior UR-26 Test citations. P2/P3 suggestions about the UR-26 multiline Test row and F27 JSON indentation were also addressed.

Rejected two P3 suggestions with reasons: `Tx::shutdown(&mut self)` is private and only called by `Tx::run`, which immediately returns, so its one-shot `self` type provides no reachable runtime guard; the mutable receiver enables the required post-shutdown `held`/`later` assertions. UHD loss cleanup intentionally enters `Provider::stop`/shutdown, as impl2-brief.md explicitly directs, so it passes ProviderStop; DEVICE_LOST remains the loss signal and both reasons are record-only. No third review was run because round 2 found no P1.

**Remaining risk:** the pre-existing UHD fake suite has load-sensitive wall-clock tests; one full-workspace run failed `ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned`, which passed when isolated. The normal selected full mutation lists and hardware benches were not run.
