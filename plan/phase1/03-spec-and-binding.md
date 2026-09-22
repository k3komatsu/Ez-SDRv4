# Phase 1 spec 03 — ExperimentSpec and BindingProfile

| Field | Value |
|---|---|
| Status | Draft for Gate B. Normative for `ezsdr-kernel::{spec, binding, plan}` once accepted. |
| Scope | The identifier, key, value and constraint model; the ExperimentSpec envelope; the BindingProfile with its bindings, placements and environment; the admission-check hook through which a Vocabulary enforces something like an RF envelope; composite resource resolution and binding; the compile pipeline from `validate` to `arm`; the PrepareReport and the coercion policy; document versioning. |
| Not in scope | Radio Model keys and the fields of `rf_envelope` (Vocabulary, Phase 2); the simulation `environment` sections (Phases 2 and 3); the Run state machine, Actions, Lease, Policy and the Manifest (spec 04); the role traits and `ComponentDescriptor` (spec 05); the Python builder (Phase 6). |
| Vision § covered | §8; §9; §10; §11; §20's placement rules; §31's three-part admission; §33's profiles as hints; §52 including the RF safety envelope. |
| Audit §14.1 items | 6 (composite tree, arm-order DAG), 7 (BindingProfile = bindings + placements + environment), 8 (generic matcher and PrepareReport), 10 (schema-first and versioning), 13 in part (placement validated, never optimised). Findings 6, 7, 10, 11, 13, 17, 23. |
| Re-review | R1 (per-direction requests), R4 (pipeline order), R10 (the Spec tree), R16 (coercion defaults per Run kind), R21 (placement in the BindingProfile only). |
| Depends on | Spec 01 for `TimePoint`, `Duration`, `TM-21`; spec 02 for `DataContractId`, `MemoryDomainId`, SC-3, SC-21, SC-23a, SC-27; spec 05 for `ComponentDescriptor`, `IslandDecl`, `ProviderInstance` and the registry. |
| Modal verbs | "must" and "must not" are normative (OV-4a). "Should" does not appear inside a rule. |

---

## 1. Purpose

Vision §8 and §10 make one separation load-bearing: an ExperimentSpec says what an experiment requires, a BindingProfile says how that requirement is met and what surrounds it, and the same Spec runs in simulation and on hardware because only the profile changes. This document turns that into two envelopes, a generic matcher that knows no radio words, and a pipeline whose stages each have a defined input, output and failure.

Three properties have to survive the encoding. The Kernel must not learn a single Radio Model key, or Finding 7's escape hatch reopens. Placement and environment must be impossible to write in a Spec, or R21's regression returns and a Spec stops being promotable. And a document must be migrated or refused, never reinterpreted, or v3's three configuration formats grow a fourth.

## 2. Evidence

- **v3 grew three configuration formats and a chain of converters.** `v3/source/app.d:103-108` dispatches on a missing `version` key to a v1-to-v2 conversion (`:222-268`), normalises v2 (`:309-330`), and the v2-to-v3 call site is commented out, so a v2 document is normalised and then handed to code that indexes keys the v1 and v2 shapes do not have. Vision §10 states the policy before the first schema exists, and this document implements it.
- **v3's configuration became a small language.** `v3/source/settingfile.d:12-87` detects `CONSTANTS` references and `!COMPUTE(expr)` and re-walks the document until nothing expands; the four-operator evaluator it calls is at `:147-226`. `v3/changelog/v3.0.21.md:5-13` documents the feature. The motivation usually given for it, that users wanted parameter sweeps without duplicating files, is Vision §9's account and is **not** stated in the changelog, which has no background section: treat it as the Vision's reading rather than as evidence from the repository. Vision §9 rejects expressions in the Spec and puts parametrisation in a client-side builder; SB-14 makes the rejection mechanical.
- **v3's protocol version had to match exactly.** `v3/source/tcp_iface.d:22` and `:255-261` drop a connection whose version string is not `"3.0.11"`. Exact-match is one of the two honest options; this document takes the other, migrate-or-refuse, and keeps the refusal explicit.
- **Start-up order was a real failure mode.** `v3/changelog/v3.0.20.md:56-59` records, under a heading that reads "background of the change", that with a shared PPS the devices must be started beginning with the one that sources it or start-up fails, and that v3 answered by letting the user specify the order. Audit Finding 6 attributes this to `v3.0.17.md`, which is about the RFNoC device and does not mention PPS; the confusion is explicable, because `v3.0.20.md`'s own first line reads `# EzSDR v3.0.17`. SB-39 keeps v3's answer and moves it: the order is a property of the plan, derived from explicit edges and Provider declarations, rather than of the operator's memory. (Audit Finding 6 attributes the note to `v3.0.17.md`, which introduces the RFNoC device instead; the correct citation is `v3.0.20.md`.)
- **UHD coerces silently unless you read back.** A request for 19.5 Msps on an X310 is applied as 20 Msps, and UHD documents reading `get_rx_rate()` afterwards. `v3/cpp/uhd_usrp/multiusrp.cpp` prints the actual rate to standard output and nothing reaches the client. SB-44 and SB-46 make the coercion a document.

## 3. Model

```text
ExperimentSpec            intent, portable                BindingProfile        this site, this run
├── requirements          vocabulary majors               ├── bindings          resource -> Provider instance
├── resources             per-direction requirements      ├── authority         which binding keeps time
├── graph                 components and links            ├── placements        islands, components, links
├── schedule              actions at relative times       └── environment       namespaced sections
├── outputs               artifacts to produce                 ezsdr.time, ezsdr.rf_path,
├── policies              failure table, coercion policy       radio.rf_envelope, sim.channel, ...
└── extensions            namespaced
        │                                                              │
        └──────────────────────────┬───────────────────────────────────┘
                                   ▼
   validate → plan → prepare → arm         each stage defined in §6
   AdmissionResult   ExecutionPlan   PrepareReport
```

The Kernel reads the envelope and the shapes. Everything inside `requires`, `selector`, `environment` and `extensions` is namespaced content whose meaning belongs to a Vocabulary or to a Module.

## 4. Types

```text
Ident            ^[a-z][a-z0-9_]*$                              a name inside one document
Namespace        ^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$          a dotted prefix owned by a Vocabulary or Module
Key              <vocabulary prefix><path> | ext.<module-id>.<path>
Value            Bool | Int(signed 64-bit) | Num(float64, finite) | Str
                 | List[Value] | Map<Ident, Value>              List and Map nest one level only
Constraint       Eq(Value) | Range { min: optional, max: optional } | Set([Value])
                 | Min(Value) | Max(Value) | Present
CapabilityValue  One(Value) | Range { min, max } | AnyOf([Value])     declared by a Provider (spec 05)
KeyDecl          { key: Key, kind: bool|int|num|str|list|map, coercible: bool,
                   coercion_default: accept | warn | reject,
                   update_class: optional UpdateClass }          registered by a Vocabulary (spec 05)

ExperimentSpec   { version: 1,
                   requirements: { vocabularies: [{ id: Namespace, major: int }] },
                   resources: Map<Ident, ResourceReq>,
                   graph: { components: Map<Ident, ComponentDescriptor>, links: [LinkReq] },
                   schedule: [{ at: SpecTime, action: ActionTemplate }],
                   outputs: [OutputReq],
                   policies: { failure: Map<EventKind, Reaction>, coercion: Map<Key, CoercionPolicy> },
                   extensions: Map<Namespace, Value> }
ResourceReq      { kind: Namespace, requires: Map<Key, Constraint>,
                   needs: Map<Ident, SubResourceReq>, extensions: Map<Namespace, Value> }
SubResourceReq   { kind: Namespace, requires: Map<Key, Constraint> }
                 a capability this resource needs from somewhere, possibly another instance
LinkReq          { from: PortRef, to: PortRef, policy: BackPressure, capacity: u32 }
                 both mandatory: SC-19 forbids a default, and SC-21's Sink rule depends on the policy
SpecTime         { clock: Ident, offset_ticks: signed 64-bit }   a resource name plus an offset from Run start
SinkFeed         { port: PortRef, policy: BackPressure, capacity: u32 }
                 the link that feeds a Sink; policy is drop-class only (SC-19, SC-21)
OutputReq        { id: Ident, kind: Namespace, feed: SinkFeed, params: Map<Key, Value> }
                 an output *is* the declaration of the link its Sink is fed by
CoercionPolicy   accept | warn | reject

BindingProfile   { version: 1,
                   bindings: Map<Ident, Binding>,
                   authority: optional Ident,
                   placements: { islands: [IslandDecl],
                                 components: Map<Ident, { island: Ident, memory_domain: MemoryDomainId }>,
                                 links: Map<LinkKey, { link: ModuleId, memory_domain: optional }> },
                   environment: Map<Namespace, Value> }
Binding          { module: ModuleId, selector: Map<Ident, Value>, profile: optional { name, version },
                   feed: optional SinkFeed }
                 one map binds all three roles; `feed` is a Session Sink binding's only extra field

AdmissionCheck   { section: Namespace, stages: [validate | prepare | runtime],
                   check(section: Value, effective: Map<Key, Value>,
                         proposed: Map<Key, Value>, stage) -> [Violation] }
                 proposed is empty at validate and prepare; at the runtime stage it carries the
                 Action's key and value, and the check evaluates effective overlaid with proposed
Violation        { check: Namespace, key: optional Key, requested: optional Value, reason: string }

AdmissionResult  { matched: Map<Ident, ResourceId>, rejected: [RejectedConstraint],
                   violations: [Violation], coercions_preview: [Coercion], warnings: [Warning] }
ExecutionPlan    { fragments: [Fragment], links: [DataLinkDecl], deps: [(Ident, Ident)],
                   authority: Ident, class: ExecutionClass, transfer_costs: [DeclaredCost] }
Fragment         { id: Ident, instance: ModuleId, role: Role, content: Value, after: [Ident] }
Coercion         { key: Key, requested: Value, applied: Value, reason: string }
PrepareReport    { fragment: Ident, effective: Map<Key, Value>, coercions: [Coercion],
                   warnings: [Warning], constraints_hit: [Key] }
SpecError        UnsupportedVersion { found, supported } | UnknownField { path }
                 | PlacementInSpec { path } | EnvironmentInSpec { path }
                 | UnknownKeyPrefix { key } | KeyShape { key, expected, found }
                 | UnboundResource { name } | NoSingleInstance { name, constraint }
                 | ArmCycle { path } | Violation(Violation) | CoercionRejected(Coercion)
```

## 5. Normative rules

### Identifiers, keys and values

- **SB-1** An `Ident` names something inside one document and matches `^[a-z][a-z0-9_]*$`. A `Namespace` is a dotted sequence of such segments. Names are compared as bytes; there is no case folding and no Unicode normalisation, so the canonical form of `00-overview.md` OV-15 stays byte-comparable.
- **SB-2** A `Key` is a `Namespace` prefix owned by a declared Vocabulary, followed by a path, or `ext.` followed by a Module id and a path. `validate()` refuses a key whose prefix belongs to no Vocabulary the owning document declares in `requirements`, with `UnknownKeyPrefix`. The Kernel checks the prefix and the value's shape against the `KeyDecl` and never interprets the meaning (audit Finding 7). A `KeyDecl` also carries the optional `update_class` under which the key may be changed during a Run, absent meaning it may not be. This is where a **Provider** parameter's class is declared, and the only place it can be: Vision §27's own examples of runtime mutation — TX gain, antenna beam, MCS — and Vision §3's `sdr.rx.gain = 20` all target a Provider, whose parameters are Vocabulary keys and never a `ComponentDescriptor`'s `params`, so RS-17 would otherwise have nothing to consult and the Easy API's first parameter change would be refused as undeclared.
- **SB-3** A `ResourceId` is `{ node: NodeId, path: [Ident] }`. The path addresses a node of the composite resource tree: `["radio"]` is a bound device, `["radio", "rx", "0"]` one of its channels. Vision §49 writes `{node, local}`; a flat local name cannot address a sub-resource, and §8 requires sub-resources to be bindable. The node qualification §49 asks for is unchanged, and in v4.0 `node` is `LOCAL`.
- **SB-4** A `Value` is a scalar, or a list or map of scalars nested at most one level. Deeper nesting is refused. A Spec is data, not a document tree: one level is what a structured parameter such as a capture request needs, and more invites the schema-inside-a-schema that Vision §9 rejects.
- **SB-5** A `Constraint` is one of `Eq`, `Range`, `Set`, `Min`, `Max`, `Present`, and its value is a scalar. A constraint over a list or a map is refused, because the matcher would then need the Vocabulary's semantics to compare them.

### Matching

- **SB-6** A constraint is satisfied by a declared `CapabilityValue` as follows. `Eq(v)` by `One(v)`, by a `Range` containing `v`, or by an `AnyOf` containing `v`. `Range{min,max}` by a `One` inside it, by an overlapping `Range`, or by an `AnyOf` with a member inside it. `Set(s)` by any capability value whose possible values intersect `s`. `Min` and `Max` by the corresponding bound. `Present` by the key being declared at all. Every comparison is between scalars of the `KeyDecl`'s kind; a mismatch is `KeyShape`.
- **SB-7** When a key's `KeyDecl` says it is coercible, a constraint the declared capability does not satisfy directly is offered to the Provider's `coerce` (spec 05, MA-11). The matcher asks only whether a value can be coerced and what it becomes; it never decides the grid. A key that is not coercible fails immediately.
- **SB-8** The matcher is generic. No Kernel item names a radio, a channel, a sample rate, a gain or a frequency, and the Phase 1 test double proves it by declaring only `test.count`, `test.grid` and `test.flag` (`00-overview.md` OV-21).

### ExperimentSpec

- **SB-9** The top level of an ExperimentSpec is the closed set `version, requirements, resources, graph, schedule, outputs, policies, extensions`. An unknown top-level field is `UnknownField`, not a warning. v3's configuration accreted keys nobody removed; a closed envelope with a namespaced `extensions` map is how a document stays extensible without becoming unbounded.
- **SB-10** `version` is a positive integer and is mandatory. Phase 1 supports exactly `{1}`.
- **SB-11** `requirements.vocabularies` lists the Vocabulary majors this Spec's keys belong to. It is the only content of `requirements`; the per-resource requirements live in `resources[].requires`. Vision §9's tree lists both, which reads as an overlap; this is the division that makes SB-2's prefix check possible.
- **SB-12** A `ResourceReq` states `kind` and `requires` and nothing about how the requirement is met. Per Vision §8 and re-review R1, direction-asymmetric requests are expressed as distinct keys under the Vocabulary's prefix — `radio.rx.channels`, `radio.tx.channels`, `radio.rx.coherent` — not as a single count. The Kernel does not know that `rx` means anything; the flattening is what keeps it generic while still expressing the asymmetry R1 required.
- **SB-13** A Spec that contains a `placements`, `placement`, `environment`, `executor`, `memory_domain` or `island` field at any depth outside `extensions` is refused with `PlacementInSpec` or `EnvironmentInSpec`, naming the path. Vision invariants 6, 38 and 41 make these BindingProfile-only; a Spec naming an Executor cannot be promoted to a host that lacks one, which is the regression re-review R21 caught.
- **SB-14** A Spec has no variables, constants, templates or expressions. There is no evaluation pass. v3's `CONSTANTS` and `!COMPUTE(...)` are refused by SB-9 as unknown fields and by SB-4 as values that are not scalars; parametrisation is the client-side builder of Vision §9, whose source hash the Manifest records beside the Spec hash (spec 04).
- **SB-15** `graph.components` maps an `Ident` to a `ComponentDescriptor` (spec 05). `graph.links` connects two `PortRef`s and states a back-pressure policy and a capacity, both mandatory because SC-19 forbids a default and SC-21's Sink rule is a statement about the policy. A `PortRef.component` names a component of `graph.components` **or a Spec resource**, and a resource port is one the bound node declares (MA-10). `validate()` refuses an endpoint that names no declared port, and checks contract compatibility by SC-3 against whichever of the two declared it. A resource endpoint is what makes Vision §7's own `PHY Processor -> SampleStream -> Radio Port` expressible: without it a Spec cannot connect a Provider's stream to its graph at all, and Phase 2's MockRadio would have nowhere to send a block on a Spec Run. A resource endpoint is a node outside every Island, so MA-39's placement check does not ask for one. The `Block` refusal is SB-17's, because a Sink is bound rather than placed and so is never a `graph.links` consumer.
- **SB-16** A `schedule` entry places an `ActionTemplate` (spec 04, RS-49a) at a `SpecTime`: a resource name and an offset in that resource's stream clock from the Run's start. A Spec cannot name a `ClockDomainId`, because domains are allocated at `prepare` (TM-13a) and fixed at `arm` for a transmit stream (TM-13e), so a Spec cannot hold a finished timed Action at all — it holds the Action without its time field, and `arm` substitutes the resolved `AbsoluteDeadline`.
- **SB-17** `outputs` declares the artifacts the Run must produce. Each names a source port, an artifact kind, the Sink parameters, and the drop-class policy and capacity of the link that feeds it, because the output **is** the declaration of that link (SC-19, SC-21). `validate()` refuses an output whose source port does not exist, whose id has no binding (`UnboundOutput`), whose binding names a Module that does not hold the Sink role, whose bound `SinkDescriptor` does not list the artifact kind or does not accept the source port's contract (SC-3), or whose policy is not of the drop class. **A Sink is bound, not placed:** it is a Module role (MA-2, MA-25, MA-30) and not a component an Executor loads, so nothing here consults `graph.components` or an Island, and the whole rule is therefore enforceable on a Spec Run — the path a publication Run uses. The earlier reading, which made a Sink a `ComponentDescriptor` placed in an Island, gave a Sink an `impl`, an `executor_kind` and a `timing` budget that mean nothing for it, and left SC-21, this rule and SB-15 with no way to identify a Sink on the Spec path at all.
- **SB-18** `policies.failure` maps registered event kinds to reactions (spec 04). An unregistered kind is refused at `validate()`, so a misspelling is an error rather than a silently ineffective entry.
- **SB-19** `policies.coercion` sets a policy per key. SB-45 gives the resolution order.
- **SB-20** `extensions` is a map from a `Namespace` to opaque content. The Kernel copies it into the Manifest and never interprets it.

### BindingProfile

- **SB-21** The top level of a BindingProfile is the closed set `version, bindings, authority, placements, environment`, with the same unknown-field rule and the same mandatory integer `version` as a Spec.
- **SB-22** `bindings` maps a name to exactly one `Binding`: a Module id, a namespaced `selector` the Module interprets, and an optional profile name and version. Three kinds of name are bound, and the bound Module must hold the matching role: each of the Spec's **resource** names to a Provider, each **output** id to a Sink (SB-17), and each `placements.islands[].executor` name to an Executor (MA-38). A name with no binding is `UnboundResource`, `UnboundOutput` or `UnboundExecutor`. The three sets form **one** namespace, so a Spec whose resource names, output ids and Island executor names collide is refused with `DuplicateBindingName`; one map is cheaper than three and keeps SB-21's top-level set closed. A Sink binding on a **Session** profile also carries `feed`: the port it records and the drop-class policy and capacity of the link that feeds it, which RS-12 turns into the implicit Spec's output. A Spec Run's `outputs[]` already declare their feeds, so a binding that carries `feed` there is refused. A bound Sink has no id of its own — `SinkDescriptor` carries none — so it is addressed as `ResourceId { node: LOCAL, path: <output id> }`, which is what an `UpdateParameter`, a `Stop` or an `Event.source` names it by (RS-14, RS-49).
- **SB-23** A `selector` is Provider content. Vision §8's simulation example writes `instances: 2` under a Mock binding; that is selector content meaning one Provider instance emulating two motherboards, exactly as the laboratory example's `addrs: [a, b]` is one instance spanning two. It is not two Provider instances, which SB-35 would then have to reconcile with the single-instance rule.
- **SB-24** `authority` names the binding whose Provider plays the Authority role (spec 05, MA-29) and may be omitted when exactly one candidate exists. A Run has exactly one Authority (TM-16a).
- **SB-25** `placements` holds the Island declarations, the component-to-island and memory-domain assignment, and the Link Module for each graph link. Every component of the Spec's graph appears in exactly one Island. A **Sink** is bound, not placed, and so never appears in an Island's component list (SB-17, SB-22).
- **SB-25a** *Withdrawn.* The number is kept (OV-1). It let a placement entry name the `module` supplying a component's `ComponentDescriptor`, so that RS-12 could find a Session's recorder among the placed components. That reading made a Sink an Executor-loaded component, which it is not, and left a Spec Run with no way to identify a Sink at all. A Sink is now bound rather than placed (SB-17, SB-22), `ComponentPlacement` carries no `module`, and RS-12 reads the recorder from its binding.
- **SB-26** `environment` is a map from a `Namespace` to opaque content, and it is the only place a channel model, a fault schedule, a virtual-time setting, a clock distribution or a site limit may appear. The Kernel reads four sections by name: `ezsdr.time` for the time class, `ezsdr.rf_path`, `ezsdr.capture` for the optional environment capture, and `ezsdr.arm_order` for explicit ordering edges. Every other section is opaque to it. Vision §8's examples write bare `time:`, `channel:` and `rf_envelope:` keys; those are illustrative shapes, and the normative form is namespaced so that two Vocabularies cannot collide.
- **SB-27** The Manifest records `environment` verbatim (Vision §8, spec 04).
- **SB-28** *Withdrawn.* It said that performance profiles are Provider and Executor hints the Kernel does not read, which is a statement that the Kernel does nothing and therefore binds nobody. The fact remains true and is now prose: Vision §33's profiles travel in the BindingProfile as opaque content under SB-26, and `kernel_surface` is what would catch a Kernel item that read them.

### The admission-check hook

- **SB-29** A Vocabulary registers an `AdmissionCheck` against an `environment` section: the section's namespace, the stages at which it runs, and a function from the section's content and the effective configuration to a list of violations. This is how Vision §52's demand that the Kernel enforce the RF safety envelope is reconciled with audit Finding 17's ruling that the envelope's schema belongs to the Radio Model. The Kernel guarantees that the check runs; the Vocabulary owns what it means. No radio word enters the Kernel, and no Vocabulary can be bypassed.
- **SB-30** The Kernel runs every registered check whose section is present, at three points: `validate()` against the requested configuration, `prepare()` against the applied configuration returned by each Provider, and the admission of every Session Action before dispatch (spec 04). The second point is not redundant: a coercion can move an applied value outside a limit that the requested value respected, which is Vision §52's "the Provider re-checks the applied values". At the third point the check receives the Action's key and value as `proposed` and evaluates the effective configuration overlaid with it, because "before dispatch" means the value has not been applied and a check given only the current effective configuration could not see what it is being asked to admit. A non-empty violation list fails the stage, and nothing transmits until every check at every applicable stage has passed (Vision invariant 42).
- **SB-31** A registered check must be pure and must not require hardware, so that `validate()` is a true dry run (Vision §52). A section with no registered check is informational and is still recorded verbatim.
- **SB-32** *Withdrawn.* It restated Vision invariant 42 without adding a checker; SB-30 already fails the stage and now carries the sentence.

### Resources and binding

- **SB-33** A Provider instance exposes a composite resource tree (Vision §8, spec 05 MA-10): a device with channels, streams, a timekeeper, GPIO banks, replay memory and sensors beneath it, each a node with its own `ResourceId` and its own declared capabilities. *The tree's shape is spec 05's `ProviderInstance`; this rule binds the matcher, and is checked by `sb_34_*`.*
- **SB-34** A Spec resource binds to exactly one Provider instance. Its `kind` must match the kind of the instance's root or of one sub-resource, and its constraints are matched against that node's capabilities. Two Spec resources may bind to different sub-resources of one instance. A node is bound by at most **one** Spec resource or `needs` entry (SB-36) unless the bound instance declares it `shareable` (MA-10); a second binding of an exclusive node is refused with `NodeAlreadyBound { node, first, second }`, naming both resources, and is never resolved by sharing. Whether a channel, a timekeeper or a sensor is shareable is the Provider's declaration and not the Kernel's knowledge (OV-21): Vision §8's GPIO banks "may share the timekeeper" of §39, while a channel or a stream cannot be shared, and a Kernel that fixed either answer would meet the other in Phase 2. A free node is preferred over a shareable one that is already bound, so sharing is what the flag permits and not what the matcher reaches for first. *The matcher is greedy and does not backtrack; a bipartite matching is the upgrade if a profile ever needs one.*
- **SB-35** A requirement that one instance cannot satisfy fails with `NoSingleInstance`, naming the constraint. The Core never assembles a capability across instances: coherence in particular is declared by the Provider that owns the channels and is never inferred (Vision §8, §25; audit Finding 6). The Kernel holds only the single-instance rule; that a given key means coherence is the Radio Model's business.
- **SB-36** A resource may declare `needs`: named capability requirements it does not own. The matcher resolves each to a sub-resource of some bound instance, which may be a different instance from the one the resource itself is bound to, and records the resolved `ResourceId` in `AdmissionResult.matched`. A smart antenna needing a timed GPIO line declares `needs: { gpio: { kind: gpio.bank, requires: { gpio.timing_class: Eq("hardware_timed") } } }` and the Kernel resolves it to a bank of the radio's device. This is not a cross-module dependency: the peripheral knows only the capability, and the Kernel does the resolution (Vision §39). Without `needs` the rule would assert a resolution no type could express, because a `Binding` names one Provider and a `ResourceReq` had nowhere to say what it wanted from elsewhere.

### The pipeline

- **SB-37** The stages run in this order, which is re-review R4's correction: schema validation, semantic validation, resource resolution, binding resolution, capability matching against the **bound** instances, plan construction, admission checks, `prepare`, `arm`. Matching cannot precede binding, because the capabilities being matched are those of the instance that binding chose.
- **SB-38** `validate(spec, binding, registry) -> AdmissionResult` returns the matched resources, the rejected constraints, the envelope violations, a preview of the coercions and the warnings. It touches no hardware.
- **SB-39** `plan(spec, binding, admission) -> ExecutionPlan` adds the fragments, the DataLink declarations, the dependency edges, the Authority, the derived ExecutionClass and the declared transfer costs. It takes `validate()`'s `AdmissionResult`, and a Provider fragment's `content` is `{ selector, requested }`, where `requested.resource` is the node the matcher bound (SB-34) and `requested.constraints` is what the Spec asked of that resource. Without it the fragment carries the binding's selector alone, so `prepare` never sees the request and SB-44's and MA-12's rule — the report's coercions equal what `coerce` returned for the same request — has no request to be about, and a Provider would have no rate or frequency to configure. Dependency edges come from the explicit `ezsdr.arm_order` section and from each Provider instance's declared `arm_after`, which is how the device that sources PPS is armed before the devices that consume it. A cycle is `ArmCycle`. Fragments with no edge between them are ordered by their `Ident`, so a plan is deterministic.
- **SB-40** A transfer cost is a number the Link Module declares, not a measurement. The Core reports it and never uses it to choose a placement (Vision §20, §31, §63). Phase 1 neither measures nor optimises. *Checked: `sb_40_transfer_cost_is_declared`.*
- **SB-41** `prepare(plan, ctx)` calls each fragment in dependency order and collects one `PrepareReport` per fragment plus a merged effective configuration. Vision §11 says a report per fragment and §52 says "the PrepareReport"; both are produced, and the merged one is what `run.effective()` returns. `prepare` also runs SB-30's second point — every registered check, over that merged effective configuration — before it returns; a violation fails `prepare` (SB-42). Naming the call site here is deliberate: SB-30 counts three points and only this one has a function to name.
- **SB-42** Any fragment's failure fails the whole transaction, and the Run moves to cleanup (spec 04, RS-3), which releases in reverse dependency order (RS-8). *Checked jointly with spec 04.*
- **SB-43** `arm` fixes what could not be fixed earlier: each transmit stream's SampleClock origin (TM-13e) and each `SpecTime`'s resolution to an `AbsoluteDeadline` (SB-16). A burst target's conversion onto the transmit sample grid (SC-23a) therefore happens at the first admission point after `arm`; the plan-time check of a statically known lead compares durations only, which TM-21 does from the nominal rates fixed at `prepare` and which needs no origin.

### Coercion and versioning

- **SB-44** A `Coercion` records the key, the requested value, the applied value and the Provider's reason. A Provider's `prepare` must report the same coercions its `coerce` reported for the same request (spec 05, MA-12), so that a dry run and a real run agree. *Producer obligation on the Provider; checked from the Kernel side by `sb_07_coercible_key_consults_provider` and from the Module side in spec 05.*
- **SB-45** The coercion policy for a key resolves in this order: the Spec's `policies.coercion` entry; then, for a Session, `warn`; then the key's `coercion_default` from its `KeyDecl`; then `warn`. Re-review R16 asks for the split by Run kind, and this is where it lands: an interactive Session warns, while a Spec Run takes the Vocabulary's default, which the Radio Model sets to `reject` for sample rate and frequency and `warn` for gain, because a publication Run must not silently change its waveform timing.
- **SB-46** Under `reject` a coercion fails the stage with `CoercionRejected`. Under `warn` it is applied, recorded and warned. Under `accept` it is applied and recorded. In all three cases it appears in the PrepareReport and in the Manifest.
- **SB-47** A document whose `version` is not supported is refused with `UnsupportedVersion`, naming the version found and the versions supported. It is never interpreted under a newer version's defaults.
- **SB-48** A migration is a function from one major version to the next, registered per document type. Phase 1 registers none, because only version 1 exists; the refusal path and the registration point both exist and are tested, so that adding version 2 is a migration rather than a redesign. *The migration path itself is a forward obligation, tested when a version 2 exists.*
- **SB-49** When a document was migrated, the Manifest records the original version and hash beside the compiled one (spec 04).

## 6. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| B1 | Constraint vocabulary | The six of audit Finding 7, over scalars only | A general predicate language (Vision §9's ban on expressions applies to the BindingProfile too); constraints over lists (the matcher would need Vocabulary semantics to compare them) | A Vocabulary needing set-of-sets logic registers an `AdmissionCheck` instead |
| B2 | Direction asymmetry | Distinct flattened keys under the Vocabulary prefix (SB-12) | A Kernel-level `rx`/`tx` split (a radio word in the Kernel, Finding 7); a single `channels` count (re-review R1's regression) | The Kernel cannot check that `rx` and `tx` are consistent; that is the Radio Model's `AdmissionCheck` |
| B3 | Forbidding placement in a Spec | A field-name scan at any depth outside `extensions` (SB-13) | Trusting the schema (a Spec could carry placement inside a Vocabulary section and still validate); allowing it with a warning | A Vocabulary could still smuggle placement into `extensions`; that is visible in the Manifest and is the Vocabulary's defect |
| B4 | Schedule times | A resource-relative offset resolved at `arm` (SB-16, SB-43) | A `ClockDomainId` in the Spec (domains are allocated at `prepare`, so the Spec would not be portable); wall-clock times (TM-1) | An offset is from the Run's start only. An absolute device time in a Spec is deliberately not expressible, which means v3's `onTime` — one of Vision §61's four behaviours worth a regression test — is testable on the Session path, where a `Vocabulary` action carries an `at` beside its params (spec 04, RS-19), and not on the Spec path. This holds only once RS-49's `UpdateParameter` carries that `at` through to the Action; until then the admitted instant is computed and dropped (amended 2026-09-22, 00 §11, open question 2) |
| B5 | RF envelope enforcement | A Vocabulary-registered `AdmissionCheck` the Kernel is obliged to run at three points (SB-29, SB-30) | The envelope's fields in the Kernel (Vision §52 read literally, but Finding 17 puts the schema in the Radio Model); enforcement left to the Provider (Vision §52 makes it a Kernel policy, and a Provider could skip it) | A check is pure and cannot consult hardware; a site limit that needs a measurement is a Provider constraint instead |
| B6 | Arm ordering | Explicit edges plus Provider-declared `arm_after`, cycle refused, ties by name (SB-39) | Inferring order from the resource tree (the PPS relationship is not in the tree); operator documentation, which is what v3 had | No automatic ordering; a missing edge is a start-up failure the plan cannot predict |
| B7 | Versioning | A mandatory integer major, supported set `{1}`, refusal implemented and migration registered but empty (SB-47, SB-48) | A semver string for *documents* (a document has one dimension of compatibility, so a minor and a patch would be decoration). Module and Vocabulary versions are a different matter and do carry minor and patch, as `Binding.profile.version` and the Manifest's `modules[].version` show, and Vision §59 bumps a Mock profile's version on measurement; spec 05 owns that type; v3's exact-match (correct but leaves no upgrade path); silent reinterpretation | One major at a time; a v1-to-v2 migration is a function, not a converter chain |
| B8 | One instance per resource | The Kernel holds only the rule; which key means coherence is Vocabulary (SB-34, SB-35) | The Kernel understanding coherence (Finding 6 forbids it); allowing a resource to span instances (Vision §25) | A genuinely multi-instance requirement must become one Provider instance, which is what `addrs: [a, b]` is |
| B9 | Transfer cost | Declared by the Link Module, reported, never used to choose (SB-40) | Measuring it in Phase 1 (nothing to measure yet); using it to rank placements (Vision §63 rejects an optimiser) | A declared number can be wrong; the parity work of Phase 8 is where it is checked |

## 7. Phase 1 tests

| test | input | expected | rules |
|---|---|---|---|
| `sb_10_version_one_validates` | a minimal v1 Spec and BindingProfile | accepted | SB-10, SB-21 |
| `sb_47_version_two_refused_by_name` | `version: 2` | `UnsupportedVersion { found: 2, supported: [1] }` | SB-47 |
| `sb_48_migration_point_exists` | a registered no-op migration from 0 to 1 | it runs and the Manifest records the original version | SB-48, SB-49 |
| `sb_09_unknown_top_level_field_refused` | a Spec with `constants` | `UnknownField { path: "constants" }` | SB-9 |
| `sb_14_compute_expression_refused` | a value `"!COMPUTE(A - B)"` and a `CONSTANTS` map | refused as an unknown field and as a non-scalar value | SB-14, SB-4, SB-9 |
| `sb_13_placement_in_spec_refused` | `graph.placement`, then `resources.radio.memory_domain` | `PlacementInSpec` naming each path | SB-13 |
| `sb_13_environment_in_spec_refused` | an `environment` key inside a Spec resource | `EnvironmentInSpec` | SB-13 |
| `sb_02_unknown_key_prefix_refused` | `requires: { radio.rx.channels: … }` with no radio Vocabulary declared | `UnknownKeyPrefix` | SB-2 |
| `sb_06_constraint_match_table` | each of the six constraints against `One`, `Range` and `AnyOf` | the table of SB-6 exactly | SB-6 |
| `sb_06_key_shape_mismatch` | an integer constraint on a boolean key | `KeyShape` | SB-6 |
| `sb_07_coercible_key_consults_provider` | `test.grid` requested at 19.5, the double snapping to 20 | `coerce` is called once; the preview shows the coercion | SB-7, SB-44 |
| `sb_07_non_coercible_key_fails_directly` | `test.count` outside the declared range | rejected without calling `coerce` | SB-7 |
| `sb_45_coercion_policy_chain` | the same coercion as a Session, as a Spec Run with the key defaulting to `reject`, and with a Spec override of `accept` | warn, reject, accept | SB-45, SB-46 |
| `sb_22_unbound_resource_fails` | a Spec resource with no binding | `UnboundResource` | SB-22 |
| `sb_35_no_single_instance` | two instances each declaring `test.count: 2`, a requirement of `Min(4)` | `NoSingleInstance` naming the constraint | SB-35 |
| `sb_34_sub_resource_binding` | a resource whose `kind` matches a `test.line` sub-resource | bound to that node's `ResourceId` path | SB-34, SB-3 |
| `sb_34_two_resources_one_instance` | two Spec resources on two sub-resources of one instance | both bound, one instance | SB-34 |
| `sb_34_exclusive_node_bound_twice_is_refused` | three `test.line` resources on a two-line device | `NodeAlreadyBound` naming both colliding resources, never a silent share | SB-34 |
| `sb_34_shareable_node_may_be_bound_twice` | the same three, with the lines declared shareable | all three bind, and the two free lines are taken before one is shared | SB-34, MA-10 |
| `sb_30_admission_check_runs_at_three_points` | a `test.limits` section, a request inside the limit whose coercion lands outside it, then a Session Action outside it | passes validate, fails prepare, and the Action is rejected with the proposed value in the violation | SB-29, SB-30 |
| `sb_36_needs_resolves_across_instances` | a peripheral bound to one instance declaring `needs` for a `test.line` capability another instance owns | resolved to that sub-resource's `ResourceId`; unresolvable `needs` fails validate | SB-36 |
| `sb_15_link_policy_is_mandatory` | a link with no policy, then with no capacity | refused in both cases | SB-15, SC-19 |
| `sb_15_a_bound_resource_port_is_a_link_endpoint` | a link from a bound resource's declared `rx` port into a placed component, then the same naming a port the node does not declare | the first validates and plans; the second is refused | SB-15, MA-10, SC-3 |
| `sb_31_unregistered_section_is_informational` | an arbitrary environment section | recorded verbatim, no check run | SB-31, SB-27 |
| `sb_39_arm_order_follows_edges` | three fragments with `b after a`, `c after a` | `a` first; `b` and `c` in name order | SB-39 |
| `sb_39_arm_cycle_refused` | `a after b`, `b after a` | `ArmCycle` | SB-39 |
| `sb_41_prepare_report_per_fragment_and_merged` | three fragments, one coercing | three reports plus a merged effective map | SB-41 |
| `sb_42_fragment_failure_fails_the_transaction` | the third fragment failing at prepare | the whole `prepare` fails and cleanup releases in reverse order | SB-42, RS-3, RS-8 |
| `sb_16_spec_time_resolves_at_arm` | a schedule entry holding a `TxBurst` template at offset 1000 in a 20 Msps stream | the Spec validates with no time field; after `arm` the Action carries an `AbsoluteDeadline` in that stream's SampleClock | SB-16, SB-43, RS-49a, TM-13e |
| `sb_22_a_session_profile_binds_its_recorder` | a Session profile binding a recorder with a `feed`, then the same with `feed` removed | the implicit Spec carries one output and no component; the feedless binding is refused | SB-22, SB-17, RS-12 |
| `sb_15_link_contract_and_sink_policy` | a link between mismatched contracts, then a `Block` link into a Sink | `Incompatible`, then refused | SB-15, SC-3, SC-21 |
| `sb_17_capture_without_a_sink_refused` | an output id with no binding, on a **Spec** Run | refused at validate, which the placed-Sink reading could not do on that path | SB-17, SB-22 |
| `sb_18_unregistered_event_kind_refused` | `policies.failure: { RX_OVERFLOWS: stop }` | refused naming the kind | SB-18 |
| `sb_37_matching_follows_binding` | two bound instances with different capabilities, one Spec | the matcher uses the bound instance's capabilities, not the union | SB-37 |
| `sb_27_environment_portability` | one Spec, two BindingProfiles differing only in `environment` | both compile; the Spec hash is equal and the binding hashes differ | SB-27, RS-45 |
| `sb_39_a_provider_fragment_carries_the_matched_request` | a resource with a constraint, validated then planned | the fragment's `content.requested` names the bound node and carries the Spec's constraints, beside the selector | SB-39, SB-44, MA-12 |
| `sb_40_transfer_cost_is_declared` | a Link Module declaring a cost | it appears in the plan; no placement changes | SB-40 |
| `sb_24_authority_inferred_when_unique` | one Authority-capable binding, `authority` omitted | it is chosen; with two candidates and no field, refused | SB-24 |

## 8. Vision coverage

| Vision | This spec |
|---|---|
| §8 intent versus binding, per-direction requests | SB-12, SB-22, decision B2 |
| §8 composite resources, one instance, sub-resource binding | SB-33…SB-36 |
| §8 BindingProfile = bindings + placements + environment | SB-21, SB-25, SB-26 |
| §9 the Spec tree, no expressions, the builder | SB-9…SB-20, SB-14 |
| §10 the compile pipeline in order | SB-37 |
| §10 validator not optimiser | SB-40, decision B9 |
| §10 schema-first, migrate or refuse | SB-47…SB-49 |
| §11 PrepareReport, coercion policy, per-fragment reports | SB-41, SB-44…SB-46 |
| §20 placement explicit and validated | SB-13, SB-25 |
| §31 admission validates compute, memory and transfer reachability | SB-39, SB-40 (with spec 05's island checks) |
| §33 profiles are hints, not Kernel semantics | SB-28 |
| §52 validate, plan and prepare; the RF safety envelope | SB-38, SB-39, SB-29…SB-32 |

## 9. Vision issues found

1. **§9's tree lists both `requirements` and per-resource `requires`** without saying how they differ. SB-11 divides them: `requirements` declares the Vocabulary majors, which is what SB-2's prefix check needs, and `requires` holds the constraints.
2. **§8's simulation example writes `instances: 2` under one binding**, which contradicts SB-34's one-instance rule if read as two Provider instances. SB-23 reads it as selector content, mirroring the laboratory example's comment that `addrs: [a, b]` is one instance.
3. **§8's environment examples use bare keys** (`time:`, `channel:`, `rf_envelope:`) where two Vocabularies would collide. SB-26 makes the normative form namespaced; the examples are illustrative shapes, which the Vision's index already says of every `{ ... }` block.
4. **§52 says the Kernel enforces the RF envelope as a Kernel policy while audit Finding 17 puts its schema in the Radio Model.** Reconciled by SB-29 and SB-30: the obligation to run the check is the Kernel's, the content is the Vocabulary's.
5. **§11 says a PrepareReport per fragment and §52 says "the PrepareReport".** SB-41 produces both.
6. **§52's list of what validation "may determine" includes "performance risks" and "selected fallback strategies"**, which are not testable as written. Phase 1 implements exactly the `AdmissionResult` fields of SB-38.
7. **§8's binding example writes `provider:` as the Binding's field name** (`bindings: radio: provider: ezsdr.radio.uhd`). SB-22 binds Providers, Sinks and Executors through that one map, so the field is `module`. The example is an illustrative shape, which the Vision's index already says of every `{ ... }` block, but the word is a visible departure and is recorded here for §12.
8. **Audit Finding 6 cites `v3/changelog/v3.0.17.md` for the PPS start-order failure.** The note is in `v3/changelog/v3.0.20.md:56-59`, whose own first line nevertheless reads `# EzSDR v3.0.17`, which is presumably how the misattribution arose. The finding's substance is unaffected.

## 10. Deferred

Radio Model keys and their `KeyDecl`s, including `coercion_default` for rate, frequency and gain (Phase 2). The `rf_envelope` section's fields and its `AdmissionCheck` (Phase 2). The simulation environment sections `ezsdr.time` beyond the class name, `sim.channel` and `sim.faults` (Phases 2 and 3). The recorder Sink's descriptor and the capture data path (Phase 2 and later). Measured transfer costs and any placement search, which are never Kernel work (Vision §63). The Python Spec builder and its source hash (Phase 6). Migrations, until a version 2 exists.
