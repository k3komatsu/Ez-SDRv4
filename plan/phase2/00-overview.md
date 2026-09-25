# Phase 2 — Radio Model, Simulation Engine and MockRadio: overview and plan

| Field | Value |
|---|---|
| Status | **Accepted at Gate P** (owner, 2026-09-24; verdicts in §11). Specs 06–10 are binding for the implementer, and `20-implementation-plan.md` is the order of work. |
| Phase | Vision §67 Phase 2. Predecessor: Phase 1 (Kernel semantic model, accepted 2026-09-23). Successor: Phase 3 (SimulationChannel + deterministic Runs). |
| Scope | The Kernel **coordinator** that drives a Run end to end in the Simulation class; the Kernel **amendments** Phase 1 left for the first real Module (§5); the **Radio Model** Vocabulary; the **Simulation** and **Sink** Vocabularies; the **Simulation Engine**; **MockRadio**; the host-memory Link and the capture Sink that a MockRadio stream needs to reach an artifact; the Phase 2 half of Vision §58. |
| Not in scope | §3 of this file lists it. In one line: no SimulationChannel, no RealtimeEmulation, no hardware, no Python, no Reactor or Processor executor, no child Runs. |
| Language | English, like Phase 1. Code, rule text, test names and schema descriptions share one vocabulary. |
| Location | `plan/phase2/` while drafting. On acceptance at the end of Phase 2 (§9, step X), specs 06–10 move to `design/`; this file and `20-implementation-plan.md` stay here as the record. |
| Modal verbs | "must" and "must not" are normative (OV-4a). "Should" does not appear inside a rule. |

Documents:

```text
plan/phase2/
  00-overview.md            this file: scope, decisions, crates, governance, gates, traceability, exit criteria
  06-kernel-coordinator.md  KA-n  amendments to the accepted Phase 1 specs
                            KC-n  the Run coordinator (Kernel), Simulation class
                            UC-n  what each update class means
  07-radio-model.md         RM-n  the `radio` Vocabulary: resource kinds, keys, envelopes, events, verbs, the RF-envelope check
  08-simulation.md          SE-n  the `sim` Vocabulary (seed, fault schedule, PRNG) and the Simulation Engine Module
  09-mock-radio.md          MR-n  the MockRadio Module: profiles, coercion, streams, bursts, faults, stop, sections
  10-host-data-path.md      HD-n  host memory and its pool, the host Link Module, the `sink` Vocabulary, the capture Sink Module
  20-implementation-plan.md       the ordered steps for the implementer, with files, signatures, tests and commands
  patches/01-kernel-amendments.patch  the Kernel amendments of spec 06 §2, as a git patch against 96976c5 (plan step 1)
  reviews/planning-reviews.md     the three adversarial reviews of these documents before Gate P, and each finding's verdict
  prompts/                        the prompts for the implementing agent (three sessions), for Reviews K and M, and for fixes
```

---

## 1. Why Phase 2 exists

Phase 1 produced decisions and a Kernel crate whose tests run against test doubles. Vision §67's Phase 2 adds the first real Modules — a Radio Model, a discrete-event Simulation Engine and an envelope-enforcing MockRadio — so that an experiment runs **entirely in software** and leaves a Manifest.

The measure of success is Vision §59's claim, stated one phase early: an experiment written against MockRadio contains nothing Mock-specific, and the same Spec later runs on a USRP by changing only the BindingProfile. Phase 2 cannot prove the second half, but it must make the first half true and must not make the second half impossible.

Phase 1's own history (`plan/phase1/history.md` §1) is the second reason this document is long. Twenty-five review passes over the Phase 1 crate found rules that were **settled by review rather than written down**: a correct function nobody called, a rule that refused what it was meant to allow the first time real data reached it, and a fix that added code beside the code it was meant to replace. Phase 2 therefore writes every rule, every refusal and every call site down **before** implementation, and hands the implementer an order of work in which each step ends in a test.

## 2. What Phase 2 found in Phase 1 before writing a line of code

Reading the Phase 1 crate against what the first real Module needs found twenty-two places where the accepted specs or the code cannot carry Phase 2. They are listed with evidence in `06-kernel-coordinator.md` §2 as KA-1…KA-22. The five that change what a Module can do at all:

| # | What Phase 1 has | Why the first Module cannot work with it |
|---|---|---|
| KA-1 | `PrepareContext<'a>` hands `&'a dyn TimeAuthority`, `&'a dyn EventSink`, `&'a dyn ActionReceiver` (`src/module_api.rs:949-969`) | The references die when `prepare` returns, and `Box<dyn Provider>` is `'static`, so no Module can read the time, emit an event or receive an Action in `step`, `start` or `stop`. The test doubles never keep a handle (`tests/support/doubles.rs:470`), which is why no test noticed. |
| KA-3 | Host bytes are reached through `HostMemoryAccess::map_host(&self, &'a BlockRef) -> &'a [u8]`, implemented by a link (`src/stream/buffer.rs:30`) | `Endpoint::StreamIn` holds `Arc<dyn DataLink>`, which has no `map_host`, so a consumer has no route to the bytes; and a slice borrowing the block cannot be produced in safe Rust from a pool that owns the bytes. The test link leaks its buffers and uses `unsafe` (`tests/support/mod.rs:146-155`). |
| KA-4 | Admission checks receive one flat `Map<Key, Value>` (`src/binding.rs:357`) | Two radios both carry `radio.tx.gain_db`; the flat map keeps one, and the RF safety envelope silently skips the other device. |
| KA-5 | `validate` calls `coerce` only when a key is not satisfied directly, and never reads `CoerceReport.rejected` (`src/plan/matching.rs:151-166`) | A Provider has no way to refuse a **combination** — four channels × 200 Msps over a 1.25 GB/s transport — which is Vision §13 rule 1's PerformanceEnvelope refusal. |
| KA-6 | Nothing calls `Provider::coerce` for a runtime parameter change (`plan/phase1/00-overview.md` §11, the observation after D50) | A Session's `rx.sample_rate_hz = 19.5e6` is dispatched uncoerced, and §58 #16's PerformanceEnvelope refusal of a runtime rate change has no mechanism. |

KA-1…KA-22 are applied in the first implementation steps, before any Module is written, with their own tests. They change the accepted specs `design/01…05`, which is allowed before the v4.0 freeze and is recorded in each spec's own text (PO-9).

## 3. Scope

### In scope

1. **Kernel amendments** KA-1…KA-22 (spec 06 §2).
2. **The coordinator** (spec 06 §3–§9): the Kernel component that assembles a Run from documents and Module instances, drives `validate → plan → prepare → arm → start → running → stop → cleanup`, runs the stepping loop in the Simulation class, admits and dispatches every Action, applies the Policy table, and writes and seals the Manifest. Spec Runs and Sessions.
3. **Update-class semantics** UC-1…UC-6 (spec 06 §10).
4. **Radio Model** `radio` 1.0.0 (spec 07): resource kinds, the device tree shape, keys, the TimingEnvelope and PerformanceEnvelope as capabilities, event kinds with defaults and payloads, Session verbs, the `radio.rf_envelope` admission check.
5. **Simulation Vocabulary** `sim` 1.0.0 and the **Simulation Engine** `ezsdr.sim-engine` 1.0.0 (spec 08).
6. **MockRadio** `ezsdr.radio.mock` 1.0.0 with profiles `x310-like` 1.0.0 and `ideal` 1.0.0 (spec 09).
7. **Host data path** (spec 10): host memory and a buffer pool, the Link Module `ezsdr.link.host` 1.0.0, the `sink` Vocabulary 1.0.0 and the capture Sink `ezsdr.sink.capture` 1.0.0.
8. **Acceptance tests** for the Phase 2 half of Vision §58 and three of the four v3 behaviours of Vision §61 (§8 below, and `20-implementation-plan.md` step 15).

### Out of scope, with the phase that owns each

| Item | Why not now | Owner |
|---|---|---|
| SimulationChannel (loopback, gain, delay, AWGN), TX sample content reaching an RX | Vision §67 Phase 3 | Phase 3 |
| MockRadio clipping at full scale (Vision §23) | With no channel, no RX sample depends on any input, so there is nothing to clip | Phase 3 |
| Random LO phase after an untimed retune (§26) | Only observable through a channel | Phase 3 (the behaviour is **declared** now, RM-9) |
| RealtimeEmulation, HardwareInLoop, Hardware classes | Need a wall-paced Engine, Executor and Sink threads (MA-30's table), or a device timekeeper | first phase that needs real deadlines; Phase 7 for hardware |
| Link-fed TX (a Processor feeding a radio's TX port), `TX_UNDERFLOW` and the Provider half of `TX_DISCONTINUITY` (SC-24a, SC-25) | Needs a Processor that produces TX blocks | Phase 10, or the first Provider with a TX port |
| Reactor and Processor executors | Vision §67 Phases 5 and 10. The coordinator implements `ActionSubmitter` and Executor stepping, tested with doubles | Phases 5, 10 |
| Child Runs (`RunChild`, RS-25, RS-25a) | A child Run needs its Spec and profile by hash, which needs a document store nothing in Phase 2 produces | Phase 6 (`sdr.run(spec)`) |
| Python client, YAML | Vision §67 Phase 6 | Phase 6 |
| SigMF and raw-IQ Sinks, an artifact store beyond `mem:` and `file://` | Vision §67 Phase 4 | Phase 4 |
| Fault kinds other than `rx_overflow`, `rx_sequence_error`, `device_lost` | These three are what SC-18, MA-9 and §58 #5, #6 need now; spec 04's scope line is amended accordingly (KA-22) | Phase 4 |
| Session replay (RS-20's re-application) | Needs the waveform bytes by hash, i.e. the artifact store | Phase 6 |
| MA-39's budget-feasibility arithmetic | Needs a Processor with declared port rates | Phase 10 |
| Peripheral timing classes and latency emulation (§38, MA-17) | No Peripheral exists | Phase 9 |

A request to add any of these during Phase 2 is scope creep and is refused (AGENTS.md §6).

## 4. Cross-cutting decisions

Each row is open to reversal at Gate P. A reversal is recorded in §11.

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| Y1 | Execution classes in Phase 2 | **Simulation only.** The coordinator refuses a plan whose class is anything else, naming Phase 2 (KC-2) | Implementing RealtimeEmulation now (MA-30 requires Executors and Sinks on threads, a whole second driving model for no Phase 2 test); silently running a WallPaced Run free-running (a class that lies, MA-41) | The refusal is one match arm; a later phase adds the threaded driver |
| Y2 | Where the coordinator lives | In `ezsdr-kernel`, module `coordinator` | A separate `ezsdr-runtime` crate (Vision §5 puts the Run lifecycle and transaction in the Kernel; OV-8 names no split trigger that has occurred) | OV-8's triggers still apply |
| Y3 | Crate layout | Ten crates (§5): the Kernel, four Vocabulary or support crates, four Module crates, one acceptance-test crate | One crate for everything (MA-3 forbids a Module depending on another Module, so Modules cannot share a crate); one crate for all Vocabularies (Vision §5 versions each Vocabulary separately, and a crate is Cargo's version unit) | Crates are added, never merged, at a later phase |
| Y4 | Dependencies | No crate outside the workspace is added to `Cargo.lock`. Phase 2 crates may depend on `ezsdr-kernel`, on each other as §5 allows, and on `serde`, `serde_json` and `schemars`, already in the lock (PO-4) | `rand` (SplitMix64 is 10 lines, SE-6); `crossbeam` (a mutex queue suffices at Simulation speed, HD-5); `tempfile` in tests (std's `temp_dir` plus a unique name suffices) | OV-18's rule governs any later addition |
| Y5 | The resource a Spec binds | A **device**: one `radio.device` node per Provider instance, whose ports are `rx` (and, later, `tx`); the streams are sub-paths `<device>/rx`, `<device>/tx` (RM-2) | A resource per stream (a Session's implicit Spec binds the instance's root, SB-T2 row 3, so a Spec and a Session would name different nodes for one radio); a resource per channel (per-channel keys would need 64 key declarations each) | Per-channel configuration is an additive key later (RM-5's ceiling) |
| Y6 | Where the TimingEnvelope and PerformanceEnvelope live | As **capabilities** of the device node under `radio.timing.*` and `radio.perf.*`, so that `validate()` sees them and a Spec may constrain them with the generic matcher; enforced by the Provider's `coerce` (validate), `prepare` (prepare), the Kernel's runtime coerce (KA-6) and the Provider itself (runtime). The Kernel reads exactly one envelope number, `ProviderInstance.min_command_lead` (KA-7) | An envelope admission check (SB-29 checks see an environment section and a configuration, never a Provider's declaration, so the check could not know the envelope); the Kernel reading `radio.timing.*` (OV-21) | The one Kernel number is what RS-19 and SC-27's plan-time rule need; nothing else |
| Y7 | What MockRadio receives when no channel exists | A deterministic **test pattern**: zeros by default, or an index ramp on request (MR-13); RF fidelity `none` | Noise (needs a model; Phase 3); refusing to stream (then no Phase 2 test has data) | Phase 3's SimulationChannel replaces the pattern as the RX source |
| Y8 | How MockRadio transmits | `TxBurst` Actions only, whose waveform length it reads from `ArtifactRef.size_bytes`; the waveform's bytes are not read (MR-16) | A TX port fed by a link (needs a producer, Phase 10) | Phase 3 reads the bytes when the channel needs them |
| Y9 | Fault injection | Three faults in the `sim.faults` environment section, read by MockRadio for its own resource (SE-4, MR-20) | A separate FaultInjector Module (it would have to reach MockRadio, which MA-3 forbids); faults in the selector (Vision §17 and §58 #14 put them in `environment`) | Phase 4 adds kinds to the same section |
| Y10 | Determinism | One logical thread; ordered maps only; one PRNG (SplitMix64) seeded from `sim.seed` and the stream name (SE-6, PO-11) | Threads per Module in Simulation (MA-30 forbids); `HashMap` anywhere in output-affecting code | none |
| Y11 | Virtual time | One virtual Root at 1 GHz; `host.monotonic` advanced in lockstep, tick for tick (SE-10) | A root per simulated device (two MockRadios would then be related only by a `ClockRelation`, with uncertainty, and Phase 3's channel could not place a TX sample on another device's RX grid exactly) | Per-device drifting roots are Phase 3+ (TM-16b allows them) |
| Y12 | Artifacts | The capture Sink writes files under its selector's `dir`; a Session waveform is kept in the coordinator's memory under `mem:sha256:…`; a Spec's waveform reference must be `mem:` or `file://` and is verified (KC-9) | An artifact store Module now (Phase 4) | Phase 4 |
| Y13 | Envelope values of `x310-like` | Provisional, from UHD documentation and the audit, each marked VERIFIED or INFERRED in MR-3; **where uncertain, the stricter value** | Data-sheet values presented as measured (§59: values come from measurement) | Phase 8 measures and bumps the profile version |
| Y14 | Child Runs | Refused in Phase 2 with a logged rejection naming Phase 6 (KC-37) | Implementing them (needs a document store by hash) | Phase 6 |

## 5. Crate layout

```text
Ez-SDRv4/
├── Cargo.toml                          [workspace] members = the ten crates below
├── crates/
│   ├── ezsdr-kernel/                   Kernel (existing). New: src/coordinator/  (spec 06)
│   ├── ezsdr-radio/                    Vocabulary `radio` 1.0.0                  (spec 07)
│   ├── ezsdr-sim/                      Vocabulary `sim` 1.0.0                    (spec 08 §2)
│   ├── ezsdr-sink/                     Vocabulary `sink` 1.0.0                   (spec 10 §4)
│   ├── ezsdr-hostmem/                  host memory domain, pool, sample layout   (spec 10 §2)
│   ├── ezsdr-sim-engine/               Module ezsdr.sim-engine 1.0.0, Authority  (spec 08 §3)
│   ├── ezsdr-mock-radio/               Module ezsdr.radio.mock 1.0.0, Provider   (spec 09)
│   ├── ezsdr-link-host/                Module ezsdr.link.host 1.0.0, Link        (spec 10 §3)
│   ├── ezsdr-sink-capture/             Module ezsdr.sink.capture 1.0.0, Sink     (spec 10 §5)
│   └── ezsdr-acceptance/               tests only, publish = false               (§8, plan step 15)
├── schemas/                            Kernel schemas (existing) + schemas/radio/, schemas/sim/ (PO-7)
└── plan/phase2/
```

Allowed dependencies (PO-8 enforces the Module rows by test):

| Crate | Tier | May depend on |
|---|---|---|
| `ezsdr-kernel` | Kernel | `serde`, `serde_json`, `schemars`, `sha2` (X6, unchanged) |
| `ezsdr-radio`, `ezsdr-sim`, `ezsdr-sink` | Vocabulary | `ezsdr-kernel`, `serde`, `serde_json`, `schemars` |
| `ezsdr-hostmem` | support (Vocabulary content: the `host` memory kind, SC-6) | `ezsdr-kernel` |
| `ezsdr-sim-engine` | Module | `ezsdr-kernel`, `ezsdr-sim`, `serde_json` |
| `ezsdr-mock-radio` | Module | `ezsdr-kernel`, `ezsdr-radio`, `ezsdr-sim`, `ezsdr-hostmem`, `serde`, `serde_json` |
| `ezsdr-link-host` | Module | `ezsdr-kernel` |
| `ezsdr-sink-capture` | Module | `ezsdr-kernel`, `ezsdr-sink`, `ezsdr-hostmem`, `serde_json` |
| `ezsdr-acceptance` | tests | every crate above |

Every non-Kernel crate may take `ezsdr-kernel = { path = "../ezsdr-kernel", features = ["testing"] }` as a **dev**-dependency, for `ManualTimeAuthority`. No crate takes a Module crate as a dev-dependency except `ezsdr-acceptance` (PO-8).

## 6. Governance

OV-1…OV-23b bind Phase 2's documents and crates exactly as they bound Phase 1's. The rules below add what ten crates need.

- **PO-1** Phase 2 rule IDs are `KA-n`, `KC-n`, `UC-n`, `RM-n`, `SE-n`, `MR-n`, `HD-n` and `PO-n`, protected by OV-1 and split by OV-2. An amendment to a Phase 1 rule keeps that rule's ID; a new Phase 1-tier obligation takes a lettered ID after the rule it extends (`MA-5a`), never a new number in a Phase 1 series. *Process obligation.*
- **PO-2** Every Phase 2 crate's `src/lib.rs` begins with `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`, and every package uses `edition.workspace`, `rust-version.workspace` and `license.workspace`. *Checked by `po_02_every_crate_forbids_unsafe_code` (acceptance crate), which reads each member's `src/lib.rs`.*
- **PO-3** Tests follow OV-19 in each crate's own `tests/` directory: plain `#[test]`, named with the lowercased rule ID they prove. A test's assertion is the rule's obligation, not the implementation's (OV-3). *Process obligation.*
- **PO-4** `Cargo.lock` gains no package from outside the workspace. *Checked by `po_04_the_lock_gains_no_external_package` (acceptance crate), which lists every `[[package]]` without a `source` line absent from the workspace members and every external package absent from the 2026-09-24 lock.*
- **PO-5** Every new public item of `ezsdr-kernel` is on `tests/kernel_surface_allow.txt` with an audit §13 token or a `NEW:` justification, and its doc comment cites a rule ID (OV-23). The exit review reports the new `NEW:` count. *Checked by `kernel_surface`.*
- **PO-6** Every change to a committed Kernel schema regenerates it with `EZSDR_UPDATE_SCHEMAS=1 cargo test -p ezsdr-kernel --test schema_freeze` and adds one line to `schemas/SCHEMA_CHANGELOG.md` naming the KA it applies. *Checked by `schema_freeze`.*
- **PO-7** The document types a Vocabulary defines — `radio.rf_envelope`'s section, the Radio Model's envelope document and event payloads (RM-20, RM-22), `sim.faults`' entries and the seed (SE-12), the `sink` Vocabulary's event payload (HD-14) — have committed JSON Schemas under `schemas/<vocabulary>/`, generated with the Kernel's `schema::generator()` and `schema::render()`, and frozen by that crate's own `<prefix>_schema_freeze` test in both directions (OV-22). *Checked by `rm_20_schema_freeze`, `se_12_schema_freeze` and `hd_14_schema_freeze`.*
- **PO-8** A Module crate depends on `ezsdr-kernel`, on Vocabulary and support crates, and on nothing that is a Module crate, as a normal or a dev-dependency (MA-3). The four Module crates are `ezsdr-sim-engine`, `ezsdr-mock-radio`, `ezsdr-link-host` and `ezsdr-sink-capture`. *Checked by `ma_03_no_module_crate_depends_on_another` (acceptance crate), which runs `cargo metadata --format-version 1 --no-deps` and reads each Module's dependency list. This discharges MA-3's producer marker.*
- **PO-9** An amendment to an accepted Phase 1 spec is applied in `design/0N-*.md` verbatim from 06 §2, with a trailing *(Phase 2, KA-n)*, in `20-implementation-plan.md` step 2 — right after step 1 applies the code of most KAs, and before the coordinator steps implement KA-6, KA-9, KA-12, KA-13 and KA-18, whose text therefore precedes their code by at most six steps; the Phase 1 markers that name a Phase 2 test are rewritten in step 16, when the tests exist. The Vision is not edited during Phase 2 (OV-6); its issues are collected in each spec's "Vision issues found" section and applied at step X (§9). *Process obligation.*
- **PO-10** Each Phase 2 spec gets a per-rule disposition table under `plan/phase2/exit-review/` in the format of `plan/phase1/exit-review/README.md`, read from test bodies (OV-3). *Process obligation; exit criterion 2.*
- **PO-11** Output-affecting Phase 2 code iterates `BTreeMap`/`BTreeSet`/`Vec` only, never `HashMap` or `HashSet`; reads the wall clock only through `HostClock` for the UTC stamps RS-5 records and for the Lease; spawns no thread except `run_cleanup`'s; and draws randomness only from `SimRng` (SE-6). *Checked by `po_11_no_hashmap_and_no_wall_clock_in_simulation_code` (acceptance crate), a text scan of the Phase 2 code — every `.rs` file under `src/` of the nine new crates and under `crates/ezsdr-kernel/src/coordinator/` — for the substrings `HashMap`, `HashSet`, `SystemTime`, `Instant::now`, `thread::spawn` and `rand::`, outside `//` comments. The Phase 1 Kernel files keep their three legitimate uses: `RunId::generate` (`id.rs`, a Run id is not output-affecting and the determinism projection removes it), `SystemHostClock` and `run_cleanup`'s step threads (`run.rs`), and `ManualTimeAuthority`'s host base (`time/authority.rs`, behind `testing`).*
- **PO-12** Every new refusal has a test whose assertion is that refusal, and the implementer disables it once (for example `if false && …`) and confirms the test fails before committing the step. Each such check is listed in the step's done-list in `20-implementation-plan.md`. *Process obligation; the Phase 1 lesson "新しい拒否は1件ずつ無効化して，テストが落ちることを確かめる".*

## 7. Test strategy

- Kernel coordinator tests live in `crates/ezsdr-kernel/tests/coordinator.rs` and use `tests/support/` doubles, extended as `20-implementation-plan.md` step 3 lists: a shared `Probe`, a `SimAuthority` over `ManualTimeAuthority`, a `SteppedProvider` wrapping the Phase 1 `TestProvider`, a `RecordingSink`, a `ProbeExecutor` and a `TestLinkModule` over `MemLink`.
- Each Vocabulary and Module crate tests itself in isolation. A Module crate's tests may use only the Kernel (with `testing`) and the Vocabulary and support crates it depends on — never another Module (PO-8). MockRadio's tests therefore drive it with `ManualTimeAuthority`, not the Simulation Engine.
- End-to-end behaviour is proved only in `ezsdr-acceptance`, which assembles real Modules with the coordinator. Its tests are the §58 carriers.
- Determinism is compared on a **projection** of the Manifest that removes the fields whose values legitimately differ between two identical Runs: `run.id`, every `host_utc_nanos`, `artifacts[*].uri` and `hash` (plan step 15 defines `determinism_projection`).

## 8. Traceability

### Vision §58 → Phase 2

| # | Test | Phase 2 carrier (acceptance crate unless named) | Remaining |
|---|---|---|---|
| 1 | Binding substitution | `v58_01_one_spec_two_mock_profiles`: one Spec, profiles `x310-like` and `ideal`, no Spec edit | complete with UHD (Phase 7/8) |
| 2 | Virtual time faster than wall clock | `v58_02_ten_virtual_seconds_run_faster_than_wall_clock` | — |
| 3 | Deterministic with a seed | `v58_03_same_seed_same_manifest_projection`, `v58_03_the_seed_changes_block_boundaries_not_data` | Reactor decisions (Phase 5) |
| 4 | Events flow through the hardware path | `v58_04_mock_events_reach_counters_policy_and_manifest` | — |
| 5 | Fault injection triggers cleanup and policy | `v58_05_device_lost_aborts_with_full_cleanup` | more fault kinds (Phase 4) |
| 6 | Gaps equal a UHD overflow | `v58_06_injected_overflow_is_a_uhd_overflow`, `v58_06_sequence_error_is_seq_discontinuity` | Phase 8 re-measures |
| 7 | Runs record Spec, Binding, plan, events, artifacts | `v58_07_manifest_records_every_input_and_output` | — |
| 8 | Two MockRadios through a SimulationChannel | — | Phase 3 |
| 9 | Reactive TxBurst | — | Phase 5 |
| 10 | No Mock-specific API in application logic | `v58_10_experiments_name_no_mock_type` (source scan of `ezsdr-acceptance/src/experiments.rs`) | — |
| 11 | Mock enforces the envelope | `v58_11_short_lead_burst_is_a_time_error`, `v58_11_19_5_msps_is_coerced_to_20`, `v58_11_beyond_the_performance_envelope_is_rejected_at_validate` | — |
| 12 | Block-size independence | `v58_12_jitter_leaves_the_capture_unchanged` (the capture Sink is the consumer) | a Processor (Phase 10) |
| 13 | Sessions leave provenance | `v58_13_session_manifest_has_log_waveform_and_capture` | Python (Phase 6) |
| 14 | Environment portability | `v58_14_profiles_differing_only_in_environment_both_run` | — |
| 15 | TX bursts closed and contiguous | `v58_15_repeat_wraps_without_a_gap` (acceptance), `mr_24_a_bypassing_provider_gets_time_error` (MockRadio crate) | a time jump inside a burst needs link-fed TX (Phase 10) |
| 16 | Session Actions are admitted | `v58_16_runtime_retune_outside_the_rf_envelope_is_rejected`, `v58_16_runtime_rate_beyond_the_envelope_is_rejected` | hardware (Phase 7) |

### Vision §61 v3 behaviours → Phase 2

| Behaviour | Carrier | Evidence |
|---|---|---|
| Repeat is continuous across the wrap | `v61_01_repeat_is_continuous_across_the_wrap` | `v3/source/device/package.d:111-126` |
| Capture at a requested sample index | `v61_02_capture_starts_at_the_requested_sample_index` (with the ramp pattern, the first captured sample *is* its index) | `v3/source/controller/cyclicrx.d:84-145` |
| Timed TX/RX start at device time | `v61_03_timed_start_of_tx_and_capture` | `v3/client/ezsdr.py:115-119` |
| Shared 10 MHz + PPS, PPS source armed first | `v61_04_pps_source_is_armed_first_and_streams_align` (two MockRadio instances, `arm_after`) | `v3/changelog/v3.0.20.md:56-59` |

### D109's raised items

| Item | Resolution |
|---|---|
| TimingEnvelope and PerformanceEnvelope checks on Session Actions; the plan-time lead check of a statically known burst | Y6 and KA-6 for Session Actions; KC-19 (the coordinator's `RejectAtPlan` lead check at arm, from KA-7's `min_command_lead`) for scheduled bursts |
| Update-class semantics | UC-1…UC-6 (spec 06 §10) |
| Mock obligations with no rule: length jitter, clipping, same late policy, §38 latency emulation, same coercion report | MR-12 (jitter); clipping → Phase 3 (§3); MR-17 (the same `LatePolicy::decide` with the profile's lead); §38 → Phase 9 (§3); MR-6 and MR-8 (the same `coerce` at `validate`, `prepare` and runtime) |

### Phase 1 forward, producer and consumer markers

Every Phase 1 rule whose marker names Phase 2, MockRadio or "the Phase 2 coordinator", with the Phase 2 rule that carries it. A row marked *re-marked* moves the obligation to a later phase with a reason; the Phase 1 rule text is amended accordingly (PO-9, KA-19).

| Phase 1 rule | Marker | Phase 2 carrier |
|---|---|---|
| TM-13a | forward (allocation at prepare) | KA-2: the Provider declares through `PrepareContext.clocks`; MR-10 |
| TM-13b | producer (receive origin) | MR-11 |
| SC-5 | producer (wire format in the PerformanceEnvelope) | RM-7 (`radio.perf.wire_bytes_per_sample`), MR-3 |
| SC-9 | producer (pool, no allocation) | HD-2 (pool, slot reuse test); the copy-regression benchmark stays Phase 8 |
| SC-13 | producer (the flag reflects reality) | MR-20 |
| SC-15 | consumer (no fixed length) | MR-12 (jitter), HD-12 (the capture Sink slices any length) |
| SC-16a | producer (`LATE` on receive) | MR-11: a late start is refused, so no block carries `LATE` — stricter than a late start (Y13) |
| SC-18 | producer (overflow shape) | MR-20, MR-21 |
| SC-20 | forward (collector reads drop counts) | KA-18 |
| SC-24a | forward (MockRadio) | *re-marked* to the first Provider with link-fed TX (§3) |
| SC-26 | producer (no underflow at the wrap) | MR-16 |
| SC-28, SC-29 | Provider half | MR-16, MR-23 |
| SC-31a | producer (setting `ALIGNMENT`) | unchanged: MockRadio never drops a channel; Phase 7 |
| SB-23 | producer (`instances: 2`) | MR-2 |
| SB-30 | forward (third point's call site) | KC-24 |
| SB-41 | forward (the prepare loop) | KC-12 |
| SB-43 | forward (arm resolution) | KA-8, KC-16…KC-18 |
| RS-1a, RS-5, RS-10, RS-11a, RS-18, RS-23 | forward | KC-5, KC-43, KC-40, KC-44, KC-24, KC-36 |
| RS-4 | forward (re-plan entry) | *re-marked*: Phase 2 has no operation that changes a plan; the call site is the first re-plan or GraphEpoch API |
| RS-31 | producer (non-Kernel payload schemas) | RM-22 (KA-22) |
| MA-17 | producer (a Peripheral's timing class) | *re-marked* to Phase 9 (KA-19): MockRadio has no Peripheral |
| MA-10 | a Phase 2 value (the memory domain a port delivers from) | RM-2 (host memory in 1.0.0); MR-13 |
| MA-19b | producer (an Executor loads components) | *re-marked* to Phase 5 (KA-19): Phase 2 has no Executor Module |
| RS-25, RS-25a | forward (child Runs) | *re-marked* to Phase 6 (Y14, KC-37) |
| MA-3 | producer (workspace check) | PO-8 |
| MA-7, MA-9, MA-19a, MA-24 (delivery), MA-26, MA-27a, MA-29 | forward | KC-13, KC-30, KC-11, KC-25, KC-41, KC-10, KC-3 |
| MA-8 | producer (budget) | MR-7, HD-11 (both finish `prepare` without blocking) |
| MA-13 | producer (orderly tail) | MR-25 |
| MA-24 (applying) | producer | UC-2…UC-6, MR-18 |
| MA-28a | producer (Link implements its policy) | HD-5 |
| MA-39 | forward (budget arithmetic) | *re-marked* to Phase 10 (§3) |
| D20 | `Lease::validate` has no caller | KC-35 |

### Audit §14.1 item 4 (TimingEnvelope, coercion, late policy, fidelity vector — "envelope contents are Phase 2")

RM-6…RM-8 (contents), MR-3 (values), MR-6 (coercion), MR-17 (late policy), MR-4 (fidelity).

## 9. Sequencing and gates

| Step | What | Gate |
|---|---|---|
| P | The owner reads this file's §4 and §11, then specs 06–10, then `20-implementation-plan.md` | **Gate P** — acceptance of the plan. Nothing is implemented before it |
| 1 | The Kernel amendment patch (`patches/01-kernel-amendments.patch`: every KA with code outside the coordinator, with its tests) | 361 tests green on 1.85.0 and stable; clippy clean |
| 2 | The new rule text of every KA in `design/01…05` (PO-9), including KA-19's markers | every link resolves |
| 3 | The test doubles the coordinator tests need | the suite stays green |
| 4–8 | The coordinator, with doubles (KC-1…KC-45, KA-6, KA-9, KA-12, KA-13, KA-18) | after step 8: **Review K** — one adversarial pass (Opus, AGENTS.md §8) over the Kernel diff |
| 9 | The nine new crates as empty workspace members | the workspace builds |
| 10–14 | `sim` and the Simulation Engine; `radio`; host memory, the host Link and `sink`; the capture Sink; MockRadio | each crate green in isolation |
| 15 | The acceptance crate | **Review M** — one adversarial pass over the Modules and the acceptance tests |
| 16 | Exit review tables (PO-10), `handoff.md`, Vision issues collected | **Gate X** — owner acceptance of Phase 2 |
| X | Specs 06–10 move to `design/`; the Vision's Phase 2 issues are applied with owner approval (OV-6's procedure, as Phase 1 §12 did) | — |

A review pass's findings are recorded in §11 with a verdict, never silently applied (OV-5).

## 10. Exit criteria

1. Specs 06–10 accepted at Gate X and every §4 and spec decision row has a verdict in §11.
2. Every Phase 2 rule has an OV-3 disposition in `plan/phase2/exit-review/`, read from test bodies (PO-10), with no `GAP` and no `UNCERTAIN`.
3. `cargo test --workspace` passes on Rust 1.85.0 and on stable, with no `#[ignore]`; `cargo +stable clippy --workspace --all-targets -- -D warnings` passes.
4. Every acceptance test of §8's two tables that names a Phase 2 carrier exists and passes.
5. `kernel_surface`, `schema_freeze` and every Vocabulary freeze test pass; `SCHEMA_CHANGELOG.md` names every KA that changed a schema; the new `NEW:` count is recorded.
6. The Kernel's direct dependencies are still exactly X6's four; `Cargo.lock` gains no external package (PO-4).
7. Every Phase 1 marker in §8's marker table is either carried by a passing Phase 2 test or re-marked in the Phase 1 rule's own text (PO-9).
8. Every link in `design/` and `plan/` resolves, and the `v3/` path check of `handoff.md` §1 passes.

## 11. Decision log

Filled in at Gate P and after each review. One row per decision the owner confirmed or reversed.

| Decision | Gate | Verdict | Note |
|---|---|---|---|
| Y1–Y14 (§4) | P | **accepted** | owner, 2026-09-24, as recommended |
| Spec 06 decisions K1–K12 | P | **accepted** | owner, 2026-09-24, as recommended |
| Spec 07 decisions R1–R9 | P | **accepted** | owner, 2026-09-24, as recommended |
| Spec 08 decisions S1–S7 | P | **accepted** | owner, 2026-09-24, as recommended |
| Spec 09 decisions M1–M10 | P | **accepted** | owner, 2026-09-24, as recommended |
| Spec 10 decisions H1–H8 | P | **accepted** | owner, 2026-09-24, as recommended |

Review K findings (owner, 2026-09-25):

| Finding | Gate | Verdict | Note |
|---|---|---|---|
| B1 [P0] Module-origin `UpdateParameter` must not invoke Provider `coerce` | K | **accepted** | Avoid re-locking a Provider held by the stepping round; add a coordinator regression test |
| B2 [P1] KA-12 drain must stop after its cleanup operation is abandoned | K | **accepted** | Set `closing` on later step-3 entries and prevent the drain owner from acting after it wakes |
| N1 [P1] KC-30 containment for Link descriptor and Authority time-handle calls | K | **accepted** | Contain `descriptor`, `now`, `schedule` and `cancel`; fail the active stage and keep the Run manifestable |
| N2 [P1] Preserve a built non-Simulation plan in the refusal Manifest | K | **accepted** | Record the plan's execution class and plan before KC-2 refusal |
| N3 [P1] Register Executor events for every Island fragment | K | **accepted** | Add per-Island event-source coverage and a second-Island regression test |
| N4 [P2] Keep rejected validation details in `Manifest.admission` | K | **accepted** | Store the `AdmissionResult` before checking whether it was admitted |
| N5 [P2] Stop the pipeline when a lifecycle call requests an end | K | **accepted** | Check for a pending end between prepare, arm, schedule and start |
| N6 [P2] Refuse invalid Module namespaces only when sections are supplied | K | **accepted** | Match KA-13's conditional section failure |
| N7 [P2] Check lease expiry before `finish` chooses the client cause | K | **accepted** | Run `check_lease()` before normal finish cleanup |
| N8 [P2] Refuse `RejectAtPlan` when duration comparison errors | K | **accepted** | Treat comparison errors as failed plan-time validation |
| N9 [P2] Remove stale `allow(dead_code)` and duplicate/discarded schedule code | K | **accepted** | Keep the coordinator minimal and call `ActionTemplate::is_timed()` |
| N10 [P2] Collect violations from every refused compiled Action | K | **accepted** | Continue checking against the working copy; dispatch none if any Action fails |
| N11 [P2] Report unavailable link-drop counts as unknown | K | **accepted** | Store JSON `null` rather than reporting a false zero |

Review K re-review findings (owner, 2026-09-25):

| Finding | Gate | Verdict | Note |
|---|---|---|---|
| R1 [P2] Preserve StopRx-before-cleanup and prevent a stale drain owner from acting after cleanup | K | **accepted** | Coordinate StopRx and RestoreBaseline with per-instance markers and slots; restore stops a Sink or Executor when its slot is available, otherwise it records KC-39 and skips cleanup |
| R2 [P2] Do not start Modules after an Authority `now()` failure during Arm | K | **accepted** | Check for a pending end after T0 and schedule resolution/admission, before `start` |
| R3 [P2] Preserve successful partial PrepareReports after a mid-prepare Abort | K | **accepted as documented risk** | KC-12 does not require incomplete reports in the Manifest; `collect_prepare` rejects a missing fragment report, and recording partial values as merged would misstate completion |
| R4 [P2] Strengthen the KA-12 and Module-origin Action regressions | K | **accepted** | Replace negative polling with condition-variable observations; check StopRx ordering and count; compare Provider coercion count at the instant before Module submission and assert Provider receipt |
| B3 [P0] A wedged cleanup Module must not block later steps for other instances | K | **accepted** | Hold `done` only while claiming markers and acquiring one instance slot; release it before Module calls so other instances can stop, clean up, and reach the Manifest |

Confirmed individually, before the rows above (owner, 2026-09-24):

| Decision | Gate | Verdict | Note |
|---|---|---|---|
| Fault injection in Phase 2: the three kinds of SE-3, carried with §58 #5 and #6, and KA-22's amendment of spec 04's scope lines (Y9) | P | **accepted** | Further fault kinds stay Phase 4 |
| `x310-like` transport limit 1.0 GB/s per direction per motherboard (MR-3, Y13) | P | **accepted** | INFERRED; replaced by the Phase 8 measurement |
| No "record everything" capture mode: a capture needs `N` samples (HD-10, H8) | P | **accepted** | A `sink.capture_all` key can be added later without breaking anything |
| `x310-like` requires `ezsdr.time.start_lead_ns ≥ startup_latency_ns` (2 s); an earlier start is refused (MR-11, M2) | P | **accepted** | Starting late with `LATE` was rejected by Y13 |
