# Phase 5 spec 14 — The native Executor (`ezsdr.exec.native` 1.0.0)

| Field | Value |
|---|---|
| Status | **Draft, implemented under the owner's delegation** ([`00-overview.md`](00-overview.md) §11). Normative for `crates/ezsdr-exec-native` once accepted at Gate X; moves to `design/14-native-executor.md` at Step X. |
| Scope | The Executor Module `ezsdr.exec.native` 1.0.0: its descriptors, how it loads a component by its `impl` identity, `prepare`, the stepping of its components, how it submits their Actions and handles refusals, the Actions addressed to it, `stop` and `cleanup`; and its component ABI (`Component`, `ComponentContext`, `Implementation`). |
| Not in scope | Components that apply Actions (Phase 10). Threaded execution, RealtimeEmulation and hardware (KC-2; Phase 7 onwards). A component's processing time in virtual time (`00-overview.md` R4; Phase 10). WASM and GPU Executors (Phase 11). Event edges (`00-overview.md` §3). |
| Crate | `crates/ezsdr-exec-native`, library `ezsdr_exec_native`. Depends on `ezsdr-kernel` and `serde_json`. Dev-dependency: `ezsdr-kernel` with `testing`. |
| Vision § covered | §19 ("Concrete execution engines remain Modules"; the Executor owns its ABI); §20 (placement validated, never selected); §32's driving model for the Simulation class. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

---

## 1. Purpose

Vision §19 puts the descriptor of a component and the Actions it speaks in the Kernel and the way it is called in an Executor: "The execution ABI — how a native, WASM or GPU component is invoked, how buffers are handed to it — belongs to each Executor." Phases 1–4 built the Kernel side and tested it with doubles; MA-19b's producer obligation — "the Executor loads each component by its `impl` identity and returns a `PrepareReport` for the Island" — was left "tested by the Phase 5 Reactor Executor". This document is that Executor: the smallest one that runs a Reactor in a deterministic Simulation Run.

It is deliberately small. It runs compiled-in Rust components, step-driven, in the Simulation class only, and applies no Action. Phase 10's execution engines are where budgets, threads and applied parameters belong.

## 2. Evidence

- **MA-19b is the only Phase 1 Module-API obligation still marked producer with no producer** (`plan/phase1/exit-review/05-module-api.md`: "Phase 2 Simulation Engine and Phase 5 Reactor Executor"; the Simulation Engine is an Authority and loads nothing).
- **The coordinator already drives an Executor end to end**: KC-10 attaches a component's link ends to its Island fragment, KC-11 hands the Island exactly its descriptors (MA-19a), KC-23 steps every Executor in MA-30's order, KC-24 admits what it submits, KC-39 stops and cleans it up. `ProbeExecutor` in `crates/ezsdr-kernel/tests/support/run_doubles.rs` exercises each; it loads nothing.
- **The prototype** (`00-overview.md` §2): this Executor with the responder of `00-overview.md` §7 ran a PING → PONG Run through the real coordinator, and showed that a component submitting its own Actions reports a refusal during the orderly drain as an error, which the coordinator records as an abort (KE-3). NX-6 is the answer.

## 3. Types

```rust
pub const MODULE_ID: &str = "ezsdr.exec.native";
pub const EXECUTOR_KIND: &str = "ezsdr.exec.native";
pub const IMPL_KIND: &str = "ezsdr.impl.native";

pub fn descriptor() -> ModuleDescriptor;                 // NX-1
pub fn executor_descriptor() -> ExecutorDescriptor;      // NX-2

/// The execution ABI this Executor owns (MA-21).
pub trait Component: Send {
    fn prepare(&mut self, ctx: ComponentContext) -> Result<(), ModuleError>;
    fn step(&mut self, until: TimePoint, out: &mut Vec<Action>) -> Result<StepOutcome, ModuleError>;
    fn stop(&mut self, _mode: StopMode) -> Result<(), ModuleError> { Ok(()) }
    fn cleanup(&mut self) {}
}

pub struct ComponentContext {
    pub descriptor: ComponentDescriptor,     // the Spec graph's, parameters included (MA-36)
    pub class: ExecutionClass,
    pub time: Arc<dyn TimeAuthority>,
    pub clocks: Arc<ClockRegistry>,
    pub events: Arc<dyn EventSink>,
    pub inputs: Arc<dyn InputStore>,
    pub links: Vec<AttachedPort>,            // this component's link ends only
    pub source: ResourceId,                  // island_<n>, the Island's event source (KC-8)
}

#[derive(Clone)]
pub struct Implementation {
    pub id: String,                          // the impl.id a descriptor names
    pub hash: ContentHash,                   // the impl.hash it must name
    pub make: fn() -> Box<dyn Component>,
}

pub struct NativeExecutor { /* private */ }
impl NativeExecutor {
    pub fn new(implementations: Vec<Implementation>) -> Result<NativeExecutor, ModuleError>;  // NX-3
}
impl Executor for NativeExecutor { /* NX-4 … NX-8 */ }
```

A `ComponentContext` carries no `ActionSubmitter`: a component's Actions go through its Executor (NX-6).

## 4. Rules

### Identity

- **NX-1** The Module is `ModuleDescriptor { id: ezsdr.exec.native, version: 1.0.0, kernel_api: 4.0.0, roles: [Executor], vocabularies: [], deployment: InProcess, impl_hash: Some(ContentHash::of_bytes(b"ezsdr.exec.native 1.0.0")) }`, returned by `ezsdr_exec_native::descriptor()`. It declares no Vocabulary: it has no keys, no event kinds and no verbs of its own. *Checked by `nx_01_descriptor_registers`.*
- **NX-2** `executor_descriptor()` is `ExecutorDescriptor { module: { ezsdr.exec.native, 1.0.0 }, kind: ezsdr.exec.native, memory_domains: [HOST_MEMORY] (MemoryDomainId::local(0), HD-1), impl_kinds: [ezsdr.impl.native], capabilities: {} }`, and `NativeExecutor::descriptor()` returns it. Admission reads the kind, the domains and the implementation kinds (MA-18, MA-39). *Checked by `nx_01_descriptor_registers`.*

### Loading

- **NX-3** `NativeExecutor::new(implementations)` takes the compiled-in implementations the runtime assembles, as it assembles Modules (MA-32: explicitly, no link-time registration), and refuses two with one `id` (`Rejected`, "NX-3: two implementations are registered as <id>"): which one a Spec meant would be undecidable. `prepare` loads each component of the Island by the descriptor's `impl`: `impl.kind` must be `ezsdr.impl.native`, `impl.id` must name an implementation, and `impl.hash` must equal that implementation's `hash`; otherwise `prepare` returns `Rejected` naming the component and the mismatch, and nothing is built. The hash is what the Manifest records for the component (KC-45, RS-45), so a Run whose Spec names one version of a component cannot run another. This carries MA-19b's producer obligation. *Checked by `nx_03_a_component_is_loaded_by_its_impl_identity` (an unknown id, a wrong hash and a wrong kind each refused, naming the component; a duplicate registration refused; the right identity built and prepared).*

### `prepare`

- **NX-4** `prepare(island, ctx)`:
  1. `ctx.class` other than Simulation returns `Unsupported` ("NX-4: … runs the Simulation class only"): the components are stepped, which only the Simulation class does (MA-30's table).
  2. Every `ctx.links` entry must name a component of `island.components`; one that does not is `Rejected`.
  3. For each component of `island.components`, in the Island's order: a component this Executor already holds (from another Island) is `Rejected`; its descriptor is `ctx.components[id]`, and a missing one is `Rejected`; it is loaded (NX-3); `(implementation.make)()` builds it, and the Executor keeps it **before** calling its `prepare`, so that `stop` and `cleanup` reach a component whose `prepare` failed, as MA-7 has them reach every instance that reached `prepare`; its `prepare` receives a `ComponentContext` whose `descriptor` is the Spec's, whose `links` are exactly the `ctx.links` entries naming it, whose `source` is `island_<island.id.local>`, and whose other handles are `ctx`'s. A component's error is `prepare`'s error.
  4. The Executor keeps `ctx.actions` and `ctx.actions_out` (MA-5a).
  5. It returns `PrepareReport { fragment: island_<island.id.local>, effective: {}, coercions: [], warnings: [] }`.

  One Executor may run several Islands (SB-22a); `prepare` is then called once per Island (MA-7), and each call adds that Island's components. *Checked by `nx_04_prepare_cases` (a non-Simulation class `Unsupported`; a link end for a component outside the Island refused; a component placed on two Islands refused; each component receives only its own link ends and its Island's source; two Islands on one Executor; a component whose `prepare` fails fails the Island's and is still cleaned up).*

### Stepping

- **NX-5** `step(until)` first reads its Action queue (NX-7), then steps every component it holds in component-id order — the order of a `BTreeMap` keyed by `Ident`, across all its Islands — calling `step(until, out)` with an empty `out`. After each component's step, and before the next component's, it submits the Actions that component pushed, in the order pushed (NX-6). It reports `progressed` when any component reported it or pushed an Action (MA-20). Components are stepped under MA-20's contract, which the Executor passes on: consume every input at or before `until`, emit nothing after it, never block, never call `wait_until`. The order is fixed by component id, not by registration or Island order, so a Run's decisions do not depend on how the runtime assembled it (Vision §58 #3). *Checked by `nx_05_components_step_in_id_order_and_their_actions_follow_each_step`.*
- **NX-6** Each Action is submitted through `ctx.actions_out` (MA-14a). A refusal that names at least one violation, **every** one of which has the check `ezsdr.dispatch` or `ezsdr.run_state`, is dropped: those are KC-24's steps 1 and 2, which say the Run is ending (RS-6 step 1, RS-18), so the decision came too late to act on and the termination already says why; reporting it would record a clean stop as an abort (KC-32; spec 15 KE-3). Any other refusal fails the step with `Rejected`, "NX-6: component <id>'s <Action kind> was refused: <check>: <reason>; …", which the coordinator records as `Failed { run }` naming the Island (KC-30). *Ceiling: a decision refused because the Run was ending is recorded nowhere; an Executor has no Manifest section (KA-13 is the Provider's).* *Checked by `nx_06_a_refusal_because_the_run_is_ending_is_not_a_failure` (a submitter refusing with each of the two checks: the step succeeds) and `nx_06_any_other_refusal_fails_the_step` (a refusal with `ezsdr.target`, and one mixing `ezsdr.dispatch` with `ezsdr.input`: the step fails naming the component, the Action and each check), and end to end by `ke_03_a_decision_after_the_stop_is_not_an_abort`.*
- **NX-7** The Executor applies no Action. An Action in its queue — an `UpdateParameter` of a component parameter, a `SetTimer`, a `Stop` of a component — fails the step with `Rejected`, "NX-7: ezsdr.exec.native 1.0.0 applies no Action, and <kind> for <target> reached it", before any component is stepped: MA-14 forbids accepting it silently, and the Executor has no Vocabulary event to report it with. *Ceiling: the Kernel admits an update of a component parameter, because the parameter declares its class (RS-17); this Executor refuses it at run time. An Executor that applies UC-2…UC-6 to component parameters is Phase 10's.* *Checked by `nx_07_an_action_addressed_to_the_executor_fails_the_step`.*

### Ending

- **NX-8** `arm` and `start` do nothing and succeed. `stop(mode)` calls every component's `stop(mode)` in component-id order, all of them even after an error, and returns the first error. `cleanup()` calls every component's `cleanup()`, drops every component and every kept handle, and is idempotent (MA-7). *Checked by `nx_08_stop_reaches_every_component_and_cleanup_is_idempotent`.*
- **NX-9** The Executor has no randomness and no time source but the components' `ctx.time`, and iterates ordered collections only (PO-11): two Executors prepared with the same Island and fed the same blocks submit the same Actions in the same order. *Checked by `po_11_no_hashmap_and_no_wall_clock_in_simulation_code` (the crate is in its list) and, end to end, `v58_03_reactor_decisions_reproduce_with_their_seed`.*

## 5. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| N1 | How components are found | A table of compiled-in `Implementation`s handed to `new`, checked against `impl.kind`, `impl.id` and `impl.hash` (NX-3) | Dynamic loading (Rust has no stable ABI, Vision §62; it is the Plugin path); a global registry filled at link time (MA-32 rejects it for Modules for the same reasons) | Phase 10's native engine may add a loader; the table stays the explicit form |
| N2 | How a component emits an Action | Pushes it onto `out`; the Executor submits (NX-5, NX-6) | Each component holding `actions_out` (`00-overview.md` R6: each would have to know KC-24's refusal checks) | A component that must learn an admission result |
| N3 | Actions addressed to a component | Refused, failing the step (NX-7) | Applying them (no Phase 5 path sends one); dropping them (MA-14) | Phase 10 |
| N4 | Execution classes | Simulation only (NX-4) | Accepting any class and stepping anyway (the other classes run Executors on threads, MA-30's table) | Phase 7 onwards, with KC-2's threaded driver |
| N5 | `PrepareReport.effective` | Empty | The components' parameter defaults (two components of one Island may share a key, and the Spec already records every descriptor, KC-45) | An Executor that applies parameters reports what it applied |
| N6 | Where a component's `prepare` error goes | It is `prepare`'s error, so the Run fails `Failed { prepare }` naming the Island (KC-12) | Skipping the component (a Run would lack a component its Spec declares) | none |

## 6. Tests

In `crates/ezsdr-exec-native/tests/native_executor.rs`, with a harness in the style of spec 09's: a `ManualTimeAuthority` on a 1 GHz root, an `EventCollector`, a queue `ActionReceiver`, a recording `ActionSubmitter` whose answer each test sets, and test components that record their calls and push the Actions a test gives them.

| test | expected | rules |
|---|---|---|
| `nx_01_descriptor_registers` | NX-1's descriptor registers with the Executor factory; NX-2's descriptor, and `NativeExecutor::descriptor()` returns it | NX-1, NX-2 |
| `nx_03_a_component_is_loaded_by_its_impl_identity` | an unknown `impl.id`, a wrong `impl.hash`, a wrong `impl.kind` each refused naming the component, and nothing built; a duplicate id refused by `new`; the right identity built and its `prepare` called with its descriptor | NX-3, MA-19b |
| `nx_04_prepare_cases` | a RealtimeEmulation class `Unsupported`; a link end naming a component outside the Island refused; a component on two Islands refused; each component sees only its own link ends and the source `island_<n>`; two Islands on one Executor each report their own fragment; a component whose `prepare` fails fails the Island's `prepare` and is cleaned up | NX-4, MA-7 |
| `nx_05_components_step_in_id_order_and_their_actions_follow_each_step` | components `b` and `a` registered in that order step `a` then `b`; `a`'s Actions are submitted before `b` is stepped; `progressed` is true when only an Action was pushed and false when nothing happened | NX-5, MA-20 |
| `nx_06_a_refusal_because_the_run_is_ending_is_not_a_failure` | refusals with `ezsdr.dispatch` and with `ezsdr.run_state` leave the step `Ok` | NX-6, KE-3 |
| `nx_06_any_other_refusal_fails_the_step` | a refusal with `ezsdr.target`, one with `ezsdr.dispatch` and `ezsdr.input` together, and one naming no check, fail the step with a message naming the component, the Action kind and each check | NX-6 |
| `nx_07_an_action_addressed_to_the_executor_fails_the_step` | an `UpdateParameter` in the queue fails the step naming it; no component was stepped | NX-7, MA-14 |
| `nx_08_stop_reaches_every_component_and_cleanup_is_idempotent` | a component failing `stop` does not keep the next from being stopped, and its error is returned; `cleanup` twice is harmless, and a later `step` steps nothing | NX-8, MA-7 |

The end-to-end carriers, which run this Executor with the responder through the coordinator, are in `00-overview.md` §8 and spec 15.

## 7. Vision coverage

| Vision | This spec |
|---|---|
| §19 "Concrete execution engines remain Modules"; the ABI belongs to each Executor | §3, NX-3, NX-5 |
| §20 placement is explicit and validated | NX-2 (what admission reads), NX-4 |
| §32 the Simulation class steps every Island | NX-4, NX-5 |
| §58 #3, #9 | NX-5, NX-6, NX-9 and `00-overview.md` §8 |

## 8. Vision issues found

1. **§19 says "Concrete execution engines remain Modules"** and names none. A `Normative:` line naming this spec as the first (re-review R13).

## 9. Deferred

Applying Actions to components, including updates of component parameters under their classes (Phase 10). A component's processing time charged in virtual time (Phase 10). Threads, and the classes other than Simulation (Phase 7 onwards). Event edges and their queue (Phase 10). A Manifest section of an Executor's own (when one needs it).
