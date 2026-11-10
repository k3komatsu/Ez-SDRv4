# Brief — step F: the front after a closed device burst is time-based, and transmit rates start at 1 MS/s (#81, #82)

Repository: /home/komatsu/local_work/Ez-SDRv4, branch `main` at `698df19` (the orchestrator names the work branch when launching). Read first: AGENTS.md "Safety core", §3, §7, §9 (you are the implementer); `plan/phase8/bench-results.md` sections "2026-11-09（午後）" and **"2026-11-10"** (the sweep table and the owner's decisions); `plan/phase8/reviews/stepB-brief.md` and `stepB-dispositions.md` (the step that set today's + 10; this step replaces its value, not its structure); issues #81, #82 (`gh issue view 81 --comments`, `gh issue view 82`).

**Risk: risky** (AGENTS §9 step 1): it changes a booking rule both Providers call (`ezsdr_radio::staging`), the boundary between FakeDevice and the X300 it stands for, the transmit rate grid both Providers declare, and two user-visible limitations. So: **first a test plan, no code** (§9 step 2); Sol reviews it; then you implement. The owner settled the direction with its alternatives on 2026-11-10 (below), so §9 step 0's design draft is skipped; Sol's test-plan review also attacks the rule itself (orchestrator, one line: the alternatives were already measured and decided).

## The decisions (owner, 2026-11-10)

1. **The front after a closed device burst** (the next safe start, RM-15's region front and UR-22's owner front):
   - after a **standalone closed device burst of at most 32 transmitted samples** (1–3 padded to 4): the closing send's exclusive transmitted end, padding included, **+ max(3 samples, ⌈15 µs × rate⌉ samples)**, `rate` the stream's sample rate;
   - after **any other close** (a held tail's close, a two-zero close of an open device burst at a Stop or an abandon, a repeat's close): that closing send's exclusive transmitted end **+ 3 samples** (today + 1).
   - One rule for every profile and rate. Rejected: a per-interpolation table and a guard only at interpolation ≥ 80 (owner: the case arises only for bursts of ≤ 32 samples; a simple rule beats many corner cases).
2. **Transmit sample rates are at least 1 MS/s** on the `x3xx` profile (interpolation ≤ 200, i.e. `master_clock_hz / N` with `N ≤ 200`), for both Providers. Receive rates are unchanged. `ideal` is unchanged. A user-visible Ceiling: a new KNOWN_LIMITATIONS entry.

What follows from 1 for today's values (derive and check each in the plan, do not take these on trust): today a head longer than 32 samples or a joined piece fronts at payload end + 2, because its close may add one zero (worst-case closing end = payload end + 1, + 1); under the rule that is payload end + 4. A two-zero close at cursor `c` ends at `c + 2`: today's `c + 3` becomes `c + 5` (RM-16's `stop_sample` "plus three", the abandon's "cursor plus three", `tx.rs` `close_start + 3`). The short-head front `max(len, 4) + 10` becomes `max(len, 4) + max(3, ⌈15 µs × rate⌉)`.

## Hardware evidence (cite it; do not re-derive)

- VERIFIED 2026-11-10 (both X300s, identical tables; UHD only, `plan/phase8/tools/uhd-repro/tx_repro.cpp` cases `sweep`, `gap`, `flush`; 10 trials per cell at 1, 2, 100, 200 MS/s, 3 elsewhere): the minimum gap after a short closed head is at most ~10 µs at interpolation 60–400 and ≤ 1 sample at 4–50; heads of 28–32 samples or more need gap 1 at ≤ 50; at 100 and 200 MS/s every close needs gap 2 and 3 (heads of any length up to 12 800, and the `gap` case's two-sample close of a 10 000-sample burst). The B shape matters (a 100-sample first send needs less than a 1996-sample one); the rule covers the larger.
- VERIFIED 2026-11-10: at ≤ 0.4 MS/s (interpolation ≥ 500) a closed burst of ≤ 4 samples underflows itself and a two-zero untimed flush is not acknowledged within 1 s at ≤ 0.25 MS/s — the reason for decision 2. 1 MS/s: the two-sample flush and every padded head close (10/10).
- VERIFIED 2026-11-10 through the Module (+ 10 front): (c)1 passes at 1 MS/s and fails at 2 MS/s (3 of 8 padded heads); (d) R5-m5 at c + 13 and N-H1 pass at 1 MS/s.
- INFERRED: the device mechanism (the DUC at large interpolation; compare UR-33's open-first-packet result at interpolation ≥ 80, `device.rs` `OPEN_FAULT_MIN_RATIO`); rates not swept (interpolation 5–49 other than 8, 20, 40; 51–59; 71–79; 81–99) are covered by the 15 µs margin over the 10 µs maximum.

## Operation sequences and expected results (both Providers unless marked; RM-15: overlap with an accepted footprint is a refusal at receipt)

Write `g(rate) = max(3, ⌈15 µs × rate⌉)`: 15 at 1 MS/s, 30 at 2, 43 at 2.857 142 (N = 70), 150 at 10, 1500 at 100, 3000 at 200.

1. Head of n ∈ {1, 2, 3} at s (transmitted end s + 4), rates 1, 2 and 200 MS/s: a timed booking at s + 4 + k, k < g → refused at receipt; k = g → admitted, plays on time.
2. Head of n ∈ {4, 32} at s: booking at s + n + g − 1 refused, s + n + g admitted. n = 33: an open burst with a held tail; e + 3 refused, e + 4 admitted (today e + 1 / e + 2); a join at e before J(e) is still one device burst.
3. Untimed booking right after a short head: placed at the head's transmitted end + g; after a 33-sample head at end + 4. No TIME_ERROR.
4. #82: a Stop at cursor c that does not cut a 1–3-sample head at c − 1 (transmitted end c + 3): a booking at c + 3 + g − 1 refused, c + 3 + g admitted (1 MS/s: c + 18).
5. A Stop cutting a long repeat at cursor c (two-zero close): c + 4 refused, c + 5 admitted. An abandoned repeat keeps its footprint to cursor + 5.
6. uhd-tx (`crates/ezsdr-radio-uhd/src/provider/tx.rs`, the `advance_front` sites near lines 832, 868, 944, 1044): the owner's actual front after each close equals the record's; `tx_close_past_record_front` stays 0 in the on-time and parity tests at more than one rate.
7. FakeDevice (`crates/ezsdr-radio-uhd/src/device.rs`, `arm_short_guard` and the DUC packet model): a timed SOB of a new device burst before the closed burst's exclusive end + the rule's gap is refused the way the hardware does; at the rule's gap it plays. The fake keeps its own copy of the values (the comment at `SHORT_BURST_MAX` says why). It may be stricter than the hardware (it applies the time guard at every interpolation), never looser. State in the plan whether it also models the 2–3-sample gap after a long close at interpolation 1–2, or applies + 3 everywhere, and why.
8. Transmit rate floor, both Providers, `x3xx`: a transmit stream at `master_clock_hz / N` with N > 200 (e.g. 0.5 MS/s, N = 400) is refused where a rate off the grid is refused today (RM-8 coercion → `rejected`, at `connect` or the Spec's `requires`), not coerced to 1 MS/s; N = 200 (1 MS/s) is accepted; a receive stream at 0.5 MS/s is still accepted. Find how RM-29 declares `radio.d.s<k>.sample_rate_hz` from the one `Description.rates` grid (`crates/ezsdr-radio/src/channels.rs` ~362) and how UR-13 compares the device-reported rates with the profile grid (`ezsdr_radio::device::rate_set_matches`, `provider/core.rs` ~1057): the device still reports every rate, so the comparison must not start failing; say in the plan which grid each check uses.
9. MockRadio uses the record, so its receipt answers follow; the parity tests (`crates/ezsdr-acceptance/tests/parity.rs`) show both Providers give the same answers for sequences 1–5 and 8 at two rates.
10. The rate enters the rule once per transmit stream (its rate has no update class and is fixed for a Run, RM-27): say where `Bookings` learns it (today it is built with `Bookings::default()` at `control.rs:285`, `differential.rs:585`, `ezsdr-mock-radio/src/lib.rs:234` and `:1931`) and that no caller can book with a default that silently means "no guard". One function computes `g(rate)`; name its home.
11. The hardware tests that compute the front from the record (`iii::head_front`, `n47s5::front_trial`, the R5-m5 row of `hw_n47s5_held_tail_and_stop`, N-H1 `hw_n47s5_long_b_after_a_short_head_gap_9_and_10`) take the transmit rate as a parameter with the default set 1, 2, 2.857 142 857 (N = 70, the largest measured guard), 10, 100 and 200 MS/s, and compute their expected fronts from the rule (`#[ignore]`; compile-checked only here). N-H1's name and gaps follow the rule (the gap below and at g). A test whose transmit rate is below 1 MS/s on `x3xx` moves to 1 MS/s or to `ideal`.

## User-facing promises (AGENTS §9 step 1, owner 2026-11-07)

The new refusal (decision 2) must be checked against EA-16 and the specs a user reads (`design/16-easy-api.md`, `design/07-radio-model.md`, `design/09-mock-radio.md`, `design/18-uhd-radio.md`) and the Python client's examples and tests (`python/`): list every place that states, defaults to or uses a transmit rate below 1 MS/s on `x3xx`, and what changes.

## Text to change in the same step

`design/07-radio-model.md` RM-15 (the fronts; the "next safe start" sentence), RM-16 (`stop_sample` "plus three"), RM-29 or RM-8 (the transmit grid on `x3xx`); `design/18-uhd-radio.md` UR-22, UR-23, UR-33 where they state the fronts or the fake's guard, UR-12/UR-13 if they state the grid; `design/09-mock-radio.md` where it states a front, the Stop cut or the grid (MR-25); `KNOWN_LIMITATIONS.md` KL-36 and the entry at line ~130 (the values, and "the hardware measurement is only at 1 MS/s" → the 2026-11-10 sweep), and a **new entry** for the transmit rate floor (symptom, when, workaround: transmit at 1 MS/s or more and filter on the host, evidence: bench 2026-11-10, status: Ceiling). Keep each rule's `*Test:*` line naming the tests that pin it; test names that state "+ ten" are renamed. `rules.py` must pass.

## Test plan (deliver this first, then STOP and report)

Write `/home/komatsu/.cache/ezsdr-fixes/2026-11-10-stepF/test-plan.md`: for each sequence 1–11, the test (existing to change, or new: name, file), what it asserts, how it observes the behaviour (receipt answer, Manifest event, FakeDevice report, stat, capability value), and the wrong behaviour that would fail it. List the mutation rows you will add or re-anchor (Rust token anchors; `plan/maintenance/tools/mutations.json`; each row names its killing test in `filter`), and every existing test and row the change breaks (grep `CLOSED_SHORT_GAP`, `checked_add(2)`, `saturating_add(3)`, `+ 3`, `end_plus_ten`, `plus ten`, `+ 10`, `10 * ratio`, `head_front`, `front_trial`, transmit rates below 1e6 in `x3xx` tests) with what you will do with each. Do not write code before the orchestrator says the plan is accepted.

## Rules (binding)

- Do not commit, push or `git add`. Do not run `mutate.py` or apply a mutant to the tree; mutation rows run only through `remote_mutate.py`, which the orchestrator runs.
- Builds and tests remotely: `python3 plan/maintenance/tools/remote/rdev.py -- <cargo …>`, in the foreground until it ends (long timeout). Temp files under /tmp.
- Before reporting code: `cargo fmt --all`, then `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 698df19` = 0.
- Do not edit `plan/phase8/49-*.md`, `plan/phase8/bench-results.md`, `handoff.md` or `AGENTS.md` (the orchestrator's files).
- Do not delete a test of a reachable sequence without a replacement; grep rows whose `new` names a removed symbol and rows whose `filter` names a removed test.
- No test that depends on wall-clock timing.

## Sub-steps (AGENTS §9 step 5, the sub-step rule of 2026-11-07)

On a local work branch `work/stepF`; each sub-step leaves the tree green; the orchestrator commits each locally. Sol reviews the whole step once at the end.

- **F1 — the record and the grid:** `ezsdr_radio::staging` (`front`, `stop_sample`, the abandon footprint, `g(rate)` and where `Bookings` learns the rate); the `x3xx` transmit rate floor in the shared description; MockRadio following both; staging unit tests and parity tests for sequences 1–5, 8–10; RM-8/RM-15/RM-16/RM-29, MR text, KL entries. Say in the plan why the record being wider than uhd-tx's actual close for one sub-step is safe, or which test goes red.
- **F2 — the device side:** uhd-tx's fronts (sequence 6), FakeDevice (7), the hardware tests (11), UR text.

The test plan is organised by sub-step.
