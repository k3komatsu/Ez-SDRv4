# AGENTS.md — working in the Ez-SDR repository

Read this before touching anything. Current state and the next step: [handoff.md](handoff.md).

## 1. What this repository is

Ez-SDR is an SDR experiment runtime with two unrelated lines, which since 2026-09-26 live in two GitHub repositories: v4 in **[k3komatsu/Ez-SDRv4](https://github.com/k3komatsu/Ez-SDRv4)** (this repository's `origin`, default branch `main`) and v3 in **[k3komatsu/Ez-SDR](https://github.com/k3komatsu/Ez-SDR)** (`master` and the v3 tags). This local clone holds both histories:

| Branch | Content | Status |
|---|---|---|
| `main` | **v4** — clean-sheet Rust rewrite. Phases 1–7's accepted specs in `design/01-*.md` … `design/11-*.md`, `design/14-*.md`, `design/16-*.md` and `design/18-*.md` (their process records remain in `plan/phase1/` … `plan/phase7/`; the amendment-only specs 12, 13, 15, 17 and 19 are applied to `design/` and recorded in `plan/phase3/12-amendments.md`, `plan/phase4/13-amendments.md`, `plan/phase5/15-amendments.md`, `plan/phase6/17-amendments.md` and `plan/phase7/19-amendments.md`), plus the Kernel, Radio Model, Simulation Engine, MockRadio, SimulationChannel, native Executor and server implementation (`crates/`, `schemas/`) and the Python client (`python/`). Phase 5 (the Mini Reactive Radio: the native Executor `ezsdr.exec.native` and a PING responder) was accepted at Gate X on 2026-09-26. Phase 6 (the Python Easy API: the server `ezsdr-server`, the protocol `ezsdr.protocol` 1, the package `python/ezsdr`) was accepted at Gate X on 2026-09-27. Phase 7 (the Native UHD Provider `ezsdr.radio.uhd`: the crate `ezsdr-radio-uhd`) was designed and accepted at Gate P on 2026-09-27 (`plan/phase7/`: specs 18 and 19, the bench procedure; Reviews J and K), implemented on the branch `worktree-phase7-impl`, reviewed by Opus (Reviews L–T), run on an X300 + one OBX (`plan/phase7/bench-results.md`), and accepted at Gate X on 2026-10-01, when the branch was merged into `main`; Step X moved spec 18 to `design/18-uhd-radio.md` and applied its Vision issues the same day; a Kernel race found afterwards (RS-36's escalation flag against KC-31's delivered order) was fixed and reviewed (Review U), and Phase 7 was closed the same day. Phase 8 onwards is unwritten. | active development |
| `master` | **v3** — D + C++ UHD bridge + Python client. Tags `v2.11`, `v3.0.0`–`v3.0.28`. Local only here; its GitHub home is `k3komatsu/Ez-SDR`. | maintenance |
| `spike/uhd` | A throwaway UHD spike (2026-09-26), branched from `main`: a one-line Kernel change (K1) and a separate `spike/uhd/` Cargo workspace. Never merge it into `main`; its findings live in `main`'s `plan/spikes/2026-09-26-uhd.md` (handoff.md §4, "UHD spike"). | pushed; bench runs deferred |
| `worktree-phase7-impl` | The Phase 7 implementation and its bench session (2026-09-28 to 2026-10-01), merged into `main` at Gate X on 2026-10-01. Kept for its history; no further work goes there. | pushed; merged |
| `gh-pages` | GitHub Pages content for `Ez-SDRv4` only: an orphan branch (no history shared with `main` or `master`) holding a project overview site, served at https://k3komatsu.github.io/Ez-SDRv4/ from the branch root. Its status text is updated only when the user asks. | pushed; Pages published |

Rules that follow from this layout:

- Never graft, rebase or merge `master` and `main` into each other (not even `merge -s ours`). v4 shares no code with v3; the histories are unrelated on purpose.
- v4 work happens on `main` only (the throwaway `spike/uhd` excepted, and it is never merged; `worktree-phase7-impl` was merged at Phase 7's Gate X and takes no new work). Do not commit to `master` from a v4 session unless the user explicitly asks for a v3 change.
- Never push `master` or the v3 tags to `origin` (`Ez-SDRv4`): no `git push --tags`, no `git push --all`. The v3 history belongs to `k3komatsu/Ez-SDR`.
- `gh-pages` holds only what GitHub Pages serves. Never merge it with `main` or `master`, and change it only when the user asks.
- `v3/` in the working tree is a **git worktree** of `master`, listed in `.gitignore`. Never `git add v3/`. Never delete it: the design documents cite 22 `v3/...` paths as behavioural evidence (check command in handoff.md §1).
- There is no default-branch flip: the two lines are separate repositories. The v3 → v4 items that remain (a container image name shared by both, the v3 README pointing to v4) are in handoff.md §5.

## 2. Where the design truth lives

- **The Vision is the single design source.** Index: [Ez-SDR_v4_ARCHITECTURE_VISION.md](Ez-SDR_v4_ARCHITECTURE_VISION.md). Body: [design/vision/](design/vision), 11 part files, sections **§1–§68**. `cat design/vision/*.md` reproduces the whole text.
- **§N is the citation unit and is stable.** Never renumber, merge or delete a section. If content moves out, leave the section as a summary plus a link. [design/v4-vision-audit.md](design/v4-vision-audit.md) (Findings 1–34) and [design/v4-vision-rereview.md](design/v4-vision-rereview.md) (R1–R22) cite `§N` and `CMA §N`.
- [design/archive/Ez-SDR_v4_core_module_architecture.md](design/archive/Ez-SDR_v4_core_module_architecture.md) is retired and frozen. Do not edit or revive it; it exists only to keep `CMA §N` citations resolvable.
- When you edit the Vision: keep each part's header and footer navigation, and add a row to the revision history table in the index. Shapes in `{ ... }` blocks are illustrative, not schemas.
- Normative schemas and contracts live in the accepted specs `design/01-time-model.md` … `design/11-simulation-channel.md`, `design/14-native-executor.md`, `design/16-easy-api.md` and `design/18-uhd-radio.md`; a Vision section they cover keeps its reasons and ends in a `Normative:` line (re-review R13). Do not grow the Vision with more normative text.
- `plan/` holds design work in progress and process records; `design/` holds accepted text. `plan/phaseN/00-overview.md` (N = 1…7) remains each phase's record and governance log.
- Reading order for implementers: index "How to read" → Part 01 → §65 (42 invariants) → audit §13 (minimal Kernel) and §14.1 (P0 checklist) → the applicable accepted specs and phase overview → §58 (acceptance tests).

## 3. Design constraints that shape every type (settled; do not relitigate)

Full list: Vision §65, 42 invariants. The ones that bite first when writing code:

- **Kernel stays small.** Three tiers: Kernel (frozen at v4.0), Vocabulary (versioned, additive), Extensions (namespaced, unstable). A new experiment type adds a Vocabulary crate, never a Kernel concept. Nothing on the §6 list (UHD API, CUDA, wasmtime, RFNoC graph construction, IEEE 802.11 algorithms, placement optimiser, metrics framework, ...) appears in Kernel crates. §5, §6.
- **"Kernel" means only the Core tier.** The discrete-event simulator is the **Simulation Engine** (`sim-engine`), never a kernel. §5, §15.
- **Time is integer ticks at a rational rate; every timestamp names its ClockDomain.** Never floating-point seconds. A sample-rate change starts a new SampleClock. §15, §23, §27.
- **The Stream Contract is normative.** SampleBlocks are immutable-after-publish, refcounted handles tagged with a MemoryDomain; gaps are flags plus time jumps, never filled; validity is per channel; TX is a sequence of bursts with `START_OF_BURST` / `END_OF_BURST`, a time jump inside a burst is `TX_DISCONTINUITY`, never zero padding. §23.
- **Mock enforces the hardware envelope.** A Mock that accepts what an X310 would reject is a bug. TimingEnvelope and PerformanceEnvelope are checked at `validate()` and at runtime, on Mock and hardware alike. §13, §34, §59.
- **Intent and binding are separate.** ExperimentSpec = intent, with per-direction resource requests (`rx` / `tx`). BindingProfile = bindings + placements + environment (channel model, fault schedule, RF safety envelope). Placement and environment never appear in the Spec. §8, §10, §20.
- **Everything is a Run.** `connect()` opens a Session = Run with an implicit Spec and a typed action log; every Action is admitted on the control path before dispatch. §3, §50, §52.
- **Compile before RUN; the real-time path never parses Spec/JSON.** No structural graph mutation during RUN; parameters change only through declared update classes. §10, §27.
- **Modules talk only through Kernel contracts** (Resources, Ports, Events, Actions, Capabilities, DataContracts); no Module depends on another concrete Module. Coherence is declared by the owning Provider, never inferred across Providers. §7, §25.
- **Kernel public types are schema-first, versioned, language-neutral.** Old documents are migrated or refused, never silently reinterpreted. §9, §65 #39.
- **Core validates, it does not optimise.** No automatic placement, graph fusion or transfer planning. §10, §20, §31, §63.

## 4. Order of work

Vision §67: Phase 1 Kernel semantic model → Phase 2 Radio Model + Simulation Engine + MockRadio → Phase 3 SimulationChannel + deterministic Runs → … → Phase 7 UHD → Phase 8 Mock ↔ X310 parity.

- **No UHD before Core + Mock works** (§59). The parity test measures the hardware envelope and fails if the Mock profile is looser than the measurement.
- **Definition of done for Phases 1–6 is Vision §58** (16 acceptance tests). Tie every milestone to the tests it satisfies.
- §60's crate layout is directional. Do not create crates to mirror the diagram; logical boundaries matter, crate count does not.
- Most tests must run without hardware (§61).

## 5. v3 is evidence, not a template

- Use `v3/` for behavioural requirements, compatibility expectations and historical lessons only (§1). Do not port its structure, message format or command identifiers (§61: behaviour compatibility, not wire compatibility).
- The four v3 behaviours worth regression tests (§61): continuous repeat across the waveform wrap; capture at a requested TimePoint / sample index (replaces `alignSize`); timed TX/RX start at device time (`onTime`); multi-device 10 MHz + PPS aligned start with the PPS source armed first.
- Cite v3 as `v3/<path>` so the citations stay checkable.

## 6. How the user wants design work done

- **Adversarial, evidence-driven, minimal-core.** Verify claims against the repo and primary sources (UHD docs, source, papers). Mark what you could not verify as inference (the audit uses VERIFIED / INFERRED). "This should NOT be in Core" is a valid and welcome conclusion. Scope creep is a defect equal to a missing feature.
- **Pre-release: long-term benefit over compatibility** (owner, 2026-10-06). v4 is not released, and v4.0 has not frozen (OV-12). A breaking change is allowed when it is genuinely better for the software in the long term. This covers a schema before the freeze, the Python API, a Vocabulary or Module version, and existing fixtures. Do not choose a weaker design to save compatibility cost. Short-term effort is no reason either (owner, 2026-10-07): a long-term simplification is worth a large change, and a plan is staged only to control risk, never shrunk to save work. The discipline stays: regenerate a changed schema and log it in `schemas/SCHEMA_CHANGELOG.md`, bump versions, and migrate or refuse old documents, never silently reinterpreting them (§3; invariant 39). This is about compatibility cost; it does not reopen the settled constraints of §3.
- **Three strikes on one failure mechanism: doubt the design** (owner, 2026-10-07). Count bugs, issues and review findings alike, by the design mechanism that caused them, not by symptom or area: one mechanism can surface in several areas, and one symptom can have unrelated causes. Labels: `area:<name>` sorts an issue when it is filed; `mech:<name>` names its mechanism as soon as the root cause is clear enough, without waiting for a fix, with one line in the issue saying why. When a third bug is given one mechanism, stop patching locally: before fixing any open bug of that mechanism, write a short design note. Can the concept be removed or merged, is a rule implemented twice (in MockRadio and a hardware Module, say), is its spec text too complex? Record the outcome: an amendment spec, or the reason the design stands together with the tests that pin it (such as a differential harness). The rule asks for the question, not a rewrite; complexity the hardware really has stays. The mechanisms, their counts and where each note is: [plan/maintenance/failure-mechanisms.md](plan/maintenance/failure-mechanisms.md).
- Review-only tasks change no design documents unless asked.
- Lead with one recommendation plus the rejected alternatives, not a menu.
- Language: the Vision is English; the audit and re-review are Japanese with English technical terms. Either is fine for new documents; keep `§N` citations either way.

## 7. Git habits

- Stop exactly where asked: `git add` is not a commit, and a commit is not a push. The user usually commits themselves.
- Commit subjects in Conventional Commits style (`chore:`, `docs:`, `feat:`); the body records the decision, not the diff.
- `.DS_Store` is ignored; keep the tree free of OS and editor junk.
- Build output never goes inside the working tree. The main tree builds into `~/.cache/cargo-target/Ez-SDRv4` (untracked `.cargo/config.toml`). A review, mutation or probe build in a copy or worktree — which does not inherit that config — sets `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, shared by every reviewer; never a per-copy target directory. Cargo judges freshness by mtime and reuses any artifact newer than a copy's files, whichever copy built it — a mutated build included. So make such a copy with fresh timestamps (`rsync -a --no-times --exclude .cargo`, or `shutil.copy` rather than `copy2`), refresh them (`find <copy> -type f -exec touch {} +`) immediately before each build session in it if another copy may have built there since, and never interleave two copies' builds in that directory (Phase 5 Reviews F and G).
- Python (since Phase 6) follows the same rule: run it with `PYTHONDONTWRITEBYTECODE=1` so no `__pycache__` lands in the tree, keep virtual environments outside it (the one the Phase 6 suite uses is `~/.cache/ezsdr-venv`, Python 3.13 with numpy; macOS's `/usr/bin/python3` 3.9 has numpy too), and point `EZSDR_SERVER` at the `ezsdr-server` binary built from the same tree (`plan/phase6/00-overview.md` GY-7). `ezsdr-runs/`, the server's default Session directory, is ignored.

## 8. Subagent usage policy (Claude only)

Applies only when the agent working in this repo is Claude (Claude Code). Not applicable to Codex or other agents reading this file — they have their own subagent mechanics.

**Model selection.** The split is by what the subagent has to do, not by topic or which tool it uses:

- **Mechanical investigation → Sonnet at medium effort.** Grep/find, locating a definition or citation, pulling quotes from `v3/`, UHD docs or the Vision, checking whether a file/section/claim exists, summarizing a document. The output is a fact, not a conclusion — always Sonnet, no exceptions.
- **Anything requiring judgment → Opus at max effort.** Design/architecture review, VERIFIED vs INFERRED calls (§6), evaluating whether a claim or design decision is sound, drafting or editing design documents, recommendations, adversarial critique. The output is a conclusion, not just a fact — always Opus, no exceptions.
- **Never launch Fable as a subagent.** The one exception is when the user asks for it in that session: a **second
  opinion on a judgment Opus has already made**, where the point is that the reviewer is a different model family.
  Never the first pass, never mechanical work, and the report names the model that produced it.

**Whether to delegate at all** (owner, 2026-10-07). The model rules above say which model a subagent gets, not that work must leave the main session; when the main session already runs the right model, it does judgment work itself. Delegate only: a review whose value is independence (an implementer and a separate reviewer); a large implementation that would flood the main context; and mechanical searches. Analysis and decisions made with the owner, and small or medium investigations, stay in the main session, so it can answer follow-up questions from what it read rather than relaying a report.

**Parallelism.** Default to exactly one subagent at a time. Launching more than one in parallel requires the user's explicit permission first — ask before fanning out, every time; do not fan out and explain afterward.
