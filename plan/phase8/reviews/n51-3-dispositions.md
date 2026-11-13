# Note 51 stage 3 — review record and dispositions (2026-11-13/14)

No mutation verdict of the radio-uhd and Kernel test suites depends on wall-clock time under load any more, except on rows that say so ([51-test-timing.md](../51-test-timing.md) §6.5, §7.2 row 3, §8, P3/P3a, Q5).

- **S2 (3ab):** the real-clock FakeDevice tests moved to the manual clock. The lib has 139 → 6 marked, fake 41 → 28 (the real-by-design ones), x3xx 8 → 0. The `Gated` expiries write the infrastructure marker.
- **S3 (3c-1, 3c-2):** the Kernel's `device_paced.rs` observes a parked wait instead of timing it. The wake tests release their mark only after the control loop is seen parked, on a detached worker with an unreachable horizon. A scan in `governance.rs` fails any short wall bound, short wait or sleep in that file unless pinned per occurrence (P3a).
- **3d:** the receive start-lead rehearsal's SHORT is judged by a pure test of `restart_stage`.
- **3e:** a row whose run is marked real-clock and that does not declare `"kill": "bound"` is `ERROR` whatever its outcome (`mutate.py`), and the gate re-checks it from the row lines. The 27 rows still on the real clock declare it.

Not risky in production terms (test code, test doubles and the gate's accounting). A test plan and Sol's plan review were run anyway (the gate's semantics). Branch `work/n51-3` from `8d96743`, sub-steps 3ab, 3c-1, 3c-2, 3d, 3e plus fix rounds, squashed and rebased onto main. Implementer: Claude Code Sonnet 5.5 subagents, effort high ([n51-3-briefs.md](n51-3-briefs.md)). Test plan: [n51-3-test-plan.md](n51-3-test-plan.md) (v2). Sub-step reviews: fresh Opus subagents, effort high.

## Process

- Plan v1 → Sol ([n51-3-sol-plan.md](n51-3-sol-plan.md)): FAIL, 3 majors, 3 minors, all adopted. Plan v2 rebased on 8d96743 (#67 and the stage-4 tools had moved the inventory: 200 marked rows, 27 after S2) → Opus re-check: M1–M3 and m4–m6 closed, one new major N1, adopted.
- Each sub-step: one implementer run, a fresh Opus review, fixes, then a local commit. 3c-2's major fix was re-checked by Opus. After 3c-1 and 3c-2, the orchestrator ran the rows whose tests had changed but that lay outside the plan's lists.
- Full lists before Sol's code review (at 3e): 2745/2746, only the known survivor `phase7/G09`, 0 `ERROR`, every real-clock row declared.
- Sol's code review ([n51-3-sol-code.md](n51-3-sol-code.md)): PASS, one minor (fixed), two notes.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan v1 | 3 | 3 | — (M1 the wake tests could pass without a wake, M2 the SHORT replacement skipped the decoding, M3 the enforcement ignored survived and equivalent rows): plan defects, no code yet |
| Opus, re-check of plan v2 | 1 (N1) | 7 | `wall-clock-test-oracle` (N1: G36's counter read raced the data thread on the unmutated tree) — caught in the plan |
| Opus, 3ab | 0 | 3 | — (doc, leftovers, doc) |
| Opus, 3c-1 | 0 | 3 | — (ten rows outside §4.6 not run; header overclaim; a copied setup) |
| Opus, 3c-2 | 1 | 5 | `wall-clock-test-oracle` (a positive wait pinned as a negative one in `kg_02_a_failed_sink_is_not_stepped_again`) |
| Opus, re-check of the 3c-2 fix | 0 | 1 | — (rows to re-run) |
| Opus, 3d | 0 | 1 | — (doc) |
| Opus, 3e | 0 | 2 | — (`declared` unpinned; three copies of the declaration test) |
| Sol, code | 0 | 1 | — (an integer-suffixed literal bypassed the scan) |

The two `wall-clock-test-oracle` findings are the mechanism note 51 answers. Both were caught before landing, inside the rule (a predicate wait, a pinned negative wait), not beside it.

## Dispositions

All decided by the orchestrator (nothing user-visible). The full working record is in `~/.cache/ezsdr-fixes/2026-11-13-n51-3/plan-decisions.md`.

### Plan

- **Decisions 1–8:** the `FakeConfig` default stays Real; the ten probe-tool rehearsals stay Real and are declared; P3a pins negative waits with a reason; J58 stays equivalent and is declared (decision A); `Probe::wait_until` is added; the §4.2 siblings are rewritten in 3c; SHORT moves to a pure test; enforcement goes in `mutate.py` and again in `gate.py`.
- **Plan v2's E, F, G and A–D:**
  - E: no declarations on Kernel rows, since their expiries are plain kills.
  - F: the observed-park shape, and a `waiting` predicate capped at 10 s in place of the 20 ms window.
  - G: one bound rule, `Wall::from_secs(n ≥ 3)` or a pin.
  - B: the pin covers the call only.
  - C: #9 and #10 keep the bound form.
  - D: the per-site edits are re-measured by 3ab's load runs.
- **N1:** the counter is asserted once, `>= rounds`, after the run. The worker is a detached `std::thread::spawn`.

### Sub-steps

- **3ab:** the three minors went into 3d (same crate). Note 4 (inline manual `FakeDevice` literals versus `common::fake()`) is pre-existing and goes to the Phase 8-end review.
- **3c-1:**
  - The ten extra rows were run: G06, G07, G11, G12, G35, C09, C10, J73 and J110 killed; G09 is the known survivor.
  - The header and the duplicated setup went into 3c-2.
- **3c-2:**
  - Major fixed: a predicate wait on the failing step, then one negative sleep.
  - Three more calls are scanned: `wait_timeout`, `park_timeout`, `recv_deadline`.
  - Every fn is scanned, helpers included.
  - The remaining unscanned forms are listed as limits in the scan's header and in P3a.
  - The rows' anchors and filters were tightened.
  - The re-check's rows were run: N51-2A-32..35, -38, -39, C89-1-01..03 and C07, all killed.
  - The Q5 wording was fixed.
- **3d:** the doc minor and two nits were applied by the orchestrator; the 3D rows were re-run, all killed.
- **3e:** `declared` is computed once in `run_one`, returned, and pinned by cells.
- **Sol code m1:** integer suffixes are stripped before the scan parses a literal, with fixtures and a row.

### Notes kept open

These go to the Phase 8-end review unless a later stage takes them.

- The two `kc_36_` TTL tests can still fail under load. They are pinned as "TTL is the subject"; `Lease::detached` takes a `HostClock`, which would allow a manual-clock rewrite.
- The scan's pins key on the fn name alone.
- The scan does not catch `elapsed().as_*() < n`, mirrored comparisons, `Wall::new` or computed literals (listed limits).
- `main()` of `mutate.py` forwards `declared` without a test of its own (simple wiring between tested parts).
- The baseline cache key ignores `exact`. No row sets it.
