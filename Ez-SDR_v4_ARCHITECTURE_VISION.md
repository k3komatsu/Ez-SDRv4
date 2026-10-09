# Ez-SDR v4 — Architecture Vision

> **Status:** Architecture vision / design contract. Phase 1 and Phase 2's normative specifications are accepted and live in `design/01-*.md` through `design/10-*.md`; where a section here says `Normative:`, the spec it names is the contract and the section keeps the reasons.
>
> **Audience:** Ez-SDR developers, Codex/AI coding agents, reviewers, future contributors  
> **Purpose:** Define what Ez-SDR v4 is trying to become, what its Core must and must not own, and which architectural properties must remain true before implementation begins.  
> **Non-purpose:** This is not the final Rust API, wire protocol, JSON Schema, WASM ABI, scheduler implementation, or UHD adapter specification.  
> **Structure:** The Vision is split into eleven part files under `design/vision/`. Section numbers §1–§68 are stable across all parts and are the reference unit used by `design/v4-vision-audit.md` and `design/v4-vision-rereview.md`. This file is the index, the reading guide and the revision history.  
> **Shapes are illustrative:** the `{ ... }` and tree blocks in the parts show concept shapes, not final schemas. Normative schemas (Stream Contract, time model, Session, BindingProfile, PrepareReport, Manifest) live in the `design/` specifications and in the committed JSON Schemas under `schemas/`.

---

## The vision in one sentence

> **Ez-SDR v4 is a typed, deterministic Experiment Runtime for humans and AI agents, built around a small experiment microkernel and replaceable Modules, with software simulation treated as a first-class execution target equal to physical SDR hardware.**

Design principle: **Simple by default, powerful when needed.** Development principle: **Design once, validate in software, promote to hardware.** (§2)

---

## How to read

- New readers: Part 01 (why, the three Core tiers, what must not be in Core), then Part 11 §65 (the 42 invariants), then Part 03 (why simulation is a peer of hardware).
- Implementers: normative text is in `design/01-time-model.md` through `design/10-host-data-path.md`; cross-cutting decisions remain in `plan/phase1/00-overview.md` and `plan/phase2/00-overview.md`. Parts 02, 04 and 05 keep the reasons behind them — §8 (composite resources, BindingProfile), §10–§11 (compilation, PrepareReport), §15 (time), §19–§23 (descriptors, DataContracts, TxBurst, Stream Contract), §25–§29 (coherence, calibration, mutation, continuity, events). Part 10 lists the acceptance tests (§58).
- Provider and Executor authors: Parts 06–08.
- Reviewers: the audit (`design/v4-vision-audit.md`) and re-review (`design/v4-vision-rereview.md`) cite sections as §N; use the section index below to find the file.
- The whole Vision as one stream: `cat design/vision/*.md`.

---

## Parts

| Part | File | Sections | Themes |
|---|---|---|---|
| 01 | [01-purpose-and-core-boundary.md](design/vision/01-purpose-and-core-boundary.md) | §1–§7 | Why v4 exists; One-sentence vision; What Ez-SDR should feel like; Ez-SDR v4 is a microkernel architecture; What belongs in Core: three stability tiers; What must not belong in Core; Model + Provider is the default extension pattern |
| 02 | [02-spec-binding-and-compilation.md](design/vision/02-spec-binding-and-compilation.md) | §8–§11 | Experiment intent must be separated from implementation binding; ExperimentSpec is declarative intent, not a programming language; ExperimentSpec is compiled before RUN; Core is a transaction coordinator, not the sample scheduler |
| 03 | [03-simulation-mock-and-time.md](design/vision/03-simulation-mock-and-time.md) | §12–§17 | Mock is a first-class execution target; Software-only execution must work without SDR hardware; Simulation execution classes; Deterministic virtual time; SimulationChannel is separate from MockRadio; Fault injection is a first-class validation tool |
| 04 | [04-execution-model-and-data-contracts.md](design/vision/04-execution-model-and-data-contracts.md) | §18–§24 | Fixed and reactive experiments are both first-class; Processor and Reactor; Processing execution is replaceable; Data is not always IQ; Dynamic TxBurst is a first-class primitive; Time is first-class and must preserve sample relationships: the Stream Contract; ClockRelation is necessary for heterogeneous experiments |
| 05 | [05-coherence-calibration-and-runtime-semantics.md](design/vision/05-coherence-calibration-and-runtime-semantics.md) | §25–§30 | Coherent operation is different from mere multi-device operation; Calibration is a first-class research concept; Runtime parameter mutation must have semantics; Stream continuity and data validity are first-class; Observability is structured, typed, and non-blocking; Probe / Tap is important for research instrumentation |
| 06 | [06-performance-islands-and-radio-backends.md](design/vision/06-performance-islands-and-radio-backends.md) | §31–§36 | Memory placement matters as much as compute placement; Real-time execution should use bounded Execution Islands; Throughput and latency are separate optimization objectives; Performance must be described as an envelope; Radio backend architecture; Device profiles and quirks remain Provider concerns |
| 07 | [07-peripherals-and-host-io.md](design/vision/07-peripherals-and-host-io.md) | §37–§41 | External laboratory devices are first-class resources; Peripheral timing guarantees must be explicit; USRP GPIO must not create cross-module coupling; Host I/O is distinct from Peripheral I/O; Linux TUN/TAP is an explicit target |
| 08 | [08-future-workloads-and-targets.md](design/vision/08-future-workloads-and-targets.md) | §42–§49 | WASM is a future Processor/Reactor implementation target; AI is outside the hard real-time loop; AI-generated real-time components; MIMO and coherent systems must be natural, not special cases; IBFD must be a normal supported workload; ISAC must support heterogeneous sensing resources; OTFS and tensor-heavy processing must not require redesign; Distributed execution is a future implementation, but not a forbidden model |
| 09 | [09-provenance-validation-and-ownership.md](design/vision/09-provenance-validation-and-ownership.md) | §50–§56 | Artifacts, Runs, and provenance are first-class; Artifact formats should interoperate with existing ecosystems; Validation and dry-run are first-class; Resource ownership and cleanup are transactional; Python and ExperimentSpec are complementary; Time-scale separation is a design guide; Reference architecture litmus test: IEEE 802.11-like transceiver |
| 10 | [10-implementation-path-and-testing.md](design/vision/10-implementation-path-and-testing.md) | §57–§61 | Minimal implementation path; First architecture acceptance tests; UHD is added only after the Core + Mock model works; Suggested logical repository structure; Testing philosophy |
| 11 | [11-boundaries-invariants-and-roadmap.md](design/vision/11-boundaries-invariants-and-roadmap.md) | §62–§68 | Security and trust boundaries; Explicit non-goals; Features allowed to remain future work; Strong design invariants; Architecture success criteria; Development sequencing; Final perspective |

---

## Section index

| § | Title | Part |
|---|---|---|
| 1 | Why v4 exists | [01](design/vision/01-purpose-and-core-boundary.md) |
| 2 | One-sentence vision | [01](design/vision/01-purpose-and-core-boundary.md) |
| 3 | What Ez-SDR should feel like | [01](design/vision/01-purpose-and-core-boundary.md) |
| 4 | Ez-SDR v4 is a microkernel architecture | [01](design/vision/01-purpose-and-core-boundary.md) |
| 5 | What belongs in Core: three stability tiers | [01](design/vision/01-purpose-and-core-boundary.md) |
| 6 | What must not belong in Core | [01](design/vision/01-purpose-and-core-boundary.md) |
| 7 | Model + Provider is the default extension pattern | [01](design/vision/01-purpose-and-core-boundary.md) |
| 8 | Experiment intent must be separated from implementation binding | [02](design/vision/02-spec-binding-and-compilation.md) |
| 9 | ExperimentSpec is declarative intent, not a programming language | [02](design/vision/02-spec-binding-and-compilation.md) |
| 10 | ExperimentSpec is compiled before RUN | [02](design/vision/02-spec-binding-and-compilation.md) |
| 11 | Core is a transaction coordinator, not the sample scheduler | [02](design/vision/02-spec-binding-and-compilation.md) |
| 12 | Mock is a first-class execution target | [03](design/vision/03-simulation-mock-and-time.md) |
| 13 | Software-only execution must work without SDR hardware | [03](design/vision/03-simulation-mock-and-time.md) |
| 14 | Simulation execution classes | [03](design/vision/03-simulation-mock-and-time.md) |
| 15 | Deterministic virtual time | [03](design/vision/03-simulation-mock-and-time.md) |
| 16 | SimulationChannel is separate from MockRadio | [03](design/vision/03-simulation-mock-and-time.md) |
| 17 | Fault injection is a first-class validation tool | [03](design/vision/03-simulation-mock-and-time.md) |
| 18 | Fixed and reactive experiments are both first-class | [04](design/vision/04-execution-model-and-data-contracts.md) |
| 19 | Processor and Reactor | [04](design/vision/04-execution-model-and-data-contracts.md) |
| 20 | Processing execution is replaceable | [04](design/vision/04-execution-model-and-data-contracts.md) |
| 21 | Data is not always IQ | [04](design/vision/04-execution-model-and-data-contracts.md) |
| 22 | Dynamic TxBurst is a first-class primitive | [04](design/vision/04-execution-model-and-data-contracts.md) |
| 23 | Time is first-class and must preserve sample relationships: the Stream Contract | [04](design/vision/04-execution-model-and-data-contracts.md) |
| 24 | ClockRelation is necessary for heterogeneous experiments | [04](design/vision/04-execution-model-and-data-contracts.md) |
| 25 | Coherent operation is different from mere multi-device operation | [05](design/vision/05-coherence-calibration-and-runtime-semantics.md) |
| 26 | Calibration is a first-class research concept | [05](design/vision/05-coherence-calibration-and-runtime-semantics.md) |
| 27 | Runtime parameter mutation must have semantics | [05](design/vision/05-coherence-calibration-and-runtime-semantics.md) |
| 28 | Stream continuity and data validity are first-class | [05](design/vision/05-coherence-calibration-and-runtime-semantics.md) |
| 29 | Observability is structured, typed, and non-blocking | [05](design/vision/05-coherence-calibration-and-runtime-semantics.md) |
| 30 | Probe / Tap is important for research instrumentation | [05](design/vision/05-coherence-calibration-and-runtime-semantics.md) |
| 31 | Memory placement matters as much as compute placement | [06](design/vision/06-performance-islands-and-radio-backends.md) |
| 32 | Real-time execution should use bounded Execution Islands | [06](design/vision/06-performance-islands-and-radio-backends.md) |
| 33 | Throughput and latency are separate optimization objectives | [06](design/vision/06-performance-islands-and-radio-backends.md) |
| 34 | Performance must be described as an envelope | [06](design/vision/06-performance-islands-and-radio-backends.md) |
| 35 | Radio backend architecture | [06](design/vision/06-performance-islands-and-radio-backends.md) |
| 36 | Device profiles and quirks remain Provider concerns | [06](design/vision/06-performance-islands-and-radio-backends.md) |
| 37 | External laboratory devices are first-class resources | [07](design/vision/07-peripherals-and-host-io.md) |
| 38 | Peripheral timing guarantees must be explicit | [07](design/vision/07-peripherals-and-host-io.md) |
| 39 | USRP GPIO must not create cross-module coupling | [07](design/vision/07-peripherals-and-host-io.md) |
| 40 | Host I/O is distinct from Peripheral I/O | [07](design/vision/07-peripherals-and-host-io.md) |
| 41 | Linux TUN/TAP is an explicit target | [07](design/vision/07-peripherals-and-host-io.md) |
| 42 | WASM is a future Processor/Reactor implementation target | [08](design/vision/08-future-workloads-and-targets.md) |
| 43 | AI is outside the hard real-time loop | [08](design/vision/08-future-workloads-and-targets.md) |
| 44 | AI-generated real-time components | [08](design/vision/08-future-workloads-and-targets.md) |
| 45 | MIMO and coherent systems must be natural, not special cases | [08](design/vision/08-future-workloads-and-targets.md) |
| 46 | IBFD must be a normal supported workload | [08](design/vision/08-future-workloads-and-targets.md) |
| 47 | ISAC must support heterogeneous sensing resources | [08](design/vision/08-future-workloads-and-targets.md) |
| 48 | OTFS and tensor-heavy processing must not require redesign | [08](design/vision/08-future-workloads-and-targets.md) |
| 49 | Distributed execution is a future implementation, but not a forbidden model | [08](design/vision/08-future-workloads-and-targets.md) |
| 50 | Artifacts, Runs, and provenance are first-class | [09](design/vision/09-provenance-validation-and-ownership.md) |
| 51 | Artifact formats should interoperate with existing ecosystems | [09](design/vision/09-provenance-validation-and-ownership.md) |
| 52 | Validation and dry-run are first-class | [09](design/vision/09-provenance-validation-and-ownership.md) |
| 53 | Resource ownership and cleanup are transactional | [09](design/vision/09-provenance-validation-and-ownership.md) |
| 54 | Python and ExperimentSpec are complementary | [09](design/vision/09-provenance-validation-and-ownership.md) |
| 55 | Time-scale separation is a design guide | [09](design/vision/09-provenance-validation-and-ownership.md) |
| 56 | Reference architecture litmus test: IEEE 802.11-like transceiver | [09](design/vision/09-provenance-validation-and-ownership.md) |
| 57 | Minimal implementation path | [10](design/vision/10-implementation-path-and-testing.md) |
| 58 | First architecture acceptance tests | [10](design/vision/10-implementation-path-and-testing.md) |
| 59 | UHD is added only after the Core + Mock model works | [10](design/vision/10-implementation-path-and-testing.md) |
| 60 | Suggested logical repository structure | [10](design/vision/10-implementation-path-and-testing.md) |
| 61 | Testing philosophy | [10](design/vision/10-implementation-path-and-testing.md) |
| 62 | Security and trust boundaries | [11](design/vision/11-boundaries-invariants-and-roadmap.md) |
| 63 | Explicit non-goals | [11](design/vision/11-boundaries-invariants-and-roadmap.md) |
| 64 | Features allowed to remain future work | [11](design/vision/11-boundaries-invariants-and-roadmap.md) |
| 65 | Strong design invariants | [11](design/vision/11-boundaries-invariants-and-roadmap.md) |
| 66 | Architecture success criteria | [11](design/vision/11-boundaries-invariants-and-roadmap.md) |
| 67 | Development sequencing | [11](design/vision/11-boundaries-invariants-and-roadmap.md) |
| 68 | Final perspective | [11](design/vision/11-boundaries-invariants-and-roadmap.md) |

---

## Related documents

- `design/archive/Ez-SDR_v4_core_module_architecture.md` — the former Core / Module Architecture Principles document, **retired 2026-09-21**. Kept only for the `CMA §N` citations in the audit documents; its content is covered by the Vision (section map at the top of the archived file).
- `design/v4-vision-audit.md` — Phase 0 adversarial architecture audit (Findings 1–34).
- `design/v4-vision-rereview.md` — re-review of the revised Vision (Findings R1–R22).
- `v3/` — the previous implementation, a source of behavioural requirements only (§1, §61).

---

## Revision history

| Revision | Summary |
|---|---|
| 2026-09-21 | Incorporates the P0 findings (Findings 1–6 and 23) of the Phase 0 architecture audit, `design/v4-vision-audit.md`: stability tiers (§5), Time Authority (§15, §23), the Stream Contract (§23), the Timing Envelope and Mock enforcement (§13, §14, §22), Sessions (§3, §50), composite resources and provider-declared coherence (§8, §25, §39), and the BindingProfile environment (§8, §16, §17). |
| 2026-09-21, second pass | Incorporates the P1 findings (Findings 7–22; 21 and 23 were already covered by the first pass): generic capability matching (§8), descriptors versus execution ABI, cycle rule and deadline kinds (§19), DataContract registry (§21), schema-first types and Spec builder (§9, §10), PrepareReport (§11), Lease modes and the Policy table (§53), validator-not-optimiser planning (§10, §20, §31), no structural mutation (§27), calibration application and retune phase behaviour (§26), event counters (§29), RF safety envelope (§8, §52), step-driven Executors (§15, §32), Manifest envelope and sections (§50), RFNoC as radio capability (§4, §20, §32, §35, §64), and the Module / role / deployment axes (§7). |
| 2026-09-21, third pass | Incorporates the P2/P3 findings (Findings 24–34) and the audit's explicit rejections: process boundaries and UHD crash handling (§35, §62), node-qualified identifiers (§49), the one-shot TUN/TAP helper (§41), Probe as a link policy (§30), taint as a convention (§28), explicit format conversion (§21, §34), external sensors by reference (§47), timing classes per instance (§38), clock sources as Peripherals and the Sink / Link module categories (§37, §60), no metrics framework (§29), behavioural rather than wire compatibility with v3 (§61), and the corresponding non-goals and future-work entries (§6, §63, §64). |
| 2026-09-21, fourth pass | Applies R1, R2 and R21 of `design/v4-vision-rereview.md`: per-direction channel requests (§8), the TX side of the Stream Contract (§22, §23, §29, §58), and placement in the BindingProfile only (§8, §9, §10, §20, §65). |
| 2026-09-21, fifth pass | Applies R3, R6, R17 and R22 of `design/v4-vision-rereview.md`: Session Actions are admitted on the control path (§3, §52, §58, §65), a sample-rate change starts a new SampleClock (§15, §23, §27), MockRadio also enforces the PerformanceEnvelope (§13, §34, §58), and the discrete-event simulator is named the Simulation Engine so that *Kernel* means only the Core tier (§5, §13–§15, §32, §57–§58, §60, §67). |
| 2026-09-21, sixth pass | Applies the editorial items R4, R5, R7–R12, R15, R16 and R18 of `design/v4-vision-rereview.md` (pipeline order, determinism scope, profile versioning and parity measurement, the Authority role, Action and time naming, the Spec shape, remaining phrasing, language-neutral shapes, `sdr.sleep`, coercion defaults per Run kind, child-Run leases) and splits the Vision into eleven part files under `design/vision/` (R13, R14). Section numbers are unchanged. |
| 2026-09-21, seventh pass | Retired `Ez-SDR_v4_core_module_architecture.md` (archived under `design/archive/` with a section map). Its two passages without a Vision counterpart were folded in: the Module communication rule with its Bad / Good example (§7) and the smart-antenna litmus test (§66). |
| 2026-09-23 | Phase 1 Step 5 (`plan/phase1/00-overview.md` §12), re-review R13: the normative blocks of §3, §7, §8, §10, §11, §14, §15, §19, §21–§24, §27, §29, §32, §38, §49, §50, §52 and §53 are replaced by summaries that end in a `Normative:` line naming the accepted spec and its rules; each section keeps its number, title and reasons. §23's numbered RX and TX rules stay as an index, because the specs and the code cite them by number. The spec's departures are applied in the same pass: `module` bindings with an exact version and a mandatory `authority` in §8's examples, which now parse; Session verbs other than lifecycle verbs are Vocabulary verbs (§3); `DEVICE_LOST` (§29); fidelity value `real` (§14); the stepping loop is the Kernel coordinator's (§15, §32); `ResourceId { node, path }` (§49); seeds and environment capture outside the Manifest envelope (§50). The "Vision issues found" items outside those sections are applied directly: §9 (`requirements` versus `requires`, `schedule`), §28 (`Gap.cause`), §20 (a component's `requires` is `executor_kind` and `memory_bytes`, its budget is in `timing`), §31 (MemoryDomain kinds are Vocabulary content), §58 #15 (the two layers of an unclosed burst) and §65 ("delivered", not "sampled", events). Obligations with no Phase 1 rule stay as prose and are marked Phase 2 where they have an owner: §3's TimingEnvelope and PerformanceEnvelope checks on Session Actions, §22's plan-time lead check, §23's Mock options, §32's drop policies in virtual time and §29's "no metrics framework". |
| 2026-09-25 | Phase 2 Gate X acceptance and Step X (`plan/phase2/00-overview.md` §11): moved accepted specs 06–10 into `design/` and applied the fifteen Phase 2 Vision issues to §§8, 11, 13, 14, 17, 22, 26, 30, 32, 52 and 53, preserving section numbers and part navigation. |
| 2026-09-26 | Phase 3 Gate X acceptance and Step X (`plan/phase3/00-overview.md` §11): moved accepted spec 11 (the SimulationChannel) to `design/11-simulation-channel.md` and applied the eight Phase 3 Vision issues (`plan/phase3/vision-issues.md`) to §§8, 13, 15, 16, 23, 25, 26 and 57: the channel is a medium evaluated by the receiving radio, not a scheduled model or a Module; `sim.channel` is a coupling matrix with explicit ends, a delay in nanoseconds and a noise power; a loopback is a coupling; the device's own RF behaviour (gain, LO phase, clipping, path delay) is the radio model's and only propagation the channel's; §16 gains a `Normative:` line. Section numbers and part navigation are preserved. |
| 2026-09-26, Phase 4 | Phase 4 Gate X acceptance and Step X (`plan/phase4/00-overview.md` §11): applied the two Phase 4 Vision issues (`plan/phase4/vision-issues.md`): §28 and §51 gain `Normative:` lines naming SC-30…SC-32 and HD-15, since every capture is now a SigMF Recording; §29 says who owns a hot-path payload's bytes (the Vocabulary, which decodes them; `radio.RX_OVERFLOW` is the first, RM-24). Spec 13's amendments were applied to `design/` with the code and stay in `plan/phase4/13-amendments.md` as the record. |
| 2026-09-26, Phase 5 | Phase 5 Gate X acceptance and Step X (`plan/phase5/00-overview.md` §11): moved accepted spec 14 (the native Executor) to `design/14-native-executor.md` and applied the three Phase 5 Vision issues (`plan/phase5/vision-issues.md`): §9's conceptual shape of an ExperimentSpec gains `inputs` (SB-20a); §19 gains a `Normative:` line naming spec 14; §22 says what Simulation enforces of a Reactor's speed — the device's lead, counted from when the device delivered the samples — and that a component's own processing time is not charged there. Section numbers and part navigation unchanged. |
| 2026-09-27, Phase 6 | Phase 6 Gate X acceptance and Step X (`plan/phase6/00-overview.md` §11): moved accepted spec 16 (the Easy API: the server, its protocol and the Python package) to `design/16-easy-api.md` and applied three of the four Phase 6 Vision issues (`plan/phase6/vision-issues.md`): §3's `Normative:` line names spec 16 and KC-37a, §15 cites KC-29 and KC-29b for `wait_until` and `wait_for`, and §62's process-boundary table gains the clients. The fourth, withdrawing §9's builder source hash, was not needed: the owner kept the field, to be added before the freeze. Section numbering preserved. |
| 2026-10-01, Phase 7 | Phase 7 Gate X acceptance and Step X (`plan/phase7/00-overview.md` §11): moved accepted spec 18 (the UHD Radio Module `ezsdr.radio.uhd`) to `design/18-uhd-radio.md` and applied the three Phase 7 Vision issues (`plan/phase7/vision-issues.md`): §35's UHD subsection says UHD's C API is the narrow bridge — with the one exception the bench found, a throw out of UHD's own destructors (design-notes §17 F4) — and ends in a `Normative:` line naming spec 18; §15's device timekeeper also publishes its relation to UTC (MA-29, KC-45); §32's driving-model `Normative:` line names KC-46. |
| 2026-10-07, spec 20 | Maintenance spec 20 (`plan/maintenance/20-amendments.md`, issues #39–#45), its seven items implemented: applied its four Vision issues. §11 and §52 say that `prepare()` returns a PrepareReport per fragment and no merged view of them, that `run.effective()` exposes the per-fragment configuration and that the Manifest records the reports; §11's `Normative:` line gains KC-27 (KH-1). §54 gains a `Normative:` line naming EA-12 and EA-16 for how `sdr.sleep`'s `d`, and every other argument in seconds, is read, rounded and sent (VF-2). §13's illustrative TimingEnvelope key tree lists RM-6's seven keys, the restart and start leads included (VF-6). Section numbers and part navigation unchanged. |
| 2026-10-08, roadmap | §67 gains Phase 8b, multi-device (several USRPs on one 10 MHz + PPS reference, aligned start), between the Mock → X310 parity test and Packet/PDU, with the reason for that place (owner, 2026-10-08). It had been recorded only as "after Phase 8" at Phase 7's Gate P (`plan/phase7/00-overview.md`). Phase numbers otherwise unchanged; section numbers and part navigation unchanged. |
| 2026-10-09, audit item 5 | Pre-freeze audit item 5 (`plan/maintenance/24-prefreeze-audit.md`, owner decision 2026-10-09): §10 no longer promises a registered migration function and a Manifest record of the original version and hash, which nothing used; a later major refuses an older document unless a migration for it is written (invariant 39), and the `Normative:` line names SB-47 alone (SB-48 and SB-49 withdrawn). §8 and §50 no longer say the environment is recorded "verbatim": a document is recorded as parsed (RS-45). Section numbers and part navigation unchanged. |
