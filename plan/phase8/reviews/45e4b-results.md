# E4b results: the speed re-measurement with K = 4 (device) and D = 64 (per chain)

Measured as `45-experiment.md` §11 "Fixed before measurement" and "Criteria", unchanged from E2b: p99(new) − p99(today) < 100 µs, p99(new) ≤ 300 µs, loss(new) ≤ loss(today), every update accounted; 10,000 updates, ABBA then BAAB (4 repetitions per owner), the same update table, ingress rule and fail-closed per-repetition verdict, loss counted from the reader. K = 4 and D = 64 are the brain's defaults; no run overrides them. FakeDevice is unchanged. Code of the run: the tree at `ede2327` plus the E4b tooling diff (uncommitted). Raw output: this directory (`*.txt`, `*.uptime.txt`; `run-all.sh`; `analyze.py` reads the JSON lines).

Host: sim03 (32 cores), every run through `rdev.py`, one at a time. Load average before each run 0.0–0.6 (1-minute; 5-minute 0.1–0.7, 15-minute up to 2.4 for the first runs). E2b's runs were at load 16–18 on the same host; these ran on a nearly idle host, so absolute numbers are not comparable with E2b's. No run was repeated.

## Verdict (per §11)
| Condition | Owner | p50 (per repetition) | p99 | p99.9 | Failures | Loss (overflow, missing, drops) | Verdict |
|---|---|---|---|---|---|---|---|
| 1 MS/s × 2,000, spaced | today | 531–539 µs | 1,044–1,052 µs | ~1,052 µs | 0 | 0, 0, 0 | |
| | new | 89–97 µs | 104–111 µs | 108–114 µs | 0 | 0, 0, 0 | **PASS** (pooled p99 1,049 vs 110 µs) |
| 1 MS/s × 2,000, bursty | today | 513–527 µs | 1,034–1,045 µs | ~1,058 µs | 0 | 0, 0, 0 | |
| | new | 62–82 µs | 95–109 µs | 104–115 µs | 0 | 0, 0, 0 | **PASS** (1,039 vs 108 µs) |
| 50 MS/s × 500, spaced | today | 496–542 µs | 1,019–1,049 µs | ~1,050 µs | 0 | 0, 0, 0 | |
| | new | 88–97 µs | 130–202 µs | 146–230 µs | 0 | 0, 0, 0 | **PASS** (1,042 vs 174 µs) |
| 50 MS/s × 500, bursty | today | 512–544 µs | 1,038–1,057 µs | ~1,064 µs | 0 | 0, 0, 0 | |
| | new | 58–74 µs | 104–157 µs | 127–197 µs | 0 | 0, 0, 0 | **PASS** (1,048 vs 130 µs) |

All five checks hold in each condition (p99 difference, p99 ≤ 300 µs, loss, every update applied at e and the runs clean, every repetition clean). The rate trials: candidate 0 (200 MS/s × 2,000) fails for both owners as in E2b (today 79.5k blocks/s steady with 102 overflow events, new 89.7k with 51); candidate 1 (100,000 blocks/s, the 50 MS/s × 500 equivalent) passes for both at 100,000.04 blocks/s with no overflow and a flat backlog (`trial-*.txt`).

## K and D (new owner, per repetition, `verdict-*.txt`, `bound-diag.txt`)
| Condition | K rejections | D rejections | values with a K / D hit | max K load, max D occupancy (after any attempt) |
|---|---|---|---|---|
| 1 MS/s spaced | 0 in all 4 reps | 0 | 0 / 0 | 1, 1 |
| 1 MS/s bursty | 0 | 0 | 0 / 0 | 4, 4 |
| 50 MS/s spaced | 0 | 0 | 0 / 0 | 1, 1 |
| 50 MS/s bursty | 0 | 0 | 0 / 0 | 4, 4 |
| bound diag spaced (2 runs) | 0 | 0 | 0 / 0 | 1, 1 |
| bound diag bursty (2 runs) | 0 | 0 | 0 / 0 | 4, 4 |

The bursty tables reach K = 4 (the bound) without a rejection: four values released together fit exactly. D = 64 is never near (max D equals max K: one chain). The per-second samples of the bound diagnostic (10 ms readings) show K and D occupancy at most 1 (spaced) and 3 (bursty) at the sampled instants, and one second per run with a backlog of 10.6–11.0 ms at the start; no other second exceeded 1 ms (`coincidence_1ms`: one excursion second, no bound-hit second, in any run).

## Coupling check (late updates: not applied at e)
In every verdict repetition of the new owner and in all four bound-diagnostic runs: late updates 0, so late with a preceding D hit 0, with a K hit 0, with neither 0 (`new_coupling` in each `E2B_VERDICT`, `bound_hits.coupling` in each run). The fault run has late updates after the device loss (260 and 359 for today and new) with no K or D hit: they are the loss, not a bound. The backlog part (second with backlog over 1 ms) needs the sampler; it is reported in the bound-diagnostic runs only (0 of 0).

## Verdict per §11
The speed criterion is met in all four conditions, in every repetition and pooled: **no D hit and no K hit preceded any late update, because there were no late updates and no D hit at all.** The E2b failure mechanism (a successful call keeping its slot until R observes the consumption, so a receive backlog held values in flight) is not present: the bound now counts K by device time and D by residency, and the highest observed K was the design's 4, reached only by bursts of four.

## Other runs
- diags (`diag-1mhz.txt`, `diag-high.txt`): counters off / on / diagnostic, per owner: p99 99–120 µs new and 1,028–1,053 µs today in every variant, 0 failures. The new owner's `counters_off` variant is marked unclean because its loss accounting cannot reconcile without counters (as in E2b); it is a variant of the overhead diagnostic, not a verdict run.
- fault (`fault.txt`): a device loss mid-table, per owner, as in E2b: today applies 340 updates at their instant, new 241; the rest are lost with the device.
- bound diagnostic at the defaults (`bound-diag.txt`), spaced and bursty, two runs each (no second bound value to compare): 0 failures, p99 116–127 µs.

## Tooling (what changed since E2b)
- The per-second samples report K occupancy (`device_load` at the brain's time) and D occupancy (the chain's resident count), end and max, separately, with `t_end_ns`.
- Each repetition reports K and D rejections, the values with a K or a D hit, and the largest K load and D occupancy seen after a reservation attempt (computed in the brain's operation, where the rejection reason is already read), plus the coupling check (`coupling_check`, unit-tested). A value's first K and first D rejection times are recorded for it.
- Limits: the maxima are taken at reservation attempts (post-operation), not continuously; a hit's time is a value's first rejection at that bound; the backlog test is per sampled second.

# Backlog confirmation (follow-up, `45-experiment.md` §11 "Backlog confirmation")

The same 50 MS/s × 500 verdicts (spaced, bursty; 10,000 updates; ABBA/BAAB, 4 repetitions per owner) with the fake's receive DSP history preloaded with H = 10,000 timed entries for **both** owners before each repetition (`E2B_PRELOAD=10000`; `preload_dsp_history` as the trial uses it, far-past instants, applied straight to the fake). The preload is not an update: its entries are taken out of the DSP log before the accounting (`without_preload`, unit-tested, row E4-122) and never pass the owners' timing decorator. Plus one bound diagnostic per kind at H = 10,000 (`E2B_BOUND_RUNS=1`). sim03 through `rdev.py`, one at a time, load average before each run 0.0–0.9 (`preload-*.uptime.txt`). Files: `preload-verdict-high-{spaced,bursty}.txt`, `preload-bound-diag.txt`, `run-preload.sh`. No run was repeated.

## Table (per repetition, 4 per owner; `rep` in the verdict JSON is the pair index 0–3)
| Condition | Owner | p50 | p99 | p99.9 | Failures | Loss: overflow events, missing samples, drops | Verdict |
|---|---|---|---|---|---|---|---|
| 50 MS/s × 500, spaced, H = 10,000 | today | 484–531 µs | 1,016–1,048 µs | 1,046–1,056 µs | 0 | 1 each; 5.00–5.04 M; 0 | |
| | new | 83–99 µs | 184, 190, **1,102**, 196 µs | 225–349, **1,485** µs | 0 | 1 each; 5.00–5.01 M; 0 | **FAIL** (pair 2: p99 1,102 µs > 300 µs; pooled p99 692 µs > 300 µs) |
| 50 MS/s × 500, bursty, H = 10,000 | today | 510–530 µs | 1,041–1,055 µs | 1,058–1,076 µs | 0 | 1 each; 5.01–5.04 M; 0 | |
| | new | 69–95 µs | 171, 172, **496**, 167 µs | 196–239, **739** µs | 0 | 1 each; 5.01–5.03 M; 0 | **FAIL** (pair 1: missing samples new 5,026,363 > today 5,024,273; pair 2: p99 496 µs > 300 µs; pooled p99 292 µs passes) |

In both conditions: every update of both owners applied at its instant (0 failures), all runs clean, and the p99 difference check (< 100 µs below today's) holds in every pair. The checks that fail are "p99(new) ≤ 300 µs" in one pair of each condition, "loss(new) ≤ loss(today)" in one pair of the bursty condition, and therefore "every repetition clean and latency and loss hold" in both. The failing pairs are a single repetition of the four in each condition; the outliers are in the tail (max apply return 1,564 µs spaced, 897 µs bursty; in the bursty pair the apply entry p99 is 354 µs against 94–98 µs in the other repetitions) and the loss excess is 2,090 samples of about 5.0 M (0.04 %, both owners lose about 100 ms of samples once).

## K, D, coupling, backlog
- K and D hits: 0 K rejections and 0 D rejections in every repetition (verdicts and bound runs). Max K load / D occupancy after an attempt: 1 (2 in one spaced repetition) and 4 for bursty; D never near 64.
- Coupling check: late updates 0 in every repetition, so late with a D hit, with a K hit and with neither are all 0. "D hits precede the late updates" cannot occur: there are no D hits and no late updates.
- Backlog per second (bound runs, new owner, sampler maximum of 10 ms readings): spaced 97.4 ms in the first second, then 0.05–0.29 ms in the other 49 (one more reading of 0.18 ms in the last); bursty 0.06–0.32 ms in all 25 seconds (no second above 1 ms). Per-second K and D occupancy maxima at the sampled instants: 0 spaced, 3 bursty. `coincidence_1ms`: spaced one excursion second (the first), no bound-hit second; bursty none.
- In the sampled bound-diagnostic runs (VERIFIED) the H = 10,000 preload produced **a start-up transient, not a sustained backlog**: one overflow event and about 100 ms of backlog in the first second, then a backlog under 0.3 ms. The verdict runs record one overflow event and about 5.0 M missing samples per run but no gap timing, so placing them at the start there is INFERRED from the diagnostic runs. E2b's saturated 54.7k blocks/s was not reproduced; on this idle host the scan cost with 10,000 entries did not hold R below the block rate once the first overrun had passed. Most updates (spaced: the first 5 ms-spaced ones within the first second) ran with the transient behind them; no update was late.

## Verdict per the paragraph
The confirmation runs fail the criterion as written: both preload verdicts record FAIL (one spaced and two bursty repetitions break a per-repetition check), and the paragraph makes either failing a failure of the speed criterion. Read with the evidence: the failures are latency tail outliers and a 0.04 % loss excess in single repetitions of the new owner, not late updates and not bound hits (0 K, 0 D, 0 late, 0 failures), and the confirmation did not produce the sustained backlog it was meant to test (the split's behaviour under E2b's mechanism remains unexercised; only its absence of harm during a start-up overrun is shown). The idle-host verdicts above remain as recorded: PASS in all four conditions.

## Where the delays are (diagnosis from the existing raw output, no new runs; corrected after Sol's review)
Source: the per-update table each verdict run printed (`E2B_UPDATES`: acceptance delay = apply return after the release instant, entry delay = apply entry after it, so inside-apply = acceptance − entry) and each run's JSON. Time is seconds since the repetition's first update instant, which is `FIRST_NS` = 0.3 s after the table's base (the preload, 72–92 ms, ran before the base). "Top 1%" is the 100 largest of 10,000 acceptance delays. Repetitions: failing spaced pair 3 = rep 4 (new), failing bursty pair 2 = rep 4 (new); for contrast the passing pair 1 of each kind = rep 1 (new); today's owner from the same runs (spaced pair 3 = rep 5, pair 1 = rep 0).

| Repetition (new) | p99 | top-1% delays | where the top 1% lie (VERIFIED, from the table) |
|---|---|---|---|
| spaced rep 4 (fail) | 1,102 µs | 1,107.7–1,564.0 µs | 0 before 39 s; 98 of 100 in 41.6–50 s (the other two at 39.49 s and 41.58 s), rising in density to the end. The first 38 s are normal (per-5 s maxima 172–222 µs, p50 about 99 µs); from about 38.5 s to the end (11 s) p50 is 145–260 µs, 156 updates exceed 1 ms (first at 39.5 s) and 638 exceed 500 µs |
| spaced rep 1 (pass) | 184 µs | 183.8–988.0 µs | 0 in the first 4 s; one isolated 988 µs at 4.05 s (update 811); 47 of 100 in 5–10 s, 28 in 35–45 s, the rest between; no update above 1 ms, one above 500 µs |
| bursty rep 4 (fail) | 496 µs | 496.7–896.9 µs | 0 in the first 12 s; 61 of 100 in 16.7–19.05 s and 18 in 22.6–23.9 s (the other 21 between 12.55 and 24.5 s); per-second maxima 0.81–0.88 ms at 17–19 s, p50 up from 95–99 to 121–160 µs at 18–19 s; 96 updates above 500 µs, none above 1 ms; the seconds before 15 s are normal (per-5 s maxima ≤ 0.36 ms) |
| bursty rep 1 (pass) | 171 µs | 170.9–389.0 µs | 2 at 0.47–0.71 s, 68 of 100 in 20–25 s, the rest spread 8.7–20 s; none above 400 µs |

### Inside apply (VERIFIED, from the saved rows: entry and return of each timed apply)
| Repetition | owner | entry after release, p50 / p99 | inside apply, p50 / p99 / max | top-1% delays: entry, inside |
|---|---|---|---|---|
| spaced rep 4 (fail) | new | 95 / 96 µs | 4 / 1,015 / 1,482 µs | entry 71.7–96.1 µs, **inside 1,011.5–1,481.8 µs** |
| spaced rep 1 (pass) | new | 81 / 98 µs | 4 / 94 / 899 µs | entry 70.0–240.4 µs, inside 7.5–898.8 µs |
| spaced rep 5 (pair 3) / rep 0 | today | 512 / 1,013 and 518 / 1,012 µs | 3–5 / 10–11 / 17–35 µs | entry 1,005–1,054 µs, inside 0.5–35.5 µs |
| bursty rep 4 (fail) | new | 73 / 360 µs | 5 / 373 / 866 µs | entry 15.0–799.6 µs, inside 0.7–865.7 µs |
| bursty rep 1 (pass) | new | 59 / 96 µs | 4 / 83 / 296 µs | entry 34.4–291.3 µs, inside 2.8–295.7 µs |
| bursty rep 5 / rep 0 | today | 506 / 1,052 and 527 / 1,040 µs | 1 / 10–11 / 35–48 µs | entry 1,037–1,090 µs, inside 0.5–10.7 µs |

In the failing spaced repetition every one of the 100 largest delays reaches the apply at the normal time (within 96 µs of the release) and then spends 1.0–1.5 ms inside it; the new owner's inside-apply p99 is 1,015 µs there against 94 µs in the passing repetition. In the failing bursty repetition the large delays are mixed: some wait before entering (entry up to 800 µs, p99 360 µs against 96 µs in the passing one) and some sit inside (up to 866 µs). Today's owner, in the same runs, has inside-apply times of at most 58.2 µs (spaced rep 3, update 6375; orchestrator correction after Sol's re-check) in every repetition; its delay is the roughly 1 ms between the release and the entry that its design has.

### Causes (all INFERRED; the data do not separate them)
1. **A host-side disturbance** of a few seconds (another tenant on the shared sim03, a scheduling excursion of the apply thread or of the thread holding what apply waits for).
2. **FakeDevice lock contention growing with the DSP history.** The fake's receive holds the device lock across the history scan and the sample construction (`device.rs:1785–1803`), the settings apply takes the same lock (`device.rs:1423`), and the scan grows with the DSP history (`device.rs:552`). An apply that waits for a receive in the middle of a scan would show exactly an unchanged entry time and a long time inside apply, with no sustained receive backlog. The history is 10,000 entries at the start and grows by one per update, which would make a late-run onset plausible; the passing repetitions have the same history, so the history alone does not decide it.

Evidence that bears on both, none of which decides: all these repetitions had 0 K rejections, 0 D rejections and 0 late updates; `release_lateness` (the timer's send) is 61–64 µs p99 in all four new repetitions, so the delay arises after the release; the cell retry counters do not separate the repetitions (publish retries 30,137–34,718 per run, the failing spaced rep 4 the highest of its kind, the failing bursty rep 4 the lowest of its kind). The shape (an onset, a plateau of 1.4–1.56 ms in the spaced case, then the run ends) is not what a garbage collector would give (Rust, no collector); that much is VERIFIED about the language, not about the cause.

### What does not exist for these runs (VERIFIED)
The verdict runs have the diagnostic and the sampler off (`cell_hold_all_actors.count = 0`, `latency_stages = null`, `bound_diag = null`): no cell hold/wait, no brain input queue and no per-second backlog for the failing or the passing repetitions. The only per-second backlog and occupancy samples are the separate bound-diagnostic runs, which are different repetitions without such an excursion (backlog at most 0.32 ms after their first second); the start-up backlog of 97 ms is observed there, in those sampled runs, and nowhere else.

### The overflow and the loss (what is and is not established)
- VERIFIED: every verdict run, both owners, has exactly one overflow event and 5.00–5.04 M missing samples (about 100 ms at 50 MS/s); the loss is one number per run (the reader's leading and in-between gaps) and the output has no gap time or size, so **when** the overflow occurred in a verdict repetition is not in the data. In the sampled bound runs the 97 ms backlog is in the first second.
- INFERRED: the missing samples of the verdict runs are the same start-up overrun; the earliest top-1% delays are at 0.47 s or later, so no apply was slow at the start in the sampled sense, but that does not place the overflow. A later receive disruption in a verdict repetition cannot be excluded by this output.
- Bursty pair-1 loss excess: the eight bursty totals are 5,005,018–5,040,979 (spread 35,961 samples, today alone 5,010,806–5,040,979); the pair's excess of 2,090 (new rep 2: 5,026,363; today rep 3: 5,024,273) is inside that spread (VERIFIED). INFERRED: it is the run-to-run variation of one overrun's length, not a difference between owners; its position in time needs the reader's gaps recorded with positions.

### Coupling check after the timestamp fix
The check now uses one release definition for every update (the nominal `e − 3 ms`) and compares each value's first K and first D rejection time with it; a value's own later rejection does not count as preceding its release (`late_held_at_d`, `late_held_at_k` count the late updates that were themselves rejected at a bound). It is not recomputed from the saved output because there is nothing to recompute: every saved run (80 runs with the fields: the verdict, diagnostic, bound and fault runs) reports 0 K rejections, 0 D rejections and no value with a K or D hit, so the reported zeros of the coupling counts stand under either definition (no update had any hit, preceding or not).
