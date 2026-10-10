# Phase 2 spec 08 — The Simulation Vocabulary (`sim` 1.1.0) and the Simulation Engine

| Field | Value |
|---|---|
| Status | Accepted at Gate P (owner, 2026-09-24) and ratified at Gate X (owner, 2026-09-25; [`plan/phase2/00-overview.md`](../plan/phase2/00-overview.md) §11). Normative for `crates/ezsdr-sim` and `crates/ezsdr-sim-engine`. Amended in Phase 3 by VB-2; the SimulationChannel is spec 11. |
| Scope | The `sim` Vocabulary: the `sim.seed` and `sim.faults` environment sections, their admission checks, the deterministic PRNG every simulated model uses, and the virtual-time constants; from 1.1.0, the `sim.channel` section, whose content is spec 11. The Simulation Engine Module `ezsdr.sim-engine` 1.0.0: the discrete-event Time Authority of the Simulation class. |
| Not in scope | Wall-paced pacing (RealtimeEmulation, Y1). Drifting per-device roots (Y11). Fault kinds beyond three (each with the phase that owns its mechanism; Phase 4 §3). |
| Crates | `crates/ezsdr-sim` (library `ezsdr_sim`; depends on `ezsdr-kernel`, `serde`, `serde_json`, `schemars`); `crates/ezsdr-sim-engine` (library `ezsdr_sim_engine`; depends on `ezsdr-kernel`, `ezsdr-sim`, `serde_json`). |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

---

## 1. Purpose

Vision §15 makes the Simulation Environment a discrete-event engine: "MockRadio, SimulationChannel, MockPeripheral and FaultInjector are models scheduled on it. Determinism with a seed follows from delivering events in virtual-time order, not from threads happening to agree." Phase 1 fixed the interface (TM-16, TM-17, MA-29, MA-30) and a manual Authority for tests. This document specifies the Engine that implements it, and the small Vocabulary its models share: where the seed and the fault schedule live, and the one random generator every model draws from.

The FaultInjector of Vision §13 is not a Module here. A Module cannot call another (MA-3), so an injector could not make MockRadio overflow; instead the schedule is a Vocabulary document in the environment, and each model applies the entries that name it (SE-4). That keeps Vision §17's rule — the schedule lives in the BindingProfile — and MA-3's.

## 2. The `sim` Vocabulary

- **SE-1** The Vocabulary is `VocabularyDescriptor { id: sim, version: 1.1.0, prefix: sim, keys: [], event_kinds: [], verbs: [], checks: [sim.seed, sim.faults, sim.channel] }`, returned by `ezsdr_sim::vocabulary()`. `ezsdr_sim::register(registry, checks, kinds)` registers it, the two checks of SE-5 and spec 11's `ChannelCheck` (CH-2), in that order (Phase 3, VB-2). *Checked by `se_01_register_adds_the_descriptor_and_the_three_checks`.*
- **SE-2** `sim.seed`, when present, is a JSON integer in `0 ..= 2^64 − 1`; absent means 0. `ezsdr_sim::seed(environment) -> Result<u64, String>` is the one reader. The seed is recorded by the environment itself, verbatim (RS-43), so a Run without the section is reproducible under seed 0. *Checked by `se_02_seed_reader`.*
- **SE-3** `sim.faults`, when present, is a JSON array of `FaultEntry { at_ns: integer in 0 ..= 2^62, fault: rx_overflow | rx_sequence_error | device_lost, target: Ident }` with no other member (`deny_unknown_fields`); absent means no faults. `ezsdr_sim::faults(environment) -> Result<Vec<FaultEntry>, String>` is the one reader and returns the entries in document order. *Checked by `se_03_faults_reader`.*
- **SE-4** A fault is carried out by the Provider bound to its `target`, which is a fragment id (in practice a Spec resource name): the Provider reads `sim.faults` from `PrepareContext.environment` in `prepare` and keeps the entries whose `target` equals its fragment's id. `at_ns` counts nanoseconds after the Run's start instant T0 (KC-15), which the Provider receives as `start(Some(T0))`. `rx_overflow` is the Radio Model's overrun (RM-17), `rx_sequence_error` its sequence error (RM-18), and `device_lost` means that the Provider's first `step` at or after that instant returns `ModuleError { kind: DeviceLost }` and it produces nothing afterwards, which the coordinator turns into `DEVICE_LOST` (KC-30). Entries at one instant apply in document order. A Provider that does not implement the `sim` Vocabulary does not read the section; a fault schedule has meaning only in a simulated Run (Vision §17). *Producer obligation of each simulated Provider; MockRadio's tests are MR-20's.*
- **SE-5** `SeedCheck` (section `sim.seed`) and `FaultsCheck` (section `sim.faults`) run at `validate` only. `SeedCheck` reports one violation when SE-2's reader fails. `FaultsCheck` reports one violation when SE-3's reader fails, and one per entry whose `target` is not a key of the per-fragment configuration it receives (KA-4) — a fault naming nothing in the Run would otherwise be silently never applied. Each violation's `check` is the section and its reason begins `"SE-5: "`. *Ceiling: the per-fragment configuration holds outputs as well as resources, so a fault naming an output passes the check and no Phase 2 Sink applies it; a Sink that implements `sim` would.* *Checked by `se_05_checks`.*
- **SE-6** `SimRng` is SplitMix64. `SimRng::new(seed, stream)` sets its state to `seed XOR fnv1a64(stream.as_bytes())`, where FNV-1a 64 starts at `0xcbf29ce484222325` and multiplies by `0x100000001b3`. `next_u64` adds `0x9E3779B97F4A7C15` to the state (wrapping) and returns `z` computed from the new state as `z = (z ^ (z >> 30)) · 0xBF58476D1CE4E5B9; z = (z ^ (z >> 27)) · 0x94D049BB133111EB; z ^ (z >> 31)` (wrapping multiplications). `below(n)` for `n ≥ 1` is `next_u64() % n`. Every random draw in a Phase 2 model comes from a `SimRng` whose `stream` names what it drives (MockRadio's receive jitter uses `"<device>/rx"`), so adding a model never changes another model's sequence. The SimulationChannel's noise uses the streams `sim.channel/<rx>/<channel>` (CH-5) and MockRadio's LO phases the stream `<device>/lo` (MR-34) (Phase 3, VB-2). *Ceiling: `below` has modulo bias of at most `n / 2^64`, irrelevant for block lengths.* *Checked by `se_06_splitmix_vectors`: `SimRng::new(0, "")` yields `0xc3817c016ba4ff30, 0x100cdaacc0bc9316, 0x54c3a569ecf61b1b`; `SimRng::new(42, "mock/rx")` yields `0xf7aebfe5ed07745b, 0x0c111bab322a67d5, 0xd7ecc327e164609a`.*
- **SE-7** `ezsdr_sim::VIRTUAL_TICK_RATE_HZ = 1_000_000_000` and `ezsdr_sim::VIRTUAL_EPOCH = "sim.run_start"` are the virtual root's rate and epoch name (Y11). A model that needs the virtual root's rate reads it from the registry (`clocks.nominal_rate(time.primary_root())`) rather than assuming the constant.

## 3. The Simulation Engine Module

- **SE-8** The Module is `ModuleDescriptor { id: ezsdr.sim-engine, version: 1.0.0, kernel_api: 4.0.0, roles: [Authority], vocabularies: [{ id: sim, req: ^1.0.0 }], impl_hash: Some(ContentHash::of_bytes(b"ezsdr.sim-engine 1.0.0")) }`, returned by `ezsdr_sim_engine::descriptor()`. *Ceiling: `impl_hash` names the release, not the build; a build hash is later work.* *Checked by `se_08_descriptor_registers`.*
- **SE-9** `SimEngine::new(clocks: Arc<ClockRegistry>) -> Result<SimEngine, TimeError>` allocates one id and registers the virtual root `V = Root { tick_rate: 1_000_000_000/1, epoch: Arbitrary { set_by: "sim.run_start" } }`. `SimEngine::from_binding(binding, clocks) -> Result<SimEngine, ModuleError>` refuses a binding whose `module` is not SE-8's, whose `selector` is not empty or whose `profile` is present, and otherwise calls `new`. Its `AuthorityDescriptor` is `{ module: ezsdr.sim-engine 1.0.0, governs: [V, host.monotonic], pacing: FreeRunning }` (TM-16a). *Checked by `se_09_the_virtual_root_is_registered`.*
- **SE-10** The Engine's `TimeAuthority` (returned by `Authority::time()`, one shared object):
  - `primary_root()` is `V`; `pacing()` is `FreeRunning`;
  - `governs(d)` is true for `V`, for `host.monotonic`, and for every domain whose root is `V`; false otherwise;
  - `host.monotonic` is driven as virtual time in lockstep with `V`: both count 1 GHz ticks from 0, so `now(host.monotonic).ticks == now(V).ticks` at every instant (TM-16a1, Y11);
  - `now(d)` returns the current tick for `V` and `host.monotonic`, and the floor conversion of it for a domain derived from `V` (TM-16c); `NotGoverned` otherwise;
  - `schedule(t, f)` requires `t` governed (`NotGoverned`), converts it to a tick of `V` exactly (a `host.monotonic` tick is the same tick; a derived instant must be a root tick, else `Inexact`, TM-16c), refuses a tick before `now` (`InPast`), and queues `f` under `(tick, seq)` with `seq` strictly increasing;
  - `cancel(h)` removes a pending callback and reports whether it was pending;
  - `wait_until(t)` refuses an ungoverned `t`, returns at once when `now ≥ t`, and otherwise blocks until `next_wakeup` advances past `t` (TM-16d; no Phase 2 Module calls it).

  *Checked by `se_10_time_authority_contract`.*
- **SE-11** `Authority::next_wakeup()` (KA-11): with nothing pending it returns `None` and changes nothing. Otherwise it sets `now` to the smallest pending tick `t`, then repeatedly removes the pending entry with the smallest `(tick, seq)` whose tick is `t` and calls it with `TimePoint(V, t)` — outside the Engine's lock, so the callback may call `now`, `schedule` and `cancel` — until none is left at `t` or `CALLBACK_CAP = 1000` have run in this call; it wakes every `wait_until` waiter and returns `Some(TimePoint(V, t))`. A callback that schedules at `t` is run in the same call, subject to the cap; entries left at `t` by the cap are run by the next call, which returns `t` again, and KC-22 turns a thousand such calls into `STEP_LIVELOCK`. *Checked by `se_11_next_wakeup_order_ties_and_cap`.*
- **SE-12** The documents `FaultEntry`, the seed (a `u64`) and, from 1.1.0, spec 11's `ChannelSpec` have committed schemas `schemas/sim/fault_entry.v1.json`, `schemas/sim/seed.v1.json` and `schemas/sim/channel.v1.json` (PO-7; Phase 3, VB-2). *Checked by `se_12_schema_freeze`.*
- **SE-13** The Engine draws no random number and reads no clock but its own; the order in which it fires callbacks is a function of `(tick, seq)` alone. *Checked by `se_13_two_engines_fed_the_same_schedule_fire_identically`.*

## 4. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| S1 | FaultInjector | A Vocabulary document each model applies to itself (SE-4) | A FaultInjector Module (MA-3: it cannot reach MockRadio); Kernel-delivered fault Actions (the closed Action set, RS-48, has no member for it and should not) | A later phase adds a kind to the same section with the mechanism it needs (Phase 4's re-marking: `plan/phase4/00-overview.md` §3) |
| S2 | Seed location | `sim.seed` in the environment (SE-2) | An envelope field (RS-43 refuses it); a selector field (then two Modules could disagree about the Run's seed) | none |
| S3 | PRNG | SplitMix64, hand-written (SE-6) | `rand` (a dependency, and its default generators change between versions, which would change recorded Runs) | A second generator is a new `SimRng` constructor, never a change to this one |
| S4 | Streams of randomness | One `SimRng` per named stream, seeded from the seed and the name (SE-6) | One shared generator (adding a model would reorder every other model's draws) | none |
| S5 | Virtual root rate | 1 GHz (Y11) | The device's master clock (two devices with different clocks would need two roots); a rate chosen per Run (every Spec's sample grid would move with it) | TM-3 caps any root at 2^31 per term; 1 GHz is below it |
| S6 | `host.monotonic` in Simulation | Lockstep with `V`, tick for tick (TM-16a1) | Independent (a budget evaluated in `host.monotonic` would then need a conversion with no exact path) | none |
| S7 | Where the advance happens | Inside `next_wakeup` (KA-11) | A separate `advance_to` on `Authority` (a trait change the MA-30 loop does not need) | none |

## 5. Tests

`crates/ezsdr-sim/tests/sim_vocabulary.rs`:

| test | input | expected | rules |
|---|---|---|---|
| `se_01_register_adds_the_descriptor_and_the_three_checks` | empty registries, `register` | `sim 1.1.0` registered with the checks `sim.seed`, `sim.faults` and `sim.channel`; a present `sim.seed` section is checked | SE-1 |
| `se_02_seed_reader` | absent; `42`; `18446744073709551615`; `-1`; `"42"`; `1.5` | 0; 42; `u64::MAX`; three errors | SE-2 |
| `se_03_faults_reader` | absent; one entry of each fault; an unknown fault; an extra member; a negative `at_ns`; a non-array | `[]`; three entries in order; four errors | SE-3 |
| `se_05_checks` | a valid seed and schedule targeting `radio`; a schedule targeting `nobody`; a malformed seed | no violation; one naming `nobody`; one on `sim.seed` | SE-5, KA-4 |
| `se_06_splitmix_vectors` | SE-6's two streams | SE-6's six values; `below(10)` always `< 10` over 10 000 draws | SE-6 |
| `se_12_schema_freeze` | regenerate | byte-equal to the three `schemas/sim/*.v1.json`; no other file | SE-12, PO-7, CH-10 |

`crates/ezsdr-sim-engine/tests/sim_engine.rs` (dev-dependency: `ezsdr-kernel` with `testing` for nothing but the shared fixtures; the Engine is tested directly):

| test | input | expected | rules |
|---|---|---|---|
| `se_08_descriptor_registers` | `descriptor()` into a registry with `sim` | registers; roles `[Authority]` | SE-8 |
| `se_09_the_virtual_root_is_registered` | `SimEngine::new` on a fresh registry | a Root at 1 GHz with epoch `sim.run_start`; `governs` lists it and `host.monotonic`; `from_binding` refuses a selector key and a profile | SE-9 |
| `se_10_time_authority_contract` | after advancing to root tick 7: `now` on `V`, `host.monotonic`, a derived domain of ratio 3 at origin 1, and an unrelated root; `schedule` at tick 5, at tick 1 of a derived domain of ratio 2/3 (root tick 1 + 2/3), and in the unrelated root | 7, 7, 2 (floor of 6/3), `NotGoverned`; `InPast`, `Inexact`, `NotGoverned` | SE-10, TM-16c |
| `se_10_host_monotonic_advances_in_lockstep` | a callback at `V` tick 1 000 000, `next_wakeup` | `now(host.monotonic).ticks == 1_000_000` | SE-10, TM-16a1 |
| `se_11_next_wakeup_order_ties_and_cap` | schedule 30, 10, 10, then call `next_wakeup` three times; a callback at 10 that schedules another at 10; a callback that reschedules itself at its own tick | returns 10 (both fire, insertion order, each observing `now == 10`), 30, then `None`; the nested one fires in the same call; the self-rescheduler returns 10 on every call and never hangs | SE-11 |
| `se_11_empty_is_none_and_moves_nothing` | a fresh Engine | `None`; `now` still 0 | SE-11 |
| `se_13_two_engines_fed_the_same_schedule_fire_identically` | two Engines, 1 000 callbacks at pseudo-random ticks from `SimRng::new(1, "t")` scheduled in the same order | identical firing logs | SE-13 |

## 6. Vision issues found

1. **§13's Simulation Environment lists a FaultInjector** as a component beside MockRadio. With MA-3 it is a document, not a Module (S1); §13 and §17 should say so.
2. **§8's example writes `sim.faults` entries as `{ at: "2.5s", inject: rx_overflow, target: radio[0].rx }`**. SE-3's normative form is `{ at_ns: 2500000000, fault: rx_overflow, target: radio }`: an integer nanosecond offset from T0 (TM-1 forbids float seconds in a document), and a fragment id rather than an index expression.

## 7. Deferred

Wall-paced pacing and the RealtimeEmulation class. Per-device drifting roots. More fault kinds, each with the phase that owns its mechanism ([`plan/phase4/00-overview.md`](../plan/phase4/00-overview.md) §3).
