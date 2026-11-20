# #125 and #127 — MockRadio no looser than the UHD Module (lane 1) — Sonnet, implement directly

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n125` (already created from `main` ea7bc5c1). Do not commit, stash, push, reset or switch branch. Never touch `/home/komatsu/local_work/Ez-SDRv4-n111`. Work dir `/home/komatsu/.cache/ezsdr-fixes/2026-11-20-n125/`.

**Risk: not risky** (an admission check made equal in both Providers; a constant moved). No test plan; GPT-6.1 Terra and an Opus re-check reviewer review the diff. Read `gh issue view 125`, `gh issue view 127`, the inventory `/home/komatsu/.cache/ezsdr-fixes/2026-11-20-mockhw/classification.md`.

## #125 — the waveform bytes check
- Today MockRadio reads a `tx_burst`'s waveform bytes only in channel mode (`crates/ezsdr-mock-radio/src/lib.rs` ~1670; `ctx.inputs` is taken only into `ChannelMode` ~2302); the UHD Module refuses at receipt bytes that are not an input, a length other than `size_bytes`, and a non-finite sample (`crates/ezsdr-radio-uhd/src/provider/control.rs` ~437, ~446).
- Make one shared check in `crates/ezsdr-radio` (RM-13 holds the refusals both Providers make; place it beside the existing shared burst checks) that both Providers call at receipt, in every mode, with the same reason text (one `RM-13: …` text per case; the old `UR-21: …`/`MR-32: …` texts for these three cases go — list every test, fixture and doc that matched them and update them). MockRadio keeps `ctx.inputs` in every mode for this; channel mode still decodes for the medium as today (no double decode if avoidable).
- `design/07` RM-13 states the three refusals once (rule format, AGENTS §2); `design/09` MR-32 and `design/18` UR-21 cite RM-13 instead of restating. KNOWN_LIMITATIONS: only if an entry says the Mock accepts these (grep).
- Tests at the pure level for the shared check (each case), and one per Provider showing the refusal at receipt outside channel mode (the Mock case is the one that failed before; say that it fails on the old code).

## #127 — one delivery allowance
- The 3 ms is `crates/ezsdr-mock-radio/src/profile.rs` ~151 (`delivery_allowance_ns`, X3xx arm) and `crates/ezsdr-radio-uhd/src/profile.rs` ~16 (`DELIVERY_ALLOWANCE_NS`, with `DEVICE_LEAD_NS` 2 ms and a test that their sum is the x3xx command lead). Put the X3xx delivery allowance once in `crates/ezsdr-radio/src/x3xx.rs` (beside `timing()`), both Providers use it; the device lead stays derived (command lead − allowance) on both sides; the UHD test that ties them stays (moved if needed). `ideal` keeps 0.

## Rows and checks
- Mutation rows: one per decision (each refusal case in the shared check; the Mock calling it outside channel mode; the shared allowance constant if a test pins it), smallest anchors, in `plan/maintenance/tools/mutations.json`'s maintenance list; retarget any row anchored on removed text to the same decision. Run touched/new rows via `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2,sim03:2 <ids>`; read a killed row's log line if available.
- Before reporting: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `plan/maintenance/tools/remote/rdev.py -- cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`, `cargo test --workspace` (local or rdev), the Python suite if a refusal text it matches changed (`PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`, `EZSDR_SERVER` built from this tree), `python3 plan/maintenance/tools/rules.py --base ea7bc5c1`.
- Rules: mutation rows only via `remote_mutate.py` under `simq`; never `mutate.py` locally; build output never in the tree; temp files on /tmp; kill by PID; chmod 644 new files.

## Report
Japanese, short: diff stat; each issue (what, refusal texts changed and where); tests (which failed on the old code); rows; INFERRED; open questions.
