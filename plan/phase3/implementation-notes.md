# Phase 3 — implementation notes

The implementer's record of applying `patches/01…05` and of what each step's checks
printed. English, like the specs. One `## Step n` section per step
(`20-implementation-plan.md` §0.4). Review C and Step 7 are the owner's.

Scope of the session that started this file: Steps 0–6. Nothing here is a repair;
every patch was applied with `git apply` exactly as the plan's command gives, and
`git commit`, `push`, `rebase`, `reset`, `stash`, `checkout` and `restore` were
never run.

## Step 0

Base check, on `main` at `6ae6b8f` (`docs(phase3): draft and accept Phase 3 plan`).

**Done-list**

- [x] `base-ok` printed; nothing changed under `crates/`, `schemas/`, `design/`.
- [x] 550 passed on 1.85.0 and on stable; Clippy clean.

**Item 1 — the base is `612b9eb`'s code**

```bash
git diff --quiet 612b9eb HEAD -- crates schemas design Cargo.toml Cargo.lock && echo base-ok
```

printed `base-ok`. `git status --short` printed nothing at all, so in particular
nothing under `crates/`, `schemas/` or `design/`. `plan/phase3/` is committed
(`6ae6b8f`), so there were no untracked files under it either.

**Item 2 — §0.5's three commands**

| Command | Result |
|---|---|
| `cargo +1.85.0 test --workspace` | 550 passed, 0 failed, 0 ignored |
| `cargo +stable test --workspace` | 550 passed, 0 failed, 0 ignored |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean, no warning or error line |

Counts are the sum of the `test result: ok.` lines' `N passed` fields, as §0.5
prescribes. Both toolchains are the ones the plan names: `1.85.0` (MSRV) and
`stable`.

**Suggested commit subject**

`chore(phase3): record the Phase 3 base check`

Nothing under `crates/`, `schemas/` or `design/` changed in this step, so the
subject names the record only.

## Step 1

`patches/01-kernel.patch` — KB-1 and KB-2. Spec 12 §1 was read in full first.

**Done-list**

- [x] The patch applied without an error; `git apply --stat plan/phase3/patches/01-kernel.patch` lists the eleven files of Appendix A's patch-01 rows.
- [x] 553 passed on 1.85.0 and on stable; Clippy clean.
- [x] `116 NEW: items of 292 public items`.

**Item 1 — applied**

```bash
git apply --check plan/phase3/patches/01-kernel.patch && git apply plan/phase3/patches/01-kernel.patch
```

`git apply --check` printed nothing, so the check passed (§0.3's first stop
condition was not met). `git apply --stat` then listed exactly eleven files, the
patch-01 rows of Appendix A:

| File | Δ |
|---|---|
| `crates/ezsdr-kernel/src/coordinator/mod.rs` | 2 |
| `crates/ezsdr-kernel/src/coordinator/pipeline.rs` | 11 |
| `crates/ezsdr-kernel/src/module_api.rs` | 27 |
| `crates/ezsdr-kernel/tests/coordinator.rs` | 81 |
| `crates/ezsdr-kernel/tests/kernel_surface_allow.txt` | 1 |
| `crates/ezsdr-kernel/tests/module_api.rs` | 1 |
| `crates/ezsdr-kernel/tests/run_doubles.rs` | 1 |
| `crates/ezsdr-kernel/tests/support/doubles.rs` | 6 |
| `crates/ezsdr-kernel/tests/support/run_doubles.rs` | 24 |
| `crates/ezsdr-mock-radio/tests/mock_radio.rs` | 3 |
| `crates/ezsdr-sink-capture/tests/sink_capture.rs` | 1 |

**Item 2 — §0.5's three commands**

| Command | Result |
|---|---|
| `cargo +1.85.0 test --workspace` | 553 passed, 0 failed, 0 ignored |
| `cargo +stable test --workspace` | 553 passed, 0 failed, 0 ignored |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean, no warning or error line |

`cargo +stable test -p ezsdr-kernel` alone reports **446** passed, the
"Steps at a glance" figure for this step.

**Item 3 — the surface count**

```bash
cargo +stable test -p ezsdr-kernel --test kernel_surface ov_23b -- --nocapture 2>&1 | grep "OV-23b"
```

```text
OV-23b: Kernel growth = 116 NEW: items of 292 public items
```

The line the step names, verbatim.

**Item 4 — what changed under `src/`**

`git diff crates/ezsdr-kernel/src` was read once. It is what KB-1's **Code**
paragraph of spec 12 §1 describes, item for item:

- `module_api.rs`: `pub trait InputStore: Send + Sync { fn get(&self, hash: &ContentHash) -> Option<Arc<[u8]>>; }`,
  implemented for `BTreeMap<ContentHash, Arc<[u8]>>` and for
  `std::sync::Mutex<BTreeMap<ContentHash, Arc<[u8]>>>`; `PrepareContext` gains
  `pub inputs: Arc<dyn InputStore>`. The doc comments cite `RS-44a` and
  `MA-5a`, so `kernel_surface`'s rule prefixes are unchanged (GV-2).
- `coordinator/mod.rs`: `RunHandle.store` becomes
  `Arc<std::sync::Mutex<BTreeMap<ContentHash, Arc<[u8]>>>>`.
- `coordinator/pipeline.rs`: `assemble` builds the store from
  `Assembly.inputs` (whose public type is unchanged), KC-9 reads it through
  `InputStore::get`, KC-28 adds to it under the lock, and every
  `PrepareContext` receives `inputs: self.store.clone()`.

**GV-4 check — the Phase 2 tests this patch changes**

`git diff` of the four test files Appendix A names shows exactly the six
`PrepareContext` literals and nothing else:

```text
+            inputs: Arc::new(BTreeMap::<ezsdr_kernel::hash::ContentHash, Arc<[u8]>>::new()),
+        inputs: Arc::new(BTreeMap::<ezsdr_kernel::hash::ContentHash, Arc<[u8]>>::new()),
+            inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
+        inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
+        inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
+            inputs: Arc::new(BTreeMap::<ezsdr_kernel::hash::ContentHash, Arc<[u8]>>::new()),
```

Six sites in four files, as GV-4 and Appendix A state. The three new tests of
KB-1 and KB-2 pass:

```text
test kb_01_a_provider_reads_a_spec_input_by_hash ... ok
test kb_01_a_provider_reads_a_session_waveform_by_hash ... ok
test kb_02_the_manifest_records_the_fidelity_settled_in_prepare ... ok
```

**Suggested commit subject**

`feat(kernel): let Modules read Run inputs by hash (Phase 3 KB-1, KB-2)`

The subject the step proposes.

## Step 2

`patches/02-vocabularies.patch` — `radio` 1.1.0, `sim` 1.1.0 and spec 11
(CH-1…CH-11), with spec 12's VB-1 keys and VB-2. Spec 11 was read in full
first, then spec 12 §2's VB-1 and VB-2.

**Done-list**

- [x] The patch applied without an error; `git apply --stat plan/phase3/patches/02-vocabularies.patch` lists the eight files of Appendix A's patch-02 rows.
- [x] 561 passed on 1.85.0 and on stable; Clippy clean.
- [x] `se_12_schema_freeze` passes without `EZSDR_UPDATE_SCHEMAS`.

**Item 1 — applied**

```bash
git apply --check plan/phase3/patches/02-vocabularies.patch && git apply plan/phase3/patches/02-vocabularies.patch
```

`git apply --check` printed nothing. `git apply --stat` listed exactly eight
files, the patch-02 rows of Appendix A, including the three new ones the step
names — `crates/ezsdr-sim/src/channel.rs`, `crates/ezsdr-sim/tests/sim_channel.rs`
and `schemas/sim/channel.v1.json` — and the seven lines at the top of
`schemas/SCHEMA_CHANGELOG.md`:

| File | Δ |
|---|---|
| `crates/ezsdr-radio/src/lib.rs` | 12 |
| `crates/ezsdr-radio/tests/radio_model.rs` | 6 |
| `crates/ezsdr-sim/src/channel.rs` (new) | 362 |
| `crates/ezsdr-sim/src/lib.rs` | 24 |
| `crates/ezsdr-sim/tests/sim_channel.rs` (new) | 294 |
| `crates/ezsdr-sim/tests/sim_vocabulary.rs` | 13 |
| `schemas/SCHEMA_CHANGELOG.md` | 7 |
| `schemas/sim/channel.v1.json` (new) | 80 |

**Item 2 — §0.5's three commands**

| Command | Result |
|---|---|
| `cargo +1.85.0 test --workspace` | 561 passed, 0 failed, 0 ignored |
| `cargo +stable test --workspace` | 561 passed, 0 failed, 0 ignored |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean, no warning or error line |

`cargo +stable test -p ezsdr-sim` alone reports **14** passed, the
"Steps at a glance" figure for this step. As the step notes, MockRadio still
declares `radio ^1.0.0` and `sim ^1.0.0` here and the 1.1.0 Vocabularies satisfy
those requirements.

**Item 3 — the schema freeze is exact**

```bash
cargo +stable test -p ezsdr-sim --test sim_vocabulary se_12_schema_freeze
```

```text
running 1 test
test se_12_schema_freeze ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out
```

`EZSDR_UPDATE_SCHEMAS` was not set, so the committed
`schemas/sim/channel.v1.json` matches the generated one byte for byte
(PO-7, CH-10). `git diff --stat schemas/` afterwards shows only
`SCHEMA_CHANGELOG.md`, so the freeze test rewrote nothing.

**Spec 11 §6's eight tests**

```text
test ch_01_reader ... ok
test ch_02_check ... ok
test ch_03_root_instants_are_exact ... ok
test ch_04_the_field_sums_paths_with_gain_delay_and_the_frequency_gate ... ok
test ch_04_a_delay_rounds_up_to_a_root_tick ... ok
test ch_05_noise_has_its_power_its_seed_and_one_stream_per_receive_channel ... ok
test ch_05_gaussian_pair_follows_its_formula ... ok
test ch_06_join_rules_and_missing_fragments ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured
```

**GV-4 check — the Phase 2 tests this patch changes**

Exactly the kinds GV-4 names for patch 02, and nothing else: `rm_01_*` and
`rm_04_*` for the 31 keys (`1, 0, 0` → `1, 1, 0`, `29` → `31` keys, the two
`radio.{tx,rx}.path_delay_samples` rows appended) and `se_01_*` renamed to
`se_01_register_adds_the_descriptor_and_the_three_checks` for the three checks
(`sim 1.1.0`, `["sim.seed", "sim.faults", "sim.channel"]`, three violations).
No expected value was changed by me; each is what VB-1 and VB-2's **Tests**
paragraphs require.

**Suggested commit subject**

`feat(sim): add the SimulationChannel (Phase 3 spec 11, VB-1, VB-2)`

The subject the step proposes.

## Step 3

`patches/03-mock-radio.patch` — `ezsdr.radio.mock` 1.1.0 and its profiles 1.1.0
on the SimulationChannel: VB-3, VB-4, VB-5, VB-6, VB-7, VB-8. Spec 12 §2's
VB-3…VB-8 was read in full first, then `design/09-mock-radio.md` as those
amendments change it (its patched text arrives only at Step 5, so the new text
was read as spec 12 quotes it).

**Done-list**

- [x] The patch applied without an error; `git apply --stat plan/phase3/patches/03-mock-radio.patch` lists the seven files of Appendix A's patch-03 rows.
- [x] 585 passed on 1.85.0 and on stable; Clippy clean.
- [x] The `mock_radio.rs` diff contains only the kinds of change item 3 lists.

**Item 1 — applied**

```bash
git apply --check plan/phase3/patches/03-mock-radio.patch && git apply plan/phase3/patches/03-mock-radio.patch
```

`git apply --check` printed nothing. `git apply --stat` listed exactly seven
files, the patch-03 rows of Appendix A — four under
`crates/ezsdr-mock-radio/src/` (one new, `channel.rs`), two under its `tests/`
(one new, `mock_channel.rs`) and `crates/ezsdr-acceptance/src/rig.rs`:

| File | Δ |
|---|---|
| `crates/ezsdr-acceptance/src/rig.rs` | 8 |
| `crates/ezsdr-mock-radio/src/channel.rs` (new) | 141 |
| `crates/ezsdr-mock-radio/src/lib.rs` | 256 |
| `crates/ezsdr-mock-radio/src/profile.rs` | 24 |
| `crates/ezsdr-mock-radio/src/time.rs` | 16 |
| `crates/ezsdr-mock-radio/tests/mock_channel.rs` (new) | 1 040 |
| `crates/ezsdr-mock-radio/tests/mock_radio.rs` | 55 |

**Item 2 — §0.5's three commands**

| Command | Result |
|---|---|
| `cargo +1.85.0 test --workspace` | 585 passed, 0 failed, 0 ignored |
| `cargo +stable test --workspace` | 585 passed, 0 failed, 0 ignored |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean, no warning or error line |

`cargo +stable test -p ezsdr-mock-radio` alone reports **58** passed, the
"Steps at a glance" figure for this step. The workspace stayed green without
any acceptance test changing, which is why the rig moves in this step.

**Item 3 — the `mock_radio.rs` diff is only what GV-4 allows**

`git diff --stat crates/ezsdr-mock-radio/tests/mock_radio.rs` reports
`58 +++++++++++++++---------`, 32 insertions and 26 deletions. The whole diff was
read. Every hunk is one of the four kinds the step names, and nothing else:

1. **Version literals — exactly three**, as VB-3's **Code** paragraph says
   ("`tests/mock_radio.rs` (three literals)"): `module_ref()`'s
   `Version::new(1, 0, 0)` → `(1, 1, 0)`, `binding()`'s `ProfileRef` version
   `(1, 0, 0)` → `(1, 1, 0)`, and `mr_01_descriptor_registers`'s
   `assert_eq!(d.version, …)`.
2. **The `PrepareContext` field `inputs` added in Step 1** — three literals in
   this file (`Harness`'s and `mr_07_prepare_cases`'s two), which are patch
   01's six sites across four files. `mr_07_prepare_cases` changes by that field
   only.
3. **`mr_03`'s profile-version and path-delay assertions** — the profile
   `version` `{1,0,0}` → `{1,1,0}` and the two new capability assertions with
   `(45, 0)` for `x310-like` and `(0, 0)` for `ideal`, which are MR-3's two new
   rows as VB-3's **Tests** paragraph states.
4. **VB-4's one root tick** — every step instant that was a last sample's
   instant moves exactly one tick later, and nothing else moves:

   | test | before | after |
   |---|---|---|
   | `mr_13_ramp_values` | `1_999_000` (×2) | `1_999_001` |
   | `mr_14_…` "nothing yet" | `step(1_998_999)` | `step(1_999_000)` |
   | `mr_14_…` `next_due` | `1_999_000`, `3_999_000` | `1_999_001`, `3_999_001` |
   | `mr_16_burst_refusals` | `1_999_000` (×2) | `1_999_001` |
   | `mr_16_repeat_is_contiguous_across_wraps` | `4_999_000` | `4_999_001` |
   | `mr_17_late_policy_outcomes` | `2_012_999_000` | `2_012_999_001` |
   | `mr_18_a_cold_rate_change_starts_a_new_sample_clock` | `+ 999_500` | `+ 999_501` |
   | `mr_18_a_cold_receive_change_before_t0_applies_at_t0` | `t0 + 999_500` | `t0 + 999_501` |
   | `mr_18_a_cold_transmit_change_replaces_the_tracker` | `3_499_500` | `3_499_501` |
   | `mr_19_backpressure_is_an_overrun` | `2_001_999_000`, `2_003_999_000`, `2_053_999_000` | `…_001` each |
   | `mr_21_overrun_shape` | `2_052_999_000`, `2_062_999_000` | `…_001` each |
   | `mr_22_sequence_error_shape` | `4_999_000` | `4_999_001` |
   | `mr_25_orderly_stop_delivers_the_tail_abort_does_not` | `stop + 999_000`, `2_001_999_000` | `…_001` each |

   Every one is `…_000` → `…_001` or `…_500` → `…_501`, plus `mr_14`'s
   `1_998_999` → `1_999_000` and its two `next_due` values — the three moves
   item 3 spells out. The `orderly.step(2_001_999_000)` and
   `stop = 2_001_999_000` of `mr_25_orderly_…` are untouched, which is
   consistent: they are not a last sample's instant that the strict rule moves.

The set of tests the diff touches — `mr_01`, `mr_03`, `mr_07` (the `inputs`
field only), `mr_13`, `mr_14`, `mr_16_burst_refusals`,
`mr_16_repeat_is_contiguous_across_wraps`, `mr_17_late_policy_outcomes`,
`mr_18_a_cold_rate_change_starts_a_new_sample_clock`,
`mr_18_a_cold_receive_change_before_t0_applies_at_t0`,
`mr_18_a_cold_transmit_change_replaces_the_tracker`, `mr_19`, `mr_21`, `mr_22`
and `mr_25_orderly_…` — is GV-4's list for patch 03, with the `mr_07` change
belonging to patch 01.

**The rig move, as the step describes**

`git diff crates/ezsdr-acceptance/src/rig.rs` shows exactly what item 1 says:
`profile_document` names `ezsdr.radio.mock` `1.1.0` and the profile `1.1.0`, and
`assemble` creates one `ezsdr_sim::channel::Medium::new()` per Run and hands
each MockRadio a clone through `with_medium`.

**The new test file's twenty-three tests**

`cargo +stable test -p ezsdr-mock-radio --test mock_channel` reports
`23 passed; 0 failed`. They are `ch_09_the_receive_output_does_not_depend_on_the_stepping_order`
and `ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order`
(CH-9), `mr_15_a_transmit_block_is_emitted_only_after_its_last_sample` (VB-4),
`mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused` (VB-8),
`mr_17_a_target_whose_instant_has_passed_is_late_even_on_the_floor_sample`
(VB-5), the four `mr_25_a_…` (VB-6), the two `mr_31_…` (MR-31), the six
`mr_32_…` (MR-32), the two `mr_33_…` (MR-33), the two `mr_34_…` (MR-34),
`mr_35_a_cold_receive_change_samples_the_field_on_the_new_clock` (MR-35) and
`mr_36_clipping_at_full_scale_is_counted` (MR-36).

**Suggested commit subject**

`feat(mock-radio): run MockRadio 1.1.0 on the SimulationChannel (Phase 3 VB-3…VB-8)`

The subject the step proposes.

## Step 4

`patches/04-acceptance.patch` — the carriers of `00-overview.md` §8. That
section was read first (Vision §58 rows #3, #8, #12, #14 and the Vision §57
row).

**Done-list**

- [x] The patch applied without an error; `git apply --stat plan/phase3/patches/04-acceptance.patch` lists three files.
- [x] 592 passed on 1.85.0 and on stable; Clippy clean.
- [x] `v58_10_experiments_name_no_mock_type` passes.

**Item 1 — applied**

```bash
git apply --check plan/phase3/patches/04-acceptance.patch && git apply plan/phase3/patches/04-acceptance.patch
```

`git apply --check` printed nothing. `git apply --stat` listed three files, as
the step says:

| File | Δ |
|---|---|
| `crates/ezsdr-acceptance/src/experiments.rs` | 56 |
| `crates/ezsdr-acceptance/src/rig.rs` | 50 |
| `crates/ezsdr-acceptance/tests/v58.rs` | 163 |

`experiments.rs` gains `waveform_of` and `link`; `rig.rs` gains
`link_profile`, `link_session_profile` and `link_document`; `v58.rs` gains the
seven tests.

**Item 2 — §0.5's three commands**

| Command | Result |
|---|---|
| `cargo +1.85.0 test --workspace` | 592 passed, 0 failed, 0 ignored |
| `cargo +stable test --workspace` | 592 passed, 0 failed, 0 ignored |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean, no warning or error line |

`cargo +stable test -p ezsdr-acceptance` alone reports **36** passed, the
"Steps at a glance" figure for this step.

**Item 3 — `v58_10_experiments_name_no_mock_type`**

```bash
cargo +stable test -p ezsdr-acceptance --test v58 v58_10
```

```text
running 1 test
test v58_10_experiments_name_no_mock_type ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 27 filtered out
```

The new experiments name no Mock type, as §8's #8 row requires.

**The seven new carriers of §8**

All seven are in `v58.rs` and all pass. Every Phase 2 carrier still passes
alongside them (`28 passed` in the file, of which seven are new):

| §8 row | test |
|---|---|
| §58 #8 | `v58_08_two_mock_radios_communicate_through_the_channel` |
| §58 #8 | `v58_08_a_session_hears_a_burst_from_its_first_sample_in_either_instance_order` |
| §58 #14 | `v58_08_without_a_channel_the_same_spec_hears_nothing` |
| §58 #3 | `v58_03_channel_noise_reproduces_with_its_seed` |
| §58 #3 | `v58_03_a_run_reproduces_from_its_own_manifest` |
| §58 #12 | `v58_12_the_channel_output_does_not_depend_on_block_lengths` |
| §57 | `v57_a_software_loopback_session_captures_what_it_transmits` |

**Suggested commit subject**

`test(acceptance): carry Vision §58 #8 and deterministic Runs (Phase 3)`

The subject the step proposes.

## Step 5

`patches/05-design-text.patch` — GV-5: spec 12's text into `design/04…09`.
Spec 12 was read in full first (§1, §2's VB-1…VB-8, §3).

**Done-list**

- [x] The patch applied without an error; `git apply --stat plan/phase3/patches/05-design-text.patch` lists six files.
- [x] Every amended rule of spec 12 checked in its `design/` file.
- [x] Every link resolves.
- [x] 592 passed on 1.85.0 and on stable; Clippy clean.

**Item 1 — applied**

```bash
git apply --check plan/phase3/patches/05-design-text.patch && git apply plan/phase3/patches/05-design-text.patch
```

`git apply --check` printed nothing. `git apply --stat` listed six files, the six
the step names — `design/04-run-and-session.md` (4), `design/05-module-api.md`
(12), `design/06-kernel-coordinator.md` (4), `design/07-radio-model.md` (28),
`design/08-simulation.md` (20) and `design/09-mock-radio.md` (106). Spec 11
stays in `plan/phase3/` until Step X, as the step says.

**Item 2 — every quoted amendment is in its spec, verbatim and tagged**

Each rule spec 12 amends was looked up in the `design/` file it names and its
quoted passage compared character for character, tag included. Every one is
present. The list of rules checked, and where:

*KB-1 → spec 04*

| Rule | quoted text present |
|---|---|
| RS-44a | "The stored bytes are what a Module reads through `PrepareContext.inputs` (spec 05 MA-5a) (Phase 3, KB-1)." |

*KB-1, KB-2 → spec 05*

| Rule | quoted text present |
|---|---|
| MA-5a (the `PrepareContext` row) | `inputs: shared read-only InputStore,` |
| MA-5a (the `InputStore` row) | `InputStore        get(ContentHash) -> shared bytes or none: the Run's inputs, read-only (RS-44a, KB-1)` |
| MA-5a (the sentence) | "`inputs` is the Run's input store: the bytes of every input KC-9 verified and of every waveform KC-28 ingested, keyed by content hash and read-only. …" |
| MA-6 | "`InputStore` (Phase 3, KB-1)" in the handle list |
| MA-46 | "the `InputStore` a request and response (Phase 3, KB-1), and the `environment` a document sent once at `prepare`." |
| MA-10 | "`fidelity` is final when `prepare` returns, because what a simulated Provider models can depend on the environment it reads there — a SimulationChannel makes MockRadio's `rf` aspect `impairment_model` (MR-31); …" |
| MA-10's marker | "*Checked in Phase 3 by `kb_02_the_manifest_records_the_fidelity_settled_in_prepare` (KB-2).*" |

*KB-1 → spec 06*

| Rule | quoted text present |
|---|---|
| KC-11 | "the Run's input store, shared with KC-9 and KC-28 (KB-1)" |

*VB-1 → spec 07*

| Rule | quoted text present |
|---|---|
| RM-1 | `VocabularyDescriptor { id: radio, version: 1.1.0,` and the appended "Version 1.1.0 adds the two path-delay capabilities of RM-4 and RM-23 (Phase 3, VB-1)." |
| RM-4 | both new rows, `radio.tx.path_delay_samples` and `radio.rx.path_delay_samples`; the replaced last sentence "No other `radio.` key exists in 1.1.0; version 1.0.0 declared the first twenty-nine rows, …" |
| RM-9 | the replaced marker "*Producer obligation; MockRadio emulates both behaviours on the SimulationChannel (MR-34) (Phase 3, VB-1).*" |
| RM-13 | "A Provider that reads the waveform's bytes refuses one with a non-finite component the same way (Phase 3, VB-1)." |
| RM-14 | "with `now` its current instant in the transmit SampleClock rounded up to a sample — so that a target whose instant has passed is late even when it is the sample the current instant falls in (Phase 3, VB-1) — …" |
| RM-16 | the whole replaced transmit clause, through "…every pending timed command is cancelled; Phase 3, VB-1) — and the receive side second" |
| RM-23 (new) | the whole rule, through "…a per-Run override by a CalibrationArtifact is later work (Phase 3, VB-1)." |
| Tests table | `rm_01_register_adds_the_descriptor_the_check_and_the_kinds` expects "the Vocabulary is `radio 1.1.0`"; `rm_04_the_key_table_is_exactly_the_declared_one` expects "exactly RM-4's thirty-one rows" |
| §8 Deferred | "A per-Run delay override by a CalibrationArtifact (Vision §26)" |

*VB-2 → spec 08*

| Rule | quoted text present |
|---|---|
| SE-1 | the whole `VocabularyDescriptor { id: sim, version: 1.1.0, … checks: [sim.seed, sim.faults, sim.channel] }`, the amended registration sentence and the marker "*Checked by `se_01_register_adds_the_descriptor_and_the_three_checks`.*" |
| SE-6 | "The SimulationChannel's noise uses the streams `sim.channel/<rx>/<channel>` (CH-5) and MockRadio's LO phases the stream `<device>/lo` (MR-34) (Phase 3, VB-2)." |
| SE-12 | the amended sentence naming all three schema files |

*VB-3 → spec 09*

| Rule | quoted text present |
|---|---|
| MR-1 | the whole descriptor at 1.1.0 with `impl_hash: Some(ContentHash::of_bytes(b"ezsdr.radio.mock 1.1.0"))` (Phase 3, VB-3) |
| MR-2 | "an absent `profile`, or one other than `x310-like 1.1.0` or `ideal 1.1.0`" and the added `MockRadio::with_medium(self, medium: Arc<ezsdr_sim::channel::Medium>) -> MockRadio` sentence |
| MR-3 | both new path-delay rows (45 / 0 and 0 / 0); the fidelity row's `x310-like` cell "rf `none`, or `impairment_model` in channel mode (MR-31)" and its `ideal` cell "every aspect `none` (Vision §13), except rf `impairment_model` in channel mode (MR-31)"; the sentence after the table |
| MR-4 | "the step keys, booleans, strings, repeat, block, envelope and path-delay keys as `One` (Phase 3, VB-3)" |

*VB-4 → spec 09*

| Rule | quoted text present |
|---|---|
| MR-14 | the replaced first sentence in full, and the replaced wakeup clause "the first root tick strictly after the next receive block's last sample, the first root tick strictly after the next transmit block's last sample," |
| MR-15 | the replaced sentence "A block is emitted once its last sample is in the past, as MR-14 says of a receive block: …" |
| §4's algorithm | "emit every receive block whose last sample is before until (MR-14)" and "emit every transmit block whose last sample is before until (MR-15)" |
| Tests | "at 1 Msps with 1 GHz root, block 0 appears at `T0 + 1 999 001`, not at `T0 + 1 999 000`, the instant of its last sample; the next wakeup is at `T0 + 3 999 001`" |

*VB-5 → spec 09*

| Rule | quoted text present |
|---|---|
| MR-17 | the amended opening through "(RM-11)", then the original `SendAsap` sentence unchanged; the marker gains `mr_17_a_target_whose_instant_has_passed_is_late_even_on_the_floor_sample` (Phase 3, VB-5) |

*VB-6 → spec 09*

| Rule | quoted text present |
|---|---|
| MR-25 | the whole amended transmit clause from "emit the transmit block in progress up to the first sample at or after the stop instant (MR-15)…" through "…(Phase 3, VB-6)"; the added sentence "a `Stop` Action cuts at the instant it is handled (Phase 3, VB-6)." |
| MR-18 | the whole amended transmit clause "for a transmit change, the transmit side stops at `e` as MR-25 stops it — … (Phase 3, VB-6)" |
| §4's algorithm | the added comment line "-- a transmit Stop first emits the block in progress up to its first sample at or after `now` (MR-25)" |

*VB-7 → spec 09*

| Rule | quoted text present |
|---|---|
| MR-31 (new) | the whole rule, including the four refusal reasons, the `join` sentence, "…and sets `instance().fidelity.rf` to `impairment_model` (KB-2)" and the `arm` refusal with CH-7 |
| MR-32 (new) | the whole rule, including the three waveform refusals and the segment description |
| MR-33 (new) | the whole rule |
| MR-34 (new) | the whole rule, including the `2π · ⌊x / 2^11⌋ · 2^−53` draw order |
| MR-35 (new) | the whole rule |
| MR-36 (new) | the whole rule, including `tx_clipped` and `rx_clipped` |
| MR-13 | "In channel mode (MR-31) the samples are MR-35's and the pattern is `zero` (Phase 3, VB-7)." |
| MR-15 | "their bytes are never built, because what the transmit side radiates onto the SimulationChannel is computed from the waveform itself (MR-32) (Phase 3, VB-7)" |
| MR-16 | "In channel mode MR-32's three waveform refusals follow the repeat constraints and precede MR-17's decision (Phase 3, VB-7)." |
| MR-18 | "In channel mode an applied gain or frequency also enters its timeline (MR-33), and an applied frequency is a timed tune (MR-34) (Phase 3, VB-7)." |
| MR-27 | "`ezsdr.radio.mock.stats`, `{ rx_blocks, rx_samples, tx_blocks, rx_clipped, tx_clipped }` (MR-36; Phase 3, VB-7)" |
| MR-30 | "In channel mode this includes the received samples, whose noise and LO phases come from `SimRng` streams (CH-5, MR-34), whatever order the stepping loop steps the Mocks in (CH-9) (Phase 3, VB-7)." |
| §4's note | "A `hardware_timed` command does not cut a block: in channel mode each sample takes the value in force at its own instant (MR-33)." |
| §5's M4 | "Phase 3: the channel reads the waveform instead (MR-32)" |
| §5's M8 | "In channel mode the channel replaces it (MR-31, MR-35)" |
| §5's M9 | the rejected option and the ceiling "Phase 3: in channel mode gain scales and frequency gates (MR-33)" |
| §5's M11…M15 | five rows added: M11 transmit content, M12 LO phase, M13 path delay, M14 saturation, M15 device RF behaviour |
| §6 | the twenty-two Phase 3 test names, and the note that their tests are in `crates/ezsdr-mock-radio/tests/mock_channel.rs` with the `World` harness |
| §8 Deferred | "A transmit port and `TX_UNDERFLOW`; the hot-path event layout; `ALIGNMENT` injection (Phase 4); a nonzero receive path delay (MR-35)." |

*VB-8 → spec 09*

| Rule | quoted text present |
|---|---|
| MR-16 | "or it would start at or before the open burst's next sample (Phase 3, VB-8): an open burst hands over to a later burst only with a block that ends at that burst's start and carries `END_OF_BURST` (MR-15), and no such block exists at or before its next sample — …" |

*The six status rows*, as KB-1's introduction requires:

| Spec | Status row ends |
|---|---|
| `design/04-run-and-session.md` | "Amended in Phase 3 by KB-1." |
| `design/05-module-api.md` | "Amended in Phase 3 by KB-1, KB-2." |
| `design/06-kernel-coordinator.md` | "Amended in Phase 3 by KB-1." |
| `design/07-radio-model.md` | "Amended in Phase 3 by VB-1." |
| `design/08-simulation.md` | "Amended in Phase 3 by VB-2; the SimulationChannel is spec 11." |
| `design/09-mock-radio.md` | "Amended in Phase 3 by VB-3…VB-8 (`ezsdr.radio.mock` 1.1.0)." |

**Item 3 — the link checker**

```bash
python3 plan/phase3/tools/check_links.py
```

```text
ok: 322 links
EXIT=0
```

No `BROKEN` line, so the stop condition of §0.3 was not met.

**Item 4 — §0.5's three commands**

| Command | Result |
|---|---|
| `cargo +1.85.0 test --workspace` | 592 passed, 0 failed, 0 ignored |
| `cargo +stable test --workspace` | 592 passed, 0 failed, 0 ignored |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean, no warning or error line |

No code changed in this step, so the counts are Step 4's, as the step expects.

**GV-5 check — the Vision was not edited**

`git status --short` lists nothing under `Ez-SDR_v4_ARCHITECTURE_VISION.md` or
`design/vision/`. Spec 12 §3's four Vision issues and spec 11 §7's four are
collected at Step 7 (`plan/phase3/vision-issues.md`) and applied only at Step X
with the owner's approval.

**Also run, unasked but cheap and named by `handoff.md` §1**

```bash
grep -rhoE 'v3/[A-Za-z0-9_./-]+' design plan Ez-SDR_v4_ARCHITECTURE_VISION.md | sort -u | while read p; do test -e "$p" || echo "MISSING $p"; done
```

printed nothing: every cited `v3/` path still exists.

**Suggested commit subject**

`docs(phase3): apply the Phase 3 amendments to the accepted specs`

The subject the step proposes.

## Step 6

Mutation checks — GV-6, and `20-implementation-plan.md` Appendix C. Appendix C
was read before the run. No mutation was written or edited by me.

**Done-list**

- [x] 83 of 83 killed; exit status 0.
- [x] The working tree is unchanged by the run.

**Item 1 — the run**

`$CLAUDE_JOB_DIR` is not set in this environment, so the scratch directory is
one outside the repository, as the step allows:

```bash
python3 plan/phase3/tools/mutate.py plan/phase3/tools/mutations.json \
  "/private/var/folders/z1/5zzkxm116hsftmk8qc3c1f200000gn/T/opencode/phase3-mutations"
```

`mutations.json` holds 83 entries, matching Appendix C's 83 rows. The runner
copied the repository to the scratch directory, and there, for each mutation,
ran the named test unmutated (it must pass and run at least one test), replaced
`old` by `new`, ran the test again and expected it to fail. The working tree was
never written to.

**Item 2 — the 83 lines**

```text
M01 KB-1 empty store handed to Modules: killed
M02 KB-1 Session waveform not stored: killed
M03 KB-1 prepare gets a snapshot of the store: killed
M04 CH-1 object check removed: killed
M05 CH-1 gain range removed: killed
M06 CH-1 delay range removed: killed
M07 CH-1 noise range removed: killed
M08 CH-2 names not checked: killed
M09 CH-4 gain ignored: killed
M10 CH-4 delay ignored: killed
M11 CH-4 delay rounds down: killed
M12 CH-5 one stream per receiver: killed
M13 CH-5 sigma without the half: killed
M14 CH-6 another Run accepted: killed
M15 CH-6 another document accepted: killed
M16 CH-6 double join accepted: killed
M17 CH-6 zero root rate accepted: killed
M18 CH-7 missing always empty: killed
M19 CH-6 a delay beyond the root's range accepted: killed
M20 CH-4 path ignores rx_channel: killed
M21 CH-2 named omits rx: killed
M22 CH-2 named omits noise keys: killed
M23 CH-5 noise only when a path contributes: killed
M24 MR-14 unit v_after is ceil: killed
M25 MR-14 strict publication -> inclusive: killed
M26 MR-25 stop emits no partial block: killed
M27 MR-32 stop does not cut radiation: killed
M28 MR-17 floor now: killed
M29 CH-4 frequency gate removed: killed
M30 MR-33 timeline ignores tick: killed
M31 MR-34 rx retune keeps phase: killed
M32 MR-34 tx retune keeps phase: killed
M33 MR-34 no random phase: killed
M34 MR-32 transmit path delay ignored: killed
M35 MR-36 tx clip removed: killed
M36 MR-36 rx clip removed: killed
M37 MR-31 no medium accepted: killed
M38 MR-31 ramp accepted: killed
M39 MR-31 channel bound: killed
M40 MR-31 missing at arm: killed
M41 MR-31 fidelity not settled: killed
M42 MR-32 missing bytes accepted: killed
M43 MR-32 size mismatch accepted: killed
M44 RM-13 non-finite accepted: killed
M45 MR-32 segment registered on a wrong origin: killed
M46 MR-34 timed constant equals the untimed phase: killed
M47 MR-33 rx gain ignored: killed
M48 MR-33 tx gain ignored: killed
M49 MR-32 a stop cuts no burst of the current clock: killed
M50 MR-32 a channel the burst lacks radiates: killed
M51 MR-32 cold change does not cut radiation: killed
M52 MR-25 a held burst starting before the cut is withdrawn: killed
M53 MR-32 gain/phase/freq at antenna instant not sample instant: killed
M54 MR-33 timeline takes first change not last: killed
M55 MR-34/35 rx LO rotation sign flipped: killed
M56 MR-31 rx_channel bound not checked: killed
M57 MR-36 rx_clipped counts re only: killed
M58 MR-36 tx_clipped counts re only: killed
M59 MR-34 LO stream keyed by fragment id: killed
M60 MR-17 TIME_ERROR time rounded up: killed
M61 MR-14 wakeup at last sample instant: killed
M62 MR-32/RM-13 multi-channel waveform read planar: killed
M63 MR-34 rx timed tune takes the tx constants: killed
M64 MR-33 tx frequency timeline not updated: killed
M65 MR-25 stop cut one tick later: killed
M66 MR-35 rx frequency timeline ignored by the gate: killed
M67 MR-14 strict publication -> inclusive (end to end): killed
M68 MR-32 a stop ends the segments of every clock: killed
M69 MR-32 a stop withdraws the segments of every clock: killed
M70 MR-32 a cold change keeps the segment generation: killed
M71 MR-31 the medium joins with seed 0: killed
M72 MR-16 a burst at the open burst's next sample is admitted: killed
M73 CH-4 the frequency gate tolerates 1 kHz: killed
M74 MR-31 a reader error lacks the MR-7 prefix: killed
M75 MR-31 a join refusal lacks the MR-31 prefix: killed
M76 MR-25 a stop opens at most one held burst before the cut: killed
M77 CH-4 the frequency gate tolerates half a hertz: killed
M78 MR-16 a SendAsap move onto the open burst emits no refused TIME_ERROR: killed
M79 MR-15 a transmit block is emitted at its last sample's instant: killed
M80 MR-16 the open-burst refusal is judged on the target before MR-17 moves it: killed
M81 MR-16 a burst one sample after the open burst's next sample is refused: killed
M82 MR-16 the open-burst refusal at the next sample only in channel mode: killed
M83 MR-15 strict transmit emission only in channel mode: killed
```

The runner's exit status was 0. `grep -c ": killed$"` gives 83 and no line
carries `SURVIVED`, `NOT FOUND`, `COMPILE ERROR` or `BASELINE FAILED`, so no
stop condition of §0.3 was met.

**Item 3 — the working tree is unchanged**

`git status --short` after the run lists exactly the files Step 5 left: 28
modified (`crates/`, `design/04…09`, `schemas/SCHEMA_CHANGELOG.md`) and the six
untracked files the five patches created, plus this notes file. The runner
works only on its copy.

**The scratch directory**

`…/T/opencode/phase3-mutations`, 1.4 GB (mostly the copy's own `target`
directory, which the runner pointed `CARGO_TARGET_DIR` at). It is outside the
repository and holds the runner's `.ezsdr-mutation-scratch` marker, so a later
run of the runner may delete and recreate it. It is not part of the repository
and nothing in it is needed; delete it when convenient.

## Where Phase 3 stands after Step 6

Recorded here for the owner's convenience, not as a claim of anything the plan
does not already say. All of it was measured in this session:

| Check | Result |
|---|---|
| `cargo +1.85.0 test --workspace` | 592 passed, 0 failed, 0 ignored |
| `cargo +stable test --workspace` | 592 passed, 0 failed, 0 ignored |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean |
| `kernel_surface` `OV-23b` | `116 NEW: items of 292 public items` |
| Appendix C's mutations | 83 of 83 killed, exit 0 |
| `check_links.py` | `ok: 322 links`, exit 0 |
| `v3/` path check of `handoff.md` §1 | printed nothing |

Per-crate counts, the "Per-crate counts after Step 4" line of the plan:

| Crate | Count | Plan |
|---|---:|---:|
| `ezsdr-kernel` | 446 | 446 |
| `ezsdr-radio` | 10 | 10 |
| `ezsdr-sim` | 14 | 14 |
| `ezsdr-sim-engine` | 7 | 7 |
| `ezsdr-hostmem` | 2 | 2 |
| `ezsdr-link-host` | 2 | 2 |
| `ezsdr-sink` | 2 | 2 |
| `ezsdr-sink-capture` | 15 | 15 |
| `ezsdr-mock-radio` | 58 | 58 |
| `ezsdr-acceptance` | 36 | 36 |
| **total** | **592** | **592** |

Invariants the plan's exit criteria name, checked opportunistically because they
are cheap and none of them is Step 7's work to redo:

- **PO-4 / no new dependency.** `git status --short Cargo.lock Cargo.toml
  'crates/*/Cargo.toml'` is empty: no manifest and no lock entry changed. The
  Kernel's direct dependencies are still exactly `serde`, `serde_json`,
  `schemars`, `sha2`.
- **No `#[ignore]`.** `grep -rn "#\[ignore" crates/` finds none.
- **PO-6 / the schema changelog.** `schemas/` holds 58 JSON Schemas (57 before
  this phase plus `sim/channel.v1.json`), and `SCHEMA_CHANGELOG.md`'s newest
  entry is "v1 — Phase 3 — the SimulationChannel", which names the new
  `sim/channel` schema and CH-1 and CH-10.
- **GV-5.** Nothing under `Ez-SDR_v4_ARCHITECTURE_VISION.md` or
  `design/vision/` is modified. Spec 11 §7's four and spec 12 §3's four Vision
  issues are still to be collected into `plan/phase3/vision-issues.md` at
  Step 7.

**Stop condition of §0.3 was never met.** No `git apply --check` printed
anything, no command failed, no test count differed from the step's, no mutation
survived, no other check printed anything other than what the step says, no
spec and patch disagreed, and nothing asked for anything `AGENTS.md` or a spec
forbids. Nothing was repaired, and no file a patch created or changed was
edited afterwards.

**Suggested commit subjects, one per step**

| Step | Subject |
|---|---|
| 0 | `chore(phase3): record the Phase 3 base check` |
| 1 | `feat(kernel): let Modules read Run inputs by hash (Phase 3 KB-1, KB-2)` |
| 2 | `feat(sim): add the SimulationChannel (Phase 3 spec 11, VB-1, VB-2)` |
| 3 | `feat(mock-radio): run MockRadio 1.1.0 on the SimulationChannel (Phase 3 VB-3…VB-8)` |
| 4 | `test(acceptance): carry Vision §58 #8 and deterministic Runs (Phase 3)` |
| 5 | `docs(phase3): apply the Phase 3 amendments to the accepted specs` |
| 6 | `test(phase3): record the Phase 3 mutation results (83 of 83 killed)` |

Steps 1–5 change code, schemas and design text and are the substance of the
phase; 0 and 6 change only this file, so the owner may fold them into Step 1's
and Step 5's commits or keep them separate.

**Stopped here, as the step says.** Review C
(`plan/phase3/prompts/02-review-c.txt`) is the owner's. Step 7 — the exit
tables of `plan/phase3/exit-review/`, `vision-issues.md` and the `handoff.md`
update — waits for the owner to say Review C is closed.
