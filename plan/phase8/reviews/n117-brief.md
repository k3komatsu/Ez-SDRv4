# #117 — briefs

## #117 — three receive-path device calls under the device gate (lane 1) — Sonnet, implement directly

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n117` (already created from `main` 50ef353d). Do not commit, stash, push, reset or switch branch. Never touch `/home/komatsu/local_work/Ez-SDRv4-n111` (another lane). Work dir: `/home/komatsu/.cache/ezsdr-fixes/2026-11-20-n117/`.

**Risk: not risky** — it applies an existing, reviewed mechanism (`Core::admitted`, UR-29) to three more call sites; no new state, lock or lifecycle. No test plan; GPT-6.1 Terra and an Opus re-check review the diff.

Read: `gh issue view 117`; `crates/ezsdr-radio-uhd/src/provider/core.rs` ~1552–1567 (`admitted`, `rx_stop_admitted`, `rx_start_admitted`) and the existing tests that hook `rx_first_stop_before_gate` / `rx_stop_before_gate` / `rx_start_before_gate` (find them: `grep -rn before_gate crates/ezsdr-radio-uhd`); `design/18-uhd-radio.md` UR-15, UR-17, UR-29.

## The three calls (line numbers on 50ef353d)
1. `provider/rx.rs` ~441: `chain.begin_result(segment, origin, self.core.device.rx_start(self.k, origin))` — the start of a segment.
2. `provider/rx.rs` ~459: the failure-path `self.core.device.rx_stop(self.k)` after `declare_sample_clock` fails.
3. `provider/mod.rs` ~676: the first start of each later RX stream in `start` (`self.device.rx_start(stream.k, t0)`), after the `core.is_lost()` check at ~668.

For each: route it through the admitted helpers so a loss published between the caller's check and the call makes no device call (UR-29). Decide, per site, what `None` (lost, no call) leads to, matching what the code already does when it sees a loss just before (1: the `is_lost` branch above it — finish and return; 2: chain finished, no stop; 3: the `is_lost` branch at ~668 — `failed` with the UR-15 message, break). If `rx_start_admitted`'s fixed probe name cannot tell site 1 from site 3 in a test, give the helper a `before_gate` argument like `rx_stop_admitted` (one probe name per site). No other change.

Tests: one pre-gate test per site in the existing style (a test holds the site's probe, publishes a loss, releases; asserts the device saw no call of that kind after the loss and the outcome above). If a site's failure path cannot be reached without a new production hook (site 2 needs `declare_sample_clock` to fail), say so and do not add the hook; the mutation row then stands alone only if a test kills it.

Mutation rows: one per site, smallest anchor (the admitted call → the bare device call), in the list where the existing UR-29 rows live (find them by `rx_stop_admitted`/`rx_start_admitted` in `plan/maintenance/` lists). Each row names its killing test. Run them, plus the existing rows on `admitted`/`rx_*_admitted`, through `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:2,sim03:2 <ids>`.

## Rules
- Mutation rows only through `remote_mutate.py` via `simq`; never run `mutate.py` locally or apply a mutant to a tree. Build output never in the tree; temp files on /tmp. Kill by PID only. chmod 644 files you create.
- Before reporting: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `plan/maintenance/tools/remote/rdev.py -- cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`, `cargo test -p ezsdr-radio-uhd` (the new tests 20 times each — they use threads), `python3 plan/maintenance/tools/rules.py --base 50ef353d`.
- Update `design/18` only if a UR rule's text becomes false (it should not); no KNOWN_LIMITATIONS change.

## Report
Japanese, short: diff stat; each site (what `None` leads to, the test, or why none); rows and verdicts; INFERRED; open questions.

## #117 — fix round (last) — Sonnet, implement directly

Same tree, branch and rules as `brief.md`. Reviews beside this file: `terra.md` (Terra) and `opus.md` (Opus). Adopted by the orchestrator:

1. **Site 2's `None` panics** (both, major). `rx_stop_admitted` → `None` leaves the chain `Armed`, and `chain.finish()` asserts `Off | Uncertain | Lost` (`core.rs` ~230). In the `None` arm call `chain.lose()` before `finish()`, as the ordinary stop path does (`rx.rs` ~324). **Add the test** that was skipped: make `declare_sample_clock` fail without a production hook — either a segment whose `config.ratio` has a term above `RATIO_TERM_CAP` built with the in-crate helpers (`plan` / `rx_with_plan`, `rx.rs` ~877–903, which write `Segment.config` directly), or clock-id exhaustion (`crates/ezsdr-kernel/tests/time_model.rs` ~560 shows the mechanism). Hold `rx_declare_failed_stop_before_gate`, publish a loss, release; assert no `rx_stop` after the loss, the chain ends `Lost`, and the segment is refused (no panic). It must fail on the current diff (the panic) — say that it did. Add a row for site 2 like the others.
2. **The rows must keep the loss injection** (Terra major 2). The probe runs inside `admitted`, so a mutant that replaces the whole admitted call with the bare device call also removes the probe, and the test fails because no loss was injected, not because a call followed a loss. Fix in the rows only, no production change: each such row's replacement runs the same probe first, e.g. `{ self.core.probe("<site's probe>", None); Some(self.core.device.rx_start(self.k, origin)) }` (match the real types). Applies to N117-1, N117-2, the new site-2 row and N108-159. Confirm each is still killed and that the killing assertion is the "no call after the loss" one (read the failing assertion in the row's log).
3. **Site-3 test** (Terra minor, Opus note): make it a real later stream — two RX streams, the loss published at stream 1's first-start probe after stream 0's owner has begun; assert stream 1 gets no `rx_start`, the start fails with the UR-15 refusal, and stream 0's owner is joined at cleanup with no device call after the loss. If a two-stream binding is not available on that test path, keep one stream, rename the test to what it checks, and say why.
4. **Site-1 test** (Opus minor): assert `rx_chain[0].device() == RxDevice::Lost` (the assertion that tells the outcomes apart), as the restart test does (`rx.rs` ~2501).
5. **Text that is now false** (Opus minor): `design/18-uhd-radio.md` UR-17 (~233) and UR-29 (~374) say the three admitted receive calls are the first stop, the second stop and the restart — now six; add the new tests to their (or UR-15's) `*Test:*` lines; the comment at `core.rs` ~1559; row N108-135's name. Rule format per AGENTS §2 (normative text only; `rules.py` checks it).

Checks as in `brief.md` (new tests 20 times each); rows: the touched and new ones plus N108-135/136/157/158. Report (Japanese, short): diff stat; each item; rows with their killing assertion.

## #117 — last small round (sent to the implementer as a message)

From Terra's re-check: (1, major) in the site-3 test assert "no rx_start for stream 1 / no device call after mark_lost" before requiring the refusal, so N117-2 fails on the post-loss-call assertion; (2, minor) observe that stream 0's owner ran before the loss and that cleanup joined it, without a production hook; (3, minor) keep the list of the six admitted receive calls once, in UR-29, with UR-17 citing it.

