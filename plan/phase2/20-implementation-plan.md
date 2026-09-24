# Phase 2 — implementation plan

| Field | Value |
|---|---|
| Status | **Accepted at Gate P** (owner, 2026-09-24; `00-overview.md` §11), together with specs 06–10. This is the order of work. |
| Audience | The agent that implements Phase 2. This file tells you **what to do, in what order, and how to know you are done**. The specs `06`–`10` tell you **what is true**. You need nothing else except the repository itself. |
| Base | The Kernel code of commit `96976c5`: the patch of step 1 is made against it. Later commits that touch only `plan/`, `handoff.md` or other documents (such as `8456975`, which added these documents) do not move the base. Step 1 fails loudly if the code has moved. |
| Toolchains | Rust `1.85.0` (the MSRV) and `stable`. Both must pass at the end of every step. |
| Language | English, like the specs. |

Documents you will read, in this order, before step 1:

1. `AGENTS.md` (the repository's rules; §3 and §7 bind you).
2. `plan/phase2/00-overview.md` (scope, decisions, governance PO-1…PO-12).
3. The spec a step names, **at the start of that step**, in full.

---

## 0. Rules for the implementer

These rules exist because Phase 1 was implemented, reviewed twenty-five times, and found to contain rules that had been "settled by review rather than written down" (`plan/phase1/history.md` §1). Follow them literally.

### 0.1 Order

Do the steps in numerical order. Do not start step *n + 1* until every item of step *n*'s **done-list** is true. Do not do work from a later step early, even when it looks convenient: several later steps depend on the exact state an earlier step leaves.

### 0.2 The specs are the authority

- A rule (`KA-n`, `KC-n`, `UC-n`, `RM-n`, `SE-n`, `MR-n`, `HD-n`, `PO-n`) is normative. This plan explains how to implement it; where this plan and a spec disagree, **the spec is right and this plan is wrong**.
- Never change a rule to make your code pass. Never weaken a test to make it pass. Never add `#[ignore]`.
- Do not add a feature, a key, an event kind, a public function, a dependency or a crate that no rule asks for (AGENTS.md §6: scope creep is a defect).

### 0.3 When to stop and ask

Stop, and write what you found in `plan/phase2/implementation-notes.md` (create the file if it does not exist, one `## Step n` section per step), when any of these happens:

- a rule cannot be implemented as written (for example, it names a function the Kernel does not have, or two rules contradict each other);
- a test's expected value in a spec is different from what the rule it tests implies;
- `git apply` of a patch fails;
- a Phase 1 test starts failing and the cause is not your own mistake;
- anything in this plan tells you to do something the specs forbid.

Write: the step, the rule id, the exact text you are looking at, what you expected, and what you found. Then stop working and report. **Do not guess and continue.** A wrong guess costs more than the wait.

### 0.4 Git

- You never run `git commit`, `git push`, `git rebase`, `git reset`, `git stash` or `git checkout`, on anything. The owner commits (AGENTS.md §7). To undo your own edit, edit the file back.
- At the end of each step, append to `plan/phase2/implementation-notes.md` under `## Step n`: the done-list with each item marked `[x]`, the test counts printed by the commands of §0.5, and a suggested commit subject in Conventional Commits style (for example `feat(kernel): add the Run coordinator entry and prepare pipeline`).
- Then continue with the next step, **except** after step 8 (Review K) and step 15 (Review M), where you stop and report: the owner runs a review there.
- Never touch: `v3/` (a git worktree — never `git add` it, never delete it); `design/v4-vision-audit.md`; `design/v4-vision-rereview.md`; `design/archive/`; the Vision (`Ez-SDR_v4_ARCHITECTURE_VISION.md`, `design/vision/`); files whose name contains ` (1)` (Google Drive conflict copies).

### 0.5 The commands

Run all three at the end of every step, from the repository root. All must pass.

```bash
cargo +1.85.0 test --workspace
```

```bash
cargo +stable test --workspace
```

```bash
cargo +stable clippy --workspace --all-targets -- -D warnings
```

Do **not** run `cargo fmt`: the existing Kernel code is not rustfmt-formatted, and `cargo fmt` would rewrite hundreds of unrelated lines. Match the style of the file you are editing. In a new crate, write ordinary rustfmt-like style by hand.

Regenerate committed JSON Schemas only when a step says so:

```bash
EZSDR_UPDATE_SCHEMAS=1 cargo +stable test -p ezsdr-kernel --test schema_freeze
```

### 0.6 Code conventions (all crates)

1. **Doc comments cite rules.** Every `pub` item has a `///` doc comment, and in `ezsdr-kernel` it must contain a rule id with one of the prefixes `OV- TM- SC- SB- RS- MA- KA- KC- UC-` (the `kernel_surface` test enforces this, OV-23, KA-20). In the other crates, cite the spec's own prefix (`RM-`, `SE-`, `MR-`, `HD-`, `PO-`). End a doc comment with `Rule: KC-24.` or put the id in parentheses, as the Kernel does.
2. **New Kernel public items** go on `crates/ezsdr-kernel/tests/kernel_surface_allow.txt` exactly as spec 06 §4 lists them (PO-5). Items inside the coordinator's submodules are private or `pub(super)`, never `pub`.
3. **Banned tokens.** `crates/ezsdr-kernel/tests/banned_tokens.txt` lists sixteen words (`uhd`, `soapy`, `hackrf`, `rfnoc`, `replay`, `cuda`, `wasmtime`, `dev/net/tun`, `tuntap`, `af_xdp`, `dpdk`, `802.11`, `otfs`, `ibfd`, `taint`, `prometheus`) that must not appear in any line of `crates/ezsdr-kernel/src/`, **including comments**, case-insensitively, even inside an identifier (`uhd_open` is a hit). Write "the hardware Provider" instead of "UHD", "re-application" instead of "replay". The other crates are not scanned, but keep them clean too.
4. **Determinism (PO-11).** In the Phase 2 code — the nine new crates' `src/` and `crates/ezsdr-kernel/src/coordinator/` — iterate `BTreeMap`, `BTreeSet` or `Vec` only, never `HashMap` or `HashSet`; never call `SystemTime::now()` or `Instant::now()`; never spawn a thread (`std::thread::spawn`) — `run_cleanup` already does that for you; draw random numbers only from `ezsdr_sim::SimRng`. Tests may use `Instant` to measure wall time.
5. **No `unsafe`.** Every new crate's `src/lib.rs` starts with `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]` (PO-2).
6. **No new dependencies.** A crate may depend only on what `00-overview.md` §5's table allows. Never add a crate from crates.io (PO-4).
7. **Locks.** Lock a `Mutex` with `.lock().unwrap_or_else(|e| e.into_inner())`, never `.lock().unwrap()`: a panicking Module must not poison the Run (KC-30).
8. **Panics.** Library code does not panic on input. `expect("…")` is allowed only where the value cannot be absent by construction, and its message says why (for example `ResourceId::parse(KERNEL_SOURCE).expect("a valid literal")`).
9. **Errors carry rule ids.** Every refusal's message begins with the rule that refuses: `"KC-10: …"`, `"MR-7: …"`, `"RM-19: <fragment>: …"`. Tests match on those prefixes.
10. **Tests** follow OV-19: plain `#[test]` functions in the crate's own `tests/` directory, named with the lowercased rule id they prove, `-` replaced by `_` (`KC-24` → `kc_24_…`). A test's assertion is the rule's obligation (OV-3), never "the code does what it does".

### 0.7 Mutation checks (PO-12)

Every step's done-list names **mutation checks**: refusals whose test must fail when the refusal is disabled. For each one:

1. disable the refusal with the smallest possible edit, usually by prefixing its condition with `false &&` (for `if cond {`) or by replacing a `return Err(…)` with `{}`;
2. run only the named test (`cargo +stable test -p <crate> --test <file> <test_name>`) and confirm that it **fails**;
3. undo the edit exactly and confirm that the test passes again.

Record each check as `mutation: <rule> / <test>: fails when disabled — yes` in the notes. A test that still passes with its refusal disabled is a broken test: fix the test (never the rule) and repeat.

### 0.8 Where to find things

- The Kernel's public API after step 1 is listed, with exact signatures, in **Appendix A** of this file. Use it instead of guessing a signature. When Appendix A and the source disagree, the source is right; note the difference.
- Every rule of specs 06–10 is mapped to the step that implements it and the test that proves it in **Appendix B**.

---

## Steps at a glance

| Step | What | Spec | Ends with |
|---|---|---|---|
| 1 | Apply the Kernel amendment patch | 06 §2 | 361 tests green |
| 2 | Write the amended rule text into `design/01…05` | 06 §2 | links resolve |
| 3 | Test doubles for the coordinator | 06 §12 | suite green |
| 4 | Coordinator A: types, entry, validate and plan, the end path, the Manifest | 06 KC-1…KC-7, KC-33…KC-45 (part) | `kc_01_*`, `kc_02_*`, `kc_04_*` |
| 5 | Coordinator B: links, events, prepare, arm, T0, start | 06 KC-8…KC-15, KC-18 | `kc_10_*`…`kc_15_*`, `ma_07_*` |
| 6 | Coordinator C: schedule, stepping loop, drain | 06 KC-16…KC-23, KC-29, KA-11, KA-12 | `kc_16_*`…`kc_23_*`, `kc_39_*` |
| 7 | Coordinator D: admission, dispatch, Sessions, Lease | 06 KC-24…KC-28, KC-35…KC-37, KA-6 | `kc_24_*`…`kc_28_*`, `rs_17_*`, `kc_36_*`, `kc_37_*` |
| 8 | Coordinator E: failures, Policy, marks, the complete Manifest | 06 KC-30…KC-32, KC-40…KC-45 | the rest of 06 §12 — **then stop: Review K** |
| 9 | The nine new crates as empty workspace members | 00 §5 | workspace builds |
| 10 | `ezsdr-sim` and `ezsdr-sim-engine` | 08 | `se_*` |
| 11 | `ezsdr-radio` | 07 | `rm_*` |
| 12 | `ezsdr-hostmem`, `ezsdr-link-host`, `ezsdr-sink` | 10 §2–§4 | `hd_02`…`hd_06` |
| 13 | `ezsdr-sink-capture` | 10 §5 | `hd_07`…`hd_13` |
| 14 | `ezsdr-mock-radio` | 09 | `mr_*` |
| 15 | `ezsdr-acceptance` | 00 §8 | `v58_*`, `v61_*`, `po_*` — **then stop: Review M** |
| 16 | Exit tables, Phase 1 markers, handoff | 00 §10 | Gate X |

---

## Step 1 — Apply the Kernel amendment patch

**Implements:** KA-1, KA-2, KA-3, KA-4, KA-5, KA-7, KA-8, KA-10, KA-11 (doc comment), KA-14 (the SB-22h check), KA-15, KA-17, KA-20, KA-21 (spec 06 §2, "The amendment patch").

**Read first:** spec 06 §2 in full.

The patch was written and verified before this plan was accepted. You apply it; you do not retype it.

1. Confirm the base: the Kernel code must be exactly that of `96976c5`, and the working tree clean.

   ```bash
   git diff --quiet 96976c5 HEAD -- crates schemas Cargo.toml Cargo.lock && echo base-ok
   ```

   ```bash
   git status --short
   ```

   The first must print `base-ok`; the second must print nothing. If either differs, stop (§0.3).

2. Apply:

   ```bash
   git apply plan/phase2/patches/01-kernel-amendments.patch
   ```

   It touches exactly 28 files: 12 under `crates/ezsdr-kernel/src/`, 9 under `crates/ezsdr-kernel/tests/`, 6 schemas and `schemas/SCHEMA_CHANGELOG.md`. If `git apply` prints any error, stop.

3. Run the three commands of §0.5. Each `cargo test` must report **361 passed, 0 failed** in total (353 Phase 1 tests, one of them replaced, plus 8 new ones).

4. Read the diff (`git diff --stat`, then `git diff`) once, so that you know what changed. In particular note:
   - `PrepareContext` has no lifetime and owns `Arc` handles (KA-1);
   - `SampleBlock::new_host` and `host_bytes` exist and `HostMemoryAccess` is gone (KA-3);
   - `AdmissionCheck::check`, `AdmissionCheckRegistry::run` and `Admitter::admit` take per-fragment maps `&BTreeMap<Ident, BTreeMap<Key, Value>>` (KA-4);
   - `ProviderInstance.min_command_lead` exists (KA-7);
   - `binding::start_lead_ns` exists (KA-8);
   - `ClockRegistry::domains()` and `declared_sample_clocks()` exist (KA-2);
   - `ManualTimeAuthority::next_due()` exists (KA-17);
   - `SessionLog::check_entry` exists (KA-21);
   - `plan::validation::binding_description` exists, crate-private (KA-9's grouping, KC-4).

**Done-list**

- [ ] `git apply` succeeded; 28 files changed.
- [ ] 361 passed on 1.85.0 and on stable; clippy clean.
- [ ] Nothing else was edited.

No mutation checks: the patch's tests were mutation-checked when it was made.

---

## Step 2 — Write the amended rule text into `design/01…05`

**Implements:** PO-9 for every KA. **Read first:** spec 06 §2.

Each KA in spec 06 §2 quotes new rule text in double quotes after a bold label such as **TM-13a amended** or **New rule MA-5a**. Copy that text **verbatim** into the accepted spec named below, with these mechanics:

- "*X amended* (sentence added)": insert the quoted sentence(s) at the end of rule X's paragraph, **before** its italic `*Checked…*` or `*Forward…*` marker if it has one.
- "*X amended* (the … sentence replaced)": find the sentence the parenthesis names inside rule X and replace exactly that sentence.
- "*X replaced*": replace the whole paragraph of rule X after its bold `**X**` label.
- "*New rule X*": insert a new bullet `- **X** …` immediately after the rule it extends (`MA-5a` after `MA-5`).
- Keep the trailing *(Phase 2, KA-n)* that each quotation already ends with.
- Rule ids are never renumbered (OV-1).

| KA | File | What to edit |
|---|---|---|
| KA-1 | `design/05-module-api.md` | §4: replace the `PrepareContext` row of the type block with the `text` block of KA-1; add new rule **MA-5a** after MA-5; **MA-46**: replace the Plugin mapping sentence |
| KA-2 | `design/01-time-model.md` | **TM-13a**: replace the forward sentence; **TM-12**: add the sentences |
| KA-3 | `design/02-stream-contract.md` | **SC-8**: replace the rule; §7 decision **S5**: change the chosen option and add the rejected option as KA-3 says |
| KA-4 | `design/03-spec-and-binding.md` | §4: replace the `AdmissionCheck` shape with KA-4's `text` block; **SB-30**: add the sentence after "at three points" |
| KA-4 | `design/04-run-and-session.md` | **RS-17**: replace "`Admitter::admit` over one Action's key and value" as KA-4 says |
| KA-5 | `design/03-spec-and-binding.md` | **SB-7**: replace the sentence "`coerce` is called at most once per bound node…" |
| KA-5 | `design/05-module-api.md` | **MA-11**: add the sentence |
| KA-6 | `design/04-run-and-session.md` | **RS-17**: insert step 0 before "the registered admission checks" |
| KA-7 | `design/05-module-api.md` | **MA-10**: add the sentence |
| KA-7 | `design/04-run-and-session.md` | **RS-19**: replace "the earliest time the bound Provider's envelope allows" as KA-7 says |
| KA-7 | `design/03-spec-and-binding.md` | **SB-22f**: extend as KA-7 says |
| KA-8 | `design/05-module-api.md` | **MA-41**: add the sentence |
| KA-8 | `design/03-spec-and-binding.md` | **SB-16**: extend the sentence beginning "A `schedule` entry places…"; **SB-43**: replace the forward sentence |
| KA-9 | `design/05-module-api.md` | **MA-7**: add the sentence |
| KA-10 | `design/05-module-api.md` | add a subsection "Update classes" at the end of §5 holding UC-1…UC-6 of spec 06 §10, verbatim, each as a `- **UC-n** …` bullet; **MA-24**: change its second sentence as KA-10 says |
| KA-10 | `design/01-time-model.md` | **TM-13e**: add the sentence |
| KA-11 | `design/05-module-api.md` | **MA-29**: add the sentence; **MA-30**: replace its italic *Ceiling* sentence and the `text` block's "None ends the Run", as KA-11 says |
| KA-12 | `design/04-run-and-session.md` | **RS-6**: add KA-12's paragraph after the algorithm; **RS-9**: add the sentence |
| KA-12 | `design/05-module-api.md` | **MA-7**: replace the order sentence with KA-12's "MA-7 amended" text |
| KA-13 | `design/04-run-and-session.md` | **RS-39**: add the sentences |
| KA-13 | `design/05-module-api.md` | **MA-10**: add the sentence |
| KA-14 | `design/03-spec-and-binding.md` | **SB-22h**: replace the rule |
| KA-15 | `design/05-module-api.md` | **MA-27**: add the sentence |
| KA-16 | `design/01-time-model.md` | **TM-18**: add the sentence |
| KA-17 | `design/01-time-model.md` | **TM-17a**: add the sentence |
| KA-18 | `design/02-stream-contract.md` | **SC-20**: replace the forward sentence |
| KA-18 | `design/04-run-and-session.md` | **RS-38**: change `sections` as KA-18 says |
| KA-19 | `design/04-run-and-session.md`, `design/02-stream-contract.md`, `design/05-module-api.md` | the markers of **RS-4**, **RS-25**, **RS-25a**, **SC-24a**, **MA-39**, **MA-17**, **MA-19b**, replaced by KA-19's text. The other Phase 1 markers are step 16's |
| KA-20 | `plan/phase1/00-overview.md` | **OV-23**: add the sentence |
| KA-21 | `design/04-run-and-session.md` | **RS-15**: add the sentence |
| KA-22 | `design/04-run-and-session.md` | the header's "Not in scope" row and the closing out-of-scope list: replace the fault-injection item and extend the payload item as KA-22 says; **RS-31**: replace its marker |

Then, in each edited spec's own **Tests** table, add one row per new test the KA names (the test name, its input and expectation in one line each, and the rule ids). The new tests of step 1 are `tm_12_domains_lists_every_registered_domain`, `tm_13a_declared_clocks_are_listed_before_registration`, `tm_17_next_due_reports_without_advancing` (spec 01); `sc_08_host_bytes_only_for_a_block_that_carries_them` replacing `sc_08_buffer_map_host_none_for_gpu_domain` (spec 02); `sb_30_the_check_sees_each_fragment_s_own_value`, `sb_07_coerce_refuses_a_combination`, `sb_07_a_stray_rejection_is_a_violation`, `sb_22f_min_command_lead_is_in_host_monotonic`, `ma_41_ezsdr_time_is_a_closed_set` (spec 03; `ma_41_…` goes in the table that already lists the other `ma_41_*` tests).

Finally add one line to the top of each edited spec's header table, in the `Status` row, after the existing text: "Amended in Phase 2 by KA-n, …, listing the KAs applied" (for example "Amended in Phase 2 by KA-2, KA-16, KA-17.").

Check that nothing links to a missing file:

```bash
grep -rhoE '\]\(([^)#]+)' design plan AGENTS.md handoff.md | sed 's/](//' | grep -v '^http' | sort -u | while read p; do test -e "$p" || test -e "design/$p" || test -e "plan/phase1/$p" || test -e "plan/phase2/$p" || echo "CHECK $p"; done
```

Every line it prints must be a relative link that does resolve from the file that contains it; open each printed file and confirm by hand.

**Done-list**

- [ ] Every row of the table above is applied, verbatim.
- [ ] Every new test is in its spec's test table.
- [ ] `git diff --stat` shows edits only in `design/01…05` and `plan/phase1/00-overview.md`.
- [ ] The three commands of §0.5 still pass (nothing in code changed).

---

## Step 3 — Test doubles for the coordinator

**Read first:** spec 06 §3, §4 and §12.

The coordinator takes ownership of every Module (`Box<dyn Provider>` in the `Assembly`), so a test cannot read a double's fields after the Run starts. Every new double therefore writes what happens to it into a shared **probe** the test keeps a clone of.

Create `crates/ezsdr-kernel/tests/support/run_doubles.rs`, and in `crates/ezsdr-kernel/tests/support/mod.rs` add, after `pub use doubles::*;`:

```rust
pub mod run_doubles;
#[allow(unused_imports)]
pub use run_doubles::*;
```

Start the file with `//! Test doubles for the Phase 2 coordinator (spec 06 §12). Never compiled into `src/`.` and `#![allow(dead_code)]`. Everything in it is `pub`.

### 3.1 `Probe`

```rust
#[derive(Clone, Default)]
pub struct Probe(Arc<Mutex<Vec<String>>>);
impl Probe {
    pub fn new() -> Probe;
    pub fn record(&self, line: impl Into<String>);           // appends
    pub fn lines(&self) -> Vec<String>;                       // a copy, in order
    pub fn with_prefix(&self, prefix: &str) -> Vec<String>;   // lines starting with `prefix`, in order
}
```

Every double records lines of the form `<name>:<event>` where `<name>` is the double's own name given at construction. The events and their exact text are listed with each double below; tests compare these strings, so write them exactly.

### 3.2 `SimAuthority`

An `Authority` over `ManualTimeAuthority`:

```rust
pub struct SimAuthority { inner: Arc<ManualTimeAuthority>, descriptor: AuthorityDescriptor }
impl SimAuthority {
    /// Allocates a root in `clocks`, registers it as `Root { 1_000_000_000/1, Arbitrary { set_by: "test.sim" } }`,
    /// and builds a ManualTimeAuthority over it with `pacing`. The descriptor is
    /// `{ module, governs: [root, ClockDomainId::HOST_MONOTONIC], pacing }`. Returns the root id too.
    pub fn new(clocks: &Arc<ClockRegistry>, module: ModuleRef, pacing: Pacing) -> (SimAuthority, ClockDomainId);
    pub fn manual(&self) -> Arc<ManualTimeAuthority>;   // a clone, so a test can read `now`
}
impl Authority for SimAuthority {
    fn descriptor(&self) -> &AuthorityDescriptor { &self.descriptor }
    fn time(&self) -> Arc<dyn TimeAuthority> { self.inner.clone() }
    fn next_wakeup(&self) -> Option<TimePoint> {
        let t = self.inner.next_due()?;          // KA-17
        let _ = self.inner.advance_to(t);        // fires the callbacks due at t; an Err (its callback cap)
        Some(t)                                  // leaves the rest pending, and the next call returns t again (KC-22)
    }
}
```

### 3.3 `SteppedProvider`

A stepped Provider double. It wraps a `TestProvider` for everything the Phase 1 double already does (its tree, `coerce`, the `PrepareReport`), and adds time, events, Actions, links and failure injection.

```rust
pub struct SteppedProvider {
    name: String,
    inner: TestProvider,                      // TestProvider::new(path, count) or a customised one
    probe: Probe,
    // behaviour, set by the builders below
    wakeups: Vec<i64>,                        // primary-root ticks scheduled at `start`
    declare: Vec<(String, u64, u64)>,         // (stream path, num, den) declared at `prepare`
    register_at_arm: Vec<String>,             // stream paths registered at `arm`, origin = now(primary).ticks
    emit: Option<(EventKind, Severity, i64)>, // emitted once, at the first step at or after the tick
    panic_in_step: bool,
    device_lost_at: Option<i64>,
    step_error_at: Option<i64>,
    reschedule_forever: bool,
    wedge_in_stop: bool,
    tail_blocks: u32,
    publish_every: Option<i64>,
    // kept from PrepareContext (MA-5a)
    time: Option<Arc<dyn TimeAuthority>>,
    clocks: Option<Arc<ClockRegistry>>,
    events: Option<Arc<dyn EventSink>>,
    actions: Option<Arc<dyn ActionReceiver>>,
    outs: Vec<Arc<dyn DataLink>>,             // the StreamOut ends of ctx.links
    handles: Vec<SampleClockHandle>,          // what `declare` returned
    stopped_at: Option<i64>,
    emitted: bool,
    lost_reported: bool,
}
```

Builders, each `pub fn …(mut self, …) -> SteppedProvider`:

| Builder | Effect |
|---|---|
| `SteppedProvider::new(name: &str, inner: TestProvider, probe: &Probe)` | sets `inner.instance.driving.stepped = true` (add a `pub fn stepped(mut self) -> TestProvider` builder to `TestProvider` in `doubles.rs` that sets `self.instance.driving.stepped = true`, and call it) |
| `with_wakeups(&[i64])` | ticks for `start` to schedule |
| `declaring(stream: &str, num: u64, den: u64)` | a SampleClock to declare at `prepare`, on the primary root |
| `declaring_on(stream: &str, root: ClockDomainId, num: u64, den: u64)` | the same on another root (`kc_16_off_root…`) |
| `registering_at_arm(stream: &str)` | register that declared clock at `arm` |
| `emitting(kind: &str, severity: Severity, at: i64)` | one event, emitted with `emit_control` |
| `panicking_in_step()` | `step` panics |
| `device_lost_at(t)` | the first `step` with `until.ticks >= t` returns `Err(ModuleError { kind: DeviceLost, message: "test: device lost", detail: Null })`; later steps return `Ok(progressed: false)` |
| `step_error_at(t)` | the first `step` with `until.ticks >= t` returns `Err(ModuleError { kind: Rejected, message: "test: step error", … })` |
| `rescheduling_forever()` | every `step` schedules a no-op callback at `until` |
| `wedged_in_stop()` | `stop` blocks forever: `loop { std::thread::park(); }` (a test double, not Module code; PO-11 does not scan `tests/`) |
| `with_tail(n: u32)` | after `stop(Orderly)`, publish `n` blocks on every `outs` link, one per wakeup, the k-th at `stopped_at + 10·k` (k = 1…n); nothing after `stop(Abort)` |
| `publishing_every(n: i64)` | publish one block at `start + n`, `start + 2n`, … (never at the start instant itself), until `stop`; keep `next_publish: Option<i64>` |

Behaviour of each method — **the probe lines are exact**:

- `instance()`, `coerce()`: delegate to `inner`.
- `prepare(f, ctx)`: record `"<name>:prepare:<f.id>"`; keep `ctx.time`, `ctx.clocks`, `ctx.events`, `ctx.actions` (clones of the `Arc`s) and the `StreamOut` endpoints of `ctx.links`; for each `declare` entry call `ctx.clocks.declare_sample_clock(ResourceId::parse(stream)?, ctx.time.primary_root(), Rational::new(num, den)?)` and keep the handle (map any error to `ModuleError::rejected`); record `"<name>:links:<component>.<port>:<in|out>"` for each `ctx.links` entry (`in` for `StreamIn`, `out` for `StreamOut`); then return `inner.prepare(f, ctx)`.
- `arm()`: record `"<name>:arm"`; `inner.fail_if(FailAt::Arm)` semantics (fail when `inner.fail_at == FailAt::Arm`; make `fail_if` `pub` in `doubles.rs`); for each `register_at_arm` stream, `clocks.register_sample_clock(&handle, now(primary).ticks)`.
- `start(at)`: record `"<name>:start:<at.ticks or none>"`; fail when `FailAt::Start`; schedule a no-op callback (`Box::new(|_| {})`) at each `wakeups` tick; if `publish_every = Some(n)`, set `next_publish = Some(now + n)` and schedule a no-op there.
- `step(until)`, in this order:
  1. record `"<name>:step:<until.ticks>"`, then `"<name>:now:<time.now(time.primary_root()).ticks>"`;
  2. drain `actions.recv()`; for each Action record `"<name>:action:<Variant>:<target path or ->@<until.ticks>"` (`Variant` is `TxBurst`, `SetTimer`, `UpdateParameter`, `PeripheralCommand`, `Emit`, `Stop` or `Abort`); for `UpdateParameter` also record `"<name>:update:<key>=<value as JSON>"`; for `TxBurst` also `"<name>:burst_at:<at.time_point.ticks>"`;
  3. if `panic_in_step`: `panic!("test: panic in step")`;
  4. `device_lost_at` / `step_error_at` as the table says;
  5. if `emit` is due and not yet emitted: `events.emit_control(Event { source: instance().id.clone(), time: until, severity, kind, payload: json!({}) })`;
  6. publishing: if not stopped and `next_publish == Some(until.ticks)`, publish one block (below), set `next_publish = Some(until.ticks + n)` and schedule a no-op there; if stopped orderly with tail blocks remaining and `until.ticks == stopped_at + 10·k` for the next `k`, publish one and schedule the next;
  7. if `reschedule_forever`: schedule a no-op at `until`;
  8. return `Ok(StepOutcome { progressed })`, `progressed` true when an Action was drained or a block published.
- A published block is `support::block(support::header(TimePoint::new(<the first declared clock's domain if registered, else the primary root>, until.ticks), 10, 1))` sent with `link.publish(b.clone())` on every `outs` link; record `"<name>:published:<until.ticks>"`.
- `stop(mode)`: record `"<name>:stop:<Orderly|Abort>"`; if `wedge_in_stop`, park forever; fail when `FailAt::Stop`; keep `stopped_at = now(primary).ticks`; under `Orderly` with `tail_blocks > 0`, schedule a no-op at `stopped_at + 10`.
- `cleanup()`: record `"<name>:cleanup"`; drop every kept handle (set the `Option`s to `None`, clear `outs`).

### 3.4 `RecordingSink`

```rust
pub struct RecordingSink {
    name: String, descriptor: SinkDescriptor, probe: Probe,
    ins: Vec<Arc<dyn DataLink>>, received: Vec<BlockHeader>,
    spans: Vec<(i64, i64)>,          // artifacts to return, as (first, end) primary-root ticks
    fail_stop: bool, primary: Option<ClockDomainId>,
}
```

- `RecordingSink::new(name, probe)`: descriptor `{ module: ezsdr.test.sink 1.0.0, kind: test.recorder, memory_domains: [MemoryDomainId::local(0)], contracts: [ezsdr.stream.cf32], artifact_kinds: [test.capture] }`.
- `returning_spans(&[(i64, i64)])`, `failing_stop()`.
- `prepare(f, ctx)`: record `"<name>:prepare:<f.id>"`; keep the `StreamIn` ends; keep `ctx.time.primary_root()`; record the links as `SteppedProvider` does; return `PrepareReport { fragment: f.id, effective: {}, coercions: [], warnings: [] }`.
- `arm`, `start`: record `"<name>:arm"`, `"<name>:start"`.
- `step(until)`: record `"<name>:step:<until.ticks>"`; `receive()` every block from every `ins` link; for each record `"<name>:block:<first_sample_time.ticks>"` and keep its header; `progressed` = at least one block.
- `stop(mode)`: record `"<name>:stop:<Orderly|Abort>"`; if `fail_stop` return `Err(ModuleError::rejected("test: sink stop failed"))`. Otherwise return one `ArtifactRef` per `spans` entry — `{ id: "<name>_<i>", kind: test.capture, uri: "memory://<name>_<i>", hash: ContentHash::of_bytes(b"<name>_<i>"), size_bytes: 0, partial: mode == Abort, marks: [], continuity: [ContinuityMap { domain: primary, channels: 1, valid: vec![vec![]], gaps: vec![], channel_gaps: vec![], first: TimePoint(primary, first), end: TimePoint(primary, end) }] }` — or, with no `spans`, one artifact `<name>_0` with `continuity: []`.
- `cleanup()`: record `"<name>:cleanup"`; drop the links.

### 3.5 `ProbeExecutor`

```rust
pub struct ProbeExecutor { name: String, descriptor: ExecutorDescriptor, probe: Probe,
                           submit: Option<Action>, out: Option<Arc<dyn ActionSubmitter>>, submitted: bool }
```

- `ProbeExecutor::new(name, probe)`: descriptor as `TestExecutor::new(MemoryDomainId::local(0))`'s (module `ezsdr.test.executor 1.0.0`, kind `test.executor`, impl kind `test.impl`).
- `submitting(action: Action)`: at its first `step`, `out.submit(action)` and record `"<name>:submit:ok:<id.0>"` or `"<name>:submit:err:<violations[0].check>:<violations[0].reason>"`.
- `submitting_at(tick: i64, action: Action)`: the same, but at the first `step` whose `until.ticks >= tick` (in `kc_24_a_module_action_during_cleanup_is_refused` that step is in cleanup step 3's drain).
- `prepare(island, ctx)`: record `"<name>:prepare:island_<island.id.local>:<comma-separated sorted component names of ctx.components>"`; keep `ctx.actions_out`; return `PrepareReport { fragment: Ident "island_<island.id.local>", effective: {}, coercions: [], warnings: [] }`.
- `arm`, `start`, `stop`, `cleanup`: record `"<name>:arm"`, `"<name>:start"`, `"<name>:stop:<mode>"`, `"<name>:cleanup"`. `step`: record `"<name>:step:<until.ticks>"`, then the submit rule; `progressed` true only on the step that submitted.

### 3.6 `TestLinkModule`

```rust
pub struct TestLinkModule { descriptor: LinkDescriptor, probe: Probe }
impl TestLinkModule {
    pub fn new(probe: &Probe) -> TestLinkModule;                 // descriptor = test_link_descriptor()
    pub fn with_descriptor(mut self, d: LinkDescriptor) -> TestLinkModule;
}
impl Link for TestLinkModule {
    fn descriptor(&self) -> &LinkDescriptor;
    fn create(&self, decl: &DataLinkDecl) -> Result<Arc<dyn DataLink>, ModuleError>;
        // records "link:create:<from.component>.<from.port>-><to.component>.<to.port>"
        // returns Arc::new(MemLink::new(decl.policy, decl.capacity))
}
```

### 3.7 Fixture functions

```rust
/// The Phase 1 test registry of `tests/spec_binding.rs::registry()`, moved here so
/// both files share it: the `test` Vocabulary, the test Provider (roles Provider and
/// Authority), the test Sink, the test Executor, the test Link Module and its descriptor.
pub fn run_registry() -> ModuleRegistry;
/// `EventKindRegistry::with_kernel_kinds()` plus the `test` Vocabulary's kinds under owner `test`.
pub fn run_kinds() -> EventKindRegistry;
/// `test.limits` registered, as `Fixture` does when a test needs it.
pub fn run_checks(with_limits: bool) -> AdmissionCheckRegistry;
/// `run_registry()` with a `test` Vocabulary in which `test.grid` and `test.flag` are declared
/// `update_class: Some(UpdateClass::Cold)` (the Phase 1 Vocabulary declares them without a class,
/// so no runtime update of them is admissible, RS-17).
pub fn run_registry_classed() -> ModuleRegistry;
```

Four additions to `tests/support/doubles.rs`:

- `TestLimitsCheck` reads an optional `"gate": "<key>"` member of its section: when present, a fragment's `test.grid` is judged only if that fragment's merged configuration holds `Bool(true)` under the gate key. Without the member it behaves exactly as before (every Phase 1 test keeps passing).
- `TestProvider::with_effective(key: &str, value: Value) -> TestProvider`: `prepare` inserts `(key, value)` into its report's `effective` after computing it (a Session's implicit Spec requires nothing, so this is how a test gives the configuration a key).
- `TestProvider::panicking_in_coerce() -> TestProvider`: `coerce` panics (`panic!("test: panic in coerce")`) (`kc_30_a_panic_in_coerce_fails_validate`).
- `TestProvider::omitting_from_applied(key: &str) -> TestProvider`: `coerce` removes that key from `applied` after computing it and adds no `rejected` entry (`kc_26_a_provider_that_applies_nothing_is_refused`).

Do not change `tests/spec_binding.rs`; it keeps its own `registry()`.

### 3.8 A smoke test

Create `crates/ezsdr-kernel/tests/run_doubles.rs` with one test, `ma_44_the_run_doubles_record_their_calls`, that builds a `SteppedProvider`, calls `prepare` with a hand-made `PrepareContext` (a `SimAuthority`'s `time()`, an `EventCollector` over no pairs, a `support::QueueReceiver`, a `support::TestSubmitter`, an empty environment and no links), then `arm`, `start(Some(t0))`, `step(t0)`, `stop(Orderly)`, `cleanup`, and asserts the probe's lines are exactly `["p:prepare:radio", "p:arm", "p:start:0", "p:step:0", "p:now:0", "p:stop:Orderly", "p:cleanup"]`.

**Done-list**

- [ ] `run_doubles.rs` compiles with every item of §3.1–§3.7.
- [ ] `ma_44_the_run_doubles_record_their_calls` passes.
- [ ] The three commands of §0.5 pass (362 tests).

---

## Steps 4–8 — The coordinator

**Read first, before step 4:** spec 06 §3–§13 in full, twice. Then `crates/ezsdr-kernel/src/run.rs` (`RunStateMachine`, `run_cleanup`, `CleanupOps`, `Lease`), `src/session.rs` (`compile`, `Admitter`, `SessionLog`, `implicit_spec`), `src/plan.rs` (`validate`, `plan`, `collect_prepare`, `CompileInputs`), `src/module_api.rs` (the five role traits, `PrepareContext`, `step_until_quiescent`) and `src/event.rs` (`EventCollector`).

The five steps build one Kernel module. The design below is fixed: follow it, including the private type names, so that later steps and the reviewers can find things. In steps 4–8, `ctx` in pseudocode means `shared.ctx`, the Run's `Context`; a Module's `PrepareContext` is always written out as `PrepareContext`, and `spec`, `profile` mean `shared.ctx.spec`, `shared.ctx.profile`.

### The files

```text
crates/ezsdr-kernel/src/coordinator/
  mod.rs        the public API: constants, Assembly, RunHandleError, RunHandle, start_spec_run, connect
  state.rs      Shared, Context, Routing, the slots, Queue, Inst, Origin, EndRequest, helpers
  pipeline.rs   KC-1…KC-19: entry, validate, plan, inputs, links, events, prepare, arm, T0, schedule, start
  stepping.rs   KC-20…KC-23, KC-29, KC-30 (in rounds), KC-31, KC-32: rounds, the loop, the drain, reactions
  admission.rs  KC-23 rewriting, KC-24…KC-28, KC-35…KC-37: admission, dispatch, the Submitter, Sessions, the Lease
  ending.rs     KC-33, KC-34, KC-38…KC-45: cleanup operations and the Manifest
```

In `src/lib.rs`: add `/// The Run coordinator: one Run from its documents to its sealed Manifest (KC-1…KC-45).` and `pub mod coordinator;` after `pub mod contract;` (alphabetical order), and add three rows to the rule-index table in the crate doc comment:

```text
//! | `KA-n` | `plan/phase2/06-kernel-coordinator.md` §2 — amendments to the five specs above |
//! | `KC-n` | `plan/phase2/06-kernel-coordinator.md` §5–§9 — [`coordinator`] |
//! | `UC-n` | `plan/phase2/06-kernel-coordinator.md` §10 — update classes |
```

In `src/plan.rs`, change the crate-private re-export line to `pub(crate) use validation::{binding_description, check_rid, not_registered, require_role};`.

In `tests/kernel_surface_allow.txt`, append the ten lines of spec 06 §4 under the heading it names.

The coordinator's tests go in a new file `crates/ezsdr-kernel/tests/coordinator.rs`, which starts with `mod support;` and `use support::*;`.

### The private types (`state.rs`)

Write these exactly. Every field is `pub(super)`.

```rust
pub(super) type Slot<T> = Arc<Mutex<Box<T>>>;     // used as Slot<dyn Provider>, Slot<dyn Sink>, Slot<dyn Executor>

/// One instance's inbound Action queue (MA-14, KC-25).
pub(super) struct Queue(pub(super) Mutex<VecDeque<Action>>);
impl ActionReceiver for Queue {
    fn recv(&self) -> Option<Action> { lock(&self.0).pop_front() }
}

/// Which instance: an index into Shared's slot vectors (KC-13).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Inst { Provider(usize), Sink(usize), Executor(usize) }

pub(super) struct ProviderSlot {
    pub(super) names: Vec<Ident>,              // every Spec resource name of its group (KC-4), sorted
    pub(super) id: ResourceId,                 // instance().id, read once at assembly (KA-13: it never changes)
    pub(super) module: ModuleRef,              // instance().module
    pub(super) stepped: bool,                  // instance().driving.stepped
    pub(super) lead: Option<Duration>,         // instance().min_command_lead
    pub(super) object: Slot<dyn Provider>,
    pub(super) queue: Arc<Queue>,
}
pub(super) struct SinkSlot { pub(super) output: Ident, pub(super) object: Slot<dyn Sink>, pub(super) queue: Arc<Queue> }
pub(super) struct ExecutorSlot {
    pub(super) name: Ident,                    // the Island executor name it is supplied under
    pub(super) descriptor: ExecutorDescriptor, // read once at assembly
    pub(super) object: Slot<dyn Executor>,
    pub(super) queue: Arc<Queue>,
}

/// Where an Action or an event comes from (KC-24).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Origin { Schedule, Session, Module }

#[derive(Clone, PartialEq, Debug)]
pub(super) struct EndRequest { pub(super) termination: Termination, pub(super) mode: CleanupMode }

/// What never changes after entry (KC-6).
pub(super) struct Context {
    pub(super) kind: RunKind,
    pub(super) id: RunId,
    pub(super) spec: ExperimentSpec,
    pub(super) spec_section: SpecSection,      // computed at entry (KC-45)
    pub(super) profile: BindingProfile,
    pub(super) binding_section: BindingSection,// computed at entry: { hash: ContentHash::of_value(&doc), body: doc }
    pub(super) environment: Arc<BTreeMap<Namespace, serde_json::Value>>, // profile.environment, one Arc (KC-11)
    pub(super) registry: ModuleRegistry,
    pub(super) checks: AdmissionCheckRegistry,
    pub(super) kinds: EventKindRegistry,
    pub(super) contracts: ContractRegistry,
    pub(super) clocks: Arc<ClockRegistry>,
    pub(super) host_clock: Arc<dyn HostClock>,
    pub(super) declared_classes: BTreeMap<Key, UpdateClass>, // every spec.graph.components[*].params[*] (key → update_class)
    pub(super) outputs: BTreeSet<Ident>,       // every spec.outputs[*].id
}

/// Set once, right after `plan()` succeeds (KC-13, KC-23).
pub(super) struct Routing {
    pub(super) plan: ExecutionPlan,
    pub(super) matched: BTreeMap<Ident, ResourceId>,  // admission.matched
    pub(super) fragment_of: BTreeMap<Ident, Inst>,    // fragment id → instance (no entry for an Authority fragment)
    pub(super) first_fragment: BTreeMap<Inst, Ident>, // instance → its first fragment id
    pub(super) order: Vec<Inst>,                      // instances in first-fragment order (KC-13)
    pub(super) island_of: BTreeMap<Ident, Ident>,     // graph component name → its Island fragment id
    pub(super) reverse: Vec<Ident>,                   // every fragment id, reversed plan order (RS-8)
}

/// Everything a cleanup step can touch (spec 06 §3).
pub(super) struct Shared {
    pub(super) ctx: Context,
    pub(super) providers: Vec<ProviderSlot>,
    pub(super) sinks: Vec<SinkSlot>,
    pub(super) executors: Vec<ExecutorSlot>,
    pub(super) authority: Arc<dyn Authority>,
    pub(super) time: Arc<dyn TimeAuthority>,          // authority.time(), called once (KC-3)
    pub(super) primary: ClockDomainId,                // time.primary_root()
    pub(super) routing: OnceLock<Routing>,
    pub(super) machine: Mutex<RunStateMachine>,
    pub(super) collector: OnceLock<Arc<EventCollector>>,
    pub(super) policy: OnceLock<Policy>,
    pub(super) configuration: Mutex<BTreeMap<Ident, BTreeMap<Key, Value>>>, // KC-27
    pub(super) next_action: AtomicU64,                // starts at 1 (KC-24)
    pub(super) frozen: AtomicBool,                    // RS-6 step 1
    pub(super) delivered: Mutex<Vec<Event>>,
    pub(super) marks: Mutex<Vec<(EventKind, TimePoint)>>,
    pub(super) end: Mutex<Option<EndRequest>>,
    pub(super) also: Mutex<Vec<StopCause>>,
    pub(super) failure: Mutex<Option<(Stage, String)>>,   // for the ezsdr.failure section (KC-7)
    pub(super) artifacts: Mutex<Vec<ArtifactRef>>,
    pub(super) links: Mutex<Vec<(DataLinkDecl, Arc<dyn DataLink>)>>,
    pub(super) link_drops: Mutex<Vec<u64>>,           // read at cleanup step 7, same order as `links`
    pub(super) counters: Mutex<Option<Vec<CounterRow>>>, // snapshot at cleanup step 7
    pub(super) prepared: Mutex<BTreeSet<Ident>>,      // fragment ids whose prepare was called (MA-7)
    pub(super) done: Mutex<BTreeSet<(u8, Inst)>>,     // (CleanupStep as u8, instance) already acted on (KA-9)
    pub(super) drained: AtomicBool,                   // cleanup step 3's drain ran (KA-12)
    pub(super) closing: AtomicBool,                   // set on entry to every cleanup operation after the drain's own call, and when run_cleanup returns (KA-12)
    pub(super) scheduled: Mutex<Vec<ScheduleHandle>>, // the coordinator's own no-op callbacks (KC-20)
    pub(super) cleanup_failures: Mutex<Vec<CleanupFailure>>, // failures recorded outside run_cleanup
}
```

Helpers in `state.rs`:

```rust
/// Poison-tolerant lock (§0.6 rule 7).
pub(super) fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(|e| e.into_inner()) }

/// A Module call under catch_unwind (KC-30).
pub(super) fn contain<T>(f: impl FnOnce() -> Result<T, ModuleError>) -> Result<T, ModuleError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => Err(ModuleError { kind: ModuleErrorKind::Internal, message: "a Module panicked".to_owned(), detail: serde_json::Value::Null }),
    }
}

/// `try_lock` for cleanup and the Manifest (KC-44): a poisoned mutex (a Module panicked while it
/// was held, KC-30) counts as acquired; only a mutex another thread still holds is `None`.
pub(super) fn try_slot<T: ?Sized>(m: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match m.try_lock() { Ok(g) => Some(g), Err(TryLockError::Poisoned(p)) => Some(p.into_inner()), Err(TryLockError::WouldBlock) => None }
}

/// `d` rescaled into `to`, rounded up to a whole tick (KC-15, KC-24, RS-19).
pub(super) fn ceil_rescale(clocks: &ClockRegistry, d: Duration, to: ClockDomainId) -> Result<i64, TimeError> {
    Ok(match clocks.rescale(d, to)? { Rescaled::Exact { duration } => duration.ticks,
                                      Rescaled::Inexact { floor, .. } => floor.ticks.checked_add(1).ok_or(TimeError::Overflow)? })
}

/// `t` converted into `to`, rounded up (KC-29).
pub(super) fn ceil_convert(clocks: &ClockRegistry, t: TimePoint, to: ClockDomainId) -> Result<TimePoint, TimeError> {
    if t.domain == to { return Ok(t); }
    Ok(match clocks.convert(t, to)? { Converted::Exact { point } => point,
                                      Converted::Inexact { floor, .. } => TimePoint::new(to, floor.ticks.checked_add(1).ok_or(TimeError::Overflow)?) })
}

pub(super) fn kernel_source() -> ResourceId { ResourceId::parse(KERNEL_SOURCE).expect("a valid literal") }

impl Shared {
    pub(super) fn now(&self) -> TimePoint { self.time.now(self.primary).expect("the Authority governs its primary root") }
    pub(super) fn routing(&self) -> Option<&Routing> { self.routing.get() }
    pub(super) fn queue(&self, i: Inst) -> &Arc<Queue>;            // the slot's queue
    pub(super) fn source_root(&self, i: Inst) -> ResourceId;       // KC-30: provider id, sink/<output>, island_<n>
    pub(super) fn first_fragment(&self, i: Inst) -> Ident;         // from routing; for Executor(i) with no routing, its name
    pub(super) fn move_to(&self, s: RunState);                     // lock(machine).move_to(s, Some(self.now()), &*ctx.host_clock); an Err is a coordinator bug: debug_assert!
}
```

`RunHandle` (in `mod.rs`) holds only control-path state:

```rust
pub struct RunHandle {
    shared: Arc<Shared>,
    lease: Lease,
    log: SessionLog,
    inputs: Vec<ArtifactRef>,                     // Manifest.inputs, first-seen order
    store: BTreeMap<ContentHash, Vec<u8>>,        // the input store (KC-9, KC-28)
    agenda: Vec<(i64, usize, Action)>,            // (primary ticks, insertion, Action) of untimed schedule entries (KC-17)
    t0: Option<TimePoint>,
    admission: AdmissionResult,
    merged: Option<MergedPrepare>,
    links_by_ref: BTreeMap<ModuleRef, Box<dyn Link>>,   // the Assembly's Link objects, used by KC-10 and then dropped
    entry_failure: Option<String>,                // a KC-4 refusal found while assembling, reported as Failed { validate }
    last_wakeup: Option<TimePoint>, same_count: usize,   // KC-22
    manifest: Option<Manifest>,
}
```

### Step 4 — Coordinator A: types, entry, validate, plan, the end path and the Manifest

**Implements:** KC-1…KC-7 (up to `Planned`), KC-33 (the stage-failure and `finish` causes), KC-34, KC-38, KC-39 (steps 0, 1, 5, 8 only), KC-43, KC-44, KC-45 (the fields that exist before `prepare`).

1. Write `mod.rs` with the public API exactly as spec 06 §4 declares it (with `state()` returning `RunState` by value), plus `#[derive(Clone, PartialEq, Debug)]` on `RunHandleError` and `impl std::fmt::Display` for it (one line per variant). Every public item gets a doc comment citing its KC rule.
2. Write `state.rs` as above.
3. Entry (`pipeline.rs`):

   ```text
   start_spec_run(spec_doc, profile_doc, a):
       spec    = ExperimentSpec::from_json(spec_doc)?               -- Err: no Run (KC-1)
       profile = BindingProfile::from_json(profile_doc)?
       spec_section    = SpecSection::migrated(spec_doc.clone(), spec_doc)   map HashError → SpecError::Structural("KC-1: …")?
       binding_section = BindingSection { hash: ContentHash::of_value(profile_doc)?, body: profile_doc.clone() }
       run = assemble(RunKind::Spec, spec, spec_section, profile, binding_section, a, Lease::attached())   (KC-35)
       run.pipeline(); run.settle(); Ok(run)

   connect(profile_doc, a, lease):
       profile = BindingProfile::from_json(profile_doc)?
       lease.validate() → Err(e): return Err(SpecError::Structural("KC-1: RS-21: {e}"))
       refs = a.providers as BTreeMap<Ident, &dyn Provider>, sinks likewise
       spec = session::implicit_spec(&profile, &a.registry, &refs, &sink_refs)?
       doc = serde_json::to_value(&spec) → SpecSection::migrated(doc.clone(), &doc)
       run = assemble(RunKind::Session, spec, …, lease); run.pipeline(); run.settle(); Ok(run)
   ```

4. `assemble` (never fails; it records a refusal in `entry_failure`):
   - **KC-4 grouping.** For every Spec resource name the profile binds, compute `plan::binding_description(binding)`; group the names by description (a `BTreeMap<(ModuleRef, String), Vec<Ident>>`). For each group, the supplied objects are `a.providers` entries whose key is one of the group's names. Exactly one: remove it from `a.providers`, read its `instance()` once for `id`, `module`, `stepped`, `lead`, and push a `ProviderSlot` with every name of the group (sorted). Zero: push nothing (validate refuses the resource, SB-22f). Two or more: `entry_failure = "KC-4: resources <names> share one binding description and were handed <n> Provider objects"`.
   - After grouping, any key left in `a.providers` is `entry_failure = "KC-4: a Provider object under <name>, which is no bound Spec resource"`.
   - Sinks: one `SinkSlot` per `a.sinks` entry; a key that is not a `spec.outputs` id is `entry_failure = "KC-4: a Sink object under <name>, which is no output"`.
   - Executors: one `ExecutorSlot` per `a.executors` entry, `descriptor` cloned; a key that is no `placements.islands[*].executor` is `entry_failure = "KC-4: an Executor object under <name>, which no Island names"`.
   - `authority: Arc::from(a.authority)`; `time = authority.time()`; `primary = time.primary_root()`.
   - `machine = RunStateMachine::new(&*a.host_clock)` (state `Created`).
   - `store = a.inputs`; `declared_classes`, `outputs` as `Context` says; `id = RunId::generate()` (KC-5).
   - Only the first `entry_failure` is kept.
5. `with_inputs`: a `RunHandle` method that locks every Provider and Sink slot, builds `CompileInputs { registry, checks, kinds, providers: every name of every slot → &**guard, authorities: { profile.authority: authority.descriptor().clone() }, executors: { slot.name: slot.descriptor.clone() }, contracts, sinks: { slot.output: &**guard }, is_session: kind == Session }`, and calls a closure with it. This is the prototype's pattern; the guards live for the closure's duration.
6. `pipeline`, first part (the rest is steps 5 and 6):

   ```text
   if entry_failure: return fail(Validate, it)
   admission = contain_all(|| with_inputs(|i| plan::validate(spec, profile, i)));   a panic → fail(Validate, "KC-30: a Module panicked during validate"); Err(e) → fail(Validate, e.to_string())
   if !admission.is_admitted(): fail(Validate, "KC-7: not admitted: rejected <rejected:?>, violations <violations:?>")
   self.admission = admission
   policy = ctx.kinds.compile(&spec.policies.failure); Err(e) → fail(Validate, "KC-8: {e}"); shared.policy.set(policy)
   move_to(Validated)
   plan = contain_all(|| with_inputs(|i| plan::plan(spec, profile, &self.admission, i, Vec::new())));   a panic → fail(Plan, "KC-30: … during plan"); Err(e) → fail(Plan, e.to_string())

   if plan.class != ExecutionClass::Simulation: fail(Plan, "KC-2: Phase 2 runs the Simulation class only; this plan's class is <class:?>")
   routing = build_routing(plan, admission.matched)     (below);  shared.routing.set(routing)
   move_to(Planned)
   -- step 5 continues here
   ```

   `contain_all(f)` is `std::panic::catch_unwind(AssertUnwindSafe(f))`: `Ok(result)`, or `Err(_)` for a panic (KC-30). Every call of `plan::validate`, `plan::plan` and `plan::collect_prepare` goes through it (a panic → `fail(<that stage>, "KC-30: a Module panicked during <validate | plan | prepare>")`), and so does each `instance()` read in `assemble` (a panic → `entry_failure = "KC-30: a Module panicked during validate: instance()"`, which becomes `Failed { validate }`). A panic while `with_inputs` holds the slot guards poisons those mutexes; `lock` already tolerates that (§0.6 rule 7), and `try_slot` (below) does too.

   `fail(stage, reason)`: `*failure = Some((stage, reason))` if none yet; `request(Failed { stage }, Abort)` (KC-32, step 8 writes `request`; in this step write the simple form: set `end` if it is `None`); return. `settle()` then runs cleanup.

   `build_routing`: walk `plan.fragments` in order. Role `Provider`: the slot whose `names` contains `f.id`. Role `Sink`: the slot whose `output == f.id`. Role `Executor`: parse `f.content` as `IslandDecl` and take the slot whose `name == island.executor`; for each `island.components` entry `c`, `island_of[c] = f.id`. Role `Authority`: no instance. For an instance seen for the first time push it to `order` and set `first_fragment`. `reverse` is every fragment id in reverse order, including the Authority's.

7. The end path (`ending.rs`):

   ```text
   settle(&mut self):  if an end is requested and the state is not CleanedUp: self.cleanup()

   cleanup(&mut self):
       EndRequest { termination, mode } = the pending request
       lock(machine).begin_stopping(mode, Some(now), host_clock)
       order = routing.reverse or [] when there is no routing
       outcome = run_cleanup(Arc::new(Ops { shared: shared.clone() }), &order, mode,
                             &|| (current request's mode == Abort && mode == Orderly).then_some(CleanupMode::Abort))
       shared.closing = true                                          (KA-12 step 3: an abandoned drain stops)
       lease.released = true                                          (KA-12 step 8)
       lock(machine).finish(termination, Some(now), host_clock)
       self.manifest = Some(self.assemble_manifest(termination, outcome.failures))
       lock(&shared.links).clear()                                    (KA-12 step 8: links dropped after the Manifest)

   struct Ops { shared: Arc<Shared> }
   impl CleanupOps for Ops {
       fn perform(&self, step, fragment, mode) -> Result<(), ModuleError>:
           match step {
             Children => Ok(()),
             FreezeDispatch => { frozen = true; empty every Queue; for h in scheduled.drain(..) { time.cancel(h); } Ok(()) }
             StopTx => step 5 of this plan (Providers' stop)
             StopRx => step 6 of this plan (the drain, then Executors' and Sinks' stop)
             CancelPeripherals => Ok(()),
             RestoreBaseline => { let f = fragment; if !prepared.contains(f) { return Ok(()) }
                                  let Some(inst) = fragment_of(f) else { return Ok(()) };
                                  if !done.insert((5, inst)) { return Ok(()) }
                                  match try_slot(slot(inst)) { Some(mut g) => { contain(|| { g.cleanup(); Ok(()) }) }
                                     None => Err(ModuleError::rejected("KC-39: the instance is still held by an abandoned cleanup step")) } }
             FinaliseArtifacts => step 8 of this plan (KC-42)
             FlushEvents => { drain_and_react(shared); counters = collector.counters(); link_drops = links.map(drops()) }   (in this step: counters only if a collector exists)
             ReleaseAndWriteManifest => Ok(()),
           }
   }
   ```

   `CleanupStep as u8` is the key of `done`: `Children = 0 … ReleaseAndWriteManifest = 8` in `CLEANUP_STEPS` order.

8. `assemble_manifest` (`ending.rs`), KC-44 and KC-45. Build the `Manifest` literal with every field of KC-45, using `Default` values for what does not exist yet (no plan: `plan: None`, `prepare: PrepareSection::default()`, `execution_class: Simulation`, `deterministic: false`). Then:
   - `sections`: insert `ezsdr.links` (an array; empty until step 5 creates links) and, when the termination is `Failed { stage }`, `ezsdr.failure` as `json!({ "stage": <stage as lowercase string>, "reason": <the reason recorded for that stage> })` (KC-32);
   - for each Provider slot: `try_slot`; on `Some` push its `instance().fidelity` for `Fidelity::weakest` and write each `instance().sections` entry with `m.write_section(&owner, key, value)`, where `owner = Namespace::parse(slot.module.id.as_str())` — if that parse fails (a Module id that is not a Namespace, KA-13), write none of its sections and record one step-8 failure; a refusal becomes `CleanupFailure { step: ReleaseAndWriteManifest, fragment: Some(first fragment), reason: "KC-44: <error>", timed_out: false }`; on `try_lock` failure push KC-44's "still held" failure;
   - `termination.cleanup_failures` = `run_cleanup`'s failures, then `shared.cleanup_failures`, then this assembly's;
   - `m.seal()`; on `Err(e)` push `CleanupFailure { step: ReleaseAndWriteManifest, fragment: None, reason: format!("KC-44: the Manifest could not be sealed: {e}"), timed_out: false }` into `m.termination.cleanup_failures` and return `m` with `hash: None`.
9. `finish(self)` (KC-34): if not `CleanedUp`, `request(Stopped { cause: Client {} }, Orderly)` and `cleanup()`; return the Manifest. `state()`, `kind()`, `now()` are one-liners.

**Tests (step 4)**, in `tests/coordinator.rs`. Write a fixture first:

```rust
/// One Run's worth of inputs (spec 06 §12). The Authority is a SimAuthority over a 1 GHz root.
struct Rig { probe: Probe, clocks: Arc<ClockRegistry>, root: ClockDomainId, manual: Arc<ManualTimeAuthority>,
             host: Arc<FakeHostClock>, assembly: Assembly }
fn rig() -> Rig;            // run_registry(), run_checks(false), run_kinds(), ContractRegistry::with_standard_contracts(),
                            // SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning), no Modules, no inputs
fn spec_one() -> serde_json::Value;     // the smoke test's Spec: resource `radio` of kind test.device requiring test.count = 2
fn profile_one() -> serde_json::Value;  // binds `radio` to ezsdr.test.provider 1.0.0, authority `radio`, empty environment
```

| test | fixture | assertion |
|---|---|---|
| `kc_01_a_parse_failure_is_no_run` | `spec_one()` with `"version": 2` | `start_spec_run` returns `Err(SpecError::UnsupportedVersion { found: 2, .. })` |
| `kc_01_a_validate_failure_still_writes_a_manifest` | `spec_one()` plus a second resource `other` the profile does not bind; one `TestProvider::new("radio", 2)` under `radio` | `state()` is `CleanedUp { Failed { validate } }`; `finish()` returns a Manifest with `hash.is_some()`, `plan: None`, `run.transitions` states `[Created, Stopping { abort }, CleanedUp { .. }]`, `sections["ezsdr.failure"]["stage"] == "validate"` |
| `kc_01_a_validate_failure_records_its_reason` | the same | `sections["ezsdr.failure"]["reason"]` is non-empty; the first transition (`Created`) has `at: None` and every later one `at.is_some()` (KC-43); every transition has `host_utc_nanos > 0` (the `FakeHostClock` starts at 1.7e18) |
| `kc_02_a_wall_paced_authority_is_refused` | the Authority built with `Pacing::WallPaced` | `CleanedUp { Failed { plan } }`; `sections["ezsdr.failure"]["reason"]` starts with `"KC-2: "` |
| `kc_04_two_objects_for_one_description_are_refused` | two resources `a`, `b` of kind `test.line` requiring `test.count = 2`, both bound to the test Provider with empty selectors (one description), a `TestProvider::new("dev", 2)` under each name | `Failed { validate }`, reason starts with `"KC-4: "` |
| `kc_04_one_object_serves_both_names` | the same, one object under `a` | `finish()` gives `Stopped { client }`, not `Failed { validate }`: both names were served by the one object |
| `kc_04_an_object_under_no_resource_is_refused` | `spec_one()`, objects under `radio` and `nobody` | `Failed { validate }`, reason names `nobody` |
| `kc_45_the_documents_are_recorded_verbatim` | `spec_one()` / `profile_one()` passed as JSON values | `manifest.spec.body == spec_doc`, `manifest.binding.body == profile_doc`, `manifest.binding.hash == ContentHash::of_value(&profile_doc)` |

In step 4 a Run that validates reaches `Planned` and then ends through `settle()` only if something requested an end; to let the tests finish, `finish()` ends it (`Stopped { client }`).

**Mutation checks (step 4):** KC-2 (`kc_02_…`), KC-4's two-objects refusal (`kc_04_two_objects_…`), KC-4's unknown-name refusal (`kc_04_an_object_under_no_resource_…`).

**Done-list:** the files exist; `lib.rs`, `plan.rs` and the allow-list are edited; the eight tests pass; `kernel_surface` passes (every new public item is allow-listed and cites a rule, and no banned token appears); the three commands pass.

### Step 5 — Coordinator B: inputs, links, events, prepare, arm, T0, start

**Implements:** KC-8…KC-15, KC-18 (without the schedule), KA-9, KA-15, MA-5a; cleanup steps 2 (StopTx) and 3 (StopRx, without the drain).

Continue `pipeline` after `move_to(Planned)`:

1. **KC-9.** For each schedule entry whose template is a `TxBurst`, check its `waveform` against `store` as KC-9 says; on a refusal `fail(Plan, "KC-9: entry <i>: …")`; push the ref into `inputs` when its hash is not there yet.
2. **KC-10.** For each `decl` in `plan.links`, in order:
   - find the `LinkPlacement` in `profile.placements.links` with `from == decl.from && to == decl.to`; none → `fail(Plan, "KC-10: no LinkPlacement for <from> -> <to>")`;
   - take `a.links[placement.link]` (the Assembly's Link objects are kept in the `RunHandle` until this point; store them in a `links_by_ref` field and drop them after this step); none → `fail(Plan, "KC-10: no Link object for <ref>")`;
   - `Some(link.descriptor()) != ctx.registry.link_descriptor(&placement.link)` → `fail(Plan, "KC-10: MA-27a: <ref>'s descriptor differs from the registered one")`;
   - `contain(|| link.create(decl))` → `Err(e)` → `fail(Plan, "KC-10: <e.message>")`;
   - the owner of an end `p` is: `p.component` a Spec resource name → the fragment of that name; a graph component → `island_of[p.component]`; an output id → the fragment of that name. Push `AttachedPort { component: from.component, port: from.port, endpoint: StreamOut(link) }` to the owner of `decl.from` and `AttachedPort { component: to.component, port: to.port, endpoint: StreamIn(link) }` to the owner of `decl.to`, in a `BTreeMap<Ident, Vec<AttachedPort>>`;
   - push `(decl, link)` into `shared.links`.
3. **KC-8.** Build the pairs as KC-8 lists them, dedupe them with a `BTreeSet<(ResourceId, EventKind)>`, collect the set into a `Vec<(ResourceId, EventKind)>` `pairs` (in set order), and create `EventCollector::new(&pairs, &ctx.kinds.kinds(), EVENT_RING_DEPTH, policy)`; `shared.collector.set(Arc::new(it))`. "Every kind registered by a Vocabulary its ModuleDescriptor declares": `ctx.registry.modules()` → the descriptor whose `(id, version)` equals the slot's `module`, then for each `descriptor.vocabularies[*].id`, `ctx.registry.vocabulary(id)` → its `event_kinds[*].kind`. Walk a Provider's tree with `instance().tree.walk()`.
4. **KC-11, KC-12.** For each fragment in `plan.fragments`, in order:
   - Authority fragment: skip;
   - build the `PrepareContext` of KC-11 (below, `shared.ctx` is the `Context`, never the `PrepareContext` being built): `run: shared.ctx.id.clone()`, `class: plan.class`, `time: shared.time.clone()`, `clocks: shared.ctx.clocks.clone()`, `events: collector.clone()` (as `Arc<dyn EventSink>`), `actions: the instance's queue` (as `Arc<dyn ActionReceiver>`), `actions_out: Arc::new(Submitter { shared: Arc::downgrade(&shared) })` (step 7 writes `Submitter`; in this step write it as a struct whose `submit` returns `Err(vec![Violation { check: ezsdr.submit, key: None, requested: None, reason: "KC-24: not yet" }])` — step 7 replaces the body), `environment: shared.ctx.environment.clone()`, `links: the attached ports of this fragment` (removed from the map), `components` (an Island: `spec.graph.components` restricted to `island.components`; otherwise empty), `host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, DEFAULT_HOST_BUDGET_NS)).expect("positive")`;
   - insert `f.id` into `prepared` **before** the call;
   - call `Provider::prepare(f, ctx)`, `Sink::prepare(f, ctx)` or `Executor::prepare(&island, ctx)` under `contain`, holding that instance's slot lock;
   - push the result; stop after the first `Err`.

   Then `merged = contain_all(|| with_inputs(|i| plan::collect_prepare(reports, spec, profile, i, &self.admission)))` (a panic → `fail(Prepare, "KC-30: a Module panicked during prepare")`). `Err(PrepareError::Fragment { index, error })` → `fail(Prepare, "KC-12: fragment <id>: <error.message>")`; `Err(PrepareError::Violations(v))` → `fail(Prepare, "KC-12: <v:?>")`. On success: `configuration = { r.fragment: r.effective }` for every report (KC-27); `self.merged = Some(merged)`; `move_to(Prepared)`. A `DeviceLost` from `prepare` is not a KC-30 case (KC-30 starts after `prepare`).
5. **KC-14.** For each instance in `routing.order`, stopping at the first failure: lock its slot and `contain(|| arm())`; `Err(e)` → if `e.kind == DeviceLost`, emit `DEVICE_LOST` (KC-30's form); `fail(Arm, "KC-30: <first fragment>: <message>")`. Then `move_to(Armed)`.
6. **KC-15.** `lead = binding::start_lead_ns(&profile.environment)` (an `Err` → `fail(Arm, …)`; `plan()` already refused a bad one, so this does not happen); `lead_ticks = ceil_rescale(&clocks, Duration::new(HOST_MONOTONIC, lead as i64), primary)`; `t0 = now + lead_ticks` (`checked_add`, an overflow → `fail(Arm, "KC-15: overflow")`).
7. *(Step 6 inserts KC-16, KC-17 and KC-19 here.)*
8. **KC-18.** For each instance in `routing.order`: `Provider::start(Some(t0))`, `Executor::start()`, `Sink::start()` under `contain`; `Err` → as in 5, `fail(Arm, …)`. Then `move_to(Running)`, then `round(now)` (step 6 writes `round`; in this step `round` only calls `step_until_quiescent` over every stepped instance and ignores its result).
9. **Cleanup steps 2 and 3** in `Ops::perform`:
   - `StopTx`: `inst = fragment_of(fragment)`; only `Inst::Provider`; only an instance one of whose fragments is in `prepared` (KC-39); only once per instance (`done.insert((2, inst))`); lock the slot (blocking: this is the step whose deadline protects the Run) and `contain(|| stop(m))`, where `m` is the **current** end request's mode read now (KC-39) — `StopMode::Orderly` for `CleanupMode::Orderly`, `StopMode::Abort` for `Abort`; an `Err(e)` is returned (`run_cleanup` records it), after emitting `DEVICE_LOST` when `e.kind == DeviceLost`.
   - `StopRx`: `inst = fragment_of(fragment)`; only `Inst::Executor` and `Inst::Sink`; only a prepared instance; once per instance (`done.insert((3, inst))`); the `StopMode` is the **current** end request's mode read now (KC-39), not the `mode` argument; take the slot with `try_slot` (`None` → return KC-39's "still held" error); `Executor::stop(mode)`; for a Sink, `Sink::stop(mode)` and, on `Ok(refs)`, append `refs` to `artifacts` (KC-41: an `Err` contributes none).
   - `FlushEvents` also reads `link_drops` now.
10. In `assemble_manifest`, fill `plan`, `prepare = { reports: merged.reports, merged_effective: merged.effective }`, `modules`, `vocabularies`, `components` and `ezsdr.links` as KC-45 says. `modules`: for each binding in `profile.bindings` (key order) then each `placements.links[*].link`, a `ModuleEntry { module, impl_hash: registry.modules().find(|d| d.id == m.id && d.version == m.version).and_then(|d| d.impl_hash.clone()), profile: binding.profile (None for a Link) }`, skipping an entry equal to one already pushed. `vocabularies`: for each distinct module of `modules`, each `descriptor.vocabularies[*].id` → `registry.vocabulary(id).map(|v| v.version)`, inserted into the `BTreeMap`. `components`: `spec.graph.components` → `(name, descriptor.implementation.hash)`.

**Tests (step 5)**

| test | fixture | assertion |
|---|---|---|
| `ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once` | two resources `a`, `b` of kind `test.line` requiring `test.count = 2`, bound with one description (empty selectors) to one `SteppedProvider` (probe name `p`) over `TestProvider::new("dev", 2)`, supplied under `a`; the matcher binds `a` to `dev/0` and `b` to `dev/1`; `finish()` right after entry | `probe.with_prefix("p:")` is `["p:prepare:a", "p:prepare:b", "p:arm", "p:start:0", "p:step:0", "p:now:0", "p:stop:Orderly", "p:cleanup"]` |
| `kc_10_link_descriptor_must_equal_the_registered_one` | an output `rec` fed from `radio.rx` with `drop_oldest`, capacity 4; a `RecordingSink`; a `TestLinkModule` whose descriptor's `kind` is `test.other` | `Failed { plan }`; reason starts with `"KC-10: MA-27a"` |
| `kc_10_both_ends_are_attached` | the same with the right descriptor; the Provider is a `SteppedProvider` `p` with `publishing_every(100)`; `advance_to(TimePoint(root, 250))` | probe has `"p:links:radio.rx:out"` and `"rec:links:rec.in:in"`; `rec:block:100` and `rec:block:200` appear |
| `kc_11_an_island_gets_exactly_its_components` | two Islands `island_0` (components `c1`, `c2`) and `island_1` (`c3`) served by one `ProbeExecutor` `x` under the executor name `exec`, each component `recorder_component`-like with no links | probe has `"x:prepare:island_0:c1,c2"` and `"x:prepare:island_1:c3"` |
| `kc_12_a_prepare_failure_stops_the_loop_and_cleans_up_what_was_prepared` | three resources `a`, `b`, `c` on three distinct instances (distinct selectors), `b`'s inner `TestProvider` `.failing_at(FailAt::Prepare)` | `Failed { prepare }`; `c:prepare:*`, `c:stop:*` and `c:cleanup` absent; `a:stop:Abort`, `b:stop:Abort`, `a:cleanup` and `b:cleanup` present |
| `kc_13_arm_and_start_follow_instance_order_cleanup_reverses_it` | three instances `a`, `b`, `c`; `b` `arm_after` `a`'s id | the `*:arm` lines are `a`, `b`, `c`; the `*:cleanup` lines are `c`, `b`, `a` |
| `kc_15_t0_is_arm_end_plus_the_lead` | `"ezsdr.time": { "class": "simulation", "start_lead_ns": 2000000 }` (without `class` MA-41 refuses the section) | `start_instant() == Some(TimePoint(root, 2_000_000))` and `p:start:2000000` is in the probe |
| `kc_09_an_input_must_be_supplied_and_match_its_hash` | a schedule entry `TxBurst` (target `radio/tx`, a waveform ref of 80 bytes, `send_asap_and_flag`) at offset 0 on a declared clock; four failing Runs — `inputs` empty; the 80 bytes with one changed; the right bytes but the ref's `size_bytes` 81; the ref's uri `http://x` — then one with the right bytes and a `mem:` uri | `Failed { plan }` four times, each with a reason beginning `"KC-9: "`; the fifth Run passes `plan` (its termination is not `Failed { plan }`) and `manifest.inputs` holds the ref |
| `kc_14_an_arm_failure_is_failed_arm` | `SteppedProvider` over `TestProvider::new("radio", 2).failing_at(FailAt::Arm)` | `Failed { arm }`; `p:stop:Abort` and `p:cleanup` are in the probe |
| `kc_18_a_start_failure_is_failed_arm` | the same with `FailAt::Start` | `Failed { arm }`; no transition is `Running` |
| `ma_05a_a_module_keeps_its_handles_after_prepare` | a `SteppedProvider` `.emitting("test.custom", Info, 50).with_wakeups(&[50])`; `advance_to(TimePoint(root, 60))` | the Manifest's `events.delivered` contains a `test.custom` event whose `time.ticks == 50`; the probe has `p:step:50`; and `p:now:50` — give `SteppedProvider::step` one more probe line, `"<name>:now:<time.now(primary).ticks>"`, recorded right after `"<name>:step:…"`, so the test can assert that `now` read in `step` equals `until` (MA-5a) |

**Mutation checks (step 5):** KC-9's four refusals (missing bytes, hash, size, uri scheme); KC-10's descriptor refusal; KC-12's stop at the first failure (remove the `break`: `c:prepare` then appears and the test fails); KC-13's order (iterate `plan.fragments` instead of `routing.order` for `arm` — still the same here, so instead reverse `routing.order` and confirm `kc_13` fails).

**Done-list:** the eleven tests pass; the step-4 tests still pass; the three commands pass.

### Step 6 — Coordinator C: the schedule, the stepping loop, the drain

**Implements:** KC-16, KC-17, KC-19, KC-20, KC-21, KC-22, KC-23 (the stepped set and rewriting), KC-29, KA-11's use, KA-12's drain, KC-30's round half.

1. **Rewriting (KC-23)** in `admission.rs`:

   ```rust
   /// A Spec-relative target, rewritten (KC-23): the rewritten id, the instance, the fragment.
   pub(super) fn rewrite(shared: &Shared, target: &ResourceId) -> Result<(ResourceId, Inst, Ident), Violation>
   ```

   `first` = the first segment. `"sink"`: the second segment must be an output id `o`; the target stays as written; the instance is `fragment_of[o]`. A Spec resource name `R`: rewritten = `matched[R].path` + `"/"` + the remaining segments (none: just `matched[R].path`), parsed with `ResourceId::parse`; instance `fragment_of[R]`, fragment `R`. A graph component `C`: stays as written; fragment `island_of[C]`. Anything else, or a parse failure: `Violation { check: ezsdr.target, key: None, requested: None, reason: "KC-23: <target> names no resource, output or component" }`.
2. **KC-16, KC-17, KC-19** at pipeline point 7, in two passes:

   ```text
   -- pass 1: resolve every entry's instant (KC-16)
   resolved = []                                        -- (instant, entry index, entry)
   for (i, entry) in spec.schedule.enumerate():
       rewritten = entry.action.target().map(|t| rewrite(shared, t))   -- Err(v) → fail(Arm, "KC-16: entry i: " + v.reason)
       R = entry.at.clock; R must be a key of spec.resources, else fail(Arm, "KC-16: entry i: clock R is no Spec resource")
       node = matched[R]
       streams = ctx.clocks.declared_sample_clocks().filter(|h| h.stream.is_within(&node))
       chosen = the h whose h.stream == rewritten id, if any; else, if every h shares one root_ticks_per_tick, that one;
                none at all → fail(Arm, "KC-16: entry i: R has no stream clock"); otherwise → fail(Arm, "KC-16: entry i: ambiguous: R's stream clocks differ in rate")
       chosen.root != primary → fail(Arm, "KC-16: entry i: the stream clock is not on the primary root")
       entry.at.offset_ticks < 0 → fail(Arm, "KC-16: entry i: negative offset")
       instant = t0.ticks + ceil(offset · num / den)   -- in i128; out of i64 → fail(Arm, "KC-16: entry i: overflow")
       resolved.push((instant, i, entry))
   sort resolved by (instant, i)

   -- pass 2: admit the timed entries cumulatively (KC-17, KC-19); hold the untimed ones
   working = lock(configuration).clone()               -- a local copy; `configuration` changes only at dispatch (KC-27)
   batch = []                                           -- (entry index, Admitted), in (instant, index) order
   for (instant, i, entry) in resolved:
       if entry.action.is_timed():
           action = entry.action.clone().resolve(AbsoluteDeadline::new(TimePoint::new(primary, instant)))
           admitted = admit_with(shared, action, Origin::Schedule, &[], &working)   -- Err(v) → fail(Arm, "KC-17: entry i: <v:?>")
           if admitted.action is UpdateParameter { key, value, .. }:
               working[admitted.fragment][key] = value              -- KC-17's working copy: the next entry is judged with it
           KC-19: if admitted.action is TxBurst with late_policy RejectAtPlan:
                  lead = the target instance's `lead` or Duration(host.monotonic, 0)
                  if ctx.clocks.compare_durations(Duration::new(primary, instant - now.ticks), lead)? == Less:
                      fail(Arm, "KC-19: SC-27: entry i's burst is <instant - now> ticks ahead, shorter than its target's min_command_lead")
           batch.push((i, admitted))
       else:                                                -- an untimed template: Stop
           agenda.push((instant, i, entry.action.clone().resolve(AbsoluteDeadline::new(TimePoint::new(primary, instant)))))
           handle = time.schedule(TimePoint::new(primary, instant), Box::new(|_| {}))   -- Err → fail(Arm, "KC-17: …")
           scheduled.push(handle)
   ```

   `working` is a local copy, so that `configuration` changes only at dispatch (KC-27): a Run that fails at `arm` leaves `effective()` as `prepare` left it. `admit_with(…, config)` is `admit` (step 7) judging against `config` instead of `configuration` — for the Admitter's `effective` and for KC-26's `Requested`; `admit(shared, …)` is `admit_with(shared, …, &lock(configuration).clone())`. `admit` is step 7's. In this step write it as: rewrite the target (KC-23), and nothing else; step 7 completes it. After `move_to(Running)` in KC-18, dispatch every `batch` entry in `batch` order (`dispatch`, step 7; in this step push the rewritten Action onto the instance's queue and assign the id).
3. **The round** (`stepping.rs`). This replaces step 5's placeholder.

   ```rust
   /// The first instance whose step failed in a round (KC-30).
   pub(super) struct Fault(pub(super) Mutex<Option<(Inst, ModuleError)>>);

   fn guard_step(inst: Inst, fault: &Fault, f: impl FnOnce() -> Result<StepOutcome, ModuleError>) -> Result<StepOutcome, ModuleError> {
       let r = contain(f);
       if let Err(e) = &r { let mut g = lock(&fault.0); if g.is_none() { *g = Some((inst, e.clone())); } }
       r
   }

   /// A stepped Provider, watched (KC-30). Every method but `step` delegates.
   struct WatchProvider<'a> { inner: &'a mut dyn Provider, inst: Inst, fault: &'a Fault }
   impl Provider for WatchProvider<'_> {
       fn instance(&self) -> &ProviderInstance { self.inner.instance() }
       fn coerce(&self, r: &Requested) -> Result<CoerceReport, ModuleError> { self.inner.coerce(r) }
       fn prepare(&mut self, f: &Fragment, c: PrepareContext) -> Result<PrepareReport, ModuleError> { self.inner.prepare(f, c) }
       fn arm(&mut self) -> Result<(), ModuleError> { self.inner.arm() }
       fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError> { self.inner.start(at) }
       fn stop(&mut self, m: StopMode) -> Result<(), ModuleError> { self.inner.stop(m) }
       fn cleanup(&mut self) { self.inner.cleanup() }
       fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
           let (inst, fault) = (self.inst, self.fault);
           guard_step(inst, fault, || self.inner.step(until))
       }
   }
   // WatchExecutor and WatchSink: the same, over `dyn Executor` and `dyn Sink`, delegating every method.

   /// One round at `t` (MA-30, KC-21, KC-30, KC-31). `cleaning` is true in cleanup step 3's drain.
   pub(super) fn round(shared: &Shared, t: TimePoint, cleaning: bool) {
       let Some(collector) = shared.collector.get() else { return };
       let fault = Fault(Mutex::new(None));
       let result = {
           // lock: every Provider slot with `stepped`, every Executor slot, every Sink slot, each paired with its Inst;
           // with `cleaning`, use try_slot and skip a slot that is held, and skip every instance with (5, inst) in `done`
           // (its cleanup() was called, KA-12); otherwise `lock`.
           // wrap each guard in WatchProvider / WatchExecutor / WatchSink;
           // build Vec<SteppedInstance { id: shared.first_fragment(inst), inner: SteppedRef::Provider(&mut watch) … }>;
           step_until_quiescent(&mut insts, t, &**collector, &kernel_source())
       };                                                   // every guard is dropped here
       if let Err(e) = result {
           match lock(&fault.0).take() {
               Some((inst, err)) if err.kind == ModuleErrorKind::DeviceLost => emit_device_lost(shared, inst, t, &err.message),
               Some((inst, err)) => fail_run(shared, inst, &err),     // request(Failed { run }, Abort) + failure reason "KC-30: <fragment>: <message>"
               None => {}                                              // STEP_LIVELOCK from step_until_quiescent itself: already emitted
           }
           let _ = e;
       }
       drain_and_react(shared);                                         // KC-31, step 8 completes it
   }
   ```

   `emit_device_lost(shared, inst, t, message)`: `collector.emit_control(Event { source: shared.source_root(inst), time: t, severity: Severity::Fatal, kind: EventKind::parse(EventKind::DEVICE_LOST).expect("a Kernel kind"), payload: json!({ "message": message }) })`.

   In this step `drain_and_react` drains the collector into `delivered` and applies `Stop` and `Abort` reactions with `request` (KC-31's `mark_artifact` and escalation are step 8's).
4. **The loop** (`stepping.rs`), a `RunHandle` method:

   ```text
   /// Runs KC-20's loop. Returns when the Run has an end request, when a round ran at an instant ≥ `until`
   /// (if given), or — in a Session — when next_wakeup returns None.
   fn run_loop(&mut self, until: Option<TimePoint>):
       loop:
           if an end is requested or the state is not Running: return
           let Some(t) = shared.authority.next_wakeup() else {
               if kind == Spec { request(Completed {}, Orderly) }    -- KC-33
               return
           }
           KC-22: same_count = if Some(t) == last_wakeup { same_count + 1 } else { 1 }; last_wakeup = Some(t)
                  if same_count > STEP_ROUND_CAP:
                      collector.emit_control(Event { source: kernel_source(), time: t, severity: Fatal,
                                                     kind: STEP_LIVELOCK, payload: json!({ "wakeups": STEP_ROUND_CAP }) })
                      same_count = 0; drain_and_react(shared)
                      if the Policy's reaction for STEP_LIVELOCK is neither Stop nor Abort:
                          request(Failed { run }, Abort, Some("KC-22: STEP_LIVELOCK at <t>; time cannot advance"))
                      continue
           run the agenda: remove every item with instant ≤ t.ticks, in (instant, insertion) order; for each:
               Stop { target: None } → request(Completed {}, Orderly)
               any other (a Stop with a target) → admit(shared, action, Origin::Schedule, &[]) then dispatch;
                           a refusal records the failure reason "KC-17: agenda entry <i>: <violations:?>"
                           and requests Failed { run } in Abort mode
           round(shared, t, false)
           check_lease()                                              -- step 7 writes it; in this step a no-op
           if let Some(u) = until: if t ≥ u (try_cmp; an Err counts as reached): return
   ```

5. **advance_to, run_until_end** (KC-29): as spec 06 KC-29 says, using `run_loop`. `advance_to` converts with `ceil_convert` (an `Err` → `Err(NotOnPrimaryRoot { t })`), returns `Ok` when the converted `t` ≤ `now`, schedules a no-op at it (push the handle to `scheduled`), calls `run_loop(Some(t))`, then `settle()`, then returns `Err(Ended { termination })` if `CleanedUp`, else `Ok`. `run_until_end(horizon)`: `run_loop(Some(horizon))`, `settle()`, the same return.
6. **The drain**, at the **top** of `Ops::perform(StopRx, …)` — before the role and `prepared` filters, so that a Run with no Sink or Executor drains too: if the current end request's mode is `Orderly` and `drained.swap(true) == false`, run the loop below; otherwise (every later call of any step) set `closing = true` first, so that a drain RS-8a abandoned stops before this operation acts (KA-12). Also set `closing = true` on entry to every `perform` of a step after `StopRx`.

   ```text
   last = None; count = 0
   for _ in 0..DRAIN_WAKEUP_CAP:
       if the current end request's mode is Abort: break                 -- KC-32 escalated
       if shared.closing: break                                          -- a later cleanup operation started: this drain was abandoned (RS-8a)
       let Some(t) = authority.next_wakeup() else { break };
       count = if Some(t) == last { count + 1 } else { 1 }; last = Some(t)
       if count > STEP_ROUND_CAP: emit STEP_LIVELOCK (source kernel, payload { "wakeups": STEP_ROUND_CAP }, as KC-22) and break
       round(shared, t, true)
   if shared.closing: return Ok(())                                     -- abandoned: this operation must not stop an instance now
   ```

   (The Run's class is always Simulation in Phase 2, KC-2; no class test is needed here.) The `closing` rules have no deterministic test — exercising them needs a drain that outlives the 5 s deadline — and are carried by construction; `kc_44_a_wedged_step_does_not_prevent_the_manifest` covers the neighbouring timeout path.
7. KC-21: after the start dispatch in KC-18, call `round(shared, now, false)` (this replaces step 5's placeholder call).

**Tests (step 6)**

| test | fixture | assertion |
|---|---|---|
| `kc_16_a_spec_time_resolves_on_the_target_stream` | `SteppedProvider` `p` over `TestProvider::new("dev", 2)` bound as `radio`, `.declaring("dev/rx", 50, 1).declaring("dev/tx", 20, 1).registering_at_arm("dev/tx")`; a schedule entry `{ at: { clock: radio, offset_ticks: 10 }, action: TxBurst { target: radio/tx, waveform: a ref to 80 bytes supplied in `inputs`, repeat: false, late_policy: send_asap_and_flag, metadata: {} } }`; `run_until_end(TimePoint(root, 1000))` | probe has `p:burst_at:<k>` where `k` is the burst's `at` on the tx SampleClock: the tx clock's origin is the arm instant 0 and its ratio 20, so the primary-root instant `T0 + 200 = 200` is tx tick `10` → `p:burst_at:10` |
| `kc_16_an_ambiguous_spec_time_is_refused` | the same Provider; a `Stop {}` at `{ clock: radio, offset_ticks: 1 }` | `Failed { arm }`; reason starts with `"KC-16: entry 0: ambiguous"` |
| `kc_16_off_root_negative_and_overflowing_times_are_refused` | three Runs, each with one schedule entry `Stop { target: None }` on resource `radio`: (a) the only stream clock is `.declaring_on("radio/rx", other, 1, 1)` with `other` a second root registered in the Rig's clocks; (b) a clock on the primary root and `offset_ticks: -1`; (c) that clock at ratio 2 and `offset_ticks: i64::MAX` | `Failed { arm }` each time, with reasons containing `"not on the primary root"`, `"negative offset"` and `"overflow"` |
| `kc_17_a_scheduled_stop_ends_the_run_at_its_instant` | one clock `dev/rx` at ratio 10 declared; `Stop {}` at offset 100 | `Completed {}`; `manifest.termination.at == Some(TimePoint(root, 1000))` |
| `kc_19_a_reject_at_plan_burst_with_a_short_lead_is_refused` | `SteppedProvider` over `TestProvider::new("radio", 2).with_min_command_lead(Duration(host.monotonic, 5_000_000))`, `.declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")`; a `TxBurst { target: radio/tx, late_policy: reject_at_plan, repeat: false, metadata: {}, waveform: a ref to 80 bytes }` at `{ clock: radio, offset_ticks: 1_000_000 }` (1 ms), its 80 bytes supplied in `Assembly.inputs` (else KC-9 refuses first) | `Failed { arm }`; reason starts with `"KC-19: SC-27"` |
| `kc_20_virtual_time_advances_only_through_next_wakeup` | `SteppedProvider` `.with_wakeups(&[10, 20, 30])`; `run_until_end(TimePoint(root, 1000))` | the `p:step:` lines are exactly `p:step:0`, `p:step:10`, `p:step:20`, `p:step:30`; the Run is `Completed` (nothing left) |
| `kc_22_a_same_instant_wakeup_loop_is_step_livelock` | `SteppedProvider` `.rescheduling_forever()`; `run_until_end(TimePoint(root, 10))` | `Stopped { policy { STEP_LIVELOCK } }`; a `STEP_LIVELOCK` event with source `kernel` is delivered; the cleanup mode was `abort` (`transitions` has `Stopping { abort }`) |
| `kc_22_a_downgraded_livelock_still_ends_the_run` | the same with the Spec's `policies.failure: { "STEP_LIVELOCK": "continue" }` | `run_until_end` returns; `Failed { run }`; `ezsdr.failure.reason` starts with `"KC-22"`; `Stopping { abort }` |
| `kc_23_targets_are_rewritten_through_matched` | `SteppedProvider` over `TestProvider::new("dev", 2)` `.declaring("dev/0/rx", 1, 1)`; resource `radio` of kind `test.line` requiring `test.count = 2`, so `matched[radio] = dev/0`; a schedule entry `UpdateParameter { target: radio/x, key: test.gain, value: 3.0, class: hardware_timed }` at `{ clock: radio, offset_ticks: 0 }` (the clock `dev/0/rx` lies within `dev/0`, so the `SpecTime` resolves) | probe has `p:action:UpdateParameter:dev/0/x@0` |
| `kc_17_scheduled_updates_are_admitted_cumulatively` | `run_registry_classed()`; `run_checks(true)`; environment `{ test.limits: { max_grid: 30, gate: "test.flag" } }`; the Spec requires `test.grid = 20.0`, `test.flag = false`; `SteppedProvider` `.declaring("radio/rx", 1, 1)`; schedule entry 0 `UpdateParameter { radio, test.grid, 40.0, cold }` at offset 10, entry 1 `UpdateParameter { radio, test.flag, true, cold }` at offset 20 | `Failed { arm }`; `ezsdr.failure.reason` starts with `"KC-17: entry 1"` and names `test.limits`. With entry 1 removed the Run reaches `Running` (entry 0 alone is admitted: the gate is off) |
| `kc_39_orderly_cleanup_drains_the_tail` | `SteppedProvider` `.with_tail(2)` feeding a `RecordingSink` `rec` (drop_oldest, capacity 8); `finish()` at `now = 0` | `rec:block:10` and `rec:block:20` are in the probe, both **before** `rec:stop:Orderly` |
| `kc_39_abort_cleanup_does_not_drain` | two resources `p`, `q` bound with **distinct** selectors (two instances, else KC-4 refuses one object per description), a `SteppedProvider` `p` `.with_wakeups(&[10])` and a second one `q` `.step_error_at(0)`, so the Run ends `Failed { run }` in `abort` mode at 0 with `p`'s wakeup at 10 still pending | no `p:step:` or `q:step:` line comes after the first `*:stop:` line (an `orderly` drain would step `p` at 10); `p:stop:Abort` present |
| `kc_29_advance_to_refuses_an_unrelated_time` | a second root registered in the Rig's `clocks` | `advance_to(TimePoint(other, 5))` is `Err(NotOnPrimaryRoot { .. })` |

**Mutation checks (step 6):** KC-17's working copy (skip the `working[…] = value` line: `kc_17_scheduled_updates_are_admitted_cumulatively` fails); KC-16's ambiguity, off-root, negative-offset and overflow refusals; KC-22's downgrade rule; KC-19's lead refusal; KC-22's cap (make it `> usize::MAX - 1` and confirm `kc_22` hangs → use a timeout: run it with `timeout 30 cargo test …`; a hang counts as a failure); KA-12's drain (skip it: `kc_39_orderly_…` fails).

**Done-list:** the thirteen tests of the table pass; all earlier tests pass; the three commands pass.

### Step 7 — Coordinator D: admission, dispatch, Sessions, the Lease

**Implements:** KC-24…KC-28, KC-35…KC-37, KA-6, KA-7's use (RS-19), KA-21's use.

1. **`admit_with`** (`admission.rs`), complete, in KC-24's order; `admit(shared, …)` delegates to it with a clone of `configuration`, and every mention of `configuration` below means the `config` argument:

   ```rust
   pub(super) struct Admitted { pub(super) action: Action, pub(super) inst: Inst, pub(super) fragment: Ident,
                                pub(super) coercions: Vec<Coercion>, pub(super) warnings: Vec<Warning> }

   pub(super) fn admit_with(shared: &Shared, mut action: Action, origin: Origin, incoming: &[Coercion],
                            config: &BTreeMap<Ident, BTreeMap<Key, Value>>) -> Result<Admitted, Vec<Violation>>
   ```

   1. `frozen` → `ezsdr.dispatch`, `"RS-6: dispatch is frozen"`.
   2. `origin != Schedule` and `lock(machine).check_running()` fails → `ezsdr.run_state`, `"RS-18: the Run is not Running"`.
   3. `Action::Abort` from `Module`: handled by the Submitter before `admit` (below); from any other origin it cannot occur. `Action::Stop { target: None }` from `Module` → `ezsdr.target`, `"KC-24: a Module ends a Run only with Abort"`. Otherwise `rewrite` the target (KC-23) and write the rewritten id back into the Action (`TxBurst.target`, `SetTimer.target`, `UpdateParameter.target`, `PeripheralCommand.target`, `Emit.target`, `Stop.target`).
   4. `TxBurst`: KC-24 (4) exactly; the SampleClock record is the last `ctx.clocks.sample_clock_records()` entry with `stream == rewritten && ended_at.is_none()`.
   5. `UpdateParameter` to an `Inst::Provider` from `Schedule` or `Session`: KC-26 (below). Its coercion, if any, is appended to the coercion list and replaces the Action's `value`.
   6. `Admitter { checks: &ctx.checks, environment: &ctx.profile.environment, declared_classes: &ctx.declared_classes, spec_coercion: &ctx.spec.policies.coercion, registry: &ctx.registry, is_session: ctx.kind == Session }.admit(&configuration, &proposed, &coercions, CheckStage::Runtime)` with `proposed = { fragment: { key: value } }` for an `UpdateParameter` and `{}` otherwise.
   7. Return `Admitted`.
2. **KC-26** (`admission.rs`): lock the target Provider's slot; `current = configuration[fragment]`; `constraints = { k: Eq(v) for (k, v) in current if v.is_scalar() }` with `constraints[key] = Eq(value)`; `request = Requested { resource: matched[fragment].clone(), constraints }`; `report = contain(|| provider.coerce(&request))`. `Err(e)` → `ezsdr.coercion`, key, `"RS-17: <fragment>: <e.message>"`. Each `report.rejected` entry → one `ezsdr.coercion` violation with its key and `"RS-17: <fragment>: <reason>"`. `report.applied.get(key)` absent → `"RS-17: <fragment>: its Provider applied no value for <key>"`. Present and `!=` the value → `Coercion { key, requested: value, applied, reason: the report's coercion reason for key, or "RS-17: coerced by its Provider" }`.
3. **`dispatch`** (KC-25): `id = ActionId(next_action.fetch_add(1, SeqCst))`; for an `UpdateParameter`, `configuration[fragment][key] = value`; push the Action onto `shared.queue(inst)`; return `id`.
4. **`Submitter`** (the `ActionSubmitter` every `PrepareContext` carries): `shared = self.shared.upgrade()` (none → `Err([ezsdr.run_state, "KC-24: the Run has ended"])`); `Action::Abort { cause }` → `request(Stopped { cause }, Abort)`, `Ok(ActionId(0))`; otherwise `admit(&shared, action, Origin::Module, &[])` then `dispatch`.
5. **Sessions** (`admission.rs`, `RunHandle::submit`): KC-28 steps 1–6 exactly. `earliest` (step 3): for `SessionAction::Vocabulary { ns, verb, target, .. }` whose `ctx.registry.verb(ns, verb)` compiles to `UpdateParameter`, `now`; otherwise, when `rewrite(target)` gives an `Inst::Provider(i)` whose `lead` is `Some(d)`, `now + ceil_rescale(clocks, d, primary)`; otherwise `now`. The waveform (step 2): `hash = ContentHash::of_bytes(bytes)`; `r = manifest::ingest_input(Ident::parse(&format!("input_{k}")), Namespace::parse("ezsdr.input"), format!("mem:{hash}"), bytes)` with `k` = the number of inputs so far; push `r` to `inputs` and the bytes to `store` unless the hash is already there (then reuse the existing ref).
6. **The Lease** (KC-35, KC-36): `check_lease()`: `lease.expired(&*host_clock)` → `request(Stopped { cause: LeaseExpiry {} }, Orderly)`, `settle()`. `disconnect()`: `lease.on_disconnect(&*host_clock)` → `Some(cause)` → `request(Stopped { cause }, Orderly)`, `settle()`; `None` → nothing more. Call `check_lease()` at the start of `submit`, `advance_to` and `run_until_end`, and after every round in `run_loop`.
7. **KC-37** in `submit`: a compiled `ControlOp::RunChild` → `Rejected { violations: [Violation { check: ezsdr.run_child, key: None, requested: None, reason: "RS-25a: child Runs are Phase 6's" }] }`.

**Tests (step 7).** Session tests build the Run with `connect(&profile_one(), rig.assembly, Lease::attached())`.

| test | fixture | assertion |
|---|---|---|
| `kc_24_a_burst_needs_a_transmit_sample_clock` | Session; `submit(SessionAction::Vocabulary { ns: test, verb: start_repeat, target: radio/tx, at: None, params: {} }, Some(&[0u8; 80]))` with no clock registered | the entry is `Rejected`; a violation reason starts with `"SC-23"` |
| `kc_24_a_module_reject_at_plan_burst_is_refused` | a `ProbeExecutor` `x` `.submitting(TxBurst { target: radio/tx, late_policy: reject_at_plan, … })` in an Island (the target must rewrite, or step 3 refuses it first) | probe has a line starting `"x:submit:err:ezsdr.late_policy:KC-19"` |
| `kc_24_a_module_action_during_cleanup_is_refused` | a `ProbeExecutor` `x` `.submitting_at(10, SetTimer { target: radio, at: TimePoint(root, 20), token: 1 })` in an Island; a `SteppedProvider` `.with_tail(1)`, whose tail wakeup is at 10; `finish()` at `now = 0`, so the round at 10 is cleanup step 3's drain, after step 1 froze dispatch | probe has a line starting `"x:submit:err:ezsdr.dispatch:RS-6"` |
| `kc_24_a_module_stop_without_target_is_refused` | a `ProbeExecutor` `x` `.submitting(Stop { target: None })` placed in an Island (as in `kc_11`) | probe line `"x:submit:err:ezsdr.target:KC-24: a Module ends a Run only with Abort"`; the Run is still `Running` afterwards |
| `kc_24_a_burst_to_a_non_provider_target_is_refused` | Session whose profile also binds a `RecordingSink` `rec` with `feed { port: radio.rx, policy: drop_oldest, capacity: 4 }` (and its link placement); `submit(Vocabulary { test, start_repeat, sink/rec, None, {} }, Some(&[0u8; 80]))` | `Rejected`; a violation whose reason starts with `"SC-23: sink/rec is not a Provider stream"` |
| `kc_24_a_burst_time_on_an_unrelated_root_is_refused` | Session; `.declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")`; a second root `other` registered in the Rig's clocks; `start_repeat` with `at: Some(TimePoint(other, 5))` | `Rejected`; a violation whose reason starts with `"SC-23b"` |
| `kc_24_a_module_abort_ends_the_run` | `ProbeExecutor` `.submitting(Abort { cause: Abort { cause: "test" } })` | `Stopped { abort { cause: "test" } }` in `abort` mode |
| `kc_23_an_unknown_target_is_refused` | Session; `submit(SetParameter { target: nothing, key: test.gain, value: Num(1.0) })` | the entry is `Rejected`; a violation with check `ezsdr.target` and a reason starting `"KC-23: "` |
| `kc_21_an_action_is_seen_at_its_admission_instant` | Session; `advance_to(TimePoint(root, 500))`, then `submit(SetParameter { radio, test.gain, Num(1.0) })` | the probe has `p:action:UpdateParameter:radio@500` (the Action was handled in a round at 500, KC-21) |
| `kc_25_an_admitted_update_changes_the_configuration` | Session; `submit(SetParameter { target: radio, key: test.gain, value: Num(3.0) }, None)` | the entry is `Admitted` with one dispatched id; `effective()["radio"]["test.gain"] == Num(3.0)`; the probe has `p:update:test.gain=3.0` |
| `rs_17_a_session_rate_change_is_coerced_by_its_provider` | Session with `run_registry_classed()` (the Phase 1 `test.grid` has no update class, so no runtime update of it is admissible); a `SteppedProvider` over `TestProvider::new("radio", 2).with_grid(20.0)`; `SetParameter { radio, test.grid, Num(19.5) }` | `Admitted`, `coercions` holds `test.grid` 19.5 → 20.0; the probe's `p:update:test.grid=20.0` |
| `rs_17_a_scheduled_rate_change_under_reject_is_refused` | a Spec Run with `run_registry_classed()`; a `SteppedProvider` over `TestProvider::new("radio", 2).with_grid(20.0)`, `.declaring("radio/rx", 1, 1)`; a schedule entry `UpdateParameter { radio, test.grid, 19.5, class: cold }` at `{ clock: radio, offset_ticks: 0 }` (the class must equal the declared one, RS-52); the Spec's coercion policy for `test.grid` is its default, `reject` | `Failed { arm }`; reason contains `"SB-46"` |
| `rs_17_a_session_change_beyond_a_joint_limit_is_refused` | Session with `run_registry_classed()`; `TestProvider::new("radio", 2).with_joint_limit(50.0).with_effective("test.count", Int(2))` (a Session's implicit Spec requires nothing, so the configuration holds only what `prepare` reports); `SetParameter { radio, test.grid, Num(40.0) }` | `Rejected`; a violation with check `ezsdr.coercion` on `test.grid` |
| `kc_26_a_provider_that_applies_nothing_is_refused` | Session; `TestProvider::new("radio", 2).omitting_from_applied("test.gain")`; `SetParameter { radio, test.gain, Num(1.0) }` | `Rejected`; a violation of check `ezsdr.coercion` whose reason contains `"applied no value for test.gain"` |
| `kc_28_a_malformed_action_takes_no_sequence_number` | Session; `SetParameter` whose value is a `Value::Map` with a non-ASCII key; then a valid one | the first `submit` is `Err(Malformed { .. })`; the second entry has `seq == 0` |
| `kc_28_a_waveform_is_an_input_before_admission` | Session; `SteppedProvider` `.declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")`; `submit(Vocabulary { test, start_repeat, radio/tx, None, {} }, Some(&[0u8; 800]))` | `Admitted`; the Manifest's `inputs` holds one ref with `size_bytes == 800` and a `uri` starting `"mem:sha256:"`; the probe's burst line exists |
| `kc_28_an_untimed_burst_is_admitted_at_now_plus_lead` | Session; `TestProvider::new("radio", 2).with_min_command_lead(Duration(host.monotonic, 2_000_000))`, `.declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")`; `start_repeat` with no `at` at `now = 0` | the entry's coercions include `ezsdr.action.at` applied `Int(2_000_000)`; `p:burst_at:2000000` |
| `kc_36_detached_lease_expiry_ends_the_run` | Session with `Lease::detached(5000, false, "tok", &*host)`; `disconnect()`; `host.advance(5000)`; `check_lease()` | `Stopped { lease_expiry }` |
| `kc_36_attached_disconnect_ends_the_run` | Session, Attached; `disconnect()` | `Stopped { client_disconnect }` |
| `kc_37_run_child_is_refused` | Session; `RunChild { .. }` | `Rejected`, check `ezsdr.run_child` |
| `kc_35_connect_refuses_an_invalid_lease` | `connect` with `Lease { mode: LeaseMode::Detached { ttl_ms: 0, renewable: false }, token: Some("t"), holder: None, expires_at_host: None, adoptions: 0, released: false }` | `Err(SpecError::Structural { reason })` with `reason` containing `"RS-21"` |
| `kc_28_stop_run_ends_the_session` | Session; `SessionAction::Stop { target: None }` | the entry is `Admitted`; the Run is `Stopped { client }` |

**Mutation checks (step 7):** KC-24 (1)'s frozen refusal (`kc_24_a_module_action_during_cleanup_is_refused`); KC-24 (3)'s Module `Stop` refusal; KC-24 (4)'s non-Provider and SC-23b refusals; KC-26's applied-nothing refusal; KC-24 (4) SC-23; KC-24 (4) RejectAtPlan from a Module; KA-6's coerce (skip step 5: `rs_17_a_session_rate_change…` fails); KC-28's `check_entry` (skip it: `kc_28_a_malformed…` fails because the entry takes seq 0).

**Done-list:** the twenty-two tests of the table pass; all earlier tests pass; the three commands pass.

### Step 8 — Coordinator E: failures, the Policy, marks and the complete Manifest

**Implements:** KC-30 (the remaining cases), KC-31, KC-32, KC-40, KC-41, KC-42, KC-44 (the wedged slot and the seal failure), KC-45 (the rest).

1. **`request`** (KC-32), in `stepping.rs`:

   ```text
   request(shared, termination, mode, reason: Option<String>):
       end = lock(shared.end)
       None → *end = Some(EndRequest { termination, mode });
              if termination is Failed { stage }: *failure = Some((stage, reason))      -- the ezsdr.failure section (KC-7)
       Some(r) if r.mode == Orderly && mode == Abort →
           r.mode = Abort
           push to `also`: `cause` if termination is Stopped { cause }; StopCause::Abort { cause: reason } if it is Failed { .. };
           nothing for Completed
       otherwise → nothing
   ```

   `fail(stage, reason)` (step 4) and `fail_run` (step 6) now call `request(Failed { stage }, Abort, Some(reason))`; every other caller passes `None`. `ezsdr.failure` is written only from `failure`, which only the first request sets, so a failure that did not become the Termination is recorded in `also` and not as `ezsdr.failure` (KC-32).
2. **KC-31** `drain_and_react(shared)`: `events = collector.drain()`; for each event, `policy.reaction_for_event(&kind, severity)`: `Continue` nothing; `MarkArtifact` push `(kind, time)` to `marks`; `Stop` `request(Stopped { cause: Policy { kind } }, Orderly)`; `Abort` the same in `Abort`. Then `if let Some((kind, reaction)) = collector.escalation()` apply `Stop` / `Abort` the same way. Append `events` to `delivered`.
3. **KC-42** in `Ops::perform(FinaliseArtifacts, …)`: for each artifact in `artifacts`, `span` = (the first `ContinuityMap`'s `first`, the last one's `end`), each converted to the primary root with `ceil_convert` for `end` and `ClockRegistry::convert(..).floor()` for `first`; with no map, or a conversion error, every mark applies. Each mark `(kind, time)` whose `time`, converted to the primary root and floored, lies in `[first, end)` is pushed with `manifest::mark_open_artifacts(std::slice::from_mut(artifact), kind, time)`.
4. **KC-30**'s remaining cases, as the rule lists them: `DeviceLost` from `stop` and `cleanup` also emits `DEVICE_LOST`; a panic in `stop` is a `CleanupFailure` (already, through `contain`).
5. **KC-44**'s wedged slot: already in place from step 4 (`try_lock`); confirm with the test below.
6. **KC-45**'s remaining fields: `events.counters` from the step-7 snapshot (or `collector.counters()` when the snapshot is `None`); `events.delivered`; `action_log = log.entries().to_vec()`; `inputs`; `clocks = { domains: ctx.clocks.domains(), relations: vec![], sample_clocks: ctx.clocks.sample_clock_records() }`; `termination.also = also`; `termination.at = Some(now)`; `termination.host_utc_nanos = host_clock.utc_nanos()`.

**Tests (step 8)**

| test | fixture | assertion |
|---|---|---|
| `kc_30_a_panicking_module_fails_the_run_not_the_process` | `SteppedProvider` `.panicking_in_step()` | `Failed { run }`; `ezsdr.failure.reason` starts with `"KC-30: radio: a Module panicked"`; the Manifest is sealed |
| `kc_30_a_panic_in_coerce_fails_validate` | `TestProvider::new("radio", 2).panicking_in_coerce()` | `Failed { validate }`; `ezsdr.failure.reason` starts with `"KC-30: a Module panicked during validate"`; `cleanup_failures` is empty (the poisoned slot is taken by `try_slot`, not reported as held) |
| `kc_30_device_lost_is_the_kernel_event_and_aborts` | `.device_lost_at(0)` | a delivered `DEVICE_LOST` event with `source == instance().id` (`radio`) and severity `Fatal`; the counter row `(radio, DEVICE_LOST)` has count 1; `Stopped { policy { DEVICE_LOST } }`, mode `abort` |
| `kc_31_mark_artifact_marks_only_artifacts_open_then` | `policies.failure: { "test.custom": "mark_artifact" }`; `SteppedProvider` `.emitting("test.custom", Warning, 150).with_wakeups(&[150])`; a `RecordingSink` `.returning_spans(&[(100, 200), (300, 400)])` fed by the Provider; `advance_to(TimePoint(root, 160))`; `finish()` | artifact `rec_0` has one mark `(test.custom, 150)`; `rec_1` has none |
| `kc_32_an_abort_during_orderly_escalates_and_is_recorded` | one `SteppedProvider` `.with_tail(1).device_lost_at(10)`; `finish()` at `now = 0`. The orderly cleanup's drain steps it at 10 (its tail wakeup), where it returns `DeviceLost`; `DEVICE_LOST`'s default reaction is `abort` | the transitions hold `Stopping { orderly }`; the termination is `Stopped { client }` (the first request, KC-32); `termination.also == [Policy { kind: DEVICE_LOST }]`; `p:cleanup` is in the probe (cleanup still ran to its end) |
| `kc_39_every_instance_is_stopped_before_it_is_cleaned_up` | one `SteppedProvider` `p`, one `ProbeExecutor` `x` in an Island, one `RecordingSink` `rec` fed by `p` | for each of `p`, `x`, `rec`: the index of `<name>:stop:` is less than the index of `<name>:cleanup`; `rec_0` is in `manifest.artifacts` |
| `kc_41_a_failing_sink_stop_is_a_cleanup_failure` | `RecordingSink` `.failing_stop()` | `cleanup_failures` has an entry with `step == StopRx` and `fragment == Some("rec")`; `artifacts` is empty |
| `kc_44_a_wedged_step_does_not_prevent_the_manifest` | `SteppedProvider` `.wedged_in_stop()` | `finish()` returns within 20 s (the test itself takes about 5 s: one `DEFAULT_CLEANUP_DEADLINE_MS`); `cleanup_failures` has a `StopTx` entry with `timed_out: true` and a `ReleaseAndWriteManifest` entry whose reason starts with `"KC-44"`; `hash.is_some()` |
| `kc_45_manifest_fields` | one Provider (`SteppedProvider` over `TestProvider::new("radio", 2).with_fidelity(Fidelity { timing: EnvelopeFidelity::Envelope, ..Fidelity::NONE })`, publishing every 100), one `RecordingSink`, one link; `advance_to(TimePoint(root, 350))`; `finish()` | `run.kind == Spec`, `run.parent.is_none()`, `run.id` equals the id `RunHandle` reported, `run.execution_class == Simulation`, `run.deterministic`, `run.fidelity.timing == EnvelopeFidelity::Envelope` (the Provider's declared fidelity reached the Manifest); `events.counters` holds a row `(sink/rec, DEVICE_LOST)` and a row `(kernel, STEP_LIVELOCK)`, both with count 0 (KC-8's pairs); `policy.is_some()`; `plan.is_some()`; `prepare.reports.len() == 2`; `modules` has `ezsdr.test.provider`, `ezsdr.test.sink` and `ezsdr.test.link`, each with its `impl_hash`; `vocabularies["test"] == 1.0.0`; `sections["ezsdr.links"]` is an array of one `{ link, from, to, drops: 0 }`; `lease.released`; `termination.reason == Stopped { client }` |
| `kc_45_sample_clocks_and_domains_are_recorded` | `SteppedProvider` `.declaring("dev/rx", 10, 1).registering_at_arm("dev/rx")` | `clocks.sample_clocks.len() == 1` with `stream == dev/rx`; `clocks.domains` equals `clocks.domains()` of the Rig's registry; `clocks.relations` is empty |
| `kc_45_a_provider_section_outside_its_namespace_is_a_cleanup_failure` | a `TestProvider` whose `instance.sections` holds `other.ns: {}` (add a `with_section(ns, value)` builder to `TestProvider`) | the section is absent; `cleanup_failures` has a `ReleaseAndWriteManifest` entry whose reason starts with `"KC-44"` |

**Mutation checks (step 8):** KC-30's containment of `validate` (call it bare: `kc_30_a_panic_in_coerce_fails_validate` aborts the test process, which counts as failing); `try_slot`'s poison rule (use `try_lock().ok()`: the same test fails on `cleanup_failures`); KC-31's `mark_artifact` (drop the push: `kc_31` fails); KC-32's escalation record (drop the `also` push: `kc_32` fails); KC-41 (append the artifacts even on error — there are none, so instead make `StopRx` return `Ok` on a Sink error: `kc_41` fails); KC-44's `try_lock` (use `lock`: `kc_44` hangs; run with `timeout 60`).

**Done-list:** every test of spec 06 §12 exists under its exact name and passes (compare the table in §12 with `grep -n "^fn " crates/ezsdr-kernel/tests/coordinator.rs`); `kernel_surface` passes; the `NEW:` count printed by `ov_23b_kernel_growth_is_the_new_count` (run it with `-- --nocapture`) is recorded in the notes; the three commands pass.

**Then stop.** Report to the owner that step 8 is done and Review K (an adversarial review of the Kernel diff since `96976c5`) is due. Do not start step 9 until the owner says so.

---

## Step 9 — The nine new crates as empty workspace members

**Implements:** `00-overview.md` §5; PO-2, PO-4 (their baseline). **Read first:** `00-overview.md` §5 and §6.

1. Record the baseline for PO-4 **before** anything changes `Cargo.lock`:

   ```bash
   mkdir -p crates/ezsdr-acceptance/tests
   ```

   ```bash
   awk '/^\[\[package\]\]/{n="";v="";s=""} /^name = /{n=$3} /^version = /{v=$3} /^source = /{s=$3; print n" "v}' Cargo.lock | tr -d '"' | sort > crates/ezsdr-acceptance/tests/baseline_external_packages.txt
   ```

   The file lists every external package of the lock as `name version`, one per line. Check that it has 27 lines (the lock's 28 packages minus `ezsdr-kernel`) and that no line names `ezsdr-kernel`.

2. Root `Cargo.toml`: replace `members = ["crates/ezsdr-kernel"]` with:

   ```toml
   members = [
       "crates/ezsdr-kernel",
       "crates/ezsdr-radio",
       "crates/ezsdr-sim",
       "crates/ezsdr-sink",
       "crates/ezsdr-hostmem",
       "crates/ezsdr-sim-engine",
       "crates/ezsdr-mock-radio",
       "crates/ezsdr-link-host",
       "crates/ezsdr-sink-capture",
       "crates/ezsdr-acceptance",
   ]
   ```

3. For each new crate, create `crates/<name>/Cargo.toml` and `crates/<name>/src/lib.rs`. Every `Cargo.toml` has this `[package]` block, with the crate's own `name`, `version` and `description` from the table:

   ```toml
   [package]
   name = "<name>"
   version = "<version>"
   description = "<description>"
   edition.workspace = true
   rust-version.workspace = true
   license.workspace = true
   repository.workspace = true
   ```

   | name | version | description | `[dependencies]` | `[dev-dependencies]` |
   |---|---|---|---|---|
   | `ezsdr-radio` | `1.0.0` | `Ez-SDR v4 Radio Model Vocabulary radio 1.0.0 (plan/phase2/07-radio-model.md)` | kernel, serde, serde_json, schemars | kernel+testing |
   | `ezsdr-sim` | `1.0.0` | `Ez-SDR v4 Simulation Vocabulary sim 1.0.0 (plan/phase2/08-simulation.md)` | kernel, serde, serde_json, schemars | kernel+testing |
   | `ezsdr-sink` | `1.0.0` | `Ez-SDR v4 Sink Vocabulary sink 1.0.0 (plan/phase2/10-host-data-path.md)` | kernel, serde, serde_json, schemars | kernel+testing |
   | `ezsdr-hostmem` | `1.0.0` | `Ez-SDR v4 host memory domain and buffer pool (plan/phase2/10-host-data-path.md)` | kernel | kernel+testing |
   | `ezsdr-sim-engine` | `1.0.0` | `Ez-SDR v4 Module ezsdr.sim-engine 1.0.0 (plan/phase2/08-simulation.md)` | kernel, ezsdr-sim, serde_json | kernel+testing |
   | `ezsdr-mock-radio` | `1.0.0` | `Ez-SDR v4 Module ezsdr.radio.mock 1.0.0 (plan/phase2/09-mock-radio.md)` | kernel, ezsdr-radio, ezsdr-sim, ezsdr-hostmem, serde, serde_json | kernel+testing |
   | `ezsdr-link-host` | `1.0.0` | `Ez-SDR v4 Module ezsdr.link.host 1.0.0 (plan/phase2/10-host-data-path.md)` | kernel | kernel+testing |
   | `ezsdr-sink-capture` | `1.0.0` | `Ez-SDR v4 Module ezsdr.sink.capture 1.0.0 (plan/phase2/10-host-data-path.md)` | kernel, ezsdr-sink, ezsdr-hostmem, serde_json | kernel+testing |
   | `ezsdr-acceptance` | `0.0.0` | `Ez-SDR v4 Phase 2 acceptance tests (plan/phase2/00-overview.md §8)` | every crate above, serde_json | kernel+testing |

   The dependency lines are written exactly like this (the versions are the ones already in `Cargo.lock`; do not change them):

   ```toml
   ezsdr-kernel = { path = "../ezsdr-kernel" }                           # "kernel"
   ezsdr-kernel = { path = "../ezsdr-kernel", features = ["testing"] }   # "kernel+testing", in [dev-dependencies]
   ezsdr-radio = { path = "../ezsdr-radio" }                             # and so on for each workspace crate
   serde = { version = "1.0.229", features = ["derive"] }
   serde_json = "1.0.151"
   schemars = "1.2.2"
   ```

   `ezsdr-acceptance` additionally has `publish = false` in `[package]`.

4. Every `src/lib.rs` starts with a crate doc comment naming its spec, then:

   ```rust
   #![forbid(unsafe_code)]
   #![warn(missing_docs)]
   ```

   and nothing else in this step. `ezsdr-acceptance/src/lib.rs` holds `//! Phase 2 acceptance tests (plan/phase2/00-overview.md §8). The library holds the experiments and the assembly helper; the tests are in tests/.`

**Done-list**

- [ ] `baseline_external_packages.txt` exists with 27 lines.
- [ ] `cargo +stable build --workspace` succeeds; `git diff Cargo.lock` shows only nine new `[[package]]` entries without a `source` line (the new members) and the `dependencies` lists that name them.
- [ ] The three commands of §0.5 pass (the test count is unchanged).

---

## Step 10 — `ezsdr-sim` and `ezsdr-sim-engine`

**Implements:** SE-1…SE-13. **Read first:** spec 08 in full.

### 10.1 `ezsdr-sim` (`crates/ezsdr-sim/src/lib.rs`)

```rust
/// SE-7.
pub const VIRTUAL_TICK_RATE_HZ: u64 = 1_000_000_000;
/// SE-7.
pub const VIRTUAL_EPOCH: &str = "sim.run_start";
/// SE-1: the Vocabulary's id, prefix and the two section names.
pub const VOCABULARY: &str = "sim";
pub const SEED_SECTION: &str = "sim.seed";
pub const FAULTS_SECTION: &str = "sim.faults";

/// SE-1.
pub fn vocabulary() -> VocabularyDescriptor;
/// SE-1: registers the descriptor, then `SeedCheck`, then `FaultsCheck`; the first error stops it.
pub fn register(registry: &mut ModuleRegistry, checks: &mut AdmissionCheckRegistry,
                _kinds: &mut EventKindRegistry) -> Result<(), ModuleError>;

/// SE-2: absent → 0; a JSON integer in 0..=u64::MAX → it; anything else → Err("SE-2: …").
pub fn seed(environment: &BTreeMap<Namespace, serde_json::Value>) -> Result<u64, String>;

/// SE-3.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind { RxOverflow, RxSequenceError, DeviceLost }

/// SE-3.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FaultEntry { pub at_ns: u64, pub fault: FaultKind, pub target: Ident }

/// SE-3: absent → []; an array of FaultEntry with every at_ns ≤ 2^62, in document order; anything else → Err("SE-3: …").
pub fn faults(environment: &BTreeMap<Namespace, serde_json::Value>) -> Result<Vec<FaultEntry>, String>;

/// SE-5.
pub struct SeedCheck;   pub struct FaultsCheck;       // both implement AdmissionCheck, stages [Validate]

/// SE-6.
#[derive(Clone, Debug)]
pub struct SimRng { state: u64 }
impl SimRng {
    pub fn new(seed: u64, stream: &str) -> SimRng;   // state = seed ^ fnv1a64(stream)
    pub fn next_u64(&mut self) -> u64;
    pub fn below(&mut self, n: u64) -> u64;          // next_u64() % n; returns 0 for n == 0 (never panics)
}

/// SE-12: `fault_entry` → FaultEntry, `seed` → u64, generated with `ezsdr_kernel::schema::generator()`.
pub fn document_schemas() -> BTreeMap<&'static str, serde_json::Value>;
```

Details:

- `vocabulary()`: `VocabularyDescriptor { id: sim, version: 1.0.0, prefix: sim, keys: vec![], event_kinds: vec![], verbs: vec![], checks: vec![sim.seed, sim.faults] }`.
- `register`: `registry.register_vocabulary(vocabulary())?`; `checks.register(Arc::new(SeedCheck))`; `checks.register(Arc::new(FaultsCheck))`. There are no kinds; the `_kinds` parameter exists so that every Vocabulary's `register` has one signature.
- `seed`: `environment.get(&Namespace::parse(SEED_SECTION))` → `None` → `Ok(0)`; `Some(v)` → `v.as_u64().ok_or_else(|| format!("SE-2: sim.seed must be an integer in 0..=2^64-1, not {v}"))`. `serde_json` reads `-1` as `i64` and `1.5` as `f64`, so `as_u64` refuses both; `"42"` is a string and is refused.
- `faults`: `None` → `Ok(vec![])`; `Some(v)` → `serde_json::from_value::<Vec<FaultEntry>>(v.clone())`, mapping the error to `"SE-3: {e}"`; then any `at_ns > 1 << 62` → `Err("SE-3: at_ns … exceeds 2^62")`. A negative `at_ns` fails to deserialise as `u64`.
- `SeedCheck::check`: `seed(&{section})` — build a one-entry map `{ sim.seed: section.clone() }` and call `seed` on it; an `Err(reason)` is one `Violation { check: sim.seed, key: None, requested: None, reason: format!("SE-5: {reason}") }`.
- `FaultsCheck::check`: `faults` the same way; an `Err` is one violation; otherwise one violation per entry whose `target` is not a key of `effective` (the per-fragment configuration): `reason: format!("SE-5: fault target {} names no fragment of this Run", e.target)`.
- `SimRng::new`: `fnv1a64(bytes)`: `let mut h: u64 = 0xcbf29ce484222325; for b in bytes { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); } h`. `next_u64`: `self.state = self.state.wrapping_add(0x9E3779B97F4A7C15); let mut z = self.state; z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9); z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB); z ^ (z >> 31)`. `below(n)`: `if n == 0 { 0 } else { self.next_u64() % n }`.

Tests in `crates/ezsdr-sim/tests/sim_vocabulary.rs`: the six rows of spec 08 §5's first table, with exactly those inputs. `se_06_splitmix_vectors` asserts the six hexadecimal values of SE-6 **literally** (they were computed from this algorithm; if yours differ, your algorithm is wrong — do not change the vectors). `se_12_schema_freeze` follows `crates/ezsdr-kernel/tests/schema_freeze.rs::ov_22_schema_freeze` exactly, with the directory `../../schemas/sim` and `ezsdr_sim::document_schemas()`; generate the files once with `EZSDR_UPDATE_SCHEMAS=1 cargo +stable test -p ezsdr-sim --test sim_vocabulary se_12_schema_freeze`, check them into `schemas/sim/`, and add one line to `schemas/SCHEMA_CHANGELOG.md`'s newest entry: "- New `sim/fault_entry` and `sim/seed` (Phase 2, SE-12)."

### 10.2 `ezsdr-sim-engine` (`crates/ezsdr-sim-engine/src/lib.rs`)

```rust
/// SE-11.
pub const CALLBACK_CAP: usize = 1000;
/// SE-8.
pub fn descriptor() -> ModuleDescriptor;
/// SE-9.
pub struct SimEngine { time: Arc<EngineTime>, descriptor: AuthorityDescriptor }
impl SimEngine {
    pub fn new(clocks: Arc<ClockRegistry>) -> Result<SimEngine, TimeError>;
    pub fn from_binding(binding: &Binding, clocks: Arc<ClockRegistry>) -> Result<SimEngine, ModuleError>;
    pub fn root(&self) -> ClockDomainId;
}
impl Authority for SimEngine { … }       // SE-9, SE-11

struct EngineTime {                          // private
    clocks: Arc<ClockRegistry>, root: ClockDomainId,
    state: Mutex<State>, woken: Condvar,
}
struct State { now: i64, pending: BTreeMap<(i64, u64), Box<dyn FnOnce(TimePoint) + Send>>, next_seq: u64 }
impl TimeAuthority for EngineTime { … }   // SE-10
```

- `descriptor()`: SE-8's literal, with `vocabularies: vec![VocabularyRequirement { id: sim, req: VersionReq(Version::new(1, 0, 0)) }]`, `deployment: Deployment::InProcess {}`, `impl_hash: Some(ContentHash::of_bytes(b"ezsdr.sim-engine 1.0.0"))`. It registers with `Factories { authority: true, ..Default::default() }`.
- `new`: `let root = clocks.allocate_id(); clocks.register(ClockDomain::root(root, Rational::new(VIRTUAL_TICK_RATE_HZ, 1)?, EpochRef::Arbitrary { set_by: VIRTUAL_EPOCH.to_owned() }))?`; descriptor `{ module: ModuleRef { id: ezsdr.sim-engine, version: 1.0.0 }, governs: vec![root, ClockDomainId::HOST_MONOTONIC], pacing: Pacing::FreeRunning }`.
- `from_binding`: refuse with `ModuleError::rejected("SE-9: …")` when `binding.module != descriptor's ModuleRef`, `!binding.selector.is_empty()`, or `binding.profile.is_some()`; then `new(clocks).map_err(|e| ModuleError::rejected(format!("SE-9: {e}")))`.
- `EngineTime` as SE-10 says, point by point:
  - `primary_root()` → `root`; `pacing()` → `FreeRunning`;
  - `governs(d)`: `d == root || d == HOST_MONOTONIC || clocks.get(d).map(|c| c.root_id() == root).unwrap_or(false)`;
  - `now(d)`: `root` → `TimePoint(root, now)`; `HOST_MONOTONIC` → `TimePoint(HOST_MONOTONIC, now)`; governed derived → `clocks.convert(TimePoint(root, now), d)?.floor()`; otherwise `Err(NotGoverned { id: d })`;
  - `schedule(t, f)`: not governed → `NotGoverned`; tick = `t.ticks` for `root` and `HOST_MONOTONIC`, and for a derived domain `match clocks.convert(t, root)? { Exact { point } => point.ticks, Inexact { floor, .. } => return Err(TimeError::Inexact { floor }) }`; `tick < now` → `Err(InPast { now: TimePoint(root, now), requested: t })`; insert under `(tick, next_seq)`, `next_seq += 1`; return `ScheduleHandle { root, ticks: tick, seq }`;
  - `cancel(h)` → `pending.remove(&(h.ticks, h.seq)).is_some()`;
  - `wait_until(t)`: not governed → `NotGoverned`; convert as `schedule` does (a derived instant rounds up instead of refusing); then `while state.now < tick { state = woken.wait(state) }`.
- `Authority::time()` returns `self.time.clone()` as `Arc<dyn TimeAuthority>`; `descriptor()` the descriptor; `next_wakeup()` exactly as SE-11 (the lock is **released** before each callback runs):

  ```rust
  fn next_wakeup(&self) -> Option<TimePoint> {
      let t = &self.time;
      let tick = { let mut s = lock(&t.state); let (&(tick, _), _) = s.pending.iter().next()?; s.now = tick; tick };
      for _ in 0..CALLBACK_CAP {
          let next = { let mut s = lock(&t.state);
                       match s.pending.keys().next().copied() { Some(k) if k.0 == tick => s.pending.remove(&k), _ => None } };
          let Some(callback) = next else { break };
          callback(TimePoint::new(t.root, tick));
      }
      t.woken.notify_all();
      Some(TimePoint::new(t.root, tick))
  }
  ```

Tests in `crates/ezsdr-sim-engine/tests/sim_engine.rs`: the seven rows of spec 08 §5's second table. For `se_10_time_authority_contract`'s "derived domain of ratio 3 at origin 1", register `ClockDomain::derived(id, root, Rational::new(3, 1), 1)` after `SimEngine::new`, advance the Engine to root tick 7 by scheduling a no-op at 7 and calling `next_wakeup()`, and assert `now(derived) == TimePoint(derived, 2)` (floor of (7 − 1)/3). Scheduling "at a derived tick between root ticks" is not possible with an integer ratio; use a second derived domain of ratio `2/3` (`Rational::new(2, 3)`), whose tick 1 is root tick `1 + 2/3`: `schedule` returns `Inexact`.

**Mutation checks (step 10):** SE-5's unknown-target violation (`se_05_checks`); SE-10's `InPast` (`se_10_time_authority_contract`); SE-11's cap (set it to `usize::MAX`: `se_11_next_wakeup_order_ties_and_cap`'s self-rescheduling case must then hang — run with `timeout 60`).

**Done-list:** both crates' tests pass; `schemas/sim/fault_entry.v1.json` and `schemas/sim/seed.v1.json` are committed; the kernel's `schema_freeze` still passes (a subdirectory is not a `.json` file); the three commands pass.

---

## Step 11 — `ezsdr-radio`

**Implements:** RM-1…RM-21 (RM-2, RM-3, RM-5…RM-9, RM-11, RM-13…RM-18 and RM-21 are producer obligations that MockRadio carries in step 14; this crate defines their vocabulary). **Read first:** spec 07 in full.

Public API (`crates/ezsdr-radio/src/lib.rs`):

```rust
/// RM-1.
pub const VOCABULARY: &str = "radio";
/// RM-2.
pub const DEVICE_KIND: &str = "radio.device";
pub const RX_STREAM_KIND: &str = "radio.rx_stream";
pub const TX_STREAM_KIND: &str = "radio.tx_stream";
/// RM-19.
pub const RF_ENVELOPE_SECTION: &str = "radio.rf_envelope";

/// RM-4: one constant per key, named after the key (`radio.rx.sample_rate_hz` → `RX_SAMPLE_RATE_HZ`).
pub mod keys {
    pub const RX_CHANNELS: &str = "radio.rx.channels";
    pub const TX_CHANNELS: &str = "radio.tx.channels";
    pub const RX_SAMPLE_RATE_HZ: &str = "radio.rx.sample_rate_hz";
    pub const TX_SAMPLE_RATE_HZ: &str = "radio.tx.sample_rate_hz";
    pub const RX_FREQUENCY_HZ: &str = "radio.rx.frequency_hz";
    pub const TX_FREQUENCY_HZ: &str = "radio.tx.frequency_hz";
    pub const RX_GAIN_DB: &str = "radio.rx.gain_db";
    pub const TX_GAIN_DB: &str = "radio.tx.gain_db";
    pub const RX_ANTENNA: &str = "radio.rx.antenna";
    pub const TX_ANTENNA: &str = "radio.tx.antenna";
    pub const RX_FREQUENCY_STEP_HZ: &str = "radio.rx.frequency_step_hz";
    pub const TX_FREQUENCY_STEP_HZ: &str = "radio.tx.frequency_step_hz";
    pub const RX_GAIN_STEP_DB: &str = "radio.rx.gain_step_db";
    pub const TX_GAIN_STEP_DB: &str = "radio.tx.gain_step_db";
    pub const RX_COHERENT: &str = "radio.rx.coherent";
    pub const FULL_DUPLEX: &str = "radio.full_duplex";
    pub const HARDWARE_TIME: &str = "radio.hardware_time";
    pub const PHASE_BEHAVIOR_ON_RETUNE: &str = "radio.phase_behavior_on_retune";
    pub const TX_REPEAT_MAX_SAMPLES: &str = "radio.tx.repeat_max_samples";
    pub const TX_REPEAT_ALIGN_SAMPLES: &str = "radio.tx.repeat_align_samples";
    pub const RX_BLOCK_LEN: &str = "radio.rx.block_len";
    pub const MIN_TIMED_COMMAND_LEAD_NS: &str = "radio.timing.min_timed_command_lead_ns";
    pub const STARTUP_LATENCY_NS: &str = "radio.timing.startup_latency_ns";
    pub const STOP_TAIL_NS: &str = "radio.timing.stop_tail_ns";
    pub const COMMAND_QUEUE_DEPTH: &str = "radio.timing.command_queue_depth";
    pub const OVERFLOW_RESTART_GAP_NS: &str = "radio.timing.overflow_restart_gap_ns";
    pub const RX_BYTES_PER_S: &str = "radio.perf.rx_bytes_per_s";
    pub const TX_BYTES_PER_S: &str = "radio.perf.tx_bytes_per_s";
    pub const WIRE_BYTES_PER_SAMPLE: &str = "radio.perf.wire_bytes_per_sample";
    /// RM-5: the ten configuration keys, in this order.
    pub const CONFIGURATION: [&str; 10] = [RX_CHANNELS, TX_CHANNELS, RX_SAMPLE_RATE_HZ, TX_SAMPLE_RATE_HZ,
        RX_FREQUENCY_HZ, TX_FREQUENCY_HZ, RX_GAIN_DB, TX_GAIN_DB, RX_ANTENNA, TX_ANTENNA];
}

/// RM-10: one constant per kind.
pub mod kinds {
    pub const RX_OVERFLOW: &str = "radio.RX_OVERFLOW";
    pub const TX_UNDERFLOW: &str = "radio.TX_UNDERFLOW";
    pub const TX_DISCONTINUITY: &str = "radio.TX_DISCONTINUITY";
    pub const TIME_ERROR: &str = "radio.TIME_ERROR";
    pub const LATE_COMMAND: &str = "radio.LATE_COMMAND";
    pub const ALIGNMENT_ERROR: &str = "radio.ALIGNMENT_ERROR";
    pub const CLOCK_LOST: &str = "radio.CLOCK_LOST";
    pub const COMMAND_QUEUE_FULL: &str = "radio.COMMAND_QUEUE_FULL";
    pub const COMMAND_REJECTED: &str = "radio.COMMAND_REJECTED";
}

/// RM-1.
pub fn vocabulary() -> VocabularyDescriptor;
/// RM-1: the descriptor, then RfEnvelopeCheck, then every RM-10 kind with owner Some(radio); the first error stops it.
pub fn register(registry: &mut ModuleRegistry, checks: &mut AdmissionCheckRegistry,
                kinds: &mut EventKindRegistry) -> Result<(), ModuleError>;

/// RM-19.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RfEnvelope {
    pub allowed_bands: Vec<Band>,
    #[serde(default)] pub max_gain_db: Option<f64>,
    #[serde(default)] pub tx_enabled: Option<TxEnabled>,
    #[serde(default)] pub antenna_ports: Option<Vec<String>>,
}
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Band { pub lo_hz: f64, pub hi_hz: f64 }
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum TxEnabled { All(bool), PerChannel(Vec<bool>) }
/// RM-19.
pub struct RfEnvelopeCheck;     // section radio.rf_envelope, stages [Validate, Prepare, Runtime]

/// RM-20.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RadioEnvelope { pub profile: ProfileRef, pub timing: TimingEnvelope, pub performance: PerformanceEnvelope }
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimingEnvelope { pub min_timed_command_lead_ns: i64, pub startup_latency_ns: i64, pub stop_tail_ns: i64,
                            pub command_queue_depth: i64, pub overflow_restart_gap_ns: i64 }
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerformanceEnvelope { pub rx_bytes_per_s: i64, pub tx_bytes_per_s: i64, pub wire_bytes_per_sample: i64 }

/// RM-22: the payload types, exactly as spec 07 RM-22 lists them, each with
/// #[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)] and
/// #[serde(deny_unknown_fields)] on the structs, #[serde(rename_all = "snake_case")] on the enums.
pub mod payloads { /* RxOverflowPayload, RxOverflowCause, TimeErrorPayload, TimeErrorCause, TimeErrorOutcome,
                      LateCommandPayload, CommandQueueFullPayload, CommandRejectedPayload */ }

/// RM-20, RM-22: "rf_envelope" → RfEnvelope, "envelope" → RadioEnvelope, "rx_overflow_payload",
/// "time_error_payload", "late_command_payload", "command_queue_full_payload", "command_rejected_payload".
pub fn document_schemas() -> BTreeMap<&'static str, serde_json::Value>;
```

`vocabulary()` builds `keys` from RM-4's table row by row: `kind` `int` → `ValueKind::Int`, `num` → `Num`, `str` → `Str`, `bool` → `Bool`; `coercible` as the table; `coercion_default` `reject` → `CoercionPolicy::Reject`, `warn` → `Warn`; `update_class` `cold` → `Some(UpdateClass::Cold)`, `hardware_timed` → `Some(HardwareTimed)`, `—` → `None`. `event_kinds` from RM-10: severity `warning` → `Severity::Warning`, `error` → `Error`, `fatal` → `Fatal`; default `mark_artifact` → `Reaction::MarkArtifact`, `abort` → `Reaction::Abort`. `verbs` from RM-12. `checks: vec![radio.rf_envelope]`.

`RfEnvelopeCheck::check(section, effective, proposed, stage)`, as RM-19 says, in this order:

```text
env = serde_json::from_value::<RfEnvelope>(section.clone())
    Err(e) → return [Violation { check: radio.rf_envelope, key: None, requested: None, reason: "RM-19: the section does not parse: <e>" }]
    any band with lo_hz > hi_hz → return [one Violation, key None, reason "RM-19: band <lo>..<hi> has lo_hz > hi_hz"]
out = []
for fragment F in (effective.keys() ∪ proposed.keys()), in BTreeSet order:
    cfg = effective[F] (or empty) overlaid with proposed[F] (or empty)
    num(key) = cfg[key] as f64 when it is Int or Num; Some(other kind) → push a violation on key, "RM-19: F: <key> is not a number"; absent → None
    tx = num(radio.tx.channels) as i64, absent → 0
    if tx > 0:
        freq = num(radio.tx.frequency_hz): Some(f) and no band with lo ≤ f ≤ hi → violation(key radio.tx.frequency_hz, requested cfg value, "RM-19: F: <f> Hz is in no allowed band")
        gain = num(radio.tx.gain_db): Some(g), max_gain_db Some(m), g > m → violation(radio.tx.gain_db, "RM-19: F: gain <g> dB exceeds <m>")
        tx_enabled: Some(All(false)) → violation(radio.tx.channels, "RM-19: F: transmission is disabled");
                    Some(PerChannel(v)) and some c in 0..tx with v.get(c) != Some(&true) → violation(radio.tx.channels, "RM-19: F: channel <c> is not enabled")
        antenna = cfg[radio.tx.antenna] as Str: Some(a), antenna_ports Some(p), a ∉ p → violation(radio.tx.antenna, "RM-19: F: antenna <a> is not allowed")
    rx antenna the same with radio.rx.antenna, whatever tx is
return out
```

Every violation's `check` is `radio.rf_envelope` and its `requested` is the value read from `cfg` for its key (`None` for the section-level ones).

Tests in `crates/ezsdr-radio/tests/radio_model.rs`: the ten rows of spec 07 §6. `rm_04_the_key_table_is_exactly_the_declared_one` writes RM-4's table as a literal array of `(key, ValueKind, coercible, CoercionPolicy, Option<UpdateClass>)` in the test and compares it with `vocabulary().keys` element by element, in order. `rm_20_schema_freeze` follows the kernel's freeze test with `../../schemas/radio`; generate and commit the seven files `schemas/radio/{rf_envelope, envelope, rx_overflow_payload, time_error_payload, late_command_payload, command_queue_full_payload, command_rejected_payload}.v1.json`, and add "- New `radio/rf_envelope`, `radio/envelope` and the five `radio/*_payload` schemas (Phase 2, RM-20, RM-22)." to the changelog's newest entry.

**Mutation checks (step 11):** each RM-19 violation kind in turn (frequency, gain, disabled channel, antenna tx, antenna rx, malformed section); `rm_19_rf_envelope_cases` or `rm_19_a_malformed_section_is_one_violation` must fail each time.

**Done-list:** the ten tests pass; the seven schemas are committed; the three commands pass.

---

## Step 12 — `ezsdr-hostmem`, `ezsdr-link-host`, `ezsdr-sink`

**Implements:** HD-1…HD-6. **Read first:** spec 10 §1–§4.

### 12.1 `ezsdr-hostmem`

```rust
/// HD-1.
pub const HOST_MEMORY: MemoryDomainId = MemoryDomainId::local(0);
/// HD-3: bytes per `ezsdr.stream.cf32` sample.
pub const CF32_BYTES: usize = 8;

/// HD-2.
pub struct HostPool { slot_bytes: usize, slots: Vec<Arc<[u8]>> }
impl HostPool {
    pub fn new(slot_bytes: usize) -> HostPool;
    pub fn fill(&mut self, len: usize, write: impl FnOnce(&mut [u8])) -> Arc<[u8]>;
    pub fn slots(&self) -> usize;
}
/// HD-3.
pub fn write_cf32(buf: &mut [u8], len: usize, channel: usize, index: usize, re: f32, im: f32);
pub fn read_cf32(buf: &[u8], len: usize, channel: usize, index: usize) -> (f32, f32);
pub fn interleave(block: &SampleBlock, bytes_per_sample: usize, from: usize, to: usize) -> Vec<u8>;
```

`fill`: if `len > slot_bytes`: `let mut v = vec![0u8; len]; write(&mut v); return v.into();`. Otherwise, for each slot in order, `if let Some(buf) = Arc::get_mut(slot) { write(&mut buf[..len]); return slot.clone(); }`; none free: `let mut v = vec![0u8; slot_bytes]; write(&mut v[..len]); let a: Arc<[u8]> = v.into(); self.slots.push(a.clone()); a`. `write_cf32`: `let o = (channel * len + index) * 8; buf[o..o+4].copy_from_slice(&re.to_le_bytes()); buf[o+4..o+8].copy_from_slice(&im.to_le_bytes());`. `interleave`: `let h = block.header(); let Some(bytes) = block.host_bytes() else { return vec![] }; if from > to || to > h.len as usize { return vec![] }`; then for `s in from..to`, for `c in 0..h.channels as usize`, append `bytes[(c * len + s) * bps .. + bps]` where `len = h.len as usize`.

Tests `crates/ezsdr-hostmem/tests/hostmem.rs`: `hd_02_…` and `hd_03_…` as spec 10 §7 says.

### 12.2 `ezsdr-link-host`

```rust
/// HD-4.
pub fn descriptor() -> ModuleDescriptor;
pub fn link_descriptor() -> LinkDescriptor;
/// HD-4.
pub struct HostLinkModule { descriptor: LinkDescriptor }
impl HostLinkModule { pub fn new() -> HostLinkModule; }
impl Link for HostLinkModule { fn descriptor(&self) -> &LinkDescriptor; fn create(&self, decl: &DataLinkDecl) -> Result<Arc<dyn DataLink>, ModuleError>; }
/// HD-5.
pub struct HostLink { policy: BackPressure, capacity: usize, queue: Mutex<VecDeque<BlockRef>>, drops: AtomicU64, carry: Mutex<DropCarry> }
impl DataLink for HostLink { … }
```

`descriptor()`: `{ id: ezsdr.link.host, version: 1.0.0, kernel_api: 4.0.0, roles: [Link], vocabularies: [], deployment: InProcess {}, impl_hash: Some(ContentHash::of_bytes(b"ezsdr.link.host 1.0.0")) }`, registered with `Factories { link: true, .. }` and `registry.register_link_descriptor(link_descriptor())`. `link_descriptor()`: `{ module: ezsdr.link.host 1.0.0, kind: ezsdr.link.host, connects: vec![(HOST_MEMORY, HOST_MEMORY)], policies: vec![Block, DropOldest, DropNewest], cross_process: false }` — `ezsdr-link-host` depends only on the Kernel, so write `MemoryDomainId::local(0)` with a comment `// HD-1's HOST_MEMORY; this crate may not depend on ezsdr-hostmem (00-overview.md §5)`. `create`: `decl.capacity == 0` → `Err(ModuleError::rejected("HD-4: a link needs a capacity of at least 1 (SC-19)"))`; else `Ok(Arc::new(HostLink::new(decl.policy, decl.capacity)))`. `HostLink`'s `DataLink` methods behave exactly as the kernel's test `MemLink` (`crates/ezsdr-kernel/tests/support/mod.rs`) does, with two differences: it never clamps a capacity (`create` refused 0), and it locks with `.lock().unwrap_or_else(|e| e.into_inner())` (§0.6 rule 7) where `MemLink` writes `.lock().expect("lock")`.

Tests `crates/ezsdr-link-host/tests/link_host.rs`: `hd_04_descriptor_and_create`; `hd_05_policies`, which copies the bodies of the kernel's `sc_20_*` and `sc_20b_*` tests from `crates/ezsdr-kernel/tests/stream_contract.rs`, replacing `MemLink::new(policy, capacity)` with `HostLink` built through `HostLinkModule::new().create(&decl)`. Copy the block helpers (`header`, `block`) into the test file; do not depend on the kernel's `tests/support`.

### 12.3 `ezsdr-sink`

```rust
/// HD-6.
pub const VOCABULARY: &str = "sink";
pub const CAPTURE_SAMPLES: &str = "sink.capture_samples";
pub const CAPTURE_ARTIFACT_KIND: &str = "sink.capture";
pub fn vocabulary() -> VocabularyDescriptor;
pub const REQUEST_REJECTED: &str = "sink.REQUEST_REJECTED";
pub fn register(registry: &mut ModuleRegistry, _checks: &mut AdmissionCheckRegistry,
                kinds: &mut EventKindRegistry) -> Result<(), ModuleError>;   // the descriptor, then its one kind, owner Some(sink)
/// HD-14.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestRejectedPayload { pub action: String, pub reason: String }
/// HD-14: "request_rejected_payload" → RequestRejectedPayload.
pub fn document_schemas() -> BTreeMap<&'static str, serde_json::Value>;
```

`vocabulary()`: HD-6's literal (`keys: [KeyDecl { key: sink.capture_samples, kind: Int, coercible: false, coercion_default: Reject, update_class: Some(BlockBoundary) }]`, `event_kinds: [EventKindDecl { kind: sink.REQUEST_REJECTED, default: Reaction::Continue, severity: Severity::Warning }]`, `verbs: [VerbDecl { verb: capture, compiles_to: UpdateParameter { key: sink.capture_samples, class: BlockBoundary } }]`, no checks). `register`: `registry.register_vocabulary(vocabulary())?`, then `kinds.register(Some(Namespace sink), decl)` for the one kind, mapping its `RunError` to `ModuleError::rejected(e.to_string())`.

Tests `crates/ezsdr-sink/tests/sink_vocabulary.rs`: `hd_14_schema_freeze` (the kernel's freeze test pattern with `../../schemas/sink`; commit `schemas/sink/request_rejected_payload.v1.json` and add "- New `sink/request_rejected_payload` (Phase 2, HD-14)." to the changelog's newest entry); `hd_06_vocabulary` as spec 10 §7 says, calling:

```rust
let action = SessionAction::Vocabulary {
    ns: Namespace::parse("sink").expect("a valid literal"),
    verb: Ident::parse("capture").expect("a valid literal"),
    target: ResourceId::parse("rec").expect("a valid literal"),
    at: None,
    params: [(Key::parse("sink.capture_samples").expect("a valid literal"), Value::Int(1000))].into_iter().collect(),
};
let sinks: BTreeSet<Ident> = [Ident::parse("rec").expect("a valid literal")].into_iter().collect();
let compiled = ezsdr_kernel::session::compile(&action, &registry, &BTreeMap::new(), &sinks,
    TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0), None).expect("compiles");
```

and asserting one `UpdateParameter` with `target == ResourceId::parse("sink/rec")`, `class == UpdateClass::BlockBoundary` and `value == Value::Int(1000)`.

**Mutation checks (step 12):** HD-4's capacity-0 refusal; HD-5's `Block` never discards (make `Full` drop the block: `hd_05_policies` fails).

**Done-list:** the three crates' tests pass; the three commands pass.

---

## Step 13 — `ezsdr-sink-capture`

**Implements:** HD-7…HD-13. **Read first:** spec 10 §5 in full, and the kernel's `stream/continuity.rs` (`ContinuityBuilder`).

Public API (`crates/ezsdr-sink-capture/src/lib.rs`):

```rust
/// HD-7.
pub fn descriptor() -> ModuleDescriptor;
pub fn sink_descriptor() -> SinkDescriptor;
/// HD-8…HD-13.
pub struct CaptureSink { /* private */ }
impl CaptureSink { pub fn from_binding(binding: &Binding) -> Result<CaptureSink, ModuleError>; }
impl Sink for CaptureSink { … }
```

`descriptor()` registers with `Factories { sink: true, .. }`; its `impl_hash` is `ContentHash::of_bytes(b"ezsdr.sink.capture 1.0.0")`.

Private state (a suggestion; any equivalent is fine as long as the behaviour is HD-10's):

```rust
struct Capture {
    id: Option<Ident>, n: u64, at: Option<AbsoluteDeadline>,   // the request; the own capture has Some(output id) from prepare
    written: u64, file: Option<(PathBuf, std::fs::File)>,
    bps: usize, contract: Option<DataContractId>, ext: &'static str,
    builders: Vec<ContinuityMap>,                           // finished maps
    builder: Option<(ContinuityBuilder, ClockDomainId, u16)>,
    started: bool, end: Option<TimePoint>,                  // end: the instant just past its last sample
}
pub struct CaptureSink {
    dir: PathBuf, output: Option<Ident>, run: Option<RunId>,
    clocks: Option<Arc<ClockRegistry>>, actions: Option<Arc<dyn ActionReceiver>>, link: Option<Arc<dyn DataLink>>,
    queue: VecDeque<Capture>,        // the own capture first (if any), then requests, in arrival order
    done: Vec<ArtifactRef>, requests: u32, last_end: Option<TimePoint>,
}
```

Algorithm of one delivered block `b` (after taking the link's `DropCarry`), in `step`:

```text
carry = link.take_drop_carry()                          -- discarded unless a capture was recording before this block
h = b.header()
bps = match h.contract.as_str() { "ezsdr.stream.cf32" => 8, "ezsdr.stream.sc16" => 4, _ => return Err(Rejected "HD-10: contract …") }
s = 0                                                    -- the first sample of b not yet consumed
while s < h.len:
    let Some(c) = queue.front_mut() else { discard carry; break }
    if !c.started:
        start_k = the first index k ≥ s of b whose instant is at or after max(c.at, last_end), in b's domain
                  (convert each bound with clocks.convert(bound, h.first_sample_time.domain): Exact → point.ticks,
                   Inexact → floor.ticks + 1); a sample k's index in the domain is
                   h.first_sample_time.ticks + k
                   -- an Err (the bound's root is not the block's) is HD-14: emit sink.REQUEST_REJECTED, pop the capture, continue
        if start_k ≥ h.len: break                        -- this block holds none of the capture
        s = start_k; c.started = true; if c.id is None { c.id = Some("<output>_<requests>"); requests += 1 }
        open the file (below) using bps; c.contract = Some(h.contract.clone())
    if c.contract != Some(h.contract): finish c with partial: true (below); continue   -- the block goes to the next capture
    take = min(h.len − s, c.n − c.written)
    push to c's builder the header of the overlap [s, s + take): first_sample_time = h.first_sample_time + s,
        len = take, flags and lost = h's when s == 0 else NONE / None, the rest from h; with the carry taken above
        (pass it once, to the first push of this block, and only if c was already recording before this block —
        a capture that starts in this block gets DropCarry::default(), because the drops the carry counts precede it, HD-10;
        later pushes pass DropCarry::default()).
        A push that returns Err((DomainChanged | ChannelsChanged, carry)): finish the builder with that carry into
        c.builders, start ContinuityBuilder::new(new domain, new channels, false), and push again.
    write interleave(b, bps, s, s + take) to the file
    c.written += take; s += take
    if c.written == c.n: finish c with partial: false; last_end = the instant just past its last sample
finish c: finish its builder into c.builders with DropCarry::default(); close the file; read the file back;
          push ArtifactRef { id: c.id (always Some: a capture is finished only after it started), kind: sink.capture, uri: format!("file://{}", absolute path), hash:
          ContentHash::of_bytes(&bytes), size_bytes: bytes.len(), partial, marks: [], continuity: c.builders };
          pop it from the queue
```

The file name is `<dir>/<sanitised run id>_<artifact id>.<ext>`: map every character of `run.as_str()` outside `[A-Za-z0-9_-]` to `_`; `ext` is `cf32` for 8 bytes per sample and `sc16` for 4. Open it with `std::fs::File::create`. Make the path absolute with `std::path::absolute` (stable since Rust 1.79, so available on 1.85). The directory is created in `prepare` (HD-9).

`step(until)`: first `while let Some(a) = actions.recv()`: `UpdateParameter { key: sink.capture_samples, value: Int(n), at, .. }` with `n ≥ 1` → push a request `Capture { id: None, n, at, .. }` — its id is given when it **starts** (`started` becomes true), as `"<output>_<requests>"` followed by `requests += 1`, so that a request HD-14 drops before it starts takes no `k` (HD-10); the same with a value that is not an `Int` ≥ 1 → HD-14's event; `Stop { .. }` → finish the recording capture (if started) with `partial: true`; anything else → HD-14's event. HD-14's event is `events.emit_control(Event { source: sink/<output>, time: time.now(time.primary_root()), severity: Warning, kind: sink.REQUEST_REJECTED, payload: serde_json::to_value(RequestRejectedPayload { action: <the Action's kind tag: tx_burst, set_timer, update_parameter, peripheral_command, emit, stop, abort>, reason: "HD-14: <why>" }) })`, and the request is dropped. An `at` that cannot be converted when the request starts (the block's root differs) is the same event, and the request is dropped. `step` returns `Err` only for a block of an unknown contract. Then receive every block and process it. `progressed` = an Action handled or a block received.

`prepare`: HD-9 exactly; keep `ctx.events` and `ctx.time` too (HD-14). The own capture: if `OutputReq.params` holds `sink.capture_samples: Int(n)` with `n ≥ 1`, push `Capture { id: Some(output id), n, at: None, .. }` first; any other key, or a non-Int, or `n < 1` → `Rejected`.

`stop(mode)`: HD-13: finish the recording capture, if started, with `partial: true`; drop the unstarted requests; return `done` (moved out). `cleanup`: drop the link and handles; close any file; idempotent.

Tests in `crates/ezsdr-sink-capture/tests/sink_capture.rs`, the rows of spec 10 §7's table. Build the blocks with `ezsdr_hostmem::HostPool`, `write_cf32` and `SampleBlock::new_host`, publish them on a local `DataLink` double (a copy of `MemLink` in the test file), and drive the Sink directly: `prepare` with a hand-built `PrepareContext` (a `ManualTimeAuthority` over a 1 GHz root; a registered SampleClock `dev/rx` at ratio 1000 with origin 0 — so sample k is root tick 1000·k), then `step` after each publish. Use a fresh directory per test: `std::env::temp_dir().join(format!("ezsdr-capture-{}-{}", std::process::id(), test_name))`, removed at the end of the test with `std::fs::remove_dir_all` (ignore its error). Assert file contents by reading the file and decoding with `read_cf32`-style little-endian parsing of the interleaved layout.

The table includes `hd_11_an_unexpected_action_is_rejected` and `hd_14_a_bad_capture_value_is_an_event_not_a_failure`; for them the test's `EventCollector` declares the pair `(sink/rec, sink.REQUEST_REJECTED)`.

**Mutation checks (step 13):** HD-9's "exactly one link" refusal; HD-14's event for an unexpected Action; HD-10's contract refusal; HD-14's event for `N < 1`; the carry pass (drop the carry: `hd_10_the_carry_attributes_a_dropped_overflow` fails).

**Done-list:** every test of spec 10 §7's capture table passes; no file is left in the temp directory after the tests; the three commands pass.

---

## Step 14 — `ezsdr-mock-radio`

**Implements:** MR-1…MR-30, and the producer halves of RM-2, RM-3, RM-5…RM-9, RM-11, RM-13…RM-18, RM-21, UC-2…UC-6 and SE-4. **Read first:** spec 09 in full, then spec 07 again, then the kernel's `stream/burst.rs` (`BurstTracker`, `LatePolicy::decide`, `admit_burst_target`) and `time/domain.rs` (`declare_sample_clock`, `register_sample_clock`, `end`, `convert`, `rescale`).

This is the largest step. Do it in the four sub-steps below, each ending with its tests green.

### The files

```text
crates/ezsdr-mock-radio/src/
  lib.rs       descriptor(), MockRadio and its Provider impl; the section records
  profile.rs   Profile: MR-3's values, the grids, the capability tree, fidelity, the envelope document
  coerce.rs    MR-6 as a pure function
  time.rs      tick arithmetic between the virtual root V and a SampleClock
  rx.rs        the receive stream: block planning, test pattern, faults, cold changes, the tail
  tx.rs        the transmit side: held bursts, block emission through BurstTracker, late policy
  device.rs    DeviceModel (MR-24), public
```

### 14.1 Tick arithmetic (`time.rs`)

Every instant the Mock computes is either a tick of the primary root `V` (an `i64`) or a sample index `k` of a SampleClock whose origin is the V tick `o` and whose ratio is `q = num/den` V ticks per sample. Write these four functions once and use nothing else:

```rust
/// The V tick of sample `k`, rounded up: o + ceil(k · num / den). i128 inside; None on overflow.
pub(crate) fn v_of(o: i64, q: Rational, k: i64) -> Option<i64>;
/// The first sample index whose instant is at or after V tick `t`: max(0, ceil((t − o) · den / num)).
pub(crate) fn k_at_or_after(o: i64, q: Rational, t: i64) -> Option<i64>;
/// `ns` nanoseconds in host.monotonic, rescaled to V and rounded up (clocks.rescale; Inexact → floor + 1).
pub(crate) fn ns_to_v(clocks: &ClockRegistry, v: ClockDomainId, ns: i64) -> Result<i64, TimeError>;
/// A TimePoint converted to V, rounded up; a TimePoint in V is returned unchanged.
pub(crate) fn to_v(clocks: &ClockRegistry, v: ClockDomainId, t: TimePoint) -> Result<i64, TimeError>;
/// A Duration in any domain, rescaled to host.monotonic nanoseconds and rounded up (for RM-11's `late_by_ns`).
pub(crate) fn to_ns(clocks: &ClockRegistry, d: Duration) -> Result<i64, TimeError>;
```

`ceil(a / b)` for `b > 0` in i128: `(a + b - 1).div_euclid(b)` when `a ≥ 0`, and `a.div_euclid(b) + if a.rem_euclid(b) != 0 { 1 } else { 0 }` in general — use the general form. Test these four in a unit test module `#[cfg(test)] mod tests` inside `time.rs`: `v_of(0, 5/1, 3) == 15`; `v_of(10, 1000/3, 1) == 344`; `k_at_or_after(0, 1000/3, 334) == 2` (sample 1 is at 333.3…, sample 2 at 666.6…); `k_at_or_after(100, 5/1, 0) == 0`.

### 14.2 Sub-step A — identity, profiles, the tree, `coerce`

**`profile.rs`.**

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileKind { X310Like, Ideal }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Profile { pub kind: ProfileKind, pub n: u32 }   // n = the selector's `instances`, 1…4
```

with one method per MR-3 row, returning exactly the table's value (`x310-like` | `ideal`):

| method | `x310-like` | `ideal` |
|---|---|---|
| `profile_ref()` | `ProfileRef { name: "x310-like", version: 1.0.0 }` | `"ideal"`, 1.0.0 |
| `rate_grid()` | `Grid::Values(v)`, `v` = `200e6 / N` for `N` = 512 down to 1, **ascending** | `Grid::Integer { lo: 1, hi: 1_000_000_000 }` |
| `decimation(rate)` | `Some(N)` with `200e6 / N == rate`, found by `N = (200e6 / rate).round()` and checked | `None` |
| `max_channels()` | `2 · n` | 64 |
| `freq_grid()` | `Grid::Step { lo: 10e6, hi: 6e9, step: 1.0 }` | `Grid::Step { lo: 0.0, hi: 1e12, step: 0.0 }` |
| `gain_grid()` | `Grid::Step { lo: 0.0, hi: 31.5, step: 0.5 }` | `Grid::Step { lo: -200.0, hi: 200.0, step: 0.0 }` |
| `rx_antennas()`, `tx_antennas()` | `["RX2", "TX/RX"]`, `["TX/RX"]` | the same |
| `phase_behavior()` | `"random_unless_timed_tune"` | `"deterministic"` |
| `repeat_max_samples()`, `repeat_align_samples()` | 268 435 456, 2 | 4 294 967 295 (`u32::MAX`: `BurstOpen.waveform_len` is a `u32`), 1 |
| `block_len()` | 2 000 | 2 000 |
| `timing()` | `TimingEnvelope { 2_000_000, 2_000_000_000, 1_000_000, 16, 50_000_000 }` | `{ 0, 0, 0, 4_294_967_295, 0 }` |
| `performance()` | `PerformanceEnvelope { 1_000_000_000 · n, 1_000_000_000 · n, 4 }` | `{ 2^62, 2^62, 8 }` |
| `fidelity()` | `Fidelity { timing: Envelope, continuity: Envelope, coercion: Grid, rf: None, transport: None }` | `Fidelity::NONE` |

`envelope()` returns `RadioEnvelope { profile: profile_ref(), timing: timing(), performance: performance() }`.

`Grid` (in `coerce.rs`):

```rust
pub(crate) enum Grid { Values(Vec<f64>), Integer { lo: i64, hi: i64 }, Step { lo: f64, hi: f64, step: f64 } }
impl Grid {
    fn min(&self) -> f64; fn max(&self) -> f64;
    fn contains(&self, v: f64) -> bool;          // on the grid and in range
    fn nearest(&self, v: f64) -> Option<f64>;    // RM-8 Eq: None when v < min or v > max; ties toward the lower
    fn at_or_above(&self, b: f64) -> Option<f64>;// RM-8 Min, and Range's lower bound
    fn at_or_below(&self, b: f64) -> Option<f64>;// RM-8 Max
}
```

- `Values(v)`: `nearest`: the element with the smallest `|x − v|`, the lower on a tie; `at_or_above`: the first `x ≥ b`; `at_or_below`: the last `x ≤ b`.
- `Integer { lo, hi }`: `nearest(v)`: `v` must be integral (`v.fract() == 0.0`) — otherwise `None` (MR-6 step 3's "whole hertz" refusal; see below for its reason text) — and in range; `at_or_above(b)` = `b.ceil()` if ≤ `hi` (and ≥ `lo`, else `lo`); `at_or_below(b)` = `b.floor()` if ≥ `lo`.
- `Step { lo, hi, step }` with `step == 0.0`: continuous, `nearest(v) = v` in range; with `step > 0`: the multiples of `step` in `[lo, hi]`; `nearest(v)`: `let f = (v / step).floor(); let r = v / step - f; let m = if r > 0.5 { f + 1.0 } else { f }; m * step`, then range-checked; `at_or_above(b) = (b / step).ceil() * step`, `at_or_below(b) = (b / step).floor() * step`, each range-checked.

**The tree** (`profile.rs`, `fn tree(&self, id: &ResourceId) -> Resource`): the device node `{ id, kind: radio.device, capabilities, children: [rx node, tx node], ports: [Port { name: rx, direction: Out, contract: ezsdr.stream.cf32 }], shareable: false }`; the children `{ id: <id>/rx, kind: radio.rx_stream }` and `{ id: <id>/tx, kind: radio.tx_stream }`, each with no capabilities, no ports, no children, not shareable. Capabilities, key by key (MR-4):

| keys | capability |
|---|---|
| `radio.{rx,tx}.channels` | `Range { Int(0), Int(max_channels) }` |
| `radio.{rx,tx}.sample_rate_hz` | `x310-like`: `AnyOf` of the 512 `Num(200e6 / N)` for `N` = 1…512 **in that order** (descending rate); `ideal`: `Range { Num(1.0), Num(1e9) }` |
| `radio.{rx,tx}.frequency_hz` | `Range { Num(lo), Num(hi) }` of `freq_grid()` |
| `radio.{rx,tx}.gain_db` | `Range { Num(lo), Num(hi) }` of `gain_grid()` |
| `radio.rx.antenna`, `radio.tx.antenna` | `AnyOf` of `Str` antennas |
| `radio.{rx,tx}.frequency_step_hz`, `radio.{rx,tx}.gain_step_db` | `One { Num(step) }` |
| `radio.rx.coherent`, `radio.full_duplex`, `radio.hardware_time` | `One { Bool(true) }` |
| `radio.phase_behavior_on_retune` | `One { Str(phase_behavior()) }` |
| `radio.tx.repeat_max_samples`, `radio.tx.repeat_align_samples`, `radio.rx.block_len` | `One { Int(..) }` |
| the five `radio.timing.*` and three `radio.perf.*` | `One { Int(..) }` |

**Identity** (`lib.rs`):

```rust
/// MR-1.
pub fn descriptor() -> ModuleDescriptor;       // registers with Factories { provider: true, .. }
/// MR-2, MR-4.
pub struct MockRadio { /* private */ }
impl MockRadio {
    pub fn from_binding(binding: &Binding) -> Result<MockRadio, ModuleError>;
}
```

`from_binding` refuses (`ModuleError::rejected`, reason beginning `"MR-2: "`): `binding.module != { ezsdr.radio.mock, 1.0.0 }`; `binding.profile` absent or not exactly `{ name: "x310-like", version: 1.0.0 }` or `{ name: "ideal", version: 1.0.0 }`; `binding.feed.is_some()`; a selector key outside MR-2's table; `id` not a `Str` that `ResourceId::parse` accepts as **one** segment (no `/`); `instances` not an `Int` in 1…4; `block_len_jitter` not a `Bool`; `rx_test_pattern` not `Str("zero")` or `Str("ramp")`; `arm_after` not a `List` of `Str`, each parsing as a `ResourceId`. It then builds `ProviderInstance { id, module, profile: binding.profile, tree, fidelity, driving: Driving { stepped: true }, arm_after, min_command_lead: (lead > 0).then(|| Duration::new(HOST_MONOTONIC, lead)), sections: { ezsdr.radio.mock.envelope: serde_json::to_value(envelope()), ezsdr.radio.mock.bursts: [], ezsdr.radio.mock.faults: [], ezsdr.radio.mock.rejected: [], ezsdr.radio.mock.stats: { rx_blocks: 0, rx_samples: 0, tx_blocks: 0 }, ezsdr.radio.mock.applied: [] } }` — all six MR-27 sections, from construction (MR-4, MR-27).

**`coerce`** (`coerce.rs`, `pub(crate) fn coerce(p: &Profile, device: &ResourceId, request: &Requested) -> Result<CoerceReport, ModuleError>`), MR-6 step by step:

```text
1. request.resource != device → Err(ModuleError::rejected("MR-6: the request names <r>, not this device"))
2. for (key, c) in request.constraints (BTreeMap order):
     key ∉ RM-4's 29 keys → rejected(key, c, "MR-6: not a radio key"); continue
     key is a capability key (not in keys::CONFIGURATION):
         cap = the device node's capability for key
         binding::satisfies(c, cap) == Ok(true) → applied[key] = the declared value (One's value)
         otherwise → rejected(key, c, "MR-6: the device declares <cap:?>")
     key is a configuration key:
         the numeric keys (rates, frequencies, gains) use their Grid; read an Int as the f64 it is; a Bool/Str → rejected
             Eq(v):      a = grid.nearest(v); None → rejected(… "RM-8: <v> is outside <min>..<max>")
                         (for `ideal`'s rate grid and a non-integral v: "MR-6: the ideal profile represents a rate as whole hertz")
                         applied = Num(a); if a != v: coercions.push(Coercion { key, requested: c's value, applied: Num(a), reason: "RM-8: nearest grid value" })
             Min(b):     grid.at_or_above(b); Max(b): grid.at_or_below(b); Range{min,max}: at_or_above(min or grid.min()) and ≤ max;
             Set(vs):    the first v of vs with grid.contains(v)
                         (none of these is a coercion; None → rejected(… "RM-8: no grid value satisfies <c:?>"))
             Present:    the MR-5 default
         the channel counts (Int): Eq(v) with v an Int in 0..=max_channels → Int(v); Min/Max/Range/Set on the integer range the
             same way; Present → the default; anything else → rejected(… "MR-6: …")
         the antennas (Str): Eq(Str(a)) with a in the list → Str(a); Set → the first member in the list; Present → the default;
             anything else → rejected(… "MR-6: <a> is not an antenna of this device")
3. RM-7, for d in [rx, tx]:
     ch = applied[radio.d.channels] as i64, else the MR-5 default;  rate = applied[radio.d.sample_rate_hz] as f64, else the default
     need = ch · rate · wire_bytes_per_sample (f64);  cap = d_bytes_per_s (f64)
     need > cap → key = radio.d.sample_rate_hz if the request names it, else radio.d.channels;
                  remove key from applied; rejected(key, the request's constraint for key — Present {} if the request does not name it,
                  "RM-7: <d> needs <need> B/s, the transport carries <cap>")
4. warnings = []; return CoerceReport { applied, coercions, warnings, rejected }
```

A `RejectedRequest` is `{ key, requested: the constraint, reason }`. With `x310-like` and MR-5's defaults the `radio.d.channels` branch of step 3 is not reachable (at the default 1 Msps a channel count alone needs at most 32 MB/s, at `instances: 4`); it is kept because RM-7 states it for every Provider.

**Tests (sub-step A)** in `crates/ezsdr-mock-radio/tests/mock_radio.rs`: `mr_01_descriptor_registers`, `mr_02_from_binding_refusals` (one refusal per MR-2 case, each asserting the `"MR-2: "` prefix), `mr_02_the_tree_has_the_radio_model_shape`, `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`, `mr_04_instance`, `mr_06_coerce_cases` (the cases listed in MR-6, except the unreachable channel-only overflow, which is replaced by "a request naming `radio.tx.channels: Eq 2` and `radio.tx.sample_rate_hz: Eq 200e6` is rejected on the rate"), `mr_06_coerce_is_pure`.

`Min(1.1e6) → 200e6/181`: the grid values around 1.1 MHz are 200e6/182 = 1 098 901.1 and 200e6/181 = 1 104 972.4; the smallest at or above 1.1e6 is 200e6/181. Assert `applied == Num(200e6 / 181.0)` exactly (the same f64 expression).

### 14.3 Sub-step B — `prepare`, `arm`, `start`, the receive stream

**The harness** (in the test file): a `ClockRegistry` in an `Arc`; a root `V` registered at 1 GHz; `ManualTimeAuthority::new(clocks, V, &[], Pacing::FreeRunning)` in an `Arc`; an `EventCollector` over pairs `(mock, k)`, `(mock/rx, k)`, `(mock/tx, k)` for every RM-10 kind `k` (`EventCollector::new(&pairs, &kinds, 4096, &policy)` with `policy = EventKindRegistry` compiled from `ezsdr_radio::register` output); a queue `ActionReceiver` (copy `QueueReceiver` into the test file); a refusing `ActionSubmitter`; a `MemLink` copy as the `rx` link. Build the `Fragment` by hand: `{ id: radio, instance: MR-1's ModuleRef, role: Provider, content: json!({ "selector": {}, "requested": Requested { resource: mock, constraints } }), after: [] }`. The harness steps the Mock with `auth.advance_to(t)` then `mock.step(t)`.

**`prepare(f, ctx)`** — MR-7's ten steps in order. Additional precision:

- step 1: keep a `prepared: bool`; a second call → `Rejected`.
- step 3: `serde_json::from_value::<Requested>(f.content["requested"].clone())`; `requested.resource` must equal the device id; `report = coerce(…)`; `!report.rejected.is_empty()` → `Rejected("MR-7: …")`.
- step 4: `config: BTreeMap<Key, Value>` = the ten MR-5 defaults, then every `report.applied` entry whose key is in `keys::CONFIGURATION`.
- step 5: `R = ctx.clocks.nominal_rate(V)`; `R.den() != 1` → `Unsupported`.
- step 6: for `d` in `rx`, `tx` with `channels_d > 0`: `ratio` = `x310-like`: `Rational::new(R.num() * N, 200_000_000)` with `N = profile.decimation(rate_d)`; `ideal`: `Rational::new(R.num(), rate_d as u64)` (the rate is integral, MR-6). `handle_d = ctx.clocks.declare_sample_clock(ResourceId::parse("<id>/<d>"), V, ratio)`; an `Err` → `Rejected("MR-7: …")`.
- step 7: every `ctx.links` entry must be `AttachedPort { component == f.id, port == "rx", endpoint: StreamOut(link) }`; when there is more than one, none may have `link.policy() == BackPressure::Block` (MR-7 step 7); keep the links.
- step 8: faults filtered by `target == f.id`, in document order; `rng = SimRng::new(seed, "<id>/rx")`.
- step 10: `PrepareReport { fragment: f.id, effective: config (all ten keys), coercions: report.coercions, warnings: [] }`.

**`arm()`** (MR-9): `A = time.now(V).ticks`; `S = A + ns_to_v(startup_latency_ns)`; if `tx.channels > 0`, `tx_domain = clocks.register_sample_clock(&tx_handle, A)`.

**`start(at)`** (MR-11): `T0 = at.map(|t| to_v(t)).unwrap_or(now)`. `T0 < S` → emit `radio.LATE_COMMAND { key: null, requested: TimePoint(V, T0), applied: TimePoint(V, S) }` with source `<id>` and time `TimePoint(V, now)`, and return `ModuleError::rejected(format!("MR-11: the start at {T0} precedes synchronisation at {S}; the profile needs ezsdr.time.start_lead_ns ≥ {startup_latency_ns}"))` (the spec's reason, verbatim, with the three numbers). Otherwise: if `rx.channels > 0` and at least one `rx` link: `rx_domain = clocks.register_sample_clock(&rx_handle, T0)`; the receive stream starts with `next = 0`. For each kept fault: `f = T0 + ns_to_v(at_ns)`. Then call `self.schedule_wakeup()` (below).

**The receive state** (`rx.rs`):

```rust
pub(crate) struct Rx {
    pub(crate) handle: SampleClockHandle, pub(crate) domain: ClockDomainId, pub(crate) origin: i64, pub(crate) ratio: Rational,
    pub(crate) channels: u16,
    pub(crate) next: i64,                 // the next sample index not yet published or skipped
    pub(crate) planned: Option<i64>,      // the length drawn for the block that starts at `next`
    pub(crate) flags: BlockFlags, pub(crate) lost: Option<u64>,   // for the block that starts at `next`
    pub(crate) end: Option<i64>,          // no sample at or after this index is delivered
}
```

**Pending events.** The Mock keeps one ordered list `events: BTreeMap<(i64, u64), Pending>` keyed by `(V tick, insertion counter)`:

```rust
enum Pending { Fault(usize /* index into faults */), Command(PendingUpdate), BurstStart(i64 /* transmit tick */) }
```

A **receive cut** is a pending `Fault` of kind `rx_overflow` or `rx_sequence_error`, or a pending `cold` `Command` on a receive key; a **transmit cut** is a pending `BurstStart`, or a pending `cold` `Command` on a transmit key. `hardware_timed` commands, and `device_lost`, cut nothing (spec 09 §4). Faults are inserted at `start`, commands and burst starts on receipt (sub-step C).

**`step(until)`** — spec 09 §4, made exact. `u = until.ticks` (a V tick).

```text
0. if device_lost fired and was reported: return Ok(progressed: false)
   if any Fault(DeviceLost) event has instant ≤ u and it is not yet reported: mark it reported; record the fault
      (MR-27 faults section: applied true, lost 0); return Err(ModuleError { kind: DeviceLost, message: "MR-20: device lost", detail: null })
1. handle every Action from actions.recv() at now = u (sub-step C); progressed = any handled
2. loop:
       emit_rx(u)                     -- up to the earliest pending receive cut
       emit_tx(u)                     -- up to the earliest pending transmit cut (sub-step C)
       E = the smallest key of `events`; if there is none or E's instant > u: break
       remove E and apply it (below); progressed = true
3. schedule_wakeup(u)
4. progressed |= any block published or emitted in this call; return Ok(progressed)
```

`emit_rx(u)`, while the receive stream is running (`rx` exists, its `domain` is registered and `end` has not been reached):

```text
loop:
    if planned is None: planned = Some(if jitter { 1 + rng.below(2 · block_len) } else { block_len })   -- MR-12's draw, when the block begins
    cut = k_at_or_after(origin, ratio, c) for c the instant of the earliest pending receive cut, or +∞ when there is none
    stop = min(next + planned, cut, end or +∞)
    if stop ≤ next: break                                        -- nothing to deliver before the cut
    if v_of(origin, ratio, stop − 1) > u: break                  -- its last sample has not occurred (MR-14)
    outcome = publish(next, stop)                                -- the block carries the current `flags` and `lost`
    if outcome == Full:                                          -- MR-19: a device overrun at the block's first sample
        a = next
        k_g = max(stop, a + ceil(ns_to_v(overflow_restart_gap_ns) · den / num))        -- exact: the first sample at or after a's instant plus the gap
        lost = self.lost.unwrap_or(0) + (k_g − a)                -- the whole time jump since the last delivered sample
        next = k_g; planned = None; flags = flags | GAP_BEFORE | RESTARTED; self.lost = Some(lost)
        emit RX_OVERFLOW { cause: overrun, lost: k_g − a, restart_gap_ns: overflow_restart_gap_ns } at TimePoint(domain, a)
        -- the dropped block is not counted in the stats
    else:
        rx_blocks += 1; rx_samples += stop − next; next = stop; planned = None; flags = NONE; self.lost = None
```

`publish(a, b)`: `len = b − a`; `bytes = pool.fill(channels · len · 8, |buf| pattern(buf, a, len))`; `header = BlockHeader { first_sample_time: TimePoint(domain, a), len, channels, direction: Rx, valid: ChannelMask::full(channels), flags, lost, contract: ezsdr.stream.cf32 }`; `block = BlockRef::new(SampleBlock::new_host(header, HOST_MEMORY, bytes, 8)?)`; publish the same `block` to every `rx` link and return `Full` if any link returned `Full` (only possible with a single `Block` link, MR-7), else `Accepted`. `pattern(buf, a, len)`: `zero` writes zeros explicitly over the first `channels · len · 8` bytes (a reused slot holds old bytes, MR-13); `ramp` writes `write_cf32(buf, len, c, i, ((a + i) % 65536) as f32 / 65536.0, c as f32 / 64.0)` for each `c`, `i`.

The pool is `HostPool::new(block_len · 2 · channels · 8)` (it must hold a jittered block of up to `2 · block_len` samples), created at `start`.

**Applying events** (MR-20…MR-22, RM-17, RM-18); `f` is the event's V tick:

- `Fault(RxOverflow)`: if the receive stream is not running, record `applied: false, lost: 0` and stop here. Otherwise `k_f = max(k_at_or_after(origin, ratio, f), next)` (it equals `next`: `emit_rx` cut the stream there); `k_g = max(k_at_or_after(origin, ratio, f + ns_to_v(overflow_restart_gap_ns)), k_f)`; `n = k_g − k_f`; `next = k_g`; `planned = None`; if `n > 0` { `flags = flags | GAP_BEFORE | RESTARTED`; `self.lost = Some(self.lost.unwrap_or(0) + n)` }; emit `radio.RX_OVERFLOW` with payload `RxOverflowPayload { cause: Overrun, lost: n, restart_gap_ns: overflow_restart_gap_ns }` (RM-22), source `<id>/rx` and time `TimePoint(domain, k_f)`; record `applied: true, lost: n`.
- `Fault(RxSequenceError)`: the same with `n = block_len`, `k_g = k_f + n`, flags `GAP_BEFORE | SEQ_DISCONTINUITY`, payload `{ cause: Sequence, lost: n, restart_gap_ns: 0 }`.
- `Fault(DeviceLost)`: handled in `step` 0; never reached here.
- `Command`, `BurstStart`: sub-step C.

Every event payload is built with `serde_json::to_value(&<the ezsdr_radio::payloads type>)` (RM-22), never with `json!`.

`schedule_wakeup(u)`: `w` is the minimum of: (receive) if the stream is running and `next < end`, `v_of(origin, ratio, min(next + planned, cut, end) − 1)`, drawing `planned` first if it is `None` (that is when this block begins); (transmit) if a burst is open, the V tick of its next block's last sample by `emit_tx`'s `stop` rule; and the first event's instant. Ignore any value `≤ u`. If the Mock already holds a scheduled wakeup at `w`, do nothing; if it holds one at another instant, `cancel` it; then `time.schedule(TimePoint(V, w), Box::new(|_| {}))` and keep `(w, handle)`. With nothing left, cancel the one it holds and schedule none. A callback scheduled twice at one instant would be fired twice by the Authority, and the stepping loop counts wakeups (KC-22).

**Tests (sub-step B):** `mr_07_prepare_cases`, `mr_07_prepare_reports_the_coercions_coerce_reported`, `mr_08_effective_holds_exactly_the_ten_configuration_keys`, `mr_11_start_cases`, `mr_12_block_lengths`, `mr_13_ramp_values`, `mr_14_blocks_appear_when_their_last_sample_has_occurred`, `mr_19_backpressure_is_an_overrun` (exactly spec 09 §6's fixture: `x310-like` at 1 Msps, one `Block` link of capacity 1, block 1 `Full`, `k_g = 52 000`, then drained; assert the received headers, `lost == Some(50 000)` equal to the time jump, one `RX_OVERFLOW`, `stats.rx_blocks == 2`), `mr_20_faults_fire_at_their_instants`, `mr_21_overrun_shape`, `mr_22_sequence_error_shape`, `mr_30_two_mocks_one_seed_identical_output` (compare the published headers, the delivered events and the sections of two Mocks prepared from one seed and driven by the same `advance_to` sequence).

For `mr_14`: 1 Msps with the `ideal` profile gives ratio 1 000 (1 GHz / 1 MHz); T0 = 0 with `startup_latency_ns` 0; block 0 is samples 0…1 999, whose last sample is at V tick 1 999 000, so stepping at 1 998 999 publishes nothing and stepping at 1 999 000 publishes it; the next wakeup is at 3 999 000.

For `mr_21`: use `x310-like` (the `ideal` profile's `overflow_restart_gap_ns` is 0) at 1 Msps, which is on its grid with `N = 200`, so the ratio is `5 · 200 = 1000`; `startup_latency_ns` is 2 s, so arm at 0 and start at `T0 = 2_000_000_000`; a fault `rx_overflow` at `at_ns = 1_000_000` fires at V tick `T0 + 1_000_000`; `k_f = 1000`; the gap is 50 ms = 50 000 000 V ticks = 50 000 samples; the next block starts at 51 000 with `GAP_BEFORE | RESTARTED` and `lost = Some(50000)`; `ContinuityBuilder::new(domain, 1, false)` fed the headers derives one `Gap { cause: OverflowRestart {}, lost: Some(50000), .. }`.

### 14.4 Sub-step C — transmit, updates, stop

**Actions** (step 1 of `step`, at `now = u`), MR-29 first: anything but `TxBurst`, `UpdateParameter` and `Stop` → emit `radio.COMMAND_REJECTED { action: <kind tag, e.g. "set_timer">, reason: "MR-29: …" }` with source `<id>`; record it in `ezsdr.radio.mock.rejected` as `{ action, reason, at: TimePoint(V, u) }`.

**`TxBurst`** (MR-16, MR-17, RM-13…RM-15):

```text
refuse (COMMAND_REJECTED + rejected record, reason "MR-16: …") when:
    target != <id>/tx; tx.channels == 0; at.time_point.domain != tx_domain;
    size_bytes == 0 or size_bytes % (8 · tx.channels) != 0;
    L = size_bytes / (8 · tx.channels);
    repeat and (L > repeat_max_samples or L % repeat_align_samples != 0);
    !metadata.is_empty();
now_tx = clocks.convert(TimePoint(V, u), tx_domain)?.floor()
outcome = late_policy.decide(&clocks, at.time_point, now_tx, Duration(HOST_MONOTONIC, min_timed_command_lead_ns))
    OnTime                 → start = at.ticks; requested = requested_at.map(|d| d.time_point)
    SendAsap { late_by }   → start = k_at_or_after(tx.origin, tx.ratio, u + ns_to_v(lead)); requested = requested_at or Some(at.time_point)
                             emit TIME_ERROR(TimeErrorPayload { cause: Late, outcome: SendAsap, late_by_ns, target: at.time_point })
    Drop { late_by }       → emit TIME_ERROR(… outcome: Drop …); record rejected; do not hold it
    PlanViolation {late_by}→ emit TIME_ERROR(… outcome: PlanViolation …); record rejected; do not hold it
    (source <id>/tx, time TimePoint(tx_domain, now_tx); late_by_ns = to_ns(&clocks, late_by))
refuse (COMMAND_REJECTED, reason "MR-16: …"), judged on the final `start`, when:
    v_of(tx.origin, tx.ratio, start) < S                          -- before synchronisation
    a held burst already has this start (RM-15)
    a burst is open and start < its `next`                        -- that burst has reserved those samples
hold Held { start, len: L, repeat, open: BurstOpen { waveform_len: Some(u32::try_from(L).expect("RM-13 caps L at repeat_max_samples ≤ u32::MAX")), late: Some(outcome) unless OnTime → None,
                                                      requested_target: requested } }
insert Pending::BurstStart(start) at V tick v_of(tx.origin, tx.ratio, start)
```

**The transmit side** (`tx.rs`). A transmit SampleClock is registered at `arm` (origin `A`) and at each `cold` transmit change (origin `e`); each registration creates a new `BurstTracker::new(tx_domain)` and `DeviceModel::new()` (MR-15). The Mock never calls `set_actual_start`: it has no device feedback, and `actual_start` stays absent (SC-28: "where the Provider can supply it"). At most one burst is open, `Open { start, len, repeat, next, first: bool, open: BurstOpen }`; the held ones are a `BTreeMap<i64 /*start*/, Held>`.

`emit_tx(u)`:

```text
loop while a burst is open:
    rep_end = if repeat { start + ((next − start) / len + 1) · len } else { start + len }   -- the end of this repetition
    cut = the first transmit sample at or after the earliest pending transmit cut's instant (a BurstStart's own tick), or +∞
    stop = min(next + block_len, rep_end, cut)
    if stop ≤ next: break
    if v_of(tx.origin, tx.ratio, stop − 1) > u: break
    ends_here = (!repeat and stop == start + len) or (the earliest transmit cut is a BurstStart and stop == its tick)
    flags = (first ? START_OF_BURST : NONE) | (ends_here ? END_OF_BURST : NONE)
    header = BlockHeader { first_sample_time: TimePoint(tx_domain, next), len: stop − next, channels: tx.channels,
                           direction: Tx, valid: full, flags, lost: None, contract: ezsdr.stream.cf32 }
    match tracker.on_block(&header, first.then_some(open.open)):
        Ok(Ended { record })                 → push record to the bursts section; ends_here = true
        Ok(Started) | Ok(Continued)          → nothing
        Ok(Discontinuity { closed, then_ended, .. }) → push closed (and then_ended); emit COMMAND_REJECTED { action: "tx_burst",
                                                   reason: "MR-15: the burst's blocks were not contiguous" }  -- a Mock defect if it happens
        Err(e)                               → emit COMMAND_REJECTED { action: "tx_burst", reason: "MR-15: <e>" };
                                               if let Some(r) = tracker.stop() { push r }; device.close(); open = None; break
    if device.on_tx_block(&header) is Err: emit TIME_ERROR(TimeErrorPayload { cause: UnclosedBurst, outcome: Refused,
                                               late_by_ns: 0, target: header.first_sample_time }); the block is dropped
    tx_blocks += 1; next = stop; first = false
    if ends_here: open = None
```

Applying `Pending::BurstStart(start)`: the open burst, if any, has already ended at `start` (`emit_tx` cut it there with `END_OF_BURST`); move the held burst at `start` into `open` with `next = start`, `first = true`.

**`UpdateParameter`** (MR-18, UC-2, UC-3, UC-6): target must be `<id>` and `key` in `keys::CONFIGURATION` except the antennas, else `COMMAND_REJECTED`. The envelope is checked **when the update applies** (MR-18): at `e`, re-run `coerce(profile, id, Requested { resource: id, constraints: Eq of every config value at `e`, with key = Eq(value) })`; a `rejected` entry → `COMMAND_REJECTED` (reason `"MR-18: …"`) and nothing changes. The effective instant `e` (V tick): `at.map(|d| to_v(d.time_point))`. An antenna update, or an `at` that `to_v` cannot convert, is `COMMAND_REJECTED` at receipt.

- `hardware_timed` (frequency, gain): `lead_v = ns_to_v(min_timed_command_lead_ns)`; `e = at or u + lead_v`; `e < u + lead_v` → emit `LATE_COMMAND(LateCommandPayload { key: Some(key), requested: TimePoint(V, e), applied: TimePoint(V, u + lead_v) })` and `e = u + lead_v`. If `command_queue_depth` `hardware_timed` commands are already pending → emit `COMMAND_QUEUE_FULL(CommandQueueFullPayload { key, depth })` and refuse. Otherwise insert `Pending::Command` at `e`. Applying it: `config[key] = value`; push `{ key, value, at: TimePoint(V, e) }` to `ezsdr.radio.mock.applied`.
- `cold` (channel counts, rates): `e = at or u`; `e < u` → emit `LATE_COMMAND` and `e = u`; for a receive key also `e = max(e, T0)` (MR-18). Insert `Pending::Command` at `e`. Applying it at `e`, after `emit_rx`/`emit_tx` cut the stream there: set the new config value and push the `applied` record; for a receive key — if the receive clock is registered and not ended, `clocks.end(rx_domain, TimePoint(V, e))`; if the new `radio.rx.channels > 0` and an `rx` link exists, declare a new clock for `<id>/rx` from the new rate (step 6's ratio), register it with origin `e`, and restart the stream with `next = 0`, `planned = None`, `flags = NONE`, `lost = None`, `end = None`; otherwise the stream stops. For a transmit key — `if let Some(r) = tracker.stop() { push r }`, `device.close()`, cancel every held burst of the old clock (`COMMAND_REJECTED`, reason `"MR-18: cancelled by a cold change"`) and remove their `BurstStart` events, `clocks.end(tx_domain, TimePoint(V, e))` if a transmit clock is registered and not ended, then with the new count > 0 declare and register a new transmit clock with origin `e` and create its tracker and device model.

Two updates at one instant apply in insertion order (the `u64` of the key), which is delivery order (UC-2).

**`Stop` Action** (MR-25): target `<id>/tx` → the transmit half; `<id>/rx` → the receive half with the tail; `<id>` → both; any other target → `COMMAND_REJECTED`.

**`stop(mode)`** (MR-25, RM-16), at `s = time.now(V).ticks`:

1. transmit half: `if let Some(r) = tracker.stop() { bursts.push(r) }`; `device.close()`; every held burst and every pending `Command` → a `rejected` record (reason `"MR-25: cancelled by stop"`), and their events removed; `open = None`;
2. receive half: `Orderly` → `end = Some(k_at_or_after(s + ns_to_v(stop_tail_ns)))`, i.e. the last delivered sample is the last one before `s + tail`; `Abort` → `end = Some(next)`;
3. cancel every pending `Fault` event and the scheduled wakeup; under `Orderly`, `schedule_wakeup(s)` so the drain delivers the tail (KA-12).

After `stop`, `step` keeps publishing receive blocks until `next == end`, then publishes nothing and schedules nothing.

**`cleanup()`** (MR-26): drop the links, the pool, every kept handle; idempotent.

**`instance().sections`** (MR-27): all six exist from construction (empty arrays, zero stats); keep them in the `ProviderInstance` and update them in place as the records change: `ezsdr.radio.mock.envelope` (from construction), `.bursts` (array of `BurstRecord`), `.faults` (array of `{ at, fault, applied, lost }`), `.rejected` (array of `{ action, reason, at }`), `.stats` (`{ rx_blocks, rx_samples, tx_blocks }`), `.applied` (array of `{ key, value, at }`). Write each with `serde_json::to_value`.

**`device.rs`** (MR-24):

```rust
/// MR-24: the device's own framing check.
#[derive(Default)]
pub struct DeviceModel { open: bool }
/// MR-24.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceTimeError;
impl DeviceModel {
    pub fn new() -> DeviceModel;
    /// Err when a START_OF_BURST block arrives while a burst is open.
    pub fn on_tx_block(&mut self, h: &BlockHeader) -> Result<(), DeviceTimeError>;
    /// Closes an open burst, as a zero-length end-of-burst send does.
    pub fn close(&mut self);
}
```

`on_tx_block`: `if h.flags.contains(START_OF_BURST) { if self.open { return Err(DeviceTimeError) } self.open = true }`; then `if h.flags.contains(END_OF_BURST) { self.open = false }`; `Ok(())`.

**Tests (sub-step C):** `mr_26_cleanup_is_idempotent`, `mr_29_other_actions_are_command_rejected`, `mr_18_a_cold_transmit_change_replaces_the_tracker`, `mr_18_a_scheduled_pair_is_checked_when_it_applies` (on `x310-like`, exactly spec 09 §6's fixture: on `ideal` nothing is ever over the transport, so the test would not discriminate), `mr_16_burst_refusals`, `mr_16_repeat_is_contiguous_across_wraps`, `mr_17_late_policy_outcomes`, `mr_18_hardware_timed_updates`, `mr_18_a_cold_rate_change_starts_a_new_sample_clock`, `mr_24_a_bypassing_provider_gets_time_error`, `mr_25_orderly_stop_delivers_the_tail_abort_does_not`.

For `mr_25_orderly_stop_delivers_the_tail_abort_does_not`: `x310-like` (its `stop_tail_ns` is 1 ms; `ideal`'s is 0) at 1 Msps, arm at 0, start at `T0 = 2_000_000_000`; `stop(Orderly)` at V tick `s`, a sample instant — step to block 0's last sample, `T0 + 1 999 000`, and stop there — puts `end` at the first sample at or after `s + 1 ms`, so the last delivered sample is the last one **before** `s + 1 ms`: at 1 Msps its instant is `s + 999 µs`. Under `Abort` no sample at or after `next` is delivered.

For `mr_16_repeat_is_contiguous_across_wraps`: `ideal` profile, tx 1 channel at 1 Msps (ratio 1 000), a repeated waveform of 1 000 samples (8 000 bytes); with `block_len` 2 000 every transmit block is exactly 1 000 samples, because a block never spans a wrap, and the blocks are contiguous (each starts where the previous ended). Step the Mock to the V instant of the burst's sample 4 999, then `stop(Orderly)`: `ezsdr.radio.mock.bursts` holds one record with `blocks == 5`, `samples == 5000`, `wraps == 5` (`BurstRecord.wraps` counts **complete** repetitions, `samples / L`, as the kernel's `sc_29a_wraps_counted_from_burst_open` shows) and `end == Stop`. For the v3 tail pattern, the test constructs a `BurstTracker` directly and feeds 300, 300, 300, 100, 300 as the kernel's `sc_26_burst_repeat_wrap_contiguous` does, asserting `Continued` for each; that pattern is a property of the tracker every Provider shares, and MockRadio's own blocks are cut at each wrap instead.

### 14.5 Sub-step D — the whole crate

Run every `mr_*` test of spec 09 §6. Compare the list with `grep -n "^fn mr_" crates/ezsdr-mock-radio/tests/mock_radio.rs`; every row of §6 must have a function of that exact name.

**Mutation checks (step 14):** MR-18's application-time check (check at receipt instead: `mr_18_a_scheduled_pair_is_checked_when_it_applies` fails); MR-2's unknown selector key; MR-6's out-of-range rejection (`7 GHz`); RM-7's PerformanceEnvelope rejection; MR-7's second-fragment refusal; MR-11's early-start refusal; MR-16's `metadata` refusal; MR-17's `Drop` (transmit it anyway: `mr_17` fails); MR-18's queue-full refusal; MR-21's `lost` (set it to `None`: `mr_21_overrun_shape` fails); MR-24's `Err` on a second start.

**Done-list:** every §6 test exists and passes; the Mock depends on no Module crate (`cargo tree -p ezsdr-mock-radio --edges normal,dev | grep ezsdr-` shows only `ezsdr-kernel`, `ezsdr-radio`, `ezsdr-sim`, `ezsdr-hostmem`); the three commands pass.

---

## Step 15 — `ezsdr-acceptance`

**Implements:** the Phase 2 carriers of Vision §58 and §61 (`00-overview.md` §8), PO-2, PO-4, PO-8 (MA-3), PO-11. **Read first:** `00-overview.md` §7 and §8.

### 15.1 The library

```text
crates/ezsdr-acceptance/src/
  lib.rs           pub mod experiments; pub mod rig;
  experiments.rs   the experiments: Spec documents and Session procedures, written against the Kernel only (§58 #10)
  rig.rs           the runtime: builds an Assembly from a BindingProfile document; profiles; the determinism projection
```

**`experiments.rs`** may `use` only `ezsdr_kernel`, `serde_json` and `std`, and must not contain the substrings `mock`, `Mock`, `x310`, `ideal`, `sim-engine`, `sim_engine` or `ezsdr_radio_mock` anywhere, comments included (`v58_10_experiments_name_no_mock_type` scans it). Everything that names a concrete Module lives in `rig.rs`. The experiments:

```rust
/// A receive-only experiment: resource `radio` (kind radio.device) requiring the given rx channel
/// count, sample rate and frequency; output `rec` (kind sink.capture) fed from `radio.rx` with
/// drop_oldest and capacity 64; `params` holding sink.capture_samples = `capture` when Some.
pub fn receive(channels: i64, rate_hz: f64, frequency_hz: f64, capture: Option<i64>) -> serde_json::Value;
/// `receive(..)` plus tx channels = 1 at `rate_hz` and tx frequency 1 GHz (`receive`'s rx frequency is its argument), and one schedule entry: a TxBurst to `radio/tx`
/// with the given waveform ref, `repeat`, `late_policy`, at SpecTime { clock: radio, offset_ticks }.
pub fn transmit(rate_hz: f64, waveform: &ArtifactRef, repeat: bool, late_policy: &str,
                offset_ticks: i64, capture: Option<i64>) -> serde_json::Value;
/// A Spec whose schedule also holds UpdateParameter { target: sink/rec, key: sink.capture_samples,
/// value: n, class: block_boundary } at SpecTime { clock: radio, offset_ticks }.
pub fn with_timed_capture(spec: serde_json::Value, n: i64, offset_ticks: i64) -> serde_json::Value;
/// A waveform of `samples` complex samples on one channel (8 bytes each, all zero) and its ArtifactRef
/// with uri "mem:<hash>" (the runtime supplies the bytes in Assembly.inputs, KC-9).
pub fn waveform(samples: usize) -> (Vec<u8>, ArtifactRef);   // ArtifactRef { id: "waveform", kind: ezsdr.input, uri: "mem:<hash>",
                                                             //   hash, size_bytes: 8·samples, partial: false, marks: [], continuity: [] }
```

Every Spec requires the Vocabularies `radio` (major 1) and `sink` (major 1) in `requirements.vocabularies`.

**`rig.rs`**:

```rust
/// A temporary directory for capture files, unique per test, removed on drop.
pub struct TempDir(pub PathBuf);
impl TempDir { pub fn new(test: &str) -> TempDir; }            // std::env::temp_dir()/ezsdr-acc-<pid>-<test>
impl Drop for TempDir { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

/// The profile every Spec-Run test starts from: `radio` bound to MockRadio with `profile`
/// ("x310-like" or "ideal") and `selector`; `rec` bound to the capture Sink with `{ dir }`;
/// `sim` bound to the Simulation Engine, which is the `authority`; one LinkPlacement
/// radio.rx → rec.in selecting ezsdr.link.host 1.0.0; environment
/// { ezsdr.time: { class: simulation, start_lead_ns: 2_000_000_000 } } plus `environment`.
pub fn spec_profile(profile: &str, selector: serde_json::Value, dir: &Path,
                    environment: serde_json::Value) -> serde_json::Value;
/// The same for a Session: `rec` carries `feed { port: radio.rx, policy: drop_oldest, capacity: 64 }`.
pub fn session_profile(profile: &str, selector: serde_json::Value, dir: &Path,
                       environment: serde_json::Value) -> serde_json::Value;

/// The runtime: registers the Vocabularies radio, sim and sink (with their checks and kinds),
/// the four Modules and the host Link descriptor; creates the ClockRegistry; builds the
/// Simulation Engine from the binding the profile's `authority` names, one MockRadio per binding
/// whose module is ezsdr.radio.mock (keyed by binding name), one CaptureSink per binding whose
/// module is ezsdr.sink.capture (keyed by binding name), one HostLinkModule per distinct
/// placements.links ModuleRef; contracts = ContractRegistry::with_standard_contracts(); host_clock = SystemHostClock::new(); inputs as given.
pub fn assemble(profile: &serde_json::Value, inputs: BTreeMap<ContentHash, Vec<u8>>) -> Assembly;

/// 00-overview.md §7: the Manifest as JSON with `run.id`, every `host_utc_nanos` (at any depth),
/// `artifacts[*].uri` and the top-level `hash` removed.
pub fn determinism_projection(m: &Manifest) -> serde_json::Value;

/// Reads a capture file (a `file://` uri) and decodes its interleaved cf32 samples as (re, im) per
/// channel: result[c][k].
pub fn read_capture(artifact: &ArtifactRef, channels: usize) -> Vec<Vec<(f32, f32)>>;
```

`assemble` registers in this order: `ezsdr_radio::register`, `ezsdr_sim::register`, `ezsdr_sink::register` (each into the same three registries), then `registry.register(ezsdr_sim_engine::descriptor(), Factories { authority: true, .. })`, `registry.register(ezsdr_mock_radio::descriptor(), Factories { provider: true, .. })`, `registry.register(ezsdr_link_host::descriptor(), Factories { link: true, .. })`, `registry.register_link_descriptor(ezsdr_link_host::link_descriptor())`, `registry.register(ezsdr_sink_capture::descriptor(), Factories { sink: true, .. })`. It parses the profile with `BindingProfile::from_json` to read its bindings (an error is a test bug: `expect`). The `EventKindRegistry` starts as `EventKindRegistry::with_kernel_kinds()`.

### 15.2 The tests

One file per group, all in `crates/ezsdr-acceptance/tests/`: `v58.rs`, `v61.rs`, `governance.rs`. Unless a row says otherwise: profile `x310-like`, selector `{ "id": "mock" }`, 1 Msps (ratio 1 000 on the 1 GHz root: sample `k` of a stream whose origin is `o` is at root tick `o + 1000·k`), frequency 1 GHz, arm at root tick 0, `T0 = 2 000 000 000` (2 s: `start_lead_ns` in the profile, which `x310-like`'s 2 s `startup_latency_ns` requires, MR-11), receive blocks of 2 000 samples (2 ms). A **Spec Run** row is run with `run_until_end(TimePoint(root, T0 + horizon))` and then `finish()`; `run_until_end` may run one round past the horizon (KC-29), so no assertion depends on what happens after it. A **Session** row starts with `advance_to(TimePoint(root, T0 + 1 ms))`, so that every submission happens after the receive stream started (MR-11) and after synchronisation (MR-16), and ends with `advance_to(TimePoint(root, T0 + horizon))` and `finish()`. `root` is the Engine's root, the domain of `run.now()`.

| test | kind | horizon (after T0) and why | fixture | assertion |
|---|---|---|---|---|
| `v58_01_one_spec_two_mock_profiles` | Spec | 20 ms: 10 000 samples are delivered by the block that ends at sample 9 999, at 9.999 ms | `receive(1, 1e6, 1e9, Some(10_000))` under `spec_profile("x310-like", …)` and under `spec_profile("ideal", …)` | both terminations are `Stopped { client }`; both have one artifact `rec` of 10 000 samples (`size_bytes == 80_000`), `partial == false`; the two Spec documents are identical |
| `v58_02_ten_virtual_seconds_run_faster_than_wall_clock` | Spec | 10 s | `receive(1, 1e6, 1e9, None)` | `run.now() ≥ TimePoint(root, T0 + 10 000 000 000)`; the wall time of `run_until_end`, measured with `std::time::Instant`, is below 10 s |
| `v58_03_same_seed_same_manifest_projection` | Spec, but ended with `advance_to(TimePoint(root, T0 + 25 ms))` then `finish()`, so that no round runs past 25 ms | 25 ms | selector `{ id: mock, block_len_jitter: true, rx_test_pattern: ramp }`, environment `{ sim.seed: 7 }`, `receive(1, 1e6, 1e9, Some(20_000))`; two Runs | the two Manifests' `determinism_projection`s are equal |
| `v58_03_the_seed_changes_block_boundaries_not_data` | as above | 25 ms | the same with seeds 7 and 8 | the artifacts' `hash`es are equal; `sections["ezsdr.radio.mock.stats"]["rx_blocks"]` is 12 for seed 7 and 11 for seed 8. `finish()` at `T0 + 25 ms` stops orderly, and the drain delivers the 1 ms tail up to sample 25 999 (MR-25), so the counted blocks are those ending by sample 26 000: the jittered lengths drawn from `SimRng::new(seed, "mock/rx")` put block ends at …, 21 783, 23 317 and the cut 26 000 for seed 7, and at …, 22 222, 25 207 and the cut 26 000 for seed 8 |
| `v58_04_mock_events_reach_counters_policy_and_manifest` | Spec | 160 ms: with 50 000 samples lost after sample 999, the 100 000th delivered sample is sample 149 999, in the block that ends at sample 150 999 (150.999 ms) | environment `{ sim.faults: [{ at_ns: 1000000, fault: rx_overflow, target: radio }] }`; capture 100 000 | the counter row `(mock/rx, radio.RX_OVERFLOW)` has count 1; one delivered `radio.RX_OVERFLOW` whose payload `cause == "overrun"`; the artifact `rec` carries one mark of kind `radio.RX_OVERFLOW`. With the Spec's `policies.failure: { radio.RX_OVERFLOW: stop }` the Run ends `Stopped { policy { radio.RX_OVERFLOW } }` and its transitions hold `Stopping { orderly }` |
| `v58_05_device_lost_aborts_with_full_cleanup` | Spec | 10 ms | `sim.faults: [{ at_ns: 5000000, fault: device_lost, target: radio }]`; capture 100 000. Blocks ending at samples 1 999 and 3 999 are delivered first; the step at 5 ms returns `DeviceLost` | `Stopped { policy { DEVICE_LOST } }`; the transitions hold `Stopping { abort }`; `cleanup_failures` is empty; the artifact `rec` exists with `partial == true` and `size_bytes == 32_000` (4 000 samples); the Manifest is sealed |
| `v58_06_injected_overflow_is_a_uhd_overflow` | Spec | 160 ms (as `v58_04`) | the `rx_overflow` fault at 1 ms; capture 100 000 | the capture's continuity has one `Gap` with `cause == OverflowRestart {}` and `lost == Some(50_000)`; the delivered `RX_OVERFLOW` payload is `{ cause: overrun, lost: 50000, restart_gap_ns: 50000000 }`; the file holds exactly 100 000 samples |
| `v58_06_sequence_error_is_seq_discontinuity` | Spec | 160 ms | `rx_sequence_error` at 1 ms; capture 100 000 | one `Gap` with `cause == SequenceError {}` and `lost == Some(2000)` |
| `v58_07_manifest_records_every_input_and_output` | Spec | 20 ms: the burst starts at 10 ms; the capture ends at 9.999 ms | `transmit(1e6, &w, true, "send_asap_and_flag", 10_000, Some(10_000))` with `(bytes, w) = waveform(1000)`, `bytes` supplied in `inputs` | `inputs == [w]`; one artifact `rec`; `modules` holds the four Modules; `vocabularies` holds `radio`, `sim` and `sink`; `clocks.sample_clocks` holds `mock/rx` and `mock/tx`; `sections` holds `ezsdr.links` and `ezsdr.radio.mock.envelope`, `.bursts`, `.faults`, `.rejected`, `.stats`, `.applied`; `spec.body` and `binding.body` equal the documents |
| `v58_10_experiments_name_no_mock_type` | text | — | the text of `src/experiments.rs` (`include_str!("../src/experiments.rs")`) | none of the forbidden substrings of §15.1 occurs; every `use` line starts with `use ezsdr_kernel`, `use serde_json` or `use std` |
| `v58_11_short_lead_burst_is_a_time_error` | Session | 10 ms | `connect(session_profile(…))`; submit `SetParameter { radio, radio.tx.channels, Int(1) }` (a `cold` change applied at once: the transmit clock is registered with origin `T0 + 1 ms`); then `Vocabulary { radio, start_repeat, radio/tx, Some(TimePoint(root, T0 + 2 ms)), {} }` with a 1 000-sample waveform. A Spec Run cannot make a burst late: its bursts are delivered at arm, 2 s ahead | one delivered `radio.TIME_ERROR` whose payload is `{ cause: late, outcome: send_asap, late_by_ns: 1000000, target: <the burst's transmit tick 1 000> }`; the burst record's `late_by` rescales to 1 ms |
| `v58_11_19_5_msps_is_coerced_to_20` | Spec | 1 ms | `receive(1, 19.5e6, 1e9, Some(1000))` with `policies.coercion: { radio.rx.sample_rate_hz: warn }` | `admission.coercions_preview` has 19.5e6 → 20e6; the prepare report's `effective["radio.rx.sample_rate_hz"] == Num(20e6)`; the `mock/rx` SampleClock's `nominal_rate == 20 000 000/1`. Without the coercion override the Run is `Failed { validate }` (the key's default is `reject`, RM-4) |
| `v58_11_beyond_the_performance_envelope_is_rejected_at_validate` | Spec | — | `receive(2, 200e6, 1e9, None)` | `Failed { validate }`; `admission.rejected` has an entry on `radio.rx.sample_rate_hz` whose reason starts with `"RM-7"` |
| `v58_12_jitter_leaves_the_capture_unchanged` | Spec | 20 ms: jittered blocks are at most 4 000 samples, so 12 345 samples are delivered by sample 16 344 | ramp pattern; `block_len_jitter` false and true (the only difference, and it is in the profile); capture 12 345 | the two artifacts have equal `hash` and equal `continuity` |
| `v58_13_session_manifest_has_log_waveform_and_capture` | Session | 20 ms | `connect(session_profile(…))`; submit `SetParameter { radio, radio.tx.channels, Int(1) }`; `Vocabulary { radio, start_repeat, radio/tx, None, {} }` with a 1 000-sample waveform; `Vocabulary { sink, capture, rec, None, { sink.capture_samples: 5000 } }` | three log entries, `seq` 0, 1, 2, all `Admitted`; `inputs` holds the waveform; the artifact `rec_0` holds 5 000 samples; `ezsdr.radio.mock.bursts` has one record with `end == Stop` |
| `v58_14_profiles_differing_only_in_environment_both_run` | Spec | 20 ms | the Spec of `v58_01` under two profiles that differ only in `environment`: `{ sim.seed: 1 }` and `{ sim.seed: 2, radio.rf_envelope: { allowed_bands: [{ lo_hz: 0, hi_hz: 7e9 }] } }` | both end `Stopped { client }` with an artifact; the Specs are identical |
| `v58_15_repeat_wraps_without_a_gap` | Spec | 15 ms: the burst starts at 10 ms and repeats 1 000-sample waveforms | `transmit(1e6, &w, true, "send_asap_and_flag", 10_000, None)` with `waveform(1000)` | one burst record with `wraps ≥ 4`, `samples ≥ 4000` and `end == Stop`; no `radio.TIME_ERROR`, `radio.TX_DISCONTINUITY` or `radio.TX_UNDERFLOW` delivered |
| `v58_16_runtime_retune_outside_the_rf_envelope_is_rejected` | Session | 5 ms | environment `{ radio.rf_envelope: { allowed_bands: [{ lo_hz: 2.4e9, hi_hz: 2.5e9 }] } }`; submit `SetParameter { radio.tx.frequency_hz: Num(2.45e9) }`, then `SetParameter { radio.tx.channels: Int(1) }`, then `SetParameter { radio.tx.frequency_hz: Num(2.6e9) }` | entries 0 and 1 `Admitted`; entry 2 `Rejected` with a violation of check `radio.rf_envelope` on `radio.tx.frequency_hz`; `effective()["radio"]["radio.tx.frequency_hz"] == Num(2.45e9)` |
| `v58_16_runtime_rate_beyond_the_envelope_is_rejected` | Session | 5 ms | `SetParameter { radio.rx.channels: Int(2) }`, then `SetParameter { radio.rx.sample_rate_hz: Num(200e6) }` | entry 1 `Rejected` with a violation of check `ezsdr.coercion` whose reason contains `"RM-7"`; `effective()["radio"]["radio.rx.sample_rate_hz"]` is unchanged |
| `v61_01_repeat_is_continuous_across_the_wrap` | Spec | 15 ms | as `v58_15`; the wrap arithmetic under test is that of `v3/source/device/package.d:111-126` | every transmit block the Mock emitted is 1 000 samples (`stats.tx_blocks · 1000 == record.samples`); the record's `blocks == stats.tx_blocks`; no `radio.TX_DISCONTINUITY` delivered |
| `v61_02_capture_starts_at_the_requested_sample_index` | Session | 50 ms | ramp pattern; after the initial `advance_to`, find the `mock/rx` record in `run.sample_clocks()` (its domain `D`) and submit `Vocabulary { sink, capture, rec, Some(TimePoint(D, 12_345)), { sink.capture_samples: 100 } }` | `read_capture(rec_0)[0][0].0 == 12345.0 / 65536.0` and the 100th sample's real part is `12444.0 / 65536.0` |
| `v61_03_timed_start_of_tx_and_capture` | Spec | 10 ms: both actions are at 5 ms | `with_timed_capture(transmit(1e6, &w, false, "drop_and_flag", 5_000, None), 1000, 5_000)` with `waveform(1000)` | the transmit clock's origin is the arm instant 0, so T0 is its tick 2 000 000 and the burst record's `target.ticks == 2_005_000`; the capture `rec_0`'s first continuity segment starts at receive tick 5 000 (the receive clock's origin is T0) |
| `v61_04_pps_source_is_armed_first_and_streams_align` | Spec | 1 ms | two resources `pps` and `follow`, each a `radio.device`, bound to two MockRadios with selectors `{ id: pps }` and `{ id: follow, arm_after: ["pps"] }`; two outputs, `rec_pps` fed from `pps.rx` and `rec` fed from `follow.rx` (each Mock registers its receive clock only when a link is attached to `rx`, MR-11); write this profile in the test | `plan.fragments`' ids put `pps` before `follow`; `plan.deps` contains `("pps", "follow")`; the `pps/rx` and `follow/rx` records in `clocks.sample_clocks` both have `origin == TimePoint(root, T0)` (`v3/changelog/v3.0.20.md:56-59`) |
| `po_02_every_crate_forbids_unsafe_code` | text | — | every `crates/*/src/lib.rs` (list the directory `crates/` from `env!("CARGO_MANIFEST_DIR")/..`) | each contains `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`; each `crates/*/Cargo.toml` contains `edition.workspace = true`, `rust-version.workspace = true` and `license.workspace = true` |
| `po_04_the_lock_gains_no_external_package` | text | — | `Cargo.lock` at the workspace root and `tests/baseline_external_packages.txt` | every `name version` of a `[[package]]` entry with a `source` line is in the baseline (the lock may lose a package, never gain one) |
| `ma_03_no_module_crate_depends_on_another` | tool | — | `cargo metadata --format-version 1 --no-deps` run with `std::process::Command::new(env!("CARGO"))` in the workspace root, parsed with `serde_json` | for each of `ezsdr-sim-engine`, `ezsdr-mock-radio`, `ezsdr-link-host`, `ezsdr-sink-capture`: no entry of its `dependencies` (any `kind`) has a `name` in that set |
| `po_11_no_hashmap_and_no_wall_clock_in_simulation_code` | text | — | every `.rs` file under the nine new crates' `src/` and under `crates/ezsdr-kernel/src/coordinator/` | no line, after removing everything from `//` on, contains `HashMap`, `HashSet`, `SystemTime`, `Instant::now`, `thread::spawn` or `rand::` |

**Mutation checks (step 15):** none new — the acceptance tests carry rules already mutation-checked in their own crates. Instead, confirm that `v58_10_experiments_name_no_mock_type` fails when you add a comment containing `mock` to `experiments.rs`, and that `ma_03_…` fails when you add `ezsdr-link-host` as a dev-dependency of `ezsdr-mock-radio` (then undo both).

**Done-list:** every test of the two tables of `00-overview.md` §8 that names a Phase 2 carrier exists with that exact name and passes; the three governance tests pass; the three commands pass.

**Then stop.** Report that step 15 is done and Review M (an adversarial review of the Modules and the acceptance tests) is due.

---

## Step 16 — Exit tables, Phase 1 markers, handoff

**Implements:** PO-10, exit criteria 2, 5, 7 and 8 of `00-overview.md` §10, KA-19's remaining markers.

1. **Exit-review tables (PO-10).** Create `plan/phase2/exit-review/README.md` explaining the format (copy the structure of `plan/phase1/exit-review/README.md`) and one table per spec, `06.md`, `07.md`, `08.md`, `09.md`, `10.md`, `00.md` (for the PO rules): one row per rule id, in rule order, with columns `rule | disposition | test(s) | read from`. The disposition is one of OV-3's (`default`, `producer`, `consumer`, `forward`, `process`, `withdrawn`); "read from" is the test file and function whose **body** proves the obligation — open each test and check that its assertion is the rule's obligation (OV-3). No row may be `GAP` or `UNCERTAIN`.
2. **Phase 1 markers.** For every row of `00-overview.md` §8's marker table that is not *re-marked*, replace the Phase 1 rule's italic marker in `design/0N-*.md` with `*Checked in Phase 2 by `<test>` (<carrier rule>).*` naming the carrier's test from Appendix B.
3. **The `NEW:` count.** Run `cargo +stable test -p ezsdr-kernel --test kernel_surface ov_23b -- --nocapture` and record the printed count in `handoff.md` §3.
4. **Links and `v3/` paths.** Run the link check of step 2 and the `v3/` path check of `handoff.md` §1; both must print nothing unexpected.
5. **Vision issues.** Collect the "Vision issues found" sections of specs 06–10 into `plan/phase2/vision-issues.md`, one numbered list, each item citing its spec and section. Do not edit the Vision (PO-9, OV-6).
6. **`handoff.md`.** Update §3 (crates, test counts per crate, toolchains, the `NEW:` count) and §4 (Phase 2 implemented, awaiting Gate X), in the file's existing Japanese style.

**Done-list:** the six items; the three commands pass. **Stop**: Gate X is the owner's.

---

## Appendix A — The Kernel API after step 1

Generated from the source of `crates/ezsdr-kernel/src/` with `plan/phase2/patches/01-kernel-amendments.patch` applied: every public type with its public fields, every public trait with its methods, and every public function and method signature. Derives, doc comments and bodies are omitted. `ManualTimeAuthority` (in `time/authority.rs`) exists only with the `testing` feature. Where this appendix and the source differ, the source is right (§0.8).

### `src/time/point.rs`

```rust
pub struct TimePoint {
    pub domain: ClockDomainId,
    pub ticks: i64,
}
pub struct Duration {
    pub domain: ClockDomainId,
    pub ticks: i64,
}
pub enum Converted {
    Exact {
        point: TimePoint,
    },
    Inexact {
        floor: TimePoint,
        remainder: Rational,
    },
}
impl Converted {
    pub fn floor(self) -> TimePoint;
    pub fn remainder(self) -> Option<Rational>;
}
pub enum Rescaled {
    Exact {
        duration: Duration,
    },
    Inexact {
        floor: Duration,
        remainder: Rational,
    },
}
impl Rescaled {
    pub fn floor(self) -> Duration;
    pub fn remainder(self) -> Option<Rational>;
}
impl TimePoint {
    pub const fn new(domain: ClockDomainId, ticks: i64) -> TimePoint;
    pub fn try_cmp(self, other: TimePoint) -> Result<Ordering, TimeError>;
    pub fn checked_add(self, d: Duration) -> Result<TimePoint, TimeError>;
    pub fn checked_sub_duration(self, d: Duration) -> Result<TimePoint, TimeError>;
    pub fn checked_sub(self, other: TimePoint) -> Result<Duration, TimeError>;
    pub fn ticks_in(self, domain: ClockDomainId) -> Result<i64, TimeError>;
}
impl Duration {
    pub const fn new(domain: ClockDomainId, ticks: i64) -> Duration;
    pub fn try_cmp(self, other: Duration) -> Result<Ordering, TimeError>;
    pub fn checked_add(self, other: Duration) -> Result<Duration, TimeError>;
    pub fn ticks_in(self, domain: ClockDomainId) -> Result<i64, TimeError>;
}
pub struct UncertainTimePoint {
    pub nominal: TimePoint,
    pub uncertainty: Duration,
}
pub struct RelativeBudget { /* private */ }
impl RelativeBudget {
    pub fn new(duration: Duration) -> Result<RelativeBudget, TimeError>;
    pub fn duration(self) -> Duration;
    pub fn deadline_from(self, arrival: TimePoint) -> Result<AbsoluteDeadline, TimeError>;
}
pub struct AbsoluteDeadline {
    pub time_point: TimePoint,
}
impl AbsoluteDeadline {
    pub const fn new(time_point: TimePoint) -> AbsoluteDeadline;
    pub fn remaining(self, now: TimePoint) -> Result<Duration, TimeError>;
}
pub struct ExactConversion { /* private */ }
impl ExactConversion {
    pub fn from(&self) -> ClockDomainId;
    pub fn to(&self) -> ClockDomainId;
    pub fn apply(&self, t: TimePoint) -> Result<Converted, TimeError>;
    pub fn try_exact(&self, t: TimePoint) -> Result<TimePoint, TimeError>;
}
```

### `src/time/rational.rs`

```rust
pub struct Rational { /* private */ }
impl Rational {
    pub const ONE: Rational = Rational { num: 1, den: 1 };
    pub fn new(num: u64, den: u64) -> Result<Rational, TimeError>;
    pub fn integer(n: u64) -> Result<Rational, TimeError>;
    pub fn num(self) -> u64;
    pub fn den(self) -> u64;
    pub fn recip(self) -> Rational;
    pub fn checked_mul(self, other: Rational) -> Result<Rational, TimeError>;
    pub fn checked_div(self, other: Rational) -> Result<Rational, TimeError>;
    pub fn exceeds(self, cap: u64) -> bool;
}
```

### `src/time/domain.rs`

```rust
pub const RATIO_TERM_CAP: u64 = 1 << 31;
pub enum EpochRef {
    Utc1970 {},
    Arbitrary {
        set_by: String,
    },
}
pub enum ClockDomainKind {
    Root {
        tick_rate: Rational,
        epoch: EpochRef,
    },
    Derived {
        root: ClockDomainId,
        root_ticks_per_tick: Rational,
        origin: i64,
    },
}
pub struct ClockDomain {
    pub id: ClockDomainId,
    pub kind: ClockDomainKind,
    pub ended_at: Option<TimePoint>,
}
impl ClockDomain {
    pub fn root(id: ClockDomainId, tick_rate: Rational, epoch: EpochRef) -> ClockDomain;
    pub fn derived( id: ClockDomainId, root: ClockDomainId, root_ticks_per_tick: Rational, origin: i64, ) -> ClockDomain;
    pub fn root_id(&self) -> ClockDomainId;
}
pub struct SampleClockRecord {
    pub stream: ResourceId,
    pub domain: ClockDomainId,
    pub root: ClockDomainId,
    pub root_ticks_per_tick: Rational,
    pub origin: TimePoint,
    pub ended_at: Option<TimePoint>,
    pub nominal_rate: Rational,
}
pub struct SampleClockHandle {
    pub id: ClockDomainId,
    pub root: ClockDomainId,
    pub root_ticks_per_tick: Rational,
    pub stream: ResourceId,
}
pub struct ClockRegistry { /* private */ }
impl ClockRegistry {
    pub fn new() -> ClockRegistry;
    pub fn allocate_id(&self) -> ClockDomainId;
    pub fn register(&self, domain: ClockDomain) -> Result<(), TimeError>;
    pub fn get(&self, id: ClockDomainId) -> Result<ClockDomain, TimeError>;
    pub fn is_registered(&self, id: ClockDomainId) -> bool;
    pub fn end(&self, id: ClockDomainId, at: TimePoint) -> Result<(), TimeError>;
    pub fn declare_sample_clock( &self, stream: ResourceId, root: ClockDomainId, root_ticks_per_tick: Rational, ) -> Result<SampleClockHandle, TimeError>;
    pub fn declared_sample_clocks(&self) -> Vec<SampleClockHandle>;
    pub fn domains(&self) -> Vec<ClockDomain>;
    pub fn register_sample_clock( &self, handle: &SampleClockHandle, origin: i64, ) -> Result<ClockDomainId, TimeError>;
    pub fn sample_clock_records(&self) -> Vec<SampleClockRecord>;
    pub fn nominal_rate(&self, id: ClockDomainId) -> Result<Rational, TimeError>;
    pub fn conversion( &self, from: ClockDomainId, to: ClockDomainId, ) -> Result<ExactConversion, TimeError>;
    pub fn convert(&self, t: TimePoint, to: ClockDomainId) -> Result<super::Converted, TimeError>;
    pub fn rescale(&self, d: Duration, to: ClockDomainId) -> Result<Rescaled, TimeError>;
    pub fn compare_durations(&self, a: Duration, b: Duration) -> Result<Ordering, TimeError>;
}
```

### `src/time/authority.rs`

```rust
pub struct ScheduleHandle {
    pub root: ClockDomainId,
    pub ticks: i64,
    pub seq: u64,
}
pub trait TimeAuthority: Send + Sync {
    fn primary_root(&self) -> ClockDomainId;

    fn pacing(&self) -> Pacing;

    fn governs(&self, domain: ClockDomainId) -> bool;

    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError>;

    fn wait_until(&self, t: TimePoint) -> Result<(), TimeError>;

    fn schedule(
        &self,
        t: TimePoint,
        f: Box<dyn FnOnce(TimePoint) + Send>,
    ) -> Result<ScheduleHandle, TimeError>;

    fn cancel(&self, h: ScheduleHandle) -> bool;
}
    pub struct ManualTimeAuthority {
        registry: Arc<ClockRegistry>,
        primary: ClockDomainId,
        pacing: Pacing,
        state: Mutex<State>,
        woken: Condvar,
        host_base: std::time::Instant,
        callback_cap: usize,
    }
impl ManualTimeAuthority {
    pub const DEFAULT_CALLBACK_CAP: usize = 1_000;
    pub fn new( registry: Arc<ClockRegistry>, primary: ClockDomainId, extra_roots: &[ClockDomainId], pacing: Pacing, ) -> Result<ManualTimeAuthority, TimeError>;
    pub fn with_callback_cap(mut self, cap: usize) -> ManualTimeAuthority;
    pub fn next_due(&self) -> Option<TimePoint>;
    pub fn registry(&self) -> &Arc<ClockRegistry>;
    pub fn advance_to(&self, t: TimePoint) -> Result<usize, TimeError>;
}
```

### `src/time/mod.rs`

```rust
pub enum TimeError {
    DomainMismatch {
        expected: ClockDomainId,
        found: ClockDomainId,
    },
    NotARoot {
        id: crate::id::ClockDomainId,
    },
    Unrelated {
        a: ClockDomainId,
        b: ClockDomainId,
    },
    Inexact {
        floor: TimePoint,
    },
    Overflow,
    UnknownDomain {
        id: ClockDomainId,
    },
    DuplicateDomain {
        id: ClockDomainId,
    },
    NotGoverned {
        id: ClockDomainId,
    },
    OutsideValidity {
        at: TimePoint,
    },
    InvalidRational,
    LimitExceeded,
    InPast {
        now: TimePoint,
        requested: TimePoint,
    },
    Stopped,
}
```

### `src/id.rs`

```rust
pub struct NodeId(pub u32);
impl NodeId {
    pub const LOCAL: NodeId = NodeId(0);
    pub fn is_local(self) -> bool;
}
pub struct ClockDomainId {
    pub node: NodeId,
    pub local: u32,
}
pub struct MemoryDomainId {
    pub node: NodeId,
    pub local: u32,
}
pub struct IslandId {
    pub node: NodeId,
    pub local: u32,
}
pub struct DataLinkId {
    pub node: NodeId,
    pub local: u32,
}
impl ClockDomainId {
    pub const fn local(local: u32) -> Self;
}
impl MemoryDomainId {
    pub const fn local(local: u32) -> Self;
}
impl IslandId {
    pub const fn local(local: u32) -> Self;
}
impl DataLinkId {
    pub const fn local(local: u32) -> Self;
}
impl ClockDomainId {
    pub const UTC: ClockDomainId = ClockDomainId::local(0);
    pub const HOST_MONOTONIC: ClockDomainId = ClockDomainId::local(1);
    pub const FIRST_ALLOCATABLE: u32 = 2;
}
pub struct ResourceId {
    pub node: NodeId,
    pub path: String,
}
pub enum ResourceIdError {
    EmptySegment,
    BadCharacter(char),
    TooLong,
}
impl ResourceId {
    pub const MAX_PATH_LEN: usize = 256;
    pub const MAX_DEPTH: usize = 16;
    pub fn parse(path: &str) -> Result<ResourceId, ResourceIdError>;
    pub fn segments(&self) -> impl Iterator<Item = &str>;
    pub fn parent(&self) -> Option<ResourceId>;
    pub fn is_within(&self, other: &ResourceId) -> bool;
    pub fn child(&self, segment: &str) -> Result<ResourceId, ResourceIdError>;
}
pub struct ModuleId(String);
impl ModuleId {
    pub fn parse(name: &str) -> Result<ModuleId, ResourceIdError>;
    pub fn as_str(&self) -> &str;
}
pub struct RunId(String);
impl RunId {
    pub fn generate() -> RunId;
    pub fn from_string(s: String) -> RunId;
    pub fn as_str(&self) -> &str;
}
```

### `src/hash.rs`

```rust
pub enum HashError {
    NonFiniteNumber,
    NonAsciiKey {
        key: String,
    },
    NotSerialisable {
        message: String,
    },
    MalformedHash,
}
pub struct ContentHash(String);
pub fn serialize_finite_f64<S: serde::Serializer>(x: &f64, s: S) -> Result<S::Ok, S::Error>;
impl ContentHash {
    pub fn of<T: Serialize>(value: &T) -> Result<ContentHash, HashError>;
    pub fn of_value(value: &serde_json::Value) -> Result<ContentHash, HashError>;
    pub fn of_bytes(bytes: &[u8]) -> ContentHash;
    pub fn parse(s: &str) -> Result<ContentHash, HashError>;
    pub fn as_str(&self) -> &str;
}
pub fn canonical_json(value: &serde_json::Value) -> Result<String, HashError>;
```

### `src/spec.rs`

```rust
pub struct Ident(String);
pub struct Namespace(String);
pub struct Key(String);
impl Ident {
    pub fn parse(s: &str) -> Result<Ident, SpecError>;
    pub fn as_str(&self) -> &str;
}
impl Namespace {
    pub fn parse(s: &str) -> Result<Namespace, SpecError>;
    pub fn as_str(&self) -> &str;
    pub fn is_under(&self, other: &Namespace) -> bool;
}
impl Key {
    pub fn parse(s: &str) -> Result<Key, SpecError>;
    pub fn as_str(&self) -> &str;
    pub fn has_prefix(&self, prefix: &Namespace) -> bool;
    pub fn is_extension(&self) -> bool;
}
pub enum Value {
    Bool(bool),
    Int(i64),
    Num(#[serde(serialize_with = "crate::hash::serialize_finite_f64")] f64),
    Str(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}
pub enum ValueKind {
    Bool,
    Int,
    Num,
    Str,
    List,
    Map,
}
impl Value {
    pub fn is_scalar(&self) -> bool;
    pub fn kind(&self) -> ValueKind;
    pub fn check_nesting(&self, path: &str) -> Result<(), SpecError>;
    pub fn partial_cmp_scalar(&self, other: &Value) -> Option<std::cmp::Ordering>;
}
pub enum Constraint {
    Eq {
        value: Value,
    },
    Range {
        min: Option<Value>,
        max: Option<Value>,
    },
    Set {
        values: Vec<Value>,
    },
    Min {
        value: Value,
    },
    Max {
        value: Value,
    },
    Present {},
}
impl Constraint {
    pub fn values(&self) -> Vec<&Value>;
}
pub enum CapabilityValue {
    One {
        value: Value,
    },
    Range {
        min: Value,
        max: Value,
    },
    AnyOf {
        values: Vec<Value>,
    },
}
pub struct KeyDecl {
    pub key: Key,
    pub kind: ValueKind,
    pub coercible: bool,
    pub coercion_default: CoercionPolicy,
    pub update_class: Option<crate::module_api::UpdateClass>,
}
pub enum CoercionPolicy {
    Accept,
    Warn,
    Reject,
}
pub struct Coercion {
    pub key: Key,
    pub requested: Value,
    pub applied: Value,
    pub reason: String,
}
pub struct Warning {
    pub source: Namespace,
    pub message: String,
}
pub struct RejectedConstraint {
    pub resource: Ident,
    pub key: Key,
    pub constraint: Constraint,
    pub reason: String,
}
pub struct SubResourceReq {
    pub kind: Namespace,
    pub requires: BTreeMap<Key, Constraint>,
}
pub struct ResourceReq {
    pub kind: Namespace,
    pub requires: BTreeMap<Key, Constraint>,
    pub needs: BTreeMap<Ident, SubResourceReq>,
    pub extensions: BTreeMap<Namespace, serde_json::Value>,
}
pub struct LinkReq {
    pub from: PortRef,
    pub to: PortRef,
    pub policy: BackPressure,
    pub capacity: u32,
}
pub struct SpecTime {
    pub clock: Ident,
    pub offset_ticks: i64,
}
pub struct SinkFeed {
    pub port: PortRef,
    pub policy: crate::stream::BackPressure,
    pub capacity: u32,
}
pub struct OutputReq {
    pub id: Ident,
    pub kind: Namespace,
    pub feed: SinkFeed,
    pub params: BTreeMap<Key, Value>,
}
pub struct Requirements {
    pub vocabularies: Vec<VocabularyReq>,
}
pub struct VocabularyReq {
    pub id: Namespace,
    pub major: u32,
}
pub struct SpecGraph {
    pub components: BTreeMap<Ident, ComponentDescriptor>,
    pub links: Vec<LinkReq>,
}
pub struct SpecPolicies {
    pub failure: BTreeMap<EventKind, Reaction>,
    pub coercion: BTreeMap<Key, CoercionPolicy>,
}
pub struct ScheduleEntry {
    pub at: SpecTime,
    pub action: crate::event::ActionTemplate,
}
pub struct ExperimentSpec {
    pub version: u32,
    pub requirements: Requirements,
    pub resources: BTreeMap<Ident, ResourceReq>,
    pub graph: SpecGraph,
    pub schedule: Vec<ScheduleEntry>,
    pub outputs: Vec<OutputReq>,
    pub policies: SpecPolicies,
    pub extensions: BTreeMap<Namespace, serde_json::Value>,
}
pub const SUPPORTED_VERSIONS: &[u32] = &[1];
pub const SPEC_TOP_LEVEL: &[&str] = &[ "version", "requirements", "resources", "graph", "schedule", "outputs", "policies", "extensions", ];
pub const PLACEMENT_FIELDS: &[&str] = &["placements", "placement", "executor", "memory_domain", "island", ];
pub const ENVIRONMENT_FIELD: &str = "environment";
pub enum SpecError {
    UnsupportedVersion {
        found: u32,
        supported: Vec<u32>,
    },
    UnknownField {
        path: String,
    },
    PlacementInSpec {
        path: String,
    },
    EnvironmentInSpec {
        path: String,
    },
    UnknownKeyPrefix {
        key: String,
    },
    KeyShape {
        key: String,
        expected: String,
        found: String,
    },
    UnboundResource {
        name: Ident,
    },
    NoSingleInstance {
        name: Ident,
        constraint: String,
    },
    DuplicateBindingName {
        name: Ident,
        sets: String,
    },
    WrongBindingRole {
        name: Ident,
        expected: String,
        module: String,
    },
    NodeAlreadyBound {
        node: String,
        first: Ident,
        second: Ident,
    },
    ArmCycle {
        path: String,
    },
    Violation(crate::binding::Violation),
    CoercionRejected(Coercion),
    Structural {
        reason: String,
    },
}
pub fn check_version(doc: &serde_json::Value) -> Result<u32, SpecError>;
pub fn check_top_level(doc: &serde_json::Value, allowed: &[&str]) -> Result<(), SpecError>;
pub fn check_no_placement(doc: &serde_json::Value) -> Result<(), SpecError>;
impl ExperimentSpec {
    pub fn from_json(doc: &serde_json::Value) -> Result<ExperimentSpec, SpecError>;
    pub fn check_key_prefixes(&self) -> Result<(), SpecError>;
}
pub type Migration = fn(serde_json::Value) -> Result<serde_json::Value, SpecError>;
pub struct MigrationRegistry { /* private */ }
impl MigrationRegistry {
    pub fn new() -> MigrationRegistry;
    pub fn register(&mut self, from: u32, step: Migration);
    pub fn migrate( &self, doc: serde_json::Value) -> Result<(serde_json::Value, u32), SpecError>;
}
```

### `src/binding.rs`

```rust
pub struct Binding {
    pub module: ModuleRef,
    pub selector: BTreeMap<Ident, Value>,
    pub profile: Option<ProfileRef>,
    pub feed: Option<crate::spec::SinkFeed>,
}
pub struct ComponentPlacement {
    pub island: Ident,
    pub memory_domain: MemoryDomainId,
}
pub struct LinkPlacement {
    pub link: ModuleRef,
    pub from: PortRef,
    pub to: PortRef,
}
pub struct Placements {
    pub islands: Vec<IslandDecl>,
    pub components: BTreeMap<Ident, ComponentPlacement>,
    pub links: Vec<LinkPlacement>,
}
pub struct BindingProfile {
    pub version: u32,
    pub bindings: BTreeMap<Ident, Binding>,
    pub authority: Ident,
    pub placements: Placements,
    pub environment: BTreeMap<Namespace, serde_json::Value>,
}
pub const BINDING_TOP_LEVEL: &[&str] = &["version", "bindings", "authority", "placements", "environment", ];
pub const KERNEL_SECTIONS: &[&str] = &["ezsdr.time", "ezsdr.rf_path", "ezsdr.capture", "ezsdr.arm_order", ];
impl BindingProfile {
    pub fn from_json(doc: &serde_json::Value) -> Result<BindingProfile, SpecError>;
    pub fn section(&self, ns: &str) -> Option<&serde_json::Value>;
}
pub fn start_lead_ns(environment: &BTreeMap<Namespace, serde_json::Value>) -> Result<u64, SpecError>;
pub fn satisfies(c: &Constraint, cap: &CapabilityValue) -> Result<bool, SpecError>;
pub fn check_constraint_kind(decl: &KeyDecl, c: &Constraint) -> Result<(), SpecError>;
pub enum CheckStage {
    Validate,
    Prepare,
    Runtime,
}
pub struct Violation {
    pub check: Namespace,
    pub key: Option<Key>,
    pub requested: Option<Value>,
    pub reason: String,
}
pub trait AdmissionCheck: Send + Sync {
    fn section(&self) -> &Namespace;
    fn stages(&self) -> &[CheckStage];
    fn check(
        &self,
        section: &serde_json::Value,
        effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        stage: CheckStage,
    ) -> Vec<Violation>;
}
pub struct AdmissionCheckRegistry { /* private */ }
impl AdmissionCheckRegistry {
    pub fn new() -> AdmissionCheckRegistry;
    pub fn register(&mut self, check: Arc<dyn AdmissionCheck>);
    pub fn run( &self, environment: &BTreeMap<Namespace, serde_json::Value>, effective: &BTreeMap<Ident, BTreeMap<Key, Value>>, proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>, stage: CheckStage, ) -> Vec<Violation>;
}
pub struct PreviewedCoercion {
    pub resource: Ident,
    pub coercion: Coercion,
}
pub struct AdmissionResult {
    pub matched: BTreeMap<Ident, ResourceId>,
    pub rejected: Vec<RejectedConstraint>,
    pub violations: Vec<Violation>,
    pub coercions_preview: Vec<PreviewedCoercion>,
    pub warnings: Vec<Warning>,
}
impl AdmissionResult {
    pub fn is_admitted(&self) -> bool;
    pub fn into_result(self) -> Result<AdmissionResult, SpecError>;
}
```

### `src/module_api.rs`

```rust
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}
impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version;
}
pub struct VersionReq(pub Version);
impl VersionReq {
    pub fn matches(self, v: Version) -> bool;
}
pub enum Role {
    Provider,
    Executor,
    Sink,
    Link,
    Authority,
}
impl Role {
    pub fn step_rank(self) -> u8;
}
pub enum Deployment {
    InProcess {},
    Plugin {
        protocol: Version,
    },
}
pub enum Pacing {
    FreeRunning,
    WallPaced,
    Device,
}
pub enum RfPath {
    Simulated,
    Cabled,
    OverTheAir,
}
pub enum ExecutionClass {
    Simulation,
    RealtimeEmulation,
    HardwareInLoop,
    Hardware,
}
impl ExecutionClass {
    pub fn derive(pacing: Pacing, rf: RfPath) -> Result<ExecutionClass, ModuleError>;
    pub fn may_claim_determinism(self) -> bool;
}
pub enum EnvelopeFidelity {
    None,
    Envelope,
    HardwareQuirk,
    Real,
}
pub enum CoercionFidelity {
    None,
    Grid,
    Real,
}
pub enum RfFidelity {
    None,
    ImpairmentModel,
    Real,
}
pub enum TransportFidelity {
    None,
    Model,
    Real,
}
pub struct Fidelity {
    pub timing: EnvelopeFidelity,
    pub continuity: EnvelopeFidelity,
    pub coercion: CoercionFidelity,
    pub rf: RfFidelity,
    pub transport: TransportFidelity,
}
impl Fidelity {
    pub const NONE: Fidelity = Fidelity;
    pub fn weakest(declared: &[Fidelity]) -> Fidelity;
}
pub enum UpdateClass {
    Cold,
    BlockBoundary,
    AtomicRealtime,
    HardwareTimed,
}
pub const UPDATE_CLASSES: &[UpdateClass] = &[ UpdateClass::Cold, UpdateClass::BlockBoundary, UpdateClass::AtomicRealtime, UpdateClass::HardwareTimed, ];
pub struct ParamDecl {
    pub key: Key,
    pub schema: serde_json::Value,
    pub update_class: UpdateClass,
    pub default: Value,
}
pub struct ComponentTiming {
    pub budget: Option<RelativeBudget>,
    pub preferred_batch: Option<u32>,
    pub stateful: bool,
    pub parallelism: Option<u32>,
}
pub struct ComponentRequires {
    pub executor_kind: Namespace,
    pub memory_bytes: Option<u64>,
}
pub struct ComponentImpl {
    pub kind: Namespace,
    pub id: String,
    pub hash: ContentHash,
}
pub struct ComponentDescriptor {
    pub id: Ident,
    pub kind: ComponentKind,
    pub ports: Vec<Port>,
    pub params: Vec<ParamDecl>,
    pub timing: ComponentTiming,
    pub requires: ComponentRequires,
    pub implementation: ComponentImpl,
}
pub enum ComponentKind {
    Processor,
    Reactor,
}
impl ComponentDescriptor {
    pub fn validate(&self, contracts: &[DataContractId]) -> Result<(), ModuleError>;
}
pub struct Resource {
    pub id: ResourceId,
    pub kind: Namespace,
    pub capabilities: BTreeMap<Key, CapabilityValue>,
    pub children: Vec<Resource>,
    pub ports: Vec<crate::contract::Port>,
    pub shareable: bool,
}
impl Resource {
    pub fn walk(&self) -> Vec<&Resource>;
}
pub struct ProviderInstance {
    pub id: ResourceId,
    pub module: ModuleRef,
    pub profile: Option<ProfileRef>,
    pub tree: Resource,
    pub fidelity: Fidelity,
    pub driving: Driving,
    pub arm_after: Vec<ResourceId>,
    pub min_command_lead: Option<crate::time::Duration>,
    pub sections: BTreeMap<Namespace, serde_json::Value>,
}
pub struct ModuleRef {
    pub id: ModuleId,
    pub version: Version,
}
pub struct ProfileRef {
    pub name: String,
    pub version: Version,
}
pub struct Driving {
    pub stepped: bool,
}
pub struct Requested {
    pub resource: ResourceId,
    pub constraints: BTreeMap<Key, Constraint>,
}
pub struct RejectedRequest {
    pub key: Key,
    pub requested: Constraint,
    pub reason: String,
}
pub struct CoerceReport {
    pub applied: BTreeMap<Key, Value>,
    pub coercions: Vec<Coercion>,
    pub warnings: Vec<Warning>,
    pub rejected: Vec<RejectedRequest>,
}
pub struct ExecutorDescriptor {
    pub module: ModuleRef,
    pub kind: Namespace,
    pub memory_domains: Vec<MemoryDomainId>,
    pub impl_kinds: Vec<Namespace>,
    pub capabilities: BTreeMap<Key, CapabilityValue>,
}
pub struct SinkDescriptor {
    pub module: ModuleRef,
    pub kind: Namespace,
    pub memory_domains: Vec<MemoryDomainId>,
    pub contracts: Vec<DataContractId>,
    pub artifact_kinds: Vec<Namespace>,
}
pub struct LinkDescriptor {
    pub module: ModuleRef,
    pub kind: Namespace,
    pub connects: Vec<(MemoryDomainId, MemoryDomainId)>,
    pub policies: Vec<BackPressure>,
    pub cross_process: bool,
}
pub struct AuthorityDescriptor {
    pub module: ModuleRef,
    pub governs: Vec<ClockDomainId>,
    pub pacing: Pacing,
}
pub struct ModuleDescriptor {
    pub id: ModuleId,
    pub version: Version,
    pub kernel_api: Version,
    pub roles: Vec<Role>,
    pub vocabularies: Vec<VocabularyRequirement>,
    pub deployment: Deployment,
    pub impl_hash: Option<ContentHash>,
}
pub struct VocabularyRequirement {
    pub id: Namespace,
    pub req: VersionReq,
}
pub struct VerbDecl {
    pub verb: Ident,
    pub compiles_to: CompileRule,
}
pub enum CompileRule {
    UpdateParameter {
        key: Key,
        class: UpdateClass,
    },
    TxBurst {
        repeat: bool,
        late_policy: crate::stream::LatePolicy,
    },
    PeripheralCommand {},
    None {},
}
pub struct VocabularyDescriptor {
    pub id: Namespace,
    pub version: Version,
    pub prefix: Namespace,
    pub keys: Vec<KeyDecl>,
    pub event_kinds: Vec<EventKindDecl>,
    pub verbs: Vec<VerbDecl>,
    pub checks: Vec<Namespace>,
}
pub struct IslandDecl {
    pub id: IslandId,
    pub executor: Ident,
    pub components: Vec<Ident>,
    pub affinity: Option<Vec<u32>>,
    pub rt_policy: Option<RtPolicy>,
    pub batch: Option<u32>,
}
pub struct RtPolicy {
    pub sched: String,
    pub priority: i32,
}
pub enum ModuleErrorKind {
    Rejected,
    Unsupported,
    Timeout,
    DeviceLost,
    Internal,
}
pub struct ModuleError {
    pub kind: ModuleErrorKind,
    pub message: String,
    pub detail: serde_json::Value,
}
impl ModuleError {
    pub fn rejected(message: impl Into<String>) -> ModuleError;
}
pub struct StepOutcome {
    pub progressed: bool,
}
pub enum Endpoint {
    StreamIn(Arc<dyn DataLink>),
    StreamOut(Arc<dyn DataLink>),
    EventIn,
    EventOut,
}
pub struct AttachedPort {
    pub component: Ident,
    pub port: Ident,
    pub endpoint: Endpoint,
}
pub trait ActionReceiver: Send + Sync {
    fn recv(&self) -> Option<Action>;
}
pub trait ActionSubmitter: Send + Sync {
    fn submit(&self, action: Action) -> Result<ActionId, Vec<Violation>>;
}
pub struct PrepareContext {
    pub run: RunId,
    pub class: ExecutionClass,
    pub time: Arc<dyn TimeAuthority>,
    pub clocks: Arc<crate::time::ClockRegistry>,
    pub events: Arc<dyn EventSink>,
    pub actions: Arc<dyn ActionReceiver>,
    pub actions_out: Arc<dyn ActionSubmitter>,
    pub environment: Arc<BTreeMap<Namespace, serde_json::Value>>,
    pub links: Vec<AttachedPort>,
    pub components: BTreeMap<Ident, ComponentDescriptor>,
    pub host_budget: RelativeBudget,
}
pub enum StopMode {
    Orderly,
    Abort,
}
pub trait Provider: Send {
    fn instance(&self) -> &ProviderInstance;
    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError>;
    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext,
    ) -> Result<PrepareReport, ModuleError>;
    fn arm(&mut self) -> Result<(), ModuleError>;
    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError>;
    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError>;
    fn cleanup(&mut self);
    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError>;   // default: Ok(StepOutcome { progressed: false })
}
pub trait Executor: Send {
    fn descriptor(&self) -> &ExecutorDescriptor;
    fn prepare(&mut self, island: &IslandDecl, ctx: PrepareContext,
    ) -> Result<PrepareReport, ModuleError>;
    fn arm(&mut self) -> Result<(), ModuleError>;
    fn start(&mut self) -> Result<(), ModuleError>;
    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError>;
    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError>;
    fn cleanup(&mut self);
}
pub trait Sink: Send {
    fn descriptor(&self) -> &SinkDescriptor;
    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext,
    ) -> Result<PrepareReport, ModuleError>;
    fn arm(&mut self) -> Result<(), ModuleError>;
    fn start(&mut self) -> Result<(), ModuleError>;
    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError>;
    fn stop(&mut self, mode: StopMode) -> Result<Vec<ArtifactRef>, ModuleError>;
    fn cleanup(&mut self);
}
pub trait Link: Send + Sync {
    fn descriptor(&self) -> &LinkDescriptor;
    fn create(&self, decl: &DataLinkDecl) -> Result<Arc<dyn DataLink>, ModuleError>;
}
pub trait Authority: Send + Sync {
    fn descriptor(&self) -> &AuthorityDescriptor;
    fn time(&self) -> Arc<dyn TimeAuthority>;
    fn next_wakeup(&self) -> Option<TimePoint>;
}
pub enum SteppedRef<'a> {
    Provider(&'a mut dyn Provider),
    Executor(&'a mut dyn Executor),
    Sink(&'a mut dyn Sink),
}
pub struct SteppedInstance<'a> {
    pub id: Ident,
    pub inner: SteppedRef<'a>,
}
pub const STEP_ROUND_CAP: usize = 1_000;
pub fn step_until_quiescent( instances: &mut [SteppedInstance<'_>], until: TimePoint, events: &dyn EventSink, coordinator: &ResourceId, ) -> Result<usize, ModuleError>;
pub struct Factories {
    pub provider: bool,
    pub executor: bool,
    pub sink: bool,
    pub link: bool,
    pub authority: bool,
}
impl Factories {
    pub fn has(self, role: Role) -> bool;
    pub fn roles(self) -> Vec<Role>;
}
pub const KERNEL_API: Version = Version::new(4, 0, 0);
pub struct ModuleRegistry { /* private */ }
impl ModuleRegistry {
    pub fn new() -> ModuleRegistry;
    pub fn register_vocabulary(&mut self, v: VocabularyDescriptor) -> Result<(), ModuleError>;
    pub fn register( &mut self, d: ModuleDescriptor, factories: Factories, ) -> Result<(), ModuleError>;
    pub fn register_link_descriptor(&mut self, descriptor: LinkDescriptor) -> Result<(), ModuleError>;
    pub fn vocabulary(&self, id: &Namespace) -> Option<&VocabularyDescriptor>;
    pub fn modules(&self) -> impl Iterator<Item = &ModuleDescriptor>;
    pub fn link_descriptor(&self, module: &ModuleRef) -> Option<&LinkDescriptor>;
    pub fn link_descriptors(&self) -> &BTreeMap<ModuleRef, LinkDescriptor>;
    pub fn key_decl(&self, key: &Key) -> Result<&KeyDecl, ModuleError>;
    pub fn verb(&self, ns: &Namespace, verb: &Ident) -> Option<&VerbDecl>;
}
```

### `src/event.rs`

```rust
pub enum Severity {
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}
pub struct EventKind(String);
impl EventKind {
    pub const EVENTS_DROPPED: &'static str = "EVENTS_DROPPED";
    pub const LINK_BACKPRESSURE: &'static str = "LINK_BACKPRESSURE";
    pub const PROCESSOR_DEADLINE_MISS: &'static str = "PROCESSOR_DEADLINE_MISS";
    pub const DEVICE_LOST: &'static str = "DEVICE_LOST";
    pub const STEP_LIVELOCK: &'static str = "STEP_LIVELOCK";
    pub fn parse(s: &str) -> Result<EventKind, RunError>;
    pub fn as_str(&self) -> &str;
    pub fn kernel_kinds() -> Vec<EventKind>;
}
pub struct Event {
    pub source: ResourceId,
    pub time: TimePoint,
    pub severity: Severity,
    pub kind: EventKind,
    pub payload: serde_json::Value,
}
pub const HOT_PAYLOAD_BYTES: usize = 32;
pub struct EventRecord {
    pub row: u32,
    pub kind: u32,
    pub time: TimePoint,
    pub severity: Severity,
    pub len: u8,
    pub payload: [u8; HOT_PAYLOAD_BYTES],
}
pub struct CounterRow {
    pub source: ResourceId,
    pub kind: EventKind,
    pub count: u64,
}
pub struct EventHandle {
    pub row: u32,
    pub kind: u32,
}
pub trait EventSink: Send + Sync {
    fn resolve(&self, source: &ResourceId, kind: &EventKind) -> EventHandle;
    fn emit(
        &self,
        handle: EventHandle,
        time: TimePoint,
        severity: Severity,
        payload: &[u8],
    ) -> Result<(), RunError>;
    fn emit_control(&self, event: Event) -> Result<(), RunError>;
}
pub struct EventCollector { /* private */ }
impl EventCollector {
    pub fn new( pairs: &[(ResourceId, EventKind)], kinds: &[EventKind], ring_depth: usize, policy: &Policy, ) -> EventCollector;
    pub fn counters(&self) -> Vec<CounterRow>;
    pub fn escalation(&self) -> Option<(EventKind, Reaction)>;
    pub fn drain(&self) -> Vec<Event>;
}
pub struct ActionId(pub u64);
pub enum Action {
    TxBurst {
        target: ResourceId,
        waveform: ArtifactRef,
        repeat: bool,
        at: AbsoluteDeadline,
        requested_at: Option<AbsoluteDeadline>,
        late_policy: LatePolicy,
        metadata: BTreeMap<Key, Value>,
    },
    SetTimer {
        target: ResourceId,
        at: AbsoluteDeadline,
        token: u64,
    },
    UpdateParameter {
        target: ResourceId,
        key: Key,
        value: Value,
        class: UpdateClass,
        at: Option<AbsoluteDeadline>,
    },
    PeripheralCommand {
        target: ResourceId,
        verb: Ident,
        params: BTreeMap<Key, Value>,
        at: Option<AbsoluteDeadline>,
    },
    Emit {
        target: ResourceId,
        event: Event,
    },
    Stop {
        target: Option<ResourceId>,
    },
    Abort {
        cause: StopCause,
    },
}
impl Action {
    pub const MEMBERS: usize = 7;
    pub fn target(&self) -> Option<&ResourceId>;
}
pub enum ActionTemplate {
    TxBurst {
        target: ResourceId,
        waveform: ArtifactRef,
        repeat: bool,
        late_policy: LatePolicy,
        metadata: BTreeMap<Key, Value>,
    },
    SetTimer {
        target: ResourceId,
        token: u64,
    },
    UpdateParameter {
        target: ResourceId,
        key: Key,
        value: Value,
        class: UpdateClass,
    },
    PeripheralCommand {
        target: ResourceId,
        verb: Ident,
        params: BTreeMap<Key, Value>,
    },
    Stop {
        target: Option<ResourceId>,
    },
}
impl ActionTemplate {
    pub fn resolve(self, at: AbsoluteDeadline) -> Action;
    pub fn target(&self) -> Option<&ResourceId>;
    pub fn is_timed(&self) -> bool;
}
```

### `src/policy.rs`

```rust
pub enum Reaction {
    Continue,
    MarkArtifact,
    Stop,
    Abort,
}
pub struct EventKindDecl {
    pub kind: EventKind,
    pub default: Reaction,
    pub severity: Severity,
}
pub struct EventKindRegistry { /* private */ }
impl EventKindRegistry {
    pub fn new() -> EventKindRegistry;
    pub fn with_kernel_kinds() -> EventKindRegistry;
    pub fn register( &mut self, owner: Option<Namespace>, decl: EventKindDecl, ) -> Result<(), RunError>;
    pub fn get(&self, kind: &EventKind) -> Option<&EventKindDecl>;
    pub fn require(&self, kind: &EventKind) -> Result<&EventKindDecl, RunError>;
    pub fn kinds(&self) -> Vec<EventKind>;
    pub fn compile( &self, overrides: &BTreeMap<EventKind, Reaction>) -> Result<Policy, RunError>;
}
pub struct Policy {
    pub table: BTreeMap<EventKind, Reaction>,
    pub severities: BTreeMap<EventKind, Severity>,
}
impl Policy {
    pub fn reaction_for(&self, kind: &EventKind) -> Reaction;
    pub fn reaction_for_event(&self, kind: &EventKind, severity: Severity) -> Reaction;
    pub fn by_severity(severity: Severity) -> Reaction;
}
```

### `src/run.rs`

```rust
pub enum CleanupMode {
    Orderly,
    Abort,
}
pub enum Stage {
    Validate,
    Plan,
    Prepare,
    Arm,
    Run,
}
pub enum StopCause {
    Client {},
    ClientDisconnect {},
    LeaseExpiry {},
    Policy {
        kind: EventKind,
    },
    Abort {
        cause: String,
    },
}
pub enum Termination {
    Completed {},
    Stopped {
        cause: StopCause,
    },
    Failed {
        stage: Stage,
    },
}
pub enum RunState {
    Created {},
    Validated {},
    Planned {},
    Prepared {},
    Armed {},
    Running {},
    Stopping {
        mode: CleanupMode,
    },
    CleanedUp {
        termination: Termination,
    },
}
impl RunState {
    pub fn can_move_to(&self, to: &RunState) -> bool;
}
pub enum RunError {
    StructuralMutationForbidden,
    RunNotRunning,
    LeaseTtlRequired,
    LeaseNotRenewable,
    AdoptRejected,
    UnknownEventKind {
        kind: String,
    },
    EventKindAlreadyRegistered {
        kind: String,
    },
    SectionKeyNotAscii {
        key: String,
    },
    BadEventHandle {
        row: u32,
    },
    SectionNamespaceForbidden {
        ns: String,
    },
    ReplayDivergence {
        field: String,
    },
    PayloadTooLarge,
    IllegalTransition {
        from: RunState,
        to: RunState,
    },
}
pub trait HostClock: Send + Sync {
    fn monotonic_millis(&self) -> u64;
    fn utc_nanos(&self) -> i64;
}
pub struct SystemHostClock { /* private */ }
impl SystemHostClock {
    pub fn new() -> SystemHostClock;
}
pub enum LeaseMode {
    Attached {},
    Detached {
        ttl_ms: u64,
        renewable: bool,
    },
}
pub struct Lease {
    pub mode: LeaseMode,
    pub token: Option<String>,
    pub holder: Option<String>,
    pub expires_at_host: Option<u64>,
    pub adoptions: u32,
    pub released: bool,
}
impl Lease {
    pub fn attached() -> Lease;
    pub fn detached( ttl_ms: u64, renewable: bool, token: impl Into<String>, clock: &dyn HostClock, ) -> Result<Lease, RunError>;
    pub fn validate(&self) -> Result<(), RunError>;
    pub fn on_disconnect(&mut self, clock: &dyn HostClock) -> Option<StopCause>;
    pub fn expired(&self, clock: &dyn HostClock) -> bool;
    pub fn adopt(&mut self, token: &str, holder: impl Into<String>) -> Result<(), RunError>;
    pub fn renew(&mut self, clock: &dyn HostClock) -> Result<(), RunError>;
}
pub enum CleanupStep {
    Children,
    FreezeDispatch,
    StopTx,
    StopRx,
    CancelPeripherals,
    RestoreBaseline,
    FinaliseArtifacts,
    FlushEvents,
    ReleaseAndWriteManifest,
}
pub const CLEANUP_STEPS: [CleanupStep; 9] = [ CleanupStep::Children, CleanupStep::FreezeDispatch, CleanupStep::StopTx, CleanupStep::StopRx, CleanupStep::CancelPeripherals, CleanupStep::RestoreBaseline, CleanupStep::FinaliseArtifacts, CleanupStep::FlushEvents, CleanupStep::ReleaseAndWriteManifest, ];
impl CleanupStep {
    pub fn is_per_fragment(self) -> bool;
}
pub struct CleanupFailure {
    pub step: CleanupStep,
    pub fragment: Option<Ident>,
    pub reason: String,
    pub timed_out: bool,
}
pub trait CleanupOps: Send + Sync {
    fn perform(
        &self,
        step: CleanupStep,
        fragment: Option<&Ident>,
        mode: CleanupMode,
    ) -> Result<(), ModuleError>;

    fn deadline_millis(&self, step: CleanupStep, fragment: Option<&Ident>) -> u64;   // default: DEFAULT_CLEANUP_DEADLINE_MS
}
pub const DEFAULT_CLEANUP_DEADLINE_MS: u64 = 5_000;
pub struct CleanupOutcome {
    pub failures: Vec<CleanupFailure>,
    pub modes: Vec<(CleanupStep, CleanupMode)>,
}
pub fn run_cleanup( ops: Arc<dyn CleanupOps>, reverse_order: &[Ident], mode: CleanupMode, escalate: &dyn Fn() -> Option<CleanupMode>, ) -> CleanupOutcome;
pub struct TransitionRecord {
    pub state: RunState,
    pub at: Option<TimePoint>,
    pub host_utc_nanos: i64,
}
pub struct RunStateMachine { /* private */ }
impl RunStateMachine {
    pub fn new(clock: &dyn HostClock) -> RunStateMachine;
    pub fn state(&self) -> &RunState;
    pub fn transitions(&self) -> &[TransitionRecord];
    pub fn move_to( &mut self, to: RunState, at: Option<TimePoint>, clock: &dyn HostClock, ) -> Result<(), RunError>;
    pub fn begin_stopping( &mut self, mode: CleanupMode, at: Option<TimePoint>, clock: &dyn HostClock, ) -> Result<(), RunError>;
    pub fn finish( &mut self, termination: Termination, at: Option<TimePoint>, clock: &dyn HostClock, ) -> Result<(), RunError>;
    pub fn check_structural_mutation(&self) -> Result<(), RunError>;
    pub fn check_running(&self) -> Result<(), RunError>;
}
```

### `src/session.rs`

```rust
pub enum SessionAction {
    SetParameter {
        target: ResourceId,
        key: Key,
        value: Value,
    },
    Vocabulary {
        ns: Namespace,
        verb: Ident,
        target: ResourceId,
        at: Option<TimePoint>,
        params: BTreeMap<Key, Value>,
    },
    Stop {
        target: Option<ResourceId>,
    },
    Release {},
    Adopt {
        token: String,
    },
    Renew {},
    RunChild {
        spec_hash: ContentHash,
        binding_hash: ContentHash,
    },
}
pub enum Outcome {
    Admitted {
        coercions: Vec<Coercion>,
        warnings: Vec<Warning>,
        dispatched: Vec<ActionId>,
    },
    Rejected {
        violations: Vec<Violation>,
    },
}
pub struct LogEntry {
    pub seq: u32,
    pub time: TimePoint,
    pub action: SessionAction,
    pub outcome: Outcome,
}
impl SessionAction {
    pub fn check_values(&self) -> Result<(), SpecError>;
}
pub struct SessionLog { /* private */ }
impl SessionLog {
    pub fn new() -> SessionLog;
    pub fn append( &mut self, time: TimePoint, action: SessionAction, outcome: Outcome, ) -> Result<u32, SpecError>;
    pub fn check_entry(&self, time: &TimePoint, action: &SessionAction) -> Result<(), SpecError>;
    pub fn entries(&self) -> &[LogEntry];
    pub fn admitted(&self) -> impl Iterator<Item = &LogEntry>;
    pub fn check_replay_target(&self, recorded: &ContentHash, target: &ContentHash, ) -> Result<(), RunError>;
}
pub struct Admitted {
    pub coercions: Vec<Coercion>,
    pub warnings: Vec<Warning>,
}
pub struct Admitter<'a> {
    pub checks: &'a AdmissionCheckRegistry,
    pub environment: &'a BTreeMap<Namespace, serde_json::Value>,
    pub declared_classes: &'a BTreeMap<Key, UpdateClass>,
    pub spec_coercion: &'a BTreeMap<Key, CoercionPolicy>,
    pub registry: &'a ModuleRegistry,
    pub is_session: bool,
}
impl Admitter<'_> {
    pub fn admit(&self, effective: &BTreeMap<Ident, BTreeMap<Key, Value>>, proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>, coercions: &[Coercion], stage: CheckStage) -> Result<Admitted, Vec<Violation>>;
}
pub struct Compiled {
    pub actions: Vec<Action>,
    pub control: Option<ControlOp>,
    pub coercions: Vec<Coercion>,
}
pub enum ControlOp {
    StopRun,
    Release,
    Adopt {
        token: String,
    },
    Renew,
    RunChild {
        spec_hash: ContentHash,
        binding_hash: ContentHash,
    },
}
pub fn compile( action: &SessionAction, registry: &ModuleRegistry, declared_classes: &BTreeMap<Key, UpdateClass>, sinks: &BTreeSet<Ident>, earliest: TimePoint, waveform: Option<crate::manifest::ArtifactRef>, ) -> Result<Compiled, Vec<Violation>>;
pub fn implicit_spec( profile: &BindingProfile, registry: &ModuleRegistry, providers: &BTreeMap<Ident, &dyn crate::module_api::Provider>, sinks: &BTreeMap<Ident, &dyn crate::module_api::Sink>, ) -> Result<ExperimentSpec, SpecError>;
```

### `src/manifest.rs`

```rust
pub struct ArtifactRef {
    pub id: Ident,
    pub kind: Namespace,
    pub uri: String,
    pub hash: ContentHash,
    pub size_bytes: u64,
    pub partial: bool,
    pub marks: Vec<ArtifactMark>,
    pub continuity: Vec<ContinuityMap>,
}
pub struct ArtifactMark {
    pub kind: EventKind,
    pub time: TimePoint,
}
pub struct RunSection {
    pub id: RunId,
    pub kind: RunKind,
    pub parent: Option<RunId>,
    pub execution_class: ExecutionClass,
    pub fidelity: Fidelity,
    pub transitions: Vec<TransitionRecord>,
    pub deterministic: bool,
}
pub enum RunKind {
    Spec,
    Session,
}
pub struct SpecSection {
    pub hash: ContentHash,
    pub body: serde_json::Value,
    pub original_version: Option<u32>,
    pub original_hash: Option<ContentHash>,
}
impl SpecSection {
    pub fn migrated( body: serde_json::Value, original_body: &serde_json::Value, ) -> Result<SpecSection, HashError>;
}
pub struct BindingSection {
    pub hash: ContentHash,
    pub body: serde_json::Value,
}
pub struct ModuleEntry {
    pub module: ModuleRef,
    pub impl_hash: Option<ContentHash>,
    pub profile: Option<ProfileRef>,
}
pub struct ClocksSection {
    pub domains: Vec<ClockDomain>,
    pub relations: Vec<ClockRelation>,
    pub sample_clocks: Vec<SampleClockRecord>,
}
pub struct EventsSection {
    pub counters: Vec<CounterRow>,
    pub delivered: Vec<Event>,
}
pub struct TerminationSection {
    pub reason: Termination,
    pub at: Option<TimePoint>,
    pub host_utc_nanos: i64,
    pub cleanup_failures: Vec<CleanupFailure>,
    pub also: Vec<StopCause>,
}
pub struct PrepareSection {
    pub reports: Vec<PrepareReport>,
    pub merged_effective: BTreeMap<crate::spec::Key, crate::spec::Value>,
}
pub struct Manifest {
    pub version: u32,
    pub run: RunSection,
    pub policy: Option<Policy>,
    pub spec: SpecSection,
    pub binding: BindingSection,
    pub plan: Option<ExecutionPlan>,
    pub prepare: PrepareSection,
    pub admission: AdmissionResult,
    pub modules: Vec<ModuleEntry>,
    pub vocabularies: BTreeMap<Namespace, Version>,
    pub components: BTreeMap<Ident, ContentHash>,
    pub inputs: Vec<ArtifactRef>,
    pub clocks: ClocksSection,
    pub events: EventsSection,
    pub lease: Lease,
    pub action_log: Vec<crate::session::LogEntry>,
    pub termination: TerminationSection,
    pub artifacts: Vec<ArtifactRef>,
    pub sections: BTreeMap<Namespace, serde_json::Value>,
    pub hash: Option<ContentHash>,
}
pub fn mark_open_artifacts(open: &mut [ArtifactRef], kind: EventKind, time: TimePoint);
pub fn ingest_input( id: Ident, kind: Namespace, uri: impl Into<String>, bytes: &[u8], ) -> ArtifactRef;
impl Manifest {
    pub fn from_json(doc: &serde_json::Value) -> Result<Manifest, crate::spec::SpecError>;
    pub fn seal(&mut self) -> Result<ContentHash, HashError>;
    pub fn write_section( &mut self, owner: &Namespace, section: Namespace, content: serde_json::Value, ) -> Result<(), crate::run::RunError>;
}
```

### `src/plan.rs`

```rust
pub struct Fragment {
    pub id: Ident,
    pub instance: ModuleRef,
    pub role: Role,
    pub content: serde_json::Value,
    pub after: Vec<Ident>,
}
pub struct DeclaredCost {
    pub link: DataLinkId,
    pub cost: u64,
}
pub struct ExecutionPlan {
    pub fragments: Vec<Fragment>,
    pub links: Vec<DataLinkDecl>,
    pub deps: Vec<(Ident, Ident)>,
    pub authority: Ident,
    pub class: ExecutionClass,
    pub transfer_costs: Vec<DeclaredCost>,
}
pub struct PrepareReport {
    pub fragment: Ident,
    pub effective: BTreeMap<Key, Value>,
    pub coercions: Vec<Coercion>,
    pub warnings: Vec<Warning>,
}
pub struct MergedPrepare {
    pub reports: Vec<PrepareReport>,
    pub effective: BTreeMap<Key, Value>,
}
impl MergedPrepare {
    pub fn from_reports(reports: Vec<PrepareReport>) -> MergedPrepare;
}
pub fn arm_order(nodes: &[Ident], edges: &[(Ident, Ident)]) -> Result<Vec<Ident>, SpecError>;
pub fn release_order(arm_order: &[Ident]) -> Vec<Ident>;
pub fn coercion_policy( spec_override: Option<CoercionPolicy>, is_session: bool, decl: Option<&KeyDecl>, ) -> CoercionPolicy;
pub fn apply_coercion( policy: CoercionPolicy, coercion: &Coercion, ) -> Result<Option<Warning>, SpecError>;
pub struct IslandContext<'a> {
    pub islands: &'a [IslandDecl],
    pub components: &'a BTreeMap<Ident, ComponentDescriptor>,
    pub placements: &'a BTreeMap<Ident, ComponentPlacement>,
    pub executors: &'a BTreeMap<Ident, ExecutorDescriptor>,
    pub links: &'a BTreeMap<ModuleRef, LinkDescriptor>,
    pub link_placements: &'a [LinkPlacement],
    pub graph_links: &'a [(PortRef, PortRef, BackPressure)],
    pub feed_links: &'a [(PortRef, PortRef, BackPressure)],
    pub sinks: &'a BTreeMap<Ident, SinkDescriptor>,
    pub resource_endpoints: &'a BTreeSet<Ident>,
}
pub fn admit_islands(ctx: &IslandContext<'_>) -> Result<(), ModuleError>;
pub enum EdgeKind {
    Stream,
    Event,
    Action,
}
pub struct GraphEdge {
    pub from: Ident,
    pub to: Ident,
    pub kind: EdgeKind,
    pub crosses_island: bool,
}
pub fn check_cycles(nodes: &[Ident], edges: &[GraphEdge]) -> Result<(), ModuleError>;
pub fn check_effective_narrows( declared: &crate::spec::CapabilityValue, effective: &crate::spec::CapabilityValue, ) -> Result<(), ModuleError>;
pub struct CompileInputs<'a> {
    pub registry: &'a ModuleRegistry,
    pub checks: &'a AdmissionCheckRegistry,
    pub kinds: &'a EventKindRegistry,
    pub providers: &'a BTreeMap<Ident, &'a dyn Provider>,
    pub authorities: &'a BTreeMap<Ident, AuthorityDescriptor>,
    pub executors: &'a BTreeMap<Ident, ExecutorDescriptor>,
    pub contracts: &'a ContractRegistry,
    pub sinks: &'a BTreeMap<Ident, &'a dyn crate::module_api::Sink>,
    pub is_session: bool,
}
pub fn validate( spec: &ExperimentSpec, profile: &BindingProfile, inputs: &CompileInputs<'_>, ) -> Result<AdmissionResult, SpecError>;
pub fn plan( spec: &ExperimentSpec, profile: &BindingProfile, admission: &AdmissionResult, inputs: &CompileInputs<'_>, transfer_costs: Vec<DeclaredCost>, ) -> Result<ExecutionPlan, SpecError>;
pub enum PrepareError {
    Fragment {
        index: usize,
        error: ModuleError,
    },
    Violations(Vec<crate::binding::Violation>),
}
pub fn collect_prepare( reports: Vec<Result<PrepareReport, ModuleError>>, spec: &ExperimentSpec, profile: &BindingProfile, inputs: &CompileInputs<'_>, admission: &AdmissionResult, ) -> Result<MergedPrepare, PrepareError>;
```

### `src/stream/block.rs`

```rust
pub enum Direction {
    Rx,
    Tx,
}
pub struct ChannelMask(pub(crate) u64);
impl ChannelMask {
    pub fn from_bits(bits: u64) -> ChannelMask;
    pub fn bits(self) -> u64;
    pub fn full(channels: u16) -> ChannelMask;
    pub fn is_set(self, c: u16) -> bool;
}
pub struct BlockFlags(pub(crate) u16);
impl BlockFlags {
    pub fn from_bits(bits: u16) -> BlockFlags;
    pub fn bits(self) -> u16;
    pub const GAP_BEFORE: BlockFlags = BlockFlags(0x0001);
    pub const SEQ_DISCONTINUITY: BlockFlags = BlockFlags(0x0002);
    pub const RESTARTED: BlockFlags = BlockFlags(0x0004);
    pub const LATE: BlockFlags = BlockFlags(0x0008);
    pub const PARTIAL_CHANNELS: BlockFlags = BlockFlags(0x0010);
    pub const START_OF_BURST: BlockFlags = BlockFlags(0x0020);
    pub const END_OF_BURST: BlockFlags = BlockFlags(0x0040);
    pub const ALIGNMENT: BlockFlags = BlockFlags(0x0080);
    pub const RESERVED: BlockFlags = BlockFlags(0xFF00);
    pub const NONE: BlockFlags = BlockFlags(0);
    pub fn contains(self, other: BlockFlags) -> bool;
    pub fn intersects(self, other: BlockFlags) -> bool;
    pub fn is_empty(self) -> bool;
}
pub struct BlockHeader {
    pub first_sample_time: TimePoint,
    pub len: u32,
    pub channels: u16,
    pub direction: Direction,
    pub valid: ChannelMask,
    pub flags: BlockFlags,
    pub lost: Option<u64>,
    pub contract: DataContractId,
}
impl BlockHeader {
    pub fn end_time(&self) -> Result<TimePoint, TimeError>;
}
pub struct SampleBlock { /* private */ }
pub type BlockRef = std::sync::Arc<SampleBlock>;
impl SampleBlock {
    pub fn new( header: BlockHeader, buffer: BufferRef, bytes_per_sample: u32, ) -> Result<SampleBlock, StreamError>;
    pub fn new_host( header: BlockHeader, memory_domain: MemoryDomainId, bytes: std::sync::Arc<[u8]>, bytes_per_sample: u32, ) -> Result<SampleBlock, StreamError>;
    pub fn host_bytes(&self) -> Option<&[u8]>;
    pub fn header(&self) -> &BlockHeader;
    pub fn first_sample_time(&self) -> TimePoint;
    pub fn end_time(&self) -> Result<TimePoint, TimeError>;
    pub fn buffer(&self) -> BufferRef;
}
```

### `src/stream/buffer.rs`

```rust
pub struct BufferRef {
    pub memory_domain: MemoryDomainId,
    pub handle: u64,
    pub len_bytes: u64,
}
```

### `src/stream/link.rs`

```rust
pub enum BackPressure {
    Block,
    DropOldest,
    DropNewest,
}
impl BackPressure {
    pub fn is_drop_class(self) -> bool;
}
pub struct DataLinkDecl {
    pub id: DataLinkId,
    pub from: PortRef,
    pub to: PortRef,
    pub contract: DataContractId,
    pub policy: BackPressure,
    pub capacity: u32,
}
pub enum PublishOutcome {
    Accepted,
    DroppedOldest,
    DroppedNewest,
    Full,
}
pub struct DropCarry {
    pub flags: BlockFlags,
    pub lost: Option<u64>,
    pub blocks: u32,
}
impl DropCarry {
    pub const CARRIED_FLAGS: BlockFlags = BlockFlags( BlockFlags::GAP_BEFORE.0 | BlockFlags::RESTARTED.0 | BlockFlags::SEQ_DISCONTINUITY.0 | BlockFlags::ALIGNMENT.0, );
    pub fn is_empty(self) -> bool;
    pub fn absorb(&mut self, dropped: &BlockRef);
}
pub trait DataLink: Send + Sync {
    fn publish(&self, b: BlockRef) -> PublishOutcome;
    fn receive(&self) -> Option<BlockRef>;
    fn drops(&self) -> u64;
    fn take_drop_carry(&self) -> DropCarry;
    fn policy(&self) -> BackPressure;
}
pub fn check_sink_link(decl: &DataLinkDecl, consumer_is_sink: bool) -> Result<(), StreamError>;
```

### `src/stream/burst.rs`

```rust
pub struct AdmittedTarget {
    pub target: TimePoint,
    pub requested_target: Option<TimePoint>,
}
pub fn admit_burst_target( registry: &ClockRegistry, target: TimePoint, tx_domain: ClockDomainId, ) -> Result<AdmittedTarget, StreamError>;
pub enum LatePolicy {
    RejectAtPlan,
    SendAsapAndFlag,
    DropAndFlag,
}
pub enum LateOutcome {
    OnTime {},
    SendAsap {
        late_by: Duration,
    },
    Drop {
        late_by: Duration,
    },
    PlanViolation {
        late_by: Duration,
    },
}
impl LatePolicy {
    pub fn decide( self, registry: &ClockRegistry, target: TimePoint, now: TimePoint, min_lead: Duration, ) -> Result<LateOutcome, TimeError>;
}
pub struct BurstOpen {
    pub waveform_len: Option<u32>,
    pub late: Option<LateOutcome>,
    pub requested_target: Option<TimePoint>,
}
pub enum BurstEnd {
    Eob,
    Stop,
    Discontinuity,
}
pub struct BurstRecord {
    pub target: TimePoint,
    pub requested_target: Option<TimePoint>,
    pub actual_start: Option<TimePoint>,
    pub blocks: u32,
    pub samples: u64,
    pub wraps: u32,
    pub late_by: Option<Duration>,
    pub end: BurstEnd,
}
pub enum BurstState {
    Idle,
    InBurst {
        target: TimePoint,
        expected_next: TimePoint,
        blocks: u32,
        samples: u64,
    },
}
pub enum BurstStep {
    Started,
    Continued,
    Ended {
        record: BurstRecord,
    },
    Discontinuity {
        expected: TimePoint,
        got: TimePoint,
        closed: BurstRecord,
        then_ended: Option<BurstRecord>,
    },
}
pub struct BurstTracker { /* private */ }
impl BurstTracker {
    pub fn new(domain: ClockDomainId) -> BurstTracker;
    pub fn state(&self) -> BurstState;
    pub fn on_block( &mut self, h: &BlockHeader, open: Option<BurstOpen>, ) -> Result<BurstStep, StreamError>;
    pub fn set_late(&mut self, late: LateOutcome);
    pub fn set_actual_start(&mut self, t: TimePoint);
    pub fn stop(&mut self) -> Option<BurstRecord>;
}
```

### `src/stream/continuity.rs`

```rust
pub enum GapCause {
    Stream {},
    OverflowRestart {},
    SequenceError {},
    Alignment {},
    LinkDrop {},
    Mixed {
        stream_lost: u64,
    },
    Unknown {},
}
pub struct Gap {
    pub start: TimePoint,
    pub len: u64,
    pub lost: Option<u64>,
    pub cause: GapCause,
    pub link_dropped: u32,
}
pub struct ChannelGap {
    pub channel: u16,
    pub start: TimePoint,
    pub len: u64,
    pub cause: GapCause,
}
pub struct Segment {
    pub start: TimePoint,
    pub len: u64,
}
pub struct ContinuityMap {
    pub domain: ClockDomainId,
    pub channels: u16,
    pub valid: Vec<Vec<Segment>>,
    pub gaps: Vec<Gap>,
    pub channel_gaps: Vec<ChannelGap>,
    pub first: TimePoint,
    pub end: TimePoint,
}
pub struct ContinuityBuilder { /* private */ }
impl ContinuityBuilder {
    pub fn new(domain: ClockDomainId, channels: u16, lossless: bool) -> ContinuityBuilder;
    pub fn push( &mut self, h: &BlockHeader, carry: DropCarry, ) -> Result<(), (StreamError, DropCarry)>;
    pub fn finish(mut self, carry: DropCarry) -> ContinuityMap;
}
```

### `src/stream/mod.rs`

```rust
pub enum StreamError {
    Time(TimeError),
    MissingStartOfBurst,
    TimeOverlap {
        expected: TimePoint,
        got: TimePoint,
    },
    GapFlagWithoutJump,
    JumpWithoutGapFlag,
    InvalidBlock {
        reason: String,
    },
    DomainChanged {
        from: ClockDomainId,
        to: ClockDomainId,
    },
    ChannelsChanged {
        from: u16,
        to: u16,
    },
    Incompatible {
        from: DataContractId,
        to: DataContractId,
    },
}
```

### `src/contract.rs`

```rust
pub struct DataContractId(String);
impl DataContractId {
    pub fn parse(id: &str) -> Result<DataContractId, StreamError>;
    pub fn as_str(&self) -> &str;
}
pub enum Scalar {
    Int(i64),
    Float(#[serde(serialize_with = "crate::hash::serialize_finite_f64")] f64),
    Str(String),
    Bool(bool),
}
pub struct DataContract {
    pub id: DataContractId,
    pub attributes: BTreeMap<String, Scalar>,
    pub compatible_from: BTreeSet<DataContractId>,
}
impl DataContract {
    pub fn bytes_per_sample(&self) -> Option<u32>;
}
pub enum PortDirection {
    In,
    Out,
}
pub struct Port {
    pub name: crate::spec::Ident,
    pub direction: PortDirection,
    pub contract: DataContractId,
}
pub struct PortRef {
    pub component: crate::spec::Ident,
    pub port: crate::spec::Ident,
}
pub struct ContractRegistry { /* private */ }
impl ContractRegistry {
    pub fn new() -> ContractRegistry;
    pub fn with_standard_contracts() -> ContractRegistry;
    pub fn register(&self, contract: DataContract) -> Result<(), StreamError>;
    pub fn ids(&self) -> Vec<DataContractId>;
    pub fn get(&self, id: &DataContractId) -> Option<DataContract>;
    pub fn check_link( &self, from: &DataContractId, to: &DataContractId, ) -> Result<(), StreamError>;
}
pub fn standard_contracts() -> Vec<DataContract>;
```

### `src/schema.rs`

```rust
pub const SCHEMA_MAJOR: u32 = 1;
pub fn generator() -> SchemaGenerator;
pub fn file_name(name: &str) -> String;
pub fn document_schemas() -> BTreeMap<&'static str, serde_json::Value>;
pub fn render(schema: &serde_json::Value) -> String;
```


---

## Appendix B — Every rule, its step and its test

One row per rule of specs 06–10 and of `00-overview.md` §6. "Proved by" names the test or tests whose assertion is the rule's obligation, from the specs' own test tables; a row with no test says what carries the rule instead. Step 16's exit tables (PO-10) are built from this table and checked against the test bodies.

| Rule | Spec | Step | Proved by |
|---|---|---|---|
| KC-1 | 06 | 4 | `kc_01_a_parse_failure_is_no_run`, `kc_01_a_validate_failure_still_writes_a_manifest` |
| KC-2 | 06 | 4 | `kc_02_a_wall_paced_authority_is_refused` |
| KC-3 | 06 | 4 | `kc_20_virtual_time_advances_only_through_next_wakeup`, `ma_05a_a_module_keeps_its_handles_after_prepare` |
| KC-4 | 06 | 4 | `kc_04_an_object_under_no_resource_is_refused`, `kc_04_one_object_serves_both_names`, `kc_04_two_objects_for_one_description_are_refused` |
| KC-5 | 06 | 4 | `kc_45_manifest_fields` (`run.id`, `run.kind`, `run.parent`, `lease`, `policy`) |
| KC-6 | 06 | 4 | by construction: the `Assembly` is moved into the Run, and no method takes a registry afterwards |
| KC-7 | 06 | 4 | `kc_01_a_validate_failure_records_its_reason`, `kc_01_a_validate_failure_still_writes_a_manifest` |
| KC-8 | 06 | 5 | `kc_45_manifest_fields` (the `(sink/rec, DEVICE_LOST)` and `(kernel, STEP_LIVELOCK)` rows), `kc_30_device_lost_is_the_kernel_event_and_aborts`, `v58_04_mock_events_reach_counters_policy_and_manifest` |
| KC-9 | 06 | 5 | `kc_09_an_input_must_be_supplied_and_match_its_hash` |
| KC-10 | 06 | 5 | `kc_10_both_ends_are_attached`, `kc_10_link_descriptor_must_equal_the_registered_one` |
| KC-11 | 06 | 5 | `kc_11_an_island_gets_exactly_its_components` |
| KC-12 | 06 | 5 | `kc_12_a_prepare_failure_stops_the_loop_and_cleans_up_what_was_prepared` |
| KC-13 | 06 | 5 | `kc_13_arm_and_start_follow_instance_order_cleanup_reverses_it`, `ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once` |
| KC-14 | 06 | 5 | `kc_14_an_arm_failure_is_failed_arm` |
| KC-15 | 06 | 5 | `kc_15_t0_is_arm_end_plus_the_lead` |
| KC-16 | 06 | 6 | `kc_16_a_spec_time_resolves_on_the_target_stream`, `kc_16_an_ambiguous_spec_time_is_refused`, `kc_16_off_root_negative_and_overflowing_times_are_refused` |
| KC-17 | 06 | 6 | `kc_17_a_scheduled_stop_ends_the_run_at_its_instant`, `kc_17_scheduled_updates_are_admitted_cumulatively` |
| KC-18 | 06 | 5, 6 | `kc_18_a_start_failure_is_failed_arm` |
| KC-19 | 06 | 6 | `kc_19_a_reject_at_plan_burst_with_a_short_lead_is_refused` |
| KC-20 | 06 | 6 | `kc_20_virtual_time_advances_only_through_next_wakeup` |
| KC-21 | 06 | 6 | `kc_21_an_action_is_seen_at_its_admission_instant` |
| KC-22 | 06 | 6 | `kc_22_a_downgraded_livelock_still_ends_the_run`, `kc_22_a_same_instant_wakeup_loop_is_step_livelock` |
| KC-23 | 06 | 6 | `kc_23_an_unknown_target_is_refused`, `kc_23_targets_are_rewritten_through_matched` |
| KC-24 | 06 | 7 | `kc_24_a_burst_needs_a_transmit_sample_clock`, `kc_24_a_burst_time_on_an_unrelated_root_is_refused`, `kc_24_a_burst_to_a_non_provider_target_is_refused`, `kc_24_a_module_abort_ends_the_run`, `kc_24_a_module_action_during_cleanup_is_refused`, `kc_24_a_module_reject_at_plan_burst_is_refused`, `kc_24_a_module_stop_without_target_is_refused` |
| KC-25 | 06 | 7 | `kc_21_an_action_is_seen_at_its_admission_instant`, `kc_25_an_admitted_update_changes_the_configuration` |
| KC-26 | 06 | 7 | `kc_26_a_provider_that_applies_nothing_is_refused`, `rs_17_a_session_rate_change_is_coerced_by_its_provider` |
| KC-27 | 06 | 7 | `kc_25_an_admitted_update_changes_the_configuration` |
| KC-28 | 06 | 7 | `kc_28_a_malformed_action_takes_no_sequence_number`, `kc_28_a_waveform_is_an_input_before_admission`, `kc_28_an_untimed_burst_is_admitted_at_now_plus_lead`, `kc_28_stop_run_ends_the_session` |
| KC-29 | 06 | 6 | `kc_29_advance_to_refuses_an_unrelated_time` |
| KC-30 | 06 | 8 | `kc_30_a_panic_in_coerce_fails_validate`, `kc_30_a_panicking_module_fails_the_run_not_the_process`, `kc_30_device_lost_is_the_kernel_event_and_aborts` |
| KC-31 | 06 | 8 | `kc_31_mark_artifact_marks_only_artifacts_open_then` |
| KC-32 | 06 | 8 | `kc_32_an_abort_during_orderly_escalates_and_is_recorded` |
| KC-33 | 06 | 4–8 | `kc_17_a_scheduled_stop_ends_the_run_at_its_instant`, `kc_24_a_module_abort_ends_the_run`, `kc_28_stop_run_ends_the_session` |
| KC-34 | 06 | 4–8 | `kc_01_a_validate_failure_still_writes_a_manifest`, and every test that calls `finish()` on a live Run |
| KC-35 | 06 | 7 | `kc_35_connect_refuses_an_invalid_lease` |
| KC-36 | 06 | 7 | `kc_36_attached_disconnect_ends_the_run`, `kc_36_detached_lease_expiry_ends_the_run` |
| KC-37 | 06 | 7 | `kc_37_run_child_is_refused` |
| KC-38 | 06 | 4–8 | `kc_32_an_abort_during_orderly_escalates_and_is_recorded`, `kc_39_every_instance_is_stopped_before_it_is_cleaned_up` |
| KC-39 | 06 | 4–8 | `kc_39_abort_cleanup_does_not_drain`, `kc_39_every_instance_is_stopped_before_it_is_cleaned_up`, `kc_39_orderly_cleanup_drains_the_tail` |
| KC-40 | 06 | 8 | `kc_32_an_abort_during_orderly_escalates_and_is_recorded` |
| KC-41 | 06 | 8 | `kc_41_a_failing_sink_stop_is_a_cleanup_failure` |
| KC-42 | 06 | 8 | `kc_31_mark_artifact_marks_only_artifacts_open_then` |
| KC-43 | 06 | 8 | `kc_01_a_validate_failure_records_its_reason` |
| KC-44 | 06 | 8 | `kc_30_a_panic_in_coerce_fails_validate`, `kc_44_a_wedged_step_does_not_prevent_the_manifest`, `kc_45_a_provider_section_outside_its_namespace_is_a_cleanup_failure` |
| KC-45 | 06 | 8 | `kc_45_manifest_fields`, `kc_45_sample_clocks_and_domains_are_recorded`, `kc_45_the_documents_are_recorded_verbatim` |
| UC-1 | 06 | 7 | `kc_25_an_admitted_update_changes_the_configuration` (the class reaches the target unchanged) |
| UC-2 | 06 | 14 | `mr_18_hardware_timed_updates` |
| UC-3 | 06 | 14 | `mr_18_a_cold_rate_change_starts_a_new_sample_clock` |
| UC-4 | 06 | 14 | `hd_10_session_requests_are_sequential` (a capture request applies from the first sample at or after its instant) |
| UC-5 | 06 | 14 | no carrier in Phase 2: no key declares `atomic_realtime` |
| UC-6 | 06 | 14 | `mr_18_hardware_timed_updates` |
| KA-1 | 06 | 1, 2 | `ma_05a_a_module_keeps_its_handles_after_prepare` |
| KA-2 | 06 | 1, 2 | `kc_45_sample_clocks_and_domains_are_recorded`, `tm_12_domains_lists_every_registered_domain`, `tm_13a_declared_clocks_are_listed_before_registration` |
| KA-3 | 06 | 1, 2 | `sc_08_host_bytes_only_for_a_block_that_carries_them` |
| KA-4 | 06 | 1, 2 | `rm_19_rf_envelope_cases`, `sb_30_the_check_sees_each_fragment_s_own_value`, `se_05_checks` |
| KA-5 | 06 | 1, 2 | `sb_07_a_stray_rejection_is_a_violation`, `sb_07_coerce_refuses_a_combination`, `sb_07_non_coercible_key_fails_directly` |
| KA-6 | 06 | 7 | `kc_26_a_provider_that_applies_nothing_is_refused`, `rs_17_a_scheduled_rate_change_under_reject_is_refused`, `rs_17_a_session_change_beyond_a_joint_limit_is_refused`, `rs_17_a_session_rate_change_is_coerced_by_its_provider` |
| KA-7 | 06 | 1, 2 | `kc_28_an_untimed_burst_is_admitted_at_now_plus_lead`, `mr_04_instance`, `sb_22f_min_command_lead_is_in_host_monotonic` |
| KA-8 | 06 | 1, 2 | `kc_15_t0_is_arm_end_plus_the_lead`, `ma_41_an_absent_or_non_string_class_is_refused`, `ma_41_ezsdr_time_is_a_closed_set` |
| KA-9 | 06 | 5 | `ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once` |
| KA-10 | 06 | 1, 2 | descriptions only; `schema_freeze` |
| KA-11 | 06 | 1, 2 | `kc_20_virtual_time_advances_only_through_next_wakeup` |
| KA-12 | 06 | 4–6 | `kc_39_abort_cleanup_does_not_drain`, `kc_39_every_instance_is_stopped_before_it_is_cleaned_up`, `kc_39_orderly_cleanup_drains_the_tail`; the drain's `closing` rules are carried by construction (a deterministic test would need a drain outliving the 5 s deadline) |
| KA-13 | 06 | 4, 8 | `kc_45_a_provider_section_outside_its_namespace_is_a_cleanup_failure`, `kc_45_manifest_fields` |
| KA-14 | 06 | 1, 2 | `sb_22h_the_sink_path_segment_is_reserved` |
| KA-15 | 06 | 1, 2 | `kc_10_both_ends_are_attached` |
| KA-16 | 06 | 2 | `kc_45_sample_clocks_and_domains_are_recorded` |
| KA-17 | 06 | 1, 2 | `tm_17_next_due_reports_without_advancing` |
| KA-18 | 06 | 5 | `kc_45_manifest_fields` |
| KA-19 | 06 | 2, 16 | text only (steps 2 and 16) |
| KA-22 | 06 | 2 | `rm_20_schema_freeze`, `rm_22_payloads_round_trip` |
| KA-20 | 06 | 1, 2 | `kernel_surface` (every Phase 2 Kernel item cites a KA-, KC- or UC- rule) |
| KA-21 | 06 | 1, 2 | `kc_28_a_malformed_action_takes_no_sequence_number` |
| RM-1 | 07 | 11 | `rm_01_register_adds_the_descriptor_the_check_and_the_kinds`, `rm_01_register_twice_is_refused` |
| RM-2 | 07 | 11, 14 | `mr_02_the_tree_has_the_radio_model_shape` |
| RM-3 | 07 | 11, 14 | `mr_02_the_tree_has_the_radio_model_shape`, `mr_07_prepare_cases` |
| RM-4 | 07 | 11 | `rm_04_the_key_table_is_exactly_the_declared_one` |
| RM-5 | 07 | 11, 14 | `mr_08_effective_holds_exactly_the_ten_configuration_keys` |
| RM-6 | 07 | 11, 14 | `mr_04_instance` (`min_command_lead`), `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section` |
| RM-7 | 07 | 11, 14 | `mr_06_coerce_cases`, `mr_06_coerce_is_pure`, `mr_18_a_scheduled_pair_is_checked_when_it_applies` |
| RM-8 | 07 | 11, 14 | `mr_06_coerce_cases`, `mr_06_coerce_is_pure` |
| RM-9 | 07 | 11, 14 | `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section` |
| RM-10 | 07 | 11 | `rm_10_the_kinds_are_registered_under_radio_with_their_defaults` |
| RM-11 | 07 | 11, 14 | `rm_22_payloads_round_trip` |
| RM-12 | 07 | 11 | `rm_12_the_verbs_compile_to_bursts` |
| RM-13 | 07 | 11, 14 | `mr_16_burst_refusals` |
| RM-14 | 07 | 11, 14 | `mr_17_late_policy_outcomes` |
| RM-15 | 07 | 11, 14 | `mr_16_burst_refusals`, `mr_16_repeat_is_contiguous_across_wraps` |
| RM-16 | 07 | 11, 14 | `mr_25_orderly_stop_delivers_the_tail_abort_does_not` |
| RM-17 | 07 | 11, 14 | `mr_21_overrun_shape`, `v58_06_injected_overflow_is_a_uhd_overflow` |
| RM-18 | 07 | 11, 14 | `mr_22_sequence_error_shape`, `v58_06_sequence_error_is_seq_discontinuity` |
| RM-19 | 07 | 11 | `rm_19_a_malformed_section_is_one_violation`, `rm_19_rf_envelope_cases`, `rm_19_the_proposed_value_is_judged` |
| RM-20 | 07 | 11 | `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`, `rm_20_schema_freeze` |
| RM-22 | 07 | 11 | `rm_20_schema_freeze`, `rm_22_payloads_round_trip` |
| RM-21 | 07 | 11, 14 | `mr_18_a_cold_rate_change_starts_a_new_sample_clock`, `mr_18_hardware_timed_updates` |
| SE-1 | 08 | 10 | `se_01_register_adds_the_descriptor_and_both_checks` |
| SE-2 | 08 | 10 | `se_02_seed_reader` |
| SE-3 | 08 | 10 | `se_03_faults_reader` |
| SE-4 | 08 | 10 | `mr_20_faults_fire_at_their_instants` |
| SE-5 | 08 | 10 | `se_05_checks` |
| SE-6 | 08 | 10 | `se_06_splitmix_vectors` |
| SE-7 | 08 | 10 | `se_09_the_virtual_root_is_registered` |
| SE-8 | 08 | 10 | `se_08_descriptor_registers` |
| SE-9 | 08 | 10 | `se_09_the_virtual_root_is_registered` |
| SE-10 | 08 | 10 | `se_10_host_monotonic_advances_in_lockstep`, `se_10_time_authority_contract` |
| SE-11 | 08 | 10 | `se_11_empty_is_none_and_moves_nothing`, `se_11_next_wakeup_order_ties_and_cap` |
| SE-12 | 08 | 10 | `se_12_schema_freeze` |
| SE-13 | 08 | 10 | `se_13_two_engines_fed_the_same_schedule_fire_identically` |
| MR-1 | 09 | 14 | `mr_01_descriptor_registers` |
| MR-2 | 09 | 14 | `mr_02_from_binding_refusals`, `mr_02_the_tree_has_the_radio_model_shape` |
| MR-3 | 09 | 14 | `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section` |
| MR-4 | 09 | 14 | `mr_04_instance` |
| MR-5 | 09 | 14 | `mr_08_effective_holds_exactly_the_ten_configuration_keys` |
| MR-6 | 09 | 14 | `mr_06_coerce_cases`, `mr_06_coerce_is_pure` |
| MR-7 | 09 | 14 | `mr_07_prepare_cases`, `mr_07_prepare_reports_the_coercions_coerce_reported`, `mr_08_effective_holds_exactly_the_ten_configuration_keys` |
| MR-8 | 09 | 14 | `mr_07_prepare_reports_the_coercions_coerce_reported` |
| MR-9 | 09 | 14 | `mr_11_start_cases` |
| MR-10 | 09 | 14 | `mr_18_a_cold_rate_change_starts_a_new_sample_clock` |
| MR-11 | 09 | 14 | `mr_11_start_cases` |
| MR-12 | 09 | 14 | `mr_12_block_lengths` |
| MR-13 | 09 | 14 | `mr_13_ramp_values` |
| MR-14 | 09 | 14 | `mr_14_blocks_appear_when_their_last_sample_has_occurred` |
| MR-15 | 09 | 14 | `mr_18_a_cold_transmit_change_replaces_the_tracker` |
| MR-16 | 09 | 14 | `mr_16_burst_refusals`, `mr_16_repeat_is_contiguous_across_wraps` |
| MR-17 | 09 | 14 | `mr_17_late_policy_outcomes` |
| MR-18 | 09 | 14 | `mr_18_a_cold_rate_change_starts_a_new_sample_clock`, `mr_18_a_cold_transmit_change_replaces_the_tracker`, `mr_18_a_scheduled_pair_is_checked_when_it_applies`, `mr_18_hardware_timed_updates` |
| MR-19 | 09 | 14 | `mr_19_backpressure_is_an_overrun` |
| MR-20 | 09 | 14 | `mr_20_faults_fire_at_their_instants` |
| MR-21 | 09 | 14 | `mr_21_overrun_shape` |
| MR-22 | 09 | 14 | `mr_22_sequence_error_shape` |
| MR-23 | 09 | 14 | `mr_16_repeat_is_contiguous_across_wraps` |
| MR-24 | 09 | 14 | `mr_24_a_bypassing_provider_gets_time_error` |
| MR-25 | 09 | 14 | `mr_25_orderly_stop_delivers_the_tail_abort_does_not` |
| MR-26 | 09 | 14 | `mr_26_cleanup_is_idempotent` |
| MR-27 | 09 | 14 | `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`, `mr_18_hardware_timed_updates`, `mr_20_faults_fire_at_their_instants`, `mr_29_other_actions_are_command_rejected` |
| MR-28 | 09 | 14 | `mr_29_other_actions_are_command_rejected` |
| MR-29 | 09 | 14 | `mr_29_other_actions_are_command_rejected` |
| MR-30 | 09 | 14 | `mr_30_two_mocks_one_seed_identical_output` |
| HD-1 | 10 | 12 | `hd_07_descriptor` (`memory_domains`) |
| HD-2 | 10 | 12 | `hd_02_a_released_slot_is_reused_and_a_held_one_is_not` |
| HD-3 | 10 | 12 | `hd_03_layout_round_trip` |
| HD-4 | 10 | 12 | `hd_04_descriptor_and_create` |
| HD-5 | 10 | 12 | `hd_05_policies` |
| HD-6 | 10 | 12 | `hd_06_vocabulary` |
| HD-7 | 10 | 13 | `hd_07_descriptor` |
| HD-8 | 10 | 13 | `hd_09_prepare_cases` |
| HD-9 | 10 | 13 | `hd_09_prepare_cases` |
| HD-10 | 10 | 13 | `hd_10_a_capture_spans_a_gap_and_a_clock_change`, `hd_10_capture_of_n_samples_across_jittered_blocks`, `hd_10_no_capture_no_artifact`, `hd_10_session_requests_are_sequential`, `hd_10_the_carry_attributes_a_dropped_overflow`, `hd_10_the_own_capture_comes_first`, `hd_14_a_bad_capture_value_is_an_event_not_a_failure` |
| HD-11 | 10 | 13 | `hd_11_an_unexpected_action_is_rejected` |
| HD-14 | 10 | 12, 13 | `hd_11_an_unexpected_action_is_rejected`, `hd_14_a_bad_capture_value_is_an_event_not_a_failure`, `hd_14_schema_freeze` |
| HD-12 | 10 | 13 | `hd_10_capture_of_n_samples_across_jittered_blocks` |
| HD-13 | 10 | 13 | `hd_10_no_capture_no_artifact`, `hd_13_partial_on_abort_and_on_an_unfinished_capture` |
| PO-1 | 00 | all | process: rule ids of Phase 2 documents |
| PO-2 | 00 | 15 | `po_02_every_crate_forbids_unsafe_code` |
| PO-3 | 00 | all | process: test naming |
| PO-4 | 00 | 15 | `po_04_the_lock_gains_no_external_package` |
| PO-5 | 00 | 4–8 | `kernel_surface` |
| PO-6 | 00 | 1 | `schema_freeze` |
| PO-7 | 00 | 10–12 | `hd_14_schema_freeze`, `rm_20_schema_freeze`, `se_12_schema_freeze` |
| PO-8 | 00 | 15 | `ma_03_no_module_crate_depends_on_another` |
| PO-9 | 00 | all | process: steps 2 and 16 |
| PO-10 | 00 | all | process: step 16 |
| PO-11 | 00 | 15 | `po_11_no_hashmap_and_no_wall_clock_in_simulation_code` |
| PO-12 | 00 | all | process: every step's mutation checks |
