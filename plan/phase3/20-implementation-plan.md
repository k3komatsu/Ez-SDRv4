# Phase 3 — implementation plan

| Field | Value |
|---|---|
| Status | **Accepted at Gate P** (owner, 2026-09-26; `00-overview.md` §11), together with specs 11 and 12 and the patches in `patches/`. |
| Audience | The agent that implements Phase 3. This file tells you **what to do, in what order, and how to know you are done**. Specs 11 and 12 tell you **what is true**. The patches **are** the implementation: you apply them, you do not write code. |
| Base | The code, schemas and design text of commit `612b9eb`. Later commits that touch only `plan/` or `handoff.md` do not move the base. Step 0 fails loudly if the base has moved. |
| How the patches were made | In a scratch copy of `612b9eb`: each patch applied in order to a fresh clone of this repository with `git apply`, and after the last one `cargo test --workspace` passed **592** tests on Rust 1.85.0 and on stable, `cargo +stable clippy --workspace --all-targets -- -D warnings` passed after every patch, and all **83** mutations of Appendix C were killed. After each patch the workspace was green, with the counts of "Steps at a glance". Five adversarial planning reviews then applied the patches themselves, probed them and mutated them (`reviews/planning-reviews.md`); their findings are in these patches. |
| Toolchains | Rust `1.85.0` (the MSRV) and `stable`. Both must pass at the end of every step. |
| Language | English, like the specs. Your notes may be English or Japanese. |

Documents you will read, in this order, before Step 0:

1. `AGENTS.md` (the repository's rules; §3 and §7 bind you).
2. `plan/phase3/00-overview.md` (scope, decisions Z1–Z11, governance GV-1…GV-6).
3. The spec a step names, **at the start of that step**, in full.

---

## 0. Rules for the implementer

These rules exist because Phase 1 was reviewed twenty-five times and found to hold rules "settled by review rather than written down", and because Phase 2's review of retyped code found 3 P0, 6 P1 and 16 P2 findings (`plan/phase2/00-overview.md` §11). Phase 3 removes retyping: every line of code and test is in a patch that was verified before you received it. Follow these rules literally.

### 0.1 Order

Do the steps in numerical order. Do not start step *n + 1* until every item of step *n*'s **done-list** is true.

### 0.2 What is authoritative

- A rule (`CH-n`, `KB-n`, `VB-n`, and the amended or new `RM-n`, `SE-n`, `MR-n`) is normative. The patches implement the rules. Where a patch and a spec disagree, **stop** (§0.3): do not choose.
- Never edit a file a patch created or changed, except where a step tells you to. Never change a test or an expected value. Never add `#[ignore]`. Never add a dependency (PO-4).
- Never run `cargo fmt`: the code is not rustfmt-formatted, and a formatter would rewrite unrelated lines.

### 0.3 When to stop and ask

Stop, write what you found in `plan/phase3/implementation-notes.md` (create it; one `## Step n` section per step), and report, when any of these happens:

- `git apply --check` of a patch prints anything;
- a command of §0.5 fails, or prints a test count other than the one the step names;
- a mutation of Appendix C is not killed;
- any other check a step names prints something other than the step says (a file count, the `kernel_surface` line, the link checker);
- a spec and a patch disagree, or a rule cannot be true of the patched code;
- anything in this plan asks you to do something AGENTS.md or a spec forbids.

Write: the step, the command, its exact output (the failing lines), what you expected. **Do not repair and continue.** A repaired patch is an unverified patch.

### 0.4 Git

- You never run `git commit`, `git push`, `git rebase`, `git reset`, `git stash`, `git checkout` or `git restore`. The owner commits (AGENTS.md §7). If a patch applied wrongly, stop (§0.3); do not undo it yourself.
- Never touch `v3/` (a git worktree — never `git add` it, never delete it), `design/v4-vision-audit.md`, `design/v4-vision-rereview.md`, `design/archive/`, the Vision (`Ez-SDR_v4_ARCHITECTURE_VISION.md`, `design/vision/`), or files whose name contains ` (1)` (Google Drive conflict copies).
- At the end of each step, append to `plan/phase3/implementation-notes.md` under `## Step n`: the done-list with each item marked `[x]`, the test counts the commands printed, and a suggested commit subject in Conventional Commits style.
- Stop after Step 6 and report: the owner runs Review C there (§9 of the overview). Continue with Step 7 only when the owner says so.

### 0.5 The commands

Run all three at the end of Steps 1–5, from the repository root. All must pass.

```bash
cargo +1.85.0 test --workspace
```

```bash
cargo +stable test --workspace
```

```bash
cargo +stable clippy --workspace --all-targets -- -D warnings
```

To count tests, add the numbers after `test result: ok.` in each command's output, or run:

```bash
cargo +stable test --workspace 2>&1 | grep "test result" | awk '{s+=$4; f+=$6} END {print "passed", s, "failed", f}'
```

### 0.6 Before you start: Google Drive

The working tree is synchronised by Google Drive, which has deleted tracked files before (`handoff.md` §1). Before Step 0 and before every step, `git status --short` must show no line starting with ` D`. If it does, stop.

### 0.7 Mutation checks

Step 6 runs `plan/phase3/tools/mutate.py`, which copies the repository to a scratch directory and, **there**, disables each rule of Appendix C in turn, runs the named test and expects it to fail. It never modifies your working tree. You do not write or edit a mutation.

---

## Steps at a glance

| Step | What | Spec | Workspace tests after the step (stable and 1.85.0) |
|---|---|---|---|
| 0 | Check the base | — | 550 (unchanged) |
| 1 | `patches/01-kernel.patch` | 12 KB-1, KB-2 | **553** (Kernel 446) |
| 2 | `patches/02-vocabularies.patch` | 11; 12 VB-1 (keys), VB-2 | **561** (`ezsdr-sim` 14) |
| 3 | `patches/03-mock-radio.patch` | 12 VB-1 (behaviour), VB-3…VB-8 | **585** (`ezsdr-mock-radio` 58) |
| 4 | `patches/04-acceptance.patch` | 00 §8 | **592** (`ezsdr-acceptance` 36) |
| 5 | `patches/05-design-text.patch` | 12 (text) | 592; every link resolves |
| 6 | Mutation checks (Appendix C) | GV-6 | 83 of 83 killed — **then stop: Review C** |
| 7 | Exit tables, Vision issues, `handoff.md` | 00 §10 | — **then stop: Gate X** |

Per-crate counts after Step 4: `ezsdr-kernel` 446, `ezsdr-radio` 10, `ezsdr-sim` 14, `ezsdr-sim-engine` 7, `ezsdr-hostmem` 2, `ezsdr-link-host` 2, `ezsdr-sink` 2, `ezsdr-sink-capture` 15, `ezsdr-mock-radio` 58, `ezsdr-acceptance` 36.

---

## Step 0 — Check the base

1. The code, schemas and design text must be those of `612b9eb`:

   ```bash
   git diff --quiet 612b9eb HEAD -- crates schemas design Cargo.toml Cargo.lock && echo base-ok
   ```

   ```bash
   git status --short
   ```

   The first must print `base-ok`. The second must list nothing under `crates/`, `schemas/` or `design/` (untracked files under `plan/phase3/` are expected). Otherwise stop.

2. Run the three commands of §0.5. Each `cargo test` must report **550 passed, 0 failed**.

**Done-list**

- [ ] `base-ok` printed; nothing changed under `crates/`, `schemas/`, `design/`.
- [ ] 550 passed on 1.85.0 and on stable; Clippy clean.

---

## Step 1 — The Kernel amendments

**Implements:** KB-1 (a Module reads the Run's inputs by hash) and KB-2 (a Provider settles its fidelity in `prepare`). **Read first:** spec 12 §1.

1. Apply:

   ```bash
   git apply --check plan/phase3/patches/01-kernel.patch && git apply plan/phase3/patches/01-kernel.patch
   ```

   It changes eleven files (Appendix A): three under `crates/ezsdr-kernel/src/`, six under `crates/ezsdr-kernel/tests/`, and one test file each in `ezsdr-sink-capture` and `ezsdr-mock-radio` (a `PrepareContext` literal gains `inputs`).

2. Run the three commands of §0.5: **553 passed** each.

3. Record the surface count:

   ```bash
   cargo +stable test -p ezsdr-kernel --test kernel_surface ov_23b -- --nocapture 2>&1 | grep "OV-23b"
   ```

   It must print `OV-23b: Kernel growth = 116 NEW: items of 292 public items`.

4. Read the diff once (`git diff crates/ezsdr-kernel/src`) so you know what changed: `module_api::InputStore`, `PrepareContext.inputs`, and the coordinator's store behind an `Arc<Mutex<…>>`.

**Done-list**

- [ ] The patch applied without an error; `git apply --stat plan/phase3/patches/01-kernel.patch` lists the eleven files of Appendix A's patch-01 rows.
- [ ] 553 passed on 1.85.0 and on stable; Clippy clean.
- [ ] `116 NEW: items of 292 public items`.

Suggested commit subject: `feat(kernel): let Modules read Run inputs by hash (Phase 3 KB-1, KB-2)`.

---

## Step 2 — The Vocabularies: `radio` 1.1.0 and `sim` 1.1.0 with the SimulationChannel

**Implements:** VB-1's two capability keys, VB-2, and spec 11 (CH-1…CH-11). **Read first:** spec 11 in full, then spec 12 VB-1 and VB-2.

1. Apply:

   ```bash
   git apply --check plan/phase3/patches/02-vocabularies.patch && git apply plan/phase3/patches/02-vocabularies.patch
   ```

   It changes eight files (Appendix A), including the new `crates/ezsdr-sim/src/channel.rs`, the new `crates/ezsdr-sim/tests/sim_channel.rs`, the new `schemas/sim/channel.v1.json` and one entry at the top of `schemas/SCHEMA_CHANGELOG.md`.

2. Run the three commands of §0.5: **561 passed** each. MockRadio still declares `radio ^1.0.0` and `sim ^1.0.0` at this step, which the 1.1.0 Vocabularies satisfy.

3. Confirm the schema freeze is exact (no file is regenerated):

   ```bash
   cargo +stable test -p ezsdr-sim --test sim_vocabulary se_12_schema_freeze
   ```

**Done-list**

- [ ] The patch applied without an error; `git apply --stat plan/phase3/patches/02-vocabularies.patch` lists the eight files of Appendix A's patch-02 rows.
- [ ] 561 passed on 1.85.0 and on stable; Clippy clean.
- [ ] `se_12_schema_freeze` passes without `EZSDR_UPDATE_SCHEMAS`.

Suggested commit subject: `feat(sim): add the SimulationChannel (Phase 3 spec 11, VB-1, VB-2)`.

---

## Step 3 — MockRadio 1.1.0 on the SimulationChannel

**Implements:** VB-1's behaviour (RM-14, RM-16), VB-3 (versions, profiles, path delays), VB-4 (strict publication and emission), VB-5 (the late comparison), VB-6 (the stop cut), VB-7 (MR-31…MR-36), VB-8 (no burst at or before the open burst's next sample). **Read first:** spec 12 VB-3…VB-8, then spec 09 as those amendments change it (the patched text arrives only at Step 5, so read the quoted text in spec 12).

1. Apply:

   ```bash
   git apply --check plan/phase3/patches/03-mock-radio.patch && git apply plan/phase3/patches/03-mock-radio.patch
   ```

   It changes seven files (Appendix A): four under `crates/ezsdr-mock-radio/src/` (one new, `channel.rs`), two under its `tests/` (one new, `mock_channel.rs`), and `crates/ezsdr-acceptance/src/rig.rs`, whose bindings move to MockRadio 1.1.0 and which now hands one `Medium` to every MockRadio it builds. The rig moves in this step, not the next, so that the workspace stays green: without it every acceptance test would bind MockRadio 1.0.0, which 1.1.0 refuses.

2. Run the three commands of §0.5: **585 passed** each.

3. Confirm the Phase 2 MockRadio tests the patch changed are exactly those GV-4 lists:

   ```bash
   git diff --stat crates/ezsdr-mock-radio/tests/mock_radio.rs
   ```

   and read that diff. Every change is one of: a version literal (`1, 0, 0` → `1, 1, 0`); the `PrepareContext` field `inputs` added in Step 1; `mr_03`'s profile-version and path-delay assertions; or VB-4's one root tick — a step instant that was a last sample's instant moves one tick later (`…_000` → `…_001`, `…_500` → `…_501`), `mr_14`'s "nothing yet" instant moves from `1_998_999` to `1_999_000`, and `mr_14`'s expected `next_due` values move from `1_999_000` and `3_999_000` to `1_999_001` and `3_999_001`.

**Done-list**

- [ ] The patch applied without an error; `git apply --stat plan/phase3/patches/03-mock-radio.patch` lists the seven files of Appendix A's patch-03 rows.
- [ ] 585 passed on 1.85.0 and on stable; Clippy clean.
- [ ] The `mock_radio.rs` diff contains only the kinds of change item 3 lists.

Suggested commit subject: `feat(mock-radio): run MockRadio 1.1.0 on the SimulationChannel (Phase 3 VB-3…VB-8)`.

---

## Step 4 — The acceptance tests

**Implements:** the carriers of `00-overview.md` §8. **Read first:** `00-overview.md` §8.

1. Apply:

   ```bash
   git apply --check plan/phase3/patches/04-acceptance.patch && git apply plan/phase3/patches/04-acceptance.patch
   ```

   It changes three files: `crates/ezsdr-acceptance/src/experiments.rs` (`waveform_of`, `link`), `crates/ezsdr-acceptance/src/rig.rs` (`link_profile`, `link_session_profile`) and `crates/ezsdr-acceptance/tests/v58.rs` (seven tests appended).

2. Run the three commands of §0.5: **592 passed** each.

3. Confirm `v58_10_experiments_name_no_mock_type` still passes (the new experiments name no Mock type):

   ```bash
   cargo +stable test -p ezsdr-acceptance --test v58 v58_10
   ```

**Done-list**

- [ ] The patch applied without an error; `git apply --stat plan/phase3/patches/04-acceptance.patch` lists three files.
- [ ] 592 passed on 1.85.0 and on stable; Clippy clean.
- [ ] `v58_10_experiments_name_no_mock_type` passes.

Suggested commit subject: `test(acceptance): carry Vision §58 #8 and deterministic Runs (Phase 3)`.

---

## Step 5 — The amended spec text

**Implements:** GV-5 — spec 12's text in `design/04…09`. **Read first:** spec 12 in full.

1. Apply:

   ```bash
   git apply --check plan/phase3/patches/05-design-text.patch && git apply plan/phase3/patches/05-design-text.patch
   ```

   It changes six files: `design/04-run-and-session.md`, `design/05-module-api.md`, `design/06-kernel-coordinator.md`, `design/07-radio-model.md`, `design/08-simulation.md` and `design/09-mock-radio.md`. Spec 11 stays in `plan/phase3/` until Step X.

2. Check that every quoted amendment of spec 12 is now in its spec: for each rule spec 12 amends, find the rule in the named `design/` file and confirm the quoted text is there verbatim, its tag included (spec 12's introduction says which tags a quote carries; a table row may have none). Record the list of rules you checked.

3. Check that nothing links to a missing file; the checker resolves each link from the file that contains it:

   ```bash
   python3 plan/phase3/tools/check_links.py
   ```

   It must print `ok: <n> links` and exit 0. A `BROKEN` line is a stop (§0.3).

4. Run the three commands of §0.5 (no code changed; 592 each).

**Done-list**

- [ ] The patch applied without an error; `git apply --stat plan/phase3/patches/05-design-text.patch` lists six files.
- [ ] Every amended rule of spec 12 checked in its `design/` file.
- [ ] Every link resolves.
- [ ] 592 passed on 1.85.0 and on stable; Clippy clean.

Suggested commit subject: `docs(phase3): apply the Phase 3 amendments to the accepted specs`.

---

## Step 6 — Mutation checks

**Implements:** GV-6 (PO-12's obligation for Phase 3's refusals and rules). **Read first:** Appendix C.

1. Run, from the repository root, with a scratch directory outside the working tree. The runner refuses a directory that is the repository, inside it or around it, and deletes an existing directory only if an earlier run of the runner created it. It runs each mutation's test once unmutated first, which must pass. This takes several minutes:

   ```bash
   python3 plan/phase3/tools/mutate.py plan/phase3/tools/mutations.json "$CLAUDE_JOB_DIR/tmp/phase3-mutations"
   ```

   If `$CLAUDE_JOB_DIR` is not set in your environment, use any directory outside the repository, for example `/tmp/ezsdr-phase3-mutations`.

2. It prints 83 lines `Mnn <name>: killed` and exits 0. Copy the 83 lines into your notes.

3. `git status --short` must show the same files as after Step 5: the runner works on its copy only.

**Done-list**

- [ ] 83 of 83 killed; exit status 0.
- [ ] The working tree is unchanged by the run.

Then **stop and report**: the owner runs Review C (`prompts/02-review-c.txt`).

---

## Step 7 — Exit tables, Vision issues, handoff

Do this step only when the owner says Review C is closed.

**Implements:** exit criteria 2 and 8 (`00-overview.md` §10), PO-10, GV-5.

1. Create `plan/phase3/exit-review/README.md`, `plan/phase3/exit-review/11.md` and `plan/phase3/exit-review/12.md` in the format of `plan/phase2/exit-review/README.md` and its tables, one row per rule in source order, starting from Appendix B. For **each** row, open the cited test, read its body, and confirm the assertion is the row's claim (OV-3). If a body does not assert what the row says, do not edit the row to fit: stop (§0.3).
2. Create `plan/phase3/vision-issues.md` listing the eight Vision issues of spec 11 §7 (four) and spec 12 §3 (four), each with a link to its section, in the format of `plan/phase2/vision-issues.md`. Do not edit the Vision.
3. Update `handoff.md`:
   - §3: the per-crate table and total (Step 4's counts), `116 NEW / 292 public items`, the toolchain line with today's date;
   - §4: a "Phase 3 — Gate X pending" subsection naming the documents of `00-overview.md`'s header and the next step, Gate X;
   - §1's `main` row: Phase 3 implemented, Gate X pending.
4. Run the `v3/` path check of `handoff.md` §1; it must print nothing. Run `python3 plan/phase3/tools/check_links.py`; it must print `ok: <n> links`.
5. Run the three commands of §0.5 once more.

**Done-list**

- [ ] Three exit-review files, every Phase 3 rule with a disposition read from a test body, no `GAP`, no `UNCERTAIN`.
- [ ] `plan/phase3/vision-issues.md` with eight items; the Vision untouched.
- [ ] `handoff.md` §1, §3 and §4 updated.
- [ ] `v3/` check prints nothing; links `ok`; 592 passed on both toolchains; Clippy clean.

Then **stop**: Gate X is the owner's.

---

## Step X — After Gate X (owner-directed)

1. Move `plan/phase3/11-simulation-channel.md` to `design/11-simulation-channel.md`; change its status row to "Accepted at Gate P (…) and ratified at Gate X (…)", and every link to it in `plan/` and `design/` (`git grep -n "11-simulation-channel"`).
2. Apply the eight Vision issues with owner approval, adding one row to the revision history in `Ez-SDR_v4_ARCHITECTURE_VISION.md` (AGENTS.md §2).
3. Update `AGENTS.md` §1's `main` row (specs 01–11) and `handoff.md` §2.
4. Recheck every link and the `v3/` paths.

---

## Appendix A — What each patch changes

| Patch | File | Change |
|---|---|---|
| 01 | `crates/ezsdr-kernel/src/module_api.rs` | `pub trait InputStore`; two implementations (`BTreeMap`, `Mutex<BTreeMap>`); `PrepareContext.inputs` |
| 01 | `crates/ezsdr-kernel/src/coordinator/mod.rs` | `RunHandle.store` becomes `Arc<Mutex<BTreeMap<ContentHash, Arc<[u8]>>>>` |
| 01 | `crates/ezsdr-kernel/src/coordinator/pipeline.rs` | the store built from `Assembly.inputs`, read by KC-9, extended by KC-28, handed to every `PrepareContext` |
| 01 | `crates/ezsdr-kernel/tests/kernel_surface_allow.txt` | `module_api::InputStore` |
| 01 | `crates/ezsdr-kernel/tests/support/doubles.rs` | `TestProvider::set_fidelity` |
| 01 | `crates/ezsdr-kernel/tests/support/run_doubles.rs` | `SteppedProvider` keeps `inputs`, records `burst_input:…`, `settling_fidelity` |
| 01 | `crates/ezsdr-kernel/tests/coordinator.rs` | `kb_01_*` (two), `kb_02_*` |
| 01 | `crates/ezsdr-kernel/tests/module_api.rs`, `tests/run_doubles.rs`, `crates/ezsdr-sink-capture/tests/sink_capture.rs`, `crates/ezsdr-mock-radio/tests/mock_radio.rs` | `inputs` in six `PrepareContext` literals |
| 02 | `crates/ezsdr-radio/src/lib.rs` | `radio` 1.1.0; `TX_PATH_DELAY_SAMPLES`, `RX_PATH_DELAY_SAMPLES` |
| 02 | `crates/ezsdr-radio/tests/radio_model.rs` | 1.1.0, 31 keys |
| 02 | `crates/ezsdr-sim/src/lib.rs` | `sim` 1.1.0; `pub mod channel`; the third check; the `channel` schema |
| 02 | `crates/ezsdr-sim/src/channel.rs` (new) | spec 11: `ChannelSpec`, `Coupling`, `read`, `ChannelCheck`, `named`, `RootInstant`, `Radiated`, `Transmitter`, `Medium`, `gaussian_pair` |
| 02 | `crates/ezsdr-sim/tests/sim_channel.rs` (new) | the eight `ch_*` tests |
| 02 | `crates/ezsdr-sim/tests/sim_vocabulary.rs` | `se_01_…_the_three_checks` |
| 02 | `schemas/sim/channel.v1.json` (new), `schemas/SCHEMA_CHANGELOG.md` | CH-10, PO-6 |
| 03 | `crates/ezsdr-mock-radio/src/channel.rs` (new) | `Timeline`, `Segment`, `TxPlan`, `TxShared` (the `Transmitter`), `RxModel`, `decode_waveform`, `lo_phase` |
| 03 | `crates/ezsdr-mock-radio/src/lib.rs` | 1.1.0; `with_medium`; channel mode in `prepare` and `arm`; the transmit segments; `publish_rx` from the field; strict publication; the rounded-up late comparison; the stop cut (`transmit_until_cut`, `cut_segments`); timelines in `apply_update`; `rx_clipped`, `tx_clipped` |
| 03 | `crates/ezsdr-mock-radio/src/profile.rs` | profiles 1.1.0; path delays; `random_phase_on_untimed_tune` |
| 03 | `crates/ezsdr-mock-radio/src/time.rs` | `v_after` and its unit test `mr_14_a_block_is_published_strictly_after_its_last_sample` |
| 03 | `crates/ezsdr-mock-radio/tests/mock_channel.rs` (new) | the twenty-three Phase 3 tests |
| 03 | `crates/ezsdr-mock-radio/tests/mock_radio.rs` | versions 1.1.0; `mr_03`'s new assertions; twelve tests step one root tick later (VB-4: eight for receive publication, four for transmit emission) |
| 03 | `crates/ezsdr-acceptance/src/rig.rs` | MockRadio and profile 1.1.0 in `profile_document`; one `Medium` per assembly, handed with `with_medium` |
| 04 | `crates/ezsdr-acceptance/src/experiments.rs` | `waveform_of`, `link` |
| 04 | `crates/ezsdr-acceptance/src/rig.rs` | `link_profile`, `link_session_profile`, `link_document` |
| 04 | `crates/ezsdr-acceptance/tests/v58.rs` | seven tests (`00-overview.md` §8) |
| 05 | `design/04…09` | spec 12's text |

## Appendix B — Every rule, its patch and its test

The draft of Step 7's exit tables. Disposition as OV-3: `default` — a test asserts it; `producer` — a Provider's obligation, tested on MockRadio; `process` — carried by a procedure or by text.

### Spec 11

| Rule | Patch | Disposition | Carrier |
|---|---|---|---|
| CH-1 | 02 | default | `ch_01_reader` |
| CH-2 | 02 | default | `ch_02_check`; ceiling: an output id passes (the radio refuses it at `arm`, CH-7) |
| CH-3 | 02 | default | `ch_03_root_instants_are_exact` |
| CH-4 | 02 | default | `ch_04_the_field_sums_paths_with_gain_delay_and_the_frequency_gate` (a transmitter one `f64` step off is not counted), `ch_04_a_delay_rounds_up_to_a_root_tick`; end to end `mr_33_a_receiver_hears_only_its_own_frequency` |
| CH-5 | 02 | default | `ch_05_noise_has_its_power_its_seed_and_one_stream_per_receive_channel`, `ch_05_gaussian_pair_follows_its_formula`; ceiling: a stream is indexed by evaluation, so a sample lost in a gap shifts every later draw |
| CH-6 | 02 | default | `ch_06_join_rules_and_missing_fragments` (including the delay-overflow refusal); the refusal as MockRadio meets it in `mr_31_channel_mode_needs_a_medium_a_zero_pattern_and_valid_channels` |
| CH-7 | 02, 03 | default | `ch_06_join_rules_and_missing_fragments`; the `arm` refusal in `mr_31_…` |
| CH-8 | 02, 03 | producer | MockRadio's `TxShared`: `mr_32_a_loopback_receives_what_it_transmits` |
| CH-9 | 03 | producer + default | `ch_09_the_receive_output_does_not_depend_on_the_stepping_order`, `ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order`; end to end `v58_08_a_session_hears_a_burst_from_its_first_sample_in_either_instance_order`; precondition: the loop steps at every due wakeup (KC-20) |
| CH-10 | 02 | default | `se_12_schema_freeze` |
| CH-11 | 02, 04 | default | `v58_03_channel_noise_reproduces_with_its_seed`, `v58_03_a_run_reproduces_from_its_own_manifest`; ceiling: platform math library |

### Spec 12

| Rule | Patch | Disposition | Carrier |
|---|---|---|---|
| KB-1 | 01, 05 | default | `kb_01_a_provider_reads_a_spec_input_by_hash`, `kb_01_a_provider_reads_a_session_waveform_by_hash`; end to end every MockRadio channel test that transmits |
| KB-2 | 01, 05 | default | `kb_02_the_manifest_records_the_fidelity_settled_in_prepare`; end to end `v58_08_two_mock_radios_communicate_through_the_channel` and `v58_08_without_a_channel_the_same_spec_hears_nothing` (`run.fidelity.rf`) |
| VB-1: RM-1, RM-4 | 02, 05 | default | `rm_01_register_adds_the_descriptor_the_check_and_the_kinds`, `rm_01_register_twice_is_refused`, `rm_04_the_key_table_is_exactly_the_declared_one` |
| VB-1: RM-9 | 05 | producer | `mr_34_a_timed_tune_replaces_the_random_phase_with_its_constant` |
| VB-1: RM-13 | 03, 05 | producer | `mr_32_waveform_refusals` (the non-finite refusal), `mr_32_a_two_channel_waveform_is_channel_interleaved` |
| VB-1: RM-14 | 03, 05 | producer | `mr_17_a_target_whose_instant_has_passed_is_late_even_on_the_floor_sample` |
| VB-1: RM-16 | 03, 05 | producer | `mr_25_a_stopped_burst_transmits_every_sample_before_the_stop`, `mr_15_a_transmit_block_is_emitted_only_after_its_last_sample`, `mr_25_a_stop_on_a_sample_instant_does_not_transmit_that_sample`, `mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter`, `mr_25_a_stop_transmits_every_held_burst_before_it`, `ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order`, `mr_32_a_stop_after_a_cold_change_keeps_the_old_clock_s_radiation` |
| VB-1: RM-23 | 02, 03, 05 | producer | `mr_32_the_transmit_path_delay_shifts_a_loopback`; the values in `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section` |
| VB-2: SE-1 | 02, 05 | default | `se_01_register_adds_the_descriptor_and_the_three_checks` |
| VB-2: SE-6 | 02, 05 | default | `ch_05_noise_…` (the channel's streams), `mr_31_the_medium_takes_the_run_s_seed` (MockRadio joins with the Run's seed), `mr_34_a_timed_tune_…` (the `<device>/lo` stream) |
| VB-2: SE-12 | 02, 05 | default | `se_12_schema_freeze` |
| VB-3: MR-1…MR-4 | 03, 05 | default | `mr_01_descriptor_registers`, `mr_02_from_binding_refusals`, `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`, `mr_04_instance` |
| VB-4: MR-14 | 03, 05 | default | `mr_14_blocks_appear_when_their_last_sample_has_occurred`, `mr_14_a_block_is_published_strictly_after_its_last_sample`, `ch_09_the_receive_output_…`; end to end `v58_08_a_session_hears_a_burst_…` |
| VB-4: MR-15 | 03, 05 | default | `mr_15_a_transmit_block_is_emitted_only_after_its_last_sample` |
| VB-5: MR-17 | 03, 05 | default | `mr_17_a_target_whose_instant_has_passed_is_late_even_on_the_floor_sample` |
| VB-6: MR-25 | 03, 05 | default | `mr_25_a_stopped_burst_transmits_every_sample_before_the_stop`, `mr_25_a_stop_on_a_sample_instant_does_not_transmit_that_sample`, `mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter`, `mr_25_a_stop_transmits_every_held_burst_before_it`, `ch_09_a_burst_starting_between_rounds_…`, `mr_32_a_stop_after_a_cold_change_…` |
| VB-7: MR-13, MR-15, MR-16 | 03, 05 | default | `mr_31_…` (no ramp in channel mode), `mr_32_a_loopback_…` (transmit content from the waveform), `mr_32_waveform_refusals` (the refusals' place before MR-17) |
| VB-7: MR-18 | 03, 05 | default | `mr_33_gain_scales_the_samples_from_its_instant`, `mr_34_a_timed_tune_…`, `mr_32_a_cold_transmit_change_ends_the_old_radiation`, `mr_32_a_stop_after_a_cold_change_…` |
| VB-7: MR-27 | 03, 05 | default | `mr_36_clipping_at_full_scale_is_counted` (`rx_clipped`, `tx_clipped`) |
| VB-7: MR-30 | 03, 05 | default | `mr_30_two_mocks_one_seed_identical_output`, `ch_09_…`, `v58_03_channel_noise_reproduces_with_its_seed` |
| VB-8: MR-16 | 03, 05 | default | `mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused`; ceiling: with VB-4's strict emission the refusal can fire only at the open burst's next sample, so its "before" half is unreachable |
| MR-31 | 03, 05 | default | `mr_31_channel_mode_needs_a_medium_a_zero_pattern_and_valid_channels`, `mr_31_the_medium_takes_the_run_s_seed` |
| MR-32 | 03, 05 | default | `mr_32_a_loopback_receives_what_it_transmits`, `mr_32_waveform_refusals`, `mr_32_a_two_channel_waveform_is_channel_interleaved`, `mr_32_the_transmit_path_delay_shifts_a_loopback`, `mr_32_a_cold_transmit_change_ends_the_old_radiation`, `mr_32_a_stop_after_a_cold_change_keeps_the_old_clock_s_radiation`, `mr_25_a_stopped_burst_…`, `mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter`, `mr_25_a_stop_transmits_every_held_burst_before_it`; ceiling: with VB-8 the emission-refusal cuts (the MR-15 and MR-24 branches) are not reached by MockRadio's own emission (INFERRED, spec 12 VB-8), so no test exercises them |
| MR-33 | 03, 05 | default | `mr_33_gain_scales_the_samples_from_its_instant`, `mr_33_a_receiver_hears_only_its_own_frequency` |
| MR-34 | 03, 05 | default | `mr_34_a_timed_tune_replaces_the_random_phase_with_its_constant`, `mr_34_the_deterministic_profile_keeps_every_phase_zero` |
| MR-35 | 03, 05 | default | `mr_32_a_loopback_…`, `mr_35_a_cold_receive_change_samples_the_field_on_the_new_clock`, `ch_09_…`; ceiling: the receive path delay is 0 in both profiles, so no test exercises a nonzero one |
| MR-36 | 03, 05 | default | `mr_36_clipping_at_full_scale_is_counted` |
| M11–M15 (spec 09 decisions; not the mutation ids of Appendix C) | 05 | process | spec 09 §5 as patched |

### Vision §57 and §58 (acceptance, patch 04)

`v58_08_two_mock_radios_communicate_through_the_channel`, `v58_08_without_a_channel_the_same_spec_hears_nothing`, `v58_08_a_session_hears_a_burst_from_its_first_sample_in_either_instance_order`, `v58_03_channel_noise_reproduces_with_its_seed`, `v58_03_a_run_reproduces_from_its_own_manifest`, `v58_12_the_channel_output_does_not_depend_on_block_lengths`, `v57_a_software_loopback_session_captures_what_it_transmits`.

## Appendix C — Mutations

`plan/phase3/tools/mutations.json` holds each mutation's exact text (`file`, `old`, `new`); this table is generated from it and says which rule each disables and which tests the runner expects to fail (`cargo +stable test -q` followed by the column's arguments). Every one was killed on the verified patches. M03, M20–M23 and M53–M66 are the first planning review's own mutations (its R15, R01–R03, R16, R04–R14 and R17–R19), M68–M75 the second's (its N01–N03, N14, the VB-8 refusal, N04, N17, N18), M76–M78 the third's (its R3-10, R3-08, R3-03), M79–M82 the fourth's (strict transmit emission, its N01, N11, N12) and M83 the fifth's (its N13; `reviews/planning-reviews.md`); an entry with no filter runs every test of its file. M67 is M25 aimed at the end-to-end carrier.

| Id | Rule | What the mutation does | Tests that must fail |
|---|---|---|---|
| M01 | KB-1 | empty store handed to Modules | `-p ezsdr-kernel --test coordinator kb_01` |
| M02 | KB-1 | Session waveform not stored | `-p ezsdr-kernel --test coordinator kb_01_a_provider_reads_a_session` |
| M03 | KB-1 | prepare gets a snapshot of the store | `-p ezsdr-kernel --test coordinator kb_01` |
| M04 | CH-1 | object check removed | `-p ezsdr-sim --test sim_channel ch_01` |
| M05 | CH-1 | gain range removed | `-p ezsdr-sim --test sim_channel ch_01` |
| M06 | CH-1 | delay range removed | `-p ezsdr-sim --test sim_channel ch_01` |
| M07 | CH-1 | noise range removed | `-p ezsdr-sim --test sim_channel ch_01` |
| M08 | CH-2 | names not checked | `-p ezsdr-sim --test sim_channel ch_02` |
| M09 | CH-4 | gain ignored | `-p ezsdr-sim --test sim_channel ch_04` |
| M10 | CH-4 | delay ignored | `-p ezsdr-sim --test sim_channel ch_04` |
| M11 | CH-4 | delay rounds down | `-p ezsdr-sim --test sim_channel ch_04_a` |
| M12 | CH-5 | one stream per receiver | `-p ezsdr-sim --test sim_channel ch_05_noise` |
| M13 | CH-5 | sigma without the half | `-p ezsdr-sim --test sim_channel ch_05_noise` |
| M14 | CH-6 | another Run accepted | `-p ezsdr-sim --test sim_channel ch_06` |
| M15 | CH-6 | another document accepted | `-p ezsdr-sim --test sim_channel ch_06` |
| M16 | CH-6 | double join accepted | `-p ezsdr-sim --test sim_channel ch_06` |
| M17 | CH-6 | zero root rate accepted | `-p ezsdr-sim --test sim_channel ch_06` |
| M18 | CH-7 | missing always empty | `-p ezsdr-sim --test sim_channel ch_06` |
| M19 | CH-6 | a delay beyond the root's range accepted | `-p ezsdr-sim --test sim_channel ch_06` |
| M20 | CH-4 | path ignores rx_channel | `-p ezsdr-sim --test sim_channel` (every test of the file) |
| M21 | CH-2 | named omits rx | `-p ezsdr-sim --test sim_channel` (every test of the file) |
| M22 | CH-2 | named omits noise keys | `-p ezsdr-sim --test sim_channel` (every test of the file) |
| M23 | CH-5 | noise only when a path contributes | `-p ezsdr-sim --test sim_channel` (every test of the file) |
| M24 | MR-14 | unit v_after is ceil | `-p ezsdr-mock-radio --test mock_radio mr_14` |
| M25 | MR-14 | strict publication -> inclusive | `-p ezsdr-mock-radio --test mock_channel ch_09_the_receive_output` |
| M26 | MR-25 | stop emits no partial block | `-p ezsdr-mock-radio --test mock_channel mr_25_a_stopped` |
| M27 | MR-32 | stop does not cut radiation | `-p ezsdr-mock-radio --test mock_channel mr_25_a_stopped` |
| M28 | MR-17 | floor now | `-p ezsdr-mock-radio --test mock_channel mr_17_a_target` |
| M29 | CH-4 | frequency gate removed | `-p ezsdr-mock-radio --test mock_channel mr_33_a_receiver` |
| M30 | MR-33 | timeline ignores tick | `-p ezsdr-mock-radio --test mock_channel mr_33_gain` |
| M31 | MR-34 | rx retune keeps phase | `-p ezsdr-mock-radio --test mock_channel mr_34_a_timed_tune` |
| M32 | MR-34 | tx retune keeps phase | `-p ezsdr-mock-radio --test mock_channel mr_34_a_timed_tune` |
| M33 | MR-34 | no random phase | `-p ezsdr-mock-radio --test mock_channel mr_34_a_timed_tune` |
| M34 | MR-32 | transmit path delay ignored | `-p ezsdr-mock-radio --test mock_channel mr_32_the_transmit` |
| M35 | MR-36 | tx clip removed | `-p ezsdr-mock-radio --test mock_channel mr_36` |
| M36 | MR-36 | rx clip removed | `-p ezsdr-mock-radio --test mock_channel mr_36` |
| M37 | MR-31 | no medium accepted | `-p ezsdr-mock-radio --test mock_channel mr_31` |
| M38 | MR-31 | ramp accepted | `-p ezsdr-mock-radio --test mock_channel mr_31` |
| M39 | MR-31 | channel bound | `-p ezsdr-mock-radio --test mock_channel mr_31` |
| M40 | MR-31 | missing at arm | `-p ezsdr-mock-radio --test mock_channel mr_31` |
| M41 | MR-31 | fidelity not settled | `-p ezsdr-mock-radio --test mock_channel mr_31` |
| M42 | MR-32 | missing bytes accepted | `-p ezsdr-mock-radio --test mock_channel mr_32_waveform` |
| M43 | MR-32 | size mismatch accepted | `-p ezsdr-mock-radio --test mock_channel mr_32_waveform` |
| M44 | RM-13 | non-finite accepted | `-p ezsdr-mock-radio --test mock_channel mr_32_waveform` |
| M45 | MR-32 | segment registered on a wrong origin | `-p ezsdr-mock-radio --test mock_channel mr_32_a_loopback` |
| M46 | MR-34 | timed constant equals the untimed phase | `-p ezsdr-mock-radio --test mock_channel mr_34_a_timed_tune` |
| M47 | MR-33 | rx gain ignored | `-p ezsdr-mock-radio --test mock_channel mr_33_gain` |
| M48 | MR-33 | tx gain ignored | `-p ezsdr-mock-radio --test mock_channel mr_33_gain` |
| M49 | MR-32 | a stop cuts no burst of the current clock | `-p ezsdr-mock-radio --test mock_channel mr_25_a_stopped` |
| M50 | MR-32 | a channel the burst lacks radiates | `-p ezsdr-mock-radio --test mock_channel mr_32_a_loopback` |
| M51 | MR-32 | cold change does not cut radiation | `-p ezsdr-mock-radio --test mock_channel mr_32_a_cold` |
| M52 | MR-25 | a held burst starting before the cut is withdrawn | `-p ezsdr-mock-radio --test mock_channel ch_09_a_burst_starting` |
| M53 | MR-32 | gain/phase/freq at antenna instant not sample instant | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M54 | MR-33 | timeline takes first change not last | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M55 | MR-34/35 | rx LO rotation sign flipped | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M56 | MR-31 | rx_channel bound not checked | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M57 | MR-36 | rx_clipped counts re only | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M58 | MR-36 | tx_clipped counts re only | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M59 | MR-34 | LO stream keyed by fragment id | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M60 | MR-17 | TIME_ERROR time rounded up | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M61 | MR-14 | wakeup at last sample instant | `-p ezsdr-mock-radio --test mock_radio` (every test of the file) |
| M62 | MR-32/RM-13 | multi-channel waveform read planar | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M63 | MR-34 | rx timed tune takes the tx constants | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M64 | MR-33 | tx frequency timeline not updated | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M65 | MR-25 | stop cut one tick later | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M66 | MR-35 | rx frequency timeline ignored by the gate | `-p ezsdr-mock-radio --test mock_channel` (every test of the file) |
| M67 | MR-14 | strict publication -> inclusive (end to end) | `-p ezsdr-acceptance --test v58 v58_08_a_session` |
| M68 | MR-32 | a stop ends the segments of every clock | `-p ezsdr-mock-radio --test mock_channel mr_32_a_stop_after_a_cold_change` |
| M69 | MR-32 | a stop withdraws the segments of every clock | `-p ezsdr-mock-radio --test mock_channel mr_32_a_stop_after_a_cold_change` |
| M70 | MR-32 | a cold change keeps the segment generation | `-p ezsdr-mock-radio --test mock_channel mr_32_a_stop_after_a_cold_change` |
| M71 | MR-31 | the medium joins with seed 0 | `-p ezsdr-mock-radio --test mock_channel mr_31_the_medium_takes_the_run_s_seed` |
| M72 | MR-16 | a burst at the open burst's next sample is admitted | `-p ezsdr-mock-radio --test mock_channel mr_16_a_burst_at_or_before` |
| M73 | CH-4 | the frequency gate tolerates 1 kHz | `-p ezsdr-sim --test sim_channel ch_04_the_field` |
| M74 | MR-31 | a reader error lacks the MR-7 prefix | `-p ezsdr-mock-radio --test mock_channel mr_31_channel_mode` |
| M75 | MR-31 | a join refusal lacks the MR-31 prefix | `-p ezsdr-mock-radio --test mock_channel mr_31_channel_mode` |
| M76 | MR-25 | a stop opens at most one held burst before the cut | `-p ezsdr-mock-radio --test mock_channel mr_25_a_stop_transmits_every_held` |
| M77 | CH-4 | the frequency gate tolerates half a hertz | `-p ezsdr-sim --test sim_channel ch_04_the_field` |
| M78 | MR-16 | a SendAsap move onto the open burst emits no refused TIME_ERROR | `-p ezsdr-mock-radio --test mock_channel mr_16_a_burst_at_or_before` |
| M79 | MR-15 | a transmit block is emitted at its last sample's instant | `-p ezsdr-mock-radio --test mock_channel mr_15_a_transmit_block` |
| M80 | MR-16 | the open-burst refusal is judged on the target before MR-17 moves it | `-p ezsdr-mock-radio --test mock_channel mr_16_a_burst_at_or_before` |
| M81 | MR-16 | a burst one sample after the open burst's next sample is refused | `-p ezsdr-mock-radio --test mock_channel mr_16_a_burst_at_or_before` |
| M82 | MR-16 | the open-burst refusal at the next sample only in channel mode | `-p ezsdr-mock-radio --test mock_channel mr_16_a_burst_at_or_before` |
| M83 | MR-15 | strict transmit emission only in channel mode | `-p ezsdr-mock-radio --test mock_channel mr_15_a_transmit_block` |
