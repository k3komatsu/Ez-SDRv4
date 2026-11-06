# Note 47 stage 4 — briefs (2026-11-06/07)

Order: brief-plan (test plan) → brief-plan2 (rev 2) → brief-impl-a (rev 3, part (a)) → brief-impl-b (part (b) as a whole, its first slice) → b2 → b3 → b3b → b4 → b5 → b6 → fix1 (Sol's code review) → gate-fix. The rulings that bind them are in 47s4-dispositions.md. The self-review checklist is last.

---

## brief-plan.md

### Brief: note 47 stage 4 — transmit values change again, on an empty stream (TEST PLAN FIRST)

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`, tree clean. Nobody else edits the tree. **Risky** (a lifecycle, the transmit clock, the boundary between FakeDevice/MockRadio and the X3x0): **this task is the test plan only.** Write it to `~/.cache/ezsdr-fixes/2026-11-06-n47s4/test-plan.md` and stop. Do not change the tree. Sol reviews the plan; you implement only after the orchestrator's go.

## Read

- `AGENTS.md`: Safety core, §7, §9 (risky steps, Maintainability, Counterexamples).
- `plan/phase8/47-no-change-while-streaming.md`: §1; §2.1 items 2, 3, 7 (the transmit parts only; overlap and repeat switching are stage 5); §2.2; §2.3; §2.4 「開始と設定」「送る前に全部を確かめる」「送る順と部分的な失敗」「読み返す値は admitted の値」; §3 Q2, Q6; §4 stage 4, stage 2's 「段階 3・4 に残したこと」, stage 3's 済 paragraph (the receive form this stage mirrors) and the bench paragraph.
- `plan/phase8/48-simplification-inventory.md` rows B4 and B6, §3.4, §4 item 4, and decision 1 under §5.
- Stage 3, the receive form to mirror: `plan/phase8/reviews/47s3-test-plan.md`, `47s3-dispositions.md`; the code `crates/ezsdr-radio/src/staging.rs`, `crates/ezsdr-radio-uhd/src/provider/control.rs` (`book`, `book_burst`, `ColdConfig`), `crates/ezsdr-radio-uhd/src/provider/core.rs` (`owner_configure`, `record_configured`, `configure`), `crates/ezsdr-mock-radio/src/lib.rs` (the receive staging and snapshots; `TxStream`, `TxClock`, `tx_reconcile`, `tx_gen`), `crates/ezsdr-mock-radio/src/channel.rs` (`ChannelModel`).
- The transmit owner: `crates/ezsdr-radio-uhd/src/provider/tx.rs` (`Tx`: `plan`, `version`, `plan_section`, `retarget`, `end_clock`, `cut`, `dispatch_start`, `send`, `reopen_streamer`, the recovery).
- Bench evidence: `plan/phase8/bench-results.md` §「2026-11-05」 item T5 (line ~17) and T6.
- Specs: `design/07-radio-model.md` RM-13–RM-16, RM-21, RM-25, RM-27; `design/09-mock-radio.md` MR-16–MR-18, MR-25, MR-32–MR-35; `design/18-uhd-radio.md` UR-21–UR-26, UR-36; `design/16-easy-api.md` EA-16, EA-17; `KNOWN_LIMITATIONS.md`.
- Python: `python/ezsdr/session.py` (the transmit stream's `send`/`repeat`, the channel setters that raise `Error` today, `start()` at ~318 as the receive form).

## Orchestrator's rulings (they bind the plan)

**R1 — one transmit clock per stream for the whole Run.** A transmit stream's rate stays fixed for the Run in this stage (`radio.tx.s<k>.sample_rate_hz` stays classless: the bench condition is not met, #1 INCONCLUSIVE). Frequency, gain and antenna do not change sample timing, so a burst that carries new values registers **no new SampleClock**: it targets the clock registered at `arm`, as every burst does today. Consequences:
- note 47 §4 stage 4's open question (re-review 5: a refused configure ends the new clock at its origin and the Kernel then refuses the corrected burst, `admission.rs:150-162`, `state.rs:336-339`) does not arise in this stage. Record it as a named open item for the stage that accepts a transmit rate change.
- **B4 is done in this stage as part (a)**: the transmit plan has one section for the Run. Collapse the multi-section machinery to one clock: UHD `Tx.plan: Vec<Planned>`, `version`, `plan_section`, `retarget`, `end_clock`, `cut`, the `Kind::Stop` version bump (note 48 §3.4 last item); MockRadio `TxStream.clocks: Vec<TxClock>`, `cur`, `tx_gen`, `tx_reconcile`. Keep what B4's row keeps: the clock ends at the stream's end, at a loss and at `Provider::stop`, **including before its origin, at the booking** (`core.rs` ~669-679, stage 2's Sol P2); the Manifest record and the report ids. For each removed piece, say in the plan why nothing reaches it (evidence: path:line) and which tests and mutation rows go, retarget or stay.

**R2 — "not streaming" for transmit, decided at receipt from the booking record** (RM-21 extended; note 47 §2.1 item 2's acceptance emptiness). A transmit channel's `frequency_hz`, `gain_db`, `antenna` get class `cold`. A value is staged only when the channel's own stream is not streaming: **every burst booked on that stream (accepted, not cancelled by a `Stop`, a loss or `Provider::stop`) has an exclusive end at or before the receipt instant**. The end is the booked start (after any `SendAsap` move) plus its length; a repeat has no end until a `Stop` cuts it, and then its end is the cut. Otherwise `radio.COMMAND_REJECTED` with the Action's id and a reason naming the rule. The decision must not depend on how far the transmit owner has handed samples (the decide-at-receipt vs decide-at-dispatch split is #79's mechanism, `radio-rule-implemented-twice`). Write the rule **once**, pure, in `ezsdr_radio` (extend `staging.rs`: transmit keys, the transmit refusal causes), with each Provider passing only its booking record's answer; say in the plan exactly what each Provider passes and why the two answers agree. Peer isolation as on receive: another stream's state does not matter.

**R3 — the burst's snapshot.** A `TxBurst` booking takes a copy of the stream's staged transmit values, as `start_rx` does (`ColdConfig::of(snapshot, id)`). A value cannot change between two bursts of one device burst (it would be refused while the first is booked), so a contiguous burst always carries the same values as the burst it continues.

**R4 — applying a snapshot (UHD).** The transmit owner (uhd-tx) applies a burst's snapshot with an **untimed** configure (`core.configure(Dir::Tx, …)`, holding the owner claim) before the burst's first send, **only when no device burst is open on the stream and the device time has passed the previous burst's exclusive end**, and not during the recovery (F8; the same thread serialises them). Basis, VERIFIED (bench 2026-11-05 T5, both boards): untimed transmit settings from ≈0.2 ms after the previous burst's end left its tail intact and the next burst took the new values; an untimed setting 4.6 ms before the end changed the tail at once (the destructive control). Configure once: no device call when the snapshot equals what the device holds (the `configured` mark, `Owner::Nothing`); a failed configure clears the mark so the next burst configures fully. After the configure, the burst's start is decided again by its late policy (`dispatch_start`): a configure that leaves it late makes it late by its policy, never silently moved. **MockRadio**: its configure takes no time; the values take effect at the burst's start (B6: the transmit `ChannelModel` timelines get `.set()` at each burst start whose values differ, and keep their history so a sample is computed with the values in effect at its instant). Say in the plan whether any Mock/Module difference remains (the Module's configure is 15–42 µs, T6) and why it is not observable, or state the sequence where it is.

**R5 — a refused configure.** When the device refuses a burst's configure, that burst is refused (`radio.COMMAND_REJECTED`, kind `tx_burst`, the burst's Action id) and not sent; the stream does not halt; the refused value stays the configuration of record (KC-27), so a later burst with the same snapshot is refused again. **Fail → fix → resend** must work: a corrected value on the (now empty) stream is accepted and the next burst plays with it on the same clock, admitted by the Kernel. The Mock has no device refusal (an out-of-envelope value is refused at admission by both); say how parity covers this, or that it is a UHD-only path tested on FakeDevice.

**R6 — `radio.timing.tx_start_lead_ns`** decides nothing since stage 2. If nothing reads it after this stage, delete it (profile, schema regenerated with `schema_freeze` green, description, `measurement.rs`, fixtures; no version bump, no `SCHEMA_CHANGELOG`). The plan lists every reader with path:line; if deleting reaches more than the radio crates and their fixtures, say so and recommend.

**R7 — Easy API** (EA-16, EA-17; note 47 §2.4): the transmit stream's `send(…)` and `repeat(…)` take `frequency=`, `gain=`, `antenna=` (dicts keyed by logical channel, checked before sending as `start()` does) and send one `UpdateParameter` per value, then the burst; on the first `Rejected` they raise and send nothing more. `sample_rate=` raises `Error` (the bench condition). The transmit channel setters send the update and raise `Rejected` when the Provider refuses it. Find today's transmit verbs' names and signatures; keep their other parameters.

**R8 — inherited items** (note 47 §4 「段階 3・4 に残したこと」; decide each in the plan): `CancelReason::ColdChange` (rename or delete, by its remaining users); MockRadio's `applied` section (always empty: delete, or say what fills it now); §58 #16 gains a transmit form (an out-of-envelope transmit frequency on an empty stream is refused at admission, logged, not dispatched); Vision §15, §27 and §58 #16 text that the values stages made false (list each sentence with path:line; the text goes in the implementation).

**Out of scope** (stage 5): the overlap refusal, the repeat switch, removing preemption (`Tx::preempt`), the in-flight window question, #79. Transmit rate change. Do not add mechanisms beyond R1–R8.

## Sequences that matter (each needs a test; expected result)

- S1 connect tx s0 c0 at f1. Burst A plays; after A's end, set c0 f2; burst B. UHD (FakeDevice call log): the transmit apply with f2 comes after A's last packet and its end has passed, before B's first `tx_send`. MockRadio: A's samples carry f1, B's f2 (value history; a loopback receive sees it). Both Providers accept the value.
- S2 A booked for the future → set f2 before A starts: refused with the Action's id, no device call; a later burst uses f1.
- S3 a repeat is playing → set f2: refused. `Stop` → set f2: accepted → burst B plays with f2.
- S4 empty → set f2 → set f3 → burst: f3. Empty → set f2 → burst B booked → set f4 while B is booked: refused; B uses f2.
- S5 a value received at A's exclusive end exactly, and one sample before it: accepted, refused (the boundary, same on both Providers).
- S6 two transmit streams (or two channels in two streams, if a layout allows; else say why not): stream 0 busy, stream 1 empty: a value for stream 1's channel is accepted and stream 0 is undisturbed (no device call for it, no gap).
- S7 the device refuses the configure (FakeDevice apply failure): the burst is refused with its id and not sent; a second burst with the same snapshot is refused again; a corrected value → accepted → the next burst plays with it on the same clock (fail → fix → resend, through the Kernel's admission, not only the Provider rig).
- S8 a configure that ends too late for the burst's start: the burst's late policy decides (`SendAsap`: `TIME_ERROR` and moved; `Drop`: dropped) — never sent late silently.
- S9 no apply while a device burst is open: a contiguous burst (no value change) continues the device burst with no configure call; a burst whose snapshot equals the device's makes no configure call (configure once).
- S10 recovery and configure: an underflow recovery (F8) in progress when a burst with new values is due: the configure comes after the recovery ends; `Stop`, loss and `Provider::stop` during or before the configure leave no half-applied record (the mark cleared or kept, as the plan decides).
- S11 B4: a loss and `Provider::stop` before the transmit clock's origin end the clock at the booking (stage 2's Sol P2), and a burst after a `Stop` still targets the one clock.
- S12 §58 #16, transmit: an out-of-envelope transmit frequency on an empty stream is refused at admission, logged, not dispatched; `sample_rate_hz` is refused (classless) and the Python error names the bench condition.
- S13 Python: `send(x, frequency={0: f2})` on a busy stream raises `Rejected` and sends no burst; `tx_channels[0].gain = g` on an empty stream is accepted and the next burst uses it.
- S14 parity: S1–S5 decide the same on MockRadio and the UHD Module (acceptance `parity.rs`).

Hardware/UHD behaviour these rely on — mark each VERIFIED (path:line or URL) or INFERRED in the plan: the untimed transmit settings' safety after the previous burst's end (T5); that `set_tx_rate` and the transmit settings do not wait for a burst to finish (note 47 Q2); the configure's duration (T6).

## The test plan

Per AGENTS §9 step 2. Part (a) B4 first, then part (b) the values; the implementation will run in that order and leave the tree green after (a). For each sequence S1–S14 and each state or failure path the step adds: the test (file, name), what it asserts, how the assertion observes the behaviour, the wrong behaviour that fails it, and the mutation row (Rust token anchors, exact substrings elsewhere) with its killing test. List the existing tests and rows that change, move or go, each with the reason (for a deleted test of reachable behaviour, its replacement). The spec rules each test pins. Your own failure scenarios beyond S1–S14. Test pure decisions at the pure level; keep device-path tests for wiring and ordering; no new test that depends on the wall clock. Keep to what the step needs.

## Rules

- Read only: do not edit the tree or the orchestrator's files in `~/.cache/ezsdr-fixes/2026-11-06-n47s4/` other than writing `test-plan.md`.
- Never run `mutate.py`, `remote_mutate.py` or `gate.py`; never apply a mutant to the tree. A probe build, if you need one, goes in a throwaway copy outside the repository (`rsync -a --no-times --exclude .cargo --exclude target`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`). Do not load the local machine with long builds; prefer reading.
- Commands that may run long get an explicit timeout.

## Report (at most 15 lines)

Where the plan is; the open questions the orchestrator must decide, each with your recommendation; the claims you could not verify.

---

## brief-plan2.md

### Brief: note 47 stage 4 — test plan revision 2

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, HEAD `aee03ea`, clean tree. Read `AGENTS.md` §9 and, in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: the brief `brief-plan.md`, the plan `test-plan.md` (rev 1, by a previous run), the orchestrator's `rulings-plan.md` (P1–P10), Sol's review `sol-plan-final.md` and `rulings-sol-plan.md` (1–8; it overrides `rulings-plan.md` where they differ).

Task: revise the plan to **rev 2** (copy rev 1 to `test-plan.v1.md` first) so it implements every ruling: P1–P10 and 1–8. Remove what the rulings drop (P-1 and its exception test, N47S4-05, N47S4-A1 unless replaced, `Tx.applied`), restate §0 as decided (each question with its ruling), and add a "Changes in rev 2" list at the top mapping each ruling to the sections it changed. Read the code each change touches and cite path:line; where the rulings ask you to check something (today's clock-end semantics, FakeDevice's two-transmit-stream support, the loss path and `configured`, the admission point of a transmit key's RF envelope check, every `cut`/`plan`/`at`/`version` user), check it and write what is true, VERIFIED or INFERRED. Every new or changed test names its killing row and every row its killing test.

Do not change the tree; write only `test-plan.md` and `test-plan.v1.md` in that directory. Never run `mutate.py`, `remote_mutate.py` or `gate.py`. Prefer reading to building.

Report (at most 12 lines): per ruling, where it landed; anything you could not do; new open questions with your recommendation.

---

## brief-impl-a.md

### Brief: note 47 stage 4 — plan rev 3, then implement part (a) (B4: one transmit clock per stream)

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`, clean tree. Nobody else edits the tree. Read `AGENTS.md` (Safety core, §7, §9 including Maintainability) and, in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `brief-plan.md` (the step), `test-plan.md` rev 2, and the rulings in order — `rulings-plan.md`, `sol-plan-final.md` + `rulings-sol-plan.md`, `opus-plan-recheck.md` + `rulings-recheck.md` (the last overrides earlier ones where they differ).

## Task

1. **Revise the plan to rev 3** (copy rev 2 to `test-plan.v2.md` first) with every item of `rulings-recheck.md` (O-1–O-3, clock end, M1–M4, m2–m9), and a "Changes in rev 3" list at the top. Cite path:line for what you read.
2. **Implement part (a) only** (B4, rev 3 §2): one transmit clock per stream in the UHD Module and MockRadio, `version` dropped (the cut read in `pass`, O-2), the cut arithmetic in `send`, `refuse_first_send` and the tests and rows rev 3 lists deleted or moved, the Mock's transmit clock end kept as a one-clock function with its new test and row (M4), the transmit abort case in `rm_26_the_timeline_cases` (m10), UR-22's clock-cut wording and any spec text part (a) makes false. **No behaviour change**: every surviving test passes unchanged in what it asserts. Do not start part (b).
3. Mutation rows of part (a): text edits only in `plan/maintenance/tools/mutations.json` (and `plan/phase7/tools/mutations.json` if a row lives there); never re-serialize. Every removed row has its reason in the plan; every retargeted or new row matches exactly once (`plan/maintenance/tools/anchor.py`) and names its killing test. Confirm each new or retargeted row by applying it in a throwaway copy **outside the repository** (`rsync -a --no-times --exclude .cargo --exclude target`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, touch the copy before building, one copy at a time) and running its killer; a compile failure counts as not killed; logs under `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/probes-a/`. Delete the copy afterwards.
4. Removed or renamed tests: list them in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/renames.txt` in the format of `plan/phase8/reviews/48s3-renames.txt` (old name → new name or `-` with the reason), and grep `mutations.json` for rows whose `filter` names a removed test and rows whose `new` names a removed identifier.

## Rules

- Never run `mutate.py`, `remote_mutate.py` or `gate.py`; never apply a mutant to the tree. No git state changes (no add, commit, stash, checkout).
- Before reporting: `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` (or the affected crates first, then the workspace once); clippy for `ezsdr-radio-uhd --features uhd --all-targets` through `plan/maintenance/tools/remote/rdev.py -- <command>`; `git ls-files -m -o --exclude-standard | xargs chmod go+r`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/renames.txt` → 0 problems. Run long commands in the foreground with an explicit timeout (a test command up to 1800 s) and wait for them to end.
- Test temp files on `/tmp`. No test that depends on the wall clock. Leave no test process running. No `__pycache__` in the tree (`PYTHONDONTWRITEBYTECODE=1`).
- No version bumps, no `SCHEMA_CHANGELOG` entries.
- Do not delete a test of reachable behaviour without its replacement named in the plan.
- If your context gets tight, stop at a green point, write what is done and left into the plan (§8), and report.

## Report (at most 15 lines)

Plan rev 3: per ruling, where it landed. Part (a): what was removed and kept; tests removed, moved, added; rows removed, retargeted, added with probe outcomes; the end checks and their results; deviations from the plan and why; open questions.

---

## brief-impl-b.md

### Brief: note 47 stage 4 — implement part (b) (transmit values on an empty stream)

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`. The working tree holds part (a) of this stage, uncommitted: do not revert it, do not touch the index. Nobody else edits the tree. Read `AGENTS.md` (Safety core, §7, §9 including Maintainability) and, in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `brief-plan.md` (the step, R1–R8), `test-plan.md` rev 3 (part (b): §3 onward, §4 rows, §5 parity limits, §6 texts), the rulings in order — `rulings-plan.md`, `rulings-sol-plan.md`, `rulings-recheck.md` (the last overrides) — and `brief-impl-a.md` (its Rules section applies here unchanged, with probe logs under `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/probes-b/`).

## Task

1. **Implement part (b)** as rev 3 says: transmit `frequency_hz`, `gain_db`, `antenna` get class `cold` (rate stays classless); the shared `Busy` fold and the transmit staging in `ezsdr_radio`, each Provider only wiring it (booking, `Stop`, value receipt); the burst's snapshot with its ActionId; the UHD owner's apply as its own turn under the guard, `rate: None`, the generation mark (arm publishes `Some(0)`, a failed configure `None`, a loss clears it, publication only if not lost); a refused configure refuses that burst with its id; MockRadio's values take effect at the burst's start with the transmit value history (B6); FakeDevice's persistent value-keyed transmit apply refusal; `radio.timing.tx_start_lead_ns` and `timeline::start_lead_ns`'s transmit arm removed (schema regenerated, `schema_freeze` green, fixtures and the measurement harness, no version bump, no `SCHEMA_CHANGELOG`); Python `repeat(x, at, *, frequency=, gain=, antenna=)` with the checks before sending, the transmit setters, `sample_rate=` → `Error` naming the bench condition; §58 #16's transmit form; every spec, Vision and KNOWN_LIMITATIONS text in rev 3 §6 (RM-21's "at most one in-flight window after a Stop", the parity limits including #79's, generation-based configure-once), with Changes rows.
2. Every test and row in rev 3 §3–§4 for part (b), each row confirmed by a probe as in `brief-impl-a.md`; extend the plan (rev 4, keep rev 3 as `test-plan.v3.md`) for every state or failure path the implementation adds, and record deviations in its §8.
3. Removed or renamed tests go into `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/renames.txt` (append).
4. **Self-review, one round, before reporting** (AGENTS §9 step 5): run, in the foreground, `timeout 2400 opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max --agent plan --auto "$(cat /home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/self-review-checklist.md)"` from the repository, saving its output to `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/self-review.log`. If the free model is unavailable or times out, use `opencode-go/deepseek-v4.1-flash#max` once. Fix every finding or reject it with its reason in the report. The reviewer must not run mutation; a gate it leaves to the orchestrator is not a finding.

## End checks

As in `brief-impl-a.md`, plus the Python suites: `EZSDR_SERVER` pointed at the `ezsdr-server` built from this tree, `PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests` (Python 3.14), and the 3.9 container command in `handoff.md` §5. New tests whose verdict can depend on timing (a manual clock with threads, probes) run 20 times. `rules.py --base HEAD --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/renames.txt` → 0.

## Report (at most 20 lines)

What is implemented per plan item; tests and rows added, changed, removed, with probe outcomes; suites and their counts; `rules.py`; the self-review's findings and their disposition; deviations and open questions.

---

## brief-impl-b2.md

### Brief: note 47 stage 4 — part (b), sub-step b2: the receipt wiring on both Providers

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`. The working tree holds part (a) and the first slice of part (b) (the pure `Busy` in `crates/ezsdr-radio/src/staging.rs` and its tests), uncommitted: do not revert it, do not touch the index. Nobody else edits the tree. Read `AGENTS.md` (Safety core, §7, §9 including Maintainability), and in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `brief-impl-b.md` (the whole of part (b); its End checks apply, except the self-review, which comes in b4), `brief-impl-a.md` (its Rules apply; probe logs go under `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/probes-b2/`), `test-plan.md` rev 4 (part (b) and §8 deviations D-1–D-9), and the rulings `rulings-plan.md`, `rulings-sol-plan.md`, `rulings-recheck.md` (the last overrides).

Part (b) is split into three sub-steps, each leaving the tree green: **b2** (this one), **b3** (the UHD owner's apply: configure turn, mark lifecycle, FakeDevice refusal, owner-guard tests, S7–S10), **b4** (the class flip to `cold`, Kernel-path tests S1/S12/S14, Python, `tx_start_lead_ns` removal, spec and KNOWN_LIMITATIONS text, the self-review).

## b2's scope

The transmit keys stay classless in b2, so the Kernel still refuses them in a Run; what b2 adds is reached through Provider-level tests (as stage 3's `staging_rig` in `control.rs` and MockRadio's direct booking tests do).
1. `Staging` accepts transmit `frequency_hz`, `gain_db`, `antenna` (rate refused as `NotStaged`), with the transmit stream's `Busy` answer; each accepted transmit value bumps that stream's generation (the snapshot carries it).
2. Both Providers keep one `Busy` per transmit stream and call only the shared fold: `book` at a burst's booking (the booking-time `SendAsap` move included; a booking dropped at booking is not booked), `stop` at a `Stop` (and at loss and `Provider::stop` if rev 4 says so), `stageable` at a value's receipt. Never `core.held` or hand-over state (row N47S4-08). Refusal: `radio.COMMAND_REJECTED` with the value's Action id.
3. A `TxBurst` booking takes the stream's snapshot with its ActionId (UHD: `ColdConfig::of(snapshot, id)` on `Held`; MockRadio: its equivalent). UHD does not apply it yet (b3).
4. MockRadio: the snapshot's values take effect at the burst's start, through the transmit `ChannelModel` timelines with their history (B6, F6).
5. Tests and rows of rev 4 for these items at the Provider level (S2–S6, F2, F6, F9 inverted, the peer test), each row probed; extend the plan to rev 5 (keep rev 4 as `test-plan.v4.md`) and record deviations in §8.

Stop after b2 at a green point. If your context gets tight earlier, stop at a green point and write what is done and left into §8.

## Report (at most 15 lines)

What b2 implemented; tests and rows added and changed with probe outcomes; end checks; deviations; what b3 and b4 must know.

---

## brief-impl-b3.md

### Brief: note 47 stage 4 — part (b), sub-step b3: the UHD owner's apply

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`. The working tree holds part (a), b1 (the pure `Busy`) and b2 (the receipt wiring on both Providers, the snapshot with its ActionId on `Held.settings`, MockRadio's value history), uncommitted: do not revert it, do not touch the index. Nobody else edits the tree: you are the only agent. The `opencode run` process, `impl-b3.pid` and `impl-b3.log` you may see are your own run; do not look for other runs or stop for them. Read `AGENTS.md` (Safety core, §7, §9 including Maintainability), and in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `brief-impl-b.md` (its End checks apply, except the self-review, which comes in b4), `brief-impl-a.md` (its Rules apply; probe logs under `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/probes-b3/`), `brief-impl-b2.md` and the b2 report at the end of `impl-b2.log`, `test-plan.md` rev 5 (§8 deviations D-1–D-18), and the rulings `rulings-plan.md`, `rulings-sol-plan.md`, `rulings-recheck.md` (the last overrides; read M1, M2, M3, m4, m6, m7, m9 there closely).

## b3's scope (UHD Module and FakeDevice; the transmit keys stay classless, so tests are Provider-level, as in b2)

1. **The configure turn** (R4, M2): before a burst's first send, when its snapshot's generation differs from the stream's `configured` mark, uhd-tx configures in a turn of its own, holding the owner claim, under the owner guard (no open device burst; device time at or after the owner's own last closed device burst's end, padding and held tail included; not during the recovery), with `rate: None` (m4). On success it publishes `Some(g)` under the streams lock only if the stream is not lost, and returns with the burst still held; the next turn drains commands (a `Stop` → `stop_now`, `CancelReason::Stop`) and re-decides the burst with `dispatch_start` (its late policy: `SendAsap` moves with `TIME_ERROR`, `Drop` drops).
2. **The mark** (P5, M1): arm publishes `Some(0)` after the transmit open and its rate check succeed; a failed configure sets `None`; a loss clears it; a reopen keeps it (as today). Remove the `#[allow(dead_code)]` b2 left on `Held.settings` and `ColdConfig.generation`.
3. **A refused configure** (R5): the burst is refused (`radio.COMMAND_REJECTED`, kind `tx_burst`, the burst's Action id), not sent; the stream does not halt; it stays counted in `Busy` until its booked end (F9's refused-later case).
4. **FakeDevice**: a persistent transmit apply refusal keyed to a frequency on a channel, until disarmed (M3), armed after prepare; the partial case (c0 applies, c1 refuses) leaves the mark `None` and the retry configures both channels.
5. **Tests and rows** of rev 5 for these items: S7, S8, S9, S10, F5, F9 refused-later, the owner-guard tests (a) moved at dispatch, (b) one sample padded to four, (c) a Stop's held tail at exactly e with device time held in [e, e+2) (m9), M1's first burst with no new value makes no apply, M2's Stop/loss/`Provider::stop` during a paused configure (the existing `apply` probe between channels), and the two-transmit-stream fixture if FakeDevice supports it (b2 deferred S6's Provider path). Observe order through `calls()` and time on the manual clock (no log format change, m7). Each row probed. Extend the plan to rev 6 (keep rev 5 as `test-plan.v5.md`), deviations in §8.

Stop after b3 at a green point. If your context gets tight earlier, stop at a green point and write what is done and left into §8.

## Report (at most 15 lines)

What b3 implemented; tests and rows added and changed with probe outcomes; tests repeated 20 times (any whose verdict can depend on threads or probes); end checks; deviations; what b4 must know.

---

## brief-impl-b3b.md

### Brief: note 47 stage 4 — part (b), sub-step b3b: the remaining b3 tests and rows

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`. The working tree holds part (a), b1, b2 and b3 (the configure turn, the mark, the refusal, FakeDevice's persistent refusal, D-19's continuation rule), uncommitted: do not revert it, do not touch the index. Nobody else edits the tree: you are the only agent. The `opencode run` process, `impl-b3b.pid` and `impl-b3b.log` you may see are your own run; do not look for other runs or stop for them.

Read `AGENTS.md` (Safety core, §7, §9), and in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `brief-impl-b3.md` (b3's scope; its rules apply; probe logs under `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/probes-b3b/`), the b3 report at the end of `impl-b3.log`, `test-plan.md` rev 6 (§8, D-19–D-24), and `rulings-recheck.md` (M2, m9).

## Task

Build what b3 left (rev 6 §8, D-24), each test with its row probed and named killer, production code changed only where a test shows a defect (then say so):
1. The owner-guard tests (a) a burst moved at dispatch beyond its booked end, the value accepted after the booked end; (b) a one-sample burst padded to four, the value at its payload end; (c) a `Stop`'s held tail, the value exactly at e, device time held in [e, e+2) (m9). Each holds device time before the relevant end (no apply), then crosses it (apply before B's first send). Rows N47S4-09a/b/c as concrete text substitutions in the guard code.
2. S8, the slow configure: `SendAsap` moved with `TIME_ERROR`, `Drop` dropped (N47S4-15b).
3. `Provider::stop` during a paused configure: the mark, the claim's release and the `ProviderStop` cancellation (M2).
4. F5: a reopen keeps the mark; a loss clears it.
5. S10: an underflow recovery in progress when a new-value burst is due: the configure comes after the recovery.
6. S6 on the Provider path, if FakeDevice supports two transmit streams; else record why not.
7. Rows N47S4-12c, -14, -33, each with a killer or recorded as equivalent with the reason.
Tests whose verdict can depend on threads or probes run 20 times. Extend the plan to rev 7 (keep rev 6 as `test-plan.v6.md`), deviations in §8. End checks as in `brief-impl-a.md` (Python only if a Python file changed). Stop at a green point if your context gets tight and write what is left into §8.

## Report (at most 12 lines)

Per item, done or not and how; rows with probe outcomes; repeats; end checks; production changes, if any, and why.

---

## brief-impl-b4.md

### Brief: note 47 stage 4 — part (b), sub-step b4: the class flip, the Kernel path, Python, the texts, and the self-review

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`. The working tree holds part (a), b1, b2, b3 and b3b, uncommitted: do not revert it, do not touch the index. Nobody else edits the tree: you are the only agent. The `opencode run` process, `impl-b4.pid` and `impl-b4.log` you may see are your own run; do not look for other runs or stop for them.

Read `AGENTS.md` (Safety core, §7, §9 including Maintainability), and in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `brief-impl-b.md` (part (b) as a whole; **its End checks and its self-review step 4 apply here**), `brief-impl-a.md` (Rules; probe logs under `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/probes-b4/`), `test-plan.md` rev 7 (§3–§6, §8 D-1–D-34), `rulings-plan.md`, `rulings-sol-plan.md`, `rulings-recheck.md` (the last overrides).

## b4's scope

1. **The class flip**: transmit `frequency_hz`, `gain_db`, `antenna` get class `cold` (RM-27); `sample_rate_hz` stays classless. Fix every test the flip breaks by its subject (do not delete a test of reachable behaviour without its replacement).
2. **Kernel-path tests**: S1 end to end on both Providers (the value after A's end, B with it; MockRadio's radiated samples carry f1 then f2, killing N47S4-46/-47 that b2 deferred), S12 (§58 #16's transmit form in `crates/ezsdr-acceptance/tests/v58.rs`; N47S4-19 anchored on the real admission check `ezsdr-radio/src/lib.rs` ~437), S14 parity (`crates/ezsdr-acceptance/tests/parity.rs`: S1–S5 decide alike on both Providers, with bookings both accept alike — bursts over 32 samples, gaps of at least the window; #79's divergence is recorded, not tested), and F9's refused-later case through the Kernel if b3 left it Provider-only.
3. **Python** (EA-16): `repeat(x, at=None, *, frequency=None, gain=None, antenna=None)` with the checks before sending (channel in the stream, types, the waveform), one `UpdateParameter` per value then the burst, stopping at the first `Rejected`; the transmit channel setters; `sample_rate=` and the transmit rate setter raise `Error` naming the bench condition; the changed tests in `python/tests/test_easy_api.py` (130–139, 143–163) and the new ones of rev 7 S13 and Sol 8.
4. **`radio.timing.tx_start_lead_ns` removed** with `timeline::start_lead_ns`'s transmit arm, every reader of rev 7 §6.3 (including the receive callers m3 lists), the schema regenerated with `schema_freeze` green, fixtures and the measurement harness, mutation rows (P11, F36, F40, the swap rows) retargeted or removed with reasons. No version bump, no `SCHEMA_CHANGELOG`.
5. **Texts**: every sentence of rev 7 §6.2 in `design/07`, `design/09`, `design/16`, `design/18` (with Changes rows), the Vision (§15, §27, §58 #16; a revision-history row in the index), `KNOWN_LIMITATIONS.md` (KL-11, KL-35, the parity limits, #79's divergence for transmit values, the floors entry), test-file comments.
6. Rows for b4's items, each probed; plan rev 8 (keep rev 7 as `test-plan.v7.md`), deviations in §8; renames appended to `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/renames.txt`.
7. **Then the self-review** (`brief-impl-b.md` step 4: one round, `self-review-checklist.md`, free DeepSeek first, Go DeepSeek once if it fails), fix or reject every finding with its reason.

End checks: everything in `brief-impl-b.md` (Python 3.14 and the 3.9 container: `docker run --rm -v $PWD:/w -w /w -e EZSDR_SERVER=<server built for the container> python:3.9-slim …`, or the command b2 used — record the exact command you ran), 20 repeats for timing-dependent new tests, `rules.py --base HEAD --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/renames.txt` → 0. If your context gets tight, stop at a green point, write what is left into §8, and report; the self-review then runs in a later sub-step.

## Report (at most 20 lines)

Per item, done or not; tests and rows with probe outcomes; suites with counts; `rules.py`; the self-review's findings and their disposition; deviations; anything left.

---

## brief-impl-b5.md

### Brief: note 47 stage 4 — part (b), sub-step b5: what b4 left, and the self-review

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`. The working tree holds part (a) and b1–b4, uncommitted: do not revert it, do not touch the index. Nobody else edits the tree: you are the only agent. The `opencode run` process, `impl-b5.pid` and `impl-b5.log` you may see are your own run; do not look for other runs or stop for them.

Read `AGENTS.md` (Safety core, §7, §9), and in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `brief-impl-b4.md` (b4's scope; its rules and end checks apply), the b4 report at the end of `impl-b4.log`, `test-plan.md` rev 8 (§8, D-35–D-38), `rulings-recheck.md`.

## Task (rev 8 §8's "left" list)

1. The Mock S1 radiated-sample test (A's samples carry f1, B's f2), killing N47S4-46/-47; if `radiated` cannot observe it, use the loopback receive of a Session (EA-17's pattern).
2. The S14 parity test in `crates/ezsdr-acceptance/tests/parity.rs` (S1–S5 decide alike on MockRadio and the UHD Module, with bookings both accept alike).
3. F9 through the Kernel (a refused configure's burst still counts in `Busy` until its booked end), on the UHD Session path with FakeDevice.
4. The Python S1, S2, S3, S4 and S6 forms of rev 8.
5. Probe every row b4 retargeted (P11, P624, P1393, F36, F40, N47S3-01, N47S3-33) and every new row. **N47S3-01's `new` is now the tree's own text** (the class flip made every channel's key `cold`), so the row mutates nothing: delete it, or retarget it to a mutant that is still wrong (say, the transmit rate becoming `cold`) with a killer. Add the Python rows N47S4-22–27 to `mutations.json` (exact substrings) and probe them.
6. The end checks of `brief-impl-b4.md`, including clippy for `ezsdr-radio-uhd --features uhd --all-targets` through `plan/maintenance/tools/remote/rdev.py`, which b4's report did not list.
7. **Then the self-review** (`brief-impl-b.md` step 4: one round, `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/self-review-checklist.md`, free DeepSeek first, Go DeepSeek once if it fails, output to `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/self-review-b.log`); fix or reject every finding with its reason. After the fixes, run the end checks again.

Plan rev 9 (keep rev 8 as `test-plan.v8.md`), deviations in §8. If your context gets tight, stop at a green point and write what is left into §8; the self-review must still run, so do it before anything optional.

## Report (at most 15 lines)

Per item, done or not; rows with probe outcomes; suites with counts; `rules.py`; the self-review's findings and their disposition; anything left.

---

## brief-impl-b6.md

### Brief: note 47 stage 4 — part (b), sub-step b6: the last tests and rows

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`. The working tree holds part (a) and b1–b5, uncommitted: do not revert it, do not touch the index. Nobody else edits the tree: you are the only agent. The `opencode run` process, `impl-b6.pid` and `impl-b6.log` you may see are your own run; do not look for other runs or stop for them.

Read `AGENTS.md` (Safety core, §7, §9), and in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `brief-impl-b4.md` (rules and end checks apply; probe logs under `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/probes-b6/`), the b5 report at the end of `impl-b5.log`, `self-review-b.go.log` (the self-review's findings), `test-plan.md` rev 9 (§8, D-39–D-45 and the "left" list).

## Task: rev 9's left list, tests and rows only

1. **The reopen's loss branch** that b5 added in `tx.rs` (self-review fix): a test that fails without it and a row on it with that killer.
2. **S14 parity** in `crates/ezsdr-acceptance/tests/parity.rs`: S1–S5 decide alike on MockRadio and the UHD Module (accepted/refused and the Action ids; not sample values), with bookings both accept alike (bursts over 32 samples, gaps of at least the in-flight window).
3. **S7 and F9 through the Kernel** on the UHD Session path with FakeDevice: a refused configure refuses the burst with its id; a value before its booked end is refused; a corrected value after it is accepted and the next burst plays with it on the same clock.
4. **S8**: the pure late-decision test and the `SendAsap` case (moved, `TIME_ERROR`).
5. **S9**: a later burst with no new value makes no configure call.
6. **S10 MockRadio**: a `Stop` before the burst with a new value cancels it and keeps the value of record.
7. **R2's booking-time move**: a burst moved by `SendAsap` at booking counts in `Busy` from its moved start.
8. **Python S6** (two transmit streams), if the Mock profile has two; else record why not.
Every new test has its row probed with its named killer, or is recorded as covering a row that already exists. Production code changes only where a test shows a defect; then say so. Plan rev 10 (keep rev 9 as `test-plan.v9.md`), deviations in §8. End checks of `brief-impl-b4.md` (Python only if a Python file changed; clippy `--features uhd` through `rdev.py`), 20 repeats for tests that depend on threads or probes. No self-review in b6 (b5 ran it). Stop at a green point if your context gets tight and write what is left into §8.

## Report (at most 12 lines)

Per item, done or not; rows with probe outcomes; production changes, if any, and why; suites with counts; `rules.py`; anything left.

---

## brief-fix1.md

### Brief: note 47 stage 4 — fix round 1 (Sol's code review)

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `aee03ea`. The working tree holds the whole of stage 4 (parts (a) and (b)), uncommitted: do not revert it, do not touch the index. Nobody else edits the tree: you are the only agent. The `opencode run` process, `fix1.pid` and `fix1.log` you may see are your own run; do not look for other runs or stop for them.

Read `AGENTS.md` (Safety core, §7, §9), and in `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `brief-impl-b4.md` (its rules and end checks apply; probe logs under `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/probes-fix1/`), `sol-code-final.md`, `dispositions-code.md`, `test-plan.md` rev 9.

Implement the three rows of `dispositions-code.md`. Turn finding 1's sequence into a failing test first, then fix. Each new or retargeted row: probed with its named killer. Tests that depend on threads or probes: 20 runs. Plan rev 10 (keep rev 9 as `test-plan.v9.md`): the changes, and in §8 what b6 left unrecorded (its items 1–8 as its report at the end of `impl-b6.log` says) plus this round's deviations. End checks of `brief-impl-b4.md` in full: fmt, clippy for the workspace and for `--features uhd` through `rdev.py`, `cargo test --workspace` in the foreground (never in the background), Python 3.14 and the 3.9 container if a Python file changed, chmod, `rules.py --base HEAD --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/renames.txt` → 0, no `__pycache__` left. No self-review in this round.

Report (at most 12 lines): per row, done and how; tests and rows with probe outcomes; suites with counts; `rules.py`.

---

## brief-gate-fix.md

### Brief: note 47 stage 4 — gate fix round (2 mutation rows)

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2, fresh) in `~/local_work/Ez-SDRv4`, HEAD `aee03ea`; the working tree holds the whole of stage 4, uncommitted: do not revert it, do not touch the index. You are the only agent; the `opencode run` process, `gate-fix.pid` and `gate-fix.log` are your own run. Read `AGENTS.md` (§7, §9), `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/brief-impl-a.md` (its Rules apply; probe logs under `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/probes-gate-fix/`), and the gate output `/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/gate/mutation.log`.

The gate failed on two rows:
- **phase7/R07** (`plan/phase7/tools/mutations.json`): BASELINE FAILED because its `filter` names `ur_23_a_clock_ending_at_its_cut_cancels_a_held_burst_and_releases_its_reservation`, renamed in part (a) to `ur_23_a_clock_end_cancels_a_held_burst_and_releases_its_reservation` (`/home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/renames.txt` line 2). Retarget the filter; confirm the mutant is killed by it. Then grep every `mutations.json` (`plan/*/tools/mutations.json`) for a `filter` naming any old name in `renames.txt` and fix each the same way.
- **phase3/M52** (`crates/ezsdr-mock-radio/src/lib.rs` ~961, `.filter(|start| *start < cut) else { break };` → `false && …`, MR-25: a held burst starting before the cut is withdrawn): SURVIVED its filter `ch_09_a_burst_starting` after fix round 1 made held bursts open at the floor tick (`time::v_floor`). Decide whether the branch is still reachable (a held burst whose start is before a Stop's cut but not yet opened when the Stop is handled; actions are handled before `advance` in a step). If reachable: a test that fails under the mutant (prefer extending or retargeting an existing Mock test; name the killer in the row). If unreachable: say why with path:line, and either delete the code and the row or record the row as equivalent in the plan — recommend which.

Confirm each changed row by a probe in a throwaway copy outside the repository. Record both in `test-plan.md` (rev 11, keep rev 10 as `test-plan.v10.md`). End checks: `cargo fmt --all --check`; if code changed, clippy for the workspace and `cargo test` of the touched crates in the foreground; `rules.py --base HEAD --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-06-n47s4/renames.txt` → 0; chmod; no `__pycache__`. Report in at most 8 lines.

---

## self-review-checklist.md

### Self-review checklist: note 47 stage 4 (implementer's self-review, one round)

The change is the uncommitted diff in the current directory (`git diff`, HEAD `aee03ea`): part (a) B4 (one transmit clock per stream) and part (b) (transmit frequency, gain and antenna staged on an empty stream, applied at the next burst). Read-only: do not edit, do not format, do not run `mutate.py`, `remote_mutate.py` or `gate.py`. Read-only commands (`git diff`, `grep`, reading files) are fine; do not build in the tree.

Context, in `~/.cache/ezsdr-fixes/2026-11-06-n47s4/`: `test-plan.md` (the latest rev; §8 lists deviations), `brief-plan.md` (R1–R8), `rulings-plan.md`, `rulings-sol-plan.md`, `rulings-recheck.md` (the last overrides earlier ones).

Report each finding as: severity (P1 correctness or safety, P2 test or spec gap, P3 maintainability or wording), `path:line`, the claim, and for a P1 a minimal operation sequence and the wrong observable outcome. Say which findings are INFERRED. If an item has no finding, say what you checked.

1. Every planned sequence (S1–S14, F-cases, the owner-guard tests (a)–(c), M1–M4) has its test; the test observes what the plan says; its mutation row in `plan/maintenance/tools/mutations.json` breaks it.
2. Scope against the plan, missing and extra: no overlap refusal, no repeat switch, no preemption removal (stage 5); no transmit rate change; no new clock per burst.
3. Every "every / only / all" claim the change adds: the receipt answer comes only from the shared `Busy` fold on both Providers (never from hand-over state or `core.held`); every transmit configure between bursts is under the owner guard with `rate: None`; the `configured` mark is set only at arm and after a successful configure while not lost, and cleared on a failed configure and on loss. Check each against every call site.
4. Every `design/`, Vision and `KNOWN_LIMITATIONS.md` text the change makes false is amended (RM-20, RM-21, RM-25, RM-27, MR-18, MR-32–MR-35, UR-21–UR-26, EA-16, spec 18 §3's start-lead text, `tx_start_lead_ns` everywhere).
5. Maintainability (AGENTS.md §9): duplication (the busy rule written once; no second copy of the snapshot or of the mark), state (the mark's lifecycle in one place), tests (fixtures reused), leftovers (dead code, obsolete parameters, rows), mutation rows (the smallest unique anchor). Only problems this change introduces or worsens.
