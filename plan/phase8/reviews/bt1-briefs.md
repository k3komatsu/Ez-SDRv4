# bt1 — briefs (bench tools for maintenance table 3, cells C2–C6)

The implementer's brief (v2, after Sol's pre-check), then the two fix-round messages.

## Brief (v2, after Sol's pre-check `sol-pre.md`: every finding adopted) — bench tools for maintenance table 3, cells C2–C6 (step `bt1`)

You are the implementer (Claude Code Sonnet subagent). Worktree `/home/komatsu/local_work/Ez-SDRv4-e3`, branch `work/bt` (main `e7459b8b`); it builds into its own target (its `.cargo/config.toml`) — never set CARGO_TARGET_DIR to another tree's directory, and never touch the main tree `/home/komatsu/local_work/Ez-SDRv4` (the owner's bench runs from it on 11-15). You edit the worktree; you do **not** commit, stash, push or switch branches.

Risk: measurement tooling (owner 2026-10-26: no test plan). It changes no production code. Self-review waived (Sonnet implementer). A fresh Opus reviewer reads the diff after you; Sol reviews the step at its end. C7 (the Module-level repeat resume with an `m` sweep) is **not** in this step: it needs a production hook and comes later as `bt2`.

### What exists
- Cell definitions: `plan/phase8/bench.md` section "11-15 の後の手順（保守表 3 …）" (C1–C7 table, C1's procedure and VERDICT rules). Design evidence: `~/.cache/ezsdr-fixes/2026-11-14-mt3/fable-review.md` §1 (R1–R10, the FPGA state machine), §2 (the drain: one report per dropped DUC packet, gaps of packet + 2 clocks), M2, M3, m1.
- C1's tool, the template for every cell here: `crates/ezsdr-radio-uhd/tests/common/mt3.rs` (`late_chunked_trial`, `send_chunked`, `read_reports`, `read_rx`, `tone_intervals`, the pure `late_chunked_verdict` with unit tests, `cell_verdict`), the `#[ignore]` wrapper `hw_mt3_c1_chunked_late_burst` in `tests/hardware.rs:1774`, the rehearsals `mt3_c1_rehearsal_*` in `tests/fake.rs:7980`.
- The fake's drain limit (what C4 checks the device against): `src/device.rs:735-740` (`drain_limit`: the drain's end, then `pad + g`, `pad = ceil(S/100) + 2·packets + bursts·(g + DUC_WAIT samples)`).
- Slot B gating for two-channel transmits: `tests/common/bench1115.rs:1388-1430` (`SLOT_B_CONFIRMED`, `two_channel_gate`), used by `tests/hardware.rs:3990-3995`. Reuse them; do not copy.

### The cells (each: a lifted body that runs on the USRP and on `FakeDevice`, a pure verdict on a plain evidence value with hand-built unit tests, a FakeDevice rehearsal in the default suite that books the same boundary, and one `#[ignore]` hardware test, bounded like C1, transmitting at 1 GHz (`SAFE_TX_HZ`), gain 0, amplitude ≤ 0.5 through `tone_wave`, slot A unless stated)

Common rule for every late burst below: **its duration is 20 ms at every rate** (chunks of the Module's `send_cap` at that rate: 8 × 2 500 at 1 MS/s; 20 ms ÷ the chunk at higher rates), so the last chunk can be handed well before its own time even when the host only feeds faster than real time by a small factor (at 200 MS/s, 10 GbE is ≈ 1.5× real time; with a 1 ms lateness the handed time catches the chunks' own times only after ≈ 6 ms). C1 itself stays as it is.

#### C2 — C1 at 100 and 200 MS/s (and board #2 by running the same test there)
- Transmit at 100 and 200 MS/s. **Receive at 10 MS/s** (the receive and transmit rates are set separately through `Device::apply`), so the tone detector keeps C1's window and threshold; the evidence then carries the receive spacing separately from the transmit spacing. Keep the 100 kHz tone.
- Read back both actual rates (`apply`'s result) and print them; a rate that is not the requested one makes the trial INCONCLUSIVE.
- A new observation on every late trial (Sol m6): print **every** late report's tick in order, and the consecutive gaps. Do not derive the packet length from the same gaps: the packet length is the UHD streamer's samples per packet if the Device exposes it, otherwise it is labelled INFERRED (the modal gap). Flag gaps that are a multiple of the modal one as skipped reports (the radio's 32-entry error FIFO can drop reports, R7). Printed as `MT3 C2 gaps <rate>: …` (Fable §2.1: "at 200 MS/s check the gap is still packet + 2 clocks"). An observation, never a verdict term.
- Verdict: C1's (`late_chunked_verdict`), generalised only as far as the separate receive spacing needs.

#### C3 — C1 with a first send of 32 samples
- The late burst's first send is 32 samples (UR-22's minimum first send), then the Module's chunks to 20 ms. 1 and 10 MS/s.
- Verdict: C1's. Observation as C2's gaps (`MT3 C3 gaps`), plus the span from the first late report's tick to the last, labelled a **partial span** (it omits the last packet's own drain), against `S × per` (the overhead the fake's `2·packets` term pads). Never a verdict term.

#### C4 — the cascade boundary (UR-33, Fable M3): does the X310 end a drain no later than the fake's limit?
- Trial: send A (the C1 late burst shape, 1 ms late, 20 ms). Read the transmit reports until A's first TimeError; its tick is `t0`. **What `t0` is** (Sol M2, VERIFIED in `radio_tx_core.v:280-284, 337-355, 450-459`): the radio time (`radio_time_d1`) when the radio's `ST_TIME_CHECK` found A's first packet late — the start of the drain as the radio sees it, not the packet's timestamp and not a host arrival time. `δ` is a measurement coordinate relative to `t0 + S·per`; the comparison with the fake (whose drain starts when it takes the packet) carries an uncertainty of one sample period plus a few master clocks, which the tool states and applies (a dropped B within that uncertainty of the fake's limit is reported, not a FAIL). Then hand B — one timed burst of one chunk (SOB+EOB in one send) — at `B.first = t0 + S·per + δ`, aligned to the sample grid (`S` = A's samples).
- Validity (Sol M1). Record every A send's sample range and the device time read when it was handed. Each A send must be handed at least 1 ms before the drain reaches it (`t0 + its first sample offset · per`); otherwise the host may have starved the drain (`radio_tx_core.v:399-426`: a drain with no data waits, so a host stall lengthens it) and the trial is INCONCLUSIVE. B must be handed before `t0 + S·per − 2 ms`; this margin for B to be queued behind the drain is an INFERRED transport assumption — say so in the output. A trial with any Underflow, `UnderflowInPacket` or sequence-error report is INCONCLUSIVE (a partly played B must not count as dropped).
- B played ⇔ a BurstAck with tick ≥ B.first (A never has one). B dropped ⇔ no such ACK and a TimeError at or after B.first. Anything else (no TimeError for A, a call error, a report without a tick) is INCONCLUSIVE. No receive is needed for C4.
- Sweep (Sol M2: resolve the boundary, not only bracket it): a fixed grid, no adaptive search — coarse δ ∈ {−20, −10, 50, 100} µs, fine δ from 0 to 30 µs in 1 µs steps (each aligned to the sample grid), plus the fake's own limit for this A (`drain_limit` − drain end, computed from the same formula on the test side — do not change production code) and that limit + 20 µs; 10 trials per δ; rates 1 and 10 MS/s. (AGENTS §9 "Hardware tests and bench values": ≥ 10 trials per cell near a boundary.) Print one line per trial and a cell line per δ with played/dropped/inconclusive counts, and a summary line `MT3 C4 boundary <rate>: last all-dropped δ, first all-played δ, the mixed cells between` (the bracket and its mixed cells, not one number).
- Verdict of the sweep: **FAIL** if any trial with δ ≥ the fake's limit plus the stated uncertainty was dropped (the device ends a drain later than the fake: UR-33 broken, the fake is looser); **INCONCLUSIVE** if a δ ≥ the fake's limit has fewer than 10 decided trials; **PASS** otherwise. The δ below the limit are measurements (the device's own boundary), not verdict terms.
- Rehearsal: on the FakeDevice the same body must give dropped below the fake's limit and played at and after it — this pins that the tool computes the same boundary the fake uses.

#### C5 — two channels, a late start (Fable m1: can one channel play alone?)
- Transmit on slot A and slot B (channels 0 and 1) as one two-channel streamer, receive on both. Runs only through `two_channel_gate` (an OBX or UBX in both slots, `EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1`); otherwise the cell is reported "not runnable" and transmits nothing.
- Two cells: clearly late (1 ms, both channels must drop: C1's verdict per channel), and marginal — the first sample timed at the host's reference instant plus δ ∈ {−200, −100, −50, −20, 0, 20, 50, 100, 200} µs (a wide fixed grid: the host's control-call latency decides where the transition lies, so the grid must straddle it), 10 trials each, each trial classified per channel as played (ACK or heard) / dropped / inconclusive.
- Reading (Sol M3: a split is the thing sought, not a tool failure). Separate validity from the finding:
  - **SPLIT observed** (one channel played, the other dropped, in any trial): positive evidence for the contract review (RM-11's "nothing was transmitted" would need "on at least one channel"). The test fails so the operator brings it to the orchestrator; the line says it is a device fact, not a tool defect.
  - **No split, coverage valid**: the clear cell passes as C1 per channel, and the marginal grid contains at least one δ where both channels played in every decided trial and one where both dropped in every decided trial (the transition was straddled). PASS, worded "no split in N marginal trials at these conditions".
  - **INCONCLUSIVE**: the transition was not straddled, the clear cell is not PASS, or too few decided trials (< 10 per δ in the cells next to the transition).

#### C6 — the held tail starves (Fable M2: Underflow, then TimeError, no ACK)
- Trial: a chunked burst of `S` samples at 1 and 10 MS/s. The head is its first `S − 1` samples in the Module's chunks, on time (SOB on the first). The close is the Module's closing send (Sol M4, `src/provider/tx.rs:865-886`, `tests/common/raw.rs:183-185`): **two samples, the held tail and a zero, timed at `first + (S−1)·per`, SOB false, EOB true**, handed only once the device time is past that instant + 1 ms (so the device ran dry first). Host load is replaced by this deterministic hold: the device's behaviour does not depend on why the close is late.
- Verdict per trial: **FAIL** if a BurstAck at or after the burst's first sample, or if, in report order on the channel, a TimeError comes before the first Underflow (or `UnderflowInPacket`), or a TimeError's tick is before the first Underflow's tick. **INCONCLUSIVE** (Sol M5) if either an Underflow or a TimeError is missing (both are required, on the same channel, with ticks), the close was not handed ≥ 1 ms after its instant, the head starved early (the first Underflow's tick is earlier than the head's last sample instant minus one chunk: a host starvation, not the held tail), a call failed, or a report has no tick. **PASS** with an Underflow at the head's end, at least one TimeError after it in order and tick, and no ACK. 10 trials per rate. (R3/R4, `radio_tx_core.v:359-370, 399-410`: R4 may first discard the rest of an interrupted packet; that is consistent with PASS.)

### Bench procedure
Update the bench.md section: for each of C2–C6 the command per board (as C1's block: `EZSDR_UHD_ARGS`, `BOARD`, `EZSDR_BENCH_NIC`, `hw <test>`), the expected duration (estimate), what to `grep`, the VERDICT rules in one line each, and **the owner's attenuator confirmation before every transmitting command** (C5: slot B too). Keep the section's Japanese style. Mark C7 as "bt2, not yet".

### Exit checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (the whole workspace, not one crate: J154 in table 3 was missed that way), `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e7459b8b` (foreground).
- The new rehearsals 20 serial + 3 × 8 parallel via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>` run from the worktree, 0 failures.
- Mutation rows for each verdict's decisive branches (FAIL, the INCONCLUSIVE terms that make silence mean something, PASS's conjunction), Rust token anchors, smallest unique text; run the new rows through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:7,sim03:7 <ids>` from the worktree: all `killed`, none needing `"kill": "bound"`. Check first that nobody else is using sim02/sim03 (`~/.cache/ezsdr-fixes/tools/simq` holds the lock; run the rows through it if it exists).
- `cargo test -p ezsdr-radio-uhd --features uhd --no-run` compiles the hardware tests (if libuhd is present locally; otherwise via rdev).
- The AGENTS §9 audit table (test, expectation, still right / fixed / cannot tell) for every rehearsal of a hardware test that shares code with what you changed in `mt3.rs`.

### Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only, except `plan/phase8/bench.md`'s section above and `plan/maintenance/tools/mutations.json`.
- No production code changes. Do not add tests that judge by wall-clock time (the hardware bounds are infra markers, as C1's). Do not add mechanisms the brief does not name. Reuse C1's helpers; if one must generalise (a separate receive spacing, a first send length, a channel list), change it in place with C1's tests still green, not a copy.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: as `~/.cache/ezsdr-fixes/tools/deepseek-offload.txt` says (foreground, one at a time, read its diff and run the tests before you accept it; list what you offloaded).

### Report (short)
`git diff --stat`, each check's counts, the rows' outcomes, the audit table, the bench commands added, any deviation from this brief and why, anything INFERRED (in particular: whether C4's `t0` dates the arrival; the 10 MS/s receive for C2; C5's marginal offsets against the host's jitter).

## Fix round 1 (from the Opus sub-step review)
M-a: C4's grid gains limit + uncertainty. M-b: C5 classifies each channel from its reports first; a heard tone with a late TimeError and no ACK is inconclusive (leak suspected). Minors: C5's `hw` passes `EZSDR_SLOT_B_ATTENUATOR_CONFIRMED`; C5's trace records the first late send's handed time and PASS counts the transition cells; C6 filters TimeErrors by tick; C6's last head send handed ≥ 1 ms early; C4's trace prints each A send; dedupe (`rate_errors`, tone helpers, test helpers, `US`); rows (BT1-10 by assertion, C6's ACK filter, C4's `.skip(1)`, the M-a/M-b branches).

## Fix round 2 (from Sol's code review and the Opus re-check)
C6: a sequence error, tickless included, makes the trial inconclusive. C5: a split hidden by a leak in any cell makes the reading at least inconclusive. C4: each dropped trial is judged by its effective δ. C5's PASS headline counts decided trials. C6's Underflow filtered by tick. BT1-53 killed by its assertion, not a panic. C1's reduced time reads accepted.
