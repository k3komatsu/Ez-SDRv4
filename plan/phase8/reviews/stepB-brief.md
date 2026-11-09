# Brief — step B: the front after a short closed device burst is its transmitted end + 10 (#81, #82)

Repository: /home/komatsu/local_work/Ez-SDRv4, branch `main` (the commit that lands step A; the orchestrator gives it when launching). Read first: AGENTS.md "Safety core", §3, §7, §9 (you are the implementer); `plan/phase8/bench-results.md` section "2026-11-09"; issues #81 (with its 2026-11-09 comment) and #82 (`gh issue view 81 --comments`, `gh issue view 82`); design note `plan/phase8/49-time-and-booking-inventory.md` §3 rows R3, R6, R8, U2 (read only; it is the orchestrator's file).

**Risk: risky** (AGENTS §9 step 1): it changes a booking rule both Providers call (`ezsdr_radio::staging`), the boundary between FakeDevice and the X300 it stands for, and a user-visible limitation (KL-36). So: **first a test plan, no code** (§9 step 2); Sol reviews it; then you implement.

## The decision (owner, 2026-11-09)

After a **closed device burst of at most 32 transmitted samples** (a standalone short head; 1–3 samples are padded to 4), the next device burst may start no earlier than that burst's **exclusive transmitted end + 10 samples**. Today the record's front is end + 1 (`crates/ezsdr-radio/src/staging.rs` `front()`, about line 586: `start + max(len, 4) + 1`). Everything else about fronts stays: a head longer than 32 samples and a joined piece end in a held tail with front end + 2; a two-zero close of an open device burst (Stop, abandon) has front cursor + 3; a repeat never ends. One value (10), not a table: the owner rejected per-length values.

## Hardware evidence (do not re-derive; cite it)

- VERIFIED (2026-11-09, #2 UBX slot A, UHD only, `plan/phase8/tools/uhd-repro/tx_repro.cpp` case `sweep`, 2–4 trials per cell, 1 and 2 MS/s same pattern): after a closed burst A of transmitted length L, a 10 000-sample timed burst B starting `gap` samples after A's exclusive end plays only when: L = 4–8: gap ≥ 10 (gap 7 fails); L = 10–14: gap ≥ 7 (gap 5 fails); L = 16: gap ≥ 5 (gap 3 fails); L = 16–30: gap ≥ 5 all play; L = 31, 32: gap 1 plays. Failing trials draw Underflow and/or ~100–340 TimeError (late packets) on B.
- VERIFIED (2026-11-09 morning): a *short* B (100 samples, one closed send) plays at gap 1 after any short A — the constraint is on a following long burst.
- VERIFIED: through the Module, `hw_n47s5_front_after_a_short_burst` fails 8–9 of 12 rows on both boards with the record's end + 1 front; `hw_n47s5_held_tail_and_stop` row R5-m5 (a 1–3-sample head the Stop cuts at c − 1, padded to end at c + 3, a booking at c + 3) fails on both boards (#82). The Module's send shape is as designed (ProbeDevice trace in #81).
- INFERRED: the same holds on #1 OBX and at other rates (only #2, 1 and 2 MS/s measured); L between 17 and 30 at gaps 1–4 is not measured (the rule covers it conservatively); the mechanism inside the device is unknown.

## Operation sequences and expected results (both Providers unless marked; principle 4: overlap with an accepted booking's footprint is a refusal at receipt, RM-15 / §11.2 (a) of note 49)

1. Head of n ∈ {1, 2, 3} samples at s (padded, transmitted end s + 4). A timed booking at s + 4 + k: k ∈ {0 … 9} → refused at receipt (`UR-21: the burst overlaps an accepted booking` / `MR-16: …`), regardless of late policy; k = 10 (s + 14) → admitted and plays on time.
2. Head of n ∈ {4, 10, 31, 32} at s (end s + n). Booking at s + n + 9 → refused; at s + n + 10 → admitted.
3. Head of n = 33 at s (open device burst, held tail): unchanged — e + 1 refused, e + 2 admitted, a join at e before J(e) is one device burst.
4. Untimed booking right after a short head: placed at the head's transmitted end + 10 (first free sample), no TIME_ERROR.
5. #82: a Stop at instant t whose cursor c cuts nothing of a 1–3-sample head starting at c − 1 (transmitted end c + 3): a booking at c + 3 … c + 12 → refused; c + 13 → admitted and plays. Find the code that caps a chain's front at the Stop bound (`Bookings::halt`, `c.front = c.front.min(stop)`, about `staging.rs:346-368`) and make the short head keep its own front when the Stop does not cut it; a chain the Stop does cut keeps today's two-zero-close rule (cursor + 3; VERIFIED 2026-11-09 (d) raw c + 3 / c + 4).
6. A Stop cutting a long repeat at cursor c (two-zero close): a booking at c + 3 → admitted (unchanged).
7. The UHD transmit owner's actual front after closing a short head (`crates/ezsdr-radio-uhd/src/provider/tx.rs` `advance_front(… open.next + pad + 1)`, about line 800) equals the record's front (transmitted end + 10); the stat `tx_close_past_record_front` stays 0 in the on-time and parity tests.
8. FakeDevice (`crates/ezsdr-radio-uhd/src/device.rs`, the DUC `packet` model, about lines 640–760): a timed SOB of a new device burst that starts less than 10 samples after the exclusive end of a closed device burst of at most 32 transmitted samples is refused the way the hardware does (TimeError on its timed packets and/or Underflow), and at end + 10 it plays. The fake must not be looser than the hardware (AGENTS §3: Mock enforces the envelope); it may be stricter (it applies 10 for every L ≤ 32 although L = 16–30 need only 5). A short closed B after a short A is allowed by the hardware at gap 1; decide in the plan whether the fake models that exception or is stricter, and say why.
9. MockRadio uses the record (it calls `ezsdr_radio::staging`), so its receipt answers change with the record; the parity tests (`crates/ezsdr-acceptance/tests/parity.rs`, `s15_*`) must show both Providers give the same answer for sequences 1, 2, 4, 5.
10. The hardware tests that compute the front from the record (`iii::head_front`, `n47s5::front_trial`, the R5-m5 row of `hw_n47s5_held_tail_and_stop`) move with the rule; update their comments and expected values so the next bench checks end + 10 (and the R5-m5 row books at the head's new front, c + 13).

## Text to change in the same step

`design/07-radio-model.md` RM-15 (the front values; RM-16 if it states the Stop front of a short head), `design/18-uhd-radio.md` UR-22/UR-23 where they state the front after a short closed burst, `design/09-mock-radio.md` if it states the value, `KNOWN_LIMITATIONS.md` KL-36 (the "When" and "Workaround" values: end + 10 for 1–32 samples; also fix its title, which says "decided late" while the body says "refused at receipt" — make the title say refused). Keep each rule's `*Test:*` line naming the tests that pin it. `rules.py` must pass.

## Test plan (deliver this first, then STOP and report)

Write `/home/komatsu/.cache/ezsdr-fixes/2026-11-09-stepB/test-plan.md`: for each sequence 1–10, the test (existing to change, or new: name, file), what it asserts, how the assertion observes the behaviour (receipt answer, Manifest event, FakeDevice report, stat), and the wrong behaviour that would fail it. List the mutation rows you will add or re-anchor (Rust token anchors; `plan/maintenance/tools/mutations.json`; each row names its killing test in `filter`), and every existing test and row the rule change breaks (grep the old values `checked_add(1)`, `+ 5`, `end + 1`, `start + 5`, `head_front`, `+ 3` near Stop code) with what you will do with each. Do not write code before the orchestrator says the plan is accepted.

## Rules (binding)

- Do not commit, push or `git add`. Do not run `mutate.py` or apply a mutant to the tree; mutation rows run only through `remote_mutate.py`, which the orchestrator runs unless the brief for the implementation phase says otherwise.
- Builds and tests remotely: `python3 plan/maintenance/tools/remote/rdev.py -- <cargo …>`, in the foreground until it ends (long timeout). No cargo locally. Temp files under /tmp.
- Before reporting code: `cargo fmt --all`, then `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base <the base commit>` = 0.
- Do not edit `plan/phase8/49-*.md`, `handoff.md` or `AGENTS.md` (the orchestrator's files).
- Do not delete a test of a reachable sequence without a replacement; grep rows whose `new` names a removed symbol and rows whose `filter` names a removed test.
- Do not add tests that depend on wall-clock timing.

## Sub-steps (owner, 2026-11-09; AGENTS §9 step 5, the sub-step rule of 2026-11-07)

The step is implemented in two sub-steps on a local work branch `work/stepB`. Each sub-step leaves the tree green, gets one cheap read-only review of its own diff (a fresh reviewer), and the orchestrator commits it locally after the findings are fixed or rejected. Sol reviews the whole step once at the end.

- **B1 — the record's rule (receipt answers):** `ezsdr_radio::staging` `front()` → transmitted end + 10 for a closed device burst of ≤ 32 samples; `Bookings::halt` keeps a short head's own front when the Stop does not cut it (#82); MockRadio follows the record; the staging unit tests and the parity tests (`s15_*`) for sequences 1, 2, 4, 5, 6; RM-15 and KL-36 text. After B1 the record's front is wider than uhd-tx's actual close (end + 1) — the test plan must say why that is safe for one sub-step (the record is more conservative than the owner) or which test would go red.
- **B2 — the device side:** uhd-tx's actual front after closing a short head (`tx.rs` `advance_front`, sequence 7, `tx_close_past_record_front` stays 0); the FakeDevice model (sequence 8); the hardware tests' expected values and comments (sequence 10); UR-22 / UR-23 text.

The test plan is organised by sub-step: which sequences, tests and mutation rows belong to B1 and which to B2.
