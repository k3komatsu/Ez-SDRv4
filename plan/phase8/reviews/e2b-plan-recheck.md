# E2b test plan — Opus re-check after Sol's review (2026-11-12, read-only, nothing built or run)

Verdicts: M1 CLOSED WITH CHANGES (token design holds; P7(i),(ii) kill E2B-15, P7(i),(iv) kill E2B-17; Send token is local: staging.rs:424,435, tx.rs:689,717,739,840; Mock lib.rs:1930-1931 respelled). M2 CLOSED (hook arms at tx.rs:619, first firing at :621 re-arms, second at drop_piece's lock :632; Z refused at control.rs:581-584 while X's chain is recorded — INFERRED until run; E2B-16 killed; hook test-only: core.rs:60-66, mod.rs:12-13; no deadlock). M3 CLOSED (no streams lock between tx_reopen_done and :954, nor :619-:621; the mutant writes Some on a lost device, nothing clears it again: core.rs:1046-1054; device_lost in the hook cannot deadlock: core.rs:1294-1349). Six adopted minors CLOSED.

New findings:
- N1 major (VERIFIED): moving the in-flight op from `Open` to the book breaks `txbook_take_untake_and_end_open_keep_one_open_booking_and_change_no_record` (crates/ezsdr-radio/tests/staging.rs:1748-1762): take(pl(7)), issue(S1), end_open() with S1 in flight, take(pl(8)), issue(S2) → panics "in flight". Contradicts "spelling only". No production end_open with a Send in flight (tx.rs:879,992,1036). Recommendation: `end_open` and `take` assert nothing in flight; amend the test; list it as changed.
- N2 minor: P2/P3 "should_panic ×3" in one function runs only the first case; E2B-03, E2B-07 would survive; "in flight" in every message. One function per case, or catch_unwind per case with the exact message.
- N3 minor (VERIFIED): failed reopen's loss read (tx.rs:958) has M3's gap and now decides Failed vs Interrupted; add C9(c) and a hoisted-read row.
- N4 minor: C7, C9(a), R2b run on a `released` participant thread: 20 runs (AGENTS §9 step 8).
- N5 minor: C8 asserts the hook ran and the slot is empty before Z′; the test thread holds no Control guard during `turns`, else a leftover hook under a mutant hangs instead of failing.
- N6 minor: `issue_reopen` placed before the early returns (tx.rs:930-940) has no row (unlike E2B-10); add one or say why none.

Not checked: C-test helpers; whether P1's comparisons compile against private `unread`; R1 precondition soundness outside these turns.

Orchestrator: all adopted (decided by the orchestrator, AGENTS §9 "What the owner decides"); N1 as recommended.
