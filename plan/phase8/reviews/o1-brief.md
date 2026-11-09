# Brief — O1: a spec oracle for transmit, passing on today's code (note 49 §5.4.11, §12 row O1)

Repository: /home/komatsu/local_work/Ez-SDRv4, branch `main`, base commit `25b9b15` (N0). Work in the main tree, uncommitted. Read first: AGENTS.md "Safety core", §3, §7, §9 (you are the implementer); design note `plan/phase8/49-time-and-booking-inventory.md` §5.4.2, §5.4.5, §5.4.8, §5.4.10, §5.4.11, §6, §12 (read only; it is the orchestrator's file); the accepted rules the oracle encodes: `design/07-radio-model.md` RM-13, RM-14, RM-15, RM-16, RM-37; `design/18-uhd-radio.md` UR-21, UR-22, UR-23, UR-36, UR-38; `design/09-mock-radio.md` MR-15, MR-16, MR-17, MR-25, MR-42.

**Risk: not risky by AGENTS §9's list** (tests only, production unchanged). **But it is the safety net of D′'s stages E1–E3**, so: **first a test plan, no code**; Sol reviews the plan; then you implement. Self-review is waived (Sonnet implementer). After the code: the orchestrator's review, the full mutation run, Sol's code review.

## Goal

Add a small **spec oracle** to test support and run it against **today's code** on both Providers (MockRadio, and the UHD Module on FakeDevice). The oracle does **not** call `ezsdr_radio::staging` (`Bookings`, fronts, `halt`, `install`, `repeat_boundary`, `is_switch`, …) or any Provider code: it is written from the spec text, so that a later stage that moves the shared decision cannot make the oracle agree with a wrong answer (note 49 §5.4.11: parity alone stops being independent once both Providers share the code). Production code does not change in O1.

Input: the sequence of bookings, Stops and switches a test submits, with what the Provider answered at receipt (accepted / refused / moved — the oracle may take the receipt answers as input; it decides what the **device** should emit, not admission. Where the spec fixes admission exactly — e.g. a booking inside a footprint, RM-15 — the oracle may also assert the answer; say which in the plan). Output, per note 49 §5.4.11:
1. for each device sample in a window: which booking's sample (and which index of its waveform) should sound, or silence; a repeat at `(k − origin) mod N`, after an applied switch the new waveform's period from the boundary;
2. the Kernel burst records (identity, `requested_target`, sample counts) the Manifest's `bursts` should hold;
3. the ranges the spec leaves open, not a point: a Stop's cut is any sample in `[t, B]` the Provider's spec allows (RM-16, MR-25 for the Mock's exact rule if you choose to encode it), a switch's boundary is on its grid (RM-37, UR-38);
4. the `switches` rows' outcomes.

Observation points that exist (do not widen production for the oracle): UHD on FakeDevice — `FakeDevice::transmitted(from, n)` (channel 0, `crates/ezsdr-radio-uhd/src/device.rs:1021`), `calls()` (`:997`), the Manifest `bursts`, `switches`, `rejected`, `timing` rows and events; MockRadio — the Manifest and events, and whatever sample-level view the Mock or its SimulationChannel already exposes (find it; if the Mock has no sample-level view, say so and compare records and events only for the Mock). Multi-channel: `transmitted` is channel 0 only; do not widen it in O1 — note it as a limit.

Waveforms in the oracle's tests carry a distinct value per sample (a ramp or the sample index encoded in I), so a waveform shifted by one sample, a wrong repeat phase or a switch at the wrong grid point is visible.

## Sequences the plan must cover (each with its expected result from the spec; both Providers unless marked)

1. A finite burst on time; a second finite burst after it at its front (RM-15: short ≤ 32 samples → padded end + 10; longer → end + 2) — samples, gap, records.
2. A chain: a head longer than 32 samples and a booking joined at its exact end, received in time (RM-15 join) — one continuous waveform, two Kernel burst records.
3. A repeat started on time, observed across at least two wraps (phase `(k − origin) mod N`), then a `Stop` at instant t — silence from somewhere in `[t, B]` (UHD) / the MR-25 cut (Mock); records.
4. A repeat switch received before its boundary (applied at `b` on the grid; new period from `b`); a switch received so late the UHD owner's cursor has passed `b` (UHD: moved to the next grid point with `TIME_ERROR { send_asap }`); a Stop before the boundary (switch cancelled, no new-waveform sample).
5. A late `SendAsapAndFlag` booking (moved past every accepted region, RM-14) and an untimed booking (first free sample, RM-15).
6. A Stop that cancels held bookings not yet handed (RM-16) and a later booking after the Stop region.
7. UHD only: an abandon (UR-22) through a FakeDevice fault the fake already scripts (underflow or a failed send): a repeat ends at its cursor, the pending switch is cancelled (RM-37), a finite booking keeps its region (KL-39). Use the existing FakeDevice fault configuration; add none.
8. **Known divergence** (issue #84): a repeat dropped at dispatch with a pending switch — the spec (RM-37, UR-38) says the switch is cancelled; today's UHD code settles it `refused_past_boundary` (`ur_21_s4_a_switch_whose_repeat_the_host_dispatched_late_is_refused_without_a_cursor`, `tx.rs`). The oracle encodes the spec's answer; the test for this case is marked as a known divergence naming #84 (propose the form in the plan: `#[ignore = "…#84"]` or an explicit expected-divergence assertion that fails when the code is fixed), so that E2a removes the mark. Any **other** case where today's code disagrees with the spec is not marked silently: stop, and report it (the orchestrator files it).

Keep the oracle small: the spec rules it needs, not a second Provider (note 49 estimates a few hundred lines, INFERRED). If a sequence needs a rule the oracle cannot state from the spec text without re-deriving the implementation (e.g. the UHD's exact send framing), the oracle asserts the spec's range, not the implementation's point.

## Test plan (deliver this first, then STOP and report)

Write `/home/komatsu/.cache/ezsdr-fixes/2026-11-09-o1/test-plan.md`:
- where the oracle lives (a test-support module both Providers' tests can use without a dependency cycle — e.g. `crates/ezsdr-acceptance` beside `tests/parity.rs`, or a `tests/common` module; justify) and its interface (input and output types, in a few lines);
- for each sequence 1–8: the test (name, file), the rig it reuses (existing parity/FakeDevice manual-clock fixtures; no wall-clock waits, AGENTS: a test's verdict never depends on when the test thread runs), what it compares (oracle output vs which observation), and the wrong behaviour that would fail it;
- which spec sentence each oracle rule encodes (rule id), so a reviewer can check the oracle against the text;
- **mutation rows** that show the oracle's power: mutate production behaviour the oracle should catch and existing tests may not — at least a one-sample shift of the waveform position in the UHD owner's send, a wrong repeat phase after a wrap, a switch installed one grid point off, and the Mock's equivalent where the Mock exposes samples; each row names the oracle test in `filter`. Rust token anchors (AGENTS §7);
- the observation limits (channel 0 only, Mock without samples if so) and what they leave unchecked.

Do not write code before the orchestrator says the plan is accepted.

## Rules (binding)

- Do not commit, push or `git add`. Do not run `mutate.py` or apply a mutant to the tree; mutation rows run only through `remote_mutate.py`, which the orchestrator runs.
- Builds and tests: `python3 plan/maintenance/tools/remote/rdev.py -- cargo …` in the foreground (long timeouts). Test temp files under /tmp. `PYTHONDONTWRITEBYTECODE=1` for Python.
- Before reporting code: `cargo fmt --all`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 25b9b15` = 0.
- Production code under `crates/*/src` does not change, except test-support modules gated by `#[cfg(test)]` or a test-support feature that already exists; if the oracle needs a production accessor, stop and ask.
- Do not edit `plan/phase8/49-*.md`, `handoff.md`, `AGENTS.md`, `KNOWN_LIMITATIONS.md` or `design/`.
- No tests that depend on wall-clock timing. No new FakeDevice faults. Do not delete existing tests.
- Kill processes only by PID.

## Report

End your final message with: the plan's path (or, after implementation, the files changed), the sequences covered and any not covered with the reason, every spec/code disagreement found (first), the mutation rows added, and the commands run with pass/fail counts.
