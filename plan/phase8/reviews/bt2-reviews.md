# bt2 — reviews


## sol-plan

Read-only review of `work/bt2` at `715ebd67`. Before opening the plan, I traced six failure scenarios: the Run ignores the margin override, the stall exercises the wrong send, a planned resume differs from what reaches the radio, the RF check misses a discontinuity, report timing misstates the available margin, and C2’s new timestamps change which packets can play. I checked those scenarios against the plan, the orchestrator’s decisions, the raw C2/C3 logs, and UHD 4.10 source.

1. **Major — The FakeDevice’s idle ramp breaks the shared silence check.** Run the late-start rehearsal with a quiet-floor window near the bottom of the fake’s 65,536-sample ramp and the induced gap near the top. The fake returns the ramp wherever it has no matching TX samples, even with equal RX and TX rates. Two windows can exceed four times the measured floor while the fake transmitted nothing. The observable is a false `SilenceViolated` on the good path. Use the known ramp as the fake’s baseline and add a fixture that distinguishes it from actual PN leakage. `test-plan.md:124` · `device.rs:1304`

2. **Major — S4 can miss the fake’s drain boundary on every trial.** Open the repeat as soon as 32 samples fit in the handover window, hold that first send for 40 ms, then close it with the two-zero EOB. The first send can contain 32 samples. The 2,500-sample value is a cap. With that short send, `now + 2 ms` dominates `busy + 20 µs` and leaves ample clearance beyond the fake’s drain pad. Every point in S4’s sweep can be `CLEAN` on correct code. The same sequence can make S1b’s asserted 10-to-1 ms shift too large. Pin the accepted send size and both sides of the boundary with an independent expected `R`. The existing provider test explicitly arranges and asserts a full 2,500-sample send. `test-plan.md:48` · `test-plan.md:142` · `segment.rs:133` · `mt3d.rs:599`

3. **Major — The RF verdict can pass a restart one full period late.** Record an accepted SOB at `R_act`, receive silence for one PN period, then receive the correct PN. At offset zero, the 20-period correlation still has about 95% of the control’s gain and the same phase. Signal also appears within the first ten periods. The observable is `CLEAN` even though RF started 100 µs late. The repeated control introduces another ambiguity because correlation peaks a whole period apart can tie. Anchor loop delay to an observed control onset, check the first resumed period independently, and require contiguous capture across the entire interval used for phase. The current coverage gate checks ten periods while the phase check uses twenty. `test-plan.md:84` · `test-plan.md:120` · `tone.rs:275`

4. **Major — The verdict does not establish which fault the trial exercised or why a retry dropped.** For example, hold a mid-play continuation, get no attributable starvation report, record an `expired` resume, and hear good RF at the final start. The table can emit `CLEAN` for the starvation cell with no evidence of starvation. Conversely, resume after a genuine first drop, then suffer an unrelated host stall inside the recovery send. Two rows suffice to print “m too small,” even if the old burst had drained. Require mode-specific fault evidence for a decided trial, including pre-stall RF for starvation. Pair each resume with its accepted SOB, report evidence, and RX span. Count retries separately from evidence that `m` caused them, and assert `tx_gap_samples == Σ(R_plan − H)` when the rows are complete. `test-plan.md:120` · `test-plan.md:127` · `tx.rs:1025` · `tx.rs:1136`

5. **Major — `tau_first` cannot validate C2’s delivery margin.** Hand UHD a timed send that it fragments. Let the first fragment reach the radio and produce a `TimeError` while `send()` is still handing over later fragments. Even with a perfect clock map, `t1 − r1` can be negative while the final fragment still has positive delivery delay. Using it as an upper bound can validate C2’s 1 ms premise incorrectly. UHD’s FPGA timestamps the radio’s time check, which can also occur after arrival. Keep the signed measurement with its actual endpoints and clock uncertainty. Establish a C2 delivery bound on a comparable path before using it to qualify the verdict. `test-plan.md:108` · `test-plan.md:176` · [UHD TX fragmentation](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/tx_streamer_impl.hpp#L293-L325) · [UHD radio time check](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L332-L357)

6. **Major — The drain estimator discards the evidence that a resume was early.** Let the original burst still be draining at `R_act`, with surviving error reports immediately before and after it. Selecting only ticks `< R_act` drops the second report and guarantees a positive `R_act − D_obs` whenever a value exists. The tool can report positive apparent clearance for the very trial where the drain crossed the resume point. This also invalidates S4’s equivalence: the fake reports the **start** of a dropped buffer’s drain and advances the drain end by its length. Even with lossless fake reports, `D_obs` is not exact and `D_obs − busy_replay` can be negative. Report attributable bounds or an unresolved result, and pin S4 against the fake’s actual drain limit. `test-plan.md:102` · `test-plan.md:142` · `test-plan.md:155` · `device.rs:762`

7. **Major — C2’s revised premise is not enforced by its verdict.** Send a late head and a future-stamped tail. The last call returns more than 1 ms before its stamp, but its packets reach the radio after their deadlines. A packet-local drop path can then produce the same silence, TimeErrors, and absence of ACK as a whole-burst drop, and the proposed verdict prints `PASS`. The jump also changes software pacing if `send_chunked` uses the jumped `at` in its window wait, so the old trace cannot establish the new handover margin by adding 3 ms. Keep pacing and wire stamps explicit, require a credible on-time opportunity for an identifiable tail region, and put the same-shape control in the verdict. The plan currently calls control placement observation-only and also promises an INCONCLUSIVE result from it. Limit the conclusion to the exercised shape. `test-plan.md:172` · `mt3.rs:255` · [UHD DUC timestamp accumulator](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L396-L433)

8. **Minor — Correct the raw-log inference and keep C2’s two lengths distinct.** Both trial-zero traces are present, with UHD `L` output prefixed to the trace lines. All 20 trials missed the original premise. The paper agreement between reconstructed and recorded last-send lateness is an algebraic identity because `lead` already includes `handed_last`. It establishes neither a fixed flow-control window across burst lengths nor the first send’s handover delay, which is `null` in these traces. Also retain `S × tx_per` for the packet-spacing diagnostic when the RF observation hull grows by 3 ms. `test-plan.md:163` · `raw C2 log:8` · `raw C2 log:40` · `mt3.rs:593`

9. **Minor — Several proposed mutation checks can survive.** Removing the `!flush` guard from `Every` is equivalent with the `at.is_some()` guard still in place, since T3 is untimed. The guard as written also admits timed two-zero EOB closes, so the storm changes recovery timing as well as payload timing. In the verdict table, a silent final part fails the phase/gain check before it can reach `NoResume`, leaving a deletion of `NoResume` alive on the named silence fixture. Pin flush, payload, and close as distinct inputs, and give each verdict mutant a fixture that reaches its branch. Rust files under `tests/` use token anchors too. `test-plan.md:66` · `test-plan.md:125` · `tx.rs:894` · `anchor.py:243`

10. **Minor — The hook’s shared Run path is sound in outline, but two checks should change.** The accepted Rust builder fits the existing `assembly` adjustment path, and the normal server construction uses `from_binding`. Add a default Run after a custom-margin Run to test isolation through that path. Drop the `env::var` file-list assertion. Adding a margin read in the already-listed `device.rs` leaves the set unchanged, so the assertion misses the regression it claims to catch. For S8, use `section_unchecked` when reading the intentionally truncated Manifest. The normal accessor panics as soon as it sees omissions. `decisions.md:4` · `test-plan.md:50` · `catalogue.rs:147` · `common/mod.rs:659`

No files changed. The findings come from source and log review. No tests, mutations, or bench transmissions ran.

Verdict: FAIL

## opus-recheck-plan

# bt2 test plan v2 — Opus re-check (read-only, work/bt2 = 715ebd67)

## Step 1 — own failure scenarios

(I drew these up after reading the resume path in `tx.rs` and the fake's drain in `device.rs`, before opening `sol-plan.md`, `decisions.md` or the plan files. I wrote them down here at the end.)

1. Hook unread: the builder stores `m` but `tx.rs:1112` still reads `RESUME_MARGIN_NS`. Every m cell then reports the 10 ms behaviour, and a sweep prints CLEAN at "0.2 ms".
2. Hook leaks: a later Run, a `with_reopened` copy, or a selector key carries a non-default m into an ordinary Run.
3. m never decides R: `R = lattice(max(busy+m, now+lead, front))`. After a starvation or a late start, `busy ≈ r + n1·per`. With a short first send, or at 10 MS/s where the cap (2 ms) equals the lead, `now + 2 ms` binds for m ≤ 1–2 ms. The 1 ms and 0.2 ms cells are then identical, and their CLEAN says nothing about m.
4. Lattice rounding swamps m: when the period is long compared with m, two m values give the same R.
5. Retry misattributed: a part dropped again because the host stalled its send, or because only the lead was left, is reported as "m too small".
6. Evidence-free CLEAN: the stall misses the intended send, nothing starves, and the trial still reports CLEAN.
7. Fake vs X310: the fake reports a drop's *start* and keeps a held drain with a growing pad. The X310 reports are throttled. So a drain bound, a "dropped again" outcome or a "settles" outcome observed on the fake may not carry over to the X310.
8. RF false PASS: a restart one period late, a capture hole in the gap, or a phase reference taken from another burst.
9. Rehearsal false PASS: the hold sleeps on `std::thread::sleep`, so the manual clock never moves and nothing is late; or the fake's ramp is read as signal or as silence.

## Step 2 — Sol findings (adopted set)

| Sol | Status | Evidence |
|---|---|---|
| F1 ramp baseline | **CLOSED** | Plan §4.3 adds the `sample == ramp(tick)` baseline and §9 C7-20 checks ramp+0.1·PN against a wrap. The ramp formula and the transmitted-replaces-ramp behaviour match `device.rs:2609–2690`. A wrap of the DC-free correlation gives ≈ 0.2 > T, so the C7-20 mutant dies. |
| F2 first-send size / boundary | **PARTLY** | I re-derived S1b's numbers from the code. busy2 = 22 532 (`staging.rs:756`; V5 `mt3d.rs:599–626`) and decided = 23 000. Limit 23 572 = 23 502 + (⌈2502/100⌉ + 2·2 + 1·(15+10) + 15) (`device.rs:764–783`, close drained via `device.rs:928`). So R is 23 630 at 999 µs and 23 530 at 998 µs, 23 030 at 200 µs and 32 630 at 10 ms: all correct. n1 is asserted from the probe, and F11 holds (`segment.rs:44,97,135–139`, `tx.rs:66–80,635`). However, `decisions.md` also names S4 as F2's pin, and the Session-level S4 cannot pass as written (finding 1). |
| F3 one-period-late restart | **CLOSED** | W0 alone, W−1, LateRestart (row 8), coverage = phase window, onset-anchored d. Window semantics: finding 3. |
| F4 fault evidence / pairing / retries / counters | **CLOSED** | §4.4 and §5 rows 1, 2, 10, 11. The counters identity matches `tx.rs:1136–1140`. Small gaps: finding 5. |
| F5 τ as a bound | **CLOSED** | `delta_first` is signed, printed with its endpoints and fed to no class (§4.6, C7-33). |
| F6 one-sided drain | **PARTLY** | `D_before` / `D_after` with `cleared/crossed/unresolved`. The plan states that the fake reports a drop's start (`device.rs:772–783`, F6). F7's numbers match the raw C3 logs: I recomputed all 40 trials. 362–373 reports per trial, a dense prefix of 33 (1 MS/s) or 20 (10 MS/s) reports, a tail median of 11 988 ticks and a span of 19.97–20.03 ms. S4's first-incident offsets against the limit are right, but what it expects after the drop is not (finding 1). |
| F9 send classes / verdict fixtures / anchors | **CLOSED** | Every Module send is timed (`tx.rs:682,718,905`), and the close is always 2 samples (`tx.rs:886–905`), so the `Close` predicate is sound. The flush is untimed `sob&&eob` (`core.rs:1307`). NoResume now comes before PhaseDiscontinuous. One mutation row does not compile: finding 4. |
| F10 isolation / env::var / section_unchecked | **CLOSED** | S1e runs a default Run after a custom one. The env assertion is dropped. S8 uses `section_unchecked`. `with_reopened` takes `mut self` (`provider/mod.rs:346`), so the field carries over. |

## New findings

**1. Major — S4's Session B cannot reach `DROPPED_AGAIN_M(1)` with a heard part, and §5 has no class for a retry chain that never settles.** (I derived this by hand from the code; nothing was run.)

*Why every retry is handed inside the fake's held drain.*
- A paused part that `gap_turn` does not move goes to `send` (`tx.rs:459` → `tx.rs:487`).
- `send` hands it as soon as `R < window_cut = now + in_flight` (`tx.rs:448`; `segment.rs:134–139`).
- The missed-R rule moves R whenever `R < now + lead` (`tx.rs:1054–1055`). So a part is always handed at `now ≤ R − 2 ms`, which is below `busy_k`, which is below `limit_k`.
- In `drain()` (`device.rs:772–783`), `from = max(now, cursor) < limit_k`. The fake therefore keeps the old drain `held` and adds to its `samples`, `packets` and `bursts`.
- The pad (`device.rs:764–767`) then grows by about +25 (samples) +4 (packets) +25 (burst: g + DUC wait) ≈ **+54 samples per incident** at 1 MS/s.
- Accumulation within one drain is designed and tested: `fake_limit_ticks(200, 20_000, 8, 1) = 256` samples (`mt3_cascade.rs:494`) is the same formula with 8 packets.
- Meanwhile `R − busy` stays fixed. `busy_k = R_{k−1} + 2 502` with R on the period-100 lattice, so `R_k − busy_k = 98` µs for every m ≤ 98 µs.

*The sequence at 1 MS/s, m = 30 µs:*
```
incident 1: busy1 = r+2502, limit1 = r+2572, R_B = busy1+30 = r+2532       -> part dropped (the plan's intent)
            part (2 500 at R_B) handed at now <= R_B-2 ms < limit1 -> held drain: end R_B+2500, samples 5002, packets 3, bursts 1
            close -> end R_B+2502, samples 5004, packets 4, bursts 2
incident 2: busy2 = R_B+2502, R2 = lattice(R_B+2532) = R_B+2600
            limit2 = R_B+2502 + (51 + 8 + 50) + 15 = R_B+2626 > R2          -> dropped again
incident 3: R3 - busy3 = 98, limit3 - busy3 = 178                           -> dropped again ... until the trial ends
```
- The same happens at 10 MS/s: R − busy = 99.8 µs, while limit − busy goes 60.5 → 105.9 µs → …
- With N = 100 µs, no m that drops the first part (< 70 µs at 1 MS/s, < 60.5 µs at 10 MS/s) lets any later part play.

*What it prints.* Rows ≫ 2, nothing is heard after any `Start`, and the Stop comes at the cell's latest time. §5 row 7 (`NoResume`) comes before rows 10 and 11, so the trial is `FAIL NoResume`. It is not `DROPPED_AGAIN_M(1)` (1 MS/s) or `RETRY_UNATTRIBUTED(1)` (10 MS/s). The plan's "Can: … that the repeat settles" (§6) is false on the fake.

*Why this is more than a wrong rehearsal expectation.*
- The same row order turns an unsettled chain on the X310 into a `FAIL` that fails `hw_c7_*`: every part dropped again until the trial's time runs out.
- §0.3 says a retry charged to m is a datum, never a FAIL.
- The X310 is unlikely to storm. C4 puts its per-drain overhead at about 20 µs (`bench/2026-11-18-mt3/README.md:11`), well below `m + lattice slack` ≥ 200 µs at the swept m, so a crossing shrinks with each retry. That is INFERRED; the table must not depend on it.

*Fix.*
- Add an explicit class, for example `UNSETTLED(k)`, a variant of the retry classes. It applies when every `Start` after the first drop is followed by drop evidence (a late report at or after its `at`, then a further `tx_resume` row) and the trial ends with no part heard.
- Place it before `NoResume`, so `NoResume` keeps meaning "started, not dropped, silent".
- Make it a datum, not a FAIL, as §0.3 already says for m-charged retries. Use the fake storm (Session B) as its fixture.
- Rewrite Session B to expect: retry 1's attribution (charged at 1 MS/s, unattributed at 10 MS/s), `rows ≥ 2`, no end note, nothing radiated, and the new class. Compare `mt3d_a_repeat_whose_every_part_is_dropped_late_keeps_resuming_and_transmits_nothing`, `mt3d.rs:751`.
- Drop "settles" from §6.
- Unconditionally moving `DROPPED_AGAIN_M` ahead of `NoResume` is not a fix: a part that started, was not dropped, and stayed silent must still be `NoResume`.

*Correct as written.* Session C (m = 31 µs → R_B + N > limit1). S4': the 12 ms hold hands the third part after a fresh drain whose limit it clears. The S1b Tx-rig pair.

**2. Minor — S8 does not check the two properties bench.md's C7 row assigns to the storm.** `plan/phase8/bench.md:1589` asks for confirmation that a repeat whose every part is dropped late "sends nothing" and that there is "one `TIME_ERROR { late_at_device }` per cycle". S8 (§6) asserts only the record bound, `omitted`, `Fine` and `held_ns == 0` on closes. Add RF silence over the storm on the hardware run (the §4.3 detector applies), plus events = resumes for the cycles the bound kept (or the per-kind omitted count for events). Otherwise the bench line has no tool output to cite.

**3. Minor — window semantics of the detector are underspecified in two places that can false-INCONCLUSIVE or false-FAIL a good trial.**
- (a) The onset check uses the "first 25-window above T" (§4.3), but T is a gain threshold for a full-period PN correlation. If the onset is computed as `gain(t)` at stride N/4, a window that only partly overlaps the signal at a non-zero lag gives a sidelobe of about `g_c·√f/√N`. At N = 100 that is about 0.1·g_c·√f, which exceeds T = 0.05·g_c once the overlap f > 0.25, so the onset fires 0.25–0.75 N early and lands outside "half a period of at + d". The result is INCONCLUSIVE "d not resolved" on good trials, rehearsals included. Specify the onset as a power test (mean |s|² over N/4 samples against the quiet floor's power statistic).
- (b) The silence region ``from+d+2N, A+d)` must use windows lying **wholly** inside it. A window that starts inside but runs past `A+d` picks up the part's PN at a non-zero lag, and those sidelobes exceed T. Row 6's fixture list covers the front guard only; add a fixture with good PN starting exactly at `A+d`, which must stay CLEAN.

**4. Minor — mutation row C7-02 will not compile as written.** After phase B, `RESUME_MARGIN_NS` is imported in `provider/mod.rs` (the `from_binding` default) and leaves `tx.rs`. `core.rs` imports only `DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS` (`core.rs:27`). The replacement `self.ticks(RESUME_MARGIN_NS)` inside `Core::resume_margin` is then a compile error, which `remote_mutate.py` counts as ERROR, not a kill. Use `self.ticks(crate::profile::RESUME_MARGIN_NS)`.

**5. Minor — loose ends in the F4 text.**
- §4.4 makes an unpaired row (no `Start` ≥ 5 ms past R) `NoResume`. But §5 row 7 is defined only from RF after the last `Start`, and no fixture covers a row with no `Start`. Add the condition and a fixture to row 7, or make the case INCONCLUSIVE in row 1.
- `plan-v2-changes.md` §2 F4 says the charge needs "return ≤ A − 1 ms", while the plan §4.4(ii) says `≤ A − lead` (2 ms). The plan is binding; the implementer should not follow the changes file.

**6. Note — Q7's risk is stated backwards.** `avail = in_flight − (start − now)` (`segment.rs:134`). A booking that reaches the Module *later* gets more window, so it still produces a full `n1`. Admission latency does not shorten `n1`. It only eats the 1.5 ms above the 5 ms command lead, so the booking turns late, moved or refused. The 6.5 ms choice stands. The trial should print the booking's lateness or refusal, which §6 S2's self-review already covers.

**7. Note — the phase reference crosses device bursts.** P0 and P3 compare the part with the control's phase from another device burst, about 100–150 ms earlier. Every existing phase check (`tone::phase_is_steady`, `tests/common/mod.rs:1094–1113`) compares within one burst. No bench record found shows two bursts in one Run with the same loop phase. Searches run: `grep -n -i phase` on `bench/2026-11-18-mt3/README.md`, `plan/phase8/bench-results.md` and `plan/phase8/bench.md`; `grep circular_spread_deg` across `tests/common/`. Within-burst steadiness on these boards suggests near-zero residual DSP shift, so this is probably fine, but it is INFERRED. The plan lists it as INFERRED, but only as "LO drift". If it fails, every trial of a cell gives PhaseDiscontinuous with a consistent offset. Suggest printing, in mode (b), the part's phase against the pre-stall periods of the same repeat as a cross-check. Not a blocker.

**8. Note — the first incident in mode (a) will almost always resume as `expired`, not `late_at_device`.** After the held send returns, the next turn hits `clock.instant(next) < now` (`tx.rs:468–470` → `timeline_expired`, `tx.rs:1027–1029`) before any report is read. That happens on the fake and the X310 alike, so `late_at_device` as a resume cause shows up only on retries. The plan's evidence rule allows for this. The bench summary should say so, so that nobody reads it as a missing `late_at_device` observation.

**9. Note — two F7 details in the plan are off.** My recomputation from the raw C3 logs puts last − first at 19 968–20 029 µs, about −32…+29 µs around 20 ms, not the plan's −12…+27 µs. It is still within one 60 µs spacing, so the "earlier-or-equal bound with one-spacing resolution" use holds. §12 still says "6 trials at 1 MS/s"; F7 is now 20 trials per rate.

Checked and right:
- The S2'' and S2' arithmetic: exactly two R values over a 100 µs hold sweep, and the short/full lead-bound split.
- `m_bound`'s threshold logic.
- The S4 Session-B offsets against the first limit: busy + 70 µs at 1 MS/s, busy + 60.5 µs at 10 MS/s.
- The SNR arithmetic: σ_g = 1.7e-5, q ≈ 2.6σ_g, T, `g_c1/T ≈ 14–20`.
- The 360 ms capture and its timeline.
- F3 (`plan/phase8/bench-results.md:223`) and F14 (`bench/2026-11-18-mt3/README.md:11`) against their records.
- The hardware run time. `hw_n47s5_front_after_a_short_burst` (2026-11-11, board 36) ran 36 Sessions, one per VERDICT line, in 88.6 s, or about 2.5 s per Session. The plan's 3.5–5 s per Session puts 100 Sessions at 7 minutes or less, under the 15-minute bound.

Not checked / not opened:
- The fake rehearsals' wall time: about 40 Sessions with a 360 ms capture, up to 3.6 M samples at 10 MS/s, plus S8's ≥ 1 100 resumes, against the per-test timeout. I did not check whether any rehearsal could die by timeout. The plan should state an expected wall time per rehearsal, and S8 should run without a capture.
- `draft.md` E2–E6, F12's UHD sources, design/18 lines 446–448, `n47s5::book_capture`, `mt3::covers` and `common::omitted_in`.
- Whether the X310 attaches a tick to an Underflow report (the starved-mode evidence rule relies on it; the Module's own code already does).

Verdict: FAIL


## terra-recheck-plan

**Verdict: FAIL.** The revised plan closes several original findings, but its proposed hardware verdict can still attribute a retry to the margin without establishing that the earlier drain reached the restart. Two proposed mutation fixtures also cannot kill the mutations they name.

1. **Major — A recent report does not establish that the drain was still running at the retry.**  
   Sequence: the old drain ends 100 µs before the resumed start; its last report is 150 µs before that start; the on-time resumed send is then dropped for another reason. The 200 µs minimum report window accepts that report as proof that the drain was running at the start, and the tool prints `DROPPED_AGAIN_M` and “m was short by the radio’s drain.” The cited measurements concern 20 ms bursts and explicitly leave C7’s shorter shape and the Module reader unmeasured. A report *before* a start proves neither that drain continued to it nor that drain caused the retry. Keep this retry unattributed unless evidence actually brackets or otherwise establishes the cause. [test-plan-v2.md:28`, `test-plan-v2.md:118`, `test-plan-v2.md:150`.

2. **Major — The “cleared” classification can hide a lost report tail.**  
   Sequence: the last received report is within 0.5 ms of `busy_replay`, but later reports are lost and the actual drain continues across `R_act`; the resumed part plays. The stale-tail test passes, no `D_after` is available, and the tool prints `cleared` with a positive clearance statistic. Reaching the Module’s estimated busy bound does not establish that the radio’s report train ended there; the plan itself leaves delivery and reader loss unmeasured. Classify the end as unresolved without evidence of the tail, and label `R − D_before` as conditional on report attribution rather than a measured clearance. `test-plan-v2.md:28`, `test-plan-v2.md:119`.

3. **Major — A resume row with no accepted start has no reachable verdict as specified.**  
   Sequence: the Module records a resume point, remains running beyond it, but never hands a new SOB to the device. Pairing says this is `NoResume`, yet the verdict’s `NoResume` branch requires `W0` at the last accepted start. There is no `A` from which to construct `W0`; coverage also requires that start. The named silent-RF fixture has an accepted start and therefore does not exercise this missing-SOB case. Add an explicit branch and fixture for an unpaired resume row, before checks requiring `A`. `test-plan-v2.md:116`, `test-plan-v2.md:140`, `test-plan-v2.md:146`.

4. **Minor — Two named mutation killers cannot distinguish their mutants.**  
   C7-21 removes mean subtraction from correlation against a zero-mean waveform. For a constant input, `Σw*·constant = 0` with or without subtraction, so the proposed DC fixture does not kill it; mathematically the operation is redundant under the stated waveform invariant. C7-34 removes `r` from the busy recursion, but the specified V5 fixture has `r = 17,000 < x = 20,030`; both formulas yield 22,532. Use a case with `r > max(busy, x)` for C7-34, and remove or redesign the equivalent DC mutation. `test-plan-v2.md:107`, `test-plan-v2.md:204`, `plan-v2-changes.md:136`.

5. **Minor — Mixed retry outcomes are not fully represented.**  
   If a trial has both charged and unattributed retries, row 10 applies only when none are charged, while row 11 prints only the charged count. The cell line therefore can hide an unattributed retry. Report both counts independently. `test-plan-v2.md:149`.

Disposition of the adopted findings:

| Finding | Status | Evidence |
|---|---|---|
| F1 | **CLOSED** | The fake baseline and ramp-wrap/leakage fixtures are specified. `Plan:108` |
| F2 | **CLOSED** | Accepted sizes, independent R, and the exact fake boundary are pinned. The S1b hand arithmetic matches the fake’s formula. `Plan:53`, `device.rs:764` |
| F3 | **CLOSED** | The onset anchor, first-period check, and full phase-window coverage address the stated one-period-late scenario. `Plan:109` |
| F4 | **PARTLY** | Mode evidence, pairing, and counter checks are added; retry attribution and a missing accepted SOB remain problematic as findings 1 and 3 describe. `Plan:116` |
| F5 | **CLOSED** | `delta_first` is signed, has endpoints and uncertainty, and is not used as a delivery bound or verdict input. `Plan:128` |
| F6 | **PARTLY** | Reports on both sides are retained and the fake limit is pinned, but the proposed `cleared` inference remains unsound when the tail is lost. `Plan:119` |
| F9 | **PARTLY** | Send classes, branch fixtures, and Rust token anchors are specified, but not every claimed mutation killer works; see finding 4. `Plan:64` |
| F10 | **CLOSED** | The default-after-custom Run, removal of the file-list assertion, and unchecked truncated-Manifest access are specified. `Plan:56`, `Plan:172` |

The cited C4 boundary agrees with the bench summary. The per-board runtime is an explicitly inferred estimate, and its individual test bounds exceed that estimate; I found no demonstrated bound overrun from the stated figures. No tests, mutations, or bench transmissions were run, and no files were changed.

Verdict: FAIL

## opus-recheck-v3

# bt2 test plan v3 — Opus re-check (read-only, work/bt2 = 715ebd67)

Nothing was built or run, and no file in the tree was changed. Every number below was derived by hand from the code it cites. I did not open `terra-recheck-v3.md`.

## Step 1 — own failure scenarios (written before reading the plan)

1. **Hook unread.** The builder stores m, but `tx.rs:1112` still reads `RESUME_MARGIN_NS`. Every m cell then measures 10 ms.
2. **Hook leaks.** m is held in a global or carried into another Run.
3. **m invisible.** `R = lattice(max(busy+m, now+lead, front))`. When `now + lead` binds, the 1 ms and 0.2 ms cells cannot be told apart.
4. **Evidence-free CLEAN or UNSETTLED.** The stall misses its send, or a report belonging to an *earlier* burst is read as the drop of a later start.
5. **Fake ≠ X310 in time.** The fake decides that a packet is late, and reports it, at `tx_send`, before device time reaches the packet (`device.rs:787–791, 859`). The Module reads reports before each step (`tx.rs:275–277`). So on the fake a chain of drops runs without virtual time advancing. The X310 drops a packet only when the radio reaches it.
6. **First-send size.** A first send takes `min(wanted, avail)` once `avail ≥ 32` (`segment.rs:133–139`). Any hand number that assumes full chunks breaks when the Module hands the send at the window's edge.
7. **Host times taken after a hold.** If the probe reads `host_before` after the sleep, a window anchored on it starts after the event it is meant to bracket.
8. **RF.** A ramp wrap read as signal, a window that straddles a start, or a cross-burst phase reference.

## Step 2 — status of the adopted findings

### Sol (sol-plan.md)

| # | Status | Evidence |
|---|---|---|
| F1 ramp baseline | **CLOSED** | v3 §4.3 adds the baseline; C7-20 has wrap and leakage fixtures; the ramp formula matches `device.rs:2630`. |
| F2 first-send size / both sides | **CLOSED** for what F2 asked. | S1b is the exact Tx-rig pair. `expected_r` is checked on every row, and `n1 == cap` is asserted. |
| F3 one period late | **CLOSED** | W0 alone, W−1, LateRestart (row 9), coverage equal to the phase window, and a power-test onset. |
| F4 fault evidence / pairing / counters | **PARTLY** | Pairing, the counters identity (`tx.rs:1136–1140`) and counting without attribution are in place. The starved-evidence window, however, cannot contain the starvation report (finding 1). |
| F5 τ as a bound | **CLOSED** | `delta_first` is signed, printed with its endpoints, and feeds no class. |
| F6 one-sided drain | **CLOSED** | `D_before` / `D_after` are raw data with no class and no "cleared". The fake's limit is pinned in S1b and on S4's first row. |
| F9 send classes / fixtures / anchors | **CLOSED** | Flush, Close, Start and Continuation are separate inputs. Every verdict row has a fixture that reaches it. `.rs` rows are token-anchored. I checked C7-30, 50 and 51: their mutants flip the named fixtures. |
| F10 isolation / env / unchecked | **CLOSED** | S1e is in place, the `env::var` assertion is gone, and S8 uses `section_unchecked`. |

### Opus v2 (opus-recheck-plan.md)

| # | Status | Evidence |
|---|---|---|
| 1 chain without a class | **PARTLY** | `UNSETTLED(k)` comes before `NoResume`. But the Session B expectations are re-derived wrongly (findings 2 and 4). |
| 2 S8 silence / events | **CLOSED** | `storm_verdict`, the 600 ms RF window, and the events relations, with C7-52/53. |
| 3a onset power test | **CLOSED** | §4.3 and C7-55. I checked `P_c = g_c²·0.245` and `Tp = 0.1·P_c ≥ 7.7·p_q`. |
| 3b windows wholly inside | **CLOSED** | §4.3 requires `t + N ≤ E`. C7-54 uses good PN starting at `A+d`. |
| 4 C7-02 compile | **CLOSED** | Uses `crate::profile::RESUME_MARGIN_NS`. `core.rs:27` imports only the other two constants. |
| 5 row with no Start | **CLOSED** | Paired, open and unpaired states; R11; C7-50. |
| 6 Q7 backwards | **CLOSED** | §4.1. |
| 7 cross-burst phase | **CLOSED as text** | But see finding 5: the phase now also gates evidence. |
| 8 first incident `expired` | **PARTLY** | The text now says "on the fake and the X310 alike". That is wrong for the fake (finding 6). |
| 9 F7 numbers | **CLOSED** | F7 reads 19 968…20 029 µs, and §12 no longer mentions "6 trials". |
| not checked: wall time | **PARTLY** | A wall time is now stated, but on the premise of "~50 rows". Session B produces several hundred rows (finding 2). The wall time is probably still fine. |

### Terra v2 (terra-recheck-plan.md)

| # | Status | Evidence |
|---|---|---|
| 1 report before a start as proof | **CLOSED** | The charge rule and `w` are gone. |
| 2 "cleared" hides a lost tail | **CLOSED** | No class, no "cleared"; Q11. |
| 3 row with no Start | **CLOSED** | R11 with its fixture, evaluated before any check that needs `A`. |
| 4 equivalent mutants | **CLOSED** | C7-21 now mutates `c7::wave`; a random QPSK has `|Σw| ≈ √N·0.35 ≫ 1e-9`. C7-34 recomputed: max(0, 21 000, 20 030)+2 500 = 23 500, then +2 = 23 502; the mutant gives 22 530 → 22 532 (`staging.rs:755–757`). |
| 5 mixed outcomes | **CLOSED** (moot) | Counts are independent. |

### Binding simplification checks

- **No attribution is left over.** I grepped for `attribut|charg|m too|m was|too small|cleared|settle`. Every hit is a negation, the starvation-evidence word "attributable", or `m_bound` / `m_informative`. The last two say which term set R, not why a retry happened, and no class reads them (§5, C7-37). Nothing left over.
- **Mutual exclusion holds for the cases the plan covers.**
  - R11 (≥ 5 ms past R) and T1 (< 5 ms past R) are complementary.
  - NoResume (judged, no drop evidence) and T3 (last start open) are complementary.
  - NoResume and T2 (last start dropped) are disjoint.
  - R11 is evaluated in row 1, before row 7.
- **Each class has a fixture:** UNSETTLED (T1/T2/T3 at k = 1 and 3), unpaired (C7-50, the 8 ms case), NoResume (C7-43 and row 7's negative fixture).
- **Two holes remain** (finding 3 and note 8): the drop evidence of the last start can be satisfied by an earlier burst's reports, and a rows == 1 trial whose start was dropped has no class.

## New findings

### 1. Major — the starved-mode evidence window starts after the starvation report, so every correct starved trial is INCONCLUSIVE

- **The window.** §4.4(b) requires an Underflow report with tick "between the held send's mapped `host_before − bracket` and `A_1`" (test-plan-v3.md:121).
- **Where `host_before` is read.** §3 makes the hold sleep **before** `host_before_ns` is read (test-plan-v3.md:85). So `host_before` is the hold's end.
- **Where the Underflow tick lies.** The fake reports the Underflow at the radio's cursor, the instant it ran dry (`device.rs:844–846`; `device.rs:2150–2161` on a read). On the X310 the underflow time is INFERRED to be the dry point too; the Module uses `report.tick` (`tx.rs:1429–1435`).

Sequence (mode (b), any m, any rate; times from §4.2, test-plan-v3.md:105):
```
+90   first Start handed (not held); repeat plays
+150  Midplay fires: the continuation is held 40 ms (inner.host().sleep)
~+160 the radio runs dry -> Underflow, tick = cursor ~ +160
+190  hold ends; host_before/host_inner_before read now (~+190); the inner tx_send runs
      (on the fake the Underflow is queued here with tick = cursor ~ +160)
      Module reads it -> tx_resume(cause underflow), R ~ +195..+210, part plays
verdict: is there an Underflow tick in [190 - bracket, A_1]? No, it is ~160
       -> R7 INCONCLUSIVE "no attributable starved evidence"
```

- **Wrong outcome.** Every good starved trial is INCONCLUSIVE. S3's rehearsal fails on correct code, and `hw_c7_starved` would fail all 100 Sessions on the bench. The C7-26 fixture ("no Underflow row") stays INCONCLUSIVE either way, so no mutation row notices.
- **Fix.** Anchor the window's lower edge on the held call's **entry** (`host_inner_before − held_ns`, or record an `host_entered_ns` before the sleep), mapped, minus the bracket. Or use the end of the last sample accepted before the hold. Add a fixture with the Underflow tick between the call's entry and its post-hold `host_before`, which must give CLEAN.
- **Same anchor, weaker use.** The pre-stall RF clause ("ending before … `host_before − bracket`") uses the same anchor. It is then satisfied by RF that ends 30 ms before the stall begins. Use the same entry anchor there.

### 2. Major — S4 Session B pins wrong per-row offsets, and F15 ("`R − busy` stays the same", marked VERIFIED) is false on the fake: the rehearsal fails on correct code

The v3 claims:
- S4 asserts that every later row has `R_row − busy_replay` within 2 µs of 98 µs (1 MS/s) or 99.8 µs (10 MS/s) (test-plan-v3.md:174).
- F15 says the chain keeps `R − busy` constant (test-plan-v3.md:39).
- §4.2 says "up to ~50 rows fit" (test-plan-v3.md:105).

All three assume that every retry's part goes out as full 2 500-sample chunks. That holds only for the first two or three retries. The mechanism:
- The fake drops a part inside its held drain at `tx_send`, before device time reaches it, and queues the report at once (`device.rs:787–791, 859–861`).
- `pass()` reads reports before each step (`tx.rs:275–277`). The owner sleeps only when a pass did nothing (`tx.rs:250–252`, `IDLE` = 200 µs at `tx.rs:173`).
- So retries are decided at one virtual instant `r`, while R climbs by about 2.6 ms per row.
- Once R is more than about 7.5 ms ahead, the first send takes `avail = window_cut − R`, which is less than the cap (`segment.rs:133–139`: a first send needs only 32 samples of window).

1 MS/s, m = 30 µs, R_1 = busy1 + 30 = r + 2 532 µs (all at virtual time r):
```
part 1: avail = 10 000 - 2 532 = 7 468 -> 2 500; busy2 = R_1+2 502; R_2 = R_1+2 600 (offset 98)  ok
part 2: avail = 10 000 - 5 132 = 4 868 -> 2 500; R_3 = R_2+2 600 = r+7 732          (offset 98)  ok
part 3: avail = 10 000 - 7 732 = 2 268 -> 2 268 (short); busy4 = R_3+2 270; R_4 = lattice(R_3+2 300) = R_3+2 300 -> offset 30 µs  (plan pins 98)
part 4: R_4 = r+10 032 > window -> pass() idle, sleep 200 µs; next look avail = 200-32 = 168 -> 168 samples;
        R_5 = R_4+200 -> offset 30 µs; and so on: one row per 200 µs IDLE look
```
- At 10 MS/s the same thing happens at row 5: avail = 16 698 < 20 000, and the offset is 300 samples (30 µs) instead of 99.8 µs.
- The chain still never ends, because the held drain grows by at least 29 samples a row (`device.rs:764–783`). So `UNSETTLED` is right.
- From about +165 ms to the +300 ms Stop there are **≈ 650–700 rows**, not ~50.

**Wrong outcome.** `c7_rehearsal_a_part_inside_the_fakes_drain_is_dropped_again…` fails its per-row offset assertion from row 4 (1 MS/s) or row 5 (10 MS/s) on correct code. The plan treats that as "stop-and-report", so phase B stalls on a plan error.

**Fix.**
- Keep the per-row `resumes_at == expected_r(...)`, which already pins R exactly from the probe's real sizes. Pin the fixed offset only on row 1 (30 µs) and on row 2 (98 / 99.8 µs).
- Rewrite F15: the drain grows each row while `R − busy` varies with the first send's size.
- Correct the "~50 rows" figure: on the fake, hundreds of rows per Session B trial. Retry lines are already capped at 6, and `record_rows_per_kind` is 100 000, so nothing else breaks.

### 3. Major — T2's drop evidence for the last start accepts the *previous* burst's drain reports, so at small m a silent part with no report becomes `UNSETTLED` (a datum) instead of `NoResume` (a FAIL)

**The rule.** §4.4 calls a start dropped when its W0 is silent **and** a TimeError with tick `≥ A_i − 2 ms` exists **and**, for the last start, no row follows (test-plan-v3.md:120). T2 then gives UNSETTLED (row 7).

**Why it fires on old reports.** At m = 0.2 or 1 ms the previous dropped burst drains until about `busy + τ` ≈ `A − m`. Its reports (one per ~60 µs, F7) therefore fall inside ``A − 2 ms, A)`. Nothing ties the report to the last start.

Sequence, mode (a) on an X310, m = 0.2 ms:
```
row 1 -> S_1 at A_1 dropped (inside the old drain), reports until ~A_1+2.5 ms+τ; row 2 -> R_2 ~ busy2+0.2 ms
S_2 handed at A_2; the radio drops it but its reports are lost (F2: 32-entry FIFO), or a Module defect leaves it silent with no report;
no row 3; Stop at +300 ms (A_2 + 5 ms <= judge_end: judged)
evidence: W0_2 silent; S_1's drain reports at A_2-1.5 ms ... A_2-0.1 ms are ">= A_2 - 2 ms"; last start, no row after -> "dropped"
verdict: row 7 T2 -> UNSETTLED(1) -> "PASS (contract); unsettled in 1/n"
```
- **Wrong outcome.** A repeat that went silent with no report is NoResume, the failure the class exists to catch. Here it is reported as a datum. Worse, the owner reads it in the m cells as "m too small → unsettled", the very attribution the tool must not suggest.
- **rows == 1.** The same evidence makes the trial fall past rows 7–9 and print `PhaseDiscontinuous`, a wrong label.
- **Fix.**
  - For the **last** start (no row after it), require a TimeError with tick `≥ A_i`. A late head is checked against the radio's time after `A_i` (F17), and an earlier burst's reports lie below `A_i`.
  - Add a fixture: old-drain reports at `A − 1 ms`, no report at or after `A`, silent W0, rows = 2 → `NoResume`.
  - Optionally make T2 INCONCLUSIVE "a drop with no resume", since a T2 on hardware means the Module read a drop and did not resume before the Stop.

### 4. Minor — "open" for starts other than the last is unspecified, and Session B depends on it

- **The rule.** §4.4 defines a Start as open when `A_i + 5 ms > judge_end`, for any start (test-plan-v3.md:119). Drop evidence also asks for W0 "inside the coverage", and coverage ends at `E = min(capture end, judge_end + d)` (test-plan-v3.md:114, 120). Row 7 requires every start `S_i`, `i < rows`, to be dropped.
- **Session B on the fake.** Parts are handed about 9.8 ms ahead, with one row per 200 µs (finding 2). At the Stop, about 75 non-last starts are open by that rule, and about 50 of them lie past `judge_end`, outside the coverage.
- **What it produces.** If "open" or "outside coverage" means "not judged", row 7 fails. Rows 8–9 do not apply, and row 10 prints `FAIL PhaseDiscontinuous`.
- **Fix.** State that a start followed by a row is judged by its drop evidence whatever `judge_end`; the next row shows that the drop was read. Keep "open" for the last start only. Extend the drop-evidence W0 check to the capture, not the examined silence region. On hardware only one or two starts lie past `judge_end`, so this mainly protects the rehearsal.

### 5. Minor — the starved evidence gates on the cross-burst phase

- **The gate.** §4.4(b) requires pre-stall periods "with phase within 15° of `phase_c`" (test-plan-v3.md:121). That phase reference comes from another device burst, which Opus 7 and §12 mark INFERRED.
- **What it does.** If the phase does not hold across bursts, every starved trial becomes INCONCLUSIVE R7 instead of showing the offset through `phase_vs_prestall_deg`, the cross-check v3 added for exactly this case.
- **Fix.** Gate on `heard` (gain) only; keep phase in P0/P3 and in the printed cross-check.

### 6. Note — the first incident of mode (a) is `late_at_device` on the fake, not `expired`

- **Why.** The fake queues the TimeError inside the held `tx_send` (`device.rs:787–791`). `pass()` reads reports before the step (`tx.rs:275–277`). `underflowed(tick)` hits because `tick = r ≥ first` (`staging.rs:603–607`). The existing `ur_22_a_late_send_asap_repeat_resumes` sees `late_at_device` (`tests/fake.rs:2679–2681`).
- **Where the plan says otherwise.** "On the fake and the X310 alike" (test-plan-v3.md:121) is wrong.
- **Effect.** Both causes are accepted, so nothing breaks. The bench text (§7) should say "`expired` or `late_at_device`".

### 7. Note — §4.2's per-retry spacing is wrong, without consequence

- **The claim.** "≈ 5 ms at the default m" (test-plan-v3.md:105) cannot hold, since m alone is 10 ms.
- **On the X310.** The Module keeps handing continuations into the 10 ms window until the drop is read, so `busy ≈ R + 10 ms` and retries come about `10 ms + m` apart.
- **Effect.** The "fits in 360 ms" conclusion holds more easily, so only the text needs fixing.

### 8. Note — `rows == 1` with a dropped start and no row after it has no class

- **What happens.** Row 7 needs `rows ≥ 2`. Row 8 needs no drop evidence. So the trial falls to row 10, `PhaseDiscontinuous`.
- **Reachability.** It cannot happen in the planned timing: the Stop comes at least 90 ms after `R_1`. With finding 3's fix the hole narrows further.
- **Fix.** Name it (INCONCLUSIVE `a drop with no resume`) so no fixture can reach a wrong label.

## Checked and right

- S1b's period-100 numbers, cross-checked with the v2 re-check: limit 23 572 = 23 502 + 55 + 15.
- S2'' four cases:
  - short / m = 10 ms → m-bound;
  - short / m = 1 ms → `R − (r + 2 ms) ∈ [0, 100)` → lead-bound;
  - full / m = 1 ms → m-bound;
  - full / m = 0.2 ms → `R − (r + 2 ms) ≥ 702 µs` → m-bound.
- C7-30's mutant flips the lead-bound case.
- S2' always gives exactly two R values over a 100 µs sweep of five points.
- Session C: `R_C = busy + 130 µs > limit busy + 70` → plays.
- Session B row 1 at 10 MS/s: pad 201 + 4 + 250 = 455, limit 605 samples = 60.5 µs.
- C7-34's numbers.
- The SNR and power arithmetic: `σ_g = 1.69e-5`; `P_c` from 9.8e-7 to 5.5e-5.
- The hardware run time: 50 × 3.5 + 50 × 5 s ≈ 7 min a test, under the 15 min bound.
- The probe's row budget (65 536, `probe_device.rs:10`) against a 60 s storm and a 360 ms capture at 10 MS/s (≈ 1 800 `rx_recv` rows).

## Not checked

- `terra-recheck-v3.md` (not opened, for independence).
- The F7 raw logs (I relied on the v2 re-check's recomputation).
- UHD source for the X310's Underflow tick (finding 1's X310 half is INFERRED; its fake half is VERIFIED from code).
- How the trial decides at run time that "the last part has played" before its Stop (§4.2 gives no observable).
- Whether events are record-bounded (S8).

Verdict: FAIL


## terra-recheck-v3

1. **Major — The starvation window starts after the stall has finished.** The wrapper records `host_before` after its 40 ms sleep, but the starvation verdict treats that timestamp as the beginning of the hold. In the plan’s own timeline, the hold runs around +150 to +190 ms and the radio runs dry around +162 ms. A valid underflow at +162 ms therefore falls before the proposed evidence window and the correct trial is **INCONCLUSIVE** instead of CLEAN. Record a separate timestamp before sleeping and use it for the pre-stall RF and underflow checks. Add a fixture with the underflow strictly inside the hold. [Plan:85`, `plan:105`, `plan:121`.

2. **Major — The clock anchor and the proposed send timestamps use different origins.** `clock_anchor()` brackets device time with `ProbeDevice::host_ns()`, measured from the probe’s `Instant` origin. The new `host_inner_*` values use the inner device’s `HostClock`; the plan supplies no conversion between those axes. Sequence: obtain an anchor, hold and return a send, then map `host_inner_after` using that anchor. The mapped return can be wrong, corrupting `busy_replay`, expected R, `m_bound`, and the signed delay record; on a manual clock it can also make a correct rehearsal fail. Capture anchors on the same clock axis as the values being mapped, or explicitly calibrate the two axes and test that conversion. `ProbeDevice:105`, `probe:128`, `probe:296`, `plan:128`.

3. **Major — The no-start tail remains undefined for a single resume.** Sequence: the first incident records one resume row; the Run stops three milliseconds after R without an accepted Start. Pairing calls the row *open*, so R11 does not apply. `UNSETTLED` requires at least two rows, while `NoResume`, phase checks and CLEAN require a Start. The specified table has no outcome, although the multi-row T1 fixture passes. Define and test this first-incident tail separately, without calling it a retry chain. `Plan:119`, `plan:150`, `plan:157`.

4. **Major — Pairing pre-empts the early and off-lattice Start failures.** Sequence: a resumed payload Start is accepted one period before its planned R. Pairing requires `at ≥ R`, so the Start is unmatched and R10 returns **INCONCLUSIVE** before `StartedEarly` can report FAIL. Likewise, a Start off lattice cannot pair under the stated rule, so R10 pre-empts the Start-side `LatticeOff` check. The C7-42 fixtures claim those failure branches are reached, but do not satisfy the stated pairing rules. Classify unmatched Starts against the relevant resume before the generic unpaired reason, and make the fixtures exercise the whole verdict path. `Plan:119`, `plan:144`, `plan:147`, `plan:231`.

5. **Major — A report can be reused as evidence for the wrong dropped Start.** The drop rule accepts *any* TimeError at or after `A_i − 2 ms`, with no upper bound or report-to-incident association. Sequence: a resumed Start is silent for an unexplained reason; a later `expired` row occurs; a still later, unrelated TimeError exists. That later report can satisfy the earlier Start’s drop predicate, allowing the chain to print **UNSETTLED** or **RETRIED** rather than flagging missing evidence. This does not require attributing a retry to `m`: constrain and test what evidence supports each incident, and return unresolved when the reports cannot distinguish them. `Plan:120`, `plan:150`.

6. **Minor — The planned C2 note overstates what C7 measures.** It says C2 will be revisited after C7 “has measured the host-to-radio delay.” The actual `delta_first` is expressly neither a delivery delay nor a bound. Preserve the latter qualification in the bench note so the later C2 work does not rely on a measurement C7 did not make. `Plan:132`, `plan:193`.

7. **Minor — C7-37 does not specify a killable mutation.** “A verdict row reads” a retry field does not itself change a verdict, so the proposed pair of fixtures cannot establish that every such mutation dies. There is also an apparent distinction to make explicit: hold data are used to validate the intended stall, even though the retry record must not determine the retry class. Specify a concrete behavioral mutant and the exact field use it forbids. `Plan:86`, `plan:122`, `plan:227`.

The adopted findings from Sol’s first review have these dispositions:

| Finding | Status | Evidence |
|---|---|---|
| F1 | **CLOSED** | Fake ramp baseline and wrap/leakage fixtures are specified. `Plan:111` |
| F2 | **CLOSED** | The Tx-rig pair pins accepted size and both sides of the fake limit; the stated 998/999 µs lattice values and 70 µs limit agree with the cited drain formula. Session repeatability remains explicitly inferred and asserted. `Plan:56`, `device:762` |
| F3 | **CLOSED** | Onset anchoring, W0 and coverage through all 20 periods address the requested one-period-late case. `Plan:112` |
| F4 | **PARTLY** | Mode evidence, pairing and counter identity are specified, but findings 1, 3–5 leave the evidence and pairing incomplete. `Plan:119` |
| F5 | **CLOSED** | `delta_first` is signed, includes endpoints and uncertainty, and does not decide a class; the bench-note wording needs correction under finding 6. `Plan:132` |
| F6 | **CLOSED** | Both sides of the Start are retained as raw reports, without claiming clearance; S4 pins the fake’s limit. `Plan:123` |
| F9 | **PARTLY** | Send classes and token anchors are specified, but the early/off-lattice branch fixtures do not reach their claimed verdicts; C7-37 is also underspecified. Findings 4 and 7. |
| F10 | **CLOSED** | Default-after-custom isolation, removal of the env-file-list assertion and unchecked access to the truncated Manifest are specified. `Plan:59`, `plan:179` |

For the adopted v2 re-check findings:

| Finding | Status | Evidence |
|---|---|---|
| Opus 1 | **PARTLY** | Multi-row T1/T2/T3 fixtures and a non-failing UNSETTLED class exist, but the single-row open tail and incident evidence remain incomplete: findings 3 and 5. |
| Opus 2 | **CLOSED** | S8 specifies captured-window silence and the binding event relations. `Plan:179` |
| Opus 3 | **CLOSED** | Power onset, wholly contained silence windows and a boundary fixture are specified. `Plan:112`, `plan:114` |
| Opus 4 | **CLOSED** | C7-02 uses the fully qualified constant. `Plan:63` |
| Opus 5 / Terra 3 | **PARTLY** | The ≥5 ms unpaired-row fixture reaches R11 and is distinct from the multi-row UNSETTLED and NoResume fixtures. The single-row open case is still uncovered: finding 3. `Plan:237` |
| Opus 6–9 | **CLOSED** | The latency risk, inferred cross-burst phase reference, `expired` first incident and corrected F7 figures are recorded. `Plan:28`, `plan:96`, `plan:113`, `plan:121` |
| Opus “not checked” | **CLOSED** as a plan disposition | Expected rehearsal times and their inferred status are stated; S8 captures only 600 ms. The estimates remain unverified until implementation. `Plan:179`, `plan:183` |
| Terra 1–2 | **CLOSED** | The per-retry margin attribution and “cleared” classification are removed; report distances are raw, qualified data. `Plan:11`, `plan:123` |
| Terra 4 | **CLOSED** | C7-21 now targets waveform construction; C7-34 has an `r > max(busy, x)` fixture. Its 23,502 versus 22,532 arithmetic is consistent with the stated recursion. `Plan:213`, `plan:224` |
| Terra 5 | **CLOSED** under the binding simplification | Retry and unsettled counts are independent; the removed cause categories no longer need mixed-category accounting. `Plan:160` |

I found no remaining rule that attributes an individual retry to `m`. The proposed multi-row UNSETTLED, unpaired-row and NoResume fixtures describe mutually exclusive cases, subject to the gaps in findings 3 and 5. I found no demonstrated runtime-bound overrun from the stated estimates; they have not been measured. No tests, mutation runs, transmissions or edits were performed.

**Verdict: FAIL**

## opus-b1

# bt2 B1 — Opus sub-step review (read-only), commit dab47cc0 (work/bt2, diff HEAD~1..HEAD)

Reviewer: Claude Opus 5.5 subagent, 2026-11-19. Nothing in the repository was edited, formatted or committed. I checked builds and tests in a /tmp copy (`rsync --no-times`, then touched), with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` and `CARGO_INCREMENTAL=0`.

## Own failure scenarios (written before reading the plan) and what I found

| # | Scenario | Result |
|---|---|---|
| a | A reopen loses the margin (UR-7 reopen, `with_reopened`) | No. `with_reopened` keeps every field (`provider/mod.rs:349-352`). The server builds with `from_binding` and then `with_reopened` only (`ezsdr-server/src/catalogue.rs:147-149`), so it has no path to the margin. The bench `assembly` opens the authority, with its PLL retries, before it calls `adjust(from_binding(..))` (`tests/common/mod.rs:426-431`). |
| b | Another `Core::new` caller misses the argument | No. There are only two callers, `provider/mod.rs:525` and `provider/test_support.rs:152`, and both pass it. The only production reader is `tx.rs:1112`. `RESUME_MARGIN_NS` is now read only at `profile.rs:14`, `mod.rs:332` and the test helpers (grep `RESUME_MARGIN_NS\|resume_margin` over `crates/`). |
| c | The margin changes during a Run or leaks into another Run | No. The builder consumes `self` before the Provider is boxed, and the value is a per-instance field copied into a per-open `Core`. S1e pins the leak case. |
| d | `with_measurement_timing` or `with_measurement_performance` rebuilds and drops the margin | No. Both mutate `self` in place (`mod.rs:372-389`), so the order of builder calls does not matter. |
| e | A Mock parity gap | No. MockRadio never resumes a repeat (`ezsdr-mock-radio/src/lib.rs:1004, 1966`, `resumes_at: None`), so there is no Mock `m` to match. |
| f | A negative or huge `m`, or overflow in the tick conversion | Negative values are refused by an assert. A huge value is safe: `Core::ticks` computes in i128, and `as i64` cannot wrap for any mcr ≤ 1 GHz. `busy.saturating_add(margin)` followed by `resume_boundary(..)?` returning `None` ends the repeat, as UR-22 says for an `R` that does not fit the clock. `m = 0` is accepted and stays consistent with UR-22's "at least `m` + 32 samples apart". |
| g | The default changes silently | S1a compares the default with the literal `10_000_000` passed through the hook, so a change to the constant in `profile.rs` or to the initial value in `from_binding` turns it red. |
| h | Rows that survive or fail to compile | All five mutants compile. Each one leaves at most an unused-variable or dead-code warning, and no flag turns those warnings into errors in the tooling. A grep for `RUSTFLAGS\|D warnings\|deny.*warnings` over `plan/maintenance/tools/` (gate.py and remote/ included) and `Dockerfile*` finds only an example `clippy -- -D warnings` in `rdev.py:8`, which runs on the tree, not on mutants. The killers, worked out from the spans I measured (below): C7-01 makes every span about 23 300, so ten − one ≈ 0 fails. C7-03 gives a default span of about 13 300 against 23 300 with the hook, which fails. C7-02 gives R = 32 630 in the 1 000 µs case and C7-04 gives R ≈ 23 030 in the 10 000 µs case, and both are refused by the exact R check. C7-05 makes `negative.is_err()` false. Construction inside the closure succeeds: the same `from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), fake(FakeConfig::default())).unwrap()` is the helper at `fake.rs:522`, which passing tests use. It would still be cleaner to build outside the closure, so that only the builder can panic. I did not execute them through `remote_mutate.py`. |

Checks I ran: the whole `--test fake` binary three times (729 passed each time, about 112 s, load average about 2.3 on 16 cores) and `--lib` once (843 passed). `mt3d_a_core_margin_sets_r_and_the_fakes_drain_limit_splits_998_from_999` and the four new `tests/fake.rs` tests pass. I ran the three `c7_rehearsal_*` tests three times, idle machine only, with a temporary `eprintln!` in the /tmp copy. The spans were stable to ±100 µs, which is one lattice period: 23 200–23 300 at m = 10 ms (with or without the hook), 14 300 at 1 ms and 13 500 at 0.2 ms. `cargo fmt --all --check` is clean, `cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings` is clean, and `rules.py --base HEAD~1` reports 0 problems.

Plan conformance (test-plan-v3 §2, decisions.md "B1"): the code follows the planned path (`UhdRadio` → `Core::new` → `Core::resume_margin()` → `tx.rs:1112`) and the planned rows C7-01…05, including C7-02's full-path `new`. Two things differ from the plan. S1a, S1b' and S1e compare spans within a jitter allowance instead of exact R, which decisions.md records after F13 was refuted. S1c is a `catch_unwind` inside a combined test instead of `#[should_panic]`, which is equivalent. S1b' runs at 1 MS/s only, although the plan said "both rates" (see finding 2).

## Findings

1. **minor — The Manifest of a Run whose `m` was replaced does not say so.** `with_rx_stall` writes a timing row when it fires (`provider/rx.rs:236`), `with_reopened` writes `reopened_on_unlock` (`provider/mod.rs:539`), and `with_measurement_timing` and `with_measurement_performance` rewrite the `envelope` section (`mod.rs:372-389`). `with_measurement_resume_margin` (`mod.rs:364-368`) leaves no trace. In my runs, a 1 ms Run's `tx_resume` row has `resumes_at − from` = 14 300 µs against 23 300 µs by default, and nothing in its Manifest explains the difference. A later reading of the raw archive could take it for the 10 ms rule misbehaving, and the bench rows depend on the tool printing `m` itself. Proposed fix: one `timing` row `{ what: "resume_margin", ns }`, written beside `reopened_on_unlock` when the value differs from the profile's. Alternatively, record that `conditions.json` and the tool's output carry `m`. A new Manifest row is "what the Manifest records", which AGENTS §9 gives to the owner, so the orchestrator should bring it to the owner if it adopts the fix.

2. **minor — The Session-level hook test cannot resolve sub-millisecond values, and nothing exact covers the builder itself.** `SPAN_JITTER` is 1 500 µs (`tests/fake.rs:2723`). That is wider than the 800 µs that separates m = 1 ms from m = 0.2 ms. Counterexample: a builder that clamps or rounds, such as `self.resume_margin_ns = ns.max(1_000_000)`, gives ten − fifth ≈ 9 000, which is inside `9 800 ± 1 500` (`fake.rs:2743`), so the test passes. The exact sub-millisecond test, S1b (`mt3d.rs:628`), enters at `Core::new` and never touches the builder. C7 is a sweep down to 0.2 ms, so the bench value at 0.2 ms would silently be 1 ms. On my idle runs the spans were stable to ±100 µs; I did not reproduce the "1 200 with every core busy" figure. Fix: an in-crate unit test asserting that `from_binding(..).with_measurement_resume_margin(200_000).resume_margin_ns == 200_000`. That is exact and needs no clock. Tightening the Session tolerance is not an option, because the test's own comment gives 1 200 µs of load jitter. Finding 1's Manifest row, if adopted, would also give the Session tests an oracle that does not depend on timing. Separately, the plan's "both rates" for S1b' was not done. The conversion goes to root ticks and does not depend on the rate, so the risk is low, but the report should say it was dropped.

3. **note — The S1b test's name claims more than it pins.** The waveform period is 100, so R jumps from 23 530 (m = 998 µs) to 23 630 (m = 999 µs). The pair 998/999 therefore holds the fake's limit only somewhere in (23 530, 23 630], not at the hand-computed 23 572 (`mt3d.rs:628-633`). That is enough to show the hook is read, but B3 should not cite it as an exact pin of the drain limit. MT3-21…24 pin the pad.

4. **minor — A new oracle that depends on scheduling, with thin headroom.** The three `c7_rehearsal_*` tests compare spans across separate Sessions. By the test's own comment (`fake.rs:2721-2723`), the jitter is 100 µs idle and 1 200 µs "with every core busy", against an allowance of 1 500 µs. That leaves 300 µs of headroom under the load the gate and the mutation shards produce. A miss fails the gate, or makes a row look killed when it is not. They passed 3/3 in the full binary at load about 2.3 on 16 cores. I did not test full-core load. AGENTS §9 asks for 20 repeated runs of a new test whose verdict can depend on timing: the orchestrator should run those under shard-like load before the gate. If they show misses, B2 should replace the cross-Session comparison with a within-Session or Tx-rig oracle (see finding 2's fix).

5. **note — Maintainability (AGENTS §9).** The parameter is threaded once, and the default rigs still go through `RESUME_MARGIN_NS` (`test_support.rs:97`). The cost is two small wrappers (`rig_on_inputs_with_margin`, `rig_on_description_links_with_margin`), `switch_rig_with_margin`, and an `#[allow(clippy::too_many_arguments)]` on `core_on`, which is acceptable. The new mt3d test repeats the existing V5 sequence (`mt3d.rs:598-623`) but varies a different dimension (`m` rather than the period), so a merge is optional. Nothing is left over: the `RESUME_MARGIN_NS` import left `tx.rs`, and no unused helper remains. The design/18 sentence sits with the other measurement builders under UR-34, and the UR-22 clause points to it. `rules.py` passes.

Not checked: the mutants under `remote_mutate.py`, span jitter under full CPU load (I did not want to load a shared host), the 20-run repetition, and the 10 MS/s path through the hook.

Verdict: PASS


## terra-b1

1. **Major — The Session test can pass when the 0.2 ms hook value is ignored.** `fake.rs` allows 1.5 ms of difference between Sessions, but the 0.2 ms and 1 ms inputs differ by only 0.8 ms. The exact Tx-rig test bypasses the public builder, and C7-01 tests only a builder that ignores *all* inputs.

   Minimal sequence: a faulty builder clamps values below 1 ms to 1 ms; run the 10 ms, 1 ms, and 0.2 ms Session cases. The 0.2 ms case uses 1 ms, yet its comparison can still pass because the 0.8 ms error is within tolerance. A C7 sweep could consequently report measurements for a margin the Run did not use. Assert each Session’s resume point against its own observed inputs, as S1b′ planned, or otherwise add a deterministic builder-to-Run check that distinguishes these values.

The instance and Core wiring, default construction, `with_reopened`, server construction, selector refusal, and repeat resume path were reviewed. The accepted 200 MHz master clock keeps conversion of nonnegative `i64` nanoseconds within `i64` ticks. The planned shared C7 hardware trial does not exist in this commit, so its eventual use of the builder cannot yet be verified. I did not run tests or mutation rows.

Verdict: FAIL

## opus-b2

# bt2 B2 sub-step review (Opus, read-only) — commit 8a980fe9 (diff dab47cc0..8a980fe9)

Inputs: `review-b2-prompt.md`, `brief-b2.md`, `test-plan-v3.md` (§0–§6, §8–§9), every section of `decisions.md` (including the "B2 report — B3 grid" section added while this review ran). `terra-b2.md` was not read.

## My own failure scenarios first (written before comparing with the plan)

1. Stamps on the wrong axis: probe stamps on the `Instant` axis mapped through an anchor whose tick follows the manual clock, so a stall moves the device tick but not the stamp.
2. Stamps taken at the wrong moment: `host_entered` taken after the sleep, so the starvation window starts at the return and misses the Underflow.
3. A hold that reaches the wrong send: a close, a flush, the control's sends, or the part after the resume.
4. `busy_replay`/`expected_r` that agree with the Module only because both drop `returned`, the close, or the margin; or an aimed stall whose own model is the oracle.
5. Tolerances that hide a disagreement: `MAP_TOLERANCE_TICKS` (1 µs), the unpinned ±1-period allowance, `sent_before`'s slack. Or tolerances that do not fit the X310's 0.05–0.15 ms anchor bracket.
6. The Manifest `resume_margin` row written without the builder (the server path), or missing with it.
7. Switching `read_time_now`/`tx_send` from `host_ns` to `clock_ns` breaks or weakens existing tests that mix those stamps with `rx_*` rows or with `probe.host_ns()`.
8. "One-incident" rehearsals that never pin one incident, so a re-drop chain passes.
9. Rows that die by timeout or survive.

## How each scenario came out

- (1) is closed. `clock_ns()` (`tests/common/probe_device.rs:199-202`) puts the `tx_send`, `time_now` and anchor stamps on the inner manual clock. On a real clock it falls back to `host_ns`, so hardware is unchanged. The fake's device time is an exact function of `host.now()` (`src/device.rs:1214-1230`), and so is the Authority's extrapolation (`src/authority.rs:139-145`), so the mapping is exact on the fake. `probe_stamps_map_through_the_anchor_on_the_manual_clock` and C7-71 pin it.
- (2) is closed. `entered` is stamped before `hold_for` and the sleep (`probe_device.rs:558-565`). C7-72 pins it, and S3 asserts the negative case: a window from `host_before` misses every Underflow (`tests/fake.rs:2851-2856`).
- (3) is closed. `send_class` (`probe_device.rs:38-48`) is exercised by `probe_send_class_table`. The hold is armed after the control's close is in the probe (`c7.rs:360, 381`). The tests count from arming, fire once and never fire on a close (C7-64…70).
- (4) is closed for B2. `busy_replay` restates `TxBook::note_sent` (`max(busy, returned, first) + n`, `src/provider/tx.rs:829, 907`), and the close is included. The aimed stall's model (`busy = return + (cap+2)·per`, `c7.rs:377`) is checked twice by S2/S3:
  - against the Module's actual R exactly (`fake.rs:2824, 2858`);
  - against `check` over the probe's own rows.

  The pure numbers come from the hand-derived mt3d V5 table, not from the Module (C7-73/74/75).
- (5): see findings 5 and 7.
- (6) is closed. `resume_margin_ns: Option<i64>`. The row is written only on `Some` (`src/provider/mod.rs:541-543`). Both directions are tested (`margin_rows()`), with rows C7-61/62. The Core value is exact through `from_binding` → builder → `with_reopened` → `prepare` (`ur_22_the_resume_margin_builder_keeps_its_value_exactly`, C7-60/63/82).
- (7): no current reader mixes the two axes. My search was `grep -rn 'host_before_ns\|host_after_ns\|\.host_ns()\|clock_anchor' crates/ezsdr-radio-uhd/{src,tests}`.
  - s9bench reads only `rx_*` rows against `probe.host_ns()`, which is unchanged.
  - h4 reads `tx_send` rows against `clock_anchor`, both now on `clock_ns`. That is consistent, and identical on real clocks.
  - hardware.rs runs on a real clock.

  h4's fake tests do run on the manual clock (`common::fake`, `tests/common/mod.rs:494-500`), so the switch does reach them. I ran them: `h4_` and `s9` gave 41 passed. The remaining risk is in naming (finding 2).
- (8): finding 1.
- (9) is closed.
  - `b2-rows2.log` reports "65/65 passed"; `grep -c '"outcome": "killed"'` gives 65, and there is no SURVIVED or ERROR.
  - C7-66 survived in the first run (`b2-rows.log`). It was re-anchored to `starts == 1` and is killed in the second run, as are C7-82 and C7-01…05.
  - No row is marked `real_clock`. The killers are assertions, not timeouts: C7-68 stops the manual clock, and C7-81 leaves no resume.
  - The removed `late_start_span` leaves no orphan: `Descheduled::slow_first_send` still has a caller (`tests/fake.rs:2674`).

## What I ran, in a /tmp copy (`rsync --no-times`, the shared review target, `CARGO_INCREMENTAL=0`)

- `cargo test -p ezsdr-radio-uhd --test fake -- c7_ probe_`: 43 passed (34.8 s).
- `c7_rehearsal` ×5 at the default thread count: 15/15 passed each time (≈10 s a run). It was not run under a separate full-core load.
- `rules.py --base 715ebd67`: 0 problems.
- `cargo test -p ezsdr-radio-uhd --test fake -- h4_ s9`: 41 passed.
- `cargo test -p ezsdr-radio-uhd --lib -- ur_22_the_resume_margin_builder mt3d_`: 61 passed.
- `cargo test -p ezsdr-radio-uhd --no-run`: every target builds (lib, fake, hardware, uhd_api).
- Finding 1's fix, tried in the /tmp copy only (reverted afterwards): `rows.len() == 1` and exactly one `start` row after the held send, added to both incident helpers. Over all 16 S2/S3 Sessions each held, and the one event (`TIME_ERROR` or `TX_UNDERFLOW`) named `rows[0].resumes_at`.
- Not checked: the brief's ×20 under full-core load (I ran ×5 with the tests' own parallelism only), and the Sol / full-list mutation gate.

## Findings

1. **minor — the "one-incident" rehearsals S2/S3 do not pin one incident or its event.**
   - Where: `c7_late_start_incident` / `c7_starved_incident` (`tests/fake.rs:2804-2834`, `2836-2870`) check every row through `c7_rows_follow_the_rule` (`fake.rs:2715-2729`).
   - What is missing:
     - neither asserts `rows.len() == 1`;
     - neither asserts that the part at R is the last accepted start;
     - neither asserts that the event names `rows[0].resumes_at`.
   - The plan asks for these. Plan §6 S2: "one `tx_resume` row per incident; events at most one per Kernel burst". Plan §6 S3: "`TX_UNDERFLOW` with plan R when emitted".
   - None of these needs the B3 RF detector.
   - Counterexample (a regression of the fake's drain limit or of the part's first send that makes the device drop the part at R again):
     ```
     arm Start{nth:1, Until(aim)}; book the cell repeat
     → row 1 (expired, R1 = L)
     → part at R1 handed and dropped again
     → row 2 (late_at_device, R2 = expected_r(rows so far, m))
     → part at R2 plays
     ```
     `c7_rows_follow_the_rule` passes both rows, R1 == L passes, and "no start before R1" passes. S2 reports PASS on a trial that is actually a retry chain.
   - Fix: assert `rows.len() == 1` in the two incident helpers, and that every event's plan is a row's `resumes_at`. The second is cheap: the events are in the Manifest.
   - Verified in a /tmp copy, so the fix is known to hold:
     - every one of the 16 S2/S3 Sessions has exactly one row and one start after the held send;
     - one `TIME_ERROR` (late start) or `TX_UNDERFLOW` (starved) event carries `resumes_at` equal to the row's.

     So this is a missing pin, not a hidden defect.

2. **minor (maintainability §2) — one probe row stream now carries two time axes under the same field names.**
   - On a manual-clock fake:
     - `tx_send` and `time_now` rows' `host_before_ns`/`host_after_ns` (and the new `host_entered_ns`) are on the inner clock (`probe_device.rs:395-397, 558-567`);
     - `rx_start`, `rx_stop`, `rx_recv` and `device_drain` rows keep the same field names on the probe's `Instant` (`probe_device.rs:358-361, 490-492, …`).
   - On hardware the two axes are identical, so a reader that compares, say, a `tx_send` stamp with an `rx_recv` stamp is right on the bench and silently wrong only on the fake.
   - Counterexample: a future rehearsal computes "the TX send returned before the RX block arrived" from `row_tx["host_after_ns"] < row_rx["host_before_ns"]`. On the manual fake the inner clock lags real time by every busy interval, so the comparison can invert.
   - Nobody does this today (search in scenario 7).
   - The `clock_ns` doc is also inaccurate (`probe_device.rs:196-198`). It says the receive rows "stay on `host_ns`, which the S9 and the H4 bench tools measure real time with", but H4 reads `tx_send` rows and `clock_anchor`, which moved.
   - Plan §3 had proposed separate `host_inner_*` fields.
   - Fix: either add an axis marker to the rows (or a doc paragraph at `PROBE_ROW_LIMIT`/`rows()` stating which calls are on which axis), or correct the doc sentence about H4.

3. **minor — `Hold::Until`'s doc names the wrong axis.**
   - `probe_device.rs:49` says the instant is on "the probe's own axis ([`ProbeDevice::host_ns`])". `hold_for` compares it with `entered = self.clock_ns()` (`probe_device.rs:226`, `Hold::Until(until) => until.saturating_sub(entered)`).
   - On a manual fake a caller that follows the doc and passes `probe.host_ns() + d` gets a hold of the wrong length. It is zero once the real-time axis is ahead of the manual clock, and the stall silently does not happen. That would be caught downstream only by "a send was held" panics.
   - `c7::trial` uses `anchor.host_at` (the clock axis), so it is correct.
   - Fix the doc to say `clock_ns`.

4. **note — the comment of `probe_stamps_map_through_the_anchor_on_the_manual_clock` explains the kill wrongly.**
   - It says (`probe_device.rs:826-827`) that on a real-time axis "the stamps would not move at all and the mapping would be 40 ms short". Under M5 (`src/host.rs:19-20`) the manual clock runs no faster than real time, so a real-time axis moves at least 40 ms.
   - C7-71 is killed for another reason: the manual clock lags real time by the busy time it loses (M3). That lag is microseconds, against an exact tick equality (1 tick = 5 ns), which is robust.
   - The test itself is deterministic on the correct code: the test thread drives the clock, the anchor reads are at frozen instants, and 40 ms is a whole number of ticks.
   - Only the comment should say why it kills.

5. **note — fake-calibrated tolerances in the shared body; already carried to B3 by the orchestrator ("B2's notes for B3").** Three additions for B3:
   - (a) `sent_before`'s cut `returned <= at + bracket + MAP_TOLERANCE_TICKS` (`c7.rs:269-275`) must also cover the Module's own clock-read error. On hardware, a close whose mapped return lands a few µs past the Module's `at` would be cut, `busy` would lose it, and the check would false-FAIL.
   - (b) `trial` reads one anchor before the capture (`c7.rs:347`). Plan §4.6 asks for 3 before and 3 after and an affine map. Drift over ~300 ms is µs-level, so this is only a record, but the plan text and the code should agree.
   - (c) `MAP_TOLERANCE_TICKS`'s justification ("25 to 38 ticks apart on the manual-clock fake", `c7.rs:117-119`) is from idle runs.
     - The 1 µs tolerance and the ±20 µs aimed side are both fixed, and the probe's anchor is read once.
     - A larger anchor bracket or Module-read error under load makes these tests fail falsely. It does not make them pass falsely:
       - S1: a pinned point lands on the other side;
       - S2/S3: the error exceeds 20 µs and flips the side.
     - The brief's ×20 under full load is what bounds this, and I did not verify it.

6. **note — rehearsal assertions narrower than the plan, which B3's hardware verdict must not copy.**
   - S3 and `c7_bench_stall` assert cause `underflow` only (`fake.rs:2768, 2847`). Plan §4.4/S3 allows `underflow` or `expired`, with the report as the evidence. It is deterministic on the fake (five clean runs), so it is acceptable for the rehearsal.
   - On the fake the late start's first row came out `late_at_device` in all 8 aligned Sessions I printed, not `expired`. Plan §4.4 expected "almost always expired". The bench text should name both, as decided ("Opus v3 6, 7"), and the B3 verdict must accept both.
   - `trial` never checks `probe.rows_complete()`. A probe that hit `PROBE_ROW_LIMIT` would make `sent_before` lose sends silently. That is a failure direction, not a pass, and unreachable at these capture lengths on the fake. Plan §4.2/S8 lists the check, and B3 should make it an INCONCLUSIVE.

7. **note — the `check` oracle's `pinned` gate has no mutation row.**
   - A mutant `near = terms.r == row.resumes_at || (terms.r - row.resumes_at).abs() == self.period` (dropping `!terms.pinned`, `c7.rs:291`) would survive every test:
     - S2/S3 assert R separately and exactly;
     - S1's cross-margin `is_err()` assertions differ by ≥ 0.8 ms;
     - `ur_22_…` pins the Core value exactly.
   - So S1a/S1b'/S1e's exactness to the lattice point is not held by any row. This is a test-oracle weakening, not a production one, so it is a note. If B3 makes `check` the hardware oracle, give it a row then.

8. **note (maintainability) — small duplications, all introduced by this step, none a defect.**
   - The ceiling `ticks` closure is written twice (`c7.rs:283, 376`).
   - `prepared_margin_ticks` (`src/provider/mod.rs:870-902`) restates the binding JSON of `prepared_rig` (`mod.rs:839-845`). It is justified, because `prepared_rig` bypasses `prepare` and this test must go through it.
   - There are eight one-line rehearsal wrappers with copied comments (`fake.rs:2872-2934`). Separate test names give parallelism and per-cell failure names, so I would keep them, but the comment could live once on the helper.
   - `#![allow(dead_code)]` on all of `c7.rs` (`c7.rs:9`) will also hide B3 leftovers. Check at B3 that nothing in it is unused by both binaries.

## Against the binding decisions

- B1 review:
  - exact builder-to-Core test and clamp row: done (`ur_22_…`, C7-60);
  - S1b's name/doc: done (`mt3d.rs:628-632`, design/18 *Test:* updated);
  - within-Session oracle for the three `c7_rehearsal_*` tests: done (`fake.rs:2731-2763`);
  - the owner's `timing {what:"resume_margin", ns}` row only when the builder set m: done, with design/18:450 text and rows C7-61/62.
- Terra v3 1 / Opus v3 1 (starvation window from the held call's entry): done, as a pure fixture and as the negative Session assertion.
- Terra v3 2 (the anchor's axis): done.
  - The brief's literal "`host_ns`" was superseded by `clock_ns`, which puts the anchor on the same axis. That is the decision's intent.
  - The deviation from the brief's wording should be in the report.
- F9 (Flush/Close/Start/Continuation as distinct inputs): done.
- B2 contains nothing of the verdict table, S4 or S8: confirmed.
- S2'/S3' (five-Session sweeps) are replaced by `Stall::Aligned` ±20 µs, which books both sides of the lattice point by construction and asserts R exactly. That matches the brief's "each booking R's boundary on both sides", and C7-80 pins it.

Verdict: PASS


## terra-b2

1. **Major — The oracle can accept a resume one period too early or too late.** `c7.rs:150` marks a point near either side of a lattice boundary as unpinned, and `c7.rs:291` accepts a disagreement of one period in *either* direction.

   Minimal sequence: `busy + m` maps just below a lattice point; the module incorrectly resumes at the preceding point. The oracle accepts that early R, although mapping uncertainty could move the correct answer only to the point at or *after* the boundary. The converse holds just above the boundary. The rehearsals aim 20 µs away from the boundary (`fake.rs:2798`), so they do not exercise this allowance. The binding B3 decision also requires the tolerance to follow the measured bracket; the fixed 1 µs does not cover the stated X310 bracket. The allowed answers should come from rounding the endpoints of the uncertainty interval, with fixtures on both sides.

2. **Major — `sent_before` can include a later burst in an earlier resume’s busy replay.** `c7.rs:269` includes sends returned up to `at + bracket + 1 µs`, then cuts at the *last* close in that set (`c7.rs:275`). That does not establish that the close belongs to the decision being checked.

   Minimal sequence: a decision follows its close; the resumed start and its close both return within the mapping bracket after that decision. Replay selects the later close and counts its burst in the earlier decision’s busy value. A correct module can consequently fail the oracle, or an incorrect R can be compared against the wrong history. C7-77 checks a later send after the selected close, but does not cover a later **close** inside the uncertainty window. The selection needs an ordering or identity boundary, or must report the ambiguous case as such.

3. **Major — The planned lead-bound rehearsal is absent.** The shared trial fixes the late-start booking at a 6.5 ms lead (`c7.rs:362`); the boundary rehearsals assert that `busy + m` binds (`fake.rs:2826`). The binding S2″ plan calls for a short first send at 1 ms where `now + lead` binds, alongside margin-bound controls.

   Minimal sequence: book the short-first-send case with `m = 1 ms`; the decision lead exceeds `busy + m`. A tool that ignores the lead term, or counts the trial as margin-informative, is not caught by these Session rehearsals and can report the wrong expected R or cell count. The pure hand-number test does not replace the planned end-to-end case.

4. **Note — Verification limits.** The recorded targeted mutation run reports 65/65 rows passed, including the changed C7 rows; its summary does not expose each failure’s underlying test output. I did not rerun mutation, build, or tests. The verdict table, S4 and S8 remain outside this review’s sub-step.

Verdict: FAIL

## opus-b3

# bt2 B3 — Opus sub-step review (read-only), 1124a724 (HEAD~1..HEAD; step 715ebd67..HEAD)

## What I read and ran

**Read in full:**
- the diff of `c7.rs`, `c7_verdict.rs`, `c7_cells.rs`, `c7_storm.rs`, `probe_device.rs`, `fake.rs`, `hardware.rs` and `bench.md`;
- `c7_rf.rs` lines 1–350 (the detector; the `synth` module skimmed);
- the new `mutations.json` rows (C7-90…179: ids, files and filters);
- `decisions.md` (all), `brief-b3.md`, `opus-b2.md` finding 5, `terra-b2.md`.

**Read in part:**
- `test-plan-v3.md`: the F-table, §4.1, §4.4 (lines 120–121), the S2–S8 rows (lines 172–179) and §12 (line 264).
- `spec-b3-rf-verdict.md`: the verdict order (lines 23–41, 97, 114).
- `authority.rs`: the Module's clock anchor (lines 25–150 and 240–282).
- `tx.rs`: T1 (lines 715–718), the resume decision (lines 1105–1136) and `stop_now` (lines 1324–1333).
- the `ds-b3-*` logs, searched with grep.

**Ran:** in a /tmp copy of HEAD (deleted afterwards), `cargo test --test fake c7_rehearsal_a_drop_storm` twice with one `println!` added:
- as committed, 20 s;
- with `storm_ms` set to 60 000, the hardware test's duration.

**Not run:** the full fake suite, the `hardware` build, and ×20 repeats.

**Mutation rows:** the full run (`full.log`, started 04:05:55, 3 355 rows on 16 shards) had not finished when I wrote this, so I have no outcome for C7-90…179.
- `ds-b3-1.log:2058` reports that C7-94 was killed only by a then-flaky trial.
- That trial is the S2″ case with a 9.9 ms lead. B3-2 replaced the 9.9 ms lead with 11 ms (see note 13).

## My failure scenarios first (written before reading the plan and the decisions)

1. The R oracle and `history`'s ambiguity test compare two mappings to device time. One is a host stamp mapped through the probe's anchor; the other is the Module's own extrapolation. On the X310 each has its own error. If the slack holds only the probe's error, a correct Module is read as off-rule or ambiguous. → finding 1.
2. The hardware storm runs longer than its rehearsal. The probe's 65 536-row record could fill up with `time_now` and `rx_recv` rows. → finding 2, measured.
3. A starved repeat that never resumes. Its only Start is the pre-stall one, which was heard. → finding 4.
4. Mode (a)'s first row is `expired`, and the TimeError is lost or carries an earlier tick (F17, INFERRED on the X310). → finding 5.
5. A trial with no capture or no Manifest section panics, aborting a 100-trial grid. → finding 3.
6. A final Start cut by the Stop. → checked, not reachable as a false FAIL (note 11).
7. A late continuation after a starvation radiating inside the gap. → not reachable. T1 stamps every payload send with its time (`tx.rs:715-717`), so a late continuation is a late timed packet and is dropped.
8. Grid cells overlapping or unreachable. → checked.
   - The nine cells map one to one onto `verdict`'s final `match` (`c7_verdict.rs:338-385`).
   - Every cell has a fixture.
   - Only (≥2, silent) has a single fixture, the UNSETTLED one.
9. A one-period-off R. → note 9.
10. Storm counts under the record bound. → right for a first-N bound (`c7_storm.rs:89-108`).
11. Rehearsal and hardware disagreeing. → finding 2 and minor 8.
12. bench.md. → finding 6. The commands are sound:
    - they use the `hw` recipe (`bench.md:616-628`);
    - the log names match its `$1.$BOARD.log`;
    - every transmit line is preceded by the owner's attenuator and PN confirmation.

## Findings

### 1. Major — the mapping slack leaves out the Module's own clock-read error, so a correct Module can be read `ambiguous` or `ResumeOffRule` on the X310

**The binding disposition.** decisions.md, the section "B2 review … fixed first in B3", says: "Opus 5: the slack also covers the Module's own clock-read error; … the bracket is the worst." Opus 5 is `opus-b2.md:102-103` (5a). It says the cut by `at + bracket + MAP_TOLERANCE_TICKS` "must also cover the Module's own clock-read error. On hardware, a close whose mapped return lands a few µs past the Module's `at` would be cut … and the check would false-FAIL."

**The code.**
- `Anchor::read` keeps the smallest of three brackets (`c7.rs:193`, `min_by_key`).
- `uncertainty()` is the larger of the two best brackets, plus drift, plus `MAP_TOLERANCE_TICKS` (`c7.rs:367-370`).
- `MAP_TOLERANCE_TICKS` is still 200 ticks, which is 1 µs (`c7.rs:122`). The fake's figure was 25–38 ticks, but the doc now calls it "the allowance for the Module's own reading of its clock".
- `uncertainty()` is both the slack of `history`'s test `sends[end].returned > at + slack` (`c7.rs:253`, through `c7.rs:406`) and the half-width of `check`'s interval (`c7.rs:416-424`).

**What the Module reads (VERIFIED).**
- The decision instant is read right after `close_device_burst`: `let (margin, lead, now) = (…, self.core.now())` (`tx.rs:1111-1112`). The row records it as `"at": now` (`tx.rs:1136`). The sends feed `busy` through `book.note_sent(self.core.now(), …)` (`tx.rs:829, 907`).
- `core.now()` (`core.rs:968-969`) calls the authority's `now`, which is `extrapolate()` from one device read (`authority.rs:139-145, 240-251`).
- That read is anchored at the midpoint of its bracket. It is re-taken after each nap or wait, and accepted up to a 1 ms width (`authority.rs:31-34, 99-135, 265, 277, 552`).
- So the Module's error is up to half the width of its own last single read. It is independent of the probe's best-of-three read.

**Counterexample (recorded hardware brackets; the error split is INFERRED).**
- Setup: X310, `late_start 1MS/s m=1`.
- The probe's best bracket is 0.05 ms. The slack is therefore 50 µs, plus 1 µs, plus a drift of a few µs.
- The Module's last re-anchor was a read with a 0.15 ms half-width, the top of the recorded 0.05–0.15 ms range.
- Suppose the probe's midpoint error is +40 µs and the Module's is −100 µs. The dropped part's close returns at device instant X. The probe maps it to X + 40 µs. The Module decides a few µs later and records `at` = X − 100 µs + ε.
- `history` then finds `returned (X+40) > at + slack (X−100+ε+51)` and returns `Err("ambiguous: resume row 0's close returned at …, later than its decision …")`.
- The verdict pushes that text as an INCONCLUSIVE reason (`c7_verdict.rs:192-197`). `run_bench` counts any INCONCLUSIVE as a cell failure, so `hw_c7_late_start` fails. The Module did nothing wrong.
- With the opposite signs, `check` gives `ResumeOffRule`. For example, the probe's `busy + m` lies 30 µs below a lattice point L, so `[lo, hi] = [L, L+period]`. The Module's `busy` is 140 µs later, so it resumes at L + 2 periods. That is a false FAIL.

**How likely (INFERRED).** Both reads are midpoint estimates, so their systematic bias cancels. Only the jitter of the read widths remains, and any Module read up to 1 ms wide is accepted. With 200 trials per board, a few such alignments are plausible, and one fails the test.

**Fix (orchestrator).**
- `history` pairs a row with its close by ordinal (Terra 2). Its time test is only a sanity check, so a wide slack costs nothing there. Use half of `WIDEST_BRACKET` (0.5 ms), or the worst of the six probe reads.
- For `check`, take the worst of the six reads' brackets, as the disposition says ("the bracket is the worst").
- Add a fixture with a Module-side offset larger than the probe's bracket.

**Trade-off.** Widening `check` widens `allowed`, which is already 2–4 points on the X310 (note 9). The alternative keeps the interval narrow and reports `ResumeOffRule` only when R is more than one lattice period outside `[lo, hi]`, printing a nearer miss as data. The orchestrator should pick one.

### 2. Major — `hw_c7_storm` runs 60 s, three times its rehearsal; at 60 s the probe's record overflows, so the storm is INCONCLUSIVE

**The code.**
- `run_storm` books 60 s (`c7_cells.rs:528`); the fake rehearsal books 20 s (`fake.rs:3075`).
- `storm_verdict` makes `!probe_complete` an INCONCLUSIVE reason (`c7_storm.rs:56`).
- `PROBE_ROW_LIMIT` is 65 536 (`probe_device.rs:10`).
- The probe records every `time_now` and `rx_recv` call, not only `tx_send`.

**Measured on the fake (a /tmp copy of HEAD, plus one `println!`):**

| | 20 s, as committed | 60 s, the hardware duration |
|---|---|---|
| storm line | resumes=1629 rows_kept=1000 omitted=629 events=1629 | resumes=4907 rows_kept=1000 omitted=3907 events=4907 |
| test result | ok, 29.5 s | panicked at `fake.rs:3089` |
| probe rows | 25 211 in all | the record is full at 65 536 |
| `time_now` | 11 515 | 29 784 |
| `rx_recv` | 10 403 | 27 016 |
| `tx_send` start / close | 1 631 / 1 631 | 4 353 / 4 353 |
| dropped rows | – | 9 250 ("the probe dropped 9250 rows at its limit of 65536"); 74 786 calls in all |

So at the hardware duration the record is incomplete, and `storm_verdict` returns `Inconclusive(["the probe is incomplete"])`.

**On the X310 (INFERRED).**
- The `time_now` rows come from the authority's re-anchor after every nap and wait (`authority.rs:265, 277, 552`) and from the test's own `wait(ms(5))` loop. That is hundreds a second on a real clock too.
- The `rx_recv` rate depends on the receive request size, which I did not measure.
- Whatever the hardware rate, the rehearsal stops at 20 s and so does not book the hardware test's own length (AGENTS §9 hardware-test rule).

**Not needed.** The cycle is set by the 12 ms hold and the window: about 12.3 ms a cycle on the fake. 20 s already gives about 1 600 resumes, more than the bound of 1 000.

**Fix.**
- Run the hardware storm for 20 s, like the rehearsal, or stop it once `stats.tx_resumes` passes the bound + 200. Either way, keep the hardware and rehearsal durations equal.
- Print `storm.probe.len()` on the `C7 storm` line, so the margin to the limit is visible.
- Do not add a probe switch for this.

### 3. Minor — a trial with no capture or no Manifest section panics inside `evidence`, aborting the 15-minute grid instead of being INCONCLUSIVE

- `c7_verdict::evidence` calls `n47s5::captured_samples(manifest)` unconditionally (`c7_verdict.rs:453`). That reaches `capture_of`, which panics "no artifact of rec" (`mod.rs:673-675`).
- `Common::resumes()` and `reports()` call `section_unchecked(...).as_array().unwrap()`. `section_unchecked` panics when the section is absent (`mod.rs:660-662`).
- Sequence: the capture submit is refused, or the Run ends with a device error before `sink.CAPTURE_WRITTEN`. `trial` returns normally, with a non-empty `incomplete` or a Run that is not Fine. Then `evidence` panics, `bounded` reports the panic, and the remaining cells of that mode never run.
- B2's note asked that a missing control close "returns evidence (INCONCLUSIVE), never panics". The neighbouring gaps should behave the same.
- Fix: make the capture an `Option`, with "no capture" as a reason. Or wrap one trial's `evidence` and `verdict` in `catch_unwind` and record the panic text as INCONCLUSIVE.

### 4. Minor — starved, m = 0: the "final Start" is the pre-stall Start, so a starvation that is never resumed reads `INCONCLUSIVE no incident`

- For `m == 0`, `final_start` is `starts[0]` (`c7_verdict.rs:227`). In a starved trial that is the repeat's own first Start, which was heard before the stall. So `(0, heard)` gives `no incident` (`c7_verdict.rs:342`) whatever followed the stall.
- The fixture `c7_verdict_m0_heard_is_no_incident` (`c7_verdict.rs:664`) shows this. It carries the starvation's Underflow at `STALL` (`c7_verdict.rs:537`, about +150 ms) and a tail of about 10 ms past it (`stop_tick = entered − BRACKET + 30 ms` ≈ +160 ms, `c7_verdict.rs:575`).
- By the grid's row 0, "silent" rule (drop evidence and a tail ≥ 5 ms → `dropped, never resumed`), this is the Module failure C7 exists to catch. The line says "no incident".
- It still fails the hardware test, because INCONCLUSIVE fails, but the reason points the owner at the hold rather than the Module.
- Fix: in `Mode::Starved` with `m == 0`, choose the column from the evidence after `held_time` (`evidence_after(held_time)` and/or the RF after the stall), not from `starts[0]`. This fixture's expected class is then `DroppedNeverResumed`; add its short-tail and no-evidence variants.

### 5. Minor — mode (a)'s `expired` first row is gated on a TimeError tick ≥ the held start's time

- `c7_verdict.rs:245` gives `INCONCLUSIVE no attributable late-start evidence` when the first cause is `expired` and no `TimeError` has a tick ≥ `held.at`. This follows the B3 spec (`spec-b3-rf-verdict.md:97`).
- But the X310 side of F17 is INFERRED (`test-plan-v3.md:38`), and a lost report (F2) also trips the gate. That works against the "INFERRED premises print as data" generalisation (decisions, Opus v3 5).
- If `expired` is the X310's usual first cause (`test-plan-v3.md:121`; on the fake it is `late_at_device`, `opus-recheck-v3.md` note 6), every late-start trial rests on this premise. If the premise fails, every late-start cell is INCONCLUSIVE.
- `shown_late` (`returned − bracket > at`, `c7_verdict.rs:236-238`) is already host-side proof that the start was handed over after its time.
- Recommendation (orchestrator): print the report's presence as data (`t1` is already printed) and drop the gate, or keep it and record why.

### 6. Minor — bench.md text

- `bench.md:1708` says "最初の出来事では fake は `late_at_device` を出し，Module は `expired` を出すことがある". Both causes are written by the Module; the fake emits neither. Suggest: 「最初の出来事の cause は `late_at_device` か `expired`（どちらも Module が書く；どちらも失敗ではない）」.
- `bench.md:1589`: the C7 row still says the late-start cell also confirms "1 周期に `TIME_ERROR { late_at_device }` が 1 つ". The hardware storm checks only `events ⊆ kept rows` and `events ≤ resumes` (Q12; `c7_storm.rs:98-104`). Say so, or say that the one-per-cycle equality is checked on the fake only.

### 7. Minor — rows for the B2 review fixes are incomplete

The brief asks that each review fix come with a test and a row.

- Terra 1, Terra 2 and Terra 3 have rows: C7-90/91, C7-92/93 and C7-94/95.
- **Opus 1** has no row. Its fix is that S2 and S3 assert exactly one `tx_resume` row, and that the event names that row's R. Nothing breaks the one-row assertion: no mutant writes the resume row twice, and no existing `tx.rs` row is re-filtered to these rehearsals.
- **Opus 7** asked for "a row on the `pinned` gate". `pinned()` is now used only by the pure test (`c7.rs:710-722`), so that row is moot. Record it as such.

### 8. Minor — S4′ (a retry, then a part that plays) is missing; no Session produces RETRIED end to end

- test-plan-v3.md:175 lists `c7_rehearsal_a_host_stall_in_the_recovery_send_gives_one_retry_and_the_next_part_plays`. It holds `Start { nth: 1, 40 ms }` and then `Start { nth: 2, 12 ms }`, with `rows == 2`, class `RETRIED(1)`, and the retry line showing `held_ns = 12 ms`. That entry is in the plan's C7-31 row.
- The test is not in the tree. I searched `tests/` for `host_stall_in_the_recovery` and `nth: 2`; the only hits are probe unit tests. The decisions do not drop it.
- Without it, the heard-with-retries column (`starts[m]`, and StartedEarly pairing `starts[j]` with `rows[j-1]`) is checked only on hand-built fixtures, never against the Module's real send sequence with a retry. S4's Session B covers the silent and none columns with real rows; nothing covers RETRIED.
- `TxHold::Start { nth: 2 }` already exists (`probe_device.rs:763`), so the test is cheap. Add it, or record why it is dropped.

### 9. Note — the R oracle on the X310 cannot see a one-period-off R

- With the X310's 0.05–0.15 ms bracket, `[lo, hi]` allows 2–4 lattice points of 100 µs each (the pure test `c7_the_resume_interval_spans_a_boundary…` reaches "four points").
- So a Module bug that resumes one period late is CLEAN on hardware. Only the fake's rehearsals pin R exactly, with their small bracket and Aligned stalls.
- This is inherent, and `allowed` is printed. The bench summary should say "R within `allowed` lattice points", not "R exact".

### 10. Note — the 15° carrier-phase gate in late-start trials rests on the cross-burst phase

- The phase reference is the control, which is another device burst (`c7_rf.rs:281` and the W0 check). Opus v3 finding 5 moved this premise out of the starved evidence gate but kept it in the final part's check (`opus-recheck-v3.md:152-156`). That follows the decisions.
- If the X310's carrier phase does not hold across device bursts (INFERRED), every heard late-start trial becomes `PhaseDiscontinuous`. The sign is a uniform `phase_vs_control_deg` across a cell.
- Starved trials have `phase_vs_prestall_deg` as a cross-check; late-start trials have none. This is worth one line in the bench's 読み方.

### 11. Note — a final Start cut by the Stop is a datum, not a false FAIL

- `trial` stops 30 ms after the third `start` row (`c7.rs` `trial`). `stop_now` closes the open burst after what was already sent (`tx.rs:1324-1325`).
- So any Start accepted before the Stop has its first chunk played. At 1 MS/s that is 2 500 samples, or 25 periods; at 10 MS/s it is 20 000 samples, exactly 20 periods, with no margin. `read_part` needs W0 plus 19 steady periods.
- A row whose Start was not yet sent lands in the "none" column with a negative tail: `UNSETTLED(m−1)`, or `ended before R + 5 ms` when m = 1.
- A Start dropped by the drain lands in the "silent" column with a negative tail: `UNSETTLED(m)`, provided its TimeError is recorded.
- What remains is the general report-loss case, `NoResume`, which is not specific to the Stop.

### 12. Note — other observations

- A retried part whose drain ends mid-part could radiate its tail inside the gap. In an m = 0.2 ms cell that reads as `SilenceViolated`, a contract FAIL by the grid. The `C7 start … played=` lines let the owner tell this apart.
- One INCONCLUSIVE in 100 trials fails the hardware test, as `hardware.rs` and bench.md state. `run_bench` finishes every cell first, so no data is lost.
- The brief says "no capture on the fake" for S8, but `storm_trial` captures 600 ms on the fake as well. This matches test-plan-v3's S8 and is harmless.
- S4 matches the plan's hand numbers without tuning: the first row at busy + 50 µs ± 2, the later rows at 19 600 / 19 960 ± 400 ticks (98 / 99.8 µs), C is CLEAN, B is `UNSETTLED(k ≥ 2)`. B uses an Aligned −20 µs stall at m = 30 µs (R = busy + 50 µs), not the plan's calibration Session. That is within the plan's margins: below the fake's limit at busy + 70 µs (1 MS/s) or busy + 60.5 µs (10 MS/s).

### 13. Note — S2″'s lead was changed after a run disagreed

- B3-1 stopped and reported correctly: with a 9.9 ms lead, case 2 was m-bound in 4 of 5 runs (`ds-b3-1.log:2050-2058`), because the Module hands the send over about 1 ms after the booking.
- B3-2 then changed the scenario to an 11 ms lead (`fake.rs` S2″ cases; `ds-b3-2.log:441-443`). The new lead is past the 10 ms window, so the first send is the window's first few samples.
- This changes the scenario, not an expectation, and the reason is stated in the test's comment. The orchestrator should still confirm:
  - that it was reported and accepted, given the brief's "stop and report, never tune";
  - that the 11 ms case passed ×20 under load, since it is timing-dependent;
  - that C7-94 is now killed reliably.

### 14. Note — maintainability (AGENTS §9; none blocks)

- `c7_cells::report_tick` (`c7_cells.rs:124`) restates `c7_verdict::is_drop` (`c7_verdict.rs:148`).
- The closure for a row's accepted samples is repeated in `c7.rs` `open` (three inline filters), in `c7_verdict::evidence`, in `Common::accepted` and inline in `fake.rs`. Opus 8 said to fix it where cheap: make `Common::accepted` `pub` and use it everywhere.
- `c7_cells::datum` has an unused `_class` parameter (`c7_cells.rs:218`).
- `datum` reruns `trial.expected` for every row, and `analyse_control` and `read_part` for every Start, all of which `evidence` and `verdict` already computed. The two copies can drift apart.
- The storm's failures share the cell trials' `Fail` enum, and `run_storm` matches `Retried` and `Unsettled` arms that `storm_verdict` cannot return (`c7_cells.rs:537-538`).

## Verdict

Two majors.

- **Finding 1** departs from a binding B2 disposition (Opus 5). With the recorded X310 brackets, there is a concrete sequence that turns a correct Module into a false INCONCLUSIVE or a false `ResumeOffRule`.
- **Finding 2**: the hardware storm runs 60 s against a 20 s rehearsal, and measured at 60 s on the fake it overflows the probe's record by 9 250 rows. The storm verdict is then INCONCLUSIVE.

Both are small changes to the test tool; neither touches production code.

Verdict: FAIL


## terra-b3

1. **Major — the R oracle can pass a resume one or more periods off.** `c7.rs:163`, `c7.rs:417`

   Sequence: run an X310 trial with a 0.15 ms anchor bracket and a 0.1 ms lattice period; the nominal R is a lattice point L, but the implementation resumes at L + one period. The uncertainty interval admits four points, including L + one period, and `check` returns success. The verdict can therefore be CLEAN despite the wrong R. The test at `c7.rs:717` explicitly demonstrates this interval; `pinned()` is never used by the verdict. The binding disposition calls for one or two allowed points and a pinned gate. When the evidence cannot distinguish the answers, the trial needs to say so rather than certify the rule.

2. **Major — an `expired` first incident requires a separate TimeError report.** `c7_verdict.rs:241`

   Sequence: hold the initial Start past its time; the Module records `expired`, resumes correctly, and the resumed PN is heard, but the device’s TimeError report is lost. The verdict returns INCONCLUSIVE “no attributable late-start evidence,” and the hardware cell fails. The binding plan accepts the `expired` row itself as late-start evidence, and report delivery can be sparse. The fake rehearsals do not cover this hardware case.

3. **Major — S4 does not establish both sides of the fake drain boundary within one Session.** `fake.rs:3009`, `fake.rs:3057`

   Sequence: run the below-boundary case, then the above-boundary case. Each call creates a fresh trial and Session; the test checks the resulting R and observed class, but does not calculate and compare both starts against that Session’s actual drain limit. A change to the fake’s limit can leave these observations passing without proving the intended boundary. This misses the binding B1 disposition, which specifically moved S4 away from cross-Session timing assumptions.

4. **Major — retry evidence needed to interpret the margin sweep is discarded.** `c7_cells.rs:325`, `c7_cells.rs:371`

   Sequence: a trial produces seven or more retries. The output retains only the first five and last retry and Start lines, and the retry line does not contain the surrounding report ticks or send returns. The owner cannot inspect the raw evidence for each retry from the requested bench log, although the binding decision requires it and uses retry rates across margins to choose m. The trial summary also totals retries only for `RETRIED` trials, excluding retries in unsettled trials (`c7_cells.rs:404`).

5. **Minor — the C7 table still promises one event per storm cycle.** `bench.md:1589`

   The later storm instructions correctly allow sparse events and an initial `expired` incident, but the table says each cycle has one `TIME_ERROR { late_at_device }`. That conflicts with the approved hardware expectation and could lead the bench operator to judge a valid result incorrectly.

This was a read-only review; I did not run tests, mutation rows, or hardware.

Verdict: FAIL

## sol-code-final

Independent read-only review of `work/bt2`, `715ebd67..500726dd`. The Rust builder reaches the TX resume calculation through the bench Session and records the override in that Run’s Manifest. The blockers are in how the bench schedules trials, interprets the evidence, and presents the commands.

1. **Major. The trial can stop a successful retry before the hardware verdict can score it.** Let the first recovery Start be accepted and dropped. Let the next Start play correctly 15 ms later. The runner still stops 30 ms after observing the first recovery Start, leaving the final part about 15 ms. With the hardware’s 1 ms PN, the verdict requires 20 ms and returns `INCONCLUSIVE`, failing the cell. S4′ uses a 100 µs PN and needs only 2 ms, so it misses this boundary. Base the observation window on the latest accepted Start’s device tick, retain the trial deadline, and rehearse the retry at 1 ms. `c7.rs:663` · `c7_verdict.rs:303` · `fake.rs:3202`

2. **Major. The R oracle uses the requested lattice after UHD moves the initial Start.** Book a late-start trial, then delay dispatch until `start_repeat` takes its `SendAsapAndFlag` path and moves the initial Start by a fraction of a PN period. Let the hold fire and the Module recover correctly. The Module rounds R on the moved booking’s lattice, while the oracle and RF scan retain the requested origin. The correct resume can fail `ResumeOffRule` or `LatticeOff`. Use the first accepted Start as the observed origin, or mark a moved initial booking `INCONCLUSIVE` with both ticks. `radio lib.rs:221` · `tx.rs:1266` · `c7.rs:633` · `c7.rs:497`

3. **Major. Clock uncertainty at the decision can understate uncertainty in the sends that set `busy`.** Let a held send return while the Authority uses an accepted 0.8 ms clock-read bracket. Let a narrow read replace that anchor before the close and resume decision. The send’s timestamp can still dominate `busy`, but the oracle budgets only the last qualifying read’s half-width at the decision. Near a lattice boundary this can exclude the correct R by one period. The probe stamp also precedes the Module’s post-send bookkeeping and `Core::now()` sample. Bound the timestamps that actually contribute to `busy`, or report R unresolved when that bound cannot be established. `authority.rs:118` · `tx.rs:824` · `probe_device.rs:588` · `c7.rs:281`

4. **Major. The RF verdict can pass a gap that carried RF.** In a 1 ms trial, radiate three periods of the PN a quarter-period off the expected lattice during the gap, then resume correctly. The existing detector fixture shows that this signal is read as silence, so the trial can print `CLEAN`. There is a second path to a false pass: let an intermediate part radiate, then drop again and stop shortly after the next planned R without another accepted Start. The verdict takes the `UNSETTLED` branch without scanning the gap, and the cell prints `PASS (contract)`. Check the claimed silent interval independently of PN alignment and final-Start status. Keep lattice-aligned correlation for the phase verdict. `c7_rf.rs:320` · `c7_rf.rs:798` · `c7_verdict.rs:282` · `c7_verdict.rs:381`

5. **Major. The storm can pass with a truncated capture.** Run past 1,000 resumes, but supply a capture that contains the control and remains contiguous only through 250 ms. The verdict checks coverage through the last sample it received, so the missing tail through 600 ms disappears from the test and the storm can print `PASS (contract)`. The Sink can return a partial artifact at Stop, and the C7 reader accepts a partial artifact with valid spans. Check the scheduled capture endpoint and classify a short or unreadable capture as `INCONCLUSIVE`. `c7_storm.rs:81` · `c7_storm.rs:104` · `c7_verdict.rs:426` · `sink-capture lib.rs:468`

6. **Major. The storm takes its expected bound from the Manifest it checks.** With 1,200 resumes, supply a Manifest declaring a bound of 500, with 500 kept and 700 omitted. The hardware verdict reports `CLEAN`; the fake rehearsal separately requires 1,000 and fails on the same evidence. Give both paths the expected default independently. Also check that the kept rows are the first rows: the current counts and sparse-event check can accept a substituted row when no event names it. `c7_storm.rs:97` · `c7_storm.rs:124` · `fake.rs:3257`

7. **Major. The log drops the first effective margin needed to interpret retries.** Take two trials in the overlapping 0.2 ms and 1 ms cells, one clean and one retried. The log cannot associate either trial’s first `R − busy` with its outcome. `C7 trial` omits it, the first `C7 retry` line prints the second row’s value, and the cell summary pools the values. Print `R − busy` for row 1 on every trial so the owner can compare retry outcomes against the effective margin as agreed. `decisions.md:98` · `c7_cells.rs:368` · `c7_cells.rs:421` · `c7_cells.rs:523`

8. **Major. The copyable bench block can transmit on both boards after a single confirmation.** Paste the C7 block after the owner checks board 36 for the first command. Bash treats the remaining confirmation comments as comments and proceeds through the other transmitting commands, including board 37. A failed `hw` call also leaves the next command runnable. Put each transmit invocation in its own copyable block with its board-specific confirmation immediately above it. `bench.md:1715` · `bench.md:1719`

9. **Minor. Sparse reports can attach a later incident’s timestamp to the first delay measurement.** If the first drop’s `TimeError` is missing and only a retry reports one, `delta_first_us` subtracts the original held send’s return from that later report. The value is diagnostic, but the first-incident label is misleading. Leave it missing when attribution is ambiguous, or label it as the first observed report after the original hold. The report greps also use the unsuffixed filename after a rerun, even though `hw` writes `.2.log` and later suffixes. `c7_cells.rs:283` · `bench.md:821` · `bench.md:1726`

10. **Minor. The tool can reuse its analysis and carry less parallel state.** The verdict and formatter separately fit the same control and read the same final part. The formatter also reconstructs send history twice per resume and stores the results in parallel vectors. Compute those facts once and keep each resume’s evidence together. Use the probe’s existing `with_rows` accessor for the polling loop, which currently clones the growing JSON trace under the writer lock. `C7-76` now exercises a fake-only report-window helper. Keep the fake behavior assertion with its caller and drop the standalone helper and row when folding it in. `c7_verdict.rs:205` · `c7_cells.rs:241` · `c7.rs:666` · `probe_device.rs:256` · `mutations.json:29660`

11. **Note. Mutation and command verification.** All 97 added or changed anchors matched exactly once at HEAD. The recorded fix run reports all 96 C7 rows killed, with zero timeout kills. The earlier full run reports `phase7/G09` surviving. The three documented C7 test names resolve, and the shell block passes `bash -n`. Rust and X310 behavior were reviewed statically. `rows-fix.log:198` · `full.log:6731` · `hardware.rs:4276`

Verdict: FAIL

## opus-recheck-final

# bt2 — final re-check (Opus, read-only): fix round `500726dd..d26e8c14` on top of the step `715ebd67..d26e8c14`

Reviewer: Claude Opus (re-check reviewer, AGENTS §9 step 8). Read-only. Nothing built or run; the tree, the flake logs in `/tmp/ezsdr_x20*.{sh,log}` and the bt2 records were read.

## Step 1 — my own failure scenarios (written before reading Sol's review and the fix brief)

1. **The R oracle is too tight on the X310.** Each send that sets `busy` carries the Module's own stamp error: the Authority's anchor read half-width (≤ 0.5 ms), the delay between the probe's `host_after_ns` and the Module's `Core::now()`, and the `last.fetch_max` monotonic clamp in `DeviceTime::extrapolate` (`authority.rs:136-142`), which keeps stamps at a previous anchor's value after a narrower read lands behind it. If the oracle budgets less than that, a correct R near a lattice boundary is refused (a false `ResumeOffRule`).
2. **The R oracle is too loose.** If "R unresolved" happens often, the R check decides nothing and the cell still prints `PASS (contract)`, so a Module that rounds on the wrong `busy` passes.
3. **The lattice origin is wrong.** UHD's `SendAsapAndFlag` can move the booked start. If the oracle uses the booking while the Module rounds on the moved start, the trial can get a false `LatticeOff` or `ResumeOffRule`.
4. **The observation window ends too early.** A retried part on the 1 ms PN needs 20 periods (20 ms). A window keyed to the first resumed Start (or to host time) stops before then, and a correct RETRIED becomes INCONCLUSIVE, which fails the cell.
5. **Silence is read only on the lattice.** A gap leak a quarter period off the lattice reads at the PN's off-peak floor, so the trial prints a false CLEAN. A second path: an UNSETTLED exit that never scans the gap passes a part that radiated before it was dropped again.
6. **The power test gives a false positive on hardware.** In a starved gap the ATR may stay in the TX state, so LO leakage is present that the TX-idle quiet floor did not see. There is also the filter ring-down at `from`.
7. **The storm is judged against its own Manifest.** It takes the record bound the Manifest declares, it accepts a short capture, and it never checks that the kept rows are the first ones.
8. **The bench.md blocks are unsafe.** A block runs several transmits after one confirmation, the confirmation is not tied to a board, or the log grep reads `.log` when a rerun wrote `.2.log`.
9. **The rehearsals depend on the host.** The manual clock (`host.rs` M1–M5) stops only for the `uhd-*` participants. The Kernel's capture sink, the data link and the test thread are not participants, so their lag under load changes what the fake records.
10. **A test assertion observes nothing.** It compares a test-local helper with itself.

## Step 2 — Sol's findings F1–F10

| # | Status | Evidence |
|---|---|---|
| F1 (window cuts a successful retry) | **CLOSED** | `c7.rs:719-733`: the window follows `max` of the first `MAX_FOLLOWED_STARTS` (9) accepted Start `at` ticks after the control (`start_ticks`, `c7.rs:151-157`). It is device time, not the host instant the probe first saw a row, and it keeps the `c0 + 300 ms` deadline. The 1 ms rehearsal `fake.rs:3243-3248` (m = 25 ms, so the old window would end early) and rows C7-240 and C7-246 are both killed (`rows-final.log`). |
| F2 (oracle uses the requested lattice) | **CLOSED** | `Common::origin` (`c7.rs:466-468`) is the first accepted payload Start after the control. It is used by `expected_from` (`c7.rs:552`), `check` (`c7.rs:561`), `evidence` (`c7_verdict.rs:535`) and the storm's `lattice_tick` (`c7_storm.rs:205`). The datum prints `start_moved_samples` (`c7_cells.rs:376`). The fixture is `fake.rs:3250-3268`; rows C7-241…245 are killed. |
| F3 (decision-time read understates the stamps that set `busy`) | **CLOSED** (minor residue, N1) | Per-send `err` = probe uncertainty + the half-width of the last accepted read before that return + the offset to the next send's entry, or to the decision for the paired close (`c7.rs:376-383`). `busy` is replayed at `returned ± err` (`c7.rs:135-148`); this is correct because the fold is monotone in each return. `unresolved` is the whole width > 1 period (`c7.rs:223`). Sol's wide-then-narrow scenario is the fixture `c7.rs:1045-1067`; a wide read gives an unresolved interval (`c7.rs:1071-1084`). Rows C7-250…253 are killed. |
| F4 (gap RF read only on the lattice; UNSETTLED not scanned) | **CLOSED** | `quiet` (`c7_rf.rs:353-373`) uses n/4 power windows independent of the PN phase, with threshold `tp = max(4·p_q, 0.1·p_c)` (`c7_rf.rs:236`). The verdict runs it after `silence` (`c7_verdict.rs:322-339`). Both UNSETTLED exits now scan up to `stop_tick + d` (`c7_verdict.rs:316-320`). Fixtures: `c7_verdict.rs` tests at the hardware period (off-lattice leak), "an intermediate part that radiated before an unsettled end" and "a gap leak before a silent dropped-again final Start". Rows C7-260…264 are killed. |
| F5 (storm passes a truncated capture) | **CLOSED** | `c7_storm.rs:89-98`: `scheduled_end` = the capture's first sample + (scheduled − 1)·per. A short capture or a hole is INCONCLUSIVE. Fixtures at `c7_storm.rs:364-391`, including the one-sample tolerance. Rows C7-270, 271 and 273 are killed. |
| F6 (bound taken from the Manifest; first rows not checked) | **CLOSED** | `DEFAULT_BOUND` = 1 000 restated (`c7_storm.rs:18`), with `bound`/`rows_kept`/`omitted` checked against it (`c7_storm.rs:110-115`). `kept_rows_are_the_first` (`c7_storm.rs:152-169`) pairs kept row i with Start i+1, which is correct in a storm where every Start is at its R. Fixtures at `c7_storm.rs:420-437`; rows C7-272, 274 and 275 are killed. |
| F7 (row 1's R − busy missing) | **CLOSED** | `Datum::first_r_minus_busy_us` (`c7_cells.rs:110-113`) is printed on every `C7 trial` line (`c7_cells.rs:330, 348`) and in the cell line's `r_minus_busy_first_us`. Row C7-280 is killed. |
| F8 (one confirmation, many transmits) | **CLOSED** | `bench.md:1717-1772`: six blocks, each one `export … && hw <test>` with that board's attenuator/PN confirmation comment directly above it. The extraction blocks are marked "送信しない" and contain no `hw`. `grep -n hw_c7 plan/phase8/*.md` finds no other transmit invocation. |
| F9 (`delta_first` attribution; log suffix) | **CLOSED** | `first_time_error(…, at, before = R1)` (`c7_cells.rs:265-270, 312-317`) gives `-` when only a retry reported. The greps take the newest log with `ls -t "$d"/$t.$BOARD*.log \| head -1` (`bench.md:1748-1749, 1777-1778`), which matches `hw()`'s `.N.log` naming (`bench.md:821-823`). Rows C7-282 and C7-283 are killed. |
| F10 (duplicate analysis, parallel state, C7-76) | **CLOSED** | `Readings` is computed once and passed to `verdict_with` and `datum` (`c7_verdict.rs:175-200`, `c7_cells.rs:272`, `c7_cells.rs:577-580`). Each resume is one `RowDatum` (`c7_cells.rs:65-70`). The polling loop uses `probe.with_rows` (`c7.rs:723`). `starvation_reported` and its row are gone: `mutations.json` has no C7-76, and the closure is inline in `fake.rs:2939-2945`. |

Mutation runs: `rows-final.log` shows 120/120 C7 rows killed, every outcome `killed` with `reruns: 0` (since note 51 stage M a timeout or bound would be ERROR, not `killed`). No file the rows touch changed after the run started (mtimes ≤ 09:19, run at 09:18). No killer row filters on a 10 MS/s rehearsal that flaked: the four 10 MS/s or hardware-period filters are pure or synth tests, plus the 1 MS/s retry, which did not flake.

## New problems in the fix round, and residue

**N1 (minor). The per-send read half-width can still understate the Module's stamp error.** I traced two paths in the code. Their size on the X310 is INFERRED to be small: reads are about 0.1 ms, and the clamp and the look-ahead need a wide read next to the held return.
- (a) `read_half_ticks` (`c7.rs:318-326`) chooses the last read whose `after` ≤ the send's return + the anchor's worst bracket. A narrow read that completed inside that look-ahead, but after the Module stamped the return, replaces the wide read that was actually in effect.
- (b) The Authority's monotonic clamp, `self.last.fetch_max(t)` (`authority.rs:140-141`), keeps stamps at the previous (wide, ahead) anchor's extrapolation for up to that anchor's error after a narrower read lands behind it. The oracle budgets only the narrow read's half.

Minimal sequence (pseudocode): accepted read A, width 0.9 ms, extrapolating +0.4 ms → read B, width 20 µs, lands behind → the held send returns 0.1 ms after B, and the Module stamps max(last, t) ≈ +0.3 ms → the oracle's err for that send = 10 µs + probe uncertainty → with `busy + m` 0.15 ms below a lattice point, the correct R (the next point) lies outside `[lo, hi]` → a false `ResumeOffRule` FAIL.

Fix: take the widest accepted read completing between the previous read before the return and return + offset + slack. Bounding the clamp exactly needs the read preceding that one as well. It is a FAIL that fails loud (printed with its terms), so minor.

**N2 (minor). The bench.md storm text disagrees with the code on coverage.** `bench.md:1798` says the capture must hold the scheduled samples "from its own first sample" in one contiguous run. `c7_storm.rs:89-98` checks contiguity only over `[held_first_at, scheduled_end)`; the control region is covered only by `analyse_control`'s own hole checks. The behaviour is fine; the text should say "from the first drop to the scheduled end".

**N3 (minor). A capture hole does not name its cause.** The `rec` feed is a `drop_oldest` link of capacity 64, in the fake profile and the bench profile alike (`common/mod.rs:297, 324`; `bench_profile` calls `profile`, `common/mod.rs:265-267`). A host-side eviction (SC-20b, `ezsdr-kernel/src/stream/link.rs:67-73`) reaches the verdict only as "hole in the coverage", "the control has a hole" or "hole in the capture" (INCONCLUSIVE). The Manifest's link-drop count (`coordinator/ending.rs:119-121, 421`) is not read. On the shared bench host this costs a rerun with no stated cause; it fails safe. Suggested: add the `rec` link's drop count as its own INCONCLUSIVE reason ("the capture link dropped N blocks").

**N4 (note). Half of the power threshold is untested.** `(4.0 * p_q).max(0.1 * p_c)` has no mutation row, and no fixture makes the `4·p_q` term bind (`c7_rf_quiet_threshold_both_sides` uses a floor of 1e-5, so p_q ≈ 1e-10). The noise-floor side of the threshold is unpinned. The formula predates the fix round, but the round made it a verdict threshold. By the bench's SNR numbers (`bench.md:1714`), `0.1·p_c` should bind on the X310 (INFERRED).

**N5 (note). Wording.** `MAX_FOLLOWED_STARTS` is documented as "the cell's first Start and its first 8 retries" (`c7.rs:35`, `bench.md:1711`). In the retry lines' own terms, Start 2 is the first resume and retries begin at row 2, so 9 Starts are the first Start, the resume and 7 retries. Separately, `r_check`'s doc says "uncertainty exceeds half a period" (`c7.rs:228`), while the code uses the whole width `busy_hi − busy_lo > period·per` (`c7.rs:223`). These agree only when the bounds are symmetric, which the replay need not give.

**N6 (note). Two assertions check only the test's own closure.** `c7_starved_incident`'s new edge assertions (`fake.rs:2941-2945`) check the closure `reported`, not the Module. `!reported(latest + bracket + 1)` can fail only if the closure's lower edge changes. This is acceptable as the replacement for C7-76's helper test, but the comment overstates it as "the window's own edges at the real ticks".

**N7 (minor, latent flake). The pinned assertions depend on read widths under load.** The hardware-period rehearsals assert `!terms.unresolved` and `allowed == 1` (`fake.rs:2900-2903, 2951-2954`). Per-send `err` includes the half-width of the Module's last read before the held send's return. On the manual clock, the widths of the nap reads follow real time (`authority.rs:124-131`; decisions.md: 35–86 µs measured under load), and up to 1 ms is accepted. A nap read wider than about 0.55 ms just before the held return makes `allowed == 2`, and the test fails, although the Module's anchor on the manual clock is exact. This was not observed in 45 loaded runs. The dependence existed at `500726dd` (the half-width at the decision); the fix round moved it to the per-send read.

## Step 3 — the flakes

Evidence read:
- `/tmp/ezsdr_x20.log`, run 8 of 20: both 10 MS/s tests failed. The script keeps only `head -3` of the FAILED/failed lines, so only the drain rehearsal's message survived: "drain at 10000000 S/s, below L: nothing radiated after the first drop". **The message of `c7_rehearsal_a_starved_repeat_resumes_on_its_lattice_after_busy_plus_0_2_ms_at_10_msps` was not recorded.**
- `/tmp/ezsdr_x20c.fail.10`: `c7_rehearsal_a_starved_repeat_at_the_hardware_period_resumes_on_its_lattice_at_10_msps` → `Inconclusive(["the control has a hole"])` at `fake.rs:2965`.
- `/tmp/ezsdr_x20b.log`: the two run-8 tests alone, 60 runs, no failure.

**Can each assertion depend on host scheduling? Yes. VERIFIED: the mechanism exists in the code. INFERRED: that it is what fired, because the failing Manifests, and with them their link-drop counts, were not kept.**

- **Mechanism.** The capture sink consumes the `rec` feed through a `drop_oldest` link of capacity 64 (`common/mod.rs:297`). The sink and the link run on Kernel threads that are not manual-clock participants: the manual clock stops only for `uhd-rx`, `uhd-tx`, `uhd-control` and `uhd-clock` (`host.rs:1-21`, M1–M3). While every participant is idle, the clock runs at real speed (M5), so the fake keeps producing RX blocks against clock time. If the sink thread is descheduled for longer than 64 blocks last, the link evicts the oldest (`link.rs` DropOldest) and the `rec` continuity map gets a gap. At 10 MS/s, 64 blocks last a tenth as long as at 1 MS/s, which matches failures at 10 MS/s only.
- **Sequence for the drain rehearsal.** The control and the cell repeat play. While the 16 `yes` processes hold the cores, the sink thread is preempted for more than 64 blocks' worth of clock time, and the link evicts blocks between `held.at + d − n·per` and `min(capture.end, stop_tick + d)`. `silence` → `covers` is false → `Err("hole in the coverage")` → `assert_eq!(…, Ok(None))` fails at `fake.rs:3185` with "nothing radiated after the first drop". A heard part (`Ok(Some)`) is unlikely: the per-row drain-limit assertions just before it (`fake.rs:3168-3171`) passed in the same run, and the drop-or-play decision is a pure function of manual-clock stamps.
- **Sequence for the hardware-period starved test at 10 MS/s.** An eviction lands inside the control's analysis span (`control_at + k·per + p·span` for the lag search, `control_at + d + 1..19 periods`, `c7_rf.rs:219, 233, 257, 265`) → `analyse_control` returns `Err("the control has a hole")` → the verdict is INCONCLUSIVE → `assert_eq!(class, Clean)` fails. No `RX_OVERFLOW` reason appeared, which is consistent with a link eviction, not a device overflow.
- **The 0.2 ms starved test at 10 MS/s.** Its message is unknown, so I cannot tie it to the mechanism. Same run, same rate: INFERRED to be the same mechanism.
- **Other host-scheduling dependences in the same tests.** They are pre-existing and were not seen failing. The test thread is not a participant either. Between `to(...)`/`now_root_of` and `run.submit` (`c7.rs:637-649, 687-716`), the clock runs while the test thread is preempted. That shrinks the 2 ms capture lead and the 6.5 ms start lead, so a capture can start late, or a first send can be short, which trips `accepted == cap` (`fake.rs:2883, 3130`). There is also N7 above.
- **Did the fix round introduce it? No.** The feed policy and capacity, the non-participant sink, and the coverage errors in `silence` and `analyse_control` all predate `500726dd..d26e8c14`, as do both 10 MS/s rehearsals; the fix round's `fake.rs` diff changes only the drain rehearsal's lattice tick (`start` → `origin()`). The fix round's `quiet` is one more coverage-dependent reader, but its range lies inside the range `silence` already checks. F1 removed a scheduling dependence: the window used to start at the host instant the probe first saw the third Start; it now follows device ticks.
- **Severity: minor for this step.** It fails safe (INCONCLUSIVE or hole, never a false CLEAN), and no mutation row is killed by a flaky test. It is still a default-suite flake that the gate can hit under load. Two lean fixes: a larger `rec` feed capacity for the C7 rehearsals' profile (the 10 MS/s 1 ms-PN capture), or make the verdict name link drops (N3) so a host-load hole is told apart from a device gap. A third run of the loaded loop that keeps the Manifest's `link_drops` on failure would confirm the mechanism. The orchestrator decides.

## Summary

All ten of Sol's findings are closed. I found no major in the fix round: no sequence that makes C7 print CLEAN, DROPPED_AGAIN/UNSETTLED or an R verdict that is wrong on an X310 with a 1 ms PN and is reasonably likely. N1 is the one path to a false FAIL; it is minor and fails loud. The three loaded-run flakes come from a host-scheduling dependence that predates the fix round: the `rec` feed's capacity-64 `drop_oldest` link in front of a sink that is not a participant. They fail safe. Recommended follow-ups: N1, N3 or a larger capacity for the flake, and the N2 text.

Verdict: PASS


## terra-recheck-final

1. **Major — F3 PARTLY CLOSED: the per-send clock error can still be understated.** The probe brackets only the inner `time_now` call, then records the row before returning; the Authority brackets the entire probe call, including that recording work (`probe_device.rs:416`, `authority.rs:99`). The oracle uses the narrower probe width (`c7.rs:301`). It can also select a narrow read that completed *after* a send because it adds mapping slack and takes the last eligible read (`c7.rs:315`, `c7.rs:376`).

   Minimal sequence: an accepted wide Authority read is in effect when the send setting `busy` returns; a narrow read finishes within the added slack immediately afterward. Near a 1 ms boundary, the oracle can exclude the correct R and report `ResumeOffRule`. The lower/upper replay itself is monotone, but its input intervals need a defensible bound.

2. **Major — an unresolved R accepts even a point outside its computed bounds.** `expected_r` calculates `lo` and `hi`, but `r_check` returns success for *any* lattice point when the busy interval exceeds one period (`c7.rs:212`, `c7.rs:230`). The fixture explicitly expects acceptance one period outside the interval (`c7.rs:937`).

   Minimal sequence: a wide stamp interval permits R from 150 to 152 ms; a broken Module resumes at 200 ms, on the lattice, and plays correctly. The row passes the oracle and the trial can print `CLEAN`. Uncertainty may prevent choosing *which* point inside the interval is correct; it does not make points outside a valid interval possible.

3. **Major — F6 PARTLY CLOSED: the storm’s “first rows” check accepts substituted values.** The independent 1,000-row bound is now checked, but the new check only places each retained R between adjacent accepted Starts (`c7_storm.rs:112`, `c7_storm.rs:152`). Its fixture substitutes a point from a later interval, which that check detects (`c7_storm.rs:431`).

   Minimal sequence: replace a retained row’s R with another value within its permitted interval, and omit the sparse event that would identify that row. Counts and ordering still pass, so the storm can report `CLEAN` with an altered retained row. The check establishes ordering, not that the recorded rows are the actual first rows.

4. **Major — the reported load flake reflects scheduling-sensitive rehearsals.** The fake’s manual clock advances at real speed while participants are idle, and a non-participant test thread does not stop it (`host.rs:14`, `host.rs:121`). The trial schedules capture, control, stop, and holds through separate threads (`c7.rs:638`, `c7.rs:718`); fake RX constructs samples from TX history when it receives them (`device.rs:1304`, `device.rs:1875`).

   - For **“nothing radiated after the first drop,”** the drain assertion depends on the actual sequence of accepted packets and timing, not solely the intended 20 µs side of the lattice. The fake decides whether a timed packet is still inside its growing drain at delivery (`device.rs:770`, `device.rs:858`). A changed scheduling sequence can let a later part play or change the captured interval, making the silence assertion fail (`fake.rs:3166`, `fake.rs:3180`).
   - For **“the control has a hole,”** a delayed capture path can leave a missing control window; `analyse_control` converts that missing window into the reported error (`c7_rf.rs:215`). The 10 MS/s rehearsal then requires `CLEAN` (`fake.rs:2964`, `fake.rs:3077`).

   These scheduling dependencies predate the fix round; the fix changes the drain scan’s origin and extends the observation window, which can expose a different portion of the run. I cannot identify the exact race in the three observed runs without their capture spans and probe traces. Also, the literal “nothing radiated after the first drop” assertion appears only at `fake.rs:3185`; I cannot attribute that exact message to the separate starved test from the source alone.

5. **Minor — the storm capture check does not establish the documented continuous capture from its first sample.** It checks continuity from the held Start through the scheduled end (`c7_storm.rs:91`). A hole between a successfully analysed control and that Start can escape this check, although the procedure says the scheduled capture must be one continuous run from its first sample (`bench.md:1798`).

6. **Minor — a silent final part needs more capture than its stated coverage check.** `read_part` checks ten periods for a silent first period, then unconditionally reads 19 steady periods (`c7_rf.rs:292`). A capture with the promised ten periods but no later samples returns a coverage error instead of the otherwise applicable silent-part verdict. The verdict fixtures use longer synthetic captures, so they do not expose that boundary (`c7_verdict.rs:877`).

7. **Review disposition for the remaining Sol findings:**

   - **F1 CLOSED for the reported retry:** the window follows accepted Starts up to nine and a 1 ms-period retry rehearsal was added (`c7.rs:718`, `fake.rs:3243`). The nine-Start cap is a stated limit, not a guarantee that every later part has 20 ms of observation.
   - **F2 CLOSED:** the oracle and RF evidence use the first accepted payload Start; the fixture changes the recorded booking to test the distinction (`c7.rs:463`, `fake.rs:3250`). That fixture does not itself exercise a real moved hardware booking.
   - **F4 CLOSED for the reported scenarios:** the added quarter-period power scan and both unsettled exits are exercised (`c7_rf.rs:353`, `c7_verdict.rs:314`). Its `max(4·p_q, 0.1·p_c)` threshold is an inferred hardware sensitivity limit; weaker or shorter RF can remain below it.
   - **F5 CLOSED for the reported truncated tail**, subject to finding 5 above (`c7_storm.rs:91`).
   - **F7 CLOSED:** each trial prints its first row’s effective margin (`c7_cells.rs:397`).
   - **F8 CLOSED:** each of the six copyable transmit blocks contains one invocation and its board-specific attenuator and PN confirmation (`bench.md:1721`).
   - **F9 CLOSED for the reported cases:** first-report attribution is bounded by the first R, and extraction selects the newest suffixed log (`c7_cells.rs:312`, `bench.md:1744`).
   - **F10 CLOSED for the requested cleanup:** readings are shared, row data are grouped, polling uses `with_rows`, and C7-76 was removed (`c7_cells.rs:603`, `c7.rs:723`).

8. **Note — mutation evidence and limits.** The recorded final mutation run reports **120/120 passed**, including the present C7-240–283 rows, with outcomes recorded as killed (`rows-final.log`). I did not rerun mutations or build. A recorded kill does not establish coverage of the counterexamples above: in particular, the unresolved-R fixture asserts acceptance outside the interval, and the substituted-row fixture moves a row outside its paired interval.

Verdict: FAIL

## opus-recheck-fix2

# bt2 fix round 2 — Opus re-check (read-only)

Reviewed: `git diff d26e8c14 f69e94f6` on `work/bt2` (HEAD f69e94f6, tree clean).

## Step 1 — own failure scenarios (written before reading brief-final-fix2.md, terra-recheck-final.md, opus-recheck-final.md)

S1 (whole-call bracket ⊂ Authority bracket). `DeviceTime::read` takes `before = host.now()` before `device.time_now()` and `after = host.now()` after it
(authority.rs:100-109); the probe stamps `entered` inside the call and `exit` under the rows lock. The probe's width is therefore a lower bound of the
Authority's. A preemption of `uhd-clock` after the probe's exit stamp (unlock, return, `host.now()`) makes the Authority's width W >> probe width w: the Module's
anchor midpoint can be off by ~W/2 while the oracle charges w/2. Wrong outcome: a correct Module's R just outside `[lo, hi]` → FAIL ResumeOffRule on the X310 under
load. Also the opposite: the Authority discards a read (W > 1 ms) the oracle counts as accepted (w ≤ 1 ms), so the oracle picks it as "in effect" while the Module
still extrapolates from an older, possibly wider anchor.

S2 (anchor installation lag). The anchor is installed after `after` and the `anchor` lock (authority.rs:121-126), not at the probe's exit. A read whose exit is
just before `opened` may become effective after the window opened; the earlier read stays in effect, and the oracle does not consider it. Covered while the lag
is under the 0.5 ms slack the 1 ms window keeps over a clamp of ≤ 0.5 ms; a longer preemption is not.

S3 (reads between the return and the Module's stamp). The window ends at the probe's return, but the Module stamps later (`Core::now()` after `note_sent`); a wide
read that lands in between shifts the stamp by its half width, which neither the read term nor the delay term (an offset, not a read error) carries.

S4 (other `time_now` callers). Any Module `Device::time_now` call that is not an anchor read (purpose `device_call`) would be taken as "the read in effect" and could
replace the real, wider anchor in the oracle's choice; only `DeviceTime::read` and `DeviceAuthority::new` (UR-8) call it in production (authority.rs:102, :373).

S5 (manual clock on the fake). `reanchor` installs only zero-width reads on a manual clock (authority.rs:124); the oracle counts every ≤ 1 ms read. On the fake the
oracle is therefore conservative, so the rehearsals cannot show an under-count — the hardware is the first place S1–S3 can bite.

S6 (r_check unresolved inside [lo, hi]). Before: an unresolved row passed any lattice point. Now an unresolved row whose error budget misses a term (S1–S3, the
device front left out of `expected_r`) refuses a correct Module where before it was data. Conversely, a mutation that re-admits "any lattice point when
unresolved" must die on a test that builds an unresolved interval and a lattice point outside it.

S7 (storm row checks). Each kept row pairs with close number `armed_closes + i + 1`. A close with `accepted_samples` 0 on hardware shifts every later pairing by one:
later rows get one extra part in `busy` → R below `lo` → FAIL ResumeOffRule, not INCONCLUSIVE. A storm with one ambiguous row and one off-rule row is INCONCLUSIVE
(reasons first): acceptable by the spec's order. A storm whose margin is not the Module's default (`DEFAULT_MARGIN_NS` must equal `profile::RESUME_MARGIN_NS`).

S8 (capture link 1024). The profile change is applied in `open` only through `with_capture_capacity`; if the bench profile's capture binding is not `rec` or has
no `feed`, the bench silently keeps 64 blocks. A test that checks only the helper does not show that a C7 Session runs with 1024. Memory: 1024 × 2000 × 8 B
= 16.4 MB per Session, only if the receive block stays 2000 samples (control.rs:203 takes `max(block_len, rx_packet_samples)`).

S9 (link drop reason). `link_drops` sums every Manifest link, not the capture link; a drop on another link is printed as "the capture link dropped". A `None`
count (cleanup never read it) counts 0, leaving only the hole message.

S10 (storm continuity from the first sample). If `capture.start()` is the first *valid* sample and the capture began late (after the held Start), coverage from its
own start passes while the region from the held Start is uncovered; the silence scan's own coverage check (`silence`, c7_rf.rs) must then turn it INCONCLUSIVE.

S11 (read_part 1..=9 silent). A silent first period's steady flags over 1..=9 are meaningless (silent periods have gain ~0); a verdict branch that reads
`steady_ok` / `worst_gain_ratio` of a silent part would misclassify a late restart. Periods 10..=19 of a late-heard part are no longer read.

S12 (rehearsals under load). The pinned rehearsals' "exactly one allowed point" assertions were dropped; the stall now aims 300 µs off the lattice point. A loaded
host that widens a read by more than 300 µs can still move R across the boundary; the new assertion (`R in lo..=hi`) holds by construction when the oracle is
right, so it fails only if the oracle under-counts.

## Step 2 — the eight dispositions (brief-final-fix2.md)

| # | Status | Evidence |
|---|---|---|
| 1 clock bracket (Terra 1 = Opus N1) | **CLOSED** (minor residue, finding 1) | The probe stamps `entered` before the control-log push (`probe_device.rs:499`) and `host_exit_ns` under the rows lock (`probe_device.rs:407-444`); `module_reads` takes `exit - entered` ≤ 1 ms (`c7.rs:312-320`). `read_half_ticks` (`c7.rs:329-334`) = max over the read in effect when `[return - 1 ms, return]` opened and the reads that completed inside it; nothing after the return. The clamp argument holds: `last.fetch_max` (`authority.rs:144`) keeps a stamp ahead by at most the superseded read's error (≤ its half width ≤ 0.5 ms) for at most that long after the narrower read lands, so a 1 ms window covers it. Terra's sequence: `c7_a_narrow_read_just_after_the_held_return_…` (`c7.rs:1191`) and the `after` case of `c7_the_module_read_half_width_is_the_widest_read_in_the_window_before_the_return`. Rows C7-202 (max→last), 203, 290–296 killed. |
| 2 unresolved R | **CLOSED** | `r_check` (`c7.rs:237-247`) no longer returns early on `unresolved`; `lo..=hi` decides for every row. The fixture that accepted one period outside is now `c7_the_r_check_requires_the_interval_resolved_or_not` (refuses 130 and 90). Terra's 150–152 ms / 200 ms sequence is `c7.rs:1027`; I recomputed it by hand (lo 150 000, hi 152 000, width 400 100 > 200 000 ticks; 149/153/200 ms refused). The retargeted row (`unresolved \|\| in range`) is killed. Datum text (`c7_cells.rs:425`) and `bench.md:1710, 1788` updated. |
| 3 storm rows on their own range | **CLOSED** | `Common::checks` (`c7.rs:594-603`) runs `history` → `expected_r` → `r_check` for every kept row with `DEFAULT_MARGIN_NS` (= `profile::RESUME_MARGIN_NS`, 10 ms; the storm opens with `margin: None`, `c7.rs:833`). `storm_verdict`: an `ambiguous` row is INCONCLUSIVE (`c7_storm.rs:109-112`), any other error is `ResumeOffRule` after the bound checks (`c7_storm.rs:133`). Fixtures: unit (`c7_storm_fail_a_kept_row_off_its_own_rule_range`, `…_lacks_the_data_the_rule_needs`) and the real storm rehearsal (`fake.rs` row 500 moved 3 periods inside its Start interval → `ResumeOffRule`; `armed_closes + 100 000` → INCONCLUSIVE). Rows C7-297–300 killed. |
| 4 load flake / capture link | **CLOSED** (minor test gap, finding 2; text, finding 3) | (a) `CAPTURE_LINK_CAPACITY` 1 024 (`c7.rs:665`) applied in `open` (`c7.rs:679`), which both the trial (`c7.rs:736`) and the storm (`c7.rs:833`) use; the bench profile goes through the same `profile(…, session = true)` (`common/mod.rs:265-266, 296-297`), so the bench gets it too. Arithmetic: 1 024 × 2 000 samples × 200 ns = 204.8 ms at 10 MS/s; 1 024 × 2 000 × 8 B = 16.4 MB, negligible on the bench host. (b) `link_drops` from the Manifest (`c7_verdict.rs:479-481`) is its own reason before the readers' hole messages in the trial (`c7_verdict.rs:219`) and the storm (`c7_storm.rs:76`); unit and Manifest-level fixtures (`fake.rs:3267`, storm `links[0]`). ×40 under 16 `yes`: 40/40 ok (`x40-fix2.out`). Rows C7-301–305 killed. |
| 5 pinned assertions | **CLOSED** | `fake.rs:2901, 2951`: `!unresolved` / `allowed == 1` replaced by `R ∈ lo..=hi` (see note 4). |
| 6 storm continuity | **CLOSED** | `c7_storm.rs:102`: `covers(capture.start(), scheduled_end)`; fixture `c7_storm_inconclusive_a_hole_between_the_control_and_the_held_start` (hole at 100 ms, part after the held Start whole). Row C7-306 killed. A capture that starts late is still caught by `silence`'s own coverage check from `held + d − n·per`. |
| 7 silent final part | **CLOSED** | `c7_rf.rs:307`: steady periods `1..=9` for a silent first period, matching the 10-period coverage check; pure fixture at the exact boundary (10 periods ok, one sample short → coverage error) and the verdict fixture (`DroppedNeverResumed` at exactly 10 periods). `steady_ok` of a silent part is not what decides its class (it goes by `late_by_periods`). Rows C7-307, C7-310 killed. |
| 8 N4–N6 | **CLOSED** | N4: `c7_rf_the_noise_floor_term_…` makes `4·p_q` bind (leak 0.4 between the terms → not heard; 1.0 → heard); rows C7-308/309 killed. N5: `c7.rs:35`, `bench.md:1711`, `r_check` doc. N6: `fake.rs:2940`. |

5 000 → 44 000 (`c7.rs:1170`), checked from the code: the close returns at 140 730 000 ns (tick 28 146 000); its window opens at 139 730 000 ns; the read in effect then is the 400 µs read (exit 100 400 000 ns), and the 10 µs read (exit 140 710 000 ns) completed inside the window; max = 400 µs → ⌈400 000 · 200e6 / 2e9⌉ = 40 000 ticks; plus the offset to the decision 28 150 000 − 28 146 000 = 4 000; probe uncertainty 0 → 44 000. Physically right: the close returned 20 µs after the narrow read landed, inside the up-to-200 µs clamp of the wide read. The held Start's 54 400 = 40 000 + (28 144 000 − 28 129 600) likewise.

## Findings

1. **minor — the whole-call bracket is still a lower bound of the Authority's; bench.md states it as equal.** `DeviceTime::read` takes `before = host.now()` before `device.time_now()` and `after = host.now()` after it returns (`authority.rs:100-104`); the probe's `entered` is taken inside the call (`probe_device.rs:499`) and its exit before the rows lock is released (`probe_device.rs:444`). On the X310's real host clock (`host.rs:292-293`, no lock) the gap is a few instructions, so only a preemption of `uhd-clock` in that gap (or between `after` and the anchor install, `authority.rs:121-125`, which delays when a read takes effect) leaves the Module's anchor error above the oracle's w/2. Sequence: a read A accepted with Authority width 0.8 ms, the probe recording 50 µs because `uhd-clock` was preempted after the exit stamp; a held send returns 0.2 ms later with `busy + m` 0.1 ms below a lattice point → the Module's stamp can be off by ~0.4 ms, the oracle allows 25 µs → `ResumeOffRule` on a correct Module. It needs a ≥ 100 µs preemption inside a sub-µs window next to a send near a boundary, and it fails loud with its terms, so minor. Ask: `bench.md:1710` should say the probe's whole call is the closest observable bound *inside* the Authority's bracket (INFERRED that the difference is negligible), not "つまり" the same thing. The fake cannot show this residue: on a manual clock `reanchor` installs only zero-width reads (`authority.rs:124`), so the rehearsals exercise only the conservative side of item 1; the window logic is pinned by pure fixtures only.

2. **minor — no test or row observes that a C7 Session runs with 1 024 blocks.** `c7_the_capture_link_holds_a_200_ms_stall_at_10_msps_and_the_c7_session_applies_it` (`c7.rs:919`) checks `with_capture_capacity` on a JSON value, not `open`. Deleting `let profile = &with_capture_capacity(profile);` (`c7.rs:679`) fails no test and has no mutation row; the fake would silently return to 64 blocks (the flake back), and the bench too (fails safe through finding-4(b)'s reason, but costs reruns). Fix: assert the Manifest's implicit Spec feed capacity (1 024) in one existing C7 rehearsal, and add the row.

3. **minor (text) — "最初の理由" is not what the code does.** `bench.md:1785` says the link-drop reason is the *first* reason; the trial pushes it after "the Run did not end Fine", "a device error row" and "RX_OVERFLOW" (`c7_verdict.rs:210-221`), the storm after "the Run did not end Fine" (`c7_storm.rs:73-77`). The code meets the brief ("before any hole message"); the text should say that. Also `link_drops` sums every Manifest link (`c7_verdict.rs:479-481`) while the reason says "the capture link": true today only because a C7 Session has the one `rec` feed (the fixtures write `links[0]`); a note, no change needed unless a second link appears.

4. **note — the new `R ∈ lo..=hi` assertions are implied by the line above.** `trial.check(0, first, m).unwrap()` (`fake.rs:2897, 2947`) already runs `r_check`, which now requires `lo..=hi`, so the added asserts (`fake.rs:2901, 2951`) can never fire on their own. Harmless; the exact-side assertion (`resumes_at == l` or `l + period`) is the one that pins the Module, and on the manual clock the Module's R does not move under load.

5. **note — rows.** All 159 rows of the round's run are `killed` with `reruns: 0` (none `ERROR`, so none by timeout or bound). C7-203 first SURVIVED; its fixture was fixed and only C7-203/293/294 were rerun, while C7-202 (max→last) and C7-290–292 share the same filter and were not. I recomputed each against the final fixture: C7-290 dies on `wide_after` (80 000 ≠ 2 000), C7-291 on `settled` (100 000 ≠ 2 000), C7-292 on `clamp` (2 000 ≠ 90 000), C7-202 on `clamp` (2 000 ≠ 90 000), C7-203 on `discarded` (100 100 ≠ 100 000). The full lists run at the gate anyway.

6. **note — INFERRED block size.** The 16.4 MB and 204.8 ms assume 2 000-sample receive blocks on the X310; the receive call is `max(block_len, rx_packet_samples)` (`provider/control.rs:203`). A larger packet only lengthens the buffered time and the memory in proportion; both stay small.

No major. My own scenarios S4 (other `time_now` callers: only `DeviceTime::read` and UR-8 at `authority.rs:102, 373`), S7 (margin = Module default; pairing proven on 1 000 fake rows), S10 (a late capture start is caught by `silence`) and S11 (a silent part's class is not decided by `steady_ok`) did not turn into findings; S1–S3 are finding 1, S8 is finding 2.

Verdict: PASS


## terra-recheck-fix2

**Verdict: FAIL.** The R oracle still has two ways to underestimate the Module’s clock error and reject a correct resume. The other seven dispositions are closed for the requested cases.

1. **Major — Item 1: PARTLY CLOSED — the recorded bracket is narrower than the Authority’s bracket.** The probe stamps entry inside `time_now` and exit before the row is pushed and the lock released (`probe_device.rs:407`, `probe_device.rs:430`, `probe_device.rs:498`). The Authority brackets the call outside those points (`authority.rs:99`), but the oracle treats the inner stamps as the whole bracket (`c7.rs:312`).

   Minimal sequence: the thread is preempted after the Authority’s `before` but before probe entry, or after the probe’s exit stamp but before the Authority’s `after`. The actual bracket is wide but accepted; the recorded bracket is narrow. Near a lattice boundary, a correct R can fall outside the oracle’s interval and produce `ResumeOffRule`. The new fixture pins the recorded stamps, not the unobserved outer portions.

2. **Major — Item 1, second clock sequence: a read after the inner send return can affect its Module stamp.** `host_after_ns` is taken immediately after the *inner* `tx_send`, before the probe records the send and returns to the Module (`probe_device.rs:611`). The oracle excludes all reads completing after that instant (`c7.rs:329`); the subsequent delay allowance does not account for a newly installed read’s error (`c7.rs:399`).

   Minimal sequence: a narrow anchor is in effect at the inner send return; a wider accepted read finishes while the probe records that send; the Module then stamps it using the new anchor. If that anchor is behind near a lattice boundary, the correct R can again be reported as `ResumeOffRule`. The bench procedure explicitly labels this exclusion as INFERRED, rather than establishing the bound (`bench.md:1710`).

3. **Note — Item 2: CLOSED.** `r_check` requires both lattice membership and `lo..=hi`, including for unresolved rows (`c7.rs:237`); the inside, outside, and off-lattice cases are exercised (`c7.rs:1010`).

4. **Note — Item 3: CLOSED for the requested scenarios, subject to the clock-bound findings.** Each retained row is checked using its own indexed send history (`c7.rs:593`); missing closes become INCONCLUSIVE (`c7_storm.rs:109`). The fake rehearsal changes row 500 within its ordering interval and exercises both the rule failure and missing-data path (`fake.rs:3309`).

5. **Note — Item 4: CLOSED for the stated 200 ms stall and recorded load run.** C7 applies 1,024 blocks to its capture profile (`c7.rs:661`). At 2,000 samples per block and 10 MS/s, that holds **204.8 ms** and about **16.4 MB** of sample data; that is reasonable against the host’s approximately 61 GiB of RAM. Link drops are an INCONCLUSIVE reason before subsequent hole reasons in both verdicts (`c7_verdict.rs:219`, `c7_storm.rs:76`). The `load record` reports 40/40 runs successful with 16 load processes. It does not prove immunity to longer stalls.

6. **Note — Item 5: CLOSED.** The hardware-period rehearsals now assert the verdict and interval membership rather than requiring one allowed R (`fake.rs:2897`, `fake.rs:2950`).

7. **Note — Item 6: CLOSED.** Storm continuity starts at the capture’s first sample (`c7_storm.rs:99`), matching `bench.md:1798`.

8. **Note — Item 7: CLOSED.** `read_part` reads steady periods 1–9 after a silent first period and 1–19 after an audible one (`c7_rf.rs:293`); the ten-period boundary fixture checks both sides (`c7_rf.rs:742`).

9. **Note — Item 8: CLOSED.** The noise-floor term has a binding fixture (`c7_rf.rs:766`); the nine-Start and unresolved-range descriptions now match the code (`c7.rs:35`, `c7.rs:234`).

10. **Note — Arithmetic and mutation evidence.** The changed close error of **44,000 ticks** follows the fixture: 40,000 ticks for half the 400 µs read, retained by the one-millisecond window, plus 4,000 ticks from close return to decision (`c7.rs:1151`, `c7.rs:1164`). The `mutation log` records 158/159 initially, with C7-203 surviving; after its fixture fix, C7-203 and C7-293–294 were rerun and recorded killed. Those kills do not cover the two outer-bracket sequences above. I did not rerun builds, tests, or mutations.

Verdict: FAIL