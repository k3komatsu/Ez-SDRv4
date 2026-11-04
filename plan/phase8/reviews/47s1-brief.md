# Brief: note 47 stage 1 — the initial channel set and configuration at `connect`

You are the implementer (Claude Sonnet subagent) in `~/local_work/Ez-SDRv4` (branch `main`, HEAD `6616955` or later). Read `AGENTS.md` first.

**This step is risky** (it changes the Kernel's Session entry and the implicit Spec's identity, and the boundary between a Session's intent and the Providers' initial configuration). It runs in two phases. **Phase 1 (now): write the test plan only, report, and stop. No code.** Phase 2 starts only after Sol has reviewed the plan and the orchestrator says so.

## Why

- The owner decided (plan/phase8/46-streaming-feature-ledger.md §8, F4a): a stream's channel set never changes during a Run. Today a Session puts a transmit channel into a stream after `connect` (`sdr.tx_channels[0].stream = 0`, `python/examples/minimal.py:9-10`); the default layout leaves every transmit channel in no stream (RM-30). So the channel set must be given at `connect`.
- Design: `plan/phase8/47-no-change-while-streaming.md` §2.4 (the connect contract), §6 O1 (owner decision: both (a) and (b)), §4 stage 1. Sol's scope for this stage, with every rule, code region, test and the ten sequences: `plan/phase8/reviews/47-sol2.md` §4 「段階1のscope」. Read both in full.

## Scope of stage 1 (only this)

1. **(a) Initial requests at `connect`.** The Kernel's Session entry (`coordinator::connect`, `crates/ezsdr-kernel/src/coordinator/mod.rs:165-175`; `crates/ezsdr-kernel/src/session.rs:429-494`) takes radio-independent initial requests — per resource name, a map of key → value — and puts them, **before hashing and validation**, into that resource's `requires` as `Constraint::Eq`, and declares the Vocabulary majors they need in the implicit Spec's `requirements`. The Kernel learns nothing about radio keys. The server's connect frame (`crates/ezsdr-server/src/protocol.rs:55-63`, `lib.rs`) and its schema (`schemas/server/request_frame.v1.json`, `ea_06_schema_freeze`) carry them. The Python client (`python/ezsdr/session.py:689-709`) expands `connect(rx={k: [channels…]}, tx={…}, …)` into concrete keys (`radio.<d>.c<n>.stream`), and also accepts initial values for frequency, gain, sample rate and antenna in the same form as note 47 §2.4 O3 (dict keyed by logical channel; `sample_rate` per stream). Pick a clear Python signature and say it in the plan; the orchestrator will rule on it.
2. **Map semantics** (note 47 §2.4): a direction's map is that direction's whole channel set (unlisted channels are in no stream); an omitted direction keeps the default; an empty map puts every channel of that direction in no stream; a duplicate channel, an unknown channel or stream number, or a wrong type is refused by the client before `connect` (no last-wins, no fallback to the default). Sample order stays RM-3's logical order. With more than one radio, the resource name decides the target.
3. **(b) Default layout**: transmit channel 0 joins stream 0 by default, like receive channel 0 (RM-30; `crates/ezsdr-radio/src/channels.rs:421-438,754-755`). Antenna defaults do not change (Sol verified RM-31 holds: every board's receive default is RX2); pin that with a test.
4. **Callers and fixtures**: every caller of the Session entry (kernel tests, acceptance support, sink tests, UHD measurement/H4), the Python fixture `assign_tx` and the setups that use it (`python/tests/test_easy_api.py:59-68` and the tests Sol lists), the examples (`python/examples/minimal.py`, `bench_loopback.py`, `python/ezsdr/__init__.py` docstring) move to the initial path. **Do not remove** the runtime `.stream` setter or its tests, or any move/membership machinery: that is stage 2.
5. **Specs**: amend RS-12, SB-22c and SB-T2 row 3, KC-1, RM-30, EA-4/EA-6/EA-9/EA-10/EA-16/EA-19 as the change requires, in the accepted-spec format (AGENTS.md §2: normative text, a `*Test:*` line, no rationale; history in each spec's Changes table). Vision §3 and §66 examples (`design/vision/01-purpose-and-core-boundary.md:53-72,106,110`, `design/vision/11-boundaries-invariants-and-roadmap.md:167-171`) to the connect form, with a revision-history row in the Vision index. No version bumps, no SCHEMA_CHANGELOG entries; regenerate changed schemas.

Out of scope: refusing changes to a flowing stream, staging, start_rx with values, transmit settings, overlap, repeat switch, removal of any old machinery (stages 2–6).

## Phase 1 deliverable: the test plan

Write `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s1.md`. For each sequence below: the test (crate, name, unit or integration), what it asserts, how the assertion observes the behaviour (which value, record, call trace or Manifest field), and the wrong behaviour that would fail it. Short sequences, not a product of every case. Add any sequence your design needs.

1. No initial requests → `connect`: receive and transmit c0 in stream 0, others in no stream; RM-31 not violated with default antennas; transmit membership alone sends no waveform.
2. One direction given / an empty map → `connect`: the given direction is replaced whole, the omitted direction keeps its default, an empty map leaves every channel of that direction in no stream.
3. Two channels, non-adjacent channels, and two streams → prepare → start: the right channel sets, logical order, one clock per stream; no post-connect membership Action is needed.
4. Initial frequency/gain/rate/antenna → `connect` → first sample / burst: used from the first one. On the UHD FakeDevice, the call trace shows the configure before arm/start.
5. Duplicate channel, out-of-range channel or stream, wrong type → refused by the client with a named error; nothing is silently replaced or defaulted.
6. A transmit channel in a stream plus the same board's receive antenna `TX/RX` → refused by RM-31, including when that receive channel is in no stream.
7. Throughput over RM-32, RF envelope violation, transmit not enabled → refused at the right pipeline stage; nothing starts or transmits. An explicit receive-only set (transmit map empty) works.
8. A coerced initial value → `connect` → finish: requested, effective, coercion/warning and Manifest agree; the implicit Spec declares the Vocabulary major; the same requests give the same Spec hash, different ones a different hash.
9. Default profile / explicit profile / a configured default profile (`Config.default_profile`): the same initial requests mean the same; with two Providers the target is never mixed up.
10. MockRadio and FakeDevice with the same initial set → first receive and transmit: membership, rate, clock and refusals agree; the loopback and the examples work through the initial path.

Also list:
- every existing test, fixture, example and mutation row whose setup moves to the initial path (from Sol's list; check `plan/phase6/tools/mutations.json` P01/P11 and others), and what changes in each;
- **mutation rows you plan**, one line each, ids from **P1319**, token anchors for Rust and exact substrings for other files (AGENTS.md §7, §9), each with the test you expect to kill it;
- for every time read or wait a test adds: why its verdict does not depend on when the test thread runs (`plan/phase8/37-test-thread-timing.md`).

## Rules

- Local cargo is allowed; build output stays in `~/.cache/cargo-target/Ez-SDRv4`; test temp files on `/tmp`. Heavy runs may go to sim02/sim03 through `plan/maintenance/tools/remote/rdev.py`. Python with `PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`.
- Never run `mutate.py` or `remote_mutate.py`; never apply a mutant to the tree. Out-of-tree probes (phase 2) count a compile failure as **not killed**, name the killing test for every row, and save their output to a named log under `/tmp/s1-*.log`.
- Never git commit, push, stash, reset or checkout. One agent edits the tree at a time.
- No step-5 self-review for this step (owner 2026-11-03: Sonnet-implemented steps waive it); Sol reviews the plan and the code.
- In phase 2: `cargo fmt --all` before reporting; `git ls-files -m -o --exclude-standard | xargs chmod go+r`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` passes.

## Report (phase 1, at most 15 lines)

The plan's path; the Python signature you chose; the Kernel entry's new shape; anything in the design you think is wrong or unclear; open questions for the orchestrator.
