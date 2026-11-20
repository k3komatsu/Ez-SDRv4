# bt2 (C7) — dispositions and counts

C7: a Module-level bench tool for a host-fed repeat's resume after a late start or a mid-play starvation on the X310 (R = repeat_boundary(origin, N, max(busy + m, now + device lead, device front))), the Rust-only resume-margin builder, its FakeDevice rehearsals, the drop-storm cell, and `plan/phase8/bench.md`'s C7 procedure. Risk: risky (a hardware verdict). Way of working: Sonnet subagents implemented (driving Go DeepSeek in parts); Sol 6.2 reviewed the test plan and the code; Opus and GPT-6.1 Terra reviewed each sub-step and re-checked side by side. Brief: [bt2-brief.md](bt2-brief.md); test plan: [bt2-test-plan.md](bt2-test-plan.md); reviews: [bt2-reviews.md](bt2-reviews.md). Gate: `bt2-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Sol 6.2, test plan | 7 | 3 | 0 | none counted |
| Opus / Terra, plan v2 | 1 / 3 | — | — | none |
| Opus / Terra, plan v3 | 3 / 5 | — | — | none |
| Opus / Terra, B1 | 0 / 1 | 3 / 0 | 2 / 0 | none |
| Opus / Terra, B2 | 0 / 3 | 3 / 0 | 5 / 0 | none |
| Opus / Terra, B3 | 2 / 4 | 6 / 1 | 0 | none |
| Sol 6.2, code | 8 | 2 | 1 | none counted |
| Opus / Terra, Sol fix round | 0 / 4 | 4 / 2 | 3 / 0 | one `wall-clock-test-oracle` (the capture feed evicting under host load; pre-existing, note 51's design) |
| Opus / Terra, fix round 2 | 0 / 2 (downgraded to minor) | 3 / 0 | 2 / 0 | none |

Most findings are oracle arithmetic of a measurement tool (clock uncertainty, lattice ranges, coverage), not a shared design mechanism.

# bt2 — decisions (orchestrator, 2026-11-19)

Phase A test plan by Sonnet (`test-plan.md`). Open questions, decided by the orchestrator:
- Q1 hook: `UhdRadio::with_measurement_resume_margin(ns)`, a Rust-only builder like `with_rx_stall` / `with_measurement_timing` (design/18 446–448), no cargo feature, no Spec/Binding/env surface. Accepted: the Python client and Specs cannot reach it, so it is not a user-visible surface; the rehearsal runs the bench's own body.
- Q2 rates: 1 MS/s AND 10 MS/s on the bench (not 1 only). Reason: the owner sets m once for every rate; AGENTS §9 "bench values" asks the conditions to cover the dimension the value depends on, and C4 measured both. +1/3 run time accepted.
- Q3 C2: the future-stamp redesign (later chunks stamped J = 3 ms ahead) with a 10 MS/s method control; the premise is checked per trial and a failed premise is INCONCLUSIVE. To Sol for review.
- Q4 drop-storm cell (#106's per-kind bound on hardware): added.
- Q5 `busy` stays out of the `tx_resume` row (no Manifest change).
- Q6 trials 10 / 20 / 20 per cell.

## Sol 6.2 test-plan review (sol-plan.md): FAIL, 7 majors, 3 minors — dispositions (orchestrator)
- Scope (decided by the orchestrator): **C2 at 100/200 MS/s leaves bt2.** Three of Sol's majors (5, 7, part of 8) show the future-stamp redesign changes the question (packet-local drop vs whole-burst drop, pacing vs wire stamps) and that no delivery bound qualifies its premise yet. C2's high-rate cells stay out of the next bench and the late-device-burst rule at 100/200 MS/s stays INFERRED; revisit after C7 has measured the host-to-radio delay on hardware. bench.md says so in phase B. C7 is not blocked by C2.
- F1 major adopted: the fake's idle ramp (device.rs:1304) is the fake's silence baseline; a fixture distinguishes it from PN leakage; no false SilenceViolated on the good path.
- F2 major adopted: S4 (and S1b) pin the accepted first-send size and both sides of the drain boundary with an independently computed expected R (as mt3d.rs:599 does).
- F3 major adopted: the RF check catches a restart one period late: loop delay anchored to an observed control onset, the first resumed period checked on its own, contiguous capture across the whole phase interval (coverage = phase window); resolve whole-period correlation ties.
- F4 major adopted: a decided trial needs mode-specific fault evidence (starvation: RF before the stall and an attributable report); each resume paired with its accepted SOB, report evidence and RX span; retries counted apart from evidence that m caused them; `tx_gap_samples == Σ(R_plan − H)` asserted when the rows are complete.
- F5 major adopted for C7: τ is reported as a signed measurement with its endpoints and clock uncertainty, never as an upper bound; nothing qualifies a verdict on it.
- F6 major adopted: the drain estimate keeps the reports on both sides of R_act; a drain crossing R is reported as such (or unresolved), never as positive clearance; S4 is pinned against the fake's actual drain limit (the fake reports the start of a dropped buffer's drain).
- F7 major: moot with C2 out of bt2 (kept for the C2 revisit).
- F8 minor: moot for bt2 (C2), kept for the revisit; the raw-log statements in the plan are corrected or dropped.
- F9 minor adopted: flush / payload / close as distinct inputs of the hold; each verdict mutant gets a fixture reaching its branch; Rust files under tests/ use token anchors.
- F10 minor adopted: a default Run after a custom-margin Run (isolation); drop the env::var file-list assertion; S8 reads the truncated Manifest with `section_unchecked`.

## Plan v2 (test-plan-v2.md, plan-v2-changes.md) — open questions (orchestrator)
- Q7 accepted: a trial whose first send came out short is not judged and is counted (`n1_full`, `m_informative`).
- Q8 accepted: the Session-level S4 fails (never skips) if two Sessions on the manual clock do not reproduce one timeline; then only the Tx-rig pair stays.
- Q9: the PN bandwidth (±0.5 / ±5 MHz) goes to the owner with the attenuator check at the bench.
- Q10 accepted: `delta_first` is recorded only.

## Re-check of plan v2: Opus (opus-recheck-plan.md) FAIL 1 major; Terra (terra-recheck-plan.md) FAIL 3 majors — dispositions (orchestrator)
- Principle (decided by the orchestrator, simplifies Terra 1/2 and Opus 1): **the tool never attributes a retry to m.** It reports, per cell, the counts (incidents, resumes, retries, unsettled trials) and per retry the raw drain evidence (`D_before`, `D_after`, the report ticks around R, the returns of the sends) as data; the owner reads m from retry rates across the m cells, not from a per-trial cause. Remove `DROPPED_AGAIN_M` / `RETRY_UNATTRIBUTED` and the 200 µs report window; "cleared" is printed only when a report-free stretch after the last report is itself evidenced (otherwise `unresolved`), and `R − D_before` is labelled conditional, not a measured clearance (Terra 2).
- Opus 1 adopted: class `UNSETTLED(k)` (every Start after the first drop is followed by drop evidence and nothing is heard by the trial's end) — a datum, never a FAIL, placed before `NoResume`; Session B rewritten to expect it (pattern `mt3d.rs:751`); "settles" dropped from §6.
- Terra 3 / Opus 5 adopted: a resume row with no accepted Start ≥ 5 ms past R gets its own branch before any check that needs `A`, with a fixture (INCONCLUSIVE "no start after resume", unless the trial's end explains it). `NoResume` keeps meaning "started, not dropped, silent". The plan's `≤ A − lead` is the threshold; plan-v2-changes.md is not binding.
- Terra 4 adopted: C7-21 (mean subtraction) removed as equivalent, or redesigned with a non-zero-mean input that distinguishes; C7-34's fixture with `r > max(busy, x)`.
- Terra 5: moot after the principle above (counts are independent).
- Opus 2 adopted: S8 asserts RF silence over the storm on hardware (§4.3 detector) and events = resumes for the cycles the bound kept (or the per-kind omitted count).
- Opus 3 adopted: onset as a power test (mean |s|² over N/4 against the quiet floor's power statistic); silence windows wholly inside `[from+d+2N, A+d)`, with a fixture of good PN starting exactly at `A+d` that stays CLEAN.
- Opus 4 adopted: C7-02 uses `crate::profile::RESUME_MARGIN_NS` (must compile).
- Opus 6–9 notes adopted as text: Q7's latency risk restated; the cross-burst phase reference listed INFERRED, and mode (b) also prints the part's phase against the same repeat's pre-stall periods; mode (a)'s first incident resumes `expired` (bench summary says so); F7's numbers corrected (19 968–20 029 µs; 20 trials per rate).
- Opus "not checked": the plan states an expected wall time per fake rehearsal, and S8 runs without a capture.

## Plan v3 (test-plan-v3.md) — open questions (orchestrator): Q11–Q15 accepted as recommended
- Q11 no `cleared` in bt2, no poll rows added to the probe. Q12 hardware events: `⊆ rows` and `≤ resumes`, no equality until a first measurement. Q13 storm RF silence over a 600 ms window. Q14 `RETRIED` / `UNSETTLED` never fail the hardware test; counts on the PASS line. Q15 the 5 ms open threshold INFERRED; raised (never the verdict loosened) if hardware reports arrive later.

## Re-check of plan v3: Opus (opus-recheck-v3.md) FAIL 3 majors; Terra (terra-recheck-v3.md) FAIL 5 majors — dispositions (orchestrator)
- **No plan v4 (decided by the orchestrator).** Majors went 7 → 4 → 8 over three paper rounds while the hook, stall classes, RF detector, τ-as-record and storm cell stayed closed since v2; what fails is the multi-class verdict table and hand-derived fake numbers nobody has run. AGENTS §9 step 3 asks for adjudication and amendment, not a re-check PASS. Phase B goes ahead in sub-steps with Opus + Terra on each sub-step's diff; the findings below are carried in the phase-B brief.
- Sub-steps: **B1** the builder hook (S1a–e, rows); **B2** the `ProbeDevice` hold, the trial body, the one-incident rehearsals S2/S3 with the run-revealed findings as fixtures written failing-first; **B3** the verdict, S4 and S8 — after the orchestrator approves a 3×3 outcome grid.
- Opus v3 1 + Terra v3 1 adopted (B2): the starvation evidence window starts at the held call's entry (a `host_entered` stamp before the sleep), fixture with the Underflow tick between entry and return → CLEAN.
- Terra v3 2 adopted (B2), verified by the orchestrator: `ProbeDevice::host_ns()` is the probe's `Instant` (`probe_device.rs:128`) while the hold's stamps would use the inner device's `HostClock`; every value mapped through an anchor is stamped on the anchor's own axis, with a test that the conversion holds on the manual clock.
- Opus v3 2 adopted (B3): Session B pins fixed offsets on rows 1–2 only, per-row `expected_r` after; F15 and the row-count estimate corrected (≈650–700 rows); any mismatch with a hand-computed number stops and is reported, never tuned.
- Terra v3 3, 4, 5 + Opus v3 3, 4, 8 (B3): **judge only the trial's final accepted Start** (lattice, phase, W0, silence from the first drop to it); intermediate resumes are counted and dumped raw, no pairing. The verdict is specified as a grid: resume rows {0, 1, ≥2} × final accepted Start {none, heard, silent}, one outcome per cell, each with a fixture; a silent final Start counts as dropped only with a TimeError at tick ≥ that Start's time (else NoResume). The orchestrator approves the grid before B3 code.
- Opus v3 5 adopted (B3) and generalised: every INFERRED hardware premise in v3 (Underflow tick present, cross-burst phase, reports within 5 ms) whose failure would make a whole cell INCONCLUSIVE becomes a printed datum, not a gate; the starvation evidence gates on gain only.
- Opus v3 6, 7 notes: bench text names both `expired` and `late_at_device` for mode (a)'s first incident; the "≈ 5 ms per retry" text corrected.
- Terra v3 6 adopted: the C2 note does not say C7 measures the host-to-radio delay (`delta_first` is neither a delivery delay nor a bound). Its attribution: the C2 deferral was decided by the orchestrator (bench.md:1584/1626 on main say "owner 2026-11-19" — corrected in phase B).
- Terra v3 7 adopted (B3): C7-37 names a concrete behavioural mutant (a verdict branch reading a retry field) or is dropped.

## B1 (commit on work/bt2) — report notes (orchestrator)
- Plan premise F13 refuted by the run: two Sessions with the same input do not reproduce one manual-clock timeline (decision instant differs by up to ~1 ms, ~1.2 ms under full load). S1a/S1b'/S1e therefore compare each Session's own `resumes_at − from` with a 1 500 µs jitter allowance; exact R is pinned only by S1b (Tx rig). Consequence for B3: Session-level S4 (Sessions B/C exact offsets) cannot rely on cross-Session repeatability — B3 pins exact numbers on the Tx rig or within one Session.
- UR-22 got one clause (rules.py 1 500-char line limit); the builder text sits at design/18 446–448.

## B1 review: Opus (opus-b1.md) PASS, 3 minors 2 notes; Terra (terra-b1.md) FAIL, 1 major — dispositions (orchestrator)
- Terra 1 + Opus 2 adopted (major): an exact, deterministic builder-to-Core test in the crate (the builder's 200 000 ns, 1 ms and default reach `Core::resume_margin()` exactly, through `from_binding` → builder → `with_reopened` → open), and a row that clamps values below 1 ms to 1 ms, killed by it. Done first in B2.
- Opus 1 (a Manifest `timing` row `{what: "resume_margin", ns}` when the builder changed m): **owner 2026-11-19 「推奨でok」**: a `timing` row `{what: "resume_margin", ns}`, written only when the builder set a value, beside `reopened_on_unlock` (`provider/mod.rs:539`); a test that the row appears with the builder and not without it, and a row. Goes into B2.
- Opus 3 note adopted: S1b's name/doc says it locates the fake's drain limit within (23 530, 23 630], not exactly.
- Opus 4 adopted: the three `c7_rehearsal_*` Session tests get a within-Session oracle in B2 (each Session's own R against its own `expected_r` from its probe rows, on the probe's axis), replacing the cross-Session 1 500 µs allowance.

## B2 (8a980fe9) report — B3 grid approved with changes (orchestrator)
Judge only the final accepted Start; intermediate resumes counted and dumped raw; INFERRED premises print as data. "Tail" = time from the last resume row's R (or the last drop evidence) to the Run's end; 5 ms threshold INFERRED (Q15).
| resume rows | final Start: none | heard | silent |
|---|---|---|---|
| 0 | INCONCLUSIVE `no incident` | INCONCLUSIVE `no incident` | drop evidence (TimeError/Underflow at or after the held send's time) and tail ≥ 5 ms → FAIL `dropped, never resumed`; tail < 5 ms → INCONCLUSIVE `ended before a resume`; no evidence → INCONCLUSIVE |
| 1 | tail < 5 ms → INCONCLUSIVE `ended before R + 5 ms` (**not** UNSETTLED: a first incident is no retry chain — Terra v3 3); tail ≥ 5 ms → INCONCLUSIVE `no start after resume` | CLEAN when lattice, W0/phase, silence, counters and StartedEarly all pass; else the named FAIL (one fixture each) | TimeError at tick ≥ the final Start's time: tail < 5 ms → UNSETTLED(1) datum, tail ≥ 5 ms with no further row → FAIL `dropped, never resumed`; no such TimeError → FAIL NoResume |
| ≥ 2 | tail < 5 ms → UNSETTLED(k) datum (the chain did not settle before the end); tail ≥ 5 ms → INCONCLUSIVE `no start after resume` | RETRIED(k) datum when the final Start passes the same checks as CLEAN; else the named FAIL | as row 1, with k |
B2's notes for B3, adopted: the `pinned` tolerance and `MAP_TOLERANCE_TICKS` follow the measured anchor bracket per trial (hardware 0.05–0.15 ms), not a fixed 1 µs / fake-derived value; a refused booking (1.5 ms slack) is an INCONCLUSIVE with its reason, counted; a missing control close returns evidence (INCONCLUSIVE), never panics.

## B2 review: Opus (opus-b2.md) PASS, 3 minors 5 notes; Terra (terra-b2.md) FAIL, 3 majors — dispositions (orchestrator); fixed first in B3
- Terra 1 adopted (major): the R oracle derives the allowed answers by rounding the endpoints of the mapping-uncertainty interval onto the lattice (one or two points, never ±1 period both ways); the interval comes from the trial's measured anchor bracket; fixtures on both sides of a boundary within the bracket. Opus 7: a row on the `pinned` gate.
- Terra 2 adopted (major): `sent_before` selects the history by ordering/identity (the close that precedes the decision, by the Module's own row order or a send id), not by "the last close within the bracket"; an ambiguous selection is reported as such (INCONCLUSIVE in the verdict); a fixture with a later close inside the window. Opus 5: the slack also covers the Module's own clock-read error; the anchor is read three times before and three after the trial (plan), the bracket is the worst.
- Terra 3 adopted (major): the S2″ lead-bound rehearsal end to end (a short first send at m = 1 ms where `now + lead` binds), asserting the expected R and that the trial is counted not margin-informative; with a margin-bound control.
- Opus 1 adopted: S2/S3 assert exactly one `tx_resume` row and that the event names that row's R.
- Opus 2, 3, 4 adopted: one axis per field name on the fake (or the rx stamps documented and named apart); `clock_ns` and `Hold::Until` docs; C7-71's comment.
- Opus 6 adopted: S3 accepts `underflow` or `expired`; mode (a)'s first row `late_at_device` or `expired` (bench text too); `trial` checks `rows_complete()` (lost probe rows → INCONCLUSIVE).
- Opus 8 (small duplications): fix where cheap.

## B3 review (1124a724): Opus (opus-b3.md) FAIL 2 majors 6 minors; Terra (terra-b3.md) FAIL 4 majors 1 minor — dispositions (orchestrator); fix round before Sol
- Terra 1 + Opus 1 + Opus note (the R oracle on hardware: the anchor bracket 0.05–0.15 ms plus the Module's own clock-read error is comparable to the 100 µs lattice period, so the allowed set holds 2–4 points and a one-period error passes, or a correct trial goes INCONCLUSIVE/FAIL): **decided by the orchestrator: lengthen the hardware PN period so the oracle discriminates** — period ≥ 1 ms (1 000 samples at 1 MS/s, 10 000 at 10 MS/s; ≥ 4× the worst expected probe + Module mapping error), the fake rehearsals keep their exact pins; the R check's allowed set uses the worst of the six anchor reads plus the Module's own accepted read width (≤ 1 ms per `authority.rs:31-34`, or its measured width if the Module exposes it); when that uncertainty exceeds half a period the R check is printed as `R unresolved` (data, with R_act and the interval) and does not decide the class; `history` pairs by order with a wide slack. The RF windows, W0 and the silence region follow the new period; re-derive the SNR figure for the longer PN.
- Terra 2 + Opus 5 adopted: an `expired` first incident is late-start evidence by itself (the Module's row plus the host stamps showing the held send returned after its time); a TimeError at or after the held start is printed as data, not required.
- Terra 3 adopted: S4 asserts, within each Session, the start's side against that Session's own drain limit computed from its own rows (not only the class).
- Terra 4 adopted: every retry is printed in full (no first-5/last truncation): its R, Start, the report ticks around it and the send returns; the cell totals count retries in RETRIED and UNSETTLED trials alike.
- Terra 5 + Opus 6 adopted (bench.md): the C7 table states the storm's events as `⊆ rows and ≤ resumes` (not one per cycle); line ~1708 says the Module writes either cause.
- Opus 2 adopted: the hardware storm runs 20 s (or stops at the record bound + 200 resumes), and its line prints the probe's row count and dropped rows; its rehearsal matches the hardware length.
- Opus 3 adopted: a missing capture artifact or Manifest section is INCONCLUSIVE with its reason, never a panic that ends the grid.
- Opus 4 adopted: the m = 0 / starved / heard-before-stall case reads `dropped, never resumed` per the grid (fixture corrected).
- Opus 7 adopted: a row for the "exactly one tx_resume row" assertion. Opus 8 adopted: S4′ (a host stall in the recovery send → one retry, the next part plays → RETRIED end to end).
- S2″'s lead 9.9 → 11 ms (a scenario change after a run, not an expectation tuned): accepted; repeated ×20 in the fix round.

## B3 fix round (report notes, orchestrator)
- With the 1 ms hardware period, R − busy spreads over [m, m + 1 ms): the m = 0.2 and 1 ms cells overlap in effective margin. Accepted: the owner reads m from the per-trial R − busy (printed) and the retry rate against it, not from the nominal cell alone; bench.md says so.
- The 10 MS/s hardware-period rehearsal moved its aim to 300 µs from the lattice (the Module's own `time_now` width under load reached 35–86 µs on the manual clock); fake-period rehearsals keep 20 µs.

## Sol 6.2 code review of 715ebd67..500726dd (sol-code-final.md): FAIL, 8 majors, 2 minors, 1 note — dispositions (orchestrator); all adopted
- F1: the observation window follows the latest accepted Start's device tick (the trial deadline kept); a 1 ms-period retry rehearsal (S4′ at the hardware period).
- F2: the oracle and the RF scan use the first accepted Start as the observed origin (a SendAsap-moved initial booking), or mark it INCONCLUSIVE with both ticks; a fixture with a moved initial Start.
- F3: R's uncertainty bounds the stamps that set `busy` (the widest accepted read covering each contributing send, and the probe-stamp-to-Module-sample offset), else `R unresolved`; a fixture where a wide read covers the held send and a narrow one the decision.
- F4: the gap's silence is checked by a power test independent of PN alignment (lattice-aligned correlation kept for the phase verdict), and the UNSETTLED branch scans the gap too; fixtures: off-lattice PN radiated in the gap → not CLEAN; an intermediate part radiating before an UNSETTLED end → not PASS.
- F5: the storm checks the capture reaches its scheduled endpoint; short or unreadable → INCONCLUSIVE.
- F6: the storm's expected bound is the default (1 000) given independently on both paths; the kept rows are checked to be the first rows.
- F7: every `C7 trial` line prints row 1's `R − busy`.
- F8 (bench safety): each transmit invocation is its own copyable block with its board-specific attenuator/PN confirmation immediately above it; no multi-command transmit block.
- F9: `delta_first_us` is missing when its attribution is ambiguous (or labelled "first observed report after the hold"); the report greps follow `hw`'s `.N.log` suffixes.
- F10: compute the control fit and final-part reads once; keep each resume's evidence in one struct; use `with_rows`; fold C7-76's fake-only helper into its caller and drop the row.
- Re-check after the fix: Opus + Terra (the verdict decides the bench's result).

## Re-check of the Sol fix round (d26e8c14): Opus (opus-recheck-final.md) PASS, minors N1–N3, N7, notes N4–N6; Terra (terra-recheck-final.md) FAIL, 4 majors, 2 minors — dispositions (orchestrator); all adopted (brief-final-fix2.md)
- Terra 1 = Opus N1 (clock bracket narrower than the Authority's, look-ahead past the return): whole-call bracket, no read after the return.
- Terra 2 (unresolved R accepted any lattice point — a false CLEAN): only lattice points inside [lo, hi].
- Terra 3 (storm rows checked only for order): each retained row on its own incident's rule range; INCONCLUSIVE on missing data.
- Terra 4 + Opus flake analysis + N3 (load flake: the capture feed, drop_oldest 64 blocks, evicts when its thread is starved; pre-existing, not from the fix round): capture link 1024 blocks (204.8 ms at 10 MS/s, 16.4 MB), the Manifest's link drop count is its own INCONCLUSIVE reason before any hole message; ×40 under 16 `yes` 40/40.
- Opus N7 (hardware-period rehearsals asserted one allowed R): verdict + R within [lo, hi].
- Terra 5 = Opus N2 (storm continuity): from the capture's first sample. Terra 6 (silent final part coverage): read 1..=9 when silent. N4 (4·p_q term untested), N5 wording, N6 comment.

## Re-check of fix round 2 (f69e94f6): Opus (opus-recheck-fix2.md) PASS, 3 minors; Terra (terra-recheck-fix2.md) FAIL, 2 majors — dispositions (orchestrator, brief-final-fix3.md)
- Terra 1–2 = Opus minor 1 (the recorded bracket is still inside the Authority's; a read installed between the inner send return and the Module's stamp): downgraded to minor — both reviewers agree the only outcome is a loud ResumeOffRule on a correct Module, never a false CLEAN, in a preemption window the fake cannot show. Answered in the procedure, not with more oracle code: bench.md says the probe's bracket is the closest bound inside the Authority's (INFERRED), and a ResumeOffRule exactly one lattice point outside [lo, hi] is rerun once before it counts.
- Opus minor 2 (1024 not checked in a real Session): a rehearsal asserts the capture feed capacity, with a row. Minor 3 (bench.md wording) fixed. Notes: redundant asserts removed. Minor-only round: no further re-review.
