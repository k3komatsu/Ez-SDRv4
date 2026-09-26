# Phase 6 — Python Easy API: overview and plan

| Field | Value |
|---|---|
| Status | **Planned and implemented under the owner's delegation** (2026-09-26: "それではPhase4/5と同じ用にPhase6も設計と実装をしてください"); there is no separate Gate P. Reviewed by an Opus review loop (§9). Awaiting Gate X. |
| Phase | Vision §67 Phase 6: "Python Easy API". Predecessor: Phase 5 (Mini Reactive Radio; accepted at Gate X 2026-09-26). Successor: Phase 7 (native UHD Provider). |
| Scope | Vision §3's and §57's snippets, run from Python entirely in software: `with ezsdr.connect() as sdr: sdr.tx.repeat(x); y = sdr.rx.capture(N)` yields `y` and a Session Manifest with the action log, the waveform hash, the capture's first sample time and validity, and the effective configuration. §54's `result = sdr.run(spec)` as a child Run. The three Kernel holes the prototype hit (§2), and the Phase 6 items earlier phases left open (§8). |
| Not in scope | §3 lists it. In one line: no remote transport, no Session replay, no artifact store, no Spec builder, no CLI or MCP frontend, no hardware pacing. |
| Language | English, like Phases 1–5. |
| Location | Spec 16, [`16-easy-api.md`](16-easy-api.md), is the new frontend's spec (the server, its protocol and the Python package); it moves to `design/` at Step X, as specs 11 and 14 did. Spec 17, [`17-amendments.md`](17-amendments.md), holds the Kernel and Vocabulary amendments; their text reaches `design/` in the commit that implements them (GY-5), and the file stays here as the record. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Documents:

```text
plan/phase6/
  00-overview.md           this file: scope, holes, decisions, crates, governance, traceability, sequencing, exit criteria, decision log
  16-easy-api.md           spec 16: the server ezsdr-server, the protocol ezsdr.protocol 1, the Python package ezsdr (EA-1…EA-19)
  17-amendments.md         spec 17: Kernel amendments KF-1…KF-4 and the Vocabulary amendment VD-1
  implementation-notes.md  what was run, what the reviews found, what was fixed
  exit-review/             per-rule dispositions (PO-10)
  vision-issues.md         the Vision edits specs 16 and 17 need, applied at Step X with the owner's approval
  tools/                   the mutation list (Appendix A of spec 17) and Phase 5's tool
```

---

## 1. What Phase 6 had to prove, and what already existed

Vision §3: "`connect()` opens a Session. A Session is a Run whose ExperimentSpec is implicit and whose Manifest records an action log", and "Python, CLI, MCP/AI interfaces, and future frontends must all map to the same typed domain model". §57: the snippet "should already work … entirely in software, and it must already produce a Session Manifest containing the action log, the waveform hash, the capture's first sample time and validity flags, and the effective configuration". §58 #13: "Using only the Easy API produces a Manifest with the action log, waveform hash and effective configuration". §54: `result = sdr.run(spec)` is a child Run, and `sdr.sleep(d)` is `run.wait_until(now + d)` in the Run's time. §15: "Python has `run.wait_until(t)` and `run.wait_for(event)`".

The Session itself exists in Rust and is tested end to end:

| Piece | Where | State before Phase 6 |
|---|---|---|
| `connect()` derives the implicit Spec and runs the pipeline | KC-1, SB-22c, `coordinator::connect` | tested (`v58_13_*`, `v58_16_*`, `v57_a_software_loopback_session_captures_what_it_transmits`) |
| Every Session Action admitted and logged, rejected ones included | KC-28, RS-15…RS-19 | tested (`v58_16_*`, `kc_28_*`) |
| `radio.start_repeat` and `sink.capture` compile to Kernel Actions | RS-13a, RS-14, HD-6 | tested |
| `run.wait_until(t)` | `RunHandle::advance_to` (KC-29) | tested |
| A software loopback | a `sim.channel` coupling from a radio to itself (CH-1…CH-11) | tested (`v57_*`) |
| Child Runs | RS-25, RS-25a | **refused** (KC-37: "RS-25a: child Runs are Phase 6's") |
| A Python client | — | none |

So Phase 6 began with a prototype: §57's snippet written against `RunHandle` the way a client would drive it — connect with a loopback profile, set the three parameters of §3's second snippet, `start_repeat`, `capture(100_000)`, return the samples, then `RunChild` — run through the real coordinator with MockRadio `x310-like`, the capture Sink and the Simulation Engine. It hit the holes of §2.

## 2. What the prototype hit, with evidence

| # | Hole | Evidence | Amendment |
|---|---|---|---|
| 1 | **A client cannot learn that a capture is written, nor read it, until the Session ends.** The capture Sink hands its `ArtifactRef`s to the coordinator only from `stop` (MA-26, HD-13), and `RunHandle` exposes neither the artifacts nor the delivered events before `finish`. `y = sdr.rx.capture(N)` needs `y` during the Session | The prototype's capture of 100 000 samples at 20 Msps was complete on disk at 2 005 000 000 ns (the `.sigmf-meta` appeared there), and nothing the Kernel exposes said so: `RunHandle`'s public methods are `id kind state now start_instant submit effective sample_clocks disconnect check_lease finish advance_to run_until_end` (`coordinator/mod.rs`, `stepping.rs`); the artifact reached the Manifest only at `finish` (`rec_0`, 800 000 bytes) | KF-1, KF-2, VD-1 |
| 2 | **Vision §15's `run.wait_for(event)` has no Kernel counterpart.** A client waiting for something that happens in Run time can only step `advance_to` in quanta it chooses, overshooting by up to a quantum, so the instant at which its next Action is logged depends on the frontend's quantum | same list of methods; `RunHandle::run_loop` is private | KF-2 |
| 3 | **`sdr.run(spec)` is refused.** KC-37 refuses `RunChild`; and even an admitted `SessionAction::RunChild { spec_hash, binding_hash }` names its documents by hash with no store to find them in and no Modules to run them with (the Kernel builds no Module, KC-4) | the prototype's `RunChild`: `Rejected { violations: [Violation { check: ezsdr.run_child, reason: "RS-25a: child Runs are Phase 6's" }] }` | KF-3 |
| 4 | **RS-25a lets a child leave its parent's RF safety envelope.** RS-25a binds a child's devices and Authority to its parent's, but says nothing of `environment`, where `radio.rf_envelope` lives (SB-29): a child profile that drops the section would transmit anywhere, although §52 calls the envelope "what stands between an AI-generated Spec and the antenna" | reading RS-25a against SB-29, while writing KF-3 | KF-3 |
| 5 | **SB-14 cites a Manifest field that does not exist.** "…the client-side builder of Vision §9, whose source hash the Manifest records beside the Spec hash (spec 04)": spec 04 defines no such field and `manifest::SpecSection` has `hash`, `body`, `original_version`, `original_hash` only | `grep` of `design/04-run-and-session.md` and `crates/ezsdr-kernel/src/manifest.rs` | KF-4 (text; an owner decision before the freeze, §11) |

The prototype also showed two things a client must do that are not Kernel holes, and spec 16 puts them in the server so that every frontend does them alike: a Session is `Running` at instant 0 while its devices start at T0 = 2 s on `x310-like` (MR-11), so `connect` advances to `start_instant()` before it returns; and the implicit Spec requests zero transmit channels (SB-22c), so `sdr.tx.repeat(x)` sets `radio.tx.channels` to the waveform's channel count before `radio.start_repeat` (a `repeat` without it was `Rejected` with "SC-23: local:mock/tx has no running transmit SampleClock").

## 3. Scope

### In scope

1. **KF-1** — `RunHandle::events(from)`: the events the Run has delivered, readable while it runs (KC-29a).
2. **KF-2** — `RunHandle::wait_for(kinds, from, horizon)`: advance round by round until a delivered event of one of `kinds` appears, or the horizon (KC-29b; Vision §15).
3. **KF-3** — child Runs: `RunHandle::run_child(spec, profile, assembly, drive)` admits, logs and runs a child Spec Run to its end under the parent's Lease, and records it; RS-25a gains the environment rule of hole 4; KC-37 changes to point `submit` at `run_child`.
4. **KF-4** — SB-14's dangling reference marked forward, and the field it promises put to the owner (§11).
5. **VD-1** — the `sink` Vocabulary 1.1.0 declares `sink.CAPTURE_WRITTEN`, carrying the `ArtifactRef`; the capture Sink 1.2.0 emits it when it records a capture.
6. **Spec 16** — the server `ezsdr-server` (a Rust binary that compiles the Modules in and runs one Session per process), the protocol `ezsdr.protocol` 1 between it and a client, and the Python package `ezsdr`.
7. **The carriers of §8**, in Rust (`cargo test`) and in Python (`python -m unittest`).

### Out of scope, with the phase that owns each

| Item | Why not now | Owner |
|---|---|---|
| Remote access (a TCP listener), server-owned profiles, authentication | §68's "easy remote SDR access" is v3's main use and the reason the client is out of process (S1). But a remote server must own its BindingProfiles — a remote client that sends a profile chooses the capture directory and the RF envelope — and that is a trust boundary (§62) with no Phase 6 consumer: every Phase 6 Run is simulated and local. The protocol is a byte stream, so a listener is additive (§62: "must not require a redesign") | Phase 7, with the first lab server holding a device |
| Session replay (RS-20's re-application) | A frontend loop over the admitted entries (`advance_to(entry.time)`, `submit`); the Kernel's part, `check_replay_target`, exists. It needs the waveform bytes of a finished Session, and a Session records its inputs as `mem:<hash>` (KC-28): the artifact store. No §58 test and no §3/§54/§57 snippet uses it, and it changes no Kernel type, so it does not block the freeze | owner decision (§11): with the artifact store |
| An artifact store beyond `mem:` and `file://` (Phase 2 Y12) | Its consumers were child Runs and replay. A child Run gets its inputs' bytes from the call (KF-3), so only replay remains | with replay |
| The Spec builder `ezsdr.spec.build` and the source hash SB-14 promises | `sdr.run(spec)` takes a Spec document however the user built it (§54's `build_experiment(snr)` is user code). The Manifest field is a Kernel change with no producer until a builder exists (KF-4) | owner decision before the freeze (§11) |
| A CLI or an MCP frontend | Vision §3 wants them on the same typed model; the protocol is that model (S2), so either is one more client. No Phase 6 test needs one | when first needed |
| A Run paced by the wall clock, and the spike's K8 (a hardware Run needs a coordinator loop that does not depend on client calls) | Every Phase 6 Run is Simulation, where time moves only when the client asks, which is right (§15). K8 is a hardware fact | Phase 7 (Phase 4 Gate X) |
| A Reactor inside a Session's own graph | A Session's implicit Spec has no components (SB-22c). Phase 5 deferred "a Reactor in a Session" to Phase 6's child Runs: `sdr.run(spec)` runs a Spec with a Reactor under the Session's Lease (`v58_09_a_reactor_runs_in_a_child_run_of_a_session`) | — (done through KF-3) |
| YAML profiles (Phase 1 X4: "converted to JSON by the frontend (Phase 6)") | The Python API takes a profile as a `dict`; which loader made it — `json`, a YAML library, TOML — is the user's, and so is YAML 1.1's type coercion. Nothing to build | — (settled, S10) |
| Device-side refusals raised as Python exceptions (`radio.COMMAND_REJECTED` for a repeat the radio refuses) | They arrive as events after admission, as on hardware; `sdr.events()` shows them. Raising them from `repeat()` means waiting in Run time for the burst's start, which is a design question for Phase 7's asynchronous device | Phase 7 |

A request to add any of these during Phase 6 is scope creep and is refused (AGENTS.md §6).

## 4. Cross-cutting decisions

Each row is open to reversal at Gate X; a reversal is recorded in §11.

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| S1 | Where Python runs relative to the Runtime | **Out of process**: a Rust binary, `ezsdr-server`, compiles the Modules in and runs the Session; a pure-Python package (standard library and numpy) spawns it and talks to it over its standard input and output with the typed protocol of spec 16 | An in-process extension (PyO3): it puts user Python in the Core's process, which §62 lists as separate trust zones; an in-process UHD (Phase 7) that aborts would take the user's interpreter with it, the failure v3 answered with `--retry` restarts (§35); every other frontend (CLI, MCP) would need a binding of its own; and the build would tie the client to the Rust toolchain and one interpreter ABI (this machine alone has Python 3.9, 3.12, 3.13 and 3.14, and 3.14's numpy is broken). A pure-Python reimplementation of Session semantics (v3's `SimpleMockClient`, `v3/client/ezsdr.py:430`, was a second, divergent Mock) | The sample path between Modules stays in process (§62); only a client's waveforms and captures cross the pipe, as bytes |
| S2 | What the protocol carries | The Kernel's own documents and `RunHandle`'s control API one to one — `connect`, `submit(SessionAction)`, `advance`, `wait_for`, `events`, `status`, `run_child`, `finish` — plus `read` for an artifact's bytes. Everything every frontend must do alike is the server's: the default profile, advancing to T0 at `connect`, converting durations onto the Run's clock, deriving a child's profile, writing the Manifests. Python adds names only (`sdr.rx.frequency` for `radio.rx.frequency_hz`) and composes `capture` from `submit`, `wait_for` and `read` | Easy verbs in the protocol (`repeat`, `capture`): a second command vocabulary beside `SessionAction`, the v3 controller protocol §3 names as what a Session must not become; Python doing time or Run semantics itself (every frontend would repeat them) | A CLI or MCP frontend is one more client of the same protocol |
| S3 | How a client learns a capture is written | The capture Sink emits `sink.CAPTURE_WRITTEN` with the `ArtifactRef` when it records the capture (VD-1), and the client reads it through `events` / `wait_for` (KF-1, KF-2), then the bytes through `read` | Polling the Sink's directory (a frontend coupled to one Sink's file names, with no completion signal); a Kernel `Sink::completed()` (changes a role trait for what the event path already carries, §29); only the sealed Manifest (the Easy API needs `y` during the Session) | — |
| S4 | Waiting in Run time | `RunHandle::wait_for` advances round by round and returns at the round that delivered the event, or stands at the horizon (KF-2) | The server stepping `advance_to` in fixed quanta (overshoot, and a frontend-dependent log time for the next Action) | A wall-paced class (Phase 7) waits in wall time with the same call |
| S5 | Child Runs | `RunHandle::run_child(spec, profile, assembly, drive)` is **synchronous**: it admits the child (RS-25a), logs `RunChild { spec_hash, binding_hash }`, builds the child with the parent's id and a copy of its Lease, hands it to `drive` (the caller runs it), cleans it up, records `ezsdr.children` in the parent and returns the child's Manifest. The parent's time stands still while its child runs | A live child handle returned to the caller (RS-6 step 0 has the parent clean its children up; a handle the client holds could outlive the parent's cleanup); the Kernel fetching documents by hash from a store (there is none; the documents travel with the call, and the child's Manifest records both); the child running beside the parent (the Simulation class steps one Run; KC-2) | On a wall-paced class the parent's devices keep running while the child uses them: the first hardware Provider decides whether `run_child` quiesces the parent's streams (Phase 7) |
| S6 | A child's profile when `sdr.run(spec)` names none | The server derives it: the parent's profile with every `feed` removed — RS-25a's own reason a Session profile cannot serve a Spec Run is `feed` (SB-22g) | Requiring a profile on every call (§54's `sdr.run(spec)` has none); deriving it in Python (every frontend would repeat it) | A Spec whose outputs name recorders the parent does not bind is refused by the child's own `validate()`, and the parent's log says why |
| S7 | `connect()` with no profile | The server's **default profile** (spec 16 EA-9): one simulated `x310-like` radio with a 0 dB loopback coupling on channel 0, the capture Sink fed from its receive port, the Simulation Engine as Authority, seed 0. Python never names it | The default in Python (the package would name Mock, `x310-like` and a sim section: §58 #10); no default (§3's `ezsdr.connect()` has no argument) | Phase 7's lab server makes its own profile the default |
| S8 | Sessions per server | One: the server runs one Session, answers `finish`, writes the Manifest and exits | Several Sessions over one connection (no consumer; a remote listener later runs one Session per connection) | — |
| S9 | Transport in Phase 6 | The server's standard input and output, the server being the client's child process; the server's diagnostics go to standard error | A TCP or Unix socket now (a remote listener needs the trust boundary of §3's first row) | Phase 7 |
| S10 | Python's reach | Python ≥ 3.9 and numpy; the API takes and returns plain `dict`s for Kernel documents | pyyaml, pydantic or pandas (a dependency for what a dict does); typed wrappers for every Kernel document (a second object model, §3's "must not become the internal architecture") | — |
| S11 | How Phase 6 is delivered | Planned from a prototype and implemented in one session under the owner's delegation, commit by commit on `main`; then an Opus review loop, as in Phase 5 | A Gate P before implementing (Phase 5's reason holds: the risk is in running code) | The owner can reverse any row at Gate X |

## 5. Crate layout

One crate and one Python package are added. No external Rust package is added (PO-4); `ezsdr-server` uses `serde`, `serde_json` and `schemars`, already in the lock.

| Crate / package | Tier | Change |
|---|---|---|
| `ezsdr-kernel` | Kernel | KF-1…KF-3: three `RunHandle` methods, the parent id in the coordinator's context, `ezsdr.children`; KC-37's reason; tests |
| `ezsdr-sink` | Vocabulary | 1.1.0: `sink.CAPTURE_WRITTEN` and its payload schema (VD-1) |
| `ezsdr-sink-capture` | Module | 1.2.0: emits `sink.CAPTURE_WRITTEN` (VD-1) |
| `ezsdr-server` | frontend (new) | spec 16: the protocol types and their schemas (`schemas/server/`), the Module catalogue (`assemble`), the Session loop, the binary. Not a Module (it is the Runtime that compiles Modules in, §62), so MA-3 does not bind it; depends on the Kernel, the Vocabularies and the Modules |
| `ezsdr-acceptance` | tests | `rig::assemble` delegates to the server's catalogue; profiles name the capture Sink 1.2.0; the Reactor-in-a-child-Run carrier; the governance lists; `v58_10` extended to the Python sources |
| `python/` | frontend (new) | the package `ezsdr` (`pyproject.toml`, `ezsdr/`), its tests (`python/tests/`) and examples (`python/examples/`) |
| the others | — | unchanged |

## 6. Governance

OV-1…OV-23b, PO-1…PO-12, GV-1…GV-6, GW-1…GW-5 and GX-1…GX-6 bind Phase 6. The rules below add what it needs.

- **GY-1** Phase 6 rule ids are `KF-n` (Kernel amendments), `VD-n` (Vocabulary amendments), `EA-n` (spec 16) and `GY-n`, protected by OV-1. An amended rule keeps its id (PO-1); a new rule in an existing series takes the next number (`KC-29a`). *Process obligation.*
- **GY-2** The Kernel changes only as KF-1…KF-4 say. The public API gains three `RunHandle` methods and no type; no Kernel schema changes (`ezsdr.children` is a Manifest section, whose body the Manifest schema leaves open, like `ezsdr.links`). *Checked by `kernel_surface` (`ov_23b`: 116 NEW / 292, unchanged, because methods are not items) and `schema_freeze`.*
- **GY-3** A Phase 1–5 test changes only where spec 17 changes what it observes, and each such change is listed in `implementation-notes.md` with the rule that forces it. *Process obligation.*
- **GY-4** Every new or amended rule with code has a test that fails when the rule's code is disabled, shown by a recorded mutation (spec 17 Appendix A), as PO-12 requires. *Process obligation; exit criterion 6.*
- **GY-5** Spec 17's text reaches `design/` in the commit that implements it; spec 16 moves to `design/` at Step X; the Vision is not edited before Gate X (OV-6). *Process obligation.*
- **GY-6** The Python package and its examples are application-facing code: they name no Module, device profile or simulation section (EA-18). *Checked by `v58_10_experiments_name_no_mock_type`, extended to `python/ezsdr/*.py` and `python/examples/*.py`.*
- **GY-7** The Python suite runs against the server binary built from the same tree, on Python 3.9 and on a current Python: `cargo build -p ezsdr-server`, then `EZSDR_SERVER=<target>/debug/ezsdr-server python3 -m unittest discover -s python/tests`. *Process obligation; exit criterion 4.*

## 7. Test strategy

- Kernel: KF-1…KF-3 through the coordinator with the existing doubles (`crates/ezsdr-kernel/tests/coordinator.rs`): events read mid-Run, `wait_for` returning at the delivering round and standing at its horizon, a child admitted, logged, run and recorded, and each RS-25a refusal.
- Vocabulary and Sink: `sink.CAPTURE_WRITTEN` declared and registered; the capture Sink's harness sees the event with the `ArtifactRef` it later returns, for a full and for a partial capture.
- The server: the protocol in process (`serve` over in-memory pipes) — handshake, every request, every error kind, the framing refusals, `read`'s restriction — plus one test that spawns the binary; a schema freeze test for `schemas/server/`.
- End to end in Rust (`ezsdr-acceptance`): a Session whose child Run holds the Phase 5 responder, driven through the server with the responder registered.
- End to end in Python (`python/tests/test_easy_api.py`, standard `unittest`): each Vision snippet of §8, against the binary.
- What the loopback carriers assert: `x310-like` delays what it radiates (MR-3) and rotates it by the transmit and receive LO phases it draws at `prepare` (MR-34), so a 0 dB loopback returns the repeated waveform at some offset, multiplied by one unit phasor. The carriers find the offset by magnitude and check that one phasor fits every sample, rather than asserting a constant offset or the samples themselves.

## 8. Traceability

### Vision → Phase 6

| Vision | Test | Phase 6 carrier | Remaining |
|---|---|---|---|
| §57 | the snippet, in software, with its Manifest | `test_v57_repeat_then_capture_in_software` (Python): `y` is the repeated waveform; the Manifest has the log, `inputs` holding the waveform's hash, the capture's `continuity` (first sample, validity) and the effective configuration | — |
| §58 #13 | Sessions leave provenance | `test_v58_13_the_session_manifest_is_written_and_complete` (Python): `manifest.json` in the Session directory is the Manifest `finish` returned, sealed, and names every call, the rejected one included | — |
| §58 #16 | Session Actions are admitted | `test_v58_16_a_rejected_call_raises_and_is_logged` (Python): a frequency outside the profile's RF envelope raises `ezsdr.Rejected` naming `radio.rf_envelope`, the effective value is unchanged, the log holds the rejection | — |
| §3 | the second snippet | `test_v3_parameters_through_attributes` (Python): the three setters, the read-back from the effective configuration; 19.5 Msps is coerced to 20 Msps and the coercion is on the entry | — |
| §54 | `result = sdr.run(spec)` in a loop, and `sdr.sleep` | `test_v54_a_sweep_of_child_runs` (Python): three child Runs, each with its own Manifest whose `run.parent` is the Session, and the Session's `ezsdr.children` listing them; `test_v54_sleep_is_run_time` (Python): `sdr.sleep(10)` advances the Run by 10 s in well under 10 s of wall time (§58 #2) | — |
| §58 #3 | Deterministic with a seed | `test_v58_03_the_same_calls_give_the_same_samples` (Python): two Sessions with the same calls capture the same bytes | — |
| §58 #1, #10 | Binding substitution; no Mock in application logic | `test_v58_01_the_same_script_under_another_profile` (Python): one function run under the default profile and under a user profile; `v58_10` (Rust) over the Python sources (GY-6) | Phase 8 on hardware |
| §58 #9 (Phase 5's "Reactor in a Session") | | `v58_09_a_reactor_runs_in_a_child_run_of_a_session` (acceptance): the ping-pong Spec as a child of a two-radio Session, through the server | — |
| §15 | `run.wait_for(event)` | `kf_02_*` (coordinator); `Session.wait_for` (Python) | — |

### Earlier deferrals → Phase 6

| Item | Where it was deferred | Phase 6 disposition |
|---|---|---|
| Child Runs (`RunChild`, RS-25, RS-25a, KC-37) | Phase 2 Y14; spec 06 KA-19 | KF-3 |
| A Reactor in a Session | Phase 5 §3 | through KF-3 (above) |
| §58 #13 through Python | Phase 1, 2 and 4 §8 | the carriers above |
| Session replay (RS-20's re-application) | Phase 2 §3 | out of scope; owner decision (§11) |
| The artifact store | Phase 2 Y12; Phase 4 §3 | out of scope, with replay |
| YAML (X4) | Phase 1 X4 | settled by S10: the API takes a `dict` |
| The spike's K8 | `plan/spikes/2026-09-26-uhd.md` ("before Phase 6") | Phase 7: a Simulation Run is paced by its client by design |

## 9. Sequencing

| Step | What | Check |
|---|---|---|
| 0 | This file, specs 16 and 17 committed | links resolve |
| 1 | KF-1…KF-4 (Kernel and `design/` text) | workspace tests on 1.85.0 and stable; Clippy; `kernel_surface` 116 / 292; `schema_freeze` |
| 2 | VD-1 (`ezsdr-sink` 1.1.0, `ezsdr-sink-capture` 1.2.0, the profiles that name it) | the same; the sink schema freeze |
| 3 | Spec 16's server | the same; the server's schema freeze |
| 4 | Spec 16's Python package and §8's Python carriers; the acceptance carrier | the Python suite (GY-7) |
| 5 | Appendix A's mutations | each killed |
| — | **Review H** (Opus, AGENTS.md §8): one adversarial pass over the diff and specs 16 and 17, running the tests and the mutations; fixes with tests; a further review unless the fixes are small and low-risk (the owner's instruction) | findings triaged in `implementation-notes.md`, recorded in §11 (OV-5) |
| 6 | Exit tables, Vision issues collected, `handoff.md` | **Gate X** (owner) |
| X | Spec 16 moved to `design/`; Vision issues applied with the owner's approval | links and `v3/` paths recheck |

## 10. Exit criteria

1. Specs 16 and 17 accepted at Gate X, and every decision row — S1–S11 here, A1–A8 of spec 16, the owner decisions of §3 and the reviews' — has a verdict in §11.
2. Every Phase 6 rule — KF-1…KF-4, VD-1, EA-1…EA-19, GY-1…GY-7 and each amended rule — has an OV-3 disposition in `plan/phase6/exit-review/`, read from test bodies (PO-10), with no `GAP` and no `UNCERTAIN`.
3. `cargo test --workspace` passes on Rust 1.85.0 and on stable with no `#[ignore]`; `cargo +stable clippy --workspace --all-targets -- -D warnings` passes.
4. Every carrier of §8 exists and passes; the Python suite passes on Python 3.9 and on a current Python (GY-7).
5. `kernel_surface` (116 NEW / 292 public items), `schema_freeze`, every Vocabulary freeze test and the server's freeze test pass.
6. Every mutation of spec 17's Appendix A is killed (GY-4).
7. The Kernel's direct dependencies are still exactly four; `Cargo.lock` gains no external package (PO-4); the Python package depends on numpy only.
8. Every link in `design/` and `plan/` resolves, and the `v3/` path check of `handoff.md` §1 passes.

## 11. Decision log

| Decision | Gate | Verdict | Note |
|---|---|---|---|
| Plan and implement in one session, then an Opus review loop (S11) | — | **delegated** | owner, 2026-09-26: "それではPhase4/5と同じ用にPhase6も設計と実装をしてください" (Phase 5's instruction: implement, have Opus review, fix, re-review unless the fixes are small and low-risk, and loop) |
| Session replay and the artifact store: out of Phase 6 (§3). Recommendation: build them together in the frontend, after Phase 7 has a real device to replay against; no Kernel type changes, so the freeze does not wait | X | *pending* | |
| KF-4: the builder source hash SB-14 promises. Recommendation: add an optional `source` hash to the Manifest's `spec` section, set by the caller that starts the Run, **before the v4.0 freeze**, with the first Spec builder; until then SB-14 says it is forward | X | *pending* | |
| S1–S11, A1–A8, specs 16 and 17 (exit criterion 1) | X | *pending* | |
