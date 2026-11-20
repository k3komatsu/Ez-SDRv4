# #126 — a Block link on a radio rx port: one meaning in both Providers (lane 1) — phase A: test plan only

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n126` (from `main` 5bc555d3). Do not commit, stash, push, reset or switch branch. Never touch `/home/komatsu/local_work/Ez-SDRv4-n111` (another lane). Work dir ``.

Read `gh issue view 126 -R k3komatsu/Ez-SDRv4 --comments` (the two owner decisions), `plan/phase8/54-mock-vs-x310.md` (row RX4), AGENTS.md §9 (what a test plan holds).

**Risk: risky** (a queue/back-pressure behaviour, the boundary between MockRadio and the hardware it stands for, a Kernel spec clause). AGENTS §9 steps 2–3: **phase A writes the test plan only, read-only on the tree**; Sol reviews it; phase B implements.

## The decided meaning (owner, 2026-11-20)
A `Block` link on a radio rx port is admitted by both Providers. While the link answers `Full`, the Provider keeps reading the device and discards what it reads (no device overflow, no UHD restart, no restart gap). The next block the link accepts carries `GAP_BEFORE` with `lost` = every sample discarded since the last accepted block (plus any `lost` and carried flags the discarded blocks had). Reporting: one Kernel `LINK_BACKPRESSURE` per receive segment, at its first discard (default reaction continue + mark artifact; a Policy may override); the total discarded samples, those pending at a segment's end included, in one stats count (`rx_link_discarded_samples`, both Providers' stats). No `RX_OVERFLOW` for a discard (no device overflow happened). Not one event per stretch (#128: the Kernel's delivered list has no total bound).

## What changes (VERIFIED in code at 5bc555d3 unless marked)
- MockRadio: `crates/ezsdr-mock-radio/src/lib.rs` ~697–716 (publish; a `Full` becomes MR-19's emulated overrun with the restart gap, `GAP_BEFORE | RESTARTED`, an `RX_OVERFLOW`) and the overrun path it drives; ~2208–2225 (MR-7 step 7: a `Block` link must be its port's only link). Spec: design/09 MR-19 (rewrite), MR-7 step 7, MR-25/MR-37 sentences that cite MR-19, table row M7 (~277).
- UHD Module: `crates/ezsdr-radio-uhd/src/provider/mod.rs` ~513 refuses any `Block` link (UR-12) — goes; `provider/rx.rs` ~741 publishes to every link of the port and counts a non-`Accepted` in `stats.link_drops_seen`. The block's `GAP_BEFORE`/`lost` comes from the device tick against the expected index (design/18 ~238): check whether a discarded block must leave the expected index unmoved (so the next accepted block's jump is the discarded span) or must be booked explicitly, and how pending flags (UR-18/UR-19, `RESTARTED` from a real overflow) carry past a discarded block. Spec: design/18 UR-12 (Block admitted, the one-Block-link-per-port rule), UR-17/the publish paragraph (~238), the test table row of `ur_12_prepare_refusals`.
- One rule both call: the link admission "a `Block` link must be its port's only link" and the discard accounting belong in `crates/ezsdr-radio` once (mech `radio-rule-implemented-twice`); the plan says where and what each Provider keeps.
- Kernel spec design/02 SC-20a: the clause "where its device cannot wait for a consumer (UR-12), it records the refused block's loss as its device's overrun …" becomes the discard meaning; its `*Test:*` line (`mr_19_an_abort_after_a_refusal_loses_nothing_before_its_cut`).
- `LINK_BACKPRESSURE` from a Provider: today the coordinator registers the Kernel kinds only for `EventSource::Kernel` and a Provider's nodes get the Module's own kinds plus `DEVICE_LOST` (`crates/ezsdr-kernel/src/coordinator/pipeline.rs` ~468–500). A Provider emitting it from its stream node `<device>/rx/s<k>` would fall into RS-33's unforeseen row. The plan says how the pair is registered (a Kernel change: e.g. `(node, LINK_BACKPRESSURE)` for an instance with a `Block` StreamOut attached) and which spec rule states it (RS-27/RS-33/KC-8 area) — INFERRED, verify.
- Tests that use `Block` today (list them all: `grep -rn "BackPressure::Block" crates/*/tests crates/*/src`), at least `crates/ezsdr-mock-radio/tests/mock_layout.rs` (~726–813), `mock_radio.rs`, `crates/ezsdr-radio-uhd/tests/fake.rs`, `crates/ezsdr-kernel/tests/stream_contract.rs` (`sc_20a_*`); audit table per AGENTS §9 step 1 for each test whose expectation came from MR-19's overrun meaning (still right / fixed / cannot tell).

## Sequences (expected results; one link `Block` on rx0 unless said)
1. Link full for blocks [a,b) and [b,c), accepts at [c,d) → blocks a and b discarded; block c carries `GAP_BEFORE`, `lost = c − a`, no `RESTARTED`; no `RX_OVERFLOW`; one `LINK_BACKPRESSURE` at a (stream node source, time = a on the segment clock); stats `rx_link_discarded_samples += c − a`; `rx_blocks` counts only accepted blocks.
2. A second stretch in the same segment → its gap on the next accepted block; no second `LINK_BACKPRESSURE`; stats add.
3. A new segment (Stop then Start) with a stretch → one more `LINK_BACKPRESSURE` (per segment).
4. A stretch pending when the segment ends (Stop at a sample inside the stretch, no block accepted after) → no event beyond the segment's one; stats include the pending samples up to the cut; the segment's end/tail rows as today; nothing lost before an abort's cut (SC-20a's last clause stays true).
5. A discarded block that carried flags/lost from a real device event (a scripted overflow's `RESTARTED`, a sequence error, MR-21/MR-22 on Mock, UR-18/UR-19 on UHD) → the next accepted block carries those flags and the summed `lost`; the device event's own `RX_OVERFLOW` still comes once, with the block that carries its loss (RM-17).
6. A segment whose first block is refused → its clock is registered (MR-25 today) and the first accepted block carries the gap from the origin.
7. Two links on one port, one of them `Block` → refused on both Providers with one reason text; two ports each with one `Block` link → admitted on both.
8. A `Drop*` link beside nothing → unchanged behaviour (regression).
9. MockRadio and UHD (FakeDevice) give the same blocks, flags, `lost`, event and stats for sequences 1, 2, 4, 5 — a differential test on the shared rig if one exists (VH-8 differential rig), else one test per Provider with the same expectations from one table.
10. UHD: while discarding, uhd-rx keeps calling `recv` at the device rate (no device overflow is induced by the discard itself) — FakeDevice observes no overrun during a long stretch.

## Phase A deliverable
`test-plan.md`: for each sequence (and any the code shows, e.g. a fault inside a stretch, a retune/restart inside a stretch, `continue` Policy), the test (new or changed), rig, what it asserts, how it observes it, the wrong behaviour that fails it (the old MR-19 must fail 1, 4 and 5; the old UR-12 refusal must fail 7's admission), timing dependence, mutation rows planned (Rust token anchors, smallest unique text; exact substrings elsewhere). Then: the shared rule's place in `ezsdr-radio`; the Kernel registration change and its spec rule; every spec sentence to change (design/02 SC-20a, 09 MR-7/MR-19/MR-25/MR-37/M7, 18 UR-12/UR-17 publish paragraph, 04/06 for the event registration), with draft text in rule format (AGENTS §2); the audit table of existing Block tests; KNOWN_LIMITATIONS impact (grep). Use the hardware/UHD facts marked VERIFIED or INFERRED with sources.

Read-only in phase A. If you build or run tests to check a fact: `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, temp files on /tmp.

Report (Japanese, short): the plan's path, sequences added, open questions (design questions the plan cannot settle go to the orchestrator).

## Phase B brief

# #126 phase B — implement (lane 1) — Sonnet writes production, DeepSeek writes tests

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n126` (= main 5bc555d3; base for checks: 5bc555d3). Do not commit, stash, push, reset or switch branch. Never touch `/home/komatsu/local_work/Ez-SDRv4-n111` (another lane). Work dir ``.

Read, in order: `brief.md`, `plan-decisions.md`, `test-plan.md` (v2), `sol.md`, `recheck-dispositions.md` — the last one **amends the plan and binds** (re-decided Q8, Q9 and the event order; plan fixes 1–9; item 10 goes to the code review).

## Division of work (owner 2026-11-20)
1. **You (Sonnet) write the production code and the spec text yourself**: `crates/ezsdr-radio` (RM-38's shared rule and accounting), MockRadio, the UHD Module, the Kernel registration of `(node, LINK_BACKPRESSURE)` for a Provider with an attached `Block` StreamOut, and the spec drafts (design/02, 04/06, 07, 09, 18) in rule format (AGENTS.md §2) with `*Test:*` lines and *Changes* rows citing issue 126. Keep it building (`cargo check --workspace --all-targets`) before step 2.
2. **Then Go DeepSeek writes the tests** from the plan (new tests, changed existing tests per the audit table, golden updates), the mutation rows (Rust token anchors, smallest unique text, in `plan/maintenance/tools/mutations.json`'s maintenance list; retargets/removals as the plan says), and runs the checks. One DeepSeek task at a time; you never edit the tree while it runs.
   - Tasks `ds-<n>-task.md`, logs `ds-<n>.log` in the work dir; a task file must not start with `---`. Launch: `cd /home/komatsu/local_work/Ez-SDRv4 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`; real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`; wait with `timeout 590 tail --pid=<PID> -f /dev/null` (Bash tool timeout 600000), repeat until it exits.
3. **You review DeepSeek's tests** with `/home/komatsu/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md` (per test: the smallest wrong change it must catch and that it fails on the named assertion, not a timeout; the assertion proving it reaches its named state; for thread/hook tests the worst descheduling point). Send back at most twice; fix small things yourself.
4. Failing-first: each new test that the plan says fails on the old code is shown failing on it (key line per test in the report) — the old code is the `main` version of the file(s), checked in a copy outside the tree or by the mutation rows that restore old behaviour.

## Checks before reporting
`cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `plan/maintenance/tools/remote/rdev.py -- cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`; `cargo test --workspace` (local or rdev); the Python suite if a refusal/event text it matches changed (`PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`, `EZSDR_SERVER` from this tree); timing-dependent tests 20 times; `python3 plan/maintenance/tools/rules.py --base 5bc555d3`; every new/touched row via `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase3 sim02:2,sim03:2 <ids>` (the sim lock may be held by another lane's run; simq waits). All rows killed (`kill: bound` only where the plan says).

Rules: mutation rows only via `remote_mutate.py` under `simq`; never `mutate.py` locally, never a mutant in the tree; build output never in the tree; temp files on /tmp; kill by PID only; chmod 644 new files. Do not touch `plan/maintenance/failure-mechanisms.md` or `KNOWN_LIMITATIONS.md` (no KL entry, owner).

Report (Japanese, short): diff stat; production lines you wrote vs test/row lines DeepSeek wrote (for the comparison log); each DeepSeek round and send-back with findings; failing-first lines; rows and outcomes; spec rules changed; deviations from the plan with reasons; open questions.

## Plan decisions

# #126 test plan v1 — orchestrator's answers to §9 (2026-11-20; binding for the plan and phase B)

- Q1: **device-only `lost` in `RX_OVERFLOW`** (not the plan's D3). The owner decided that a discard is not a device overflow (no `RX_OVERFLOW` for a discard; discards are counted in `rx_link_discarded_samples`), so an `RX_OVERFLOW` reports only the device's loss, stamped as today (RM-17), even when a discard stretch lies between the device event and the carrying block. The carrying block's `lost` is still the whole jump (device loss + discards). This needs the device's pending loss kept apart from the discard count in both Providers; the plan adds the corner test on both.
- Q2: payload `{}`; the source (the stream node `<device>/rx/s<k>`) names the port.
- Q3: accepted. `rx_link_discarded_samples` counts what the Provider read and discarded; under an abort it may exceed the cut and depend on the abort's timing; asserted per Provider, not across. The rule text says so.
- Q4: `Full` is not counted in `link_drops_seen` (it has its own count); the UHD publish paragraph says so.
- Q5: follow each Provider's existing convention for its stats keys (present from construction if the others are, else when nonzero); the plan states which and updates the goldens accordingly.
- Q6: narrow — register `(node, LINK_BACKPRESSURE)` only for a Provider with an attached `Block` StreamOut; keep the negative test.
- Q7: no Run-level acceptance test now; the K tests plus the Provider tests cover the chain.
- Q8: keep the literal UR-17 rule (the plan's U4s/U7).
- Q9: the plan's `read` + `jumped`.
- Q11: accept the documented window in SC-20a's text.
- Q10: no KNOWN_LIMITATIONS entry (owner 2026-11-20 「blockの意味そのものなのでokです」): discarding while full is the documented meaning of `Block` on a radio rx port.
