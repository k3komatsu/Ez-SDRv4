# Design note 48 S3 — briefs (2026-11-06)

Order: brief-impl (step 1, test plan) → brief-plan2 (plan round 2) → brief-impl2 (implementation) → brief-fix1..3. Rulings that bind them are in 48s3-dispositions.md.

---

## brief-impl.md

## Brief: design note 48 S3 — #78's remaining cause: an open device burst's first send carries at least 32 samples

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `e17b6ca`, tree clean. Nobody else edits the tree. **Risky** (it changes the transmit lifecycle in the UHD Module and the boundary between FakeDevice/MockRadio and the X3x0): you write a **test plan first** (step 1 below) and stop; Sol reviews it; you implement only after the orchestrator's go.

Read: `AGENTS.md` (Safety core, §7, §9 including Maintainability and Counterexamples); `handoff.md` §2 item (7) ③′; `plan/phase8/bench-results.md` §「2026-11-05（続き）」 and §「2026-11-06」 (the evidence below); `plan/phase8/48-simplification-inventory.md` §4 (S1); `design/18-uhd-radio.md` UR-21, UR-22, UR-23, UR-33; `design/09-mock-radio.md` MR-17, MR-25; `crates/ezsdr-radio-uhd/src/provider/tx.rs` (`send`, `refuse_first_send`, the hold logic), `src/device.rs` (FakeDevice's DUC model, `MIN_FIRST_SEND`, `Radio::packet`), `crates/ezsdr-mock-radio/src/lib.rs` (`DEVICE_MIN_FIRST_SEND`, `short_burst_floor`, MR-25's hand-over), and the S1 tests (`git log --oneline -12`, commit `c58cb32`).

### The hardware rule (VERIFIED, bench 2026-11-06, UHD-only C++ `plan/phase8/tools/uhd-repro/tx_repro.cpp` case `head`, both X300s, UBX and OBX)

- A device burst whose first send (timed SOB, no EOB) carries **28 samples or fewer** per channel, followed by its next send 0.5–2 ms later (timed or untimed), **underflows every time** at interpolation 80 or more (0.5, 1, 1.25, 2, 2.5 MS/s): one `Underflow` a few samples after the start, then every later packet `TimeError` (timed) or one `TimeError` (untimed). **30 samples or more never did** (6–10 trials per size and rate). At interpolation 50 or less (4, 5, 8, 10 MS/s) even 2 samples were fine.
- With the next send immediately after (no gap), 28 samples sometimes passed (3 of 5). S1's "at least 4 samples" came from shapes whose closing send followed at once (2026-11-05); it does not hold when the rest comes later.
- A device burst closed by its first send (one SOB+EOB send) of 4 samples or more is fine (2026-11-05, `len` case: 4–512 timed at 1 and 2 MS/s); 2 and 3 samples timed are dropped at 1–2 MS/s (hence S1's pad to four, which stays).
- The Module hit it because `send` hands a device burst's first send with only what fits the in-flight window (`tx.rs` `send`: "the burst's first timed buffer, which takes what fits"): the bench's `ProbeDevice` log showed first sends of 6–28 samples in the failing trials, 145–156 in the passing ones, all handed 7–9.9 ms ahead (the host was not late). Since no rate dependence is needed for safety (32 samples is 160 ns at 200 MS/s), the rule is rate-independent.

### The change (S3)

1. **`MIN_OPEN_FIRST_SEND` = 32** (name it as you see fit; one constant shared by the Module and FakeDevice, `MIN_FIRST_SEND` = 4 stays for the closed pad): a first send that leaves its device burst open (no EOB) carries at least 32 samples per channel. Consequences, each to be decided and tested in your plan:
   - the first send of a burst is not handed until `min(32, what remains of the burst)` fits the window (today `min(4, …)`);
   - a burst (or what remains of it up to a cut) of at most 32 samples is sent whole in one SOB+EOB send (padded to 4 when 1–3), never with its last sample held back; a burst of more than 32 keeps the held-tail logic, but its first send is at least 32 samples;
   - a repeat's first send tiles its waveform to at least 32 payload samples;
   - a burst booked at the exclusive end of such a closed short burst (≤ 32) is not a continuation: it is decided by its late policy against that burst's safe front (closing send's exclusive end, padding included, plus one sample — UR-22). Today this applies only below 4;
   - a clock cut: keep refusing one that would leave a first send under 4 (`ClockEnded`); a cut leaving 4–31 samples closes the device burst with that send;
   - a continuation of an already open device burst has no minimum (unchanged).
2. **FakeDevice models the hardware rule** (the owner asked on 2026-11-05 for #78 to be reproduced in FakeDevice before it is fixed): at an interpolation of 80 or more, a first packet with SOB, without EOB, of fewer than 30 samples per channel, followed by a later packet of the same burst, gives an `Underflow` and the rest of the device burst is lost with `TimeError` reports (timed packets) — say exactly what the model emits and why it is the observed shape. A failing test on the current code first (the bench's shape: a 10 000-sample burst whose first send the window cuts to under 30 samples), passing after the change.
3. **MockRadio parity:** MR-25's hand-over threshold becomes `min(len, 32)` (repeat: 32); MR-17's frontier for a short finite burst becomes the closed device end plus one sample for every finite burst of at most 32 samples (padded end for 1–3). Name every parity test touched. Note #79 (the Mock decides at receipt, the Module at dispatch, for a booking within the short burst's front): widening the short-burst range from 4 to 32 widens #79's window; your plan says which sequence now diverges and whether an existing parity test sees it. #79's fix stays in note 47 stage 5 unless the plan shows the divergence becomes an ordinary case — then say so and stop for the orchestrator.
4. **Specs:** design/18 UR-22 (and UR-23's "a first send of at most four samples is not held back"), design/09 MR-17 and MR-25, with their Changes rows and `*Test:*` lines; the comment at `device.rs:636`. No KNOWN_LIMITATIONS entry (#78 is an issue).

### Sequences that matter (each needs a test; expected result)

| # | Sequence | Expected |
|---|---|---|
| Q1 | 1 MS/s, window lets 28 samples of a 10 000-sample burst fit at its first chance | Module hands nothing until 32 fit; FakeDevice: no Underflow, one BurstAck; before the change the same test fails with FakeDevice's Underflow |
| Q2 | finite bursts of 1, 2, 3 samples | one SOB+EOB send padded to 4 (unchanged) |
| Q3 | finite bursts of 4, 31, 32 samples, nothing booked after | one SOB+EOB send of that length, no held tail |
| Q4 | finite burst of 33 samples | first send ≥ 32, held tail as today |
| Q5 | burst of 10 samples then a burst booked at its end (contiguous), and one at end + 1 | contiguous: late by its policy (TIME_ERROR, `drop_and_flag` drops); end + 1: on time — Module and Mock agree |
| Q6 | repeat of a 5-sample waveform | first send tiles to ≥ 32 |
| Q7 | clock cut leaving 3, and 10 samples of a held burst | 3: refused `ClockEnded` (unchanged); 10: one closed send |
| Q8 | a burst continuing an open device burst at its next sample, first buffer of 1 sample | no minimum (unchanged) |
| Q9 | FakeDevice at 10 MS/s with a 2-sample open first send | no fault (interpolation 20) |
| Q10 | MockRadio: hand-over threshold and front for Q1, Q3, Q5 | the Module's outcome |

Add any sequence your reading of `send` shows (the window refusal path, `step`'s held-tail deadline, multi-channel streams).

### Step 1 — the test plan (then stop)

Write `~/.cache/ezsdr-fixes/2026-11-06-n48s3/test-plan.md`: per sequence, the test name and level (pure, Module unit, FakeDevice integration, Mock, parity), what it asserts, how it observes it, the wrong behaviour that fails it, and the mutation rows (Rust token anchors, each matching once, each naming its killer). List the existing tests whose expectations change (with `path:line`) and why. List the spec texts to amend. Then end your turn with a short report; do not touch the tree in step 1.

### Rules (read twice)

- Production code changes only as this brief says; add no other mechanism; report a gap instead.
- Mutation rows: text edits only in `plan/maintenance/tools/mutations.json`; never run `mutate.py`, `remote_mutate.py` or `gate.py`; confirm rows in a throwaway copy outside the repository (`rsync -a --no-times --exclude .cargo --exclude target`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, `find <copy> -type f -exec touch {} +`), logs in `~/.cache/ezsdr-fixes/2026-11-06-n48s3/probes/`; a compile failure is not a kill.
- `--features uhd` builds and clippy only through `python3 plan/maintenance/tools/remote/rdev.py -- <cmd>`; hardware tests compile-checked only.
- Single test runs with `timeout 300`; long commands in the foreground with a long timeout. **Never leave a command running in the background when you end your turn.**
- Earlier mistakes, forbidden: reporting a planned item as a gap; a kept test whose observation a change removes; misreading UR-22's padding (a held-tail close is padded to two, so the first safe start after it is end + 2).
- No git state changes, no version bumps, no `SCHEMA_CHANGELOG`; test temp files on `/tmp`.
- End checks (implementation step): `cargo fmt --all`; clippy `-D warnings` workspace and `-p ezsdr-radio-uhd --features uhd --all-targets` (rdev); workspace tests; chmod go+r of changed files that exist; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` → 0.

---

## brief-plan2.md

## Brief: design note 48 S3 — amend the test plan (round 2)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `f1d502e`, tree clean. Same step as `~/.cache/ezsdr-fixes/2026-11-06-n48s3/brief-impl.md` (read it again, including its Rules); your plan is `test-plan.md` beside it.

Read, in order: `rulings-plan.md` (the owner's and orchestrator's rulings on your D1–D4), `sol-plan-final.md` (Sol's review), `rulings-sol-plan.md` (the rulings on Sol's six findings). They bind and override the brief where they differ.

Task: amend `test-plan.md` in place so that every ruling is carried: D1 as ruled (no receipt-time mechanism; #79 widened; a KNOWN_LIMITATIONS entry in the spec list), D2–D4, and P1–P6. Remove §1's open questions that are now decided (replace each with one line: the ruling and where the plan carries it). Keep the format: per sequence the test name, level, what it asserts, how it observes it, the wrong behaviour that fails it; mutation rows with one named killer each. You may probe in a throwaway copy outside the repository as the brief's Rules say (P1's frontier and the B→A receipt order are worth a probe). Do not touch the tree. Then end your turn with a short report (what changed, anything you could not carry and why). Never leave a command running in the background.

---

## brief-impl2.md

## Brief: design note 48 S3 — implement (step 2)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `f1d502e`, tree clean. Same step as `~/.cache/ezsdr-fixes/2026-11-06-n48s3/brief-impl.md`: its Rules and End checks bind (read them twice). Read, in order, in that directory: `rulings-plan.md`, `sol-plan-final.md`, `rulings-sol-plan.md`, `rulings-o1.md`, `rulings-recheck.md` (the last overrides earlier ones where they differ), and your `test-plan.md`.

The orchestrator's go is given. Do, in order:
1. Update `test-plan.md` to carry `rulings-o1.md` and `rulings-recheck.md` (short; the plan stays the step's record).
2. Implement: production changes in `provider/tx.rs`, `src/device.rs` (FakeDevice), `crates/ezsdr-mock-radio/src/lib.rs`; tests as planned, each new behaviour's test written red first where practical (Q1, P1, N1); spec text in `design/18-uhd-radio.md` (UR-21/22/23/33), `design/09-mock-radio.md` (MR-17, MR-25) with their Changes rows and `*Test:*` lines; `KNOWN_LIMITATIONS.md` KL-32 (D1's hardware limit only); mutation rows in `plan/maintenance/tools/mutations.json` (text edits only; each `old` matches once; one named killer; confirm each row kills in a throwaway copy as the Rules say, logs in `probes/`). Remove leftovers the change makes obsolete (AGENTS §9 Maintainability 4). Extend the plan for every state or failure path you add.
3. End checks (brief-impl.md), including rdev clippy for `--features uhd --all-targets` and the hardware tests' compile check.
4. Self-review, one round, in the foreground with `timeout 2400`: `opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max --agent plan --auto "<checklist>"` (if the free tier refuses, `opencode-go/deepseek-v4.1-flash#max`). Checklist: (1) every planned sequence has its test, the test observes what the plan says, and its mutation row breaks it; (2) scope against the plan, missing and extra (no receipt-time Mock rule, no burst merging, no known-divergence tests); (3) every "every / only / all" claim — every device burst's open first send ≥ 32 per channel (window, clock cut, held cut, repeat, reopen after abandon), the 32 boundary written once in the Module and once in the Mock, every Mock front site (`tx_handed_front`, both Stop sites) using the closed short floor — checked against all call sites; (4) every `design/` text the change makes false amended (UR-21/22/23/33, MR-17/25, the device.rs comment at the old MIN_FIRST_SEND), KL-32 present; (5) maintainability (AGENTS §9). Fix or reject (with reason) every finding; a second round only after a P1.
5. Report: what changed (files), the test and row lists with each row's probe result, the self-review findings and dispositions, anything not done and why. Never leave a command running in the background.

---

## brief-fix1.md

## Brief: design note 48 S3 — fix round 1

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`; the tree holds your uncommitted S3 diff on HEAD `f1d502e`. Same step: `~/.cache/ezsdr-fixes/2026-11-06-n48s3/brief-impl.md` (Rules and End checks bind) and `brief-impl2.md`. Read Sol's code review `sol-code-final.md` in that directory. All six findings are adopted, plus one of the orchestrator's:

1. **Sol 1 (major): a second Stop loses the Mock's closed short front.** Write the failing Mock test first (A of 10 inside the window, Stop, Stop, B at s+11 on time; and B at s+10 dropped), and the same shape as a parity test if the Module agrees (check it; report if not). Fix: a Stop that hands nothing new keeps the front it has; it never rebuilds +2 from a retained payload cut of a closed short burst. Mutation row with that test as killer.
2. **Sol 2 (major): FakeDevice's report mode must follow the incoming packet's own timing,** not the DUC's residual time. Failing test first: interpolation ≥ 80, timed open SOB of 2 samples, a later untimed middle packet, an untimed EOB → exactly [Underflow per channel, one TimeError per channel], no BurstAck, continuation samples absent (bench: `plan/phase8/bench-results.md` §2026-11-06 table: untimed continuations give one TimeError). Keep the all-timed shape's test. Row.
3. **Sol 3 (minor): specs.** UR-23 (`design/18-uhd-radio.md:255-256`) states the short-predecessor exception where it says an adjacent burst continues and shares the ACK; MR-17 (`design/09-mock-radio.md:143`) states that a closed short burst's frontier is its padded device end plus one and replaces the +2; move bench history and the unmeasured-boundary explanation out of rule text (`design/18-uhd-radio.md:332`) into the Changes row (AGENTS §2 format).
4. **Sol 4 (minor): KL-36 workaround:** start the next burst at the padded device end plus one (one sample after the end for 4–32 samples; after the padding to four for 1–3).
5. **Sol 5 (minor):** add the two missing renames to `plan/phase8/reviews/48s3-renames.txt`.
6. **Sol 6 and orchestrator (the owner's 2026-11-05 request that #78 be reproduced in FakeDevice before the fix): a deterministic Module + FakeDevice regression** in the bench's shape (a 10 000-sample burst at 1 MS/s whose window first allows 28 samples, manual clock), run to the burst's end, asserting no Underflow and no TimeError, one BurstAck, and the whole payload transmitted. Show in a throwaway copy that with HEAD's `provider/tx.rs` (and the new `device.rs`) it fails **through FakeDevice's Underflow/TimeError**, not only through a send-count assertion; save that log in `probes/`. Extending `ur_23_a_first_send_waits_for_32_window_samples` this way is fine. It is the killer of a row that restores the old window minimum (e.g. `wanted.min(min_open)` → `wanted.min(4)`).

Then: the end checks (brief-impl.md), the mutation rows this round added or whose code or killer changed (probe each in the throwaway copy; logs in `probes/`), the new tests 20 times where their verdict can depend on timing. No self-review round (waived for this fix round; the re-check reviewer follows). Report: each item done (files, tests, rows, probe results), anything not done and why. Never leave a command running in the background.

---

## brief-fix2.md

## Brief: design note 48 S3 — fix round 2

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`; the tree holds your uncommitted S3 diff (fix round 1 included) on HEAD `f1d502e`. Same step: `brief-impl.md` (Rules and End checks bind), `brief-impl2.md`, `brief-fix1.md` in `~/.cache/ezsdr-fixes/2026-11-06-n48s3/`.

The re-check reviewer closed Sol 1, Sol 2 and the #78 regression, and found one new major (adopted):

**R1 (major): the Mock gives the closed-short front to a short burst that continues a longer device burst.** `short_burst_floor` is applied to every finite burst of ≤ 32 samples (`crates/ezsdr-mock-radio/src/lib.rs` `tx_handed_front` ~1373-1376 and the end-of-play front ~1712, ~1772, and the Stop path), with no check that the burst starts its own device burst. MR-17 says "that starts a device burst", and the Module continues such a burst (`tx.rs` `device_sob = first && !self.chain.continues_at(start)`, ~530): it keeps the held tail and closes two samples later, front end + 2.
- Sequence (parity rig, 1 MS/s, s = 100 ms): at 50 ms book L of 40 samples at s and A of 10 at s+40; at 91 ms (inside the window) book B at s+51 (A's end + 1), drop_and_flag. Module: sends 40 open, 9, then 2 with EOB; B dropped late (`late_by_ns=1000`). Mock (with S3): B on time. HEAD's Mock dropped B too, so this is an S3 regression. Same cause on the Stop path with L and A held (Mock front s+51, Module s+52).
- Fix: the short floor applies only to a burst that opens its own device burst: no accepted finite predecessor in the same stream and clock domain ending exactly at its start whose own device burst stays open (a predecessor that is itself a closed short burst does not count: a booking at its end is decided late, D1). Use the Mock's existing records of held/open bursts; add no new lifecycle flag if a derivation from them suffices (AGENTS §9 Maintainability 2), and keep the 32 boundary in one place.
- Failing parity test first: the sequence above at B = A's end + 1 (both drop) and A's end + 2 (both play), and the Stop variant (L and A held, Stop, then B at end + 1 and end + 2), observing both Providers. A mutation row on the new condition, killed by it. Amend MR-17's text only if it no longer says exactly this.

Minor, adopted: add one FakeDevice test for an untimed SOB with an untimed middle packet and an untimed EOB at interpolation ≥ 80, delayed (Underflow, one TimeError, then silent; continuation samples absent).

Then: the end checks (brief-impl.md); the rows this round added or whose code or killer changed, probed in a throwaway copy (logs in `probes/`); new tests on a real clock 20 times. No self-review round. Report each item (files, tests, rows, probe results) and anything not done. Never leave a command running in the background.

---

## brief-fix3.md

## Brief: design note 48 S3 — fix round 3 (the Mock returns to note 40 D5)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`; the tree holds your uncommitted S3 diff (fix rounds 1 and 2 included) on HEAD `f1d502e`. Same step: `brief-impl.md` (Rules and End checks bind), `brief-impl2.md`, `brief-fix1.md`, `brief-fix2.md` in `~/.cache/ezsdr-fixes/2026-11-06-n48s3/`. Read `rulings-fix3.md` there and `plan/phase8/40-preemption-bound.md` §7 (decision D5): they bind. `git show HEAD:<path>` gives the Mock's code and specs before S3.

Do what `rulings-fix3.md` says: restore the Mock's frontier to HEAD's D5 form (keeping S1's floor below four), remove the framing inference (`continues` and the rest), keep MR-25's 32 hand-over threshold only if needed, retarget the parity and Mock tests to D5's relation (the receipt orders and Stop variants it names), remove the obsolete tests and rows (list each with its fate), restore MR-17/MR-25 text, update the test plan's §1 with one line. The UHD Module and FakeDevice are not changed in this round.

Then: the end checks (brief-impl.md); probe in a throwaway copy every row whose code or killer this round changed or that it added (logs in `probes/`); run new real-clock tests 20 times. No self-review round. Report: the Mock diff against HEAD that remains (it should be small), each test and row with its fate, whether the 32 hand-over threshold changes any verdict, anything not done. Never leave a command running in the background.
