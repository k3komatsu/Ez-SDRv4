# #101 — briefs (orchestrator → Claude Code Sonnet 5.5 subagent, effort high)

## Test plan run


You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n101` (from main 89dc141). You edit the tree; you do **not** commit, stash, push or switch branches. **Risky step** (a lifecycle — the device's loss — and the classification at the fake/hardware boundary). This run writes the **test plan only** (AGENTS.md §9 step 2): no code. Sol reviews the plan next; a later run implements it.

## What is decided (all binding; record: `/home/komatsu/.cache/ezsdr-fixes/2026-11-14-n101/decisions.md`, read all of it; issue: `gh issue view 101`)
1. **The rates check** (`provider/core.rs` `check_rates` ~:1120-1150) publishes a lost `rates` read (UR-29) instead of refusing with a plain string; a lost outcome is never cached as a permanent refusal (today `:1128` caches a `String`).
2. **`prepare`'s configure** (`provider/mod.rs` ~:530 `map_err(|e| rejected(e.message))`) publishes a lost `apply`.
3. **The transmit open at `arm`** (`provider/mod.rs` ~:574) goes through the same path as the receive open (`refuse_open`, `mod.rs:59`, since #67): a lost open publishes `DEVICE_LOST` before `arm` refuses, so `cleanup` makes no device call.
4. **`ref_locked` at `arm`** (`provider/mod.rs` ~:552) the same.
5. **Classification** (`src/uhd.rs` `check_with` :264-270): at `uhd_usrp_get_rx_stream` and `uhd_usrp_get_tx_stream` only, `UHD_ERROR_EXCEPT` (47) also counts as lost, besides IO/USB/OS and RUNTIME (owner 2026-11-14). Every other call keeps today's rule (recv/send/async/stream commands/metadata make unchanged). Evidence (VERIFIED in UHD 4.10 source and bench): decisions.md "UHD 4.10 source check" and "Facts".
6. **One conversion** (the design-note answer for the ninth `mech:failure-collapsed-into-value` bug, AGENTS §6; `plan/maintenance/failure-mechanisms.md` row): every refusal the Provider makes from a `DeviceError` in `prepare`, `arm` and the owners' opens passes through one function that publishes a loss first (generalise `refuse_open`); no second copy of the "if lost then device_lost" decision. The plan lists every such site with the grep that found it (the fact list in decisions.md is the starting point: sites 1–4; bind-time sites 5–7 stay refusals because no `Core` exists yet — say so, do not change them).
Out of scope: uhd-control's UR-29 rule, the streaming calls' classification, bind-time sites, #75, anything of maintenance table 3 (the worktree `Ez-SDRv4-e3` is another lane: do not touch it).

## The plan to write
`/home/komatsu/.cache/ezsdr-fixes/2026-11-14-n101/test-plan.md` (you may write this one file under `~/.cache/ezsdr-fixes/`). For each sequence: operations, expected result, the test (name, file, rig), what it asserts and how it observes it, the wrong behaviour that fails it, and its mutation row (Rust token anchors, smallest unique text; `plan/maintenance/tools/mutations.json` conventions). At least:
- each of sites 1–4 with a lost error on the FakeDevice (`FakeFault::Lost(t)` or a scripted lost error — find what the fake offers; if a site cannot be made lost on the fake, say what the fake needs, smallest addition): one `DEVICE_LOST`, published before the refusal, `mark_lost` before any `close_streams`, `cleanup` makes no device call, the Run ends as the receive case does today (`ur_29_a_loss_found_by_the_stop_on_open_at_arm_is_published_once_and_cleanup_closes_nothing`, `mod.rs:~1005`, is the model);
- the same sites with a non-lost error: today's refusal unchanged (text, kind), no `DEVICE_LOST`;
- the rates cache: a lost read is not cached as a refusal for a later open (and what a later open sees after the loss);
- a transmit-only Run and a Run with a receive link (the receive open comes first);
- classification: a feature-`uhd` shim test next to the existing ones (`uhd.rs` ~:1590-1620) that 47 at `get_tx_stream` and `get_rx_stream` is lost, 47 at another call (e.g. `get_time_now`, `tx_send`) is not, RUNTIME unchanged; this test runs only with `--features uhd` (through `rdev.py`, the image has libuhd) — say so, it is outside the mutation gate (handoff §3 "feature `uhd` の mutation");
- the hardware tests and rehearsals whose expectations go through these paths (AGENTS §9 "Hardware tests and bench values"): audit table (test, expectation, still right / fixed / cannot tell), e.g. the B9 unplug tests in `tests/hardware.rs`;
- the design/18 texts that change (UR-29's lost list and its `*Test:*` line, UR-13 if the refusal text changes, the "`UhdDevice` sets `lost` for ..." sentence at :343) and KNOWN_LIMITATIONS entries, if any, that mention a refusal at arm for a lost device (grep).
Read: AGENTS.md §6, §7, §9; design/18-uhd-radio.md UR-13, UR-25, UR-29, UR-34; `provider/mod.rs` prepare/arm/cleanup; `provider/core.rs` check_rates/device_lost/device_failed; `src/uhd.rs` check_with and its shim tests; `src/device.rs` the fake's faults.

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`; never run `mutate.py` locally, never apply a mutant to the tree. (This run writes no rows into the lists, only into the plan.)
- Files under `~/.cache/ezsdr-fixes/` (except the plan file above), `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only.
- Do not add tests that judge by wall-clock time. Do not plan mechanisms the decisions do not name.
- No build output inside the tree; test temp files on /tmp.

## Report (short)
The plan's path, the site list with its grep, open questions each with a recommendation, anything INFERRED.

## Implementation run


You are the implementer (Claude Code Sonnet subagent), the same role as in `brief-plan.md` in this folder (read it: tree, branch `work/n101`, decisions, scope, rules). Now: first amend `test-plan.md` → `test-plan-v2.md` (copy; one "v2 changes" line), then implement it.

## Amendments (Sol's plan review `sol-plan.md`, all adopted; decisions.md binds)
- T1: the fault must not fire at setup (`owner_with` opens TX first, tx.rs:6277-6282). Use an nth-call or armed-after-setup variant of the fake fault (smallest addition), assert setup succeeded and the recovery `tx_open` was reached; keep the non-lost twin. Q9 is adopted: a booked `Provider::stop` must not suppress the loss publication at tx.rs:855-858.
- Q5 rejected: every coordinator-level row asserts exactly one delivered `DEVICE_LOST` in the Manifest (the coordinator drains events at cleanup: crates/ezsdr-kernel/src/coordinator/ending.rs:144-147, 263, 414-419). If a row has none, investigate and report; do not weaken the oracle.
- Q3 (owner): `rates_checked` is **false** for a direction whose rates read was lost; assert it in the actual Manifest, not only Core state.
- Q1, Q2, Q4, Q6, Q8 as recommended. Q7: report what KL-2 (or any KL) would need; do not edit KNOWN_LIMITATIONS.md.

## Exit checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 89dc141` (long; foreground). Locally.
- Feature `uhd`: K1/K2 through `python3 plan/maintenance/tools/remote/rdev.py -- cargo test -p ezsdr-radio-uhd --features uhd <filter>` (and clippy with `--features uhd --tests -- -D warnings`); show K2 red before the classification change and green after (in a copy outside the repo, or by reasoning if not practical — say which).
- New timing-dependent tests 20 serial (local).
- Mutation: the plan's new rows and the re-anchored ones through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>`: all killed, none `"kill": "bound"`. Save the log as `~/.cache/ezsdr-fixes/2026-11-14-n101/rows.log`.
- design/18 texts per the plan §7 (UR-12, UR-13, UR-29, UR-30's `rates_checked`, UR-33, the §6 row, a Changes row).

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`; never run `mutate.py` locally, never apply a mutant to the tree. A row that must change with your code may be retargeted in `plan/maintenance/tools/**/mutations*.json`; say so.
- Files under `~/.cache/ezsdr-fixes/` (except the plan files in this folder and rows.log), `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only.
- Do not add tests that judge by wall-clock time. Do not plan mechanisms the decisions do not name.
- No build output inside the tree; test temp files on /tmp.

- Wait in the foreground for every long command. Kill processes by PID, never by a name pattern. Do not delete a test of a reachable sequence without a replacement; grep mutation rows for any symbol you rename. After `cargo fmt`, `rules.py` must still find every row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) is cheap for precisely described repetitive edits; read its diff and run the tests; foreground, one at a time; list what you offloaded.

## Report (short)
`git diff --stat`, each check's counts, rows' outcomes, the audit table, deviations and why, what you offloaded, anything INFERRED.
