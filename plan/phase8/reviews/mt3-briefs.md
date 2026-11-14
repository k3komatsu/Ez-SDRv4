# Maintenance table 3 — briefs (orchestrator → Claude Code Sonnet 5.5 subagents, effort high)

## design-brief

# Design brief — maintenance table 3: FakeDevice's late-burst drop vs the FPGA's per-packet policy (AGENTS §9 step 0)

You are the design drafter (Opus, effort max), read-only. Write the draft to /home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/draft.md. Do not edit anything in /home/komatsu/local_work/Ez-SDRv4 (another agent is implementing there; read only) or the worktrees. No builds needed; if you must probe, use a copy outside the tree (AGENTS §7).

## The question
Maintenance table row 3 (plan/maintenance/audits-2026-10.md, "Phase 7 後の保守", row 3): FakeDevice drops a whole burst on TimeError, but the X310 FPGA's default policy is `TX_ERR_POLICY_PACKET` (the host never writes `REG_TX_ERROR_POLICY`): only late packets are dropped and later untimed packets of the same burst play at once. Multi-packet bursts may also misalign UR-28's `unacked` bookkeeping. The bench has now measured it (H1, 2026-11-13, plan/phase8/bench-results.md ~:132 and ~:459: late burst dropped per packet by the FPGA; if every packet is timed nothing plays, the untimed tail plays; the fake drops the whole burst). Decide whether and how UR-33 (design/18-uhd-radio.md) and FakeDevice change, and what the Module and the Manifest say about a partly-dropped late burst.

## The draft must give (AGENTS §9 step 0)
- The user-facing contract: what an experimenter sees for a late multi-packet burst on the X310, on the FakeDevice and on MockRadio (events, flags, Manifest records, refusals), and what they can and cannot rely on.
- At least two alternatives (e.g. model per-packet drop in the fake and report it; keep the fake's whole-burst drop and declare the difference; make the Module time every packet so the device drops the whole burst; set the FPGA policy), each with the reason it is rejected or chosen; a recommendation first.
- The principles that decide cases; a check against the existing promises (EA, RM, KL entries in KNOWN_LIMITATIONS.md; UR-28, UR-33, the Stream Contract §23's TX bursts: no zero padding, TX_DISCONTINUITY); Mock enforces the hardware envelope (§13, §34, §59).
- The hardware evidence, VERIFIED/INFERRED, with UHD 4.10 source or FPGA pointers and the bench records (read the raw lines bench-results.md points to; do not carry a summary's claim).
- What the choice makes impossible later; which tests (FakeDevice rehearsals of hw tests, AGENTS §9 "Hardware tests and bench values") would change.
- Which decisions are the owner's (user-visible: API meaning, events/refusals, Manifest, KL entries) — list them one per line with your recommendation — and which are internal.

Read first: AGENTS.md §3, §6, §9; design/18-uhd-radio.md (UR-28, UR-33 and the TX path), design/07 (Radio Model TX bursts), KNOWN_LIMITATIONS.md, plan/maintenance/audits-2026-10.md row 3, plan/phase8/bench-results.md (H1 lines), crates/ezsdr-radio-uhd/src (FakeDevice, the tx path), crates/ezsdr-mock-radio. Keep the draft under ~250 lines; cite path:line.

## addendum-brief

# Design brief — maintenance table 3, addendum: a repeat resumes after a late drop or an underflow (AGENTS §9 step 0)

You are the design drafter (Opus, effort max), read-only. Write /home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/addendum.md (under ~200 lines, path:line citations). Do not edit, build in, or change git state in /home/komatsu/local_work/Ez-SDRv4 or its worktrees (other agents work there; read only — the worktree /home/komatsu/local_work/Ez-SDRv4-e3 is at main c04fda1).

Inputs in /home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/: draft.md (direction A), fable-review.md (A′, M1–M3, cells §2.4), decisions.md (the owner's direction A, O1–O7, and the last entry "REPEAT RESUME" — the owner's decision you are designing). The owner's view: a repeat is one endless burst command (RM-12 `start_repeat`, RM-15: it ends only at the next burst's start, a Stop, or the Run's stop), so a momentary host stall should not stop it.

Design, within the owner's decision:
1. The resume point: first period boundary at or after (end of what was handed + reported lateness δ + margin W). Check it against the drain model (draft §1.1 E2–E5, Fable §2.3/M3: 2 clocks per DUC packet, the DUC's between-burst latency) and the underflow case (what δ is there; P6 evidence). Show it cannot be early for the shapes the Module sends; say what C4 will tighten.
2. How the Module closes the dropped device burst and starts the new one (UR-22's close rules, T2/T3, CLOSE_GAP, `g`, the safe front), the period lattice (note 47 F7's switch at a period boundary), and what happens to a switch, a Stop, a burst booked behind the repeat (which ends it per RM-15), and KL-39 regions/footprints across a resume.
3. The user-facing contract: the gap in the burst record (Stream Contract §23 time jump inside a burst, TX_DISCONTINUITY or what the Module records today), the event's span [from, to) on late_at_device and TX_UNDERFLOW, the Manifest (UR-30 host view, rejected notes), the cap (a resumed device burst dropped again before any sample plays ends the repeat) — check the cap is decidable from device reports (ACKs, Underflow, TimeError) without a wall clock.
4. MockRadio: RM's rule is shared by both Providers (§3 "Mock enforces the hardware envelope"): does MockRadio have an underflow for repeats, and must it resume too? If MockRadio never underflows or is late, state the parity claim and the KL line.
5. At least two alternatives (e.g. resume at the next boundary after the last report instead of the computed point; end after N resumes in a time window) with the reason each is rejected.
6. What it changes in RM-15, RM-11, UR-22, UR-28, UR-33, KL-13/16/39 (amendment list); the tests and mutation rows it adds; the INFERRED values (W) and the bench cells that settle them.
7. Decisions that are the owner's (user-visible) beyond what decisions.md already settles — one per line with a recommendation; and internal ones you decide.
Read first: AGENTS.md §3, §6, §9; design/07-radio-model.md (RM-11..RM-16, RM-37), design/18-uhd-radio.md (UR-22, UR-28, UR-30, UR-33), KNOWN_LIMITATIONS.md (KL-13, KL-16, KL-39), crates/ezsdr-radio-uhd/src/provider/tx.rs (the repeat path, abandon ~:877-911, the underflow arm ~:1130-1150), crates/ezsdr-radio/src/staging.rs, segment.rs, crates/ezsdr-mock-radio/src/lib.rs (repeat, underflow). Report: path, recommendation in 5 lines, owner decisions list.

## brief-plan

# Brief — maintenance table 3 (a burst the X310 finds late) — test plan (plan only)

You are the implementer (Claude Code Sonnet subagent). Worktree `/home/komatsu/local_work/Ez-SDRv4-e3`, branch `work/mt3` (from main c04fda1); it builds into its own target (`.cargo/config.toml`); never set CARGO_TARGET_DIR to another tree's directory. This run writes the **test plan only**, to `/home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/test-plan.md`; no edits in any tree. Probes only in a copy outside the repository. Another agent works in the main tree `/home/komatsu/local_work/Ez-SDRv4`: never touch it.

## The step (risky: it changes the boundary between the fake and the hardware it stands for, and the identity a device report is matched to — AGENTS §9 step 1)
Design and decisions, all in `/home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/`: `draft.md` (Opus design: evidence §1, sequences S1–S5 §2, contract §3, tests §8), `fable-review.md` (Fable: M1–M3, m1–m3, cells §2.4), `decisions.md` (the owner's direction A and O1–O7; the orchestrator's A′). What lands:
1. FakeDevice: a late timed packet occupies the radio for its samples from `max(now, cursor)` (the X300 drain); every later packet of that DUC burst is late too and dropped through its EOB, one TimeError per channel per send, no BurstAck; a device burst handed behind it whose start falls inside the drain is late too.
2. UR-28 (A′): a TimeError(channel, t) retires every pending device burst on that channel whose first instant ≤ t, with one `TIME_ERROR { late_at_device }` per Kernel burst of it (joined and dropped-behind included; one event per burst on a multi-channel stream), `late_by_ns` = t − the device burst's first instant; an Underflow at t retires by the same reach silently; an abandon never touches `unacked`. The "exactly one candidate" rule and `late_reported` go.
3. UR-22: when that report retires the open device burst, the Module abandons it as after an underflow (same predicate and path); a repeat ends (send_asap included, O3); a switch on it is refused; its KL-39 region stays.
4. Texts the implementation will change (list them in the plan's amendments): UR-22, UR-28, UR-33 (the fake's claim: as strict as the X310 for the Module's shapes, except the cascade boundary, M3), RM-11 (holds per burst), KNOWN_LIMITATIONS (O6: a new device-behaviour entry; KL-13, KL-16, KL-39 lines), the stale records of draft §10.

## Sequences the plan must cover (each: the test, what it asserts, how it observes it, the wrong behaviour that fails it)
- S1: the fake at 1 MS/s, a booking whose first send reaches the device 1 ms late: nothing plays, no BurstAck, one late_at_device; today the chunk after plays (pin today's looseness first, then flip).
- S2: a late repeat ends (not fed until Stop); `async` bounded by what was handed; the switch on it refused; region stays.
- S3: a burst handed behind a late one inside the drain: dropped, its own late_at_device (on the fake; its boundary NOT pinned — M3/O7).
- S4: two Kernel bursts joined into one late device burst: an event for each.
- S5 / M2: an underflowed burst the host already closed gets TX_UNDERFLOW only, never late_at_device (report order scripted: Underflow then TimeError).
- M1: uhd-tx's own "timeline expired" abandon runs before the first TimeError is read: the wholly dropped burst still gets its event (a `tx.rs` unit that scripts the report order; the fake cannot show it).
- m1: two channels, a late start: one event per burst; one channel's report alone retires the burst.
- m2: a played burst whose ACK was lost is named late by a later TimeError at t ≥ its start (state what the rule does and whether a test pins it).
- Repeat with `send_asap` late (O3); Stop after a late abandon; the T3 open flush and #75's recovery after a late abandon (no interaction lost).
Hardware behaviour relied on: the whole-burst drop is VERIFIED in the FPGA source (draft §1.1 E2–E5) and at 1 MS/s on #1 (H1); the chunked shape, other rates, board #2, the cascade and one-channel-late are INFERRED (O7: bench after 11-15). Mark each test's reliance.

## Also give
- Every existing test that changes (draft §8), especially those pinning today's unique-candidate rule (`tx.rs:~3742`, `~3849`) and the FakeDevice rehearsals of hardware tests (AGENTS §9 "Hardware tests and bench values"): the hardware tests whose expectations come from these rules, and the audit table to fill (test, expectation, still right / fixed / cannot tell).
- Mutation rows (Rust token anchors, smallest unique text) for each rule above; which existing rows change.
- MockRadio: unchanged (it never emits late_at_device) — confirm no Mock test or parity test assumes otherwise.
- Sub-steps of ~1 500 lines max that each leave the tree green, and what you could not decide, with a recommendation.

Read first: AGENTS.md §3, §7, §9; design/18-uhd-radio.md (UR-22, UR-28, UR-30, UR-33), design/07-radio-model.md (RM-11, RM-14, RM-15), KNOWN_LIMITATIONS.md (KL-13, KL-16, KL-39), crates/ezsdr-radio-uhd/src/device.rs (FakeDevice tx path ~:700-800, ~:1940-2010), src/provider/tx.rs (~:340-360, ~:690-700, ~:870-915, ~:1080-1175), crates/ezsdr-radio/src/staging.rs (~:589), segment.rs (~:40).

Offloading to DeepSeek (reading only, here): Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast; you may hand it bulk reading (e.g. listing every test that names TimeError / late_at_device / unacked with path:line) at your judgment. It makes mistakes: check what it returns against the files. Foreground, one at a time; list what you offloaded.

Report: the plan's path, a short summary, open questions with your recommendation, anything INFERRED.

## brief-A

# Brief — maintenance table 3, sub-step A (FakeDevice: a late device burst drains whole)

You are the implementer (Claude Code Sonnet subagent). Worktree `/home/komatsu/local_work/Ez-SDRv4-e3`, branch `work/mt3` (at main 5121cdf); it builds into its own target (its `.cargo/config.toml`) — never set CARGO_TARGET_DIR to another tree's directory, and never touch the main tree `/home/komatsu/local_work/Ez-SDRv4`. You edit the worktree; you do **not** commit, stash, push or switch branches. Risky step (the boundary between the fake and the X310); its test plan was reviewed by Sol and re-checked. No self-review round: a fresh Opus reviewer reads the diff after you; Sol reviews the whole step at its end.

## The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/test-plan-v3.md` — this run does **sub-step A** only (§1.1 the drain model with the pad, the fake tests F0 (pin today's looseness, then flip), F1–F4, F3b, F3b2, F3c, rows MT3-01..06 and MT3-21..24 as the plan names them for A, the UR-33 text for the fake). Design and decisions: `decisions.md`, `draft.md`, `fable-review.md` in the same folder; binding: the last decisions.md entry (Sol v2 / re-check of v3) — for A: state B's size in F3b/F3b2/F3c (small, e.g. 100 samples, so B′ at L+1 000 stays clear of B's own drain); the UR-33 wording says the drain consumes one sample per `ratio` ticks and marks the pad's strictness as INFERRED; no test pins the cascade boundary to a device value. Do not do B (the Module) or C, and nothing of §9 (repeat). Background: AGENTS.md §3, §7, §9; design/18-uhd-radio.md UR-33.

## Exit checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p ezsdr-radio-uhd`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 5121cdf` (long; foreground).
- The new and changed fake tests 20 serial + 3 × 8 parallel via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>` (run from the worktree), 0 failures.
- Mutation: the new rows and every existing row whose filter selects a test you changed, through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` run from the worktree: all `killed`, none needing `"kill": "bound"`.
- Hardware rehearsals: the AGENTS §9 audit table (test, expectation, still right / fixed / cannot tell) for every FakeDevice rehearsal of a hardware test whose expectation goes through the fake's late path.

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only, except the design/18 UR-33 text named above. A mutation row that must change with your code may be retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a symbol, grep the mutation lists for rows whose `filter` or `old`/`new` names it, and fix them.
- Do not add tests that judge by wall-clock time. Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits, text updates — and keep your own effort for the hard parts. It makes mistakes (adds unasked mechanisms, misreads concurrency): read its diff and run the tests before you accept it. Foreground, one at a time, do not touch the tree meanwhile; list what you offloaded.

## Report (short)
`git diff --stat`, each check's counts, the rows' outcomes, the audit table, any deviation from the plan and why, what you offloaded, anything INFERRED.

## brief-B

# Brief — maintenance table 3, sub-step B (the Module: reports retire correlations, a late one-shot burst is abandoned)

You are the implementer (Claude Code Sonnet subagent). Worktree `/home/komatsu/local_work/Ez-SDRv4-e3`, branch `work/mt3` (main 5121cdf + sub-step A committed as 77b89ec); it builds into its own target (its `.cargo/config.toml`) — never set CARGO_TARGET_DIR to another tree's directory, and never touch the main tree `/home/komatsu/local_work/Ez-SDRv4`. You edit the worktree; you do **not** commit, stash, push or switch branches. Risky step (the boundary between the fake and the X310); its test plan was reviewed by Sol and re-checked. No self-review round: a fresh Opus reviewer reads the diff after you; Sol reviews the whole step at its end.

## The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/test-plan-v3.md` — this run does **sub-step B** only: UR-28 under A′ (a TimeError at t retires every pending device burst on that channel whose first instant ≤ t, one `late_at_device` per Kernel burst of it, joined targets included, per-channel retirement with a `told` mark so a multi-channel burst gets one event; an Underflow at t retires by the same reach silently; an abandon never touches `unacked`; the "exactly one candidate" rule and `late_reported` go), UR-22 (a TimeError that reaches the open one-shot device burst abandons it under the same predicate and path as an underflow), the Module tests (the new `tx/tests/mt3.rs`, U1–U13, Session tests T1–T3 in `tests/fake.rs`), the existing tests the plan changes (`tx.rs` ~:4372, ~:4387 and the others it lists), its rows, and the design/18 UR-22/UR-28 texts, RM-11's "holds per burst". Repeats: Q10 (b) — until the repeat-resume sub-step lands, a late repeat's correlation is reported and retired but the repeat is **not** abandoned. Nothing of §9. Do not do C (the C1 hardware cell, KL entries). Design and decisions: decisions.md, draft.md, fable-review.md in the same folder. Binding for B (decisions.md, the "Sol plan review of v2" entry): T2's wrong-behaviour column names the expiry abandon (tx.rs ~:398) as well; the `ReportHold` wrapper sleeps `timeout` when it returns None (no spin in the flush wait, core.rs ~:1369). The fake (sub-step A) is now stricter than the X310 at the drain boundary by construction: do not pin a cascade boundary to a device value. Background: AGENTS.md §3, §7, §9; design/18-uhd-radio.md UR-22, UR-28, UR-30, UR-33; design/07-radio-model.md RM-11.

## Exit checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p ezsdr-radio-uhd`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 5121cdf` (long; foreground).
- The new and changed tests 20 serial + 3 × 8 parallel via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>` (run from the worktree), 0 failures.
- Mutation: the new rows and every existing row whose filter selects a test you changed, through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` run from the worktree: all `killed`, none needing `"kill": "bound"`.
- Hardware rehearsals: the AGENTS §9 audit table (test, expectation, still right / fixed / cannot tell) for every FakeDevice rehearsal of a hardware test whose expectation goes through the late or underflow report path (TimeError / Underflow correlation, abandon).

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only, except the design/18 and design/07 texts named above. A mutation row that must change with your code may be retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a symbol, grep the mutation lists for rows whose `filter` or `old`/`new` names it, and fix them.
- Do not add tests that judge by wall-clock time. Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits, text updates — and keep your own effort for the hard parts. It makes mistakes (adds unasked mechanisms, misreads concurrency): read its diff and run the tests before you accept it. Foreground, one at a time, do not touch the tree meanwhile; list what you offloaded.

## Report (short)
`git diff --stat`, each check's counts, the rows' outcomes, the audit table, any deviation from the plan and why, what you offloaded, anything INFERRED.

## brief-C

# Brief — maintenance table 3, sub-step C (the C1 chunked-late hardware cell and its rehearsal; records)

You are the implementer (Claude Code Sonnet subagent). Worktree `/home/komatsu/local_work/Ez-SDRv4-e3`, branch `work/mt3` (main 5121cdf + sub-steps A 77b89ec and B af1324c); it builds into its own target (its `.cargo/config.toml`) — never set CARGO_TARGET_DIR to another tree's directory, and never touch the main tree `/home/komatsu/local_work/Ez-SDRv4`. You edit the worktree; you do **not** commit, stash, push or switch branches. Risky step (the boundary between the fake and the X310); its test plan was reviewed by Sol and re-checked. No self-review round: a fresh Opus reviewer reads the diff after you; Sol reviews the whole step at its end.

## The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/test-plan-v3.md` §3.4 and §8 row C — this run does **sub-step C without the KNOWN_LIMITATIONS entries** (they wait for the repeat-resume decisions): the pure verdict `late_chunked_verdict` (fixed order: FAIL on a BurstAck or a heard interval that OVERLAPS the late burst's interval with the loopback delay applied — not only "inside"; INCONCLUSIVE on absent or incomplete RX coverage of the interval plus one chunk each side, an `other` RX entry inside it, a control burst not heard, or no TimeError; PASS only with all evidence), its synthetic cases (including a straddling heard interval), the trial with an on-time control burst before the late one (an ACK attributed by tick: the control has its own), the FakeDevice rehearsal in the default suite, the `#[ignore]` hardware wrapper that FAILS on INCONCLUSIVE as the rehearsal does, rows MT3-25, MT3-26 and rows for the heard-interval FAIL, BurstAck FAIL and no-TimeError branches, and the stale records draft.md §10 names (maintenance table row 3 in plan/maintenance/audits-2026-10.md, plan/phase8/bench-results.md ~:459's H1 line, the 2026-11-13 session README's H1 column) — and the bench procedure: add the C1–C6 cells and C7 to plan/phase8/bench.md as a section for the bench after 11-15 (owner O7: not 11-15), with the attenuator confirmation before every transmitting command. Transmit only at 1 GHz (SAFE_TX_HZ), gain 0, slot A, as every other bench tool. Design and decisions: decisions.md, draft.md, fable-review.md in the same folder. Background: AGENTS.md §3, §7, §9 ("Hardware tests and bench values": the rehearsal books the same boundary on both sides); plan/phase8/bench.md's existing sections for the shape.

## Exit checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p ezsdr-radio-uhd`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 5121cdf` (long; foreground).
- The new and changed tests 20 serial + 3 × 8 parallel via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>` (run from the worktree), 0 failures.
- Mutation: the new rows and every existing row whose filter selects a test you changed, through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` run from the worktree: all `killed`, none needing `"kill": "bound"`.
- Hardware rehearsals: the AGENTS §9 audit table (test, expectation, still right / fixed / cannot tell) for every FakeDevice rehearsal of a hardware test that the new cell shares code with.

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only, except the records and bench.md named above. A mutation row that must change with your code may be retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a symbol, grep the mutation lists for rows whose `filter` or `old`/`new` names it, and fix them.
- Do not add tests that judge by wall-clock time. Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits, text updates — and keep your own effort for the hard parts. It makes mistakes (adds unasked mechanisms, misreads concurrency): read its diff and run the tests before you accept it. Foreground, one at a time, do not touch the tree meanwhile; list what you offloaded.

## Report (short)
`git diff --stat`, each check's counts, the rows' outcomes, the audit table, any deviation from the plan and why, what you offloaded, anything INFERRED.

## brief-D1

# Brief — maintenance table 3, sub-step D1 (repeat resume: the core)

You are the implementer (Claude Code Sonnet subagent). Worktree `/home/komatsu/local_work/Ez-SDRv4-e3`, branch `work/mt3` (main 5121cdf + sub-steps A 77b89ec, B af1324c, C 3660ec6 committed); it builds into its own target (its `.cargo/config.toml`) — never set CARGO_TARGET_DIR to another tree's directory, and never touch the main tree `/home/komatsu/local_work/Ez-SDRv4`. You edit the worktree; you do **not** commit, stash, push or switch branches. Risky step (a queue, a deadline and a lifecycle: a repeat that resumes); its §9 test plan was reviewed by Sol and re-checked twice. No self-review round: a fresh Opus reviewer reads the diff after you; Sol reviews the whole step at its end.

## The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/test-plan-v6.md` §9 — this run does **sub-step D1** exactly as §9.7 lists it (the resume core: `busy` with the stream's record, triggers V1–V3, R, the resuming state with H and R apart, the four `!first` sites and the SOB's own first instant, `cursor()`, `resumes_at` on the event and the schemas, the `timing` row, split records, the stutter rule b′, the repeat exemption and the removal of the Q10 (b) interim, the carry (b) join check with its debug assertion, the D1 tests, the D1 rows, the retargeted repeat tests, the D1 design texts UR-22/23/28/30 and RM-11/22). Not D2 (the switch rule across a gap, the missed R, M5, RT16–RT18, RT21, EA-16, KL texts, audit, records). The decisions bind: decisions.md in the same folder (authoritative; the owner decided R's formula with m = 10 ms, b′ with equality resuming, split records, triggers and endings, `resumes_at` plus a timing row), and §9.8 Q-D1..Q-D8 are decided as recommended. Background: addendum.md, fable-addendum-review.md, sol-plan9.md and the two re-checks recorded in v5/v6's change lines; AGENTS.md §3, §6 (no versions, no SCHEMA_CHANGELOG; regenerate a changed schema and keep `schema_freeze` green), §7, §9; design/18-uhd-radio.md, design/07-radio-model.md. The rig trap in §9.0/§9.1: R must not land inside the fake's drain by accident — build each geometry as the plan computes it, and keep hand-computed R values as literals. If the code's real numbers differ from a plan literal, recompute with the plan's formulas and say so in the report.

## Exit checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p ezsdr-radio-uhd` and every crate whose schema or tests you touch (`schema_freeze` green), `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 5121cdf --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/renames.txt` (append any rename you make to that file and say so) (long; foreground).
- The new and changed tests 20 serial + 3 × 8 parallel via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>` (run from the worktree), 0 failures.
- Mutation: the new D1 rows and every existing row whose filter selects a test you changed or whose anchor your code moves, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` (simq waits for any full run to finish) run from the worktree: all `killed`, none needing `"kill": "bound"`.
- Hardware rehearsals: the AGENTS §9 audit table for every FakeDevice rehearsal of a hardware test whose expectation goes through a repeat's late or underflow path (plan §9.6 lists them).
- Ordinary cargo test/clippy run locally (this host has 16 cores); only the 20 + 3 × 8 repetitions may go through rdev.

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only, except the design/18, design/07 and schema texts D1 names. A mutation row that must change with your code may be retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a symbol, grep the mutation lists for rows whose `filter` or `old`/`new` names it, and fix them.
- Do not add tests that judge by wall-clock time. Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits, text updates — and keep your own effort for the hard parts. It makes mistakes (adds unasked mechanisms, misreads concurrency): read its diff and run the tests before you accept it. Foreground, one at a time, do not touch the tree meanwhile; list what you offloaded.

## Report (short)
`git diff --stat`, each check's counts, the rows' outcomes, the audit table, any deviation from the plan and why, what you offloaded, anything INFERRED.

## brief-D2

# Brief — maintenance table 3, sub-step D2 (repeat resume: switches across a gap, a missed R, texts)

You are the implementer (Claude Code Sonnet subagent). Worktree `/home/komatsu/local_work/Ez-SDRv4-e3`, branch `work/mt3` (main 5121cdf + A 77b89ec, B af1324c, C 3660ec6, D1 committed at HEAD); it builds into its own target (its `.cargo/config.toml`) — never set CARGO_TARGET_DIR to another tree's directory, and never touch the main tree `/home/komatsu/local_work/Ez-SDRv4` (another lane works there). You edit the worktree; you do **not** commit, stash, push or switch branches. Risky step; its §9 plan was reviewed by Sol and re-checked. No self-review round: a fresh Opus reviewer reads the diff after you; Sol reviews the whole step at its end.

## The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/test-plan-v6.md` §9 — this run does **sub-step D2** as §9.7 lists it: the switch rule across a gap (V17: a switch whose boundary falls in the gap moves to the next safe period boundary — the owner's decision "Switch across a gap"), the missed R and its event rule (V18), M5 (V19), V22 and RT21 (carry (c): `resumable()` must not read a report in the join window as "no open booking" — moved from D1), RT16, RT17, RT18, RT21, rows RV-17, RV-21..RV-28, RV-31, RV-32, UR-38, RM-15/37, EA-16 and the `tx.switch()` docstring (a switch is no longer refused in a gap), the KL texts (KL-13 and any KL the resume changes; decisions.md "KL and bench" binds the wording), the hardware audit entries (§9.6; bench cell C7 itself stays with the bench-tools brief), and the records the plan names. The decisions bind: decisions.md in the same folder, including its "2026-11-14 D1 review" entry:
- **D1 review M1** (must be closed by D2): today a switch whose boundary falls in the gap reaches the `stale` branch (`tx.rs` ~:463-470): a debug panic; in release the switch is refused but its record row stays `pending` and blocks every later switch. After D2 the switch moves to the next safe boundary per V17; add a test of exactly that sequence (book a repeat, hold a switch with boundary in [H, R), the first send late, report read, pause, `step()`), asserting the switch plays from its moved boundary, its record row settles, and a later switch is accepted.
- **Missed R:** the check must compare R with `now + lead`, not with `now`: a wake inside [R − lead, R) must count as missed (else the part's SOB goes out late, the device drops it, and the zero case ends the repeat).
- **Texts made false by D1 and owed by D2:** RM-37's cancelled clause ("an underflow, an expired timeline"), RM-15, KL-13 ("an underflow ends the burst", "abandoning … kept by the owner").
Background: addendum.md, fable-addendum-review.md, sol-plan9.md; AGENTS.md §3, §6, §7, §9; design/18-uhd-radio.md, design/07-radio-model.md, KNOWN_LIMITATIONS.md, the Python client's `tx.switch()` docstring (grep `def switch`).

## Exit checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (`schema_freeze` and the cross-crate scans green), the Python client suite if you touch `python/` (AGENTS §7: PYTHONDONTWRITEBYTECODE=1, venv ~/.cache/ezsdr-venv, EZSDR_SERVER built from this tree), `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 5121cdf --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-14-mt3/renames.txt` (append any rename you make to that file and say so) (long; foreground).
- The new and changed tests 20 serial + 3 × 8 parallel via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>` (run from the worktree), 0 failures.
- Mutation: the new D2 rows and every existing row whose filter selects a test you changed or whose anchor your code moves, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` (simq waits for any full run to finish) run from the worktree: all `killed`, none needing `"kill": "bound"`.
- Hardware rehearsals: the AGENTS §9 audit table for every FakeDevice rehearsal of a hardware test whose expectation goes through a repeat's late or underflow path (plan §9.6 lists them).
- Ordinary cargo test/clippy run locally (this host has 16 cores); only the 20 + 3 × 8 repetitions may go through rdev.

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only, except the design/18, design/07, KNOWN_LIMITATIONS.md, EA-16 spec text and docstring D2 names. A mutation row that must change with your code may be retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a symbol, grep the mutation lists for rows whose `filter` or `old`/`new` names it, and fix them.
- Do not add tests that judge by wall-clock time. Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits, text updates — and keep your own effort for the hard parts. It makes mistakes (adds unasked mechanisms, misreads concurrency): read its diff and run the tests before you accept it. Foreground, one at a time, do not touch the tree meanwhile; list what you offloaded.

## Report (short)
`git diff --stat`, each check's counts, the rows' outcomes, the audit table, any deviation from the plan and why, what you offloaded, anything INFERRED.
