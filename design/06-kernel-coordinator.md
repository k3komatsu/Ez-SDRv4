# Phase 2 spec 06 — Kernel amendments, the Run coordinator, and update classes

| Field | Value |
|---|---|
| Status | Accepted at Gate P (owner, 2026-09-24) and ratified at Gate X (owner, 2026-09-25; [`plan/phase2/00-overview.md`](../plan/phase2/00-overview.md) §11). Normative for `ezsdr-kernel`. Amended in Phase 3 by KB-1 (KC-9, KC-11). Amended in Phase 5 by KE-1 (KC-9), KE-2 (§3, KC-24), KE-4 (K11, §13a) and KE-5 (UC-1, UC-2; [`plan/phase5/15-amendments.md`](../plan/phase5/15-amendments.md)). Amended in Phase 7 by KG-1 (KC-2, KC-2a), KG-2 (KC-31, KC-36, KC-46, KC-46a), KG-3 (KA-12's table, KC-29, KC-46b, KC-46c), KG-4 (KC-21a, KC-24a), KG-5 (KC-20, KC-29, KC-33), KG-6 (KC-12a), KG-7 (KC-15), KG-8 (UC-6), KG-10 (KC-30), KG-11 (KC-45), KG-12 (KC-37a) and KG-13 (UC-3; [`plan/phase7/19-amendments.md`](../plan/phase7/19-amendments.md)). Amended by maintenance spec 20: KH-1 (KC-45, and a note on KA-4's evidence; issue #43; [`plan/maintenance/20-amendments.md`](../plan/maintenance/20-amendments.md)). |
| Scope | (a) The amendments KA-1…KA-22 that Phase 1's accepted specs need before a real Module can run; (b) the coordinator, `ezsdr-kernel::coordinator`, which drives one Run end to end in the Simulation class, for a Spec Run and for a Session; (c) what each update class means (UC-n). |
| Not in scope | RealtimeEmulation, HardwareInLoop, Hardware (refused, KC-2); child Runs (refused until Phase 6, KC-37; created by `run_child` since, KC-37a); session replay; the Radio Model's content (spec 07). |
| Depends on | Specs 01–05 as amended here; [`plan/phase2/00-overview.md`](../plan/phase2/00-overview.md) §4 Y1–Y14. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

---

## 1. Purpose

Phase 1 fixed the vocabulary every Module signature uses and implemented each rule as a function with a test. What it did not have was a caller: seventeen Phase 1 rules name "the Phase 2 coordinator" as their call site. This document writes that caller down, and, before it, the twenty-two places where writing it showed that a Phase 1 rule or type cannot carry the first real Module.

## 2. Kernel amendments

Each amendment names the evidence (a file and line of the Phase 1 crate, or a rule), the new normative text, the code change, the tests, and the schema impact. The implementer applies the new rule text to `design/0N-*.md` verbatim in the same step as the code (PO-9).

### KA-1 — `PrepareContext` owns its handles, and carries the clock registry and the environment

**Evidence.** `src/module_api.rs:949-969` declares `PrepareContext<'a>` with `time: &'a dyn TimeAuthority`, `events: &'a dyn EventSink`, `actions: &'a dyn ActionReceiver`, `actions_out: &'a dyn ActionSubmitter`. Role traits take `ctx: PrepareContext<'_>` by value, and a `Box<dyn Provider>` is `'static`, so no Module can keep a handle after `prepare` returns: it cannot read the time, emit an event, receive an Action or submit one in `arm`, `start`, `step` or `stop`. The doubles never keep one (`tests/support/doubles.rs:470`, `:588`). A Provider also has no route to the `ClockRegistry`, which TM-13a's SampleClock allocation needs (KA-2), and none to the `environment`, where Vision §8 and §17 put what a Mock must read (`sim.faults`, `sim.seed`).

**New shape** (spec 05 §4, replacing the `PrepareContext` row):

```text
PrepareContext    { run: RunId, class: ExecutionClass,
                    time: shared TimeAuthority handle, clocks: shared ClockRegistry handle,
                    events: shared EventSink, actions: shared ActionReceiver,
                    actions_out: shared ActionSubmitter,
                    environment: shared read-only Map<Namespace, Value>,
                    links: [{ component: Ident, port: Ident, endpoint: Endpoint }],
                    components: Map<Ident, ComponentDescriptor>, host_budget: RelativeBudget }
```

```rust
pub struct PrepareContext {
    pub run: RunId,
    pub class: ExecutionClass,
    pub time: Arc<dyn TimeAuthority>,
    pub clocks: Arc<ClockRegistry>,
    pub events: Arc<dyn EventSink>,
    pub actions: Arc<dyn ActionReceiver>,
    pub actions_out: Arc<dyn ActionSubmitter>,
    pub environment: Arc<BTreeMap<Namespace, serde_json::Value>>,
    pub links: Vec<AttachedPort>,
    pub components: BTreeMap<Ident, ComponentDescriptor>,
    pub host_budget: RelativeBudget,
}
```

Every role-trait method that took `ctx: PrepareContext<'_>` takes `ctx: PrepareContext`.

**New rule MA-5a.** A Module may keep every handle its `PrepareContext` carries and use it from `prepare` through `cleanup`; the coordinator keeps each one valid until the Run is `CleanedUp`. A handle is shared (`Arc`), so a Module that keeps one keeps it alive; it must drop them in `cleanup` (MA-7). The `environment` is the BindingProfile's `environment` verbatim and read-only: a Module reads the sections its own Vocabularies define and no other (OV-21's rule, applied to Modules), and the Kernel still reads only SB-26's four. *Checked by `ma_05a_a_module_keeps_its_handles_after_prepare`: a stepped test Provider stores `events` and `time` at `prepare` and emits an event and reads `now` in `step`; the event reaches the Manifest.*

**MA-46 amended** (the Plugin mapping sentence): "…an `Endpoint` a shared-memory Link, the `TimeAuthority` a request and response, the `ClockRegistry` a request and response, and the `environment` a document sent once at `prepare`."

**Code.** `src/module_api.rs`: the struct above; the four role-trait `prepare` signatures (`Provider`, `Executor`, `Sink`; `Link` and `Authority` have none). `tests/module_api.rs:271-338` and every double in `tests/support/doubles.rs` follow.

**Schema.** None: `PrepareContext` is a Kernel handle, not a document (X8).

### KA-2 — A Provider declares its SampleClocks through `PrepareContext.clocks`

**Evidence.** TM-13a: "Its id and its `root_ticks_per_tick` are allocated at `prepare` from the effective, post-coercion rate in the PrepareReport … Allocating it from a PrepareReport is the Phase 2 coordinator's call site". The effective rate is a Vocabulary key (`radio.rx.sample_rate_hz`), which OV-21 forbids the Kernel to read, so the coordinator cannot allocate from the report; only the Provider knows which key is a rate and which stream it drives.

**TM-13a amended** (the forward sentence replaced): "*The Provider allocates it in its `prepare`, through `PrepareContext.clocks.declare_sample_clock`, from its own effective post-coercion rate, naming the stream by a `ResourceId` within its own tree; it registers the domain with `register_sample_clock` when TM-13b or TM-13e fixes the origin. The coordinator records every registered SampleClock in the Manifest (KC-45) (Phase 2, KA-2).*"

**TM-12 amended** (sentences added): "`ClockRegistry::domains()` returns every registered domain in id order, which is what the Manifest's `clocks.domains` records (RS-38). `declare_sample_clock` also records the handle it returns, and `ClockRegistry::declared_sample_clocks()` lists every declared handle in declaration order, registered or not: a receive clock is declared at `prepare` and registered only at its first sample (TM-13b), and KC-16 resolves a `SpecTime` against its ratio before then (Phase 2, KA-2)."

**Code.** `src/time/domain.rs`: `Inner` gains `declared: Vec<SampleClockHandle>`; `declare_sample_clock` pushes the handle before returning it; `pub fn domains(&self) -> Vec<ClockDomain>` (a clone of the map's values in key order) and `pub fn declared_sample_clocks(&self) -> Vec<SampleClockHandle>` on `ClockRegistry`. Methods on an existing type are not allow-list items (D10).

**Tests.** `tm_12_domains_lists_every_registered_domain` (a fresh registry lists `utc` and `host.monotonic`; after registering a root and a derived domain, four, in id order); `tm_13a_declared_clocks_are_listed_before_registration` (two declarations, one registered: both listed by `declared_sample_clocks`, one by `sample_clock_records`).

### KA-3 — Host bytes travel in the block

**Evidence.** SC-8 routes host bytes through `HostMemoryAccess::map_host(&self, b: &'a BlockRef) -> Option<&'a [u8]>`, "implemented by the DataLink". But `DataLink` (`src/stream/link.rs:111`) does not include it and `Endpoint::StreamIn` holds an `Arc<dyn DataLink>`, so a consumer holding its endpoint cannot call it at all. And the signature cannot be implemented in safe Rust by a pool that owns and recycles its buffers: the returned slice borrows the block, and the block holds only a `u64` handle. Phase 1's only implementation leaks every buffer and uses `unsafe` (`tests/support/mod.rs:146-155`).

**SC-8 replaced.** "Host bytes are reached only through `SampleBlock::host_bytes()`, which returns the bytes a producer attached with `SampleBlock::new_host` and `None` for a block built with `SampleBlock::new`. The bytes are shared with the block and live while any reference to the block does, which is SC-9's validity rule by construction; a pool recycles a buffer only when no block references it (HD-2). Which memory domains are host-reachable is Vocabulary content (SC-6), so the Kernel cannot refuse `new_host` for a domain that is not; attaching host bytes to such a block is a producer defect. No role-trait method takes or returns a byte slice (MA-6): `host_bytes` is a method of a block, reached through a `BlockRef` a link delivered. *Checked by `sc_08_host_bytes_only_for_a_block_that_carries_them` (Phase 2, KA-3).*"

**Decision S5 amended** (spec 02 §7): the chosen option becomes "an opaque handle, plus optional shared host bytes owned by the block"; the rejected list gains "a host-mapping trait on the link (its slice cannot be produced in safe code from a recycling pool, and the consumer's endpoint type does not expose it)".

**Code.** `src/stream/block.rs`:

```rust
#[derive(Clone)]
struct HostBytes(std::sync::Arc<[u8]>);
impl PartialEq for HostBytes { fn eq(&self, o: &Self) -> bool { self.0[..] == o.0[..] } }
impl Eq for HostBytes {}
impl std::fmt::Debug for HostBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HostBytes({} bytes)", self.0.len())
    }
}
// SampleBlock gains one private field:  host: Option<HostBytes>
impl SampleBlock {
    /// SC-8, SC-10a: a block whose bytes are host memory the producer owns.
    pub fn new_host(header: BlockHeader, memory_domain: MemoryDomainId,
                    bytes: std::sync::Arc<[u8]>, bytes_per_sample: u32)
        -> Result<SampleBlock, StreamError>;   // buffer = { memory_domain, handle: 0, len_bytes: bytes.len() };
                                               // every SC-10 / SC-10a check of `new`, then host = Some(bytes)
    /// SC-8: the attached host bytes, or None.
    pub fn host_bytes(&self) -> Option<&[u8]>;
}
```

`new` sets `host: None`. `HostMemoryAccess` and its `pub use` are **deleted**, with its line in `tests/kernel_surface_allow.txt` (the `NEW:` count falls by one). `MemLink`'s `impl HostMemoryAccess` is deleted. In `tests/support/mod.rs`, `host_buffer(channels, len)` returns `BufferRef { memory_domain: HOST_MEM, handle: 0, len_bytes: channels · len · 8 }` and leaks nothing, and `block(h)` builds its block with `SampleBlock::new_host(h, HOST_MEM, vec![0u8; n].into(), 8)`; together they remove the file's only `unsafe`.

**Tests.** `sc_08_buffer_map_host_none_for_gpu_domain` is replaced by `sc_08_host_bytes_only_for_a_block_that_carries_them`: a block from `new` returns `None`; one from `new_host` with 32 000 bytes returns 32 000 bytes; a `new_host` block with too few bytes is refused by SC-10a exactly as `new` refuses it.

### KA-4 — Admission checks see one configuration per fragment

**Evidence.** `AdmissionCheck::check` receives `effective: &BTreeMap<Key, Value>` (`src/binding.rs:357-363`). `validate` builds it by flattening every resource's `Eq` constraints into one map (`src/plan/validation.rs:20-33`), `collect_prepare` from `MergedPrepare::effective` (removed by spec 20, KH-1; issue #43), which SB-41 itself calls lossy, and `Admitter::admit` from whatever its caller passes. Two radios both carry `radio.tx.gain_db`; one value survives, and the RF envelope check never sees the other device.

**SB-29 amended** (the `AdmissionCheck` shape in spec 03 §4):

```text
AdmissionCheck   { section: Namespace, stages: [validate | prepare | runtime],
                   check(section: Value,
                         effective: Map<Ident, Map<Key, Value>>,
                         proposed:  Map<Ident, Map<Key, Value>>, stage) -> [Violation] }
                 keyed by fragment id; a check judges each entry of `effective` overlaid with
                 the same entry of `proposed`, and a Violation's reason names the fragment
```

**SB-30 amended** (sentence added after "at three points"): "At each point the configuration is given **per fragment**, keyed by fragment id (SB-T1), never merged: at `validate`, one entry per Spec resource holding the values of its `Eq` constraints and of its `needs`' `Eq` constraints (a need's value never replacing the resource's own for one key; possibly empty), and one per output holding its `params`; at `prepare`, one entry per fragment that returned a `PrepareReport`, holding that report's `effective`; at the runtime point, the coordinator's current per-fragment configuration (KC-27), with `proposed` holding only the target fragment's proposed key and value. A merged map keeps one value of a key two fragments name and so cannot be checked (Phase 2, KA-4)."

**RS-17 amended**: "`Admitter::admit` over one Action's key and value" becomes "…over one Action's key and value, with the per-fragment configuration of SB-30".

**Code.** `src/binding.rs`: the trait method and `AdmissionCheckRegistry::run` take `&BTreeMap<Ident, BTreeMap<Key, Value>>` for `effective` and `proposed`. `src/plan/validation.rs::requested_violations` builds the per-fragment map as SB-30 says. `src/plan/prepare.rs::collect_prepare` builds it from the reports (`r.fragment → r.effective`) and passes it to `checks.run`. `src/session.rs::Admitter::admit` takes the two per-fragment maps; step 3 iterates every key of every entry of `proposed`. `TestLimitsCheck` judges every entry. The four `admit` call sites in `tests/run_session.rs` and `tests/spec_binding.rs` wrap their maps as `{ radio: … }`.

**Tests.** `sb_30_the_check_sees_each_fragment_s_own_value`: two test resources on two distinct instances, `a` requesting `test.grid` 40 and `b` 20, with `test.limits` at a ceiling of 30 that only `a`'s value exceeds; exactly one violation, whose reason begins `"a: "` and whose `requested` is 40, at `validate`, at `prepare` (reports `a: 40`, `b: 20`) and at the runtime point (`proposed = { a: { test.grid: 40 } }`). A flat map built in resource order keeps `b`'s value, so the test fails against the Phase 1 code. A last case moves `a`'s 40 into a `need` of `a` (a `test.line` whose lines declare `test.grid`, `TestProvider::with_line_grid`): the violation still names `a`, because a need's values are its resource's fragment's.

### KA-5 — `coerce` is called once per bound node, and its `rejected` list is read

**Evidence.** `src/plan/matching.rs:151-153` returns before calling `coerce` when every key was satisfied directly, and `report.rejected` is read nowhere. A combination such as four channels at 200 Msps is satisfied key by key and exceeds the transport as a whole: Vision §13 rule 1 requires it "rejected at `validate()` / `prepare()`", and the only Kernel hook that sees a Provider's envelope at `validate` is `coerce` (MA-11).

**SB-7 amended** (the sentence "`coerce` is called at most once per bound node…" replaced): "`coerce` is called **exactly once** per bound node, with the resource's whole `requires` map, unless a non-coercible key has already failed — **including** when every constraint is satisfied directly, because a Provider may refuse a combination of values each of which is individually declared, which is how Vision §13 rule 1's PerformanceEnvelope refusal reaches `validate()`. A key the declared capability did not satisfy directly is judged from `applied` as before. Every entry of `CoerceReport.rejected` whose key the request names becomes a `RejectedConstraint` carrying the Provider's reason (one per key; an entry for a key already rejected replaces that reason); an entry whose key the request does not name is a malformed report and a violation, as SB-44 treats a stray coercion. A coercion the Provider reports for a key the capability satisfied directly — a `Range` capability over-approximates a grid — is previewed like any other (Phase 2, KA-5)."

**MA-11 amended** (sentence added): "`coerce` is called at every `validate()`, not only when a key needs coercion, so it must be cheap; and it is where a Provider refuses a combination its envelope does not admit (SB-7)."

**Code.** `src/plan/matching.rs::match_constraints`: remove the early return; call `coerce` when no key has been rejected by the non-coercible branch; after the `applied` loop, map `report.rejected` as the rule says.

**Tests.** `sb_07_coerce_refuses_a_combination`: the double gains `with_joint_limit(n)`, which removes `test.grid` from `applied` and adds a `rejected` entry on it when the applied `test.count · test.grid > n`; a request `{ test.count: Eq 2, test.grid: Eq 40 }` against a limit of 50, every key satisfied directly, has `coerce` called once and is not admitted, with exactly one `RejectedConstraint` on `test.grid` carrying the Provider's reason; `{ …, test.grid: Eq 20 }` is admitted. `sb_07_a_stray_rejection_is_a_violation`: the double gains `with_stray_rejection()`, rejecting `test.flag`, which no request names; the result is not admitted, has no `RejectedConstraint` and one `ezsdr.coercion` violation on `test.flag`. `sb_07_non_coercible_key_fails_directly` still sees `coerce` called zero times.

### KA-6 — A control-path parameter change is coerced by its target Provider

**Evidence.** `plan/phase1/00-overview.md` §11, the observation after D50: "on the Session path nothing calls `Provider::coerce` for a `SetParameter`, so RS-17's coercion step sees only RS-19's 'as soon as possible' record and a Session `sdr.rx.rate = 19.5e6` is dispatched uncoerced." Vision §58 #16 then has no mechanism for "a rate outside the profile's PerformanceEnvelope … is rejected and logged as rejected".

**RS-17 amended** (a step 0 added before "the registered admission checks"): "0. For an `UpdateParameter` whose target resolves to a **Provider** fragment and which arrives on the **control path** — from a Session or from the Spec's schedule — the target Provider's `coerce` is called once, with a `Requested` whose `resource` is the fragment's matched node and whose `constraints` are `Eq` of every scalar value of the fragment's current configuration (KC-27), the proposed key's `Eq(value)` replacing its own. An `Err`, or a `rejected` entry for any key, is a violation (`ezsdr.coercion`) and the Action is refused; an `applied` value for the proposed key that differs from the proposed value is a `Coercion`, which step 2 judges under SB-45 and which replaces the Action's value; a missing `applied` value for the proposed key is a refusal. An Action a **Module** submits (MA-14a) is not coerced by the Kernel, because the stepping loop holds every Module while one runs; its target enforces its own envelope and emits its Vocabulary's event (MA-14). *Forward: whether the reactive path coerces is Phase 5's to settle (Phase 2, KA-6).*"

**Code.** The coordinator (KC-26); nothing in `session.rs` changes except that `Admitter::admit` now receives the coercion.

**Tests.** `rs_17_a_session_rate_change_is_coerced_by_its_provider` (the double's grid snaps 19.5 to 20; the log entry records the coercion; under a Spec-Run policy `reject` the same change is refused), `rs_17_a_session_change_beyond_a_joint_limit_is_refused` (KA-5's double).

### KA-7 — `ProviderInstance.min_command_lead`

**Evidence.** RS-19: "An action with no `at` is admitted at the earliest time the bound Provider's envelope allows". SC-27: "`RejectAtPlan` is legal only for a statically known target", with a plan-time check D109 raised as unimplemented. Both are Kernel rules that need one number from the Provider's envelope, and the envelope is Vocabulary content the Kernel may not read by key (OV-21).

**MA-10 amended** (sentence added): "`ProviderInstance.min_command_lead` is the least lead this instance needs between receiving a timed Action and that Action's instant, as a `Duration` in `host.monotonic`; absent means zero. It is the only envelope value the Kernel reads: RS-19's earliest instant and KC-19's plan-time lead check use it. A Vocabulary that defines a timing envelope defines how its own key relates to it (RM-6). `validate()` refuses one in another domain or negative (SB-22f) (Phase 2, KA-7)."

**RS-19 amended** ("the earliest time the bound Provider's envelope allows" made exact): "…admitted at the earliest instant its target allows: for a target that resolves to a Provider fragment, the coordinator's current instant in the Authority's primary root plus that instance's `min_command_lead` rescaled to the primary root with TM-9 and rounded up; for any other target, the current instant."

**SB-22f amended** (after "…checked for X7 and for SB-1's grammar"): "…and a Provider's `min_command_lead`, when present, is in `host.monotonic` and not negative."

**Code.** `src/module_api.rs::ProviderInstance`: `#[serde(default)] pub min_command_lead: Option<Duration>`. `src/plan/validation.rs`: the SB-22f check. Every `ProviderInstance { … }` literal in the tests gains `min_command_lead: None`.

**Schema.** `provider_instance.v1.json` gains the optional property. `SCHEMA_CHANGELOG.md`: "v1 — pre-freeze revision, KA-7: `ProviderInstance.min_command_lead`".

**Tests.** `sb_22f_min_command_lead_is_in_host_monotonic` (leads of 2 000 000 and 0 in `host.monotonic` validate; −1 in `host.monotonic` and 2 000 000 in another domain are refused at `validate` with a reason naming SB-22f). The double gains `with_min_command_lead(Duration)`.

### KA-8 — The Run's start instant, and how a `SpecTime` resolves

**Evidence.** SB-16 says an offset counts "in that resource's stream clock from the Run's start", and SB-43 that `arm` resolves it. Neither says what "the Run's start" is, nor which stream clock a device with a receive and a transmit stream means; and a receive SampleClock's origin is fixed only at its first sample (TM-13b), after `arm`.

**New Kernel-read field.** The `ezsdr.time` section is the closed set `{ class, start_lead_ns }`. `start_lead_ns`, when present, is a JSON integer in `0 ..= 2^62`; absent means 0. A section with another field, or a `start_lead_ns` that is not such an integer, is refused where `plan()` reads the section (MA-41's clause: a misspelling must not become agreement).

**MA-41 amended** (sentence added): "The section's fields are `class` and `start_lead_ns` (KC-15) and no other; one it does not name is refused (Phase 2, KA-8)."

**SB-16 amended** (the sentence beginning "A `schedule` entry places…" extended): "…from the Run's start. The Run's start is the instant **T0** of KC-15. The resource's stream clocks are the SampleClocks its Provider declared at `prepare` (TM-13a, `ClockRegistry::declared_sample_clocks`) whose `stream` lies within the resource's matched node; a clock declared again after a `cold` change does not exist at `arm`. The clock used is the one whose `stream` equals the entry's rewritten target, when there is one; otherwise every stream clock of the resource must share one `root_ticks_per_tick`, which is used; otherwise the entry is ambiguous. A stream clock not hanging off the Authority's primary root cannot be used. The resolved instant is `T0 + offset_ticks · root_ticks_per_tick`, in root ticks of the primary root, rounded up to a whole tick; `offset_ticks` must not be negative (Phase 2, KA-8)."

**SB-43 amended** (the forward sentence replaced): "`arm` is the coordinator's (KC-14…KC-19): it fixes T0, resolves every entry, refuses an ambiguous or unresolvable one, and dispatches each resolved Action at start or at its instant."

**Code.** `src/plan/compile.rs::derive_class` reads the closed field set **after** its existing class checks, so that a section with no `class` is still refused as "declares no `class`" (the order `ma_41_an_absent_or_non_string_class_is_refused` asserts), and then calls `start_lead_ns`. `src/binding.rs`: `pub fn start_lead_ns(environment: &BTreeMap<Namespace, serde_json::Value>) -> Result<u64, SpecError>` — the one reader, called by `derive_class` and by the coordinator; a refusal is `SpecError::Structural` with a reason beginning `"MA-41: "`. Its allow-list line is `binding::start_lead_ns = NEW: KA-8's single reader of ezsdr.time.start_lead_ns, used by plan() and the coordinator`.

**Tests.** `ma_41_ezsdr_time_is_a_closed_set` (`{class: simulation, start_lead_ns: 5}` and `… 2^62` plan; `{class, start_lead: 5}`, `start_lead_ns` of `-1`, `1.5`, `"5"` and `2^62 + 1` are refused with a reason naming MA-41; the reader returns 0 for an absent section); the resolution tests of KC-16 (§12).

### KA-9 — Lifecycle calls per fragment and per instance

**Evidence.** SB-34 lets two Spec resources bind two nodes of one instance, so one instance may have two Provider fragments, and SB-3 makes instance identity the binding description. MA-7 says "one call at a time per instance, in the order `prepare → arm → start → (step)* → stop → cleanup`" and never says how many `prepare` calls an instance with two fragments receives, nor where it sits in the arm order.

**MA-7 amended** (sentence added): "`prepare` is called once per **fragment**; `arm`, `start`, `step`, `stop` and `cleanup` once per **instance**. An instance's position in the arm and start order is the position of its first fragment in the plan; in a per-fragment cleanup step (RS-6) it is acted on at the first of its fragments the reverse pass reaches, and not again. A Provider may refuse a second fragment in `prepare` with `Rejected` when it serves one resource per instance, as MockRadio does (MR-7) (Phase 2, KA-9)."

**Code.** The coordinator (KC-13).

**Tests.** `ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once`.

### KA-10 — What each update class means

**Evidence.** D109 raised: "The four classes' timing semantics (§27) are defined nowhere, and Executors and Providers will need them." `UpdateClass::Cold`'s doc comment reads "Requires a stop and a re-plan; a sample-rate change is one" (`src/module_api.rs:336`), while TM-13c makes a sample-rate change a `cold` update **within** a Run and RS-40 tests a capture spanning one. The comment is also the schema's description (OV-10).

**Change.** The rules UC-1…UC-6 of §10 are added to spec 05 as a new §5 subsection "Update classes", and MA-24's second sentence reads "…and is applied under the parameter's declared update class as UC-2…UC-6 define it." **TM-13e amended** (sentence added): "A transmit stream re-created by a `cold` update (UC-3) takes that update's effective instant as the origin of its new SampleClock (Phase 2, KA-10)." The four doc comments become:

| Variant | Doc comment |
|---|---|
| `Cold` | "The target stops the affected function, applies the value and restarts it; a stream continues on a new SampleClock (UC-3, TM-13c)." |
| `BlockBoundary` | "Applied from the first block or sample boundary at or after the effective instant; no block mixes the old and new value (UC-4)." |
| `AtomicRealtime` | "Applied to every sample processed after the Action arrives, with no torn state (UC-5)." |
| `HardwareTimed` | "Applied by the device at exactly the effective instant, which must respect the device's command lead (UC-6)." |

**Schema.** Descriptions only, in the five schemas that embed `UpdateClass`: `action`, `action_template`, `component_descriptor`, `experiment_spec` and `vocabulary_descriptor`. `SCHEMA_CHANGELOG.md` gains one entry, "v1 — 2026-09-24 — pre-freeze revision, Phase 2 KA-7 and KA-10", covering this and KA-7.

**Tests.** The UC rules are producer obligations of each Module (UC-2 is the Kernel's and is tested by KC-25's test).

### KA-11 — `Authority::next_wakeup` advances the Authority

**Evidence.** MA-30's loop is `until = authority.next_wakeup()` and then `step(until)`. The `Authority` trait has no other method that moves time, so if `next_wakeup` only reported the next instant, nothing would advance `now` and no callback would fire; the trait's doc comment ("The next instant at which anything is due") reads as a query.

**MA-29 amended** (sentence added): "`next_wakeup` **advances** the Authority's governed clocks to the earliest instant at which a scheduled callback is due, fires the callbacks due at that instant in TM-16c order — including ones they schedule at that instant, up to TM-17b's cap per call — and returns the instant; with nothing scheduled it returns `None` and moves nothing. A step-driven Module that needs to run at an instant schedules a callback there, which may do nothing (Phase 2, KA-11)."

**MA-30 amended** (its italic *Ceiling* sentence, which begins "Ceiling: a zero-latency Event or Action cycle", replaced): "*Ceiling: a zero-latency Event or Action cycle at one instant hits the cap, and the coordinator also counts consecutive `next_wakeup` results at one instant, more than `STEP_ROUND_CAP` of which is `STEP_LIVELOCK` (KC-22). Delivery latency on inter-Island Action edges, which would let such a cycle advance in time, is not added in Phase 2, which has no Reactor and so no such edge; it is Phase 5's (Phase 2, KA-11).*" In the rule's `text` block, "None ends the Run" becomes "None ends a Spec Run (KC-33); a Session waits for its client".

**Code.** The doc comment of `Authority::next_wakeup` in `src/module_api.rs`. The Engine (SE-9) and the test Authority implement it.

### KA-12 — What each cleanup step does in terms of role-trait calls

**Evidence.** RS-6's steps 2 and 3 are "stop TX" and "stop RX", but a role trait has one `stop(mode)`, and TX and RX are radio words the Kernel cannot distinguish (OV-21). RS-6 also names no step for stopping Executors and Sinks, while MA-7 requires `stop` before `cleanup` for every instance, and MA-26 makes a Sink's `stop` return its artifacts.

**RS-6 amended** (a table added after the algorithm): "The coordinator performs the steps as follows. **0** nothing (Phase 2 has no child Runs, KC-37). **1** refuse every further submission, empty every undelivered Action queue, cancel every callback the coordinator scheduled. **2** `Provider::stop(mode)` on each Provider instance. **3** in an `orderly` cleanup of a Simulation Run, first run the stepping loop until `next_wakeup` returns `None`, at most `DRAIN_WAKEUP_CAP` times, so that each Provider delivers what its orderly `stop` publishes (MA-13) — counting consecutive results at one instant as KC-22 does, emitting `STEP_LIVELOCK` and stopping above `STEP_ROUND_CAP`, stopping at once when the end has escalated to `abort`, and stopping at its next wakeup once the `closing` flag is set, which every cleanup operation after the drain's own call sets on entry and the coordinator sets when `run_cleanup` returns, so that a drain RS-8a abandoned stops before the next step acts; a drain round also skips every instance whose step-5 `cleanup()` has been called; then `Executor::stop(mode)` on each Executor instance and `Sink::stop(mode)` on each Sink instance, keeping the artifacts each Sink returns. **4** nothing (no Peripherals). **5** `cleanup()` on every instance that reached `prepare`. **6** record the marks of RS-30 on the artifacts (KC-42). **7** drain the event path a last time, snapshot the counters, and read every created link's `drops()` (KA-18). **8** nothing inside `run_cleanup`: when it returns, the coordinator releases the Lease, assembles and seals the Manifest (KC-44), and only then drops the created links. In steps 3 and 5 a Module slot is taken with `try_lock`: a slot an abandoned earlier step still holds is not waited for — step 3's drain steps the other instances without it, and step 5 records a `CleanupFailure` for it ("KC-39: the instance is still held by an abandoned cleanup step") — so that one wedged Module costs one deadline and not one per later step. A radio Provider stops its transmit side before its receive side inside `stop` (RM-16), which is where RS-6's TX-before-RX order lives once the Kernel cannot tell them apart. Step 3's drain runs once, when the first fragment of step 3's reverse pass is performed, before that fragment's instance is acted on (Phase 2, KA-12)." *(Steps 6 and 7 since reordered: the last drain comes before the marks; RS-6 holds the live table — note 25, #66. Since pre-freeze audit item 4 the steps are named for what they do and numbered 1–7, steps 0 and 4 deleted: this table's 5–8 are RS-6's 4–7.)*

**RS-6 amended in Phase 7** (KA-12's table, step 3, sentence added; spec 04 carries it): "In a device-paced class step 3's drain is instead: stop and join the data thread (KC-46), if it exists; then, under `orderly`, one `step_until_quiescent` at the current instant over every Executor and Sink whose step-5 `cleanup()` has not been called — the Providers stopped in step 2, so what they published is in the links and the round is finite — and under `abort` none; then `Executor::stop` and `Sink::stop` as before (Phase 7, KG-3)." *Checked by `kg_03_step_3_runs_a_final_round_over_the_sinks`.*

**RS-9 amended** (sentence added): "Under `orderly` cleanup step 3 drains the stepped instances before the Executors and Sinks stop; under `abort` it does not (Phase 2, KA-12)."

**MA-7 amended** (the order sentence): "…in the order `prepare → arm → start → (step)* → stop → (step)* → cleanup`. `step` follows `stop` only in an orderly cleanup of a Simulation Run, where cleanup step 3's drain steps every stepped instance until nothing is scheduled, so that a Provider delivers what its orderly `stop` publishes (MA-13) and an Executor or Sink receives it before its own `stop`; after an abort no `step` follows `stop` (Phase 2, KA-12)."

**Code.** The coordinator's `CleanupOps` (KC-39). `run_cleanup` is unchanged.

### KA-13 — A Provider's Manifest sections, and whose namespace they are under

**Evidence.** RS-39: "A Module writes only under its own registered namespace", but no Phase 1 document registers a namespace for a Module, and a Provider has no call through which to hand its burst records (SC-28) or its envelope to the Manifest at the end of a Run. `ProviderInstance.sections` exists and is described as "Namespaced Manifest content".

**RS-39 amended** (sentences added): "A Module's own namespace is its `ModuleId` read as a `Namespace` (SB-T0); a Module whose id is not one writes no section, and a section it hands in is refused and recorded as a cleanup failure of step 8. A Provider hands its sections to the Manifest through `ProviderInstance.sections`: the coordinator reads `instance().sections` of every Provider instance after `stop` and writes each entry with `Manifest::write_section(owner, …)` (KC-44) (Phase 2, KA-13)."

**MA-10 amended** (sentence added): "A Provider's `instance()` returns the same `id`, `tree`, `fidelity`, `driving`, `arm_after` and `min_command_lead` for the whole Run; only `sections` may change, as the Provider records what happened."

### KA-14 — Two more reserved path segments

**Evidence.** The coordinator emits Kernel events (`STEP_LIVELOCK`, `DEVICE_LOST`) under a source of its own, and `EventCollector` already uses `unforeseen` as its fallback source (`src/event.rs:260`). A Provider tree may declare a node `unforeseen` or `kernel` today, which would make one `ResourceId` mean two things in the counters.

**SB-22h amended**: "The first path segments `sink`, `kernel` and `unforeseen`, and every first segment that begins with `island_`, are reserved: a bound instance's tree may declare no node under any of them. `kernel` is the Kernel's own event source (KC-31), `unforeseen` is RS-33's fallback row, and `island_<n>` is an Island's fragment id (SB-22a), which is its Executor's event source (KC-8) (Phase 2, KA-14)."

**Code.** `src/plan/validation.rs`: the SB-22h check covers the four cases. `src/coordinator/`: `pub const KERNEL_SOURCE: &str = "kernel";` (allow-list: `coordinator::KERNEL_SOURCE = NEW: KA-14's reserved source of the Kernel's own events`).

**Tests.** `sb_22h_the_sink_path_segment_is_reserved` gains cases for `kernel`, `unforeseen`, `kernel/x`, `island_0` and `island_12/x`.

### KA-15 — `AttachedPort` names its component

**Evidence.** `AttachedPort { port, endpoint }` (`src/module_api.rs:917`) is handed to an Executor for its whole Island, whose components may each have a port called `in`.

**MA-27 amended** (sentence added): "An attached end names the component and the port it is bound to; for a Provider the component is the Spec resource name and for a Sink its output id (Phase 2, KA-15)."

**Code.** `pub component: Ident` on `AttachedPort`.

### KA-16 — The root's relation to UTC in the Simulation class

**Evidence.** TM-18: "Every Run records in its Manifest a `ClockRelation` from its root domain, device or virtual, to `utc`". A virtual root advancing ten seconds in 0.3 s of wall time has no relation to UTC; any recorded relation would be false.

**TM-18 amended** (sentence added): "In the Simulation class the primary root is virtual and has no relation to `utc`; the Manifest records none, and the Run's placement in wall-clock time is the `host_utc_nanos` of its transitions (RS-5) (Phase 2, KA-16)."

### KA-17 — `ManualTimeAuthority::next_due`

**Evidence.** The coordinator's tests need an `Authority` over the Kernel's own test Authority, and `ManualTimeAuthority` exposes no earliest pending instant.

**Change.** Behind the `testing` feature (OV-20), `pub fn next_due(&self) -> Option<TimePoint>`: the earliest pending callback's instant on the primary root, or `None`. TM-17a gains the sentence "`next_due` reports the earliest pending instant on the primary root without advancing."

**Tests.** `tm_17_next_due_reports_without_advancing`.

### KA-18 — Link drop counts reach the Manifest

**Evidence.** SC-20: "Reading the count into the event collector is the Phase 2 coordinator's — a forward obligation." A drop-class link's drops are expected, so an event per drop would mark every probe's artifact; what is owed is the count.

**SC-20 amended** (the forward sentence replaced): "The coordinator reads every created link's `drops()` at cleanup step 7, before any link is dropped, and records `{ link, from, to, drops }` per link in the Manifest section `ezsdr.links` (KC-45) (Phase 2, KA-18)."

**RS-38 amended**: `sections` "including `ezsdr.capture` when the profile asks, `ezsdr.links` always, and `ezsdr.failure` when a stage failed (KC-7)". *(Since pre-freeze audit item 4 the Kernel writes no section: the link records are the typed `links`, and the reason is in `Failed`; SC-20 and RS-38 hold the live text.)*

### KA-19 — Markers that move

**RS-4** (marker): "…its call site is the first API that changes a plan (a re-plan or a GraphEpoch switch); Phase 2 has none, because a `cold` update changes a value and not a structure (UC-3)." **RS-25, RS-25a** (marker): "…the Phase 6 coordinator's, where `sdr.run(spec)` first exists; Phase 2 refuses `RunChild` (KC-37)." **SC-24a** (marker): "…tested against the first Provider with a transmit port fed by a link (Phase 10, or earlier); MockRadio's Phase 2 TX takes `TxBurst` Actions only (MR-16)." **MA-39** (marker): "…Forward obligation, Phase 10, when a Processor declares port rates." **MA-17** (marker): "…Producer obligation of the first Provider with a Peripheral (Phase 9); MockRadio 1.0.0 has none." **MA-19b** (marker): "…Producer obligation, tested by the Phase 5 Reactor Executor; Phase 2 has no Executor Module, only the coordinator's test doubles." Every other Phase 1 marker that names Phase 2 is replaced by the Phase 2 rule of `00-overview.md` §8 that carries it, in the form "*Checked in Phase 2 by `<test>` (KC-n).*"

### KA-22 — Spec 04's scope lines

**Evidence.** Spec 04's header puts "fault injection (Phase 4)" and "the Radio Model's event payloads (Vocabulary, Phase 2)" out of its scope, and its closing list repeats "Fault injection and the acceptance test it enables (Phase 4)". Phase 2 carries three fault kinds and the acceptance tests §58 #5 and #6 that need them (`00-overview.md` Y9, §8; spec 08 SE-3, SE-4), and RM-22 commits the payload schemas.

**Spec 04 amended**, in both places: "Fault injection beyond the three kinds of spec 08 SE-3 (`rx_overflow`, `rx_sequence_error`, `device_lost`), which Phase 2 carries with the acceptance tests §58 #5 and #6 (Phase 4)"; and the payload item gains "(committed by the Radio Model in Phase 2, RM-22)" (Phase 2, KA-22). **RS-31**'s marker becomes "*Checked: `schema_freeze` for the shape; the Radio Model's payload schemas by `rm_20_schema_freeze` and `rm_22_payloads_round_trip` (Phase 2, RM-22).*"

**Code.** None.

### KA-20 — The surface gate accepts Phase 2 rule ids

**Evidence.** OV-23 requires every public Kernel item's doc comment to cite a rule id, and `tests/kernel_surface.rs` recognises an id only by its prefix: `RULE_PREFIXES` holds the six Phase 1 series `OV-`, `TM-`, `SC-`, `SB-`, `RS-`, `MA-`. Every Kernel item Phase 2 adds cites a `KA-`, `KC-` or `UC-` rule, so the gate would refuse all of them.

**OV-23 amended** (sentence added): "The Phase 2 series `KA-`, `KC-` and `UC-` are rule ids for this purpose; `RM-`, `SE-`, `MR-`, `HD-` and `PO-` are not, because no Kernel item implements them (Phase 2, KA-20)."

**Code.** `tests/kernel_surface.rs`: `const RULE_PREFIXES: [&str; 9] = ["OV-", "TM-", "SC-", "SB-", "RS-", "MA-", "KA-", "KC-", "UC-"];`.

**Tests.** `kernel_surface` itself, which the amended Kernel items of KA-2, KA-3, KA-7, KA-8, KA-17 and KA-21 pass only with this change.

### KA-21 — `SessionLog::check_entry`

**Evidence.** RS-15: a refused, malformed Action "takes no sequence number". Phase 1's `SessionLog::append` performs its checks and the append in one call, so the coordinator could learn that an Action is malformed only by appending it — after `compile` and admission had already run on it, and with a sequence number spent. KC-28 step 1 must reject a malformed Action before anything else looks at it.

**RS-15 amended** (sentence added): "`SessionLog::check_entry(time, action)` performs exactly the checks `append` performs, without appending; `append` calls it first. The coordinator calls it before compiling a Session Action (KC-28) (Phase 2, KA-21)."

**Code.** `src/session.rs`: `pub fn check_entry(&self, time: &TimePoint, action: &SessionAction) -> Result<(), SpecError>` = `action.check_values()?; check_ids(time, action)`; `append` calls `self.check_entry(&time, &action)?` in place of those two calls. A method on an existing type is not an allow-list item (D10).

**Tests.** `kc_28_a_malformed_action_takes_no_sequence_number` (§12), through the coordinator. The patch's existing `run_session` tests prove that `append` still refuses what it refused.

### The amendment patch

KA-1, KA-2, KA-3, KA-4, KA-5, KA-7, KA-8, KA-10, KA-11 (the doc comment), KA-14 (the SB-22h check), KA-15, KA-17, KA-20 and KA-21, with their tests and schema changes and the `SCHEMA_CHANGELOG.md` entry, are one patch, `plan/phase2/patches/01-kernel-amendments.patch`, against commit `96976c5`. It was produced from a working copy on which `cargo test --workspace` passes 361 tests on Rust 1.85.0 and on stable and `cargo +stable clippy --workspace --all-targets -- -D warnings` passes; every new test was confirmed to fail with its rule disabled (PO-12). The implementer applies it rather than retyping it (`20-implementation-plan.md` step 1). The amendments that live in the coordinator — KA-6, KA-9, KA-12, KA-13, KA-18 and the `KERNEL_SOURCE` constant of KA-14 — and the text-only KA-16, KA-19 and KA-22 are not in it. The one crate-private addition it makes is `plan::validation::binding_description(binding) -> (ModuleRef, String)`, which SB-3's check now calls; the coordinator's KC-4 grouping must call the same function (step 4 adds its `pub(crate) use` to `src/plan.rs`), never a copy of it.

## 3. The coordinator: model

```text
            Assembly (documents' Modules, registries, clocks, host clock)
                │
   start_spec_run(spec, profile) ─┐        connect(profile, lease) ─┐
                                   ▼                                 ▼
 Created → validate → plan → links → prepare* → collect_prepare → arm* → T0 → schedule → start* → Running
                                                                                                │
                            ┌───────────────────────────────────────────────────────────────────┘
                            ▼
   loop: t = authority.next_wakeup()   ── agenda ≤ t ── step_until_quiescent(t) ── drain events ── policy ── lease
                            │                                    ▲
     submit(SessionAction) ─┴─ compile → rewrite → coerce → admit → log → dispatch → one round at now
                            │
     end request (Stop{}, policy, client, lease, error)
                            ▼
   Stopping{mode} → run_cleanup (RS-6 via KA-12) → Manifest assembled and sealed → CleanedUp
```

The coordinator is single-threaded except for `run_cleanup`'s per-step threads (RS-8a). Every Module instance is held behind its own `Mutex`, so that a cleanup thread can reach it and a wedged one blocks only itself.

`run_cleanup` runs each step on a thread of its own and needs `'static` access, so everything a cleanup step reads or writes lives in one `Arc<Shared>` that both the `RunHandle` and the `CleanupOps` object hold: the Module slots, the Authority, the event collector, the compiled Policy, the delivered events, the recorded marks, the pending end request with its mode, the `also` list, the Sinks' artifacts, the created links, the set of fragments that reached `prepare`, the set of `(step, instance)` pairs already acted on, and the dispatch-frozen flag — each behind its own `Mutex`. Cleanup step 3's drain runs stepping rounds (KA-12), which deliver events, record marks and may escalate the end, so all of those are in `Shared` too. The Module-origin submitter (KC-24) admits Actions from inside a round, so the parsed documents, the registries, the state machine and the per-fragment configuration are in `Shared` as well. State only the control path touches — the Session log, the agenda, the Lease and the documents' Manifest sections computed at entry — stays in the `RunHandle`; the input store is in `Shared`, because admission reads it for a Module's burst from inside a round (Phase 5, KE-2).

## 4. Types

```rust
pub mod coordinator {
    /// KC-4: what the runtime hands the coordinator for one Run.
    pub struct Assembly {
        pub registry: ModuleRegistry,                        // modules, vocabularies, link descriptors
        pub checks: AdmissionCheckRegistry,
        pub kinds: EventKindRegistry,                        // the Kernel's five plus every Vocabulary's
        pub contracts: ContractRegistry,
        pub clocks: Arc<ClockRegistry>,                      // the one the Authority registered its roots in
        pub host_clock: Arc<dyn HostClock>,
        pub providers: BTreeMap<Ident, Box<dyn Provider>>,  // one object per binding description (KC-4)
        pub executors: BTreeMap<Ident, Box<dyn Executor>>,  // by Island executor name
        pub sinks: BTreeMap<Ident, Box<dyn Sink>>,          // by output id (or Sink binding name in a Session)
        pub authority: Box<dyn Authority>,                   // the one named by `authority`
        pub links: BTreeMap<ModuleRef, Box<dyn Link>>,       // one per Link Module version
        pub inputs: BTreeMap<ContentHash, Vec<u8>>,          // the bytes of every input the Spec names, by hash (KC-9)
    }

    pub fn start_spec_run(spec: &serde_json::Value, profile: &serde_json::Value, assembly: Assembly)
        -> Result<RunHandle, SpecError>;
    pub fn connect(profile: &serde_json::Value, assembly: Assembly, lease: Lease)
        -> Result<RunHandle, SpecError>;

    pub struct RunHandle { /* private */ }
    impl RunHandle {
        pub fn id(&self) -> RunId;                              // KC-5: a clone of the Run's id
        pub fn kind(&self) -> RunKind;
        pub fn state(&self) -> RunState;                        // a clone: the machine lives in Shared (§3)
        pub fn now(&self) -> TimePoint;                         // in the Authority's primary root
        pub fn start_instant(&self) -> Option<TimePoint>;       // T0, once armed
        pub fn advance_to(&mut self, t: TimePoint) -> Result<(), RunHandleError>;
        pub fn run_until_end(&mut self, horizon: TimePoint) -> Result<(), RunHandleError>;
        pub fn submit(&mut self, action: SessionAction, waveform: Option<&[u8]>)
            -> Result<LogEntry, RunHandleError>;
        pub fn effective(&self) -> BTreeMap<Ident, BTreeMap<Key, Value>>;
        pub fn sample_clocks(&self) -> Vec<SampleClockRecord>;
        pub fn disconnect(&mut self);
        pub fn check_lease(&mut self);
        pub fn finish(self) -> Manifest;
    }

    pub enum RunHandleError {
        Ended { termination: Termination },     // the Run is CleanedUp
        NotSession,                             // submit on a Spec Run
        Malformed { error: SpecError },         // not an Action at all (RS-15); takes no sequence number
        NotOnPrimaryRoot { t: TimePoint },      // advance_to a time the primary root cannot place
    }

    pub const KERNEL_SOURCE: &str = "kernel";             // KA-14
    pub const EVENT_RING_DEPTH: usize = 4096;              // KC-8
    pub const DEFAULT_HOST_BUDGET_NS: i64 = 5_000_000_000; // KC-11
    pub const DRAIN_WAKEUP_CAP: usize = 1_000_000;         // KA-12
}
```

Allow-list lines (PO-5), in `tests/kernel_surface_allow.txt` under a new heading `# --- coordinator (Phase 2, design/06-kernel-coordinator.md)`: `coordinator = Run / Session state machine`, `coordinator::start_spec_run = Run / Session state machine`, `coordinator::connect = Run / Session state machine`, `coordinator::RunHandle = Run / Session state machine`, `coordinator::Assembly = NEW: KC-4's bundle of the registries, clocks and Module instances the runtime hands one Run`, `coordinator::RunHandleError = NEW: the refusals of a live Run's control API (RS-15, RS-18, KC-28)`, `coordinator::KERNEL_SOURCE = NEW: KA-14's reserved source of the Kernel's own events`, `coordinator::EVENT_RING_DEPTH = NEW: RS-34's bounded ring, sized once by the coordinator`, `coordinator::DEFAULT_HOST_BUDGET_NS = NEW: MA-8's bound on prepare and arm where nothing declares one`, `coordinator::DRAIN_WAKEUP_CAP = NEW: KA-12's bound on the orderly drain`. The `NEW:` count rises by six. Every item in the coordinator's submodules is private or `pub(super)`, never `pub`, so that these ten are its whole public surface.

## 5. Rules — assembly and entry

- **KC-1** The coordinator is the Kernel component that drives one Run. `start_spec_run` parses the Spec with `ExperimentSpec::from_json` and the profile with `BindingProfile::from_json`; `connect` parses the profile, checks the Lease with `Lease::validate` (RS-21) and derives the implicit Spec with `session::implicit_spec` (SB-22c). A parse, Lease or derivation failure returns `Err` and there is no Run, because no document became one. Otherwise a `RunHandle` is returned, **even when a later stage fails**: the handle is then `CleanedUp` and `finish` returns its sealed Manifest (RS-11).
- **KC-2** The coordinator drives the Simulation, HardwareInLoop and Hardware classes. HardwareInLoop and Hardware are the **device-paced classes**, to which KC-2a, KC-12a, KC-21a, KC-24a, KC-46…KC-46c and the device-paced clauses of KC-29, KC-33, KC-36, KC-37a and KC-45 apply. A plan whose derived class is RealtimeEmulation is refused after `plan()` with `Failed { plan }` and the reason `"KC-2: RealtimeEmulation needs a wall-paced Simulation Engine, which no phase has built yet (Phase 10)"` (Phase 7, KG-1; until then the coordinator drove the Simulation class only, Y1). *Checked by `kc_02_a_wall_paced_authority_is_refused` (its reason changed in Phase 7), `kg_01_a_device_paced_run_reaches_running` and `kg_01_the_hardware_class_reaches_running`.*
- **KC-2a** In a device-paced class the coordinator also refuses, after `plan()` and before KC-9, with `Failed { plan }`: a Provider instance whose `instance().driving.stepped` is true, with the reason `"KC-2a: <first fragment> is a stepped Provider, and a device-paced class steps none (MA-30)"`; and an Island whose `IslandDecl` has `affinity` or `rt_policy`, with the reason `"KC-2a: <island id> declares <affinity | rt_policy>, which a device-paced class does not apply before Phase 10"`. The first offending instance in plan order is named (Phase 7, KG-1). *Checked by `kg_01_a_stepped_provider_is_refused_in_a_device_paced_class` and `kg_01_an_island_with_an_rt_policy_is_refused`.*
- **KC-3** The coordinator reads `authority.descriptor()` to build `CompileInputs::authorities` under the profile's `authority` name, calls `authority.time()` once and hands that handle to every `PrepareContext`, and calls `next_wakeup()` only from the stepping loop (KC-20), the cleanup drain (KA-12) and `advance_to` (KC-29). *This carries MA-29's forward half.*
- **KC-4** The runtime supplies each Module instance once. Providers: the coordinator groups the Spec's resource names by binding description `(module, selector, profile)` (SB-3) and requires exactly one object in `providers` under one of each group's names; two objects for one group, or an object under a name that is no resource's, fail the Run with `Failed { validate }`. Sinks by output id, Executors by Island executor name, the Authority by the profile's `authority`, Links by the `ModuleRef` `placements.links` selects. `CompileInputs::providers` then maps **every** resource name of a group to the group's one object.
- **KC-5** A `RunHandle` composes the Run's id (`RunId::generate`), kind, parent (always `None` in Phase 2), Lease, compiled Policy and, for a Session, its `SessionLog`, and the Manifest records each (RS-1). *This carries RS-1a.*
- **KC-6** The registries in the `Assembly` are the Run's; nothing is registered after entry.

## 6. Rules — from `Created` to `Prepared`

- **KC-7** Stages run in this order and each failure moves the Run to `Stopping { abort }` with `Failed { stage, reason }` and runs cleanup (RS-3), the reason a string beginning with the rule that refused, such as `"KC-2: …"` or the `SpecError`'s own text: `validate` and compiling the Policy (KC-8; a refusal is `Failed { validate }`, since it is the Spec's `policies.failure` that names an unknown kind) (→ `Validated`), `plan` and KC-2 (→ `Planned`), KC-9's inputs, KC-10's links, KC-8's event path, KC-12's prepare loop and `collect_prepare` (→ `Prepared`). A failure after `Planned` and before `Prepared` is `Failed { prepare }`, except KC-9 and KC-10, which are `Failed { plan }`.
- **KC-8** The Policy is `kinds.compile(&spec.policies.failure)`. The event collector is `EventCollector::new(pairs, kinds, EVENT_RING_DEPTH, &policy)` where `kinds` is every registered kind and `pairs` is: for each Provider instance, every node of its tree × every kind registered by a Vocabulary its `ModuleDescriptor` declares; for each Sink, `sink/<output id>` × its Vocabularies' kinds; for each Island, `island_<n>` × its Executor Module's Vocabularies' kinds; every instance's source root — a Provider's `instance().id`, `sink/<output id>`, `island_<n>` — × `DEVICE_LOST`, which KC-30 emits under it; and `kernel` × the Kernel's five kinds, so the row `(kernel, EVENTS_DROPPED)` the collector guarantees itself (RS-33) is a planned one. Duplicate pairs are passed once. The collector is handed to every `PrepareContext` as `events`. A pair not in this list is counted in RS-33's `unforeseen` row, never lost.
- **KC-9** Every `ArtifactRef` the Spec's `inputs` lists and every one a Spec schedule entry carries is an input (SB-20a; Phase 5, KE-1). Its `uri` must begin with `mem:` or `file://`, and `Assembly.inputs` must hold bytes under its `hash` whose length is its `size_bytes` and whose `ContentHash::of_bytes` is its `hash`, its `partial` must be false and its `marks` and `continuity` empty — what the Run records on an artifact it *produces* — and no other input with a different `hash` may carry its `id` (Phase 5, KE-1); anything else is refused (`Failed { plan }`, reason beginning `"KC-9: input <i>: "` for the `i`-th listed input or `"KC-9: entry <i>: "` for a schedule entry). The coordinator never opens a uri: the runtime that built the Assembly read the file or holds the buffer and supplies the bytes, so the Kernel does no file input and a Run's inputs are exactly what it was handed. The bytes of the inputs so verified become the Run's input store, which KC-28 extends; an `Assembly.inputs` entry that the Spec's `inputs` does not list and no schedule entry carries is not kept, so the store holds only verified bytes (Phase 3, KB-1; Phase 5, KE-1). *Checked in Phase 3 by `kb_01_b_the_store_keeps_no_unverified_entry`.* Each distinct hash is recorded once in `Manifest.inputs`, as the `ArtifactRef` the Spec wrote, the listed inputs first in their order and then the schedule's in schedule order (Phase 5, KE-1). *Checked in Phase 5 by `ke_01_a_declared_input_is_stored_and_recorded` and `ke_01_a_declared_input_is_verified_as_a_scheduled_one_is`.* *This is RS-44a's rule for a Spec Run; KC-28 is a Session's.* *Ceiling: an artifact a Run produced — a capture, which carries its `continuity` — is listed as another Run's input without its `partial`, `marks` and `continuity`; its hash is what links it to the Manifest that produced it (Phase 5 Review G).*
- **KC-10** For each `DataLinkDecl` in `plan.links`, in order: the `LinkPlacement` naming its `(from, to)` selects a `ModuleRef`; the `Link` supplied under it must return a `descriptor()` equal to `registry.link_descriptor(ref)`, and is refused otherwise (MA-27a, D79); its `create(decl)` builds the link. The producing end is attached to the fragment that owns `decl.from` — a resource name's Provider fragment, or a component's Island fragment — and the consuming end to `decl.to`'s owner — a component's Island, an output id's Sink fragment, or a resource's Provider fragment — each as `AttachedPort { component, port, endpoint }` (KA-15). The coordinator keeps every created link until the Manifest is assembled (RS-6 step 7). *This carries MA-27a.*
- **KC-11** Each fragment's `PrepareContext` carries: the Run id; the class; KC-3's time handle; the Assembly's `clocks`; the collector; the fragment's instance's own `ActionReceiver` (one queue per instance); the coordinator's `ActionSubmitter`; the profile's `environment`; the Run's input store, shared with KC-9 and KC-28 (KB-1); KC-10's attached ends for this fragment; for an Island fragment, the descriptors of exactly that Island's components keyed by component name, and for any other fragment an empty map; and `host_budget = DEFAULT_HOST_BUDGET_NS` in `host.monotonic`. *This carries MA-19a.*
- **KC-12** The prepare loop calls, in plan order: `Provider::prepare(fragment, ctx)` for a Provider fragment, `Sink::prepare(fragment, ctx)` for a Sink fragment, `Executor::prepare(island, ctx)` for an Island fragment, and nothing for an Authority fragment (MA-2). The loop stops at the first `Err` (or panic, KC-30); every result so far is passed to `collect_prepare` together, which then refuses the transaction (SB-42). A fragment whose `prepare` was called has "reached `prepare`" for MA-7, whatever it returned. *This carries SB-41's loop.*
- **KC-12a** In a device-paced class the coordinator makes each `prepare` (KC-12) and each `arm` (KC-14) on a worker thread of its own that holds the instance's slot, and waits for it at most the fragment's `host_budget` (`DEFAULT_HOST_BUDGET_NS`, KC-11) — the mechanism RS-8a uses for a cleanup step. A call that has not returned by then is abandoned — its thread keeps the slot, as an abandoned cleanup step does — and counts as `ModuleError { kind: Timeout, message: "KC-12a: <prepare | arm> of <first fragment> did not return within <n> ms" }`: from `prepare` it fails the stage at once, `Failed { prepare }` with that reason, **without** calling `collect_prepare`, which locks every instance's slot (`with_inputs`, KC-12) and would wait for the abandoned call forever; from `arm` it is `Failed { arm }`. Cleanup then finds that slot held: step 2's `stop` waits for it until RS-8a's deadline abandons the step, and steps 4 and 7 record KC-39's and KC-44's failures for it (Phase 7, KG-6). *Checked by `kg_06_a_prepare_that_hangs_fails_the_run_within_its_budget`, `kg_06_an_arm_that_hangs_fails_the_run_within_its_budget` and `kg_06_the_simulation_class_prepares_on_the_callers_thread`.*
- **KC-13** Instance order: an instance's position is its first fragment's position in `plan.fragments`. `arm` and `start` run in instance order; `stop` and `cleanup` in RS-6's reverse passes (KA-9, KA-12). *This carries MA-7's cross-instance half.*

## 7. Rules — arm, the start instant and the schedule

- **KC-14** `arm` is called on every instance in instance order (`Provider::arm`, `Executor::arm`, `Sink::arm`); the first failure is `Failed { arm }` and no later instance is armed. Then the Run moves to `Armed`. `start` (KC-18) stops at its first failure the same way.
- **KC-15** **T0**, the Run's start instant, is the smallest instant at or after `now + lead` that is a whole multiple of `L`, where `now` is the Authority's current instant on its primary root after the last `arm`, `lead` is `start_lead_ns` (KA-8) rescaled from `host.monotonic` to the primary root with TM-9 and rounded up to a whole tick, and `L` is the least common multiple of the numerators, in lowest terms, of `root_ticks_per_tick` of every SampleClock declared on the primary root when T0 is fixed (`ClockRegistry::declared_sample_clocks`); `L` is 1 when none is. An overflow of `L` or of T0 is `Failed { arm }` with the reason `"KC-15: overflow"`. T0 is later than `now + lead` by less than `L` root ticks (less than one period of the slowest declared stream when the ratios divide each other, as MockRadio's and the X310's `N` values of one Run usually do). Every declared stream then has a sample instant at T0, and with RM-25 two streams of one rate share every sample instant (Phase 7, KG-7; the bound after Review J, P2-4). *Checked by `kg_07_t0_lies_on_every_declared_grid`, `kg_07_with_no_declared_clock_t0_is_not_rounded` and, end to end on MockRadio 1.3.0, `kg_07_a_burst_at_t0_plus_n_samples_is_exact` (`crates/ezsdr-acceptance/tests/v58.rs`), which replaces `k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample`: the behaviour that test pinned is the defect KG-7 fixes.*
- **KC-16** Each `spec.schedule` entry resolves as SB-16 (KA-8) says: its template's target is rewritten (KC-23); its `SpecTime` gives an instant in the primary root; an ambiguous clock, a clock off the primary root, a negative offset or an overflow is `Failed { arm }` naming the entry's index.
- **KC-17** A **timed** template (`TxBurst`, `SetTimer`, `UpdateParameter`, `PeripheralCommand`, i.e. `is_timed()`) becomes its Action with `ActionTemplate::resolve(AbsoluteDeadline(instant))` and is admitted at once (KC-24, origin *schedule*); a refusal is `Failed { arm }` naming the entry. The timed entries are admitted in `(instant, schedule index)` order, each against a **working copy** of the per-fragment configuration (KC-27) to which every earlier admitted `UpdateParameter` has already been applied, so that an admission check sees the configuration the device will have when the entry applies. Without it two entries that are each safe alone — a transmit frequency outside the RF envelope while transmission is off, then transmission on — would both pass against the configuration of `prepare`. The working copy becomes the configuration when the entries are dispatched (KC-18, KC-25). An **untimed** template (`Stop`) is held in the coordinator's agenda at its instant and admitted and dispatched when the loop reaches it (KC-20); `Stop {}` with no target ends the Run `Completed` (KC-33); an agenda item that admission refuses requests the end `Failed { run }` in `abort` mode with the reason `"KC-17: agenda entry <index>: <violations>"`.
- **KC-18** `start` is called on every instance in instance order: `Provider::start(Some(T0))`, `Executor::start()`, `Sink::start()`; a failure is `Failed { arm }`. Then the Run moves to `Running`, the Actions KC-17 admitted are dispatched in the order KC-17 admitted them, `(instant, schedule index)` (KC-25), and one stepping round runs at the current instant (KC-21).
- **KC-19** Before `start`, a resolved `TxBurst` whose `late_policy` is `RejectAtPlan` is refused (`Failed { arm }`) when its instant minus the current instant is shorter than its target instance's `min_command_lead` (TM-21's exact comparison). A `RejectAtPlan` burst that does not come from the schedule is refused at admission (KC-24), because its target is not statically known (SC-27). *This is SC-27's plan-time rule and Vision §22's statically-known lead check.*

## 8. Rules — the stepping loop

- **KC-20** While the Run is `Running` and no end is requested:

  ```text
  t = authority.next_wakeup()                    None: a Simulation Spec Run ends Completed (KC-33); a Session or a device-paced Run returns
  count = (t equals the previous call's t) ? count + 1 : 1;  if count > STEP_ROUND_CAP: KC-22, then count = 0
  run every agenda item whose instant ≤ t, in (instant, insertion) order          (KC-17, KC-29)
  step_until_quiescent(stepped, t, collector, kernel)                              (MA-30; KC-23 is the set)
  drain the collector and apply reactions                                          (KC-31)
  check the Lease                                                                  (KC-36)
  ```

  The agenda's items reach the loop because the coordinator schedules a no-op callback on the Authority at each agenda instant when it adds the item (KC-17) and at each `advance_to` target (KC-29); it keeps every handle it schedules, and cleanup step 1 cancels them, every one even after a `cancel` panicked, recording the panics together as one `freeze_dispatch` `CleanupFailure`, `"KC-30: a Module panicked during cleanup: Authority cancel(), <n> of <m> handles"` (F25 (b)). *Checked by `kc_30_a_panicking_time_cancel_is_a_cleanup_failure` (the first `cancel` succeeding and every later one panicking: two handles give `1 of 2 handles`, three give `2 of 3`, so a handle after a panicking one is still tried).*
- **KC-21** After the coordinator dispatches any Action on the control path — a Session submission, the start dispatch, an agenda item — it runs one `step_until_quiescent` at the current instant, then drains and applies reactions, before returning to its caller. The target therefore sees the Action at the instant it was admitted, and a lead is measured from there.
- **KC-21a** In a device-paced class, after the coordinator dispatches Actions on the control path — a Session entry (KC-28), the start dispatch (KC-18), an agenda item (KC-20) — and before KC-21's round, it waits, in host time, until every instance it dispatched to in that call has finished with every Action dispatched to it (MA-14b), for at most `DEFAULT_HOST_BUDGET_NS` from the dispatch, re-checking at least every 10 ms whether an end has been requested or dispatch is frozen, which stop the wait. An instance that has not finished by then makes the coordinator request the end `Failed { run }` in `abort` mode with the reason `"KC-21a: <first fragment> did not finish its Actions within <n> ms"`. An Action a Module submits from a step (MA-14a) is not waited for: that step is still running on the data thread, and waiting would stall the thread that steps its target's peers (Phase 7, KG-4; after Review J, P1-2, P2-8). *Checked by `kg_04_a_session_call_returns_after_the_provider_has_finished`, `kg_04_the_next_admission_sees_the_state_the_previous_action_made` and `kg_04_a_provider_that_never_finishes_fails_the_run`; its scope by `kg_04_a_simulation_session_does_not_wait`.*
- **KC-22** When `next_wakeup` returns one instant more than `STEP_ROUND_CAP` times in a row, the coordinator emits `STEP_LIVELOCK` with source `kernel`, severity `fatal` and payload `{ "rounds": STEP_ROUND_CAP }` through `emit_control` — the one `STEP_LIVELOCK` payload, `step_until_quiescent`'s too (MA-30): each result at one instant is a round — and the Policy applies (its default is `abort`, RS-28). If the Policy's reaction is neither `stop` nor `abort` — a Spec may override any kind's reaction — the coordinator requests `Failed { run }` in `abort` mode itself, with the reason `"KC-22: STEP_LIVELOCK at <t>; time cannot advance"`, because a loop that cannot advance time cannot continue whatever the Policy says.
- **KC-23** The stepped set is every Provider instance whose `instance().driving.stepped` is true, every Executor instance and every Sink instance, each as a `SteppedInstance` whose `id` is its first fragment id; `step_until_quiescent` orders them (MA-30). *Target rewriting* — a target is Spec-relative (SB-16): a path whose first segment is `sink` names the output id in its second segment and stays as written, for that output's Sink fragment; one whose first segment is a Spec resource name `R` is rewritten to `matched[R]`'s path followed by the remaining segments, for `R`'s Provider fragment, and must parse as a `ResourceId`; one whose first segment is a graph component name `C` stays as written, for the Island fragment whose `components` list `C`; anything else is refused (`ezsdr.target`). Every origin — Session, schedule and Module — uses Spec-relative targets. An Action with no target (`Stop {}`, `Abort`) is not rewritten.

## 9. Rules — Actions, events, Sessions, cleanup and the Manifest

### Admission and dispatch

- **KC-24** Every Action passes one admission function before dispatch, whatever its origin (RS-16), in this order: (1) dispatch not frozen (RS-6 step 1); (2) for a Session or Module origin, the Run is `Running` — `RunStateMachine::check_running`, logged as RS-18 says; (3) the target rewrites (KC-23); (4) for a `TxBurst`: a `RejectAtPlan` burst from a Module origin is refused (KC-19); the target must be a Provider fragment and the Assembly's clocks must hold a registered SampleClock record whose `stream` equals the rewritten target and whose `ended_at` is absent, else refused (`ezsdr.target`, SC-23); `stream::admit_burst_target` converts `at` onto that clock (SC-23a, SC-23b) and sets `requested_at` when it advanced; and the waveform's hash must name bytes in the Run's input store whose length is its `size_bytes`, else refused (`ezsdr.input`, RS-44a; Phase 5, KE-2); (5) for an `UpdateParameter` to a Provider fragment from the Session or schedule origin, KA-6's coerce (KC-26); (6) `Admitter::admit` with the per-fragment configuration (KC-27), `proposed = { fragment: { key: value } }` for an `UpdateParameter` — the value after step 5's coercion — and empty otherwise, the coercions, `CheckStage::Runtime`, the Spec's coercion overrides and the Run kind; (7) at dispatch, not before, an `ActionId` from the Run's counter, which starts at 1, so that an entry some of whose Actions are refused consumes no id. `Action::Abort { cause }` from a Module is not dispatched: it requests the end `Stopped { cause }` in `abort` mode (KC-32) and `submit` returns `ActionId(0)` without consuming the counter. `Action::Stop { target: None }` from a Module is refused (`ezsdr.target`, reason "KC-24: a Module ends a Run only with Abort"), because a Run-ending `Stop` is the schedule's or the client's (KC-33). The refusals of steps 1–5 are these `Violation`s (`key` and `requested` are `None` unless stated):

  | step | when | `check` | `reason` |
  |---|---|---|---|
  | 1 | dispatch is frozen | `ezsdr.dispatch` | `"RS-6: dispatch is frozen"` |
  | 2 | not `Running` | `ezsdr.run_state` | `"RS-18: the Run is not Running"` |
  | 3 | the target does not rewrite | `ezsdr.target` | `"KC-23: <target> names no resource, output or component"` |
  | 3 | a Module's `Stop { target: None }` | `ezsdr.target` | `"KC-24: a Module ends a Run only with Abort"` |
  | 4 | a Module's `RejectAtPlan` burst | `ezsdr.late_policy` | `"KC-19: SC-27: RejectAtPlan needs a statically known target"` |
  | 4 | a burst whose target is not a Provider fragment | `ezsdr.target` | `"SC-23: <target> is not a Provider stream"` |
  | 4 | no running SampleClock for the target | `ezsdr.target` | `"SC-23: <target> has no running transmit SampleClock"` |
  | 4 | `admit_burst_target` fails | `ezsdr.target` | `"SC-23b: <the StreamError>"` |
  | 4 | the waveform is not in the input store (Phase 5, KE-2) | `ezsdr.input` | `"RS-44a: <hash> is not an input of this Run"` |
  | 4 | its stored length is not `size_bytes` (Phase 5, KE-2) | `ezsdr.input` | `"RS-44a: input <hash> holds <n> bytes, and the burst declares <m>"` |
  | 5 | `coerce` errs, rejects, or applies nothing | `ezsdr.coercion`, `key` = the key | `"RS-17: <fragment>: <the error, the rejection's reason, or 'its Provider applied no value for <key>'>"` |

  Step 6's violations are the `Admitter`'s own. *This carries SB-30's third point and RS-18.*
- **KC-24a** Admission and dispatch are serialized by one **admission lock** in `Shared`: a control-path call holds it from its first admission (KC-24) through its last dispatch (KC-25) — a Session entry's every compiled Action, the start's batch, an agenda item —, a Module's submission (MA-14a) holds it from its admission through its dispatch, and RS-6 step 1 holds it while it sets `frozen` and clears the queues. No Action is therefore dispatched after dispatch is frozen, and each entry is judged against the configuration the previous one left. The lock is never held across a round or across KC-21a's wait, so a Module's submission from inside a round never waits for the call that runs the round (Phase 7, KG-4; Review J, P1-2). *Ceiling (Review K, N-P2-11): a Provider that submits Actions from a thread of its own (MA-14a allows it) could, during KC-46b's `stop`, wait for this lock while the control thread holds it and waits for that Provider's slot (KC-26); the Provider's bounded join breaks the cycle after its bound. No Phase 7 Provider submits Actions.* *Checked by `kg_04_no_action_is_dispatched_after_the_freeze`.*
- **KC-25** Dispatch pushes the rewritten Action onto its target instance's `ActionReceiver` queue, which the Module drains with `recv()`; an admitted `UpdateParameter` also sets `configuration[fragment][key]` to its value. *This carries MA-24's delivery half.*
- **KC-26** KA-6's coerce: the coordinator locks the target Provider's slot (no round is running on the control path), builds the `Requested` of RS-17 step 0 from `configuration[fragment]` — every entry whose value is a scalar — and calls `coerce`.
- **KC-27** The coordinator's per-fragment configuration starts as `{ report.fragment: report.effective }` over every `PrepareReport` and changes only through KC-25. `RunHandle::effective()` returns it. It records what the Kernel **admitted**, not what a device applied: a Provider that refuses an admitted update at runtime (its Vocabulary's event, for example `radio.COMMAND_REJECTED` or `radio.COMMAND_QUEUE_FULL`) leaves the configuration holding the refused value, and later admission checks judge that value. *Ceiling: a Provider-to-Kernel feedback of applied values is later work; KA-6's coerce makes the refusals it can foresee happen at admission instead.*

### Sessions

- **KC-28** `RunHandle::submit(action, waveform)` on a Session, at the current instant `now`:
  1. refuses with `Ended` after cleanup, `NotSession` on a Spec Run, and `Malformed` when `SessionLog::check_entry(now, &action)` fails (RS-15: it takes no sequence number);
  2. a `waveform` is ingested with `manifest::ingest_input(input_<k>, ezsdr.input, "mem:<hash>", bytes)` (k counts from 0), stored in the input store, and recorded once in `Manifest.inputs` (RS-44a);
  3. `session::compile(&action, registry, declared_classes, outputs, earliest, waveform_ref)`, where `declared_classes` are the Spec components' `params` and `earliest` is RS-19's (KA-7) for the Action's target — `now` for a verb compiling to an `UpdateParameter`, which targets a Sink, and otherwise `now + min_command_lead` of the rewritten target's instance or the origin of the rewritten target stream's running clock, whichever is later;
  4. each compiled Action passes KC-24; if any is refused, none is dispatched and the entry is `Rejected` with every violation;
  5. a `ControlOp`: `StopRun` and `Release` end the Run `Stopped { client }` (KC-33), and `Release` also sets `lease.released`; `Adopt { token }` calls `Lease::adopt(token, "client")` and `Renew` calls `Lease::renew(host_clock)`, logging `AdoptRejected` or `LeaseNotRenewable` as a rejection with the violation check `ezsdr.lease`; `RunChild` is refused (KC-37). An admitted control operation's entry is `Admitted { coercions: [], warnings: [], dispatched: [] }`;
  6. if the entry is admitted, its Actions are dispatched (KC-25) in compiled order, which assigns their ids; then the entry is appended with `SessionLog::append` (dense sequence, RS-15) as `Admitted { coercions: every coercion of steps 3 and 4 in order, warnings: every warning, dispatched: those ids }` or `Rejected { violations }` (`check_entry` in step 1 guarantees the append cannot fail); then KC-21's round runs; an end request then runs cleanup (KC-38) before `submit` returns. `submit` returns `Ok(entry)` for an admitted and for a rejected entry; `Err` is only step 1's three refusals.
- **KC-29** `advance_to(t)` requires `t` in the primary root or in a domain exactly related to it (converted with `ClockRegistry::convert`, an inexact result rounded up); otherwise `NotOnPrimaryRoot`. When `t` is at or before `now` it returns `Ok` at once. Otherwise it schedules a no-op callback at `t`, runs KC-20's loop until a round has run at an instant at or after `t` or the Run ends, and returns `Err(Ended)` if it ended. `run_until_end(horizon)` schedules nothing in the Simulation class: it runs the loop until the Run ends (`Err(Ended)`), until a round has run at an instant at or after `horizon` (`Ok`; the last round's instant may be later than `horizon`, because an Authority cannot stop between its wakeups), or, in a Session, until `next_wakeup` returns `None` (`Ok`). In a device-paced class it schedules a no-op callback at `horizon` as `advance_to` does, and returns `Ok` after a round at or after `horizon` or `Err(Ended)` (Phase 7, KG-5). Every `RunHandle` method that can run a round first returns `Err(Ended)` when the Run is `CleanedUp`, and before returning runs cleanup when an end has been requested (KC-38), so that no end request is left pending across calls — except, in a device-paced class, an end the data thread requested while no call was running: the radios stop at once (KC-46b), and the rest of cleanup and the Manifest follow at the next call, until which `state()` reports `Running` and `events()` already shows what ended the Run (Phase 7, KG-3; Review J, P2-6). *Checked in Phase 7 by `kg_05_a_device_paced_spec_run_runs_to_its_horizon` and `kg_05_a_device_paced_spec_run_ends_at_its_scheduled_stop` (KG-5), and by `kg_03_a_fatal_event_stops_the_providers_without_a_client_call` (KG-3).*
- **KC-29a** `RunHandle::events(from)` returns the events delivered so far (KC-31) from index `from` on, in delivery order, and an empty list when `from` is at or past their number. Index `i` names the same event for the whole Run and in the Manifest's `events.delivered`. It runs no round and is available in every state, after cleanup included, until `finish` consumes the handle (Phase 6, KF-1). *Checked by `kf_01_the_delivered_events_are_readable_during_the_run`.*
- **KC-29b** `RunHandle::wait_for(kinds, from, horizon)` returns the index of the first delivered event at or after index `from` whose kind is one of `kinds`. It first checks the Lease and returns `Err(Ended)` for a Run that is `CleanedUp`, as every call that can run a round does (KC-29, KC-36). If a match is already delivered it then returns at once without running a round. Otherwise it places `horizon` on the primary root as KC-29 does (else `NotOnPrimaryRoot`), schedules a no-op callback at it as `advance_to` does, and runs KC-20's loop one round at a time, checking the events each round delivered: it returns `Ok(Some(i))` after the round that delivered the first match, standing at that round's instant, and cancels the no-op at `horizon`, so no later round runs at an instant only this wait chose; `Ok(None)` after a round has run at an instant at or after `horizon` with no match, standing at `horizon`; and `Err(Ended)` if the Run ended first, whether or not the round that ended it delivered a match (the events stay readable, KC-29a). With `kinds` empty it is `advance_to(horizon)` (Phase 6, KF-2; the prologue and the cancellation after Review H, P0-5 and P1-3; Vision §15's `run.wait_for(event)`). *Checked by `kf_02_wait_for_returns_at_the_round_that_delivered`, `kf_02_wait_for_stands_at_its_horizon`, `kf_02_wait_for_finds_an_event_already_delivered`, `kf_02_wait_for_returns_the_first_match_and_withdraws_its_horizon`, `kf_02_wait_for_with_no_kinds_is_advance_to` and `kf_02_wait_for_answers_ended_first`.*

### Events and the Policy

- **KC-30** Every call into a Module is made under `std::panic::catch_unwind`, including the calls Kernel functions make on the coordinator's behalf — `instance()` while the coordinator assembles the Run, and `coerce`, `instance()` and `descriptor()` inside `plan::validate`, `plan::plan` and `plan::collect_prepare`, which the coordinator therefore calls under `catch_unwind` as a whole (a panic there fails that stage with the reason `"KC-30: a Module panicked during <stage>"`); a panic is `ModuleError { kind: Internal, message: "a Module panicked", detail: null }`. The coordinator always knows which instance a call went to: outside a round it made the call itself, and inside a round each stepped instance is wrapped in an adapter that records, in stepping order, every instance whose `step` returned an error or panicked, because `step_until_quiescent` returns the error without saying whose it was. The instance's **source root** is its `instance().id` for a Provider, `sink/<output id>` for a Sink and `island_<n>` of its first Island fragment for an Executor. Then:
  - an error of kind `DeviceLost` from any call after `prepare` makes the coordinator emit `DEVICE_LOST` (severity `fatal`, source the instance's source root, time the current instant in the primary root, payload `{ "message": <the error's message> }`) through `emit_control`; from `step` nothing else happens here, and KC-31 applies the Policy (its default is `abort`); from `arm` or `start` the stage still fails as below; from `stop` it is also that step's `CleanupFailure`;
  - any other error, and a panic, from `step` requests the end `Failed { run }` in `abort` mode with the reason `"KC-30: <first fragment id>: <message>"`; from `prepare` it is `collect_prepare`'s (KC-12); from `arm` or `start` it is `Failed { arm }` with the same form of reason; from `stop` it is that step's `CleanupFailure`; a panic in `cleanup` (which returns nothing) is that step's `CleanupFailure` with the reason `"a Module panicked"`;
  - an error from `step_until_quiescent` whose `detail.event_kind` is `STEP_LIVELOCK` and that no adapter recorded is not a Module failure: the event is already emitted and KC-31 applies the Policy.

  Each of a round's failures is handled by these rules on its own terms, whatever failed before it in the round: a `DeviceLost` ahead of it in the list does not keep a later error or panic from requesting `Failed { run }` (KC-32 keeps the first end). One function applies them, to a round's failures and to a data-thread pass's (KC-46). *Checked by `kc_30_a_panic_after_a_device_lost_in_one_round_fails_the_run` and, in `device_paced.rs`, `kc_30_a_panic_after_a_device_lost_in_one_pass_fails_the_run`.*

  A `DeviceLost` found by a Provider on its own thread, off any call, is reported as MA-9a says; the coordinator then treats the event like KC-30's own (Phase 7, KG-10). *This carries MA-9's forward half. The Provider-thread report is checked by `kg_10_a_device_lost_from_a_provider_thread_aborts_the_run`.*
- **KC-31** After each round the coordinator drains the collector, appends the events to `delivered`, and for each event takes `policy.reaction_for_event(kind, severity)`: `continue` does nothing; `mark_artifact` records `(kind, time)`; `stop` requests an end `Stopped { policy { event: kind } }` in `orderly` mode; `abort` requests an end `Stopped { policy { event: kind } }` in `abort` mode. It also reads `collector.escalation()` (RS-36), which names only kinds whose body was dropped, and requests the same: a stopping event whose body was dropped ends the Run at the drain after the drop, its kind the cause unless a delivered stopping event already requested an end (Phase 7, design-notes §21). The Kernel's own events use source `kernel` (KA-14), except `DEVICE_LOST`, whose source is the lost instance's source root (KC-30). The drain, the reactions and the append to `delivered` happen under one lock, so that events drained by the data thread (KC-46) and by the control thread are appended in drain order, index `i` names one event for the whole Run (KC-29a), and the Policy reacts in the order of `delivered`: when two events of one Run each request an end, the Termination's cause is the first of them in `delivered` (KC-32) (Phase 7, KG-2). *Checked by `rs_36_a_delivered_body_raises_no_escalation_flag`, the deterministic guard of the order (a queued body raises no flag, so no flag can name a cause ahead of `delivered`); `kg_02_the_first_delivered_stopping_event_is_the_termination_cause`, the end-to-end check, whose race depends on the machine's timing (Review U, TG-U2); `rs_36_abort_survives_a_drop`; and `rs_36_a_dropped_stopping_body_ends_the_run`, that the escalation ends a Run (Review U, TG-U1).*
- **KC-32** The first end request fixes the Termination. A later request is recorded in `termination.also`, as the `Termination` it requested, only when it says something the first end does not. Three kinds do: (a) an escalation of the mode — an `abort` request while an `orderly` one is pending, which also escalates cleanup (RS-10); (b) a failure, `Failed`; (c) a Policy reaction, `Stopped { policy }` — such as a `DEVICE_LOST` the Policy maps to `stop` when a panic in the same round has already ended the Run `Failed` (review of #65). Any other later request — a completion, or a client's, a disconnect's or an expiry's stop that does not escalate — is dropped. A request equal to the Termination or to an entry already recorded is not recorded again. *Checked by `kc_32_an_abort_during_orderly_escalates_and_is_recorded`, `kc_32_a_later_failure_is_recorded_in_also`, `kc_32_a_policy_reaction_a_failure_overrode_is_recorded_in_also`, `kc_32_a_repeated_reaction_is_recorded_once`, `kc_32_a_reaction_equal_to_the_first_end_is_not_recorded`, `kc_32_a_module_abort_during_an_orderly_cleanup_escalates_and_is_recorded`, `kc_32_a_disconnect_after_a_policy_stop_is_dropped` and `kc_30_a_next_wakeup_panic_in_the_cleanup_drain_is_recorded`.*

### Ending

- **KC-33** A Run ends — the Termination, the mode — when: a scheduled `Stop {}` fires (`Completed {}`, orderly); `next_wakeup` returns `None` in a Spec Run of the Simulation class (`Completed {}`, orderly); in a device-paced class `None` ends no Run — the loop returns to its caller, and the Run ends by its schedule's `Stop {}`, `finish`, the Policy, the Lease or a failure (Phase 7, KG-5); the client calls `finish` while it runs, a Session submits `Stop {}` or `Release` (`Stopped { client }`, orderly); an Attached Lease's client disconnects (`Stopped { client_disconnect }`, orderly); a Detached Lease expires (`Stopped { lease_expiry }`, orderly); the Policy reacts (KC-31); a Module aborts (KC-24); a stage fails (`Failed { stage }`, abort, KC-7, KC-30). *The device-paced `None` clause is defensive: every device-paced loop has a callback scheduled at its horizon, so `next_wakeup` returns `None` there only after RS-6 step 1 has cancelled it, when an end is already requested and fixes the Termination (KC-32). Checked by `kg_05_a_device_paced_spec_run_runs_to_its_horizon` and `kg_05_a_device_paced_spec_run_ends_at_its_scheduled_stop`.*
- **KC-34** `finish(self)` runs cleanup if the Run has not ended, with `Stopped { client }` in orderly mode, and returns the sealed Manifest.
- **KC-35** `connect` calls `Lease::validate()` on the Lease it is given, and a Spec Run holds `Lease::attached()`. *This carries D20.*
- **KC-36** At every `RunHandle` call and after every round, the coordinator checks `lease.expired(host_clock)` and requests the end of KC-33. `disconnect()` calls `Lease::on_disconnect`: for an Attached Lease it requests `Stopped { client_disconnect }` and runs cleanup; for a Detached one it starts the TTL. `check_lease()` performs the check alone. In a device-paced class the Lease's expiry deadline, in the host clock's monotonic milliseconds as `Lease::expires_at_host` keeps it, is kept in `Shared` as well as in the handle, and is updated whenever the Lease changes (`disconnect` starting a Detached Lease's TTL, `Renew`, `Adopt`, `Release`); the data thread compares it with `host_clock` after every pass and, when it has passed, requests the end `Stopped { lease_expiry }` in orderly mode, on which it acts as KC-46b says (Phase 7, KG-2; Review J, P1-3). *This carries RS-23's forward half. The data thread's check is checked by `kg_02_a_detached_lease_expires_while_no_call_runs`.*
- **KC-37** `submit` refuses `RunChild`, logged `Rejected` with the violation `ezsdr.run_child` and the reason "RS-25a: a child Run is created with `run_child`, which carries its documents and Modules" (Phase 6, KF-3). *Checked by `kf_03_run_child_is_a_session_verb`.*
- **KC-37a** `RunHandle::run_child(spec_doc, profile_doc, assembly, drive)` on a Session, at the current instant `now`:
  1. refuses with `Ended` after cleanup and `NotSession` on a Spec Run; the entry's action is `RunChild { spec_hash, binding_hash }`, each the hash RS-45 names, and it refuses with `Malformed` when a hash cannot be computed or `SessionLog::check_entry(now, &action)` fails (RS-15: no sequence number);
  2. admits the child, the first failing check giving the entry `Rejected` with one violation `ezsdr.run_child`: the Run is `Running` (reason "RS-18: …"); the Run's class is Simulation (reason `"KC-37a: a device-paced Session runs no child Run: its devices are the parent's, and a child would open them again (Phase 7, KG-12)"`); `spec_doc` parses as an ExperimentSpec and `profile_doc` as a BindingProfile (the parse error); each binding `assembly.providers` names has a binding description — `(module, selector, profile)`, SB-3 — equal to one of the parent profile's bindings ("RS-25a: <name> binds an instance its parent does not"); the child's `authority` binding has the parent's `authority` binding's description ("RS-25a: the Authority …"); and every `environment` section of the parent's profile that an admission check registered for the parent reads at the runtime stage (SB-29, SB-30) — the checks that judge what the Session may do, `radio.rf_envelope` among them — is in the child's profile with an equal value ("RS-25a: section <ns> …"); a section only validate- or prepare-stage checks read (`sim.channel`, `sim.seed`, `sim.faults`) may differ;
  3. a rejected entry is appended (RS-15) and the call returns it with no Manifest;
  4. an admitted entry is appended as `Admitted { coercions: [], warnings: [], dispatched: [] }`; the child is started as `start_spec_run` starts a Run, with kind `Spec`, the parent's id as its parent, a copy of the parent's current Lease as its own (RS-25), and the parent's host clock and admission checks in place of the Assembly's, so the Lease copy is read on the clock it was measured on and the sections the child had to copy are judged by the checks that read them (Review H, P1-2); `drive(&mut child)` runs it; the child is finished (`finish`, KC-34), which cleans it up if `drive` left it running; the parent records `{ seq, run, manifest }` — the entry's sequence number, the child's id and its Manifest's hash (null if unsealed) — in `run.children` (KC-45); the parent then checks its Lease (KC-36), so a Detached Lease that expired while the child ran ends the child first and the parent after (RS-25);
  5. the call returns the entry and the child's Manifest.

  The parent's time does not advance while the child runs: the child has its own Authority and clocks, from its own Assembly. A child is never live when `run_child` returns, so RS-6 has no step for children (Phase 6, KF-3). *Ceiling: a child of a device-paced Session needs the parent's devices handed over, and its streams quiesced, for the child's time; a later phase that needs §54's sweep on hardware designs it (Phase 7, KG-12).* *Checked by `kg_12_run_child_is_refused_in_a_device_paced_session`, `kf_03_a_child_run_is_admitted_logged_run_and_recorded`, `kf_03_rs_25a_refusals`, `kf_03_a_child_inherits_the_lease`, `kf_03_run_child_is_a_session_verb`, `kf_03_children_are_recorded_in_order`, `kf_03_a_lease_that_expires_during_a_child_ends_the_child_first` and `kf_03_the_parents_checks_judge_the_child`.*
- **KC-38** Ending moves the Run to `Stopping { mode }` and calls `run_cleanup(ops, reverse fragment order, mode, escalate)`, where `ops` is KC-39's and `escalate` returns `Some(Abort)` once KC-32 has escalated.
- **KC-39** The coordinator's `CleanupOps::perform` does what KA-12's table says for each step, acting on an instance at most once per step, only on an instance at least one of whose fragments reached `prepare` (MA-7), under the Module's slot lock and `catch_unwind`. The mode a Module's `stop` receives is the pending end request's mode **when the call is made**, not the mode `run_cleanup` passed to the step: `run_cleanup` reads its escalation only between steps, and an abort raised during step 3's drain must reach the Executors and Sinks that step 3 stops after the drain (RS-10). A Module's `Err` from `stop` is returned as the step's error, which `run_cleanup` records as a `CleanupFailure` naming the fragment.
- **KC-40** The termination's `also` lists every Termination KC-32 recorded, in request order. *This carries RS-10's forward half.*
- **KC-41** A Sink whose `stop` fails contributes no `ArtifactRef` and its failure is a `CleanupFailure` naming its fragment (MA-26). *This carries MA-26's forward half.*
- **KC-42** At step 5, after the event path's last drain (RS-6), each artifact a Sink returned receives every recorded mark, those that drain delivered included, whose instant lies within the artifact's span `[first, end)` — the first `ContinuityMap`'s `first` converted to the primary root and floored, the last one's `end` converted and rounded up, the mark's own instant converted and floored — and every mark when the artifact has no map or a conversion fails (RS-30's "open at that moment", conservatively).
- **KC-43** Every transition after `Created` is recorded with `at = now` in the primary root, and every transition with the host clock's UTC time (RS-5); `Created` is recorded by `RunStateMachine::new` with `at: None`, because no Authority instant exists before the Run is assembled. The Manifest's `run.transitions` is `RunStateMachine::transitions()` verbatim. *This carries RS-5's forward half.*
- **KC-44** The Manifest is assembled after `run_cleanup` returns, from state the coordinator holds, locking a Module slot only with `try_lock` — where a poisoned lock (a Module panicked while holding it, KC-30) counts as acquired and only a lock still held counts as held — a slot a wedged step still holds yields a `CleanupFailure { step: release_and_write_manifest, fragment: that instance's first fragment, reason: "KC-44: the instance is still held by an abandoned cleanup step", timed_out: false }` instead of a wait, and that instance contributes no fidelity and no sections — and sealed with `Manifest::seal()`. A Provider section `Manifest::write_section` refuses (KA-13) is not written and adds a step-7 `CleanupFailure` naming the fragment and quoting the error. If `seal()` fails — possible only for a non-finite number or a non-ASCII key that reached the Manifest through a path no check guards — a step-7 `CleanupFailure` with `fragment: None` and the `HashError`'s text is appended to `termination.cleanup_failures` and the Manifest is returned **unsealed** (`hash: None`): RS-11 requires a Manifest for every Run, and an unsealed one says truthfully that it could not be sealed. Its fields are KC-45's. *This carries RS-11a.*
- **KC-45** Manifest fields: `version` 1; `run` = `{ id, kind, parent: the parent's id for a child Run (KC-37a) and `None` otherwise (Phase 6, KF-3), execution_class: the plan's class (KC-2), absent when no plan was built, fidelity: Fidelity::weakest(the fidelity of every Provider instance KC-44 could lock), transitions, deterministic: a plan was built and its class is Simulation, children: one `{ seq: u32, run: RunId, manifest: ContentHash | null }` per child Run in creation order (KC-37a; Phase 6, KF-3) }`; `policy` (absent if it never compiled); `spec` = `SpecSection::of(&spec)`; `binding` = `BindingSection::of(&profile)`, each the parsed document's hash and body (RS-45); `plan` when built; `prepare` = `{ reports }`, one per fragment in plan order (SB-41; spec 20, KH-1; issue #43); `admission`; `modules` = one `ModuleEntry` per distinct `(module, profile)` of `profile.bindings` and per Link `ModuleRef` of `placements.links`, with `impl_hash` from the registered descriptor; `vocabularies` = each Vocabulary a bound Module declares, at its registered version; `components` = each graph component's `impl.hash`; `inputs` (KC-9, KC-28); `clocks` = `{ domains: clocks.domains(), relations, sample_clocks: clocks.sample_clock_records() }`, where `relations` is, in a device-paced class, what `authority.relations()` returns, read under `catch_unwind` when the Manifest is assembled, in the order returned, and none in the Simulation class (KA-16), with the two failures stated at the end of this rule (Phase 7, KG-11); `links` = one `{ link: DataLinkId, drops: u64 | null }` per created link in `plan.links` order, empty when none was created, `drops` the count step 6 read (KA-18) and null when it could not be read — the endpoints are the link's `plan.links` entry; `events` = `{ counters: collector.counters(), delivered }`; `lease` = `{ mode, released }`, with no token and no host monotonic deadline, which are runtime state (R5, RS-22); `action_log` (Session); `termination` = `{ reason, at, host_utc_nanos, cleanup_failures, also }`, `also` the Terminations KC-32 recorded; `artifacts` (KC-41, KC-42); `sections` = every Provider's `instance().sections` under its Module id (KA-13), and no section of the Kernel's own. The Kernel events' payloads (`DEVICE_LOST`: `{ "message": string }`; `EVENTS_DROPPED`: `{ "kind": EventKind, "count": int }`; `STEP_LIVELOCK`: `{ "rounds": int }`) have the shapes these rules state and no committed schema. A relation whose `source` is not the primary root is not recorded and adds a step-7 `CleanupFailure` with `fragment: None` and the reason `"TM-18: a relation whose source is not the primary root"`; a device-paced Run whose recorded relations include none with `target` `utc` — the Authority returned none, or panicked — adds one with the reason `"TM-18: the Authority published no relation of its root to utc"` (Phase 7, KG-11). *Checked by `kg_11_a_device_paced_run_records_its_root_s_relations`, `kg_11_a_simulation_run_records_none` and `kg_11_a_device_paced_run_without_a_utc_relation_says_so`.*

### The device-paced classes (Phase 7)

The rules of this subsection apply in HardwareInLoop and Hardware only (KC-2). Their tests, and every `kg_*` test the rules above name except `kg_07_a_burst_at_t0_plus_n_samples_is_exact`, are in `crates/ezsdr-kernel/tests/device_paced.rs`, on the doubles `WallAuthority` and `ThreadedProvider` of spec 19 §0 ([`plan/phase7/19-amendments.md`](../plan/phase7/19-amendments.md)), which live in `crates/ezsdr-kernel/tests/support/paced.rs`. The code is the private module `coordinator/paced.rs`, the only coordinator code besides `run_cleanup`'s step threads that spawns a thread or reads the host clock (PO-11 as the Phase 7 overview's GZ-4 amends it).

- **KC-46** In a device-paced class, from the moment the Run enters `Running` (KC-18, before the start dispatch) until RS-6 step 3 stops it (KA-12 as amended by KG-3), a **data thread** of the coordinator repeats a **pass**: it takes `t`, the current instant on the primary root; steps every Executor and every Sink once, `step(t)`, in MA-30's order (role rank, then instance id), skipping an instance whose step-4 `cleanup` has been called and an instance whose `step` has failed earlier in the Run, each call contained and attributed as KC-30 says for a round; handles the pass's failures as KC-30 handles a round's, with the same function, so that an instance's failure is reported once; drains the collector and applies the reactions (KC-31); checks the Lease's expiry deadline (KC-36); then acts on an end it requested in the pass (KC-46b); and wakes the control loop (KC-46a) when the drain delivered at least one event. When at least one instance reported `progressed`, the next pass starts at once; otherwise the thread parks for 200 µs (`DATA_IDLE_PARK`, private). The data thread calls no `step_until_quiescent`: the device's clock moves on between passes, so repetition — not quiescence at one instant — is what drains a stream, and a Sink that finds a new block on every pass under a high block rate would reach MA-30's cap, which guards against a livelock at one instant. A pass's latency, and so how soon the Policy reacts to an event, is bounded by its slowest step (a capture Sink hashing a finished file, for example). In a device-paced class the control thread's rounds (KC-20, KC-21, KC-18's round) step no instance — no Provider is stepped (KC-2a) and the data thread steps the rest — so no instance is ever stepped by two threads; they still run the agenda, drain the collector, apply reactions and check the Lease (Phase 7, KG-2; after Review J, P1-3, P1-10, P2-5, P2-10). *Checked by `kg_02_the_data_thread_drains_a_sink_while_no_call_runs`, `kg_02_every_step_before_finish_runs_on_the_data_thread`, `kg_02_the_simulation_class_steps_on_the_callers_thread`, `kg_02_a_sink_that_always_progresses_does_not_livelock` and `kg_02_a_failed_sink_is_not_stepped_again`; that a Simulation Run never starts the thread, by `kg_02_the_simulation_class_starts_no_thread` (GZ-4).*
- **KC-46a** The data thread **wakes** the control loop by scheduling a no-op callback on the Authority at the current instant — an instant a paced Authority accepts however far its clock has moved on (TM-16c as KG-9 amends it) — unless a wake it scheduled has not fired yet. Pending wakes are counted by generation: before calling `schedule` the data thread stores the next generation `g` as the pending one, and the callback clears the pending generation only if it is still `g`, so a callback that fires before `schedule` has even returned still clears its own wake, and a later wake is never mistaken for an earlier one. The pending wake's handle is kept in `Shared` on its own, not among the handles `advance_to` schedules, under a lock held across `schedule`, and RS-6 step 1 cancels it with every other scheduled callback (Review L, NONBLOCKING 1); a paced Authority's `schedule` may therefore block only briefly (it must not wait on the control thread's `next_wakeup` for long, which RS-6 step 1 on the control thread would then wait for in turn), and a wake scheduled after step 1 has passed stays scheduled, a no-op like the one KC-46b's unconditional wake leaves (Review M, N-2). A control loop waiting in `next_wakeup` then returns and runs a round, whose end check (KC-29) and `wait_for` match check (KC-29b) see what the data thread requested and delivered. A wake the Authority refuses (a panic or an error, which a conforming paced Authority does not return) clears its generation and is not retried; the control loop sees the state at its next wakeup (Phase 7, KG-2; after Review J, P0-1, P1-1, P2-3, and Review K, N-P1-1). *Checked by `kg_02_wait_for_returns_when_the_data_thread_delivers`, `kg_02_every_delivery_wakes_a_waiting_call` and `kg_02_a_wake_that_fires_before_schedule_returns_still_clears` (an Authority whose `schedule` returns after the callback ran).*
- **KC-46b** After a pass in which the data thread itself requested an end (a step failure, KC-30; a reaction, KC-31; an escalation, RS-36; an expired Lease, KC-36), it performs, at most once per Run and on its own thread, RS-6 step 1 and then step 2 for every fragment in the reverse order, through the coordinator's cleanup operations (KC-39) with the mode then pending, and then wakes the control loop unconditionally (belt and braces: step 1 cancels the pending horizon, and a cancel already wakes a waiting `next_wakeup`, MA-29 — Review L, NONBLOCKING 3). Each step it performs is recorded as done (KC-39), so when the control thread later runs cleanup (KC-38) `run_cleanup` performs every step as usual and acts on no instance twice; RS-6 step 3 joins the data thread before any Executor or Sink stops (KA-12 as amended by KG-3), so a `stop` the data thread is still making finishes first. Under `abort` the data thread then stops stepping and ends; under `orderly` it goes on stepping Executors and Sinks, so that what the Providers deliver in step 2 reaches them. How soon this happens after the event is bounded by the pass's slowest step (KC-46). While the data thread is inside a Provider's `stop` — bounded for the UHD Provider by its joins, about 3 s at worst (spec 18 UR-16, UR-26), and by nothing the Kernel enforces for another Provider — it steps no Sink, drains no event and checks no Lease, so an orderly end the data thread requested (an expired Lease, a Policy `stop`) may lose receive samples, counted by the drop-class links, that an end requested by a client call would have delivered (the trade-off of doing without a separate thread, Review K, N-P2-4). The Run stays `Running` until the control thread's cleanup moves it to `Stopping`, whose transition records that instant; each Provider records in its own sections when it stopped (Phase 7, KG-3; after Review J, P1-1, P1-10, P2-9). *Checked by `kg_03_a_fatal_event_stops_the_providers_without_a_client_call`, `kg_03_a_step_done_early_is_not_repeated`, `kg_03_an_abort_stops_the_data_thread` and `kg_03_a_cancelled_horizon_wakes_the_control_loop`.*
- **KC-46c** In a device-paced class, dropping a `RunHandle` whose Run is not `CleanedUp` runs cleanup as `finish` does (KC-34), with `Stopped { client }` in orderly mode, and discards the Manifest; this joins the data thread. In the Simulation class dropping a handle does what it did before (nothing) (Phase 7, KG-3). *Checked by `kg_03_dropping_a_live_device_paced_handle_cleans_up`.*

## 10. Update classes

- **UC-1…UC-6** Spec 05 §5's.

## 11. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| K1 | How a Module keeps its handles | `Arc` handles in an owned `PrepareContext` (KA-1) | A lifetime on every role trait (`Box<dyn Provider + 'a>` everywhere, and a Plugin host cannot name one); handles passed again to every method (seven signature changes, and `step` would grow a context MA-20 does not describe) | none |
| K2 | Host bytes | Shared bytes owned by the block (KA-3) | A `map_host` on `DataLink` (the lifetime problem remains); `unsafe` in a pool (PO-2 forbids) | GPU and pinned domains keep the opaque handle and never call `new_host` |
| K3 | Per-fragment configuration | One map per fragment id (KA-4) | Direction or device prefixes in every key (unbounded key declarations, and still wrong for two devices) | none |
| K4 | Joint refusals | `coerce` always, `rejected` read (KA-5) | A PerformanceEnvelope admission check (it cannot see the Provider); a Kernel envelope type (OV-21) | A Provider whose `coerce` is expensive caches by request |
| K5 | The Kernel's one envelope number | `min_command_lead` on `ProviderInstance` (KA-7) | Reading `radio.timing.*` (OV-21); no number, so RS-19 stays undefined | A second number is added only for a Kernel rule that needs it |
| K6 | T0 | The end of `arm` plus a profile-declared lead (KA-8, KC-15) | The first Provider's actual start (differs per device); a Kernel constant (the right lead is the site's and the device's, not the Kernel's) | none |
| K7 | Which stream clock a `SpecTime` counts in | The target's stream, else a shared ratio, else refused (KA-8) | The primary root in nanoseconds (loses sample accuracy on a 30.72 Msps grid); the finest clock (a choice the Core does not make); resolving a `sink/<output>` target through the output's feed port (the Kernel cannot map a port to a stream node without reading the Radio Model, OV-21) | A later `SpecTime.stream` field would be additive. Until then a Spec cannot schedule a capture on a resource whose receive and transmit rates differ; a Session's `capture` with `at` in the receive SampleClock can |
| K8 | When scheduled Actions are dispatched | Timed ones at start, untimed ones at their instant (KC-17) | All at start (a `Stop` would stop at once); all at their instant (a timed command would arrive with no lead) | none |
| K9 | Threads | One logical thread; `run_cleanup`'s step threads only (RS-8a) | A thread per Module in Simulation (MA-30 forbids) | the threaded driver of RealtimeEmulation is a later phase's |
| K10 | Where a Module's lock lives | One `Mutex` per instance slot | One lock for the Run (a wedged step would block the Manifest, RS-11) | none |
| K11 | Reactive coerce | Not in Phase 2 (KA-6) | Coercing while the stepping loop holds every Module (a deadlock) | settled in Phase 5: not coerced (KE-4) |
| K12 | A Spec Run with no scheduled `Stop` and a streaming Provider | Runs until the caller's horizon or `finish` | Ending when every Sink is "complete" (no trait method says so) | A Sink completion signal would be a trait addition |

## 12. Tests

In `crates/ezsdr-kernel/tests/coordinator.rs` unless named. Each is described by its input and the assertion it makes; `20-implementation-plan.md` steps 3–8 give the fixtures.

| test | input | expected | rules |
|---|---|---|---|
| `ma_05a_a_module_keeps_its_handles_after_prepare` | a stepped double keeping `events` and `time` | an event emitted in `step` is in `manifest.events.delivered`; `now` read in `step` equals the step's `until` | MA-5a, KA-1 |
| `kc_01_a_parse_failure_is_no_run` | a Spec with `version: 2` | `Err(UnsupportedVersion)`; no handle | KC-1 |
| `kc_01_a_validate_failure_still_writes_a_manifest` | an unbound resource | handle `CleanedUp { Failed { validate } }`; `finish()` returns a sealed Manifest with no plan | KC-1, KC-7, RS-11 |
| `kc_01_a_validate_failure_records_its_reason` | the same | the termination is `Failed { validate, reason }` with a reason; `execution_class` is absent and `deterministic` false; every transition after `Created` has `at` set; every transition has a non-zero `host_utc_nanos` | KC-7, KC-43 |
| `kc_02_a_wall_paced_authority_is_refused` | a test Authority with `WallPaced` pacing | `Failed { plan }`, reason names KC-2 | KC-2 |
| `kc_04_two_objects_for_one_description_are_refused` | two resources with one binding description, two objects | `Failed { validate }` | KC-4 |
| `kc_04_one_object_serves_both_names` | the same, one object | validates; `instance()` read under both names | KC-4, SB-3 |
| `kc_04_an_object_under_no_resource_is_refused` | objects under `radio` and `nobody` | `Failed { validate }`, reason names `nobody` | KC-4 |
| `kc_09_an_input_must_be_supplied_and_match_its_hash` | a scheduled `TxBurst` whose waveform is absent from `Assembly.inputs`; present with one byte changed; present but with `size_bytes` one too large; with the uri `http://x` | `Failed { plan }` each time, reason beginning `"KC-9: "`; with the right bytes and a `mem:` uri the Run plans and `inputs` records the ref | KC-9 |
| `ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once` | one double, two resources | call log `prepare, prepare, arm, start, stop, cleanup` | KA-9, KC-13 |
| `kc_10_link_descriptor_must_equal_the_registered_one` | a Link whose `descriptor()` differs | `Failed { plan }` | KC-10, MA-27a |
| `kc_10_both_ends_are_attached` | a feed from a double's `rx` port to a test Sink | the Provider's `links` has `{component: radio, port: rx, StreamOut}`; the Sink's `{component: rec, port: in, StreamIn}`; a block published arrives | KC-10, KA-15 |
| `kc_11_an_island_gets_exactly_its_components` | two Islands, three components | each Executor's `components` is its own two or one | KC-11, MA-19a |
| `kc_12_a_prepare_failure_stops_the_loop_and_cleans_up_what_was_prepared` | the second of three fragments fails | the third is never prepared; the first two are cleaned up; `Failed { prepare }` | KC-12, MA-7, SB-42 |
| `kc_14_an_arm_failure_is_failed_arm` | a double failing in `arm` | `Failed { arm }`; the double was stopped and cleaned up | KC-14, MA-7 |
| `kc_13_arm_and_start_follow_instance_order_cleanup_reverses_it` | `b arm_after a`, `c` unrelated | arm `a, b, c`; cleanup `c, b, a` | KC-13, RS-8 |
| `kc_15_t0_is_arm_end_plus_the_lead` | `ezsdr.time: { class: simulation, start_lead_ns: 2_000_000 }` (MA-41 refuses a section without `class`) | `start_instant()` is 2 000 000 ticks after the arm instant | KC-15, KA-8 |
| `kc_18_a_start_failure_is_failed_arm` | a double failing in `start` | `Failed { arm }`; `Running` never appears in the transitions | KC-18 |
| `kc_16_a_spec_time_resolves_on_the_target_stream` | a double declaring `dev/rx` at ratio 50 and `dev/tx` at 20; a `TxBurst` at offset 10 on `radio/tx` | deadline `T0 + 200` in the primary root | KC-16, SB-16 |
| `kc_16_an_ambiguous_spec_time_is_refused` | a `Stop {}` on a resource with ratios 50 and 20 | `Failed { arm }` naming entry 0 | KC-16 |
| `kc_16_off_root_negative_and_overflowing_times_are_refused` | three Runs: a stream clock declared on a second root; an offset of −1; an offset of `i64::MAX` at ratio 2 | `Failed { arm }` each time, with the reasons `"…not on the primary root"`, `"…negative offset"` and `"…overflow"` | KC-16 |
| `kc_17_a_scheduled_stop_ends_the_run_at_its_instant` | `Stop {}` at offset 100 at ratio 10 | `Completed`, termination `at` = T0 + 1000 | KC-17, KC-33 |
| `kc_17_scheduled_updates_are_admitted_cumulatively` | `test.limits` with `{ max_grid: 30, gate: test.flag }` (the check judges `test.grid` only where `test.flag` is true); the Spec requires `test.grid = 20`, `test.flag = false`; schedule entry 0 at offset 10 sets `test.grid = 40`, entry 1 at offset 20 sets `test.flag = true` (both keys declared `cold` in this test's Vocabulary) | `Failed { arm }` naming entry 1: judged against `test.grid = 40`, not the `prepare`-time 20 | KC-17, SB-30 |
| `kc_19_a_reject_at_plan_burst_with_a_short_lead_is_refused` | `min_command_lead` 5 ms, a burst 1 ms after T0 with `start_lead` 0, its waveform's bytes in `Assembly.inputs` | `Failed { arm }`, reason names SC-27 | KC-19 |
| `kc_20_virtual_time_advances_only_through_next_wakeup` | a stepped double scheduling wakeups at 10, 20, 30 | stepped at exactly those instants | KC-20, KA-11 |
| `kc_21_an_action_is_seen_at_its_admission_instant` | a Session `SetParameter` at now = 500 | the double records the Action in a step at 500 | KC-21, KC-25 |
| `kc_22_a_same_instant_wakeup_loop_is_step_livelock` | a double rescheduling at its own instant forever | `Stopped { policy { STEP_LIVELOCK } }`, abort; the delivered `STEP_LIVELOCK` has source `kernel` and payload `{ "rounds": STEP_ROUND_CAP }` | KC-22 |
| `kc_22_a_downgraded_livelock_still_ends_the_run` | the same with `policies.failure: { ezsdr.STEP_LIVELOCK: continue }` | `Failed { run }`, abort, reason beginning `"KC-22"`; `run_until_end` returns | KC-22 |
| `kc_23_targets_are_rewritten_through_matched` | a schedule entry `UpdateParameter { target: radio/x, key: test.gain }` with `radio` (kind `test.line`) matched to `dev/0`, and a stream clock `dev/0/rx` declared so that the entry's `SpecTime` resolves | the double receives target `dev/0/x` | KC-23 |
| `kc_23_an_unknown_target_is_refused` | `SetParameter { nothing }` | `Rejected`, `ezsdr.target` | KC-23 |
| `kc_24_a_burst_needs_a_transmit_sample_clock` | a `TxBurst` whose target has no registered clock | `Rejected`, SC-23 | KC-24 |
| `kc_24_a_module_reject_at_plan_burst_is_refused` | a test Executor submitting `RejectAtPlan` | `Err(violations)` from `submit` | KC-24, SC-27 |
| `kc_24_a_module_stop_without_target_is_refused` | a test Executor submitting `Stop { target: None }` | `Err` with check `ezsdr.target` and reason `"KC-24: a Module ends a Run only with Abort"`; the Run continues | KC-24 |
| `kc_24_a_burst_to_a_non_provider_target_is_refused` | Session with a bound Sink `rec`; `start_repeat` with target `sink/rec` | `Rejected`; reason beginning `"SC-23: sink/rec is not a Provider stream"` | KC-24, SC-23 |
| `kc_24_a_burst_time_on_an_unrelated_root_is_refused` | Session; `start_repeat` with `at` on a second root | `Rejected`; reason beginning `"SC-23b"` | KC-24, SC-23b |
| `kc_24_a_module_abort_ends_the_run` | a test Executor submitting `Abort { cause: Abort { "test" } }` | `Stopped { abort { "test" } }` in `abort` mode | KC-24, KC-33 |
| `kc_24_a_module_action_during_cleanup_is_refused` | a test Executor submitting at tick 10, which falls inside the orderly drain because a Provider's last wakeup is at 10; the Run ended at 0 | `Err` with check `ezsdr.dispatch` | KC-24, RS-6 |
| `kc_25_an_admitted_update_changes_the_configuration` | `SetParameter test.gain 3` | `effective()["radio"]["test.gain"] == 3`; the double received it | KC-25, KC-27 |
| `rs_17_a_session_rate_change_is_coerced_by_its_provider` | `test.grid` 19.5 with grid 20 | entry `Admitted` with a coercion to 20.0; the Action's value is 20.0 | KA-6, KC-26 |
| `rs_17_a_session_change_beyond_a_joint_limit_is_refused` | KA-5's joint limit of 50; the double reports `test.count = 2` in its `effective` (a Session's implicit Spec requires nothing, so the configuration holds only what the Provider reports); `test.grid` declared `cold` in this test's Vocabulary; `SetParameter { test.grid: 40 }` | `Rejected`, `ezsdr.coercion` on `test.grid` | KA-6 |
| `rs_17_a_scheduled_rate_change_under_reject_is_refused` | the rate change of `rs_17_a_session_rate_change…` as a Spec schedule entry | `Failed { arm }`, reason containing `SB-46` | KA-6, SB-46 |
| `kc_26_a_provider_that_applies_nothing_is_refused` | Session; a double whose `coerce` leaves the proposed key out of `applied` without rejecting it | `Rejected`; check `ezsdr.coercion`, reason containing `"applied no value"` | KC-26, KA-6 |
| `kc_28_a_malformed_action_takes_no_sequence_number` | a value map with a non-ASCII key | `Err(Malformed)`; the next entry has `seq` 0 | KC-28, RS-15 |
| `kc_28_a_waveform_is_an_input_before_admission` | `start_repeat` with 800 bytes | `inputs` holds one ref with `size_bytes` 800 and `mem:` uri; the `TxBurst` carries it | KC-28, RS-44a |
| `kc_28_an_untimed_burst_is_admitted_at_now_plus_lead` | `start_repeat` with no `at`, lead 2 ms | the `TxBurst.at` is now + 2 ms on the TX grid; the entry's coercion records it | KC-28, RS-19, KA-7 |
| `kc_28_an_untimed_burst_is_not_admitted_before_its_clock_s_origin` | `start_repeat` with no `at`, lead 2 ms, the transmit clock's origin 5 ms after now | the `TxBurst.at` is the clock's tick 0; the coercion records 5 ms | KC-28, RS-19 |
| `kc_28_an_untimed_burst_waits_for_its_clock_s_origin_without_a_provider_lead` | as above, with no `min_command_lead` declared | the same: the origin applies without a lead | KC-28, RS-19 |
| `kc_28_stop_run_ends_the_session` | `SessionAction::Stop { target: None }` | the entry is `Admitted`; `Stopped { client }` | KC-28, KC-33 |
| `kc_29_advance_to_refuses_an_unrelated_time` | `advance_to` a time on another root | `Err(NotOnPrimaryRoot)` | KC-29 |
| `kc_35_connect_refuses_an_invalid_lease` | `connect` with `Lease { mode: Detached { ttl_ms: 0, .. }, .. }` | `Err(SpecError::Structural)` naming RS-21; no Run | KC-35, RS-21 |
| `kc_30_a_panicking_module_fails_the_run_not_the_process` | a double panicking in `step` | `Failed { run }`; the Manifest is sealed | KC-30, MA-9 |
| `kc_30_a_panic_in_coerce_fails_validate` | a double whose `coerce` panics | `Failed { validate }`, reason beginning `"KC-30: a Module panicked during validate"`; `cleanup_failures` empty (a slot poisoned by the panic is not "still held", KC-44) | KC-30, KC-44 |
| `kc_30_device_lost_is_the_kernel_event_and_aborts` | a double returning `DeviceLost` from `step` | `DEVICE_LOST` counted and delivered with source the instance root; `Stopped { policy { DEVICE_LOST } }`, abort | KC-30, MA-9 |
| `kc_30_a_panic_after_a_device_lost_in_one_round_fails_the_run` | the Policy maps `DEVICE_LOST` to `continue`; in one round the Provider returns `DeviceLost`, then a Sink panics | `Failed { run }`, reason beginning `"KC-30: rec: a Module panicked"`; `DEVICE_LOST` from the Provider delivered | KC-30 |
| `kc_31_mark_artifact_marks_only_artifacts_open_then` | `test.custom` at severity warning (mark) at 150 and a capture spanning 100–200, another 300–400 | only the first carries the mark | KC-31, KC-42, RS-30 |
| `kc_42_a_mark_delivered_during_an_orderly_cleanup_reaches_the_artifact`, `kc_42_a_mark_delivered_during_an_abort_cleanup_reaches_the_artifact` | `test.custom` (mark) emitted by the Provider's `stop`, so no round drains it; captures spanning 100–200 and 300–400; the Run ends orderly at 160, or aborts on a `DeviceLost` at 150 | the first capture carries the mark at the stop instant, the second none | KC-42, RS-6, RS-30 |
| `kc_32_an_abort_during_orderly_escalates_and_is_recorded` | a stop request, then an abort event during the drain | remaining steps in abort mode; `also` holds the abort cause | KC-32, KC-40, RS-10 |
| `kc_32_a_later_failure_is_recorded_in_also` | a Provider and a Sink that both panic in one round | `Failed { run }` with the Provider's reason; `also` holds the Sink's `Failed { run }` with its own | KC-32, KC-30 |
| `kc_32_a_policy_reaction_a_failure_overrode_is_recorded_in_also` | the Policy maps `DEVICE_LOST` to `stop`; in one round the Provider returns `DeviceLost`, then a Sink panics | `Failed { run }`; `also` holds `Stopped { policy { DEVICE_LOST } }` once | KC-32 |
| `kc_32_a_repeated_reaction_is_recorded_once` | a Run failed by a Sink's panic whose Provider emits a kind the Policy maps to `stop` in its step and again in its `stop` | both events delivered; `also` holds `Stopped { policy { test.custom } }` once | KC-32 |
| `kc_32_a_reaction_equal_to_the_first_end_is_not_recorded` | a Run whose Provider emits a kind the Policy maps to `stop` in its step and again in its `stop` | both events delivered; `Stopped { policy { test.custom } }`; `also` empty | KC-32 |
| `kc_32_a_disconnect_after_a_policy_stop_is_dropped` (device_paced) | a device-paced Session whose data thread ends it on `DEVICE_LOST` (`stop`), then `disconnect()` on its Attached Lease before the next call | `Stopped { policy { ezsdr.DEVICE_LOST } }`; `also` empty | KC-32 |
| `kc_32_a_module_abort_during_an_orderly_cleanup_escalates_and_is_recorded` | a Run finished by its client whose Executor submits `Abort` from its orderly `stop` | `Stopped { client }`; `also` holds `Stopped { abort { "late" } }` | KC-32, RS-10 |
| `kc_30_a_next_wakeup_panic_in_the_cleanup_drain_is_recorded` | a Run with a Sink whose Authority panics in `next_wakeup`, finished by its client | `Stopped { client }`; `also` holds `Failed { run, "KC-30: authority panicked during next_wakeup" }`; the Sink stops in `abort` mode | RS-6, KC-32 |
| `kc_36_detached_lease_expiry_ends_the_run` | Detached 5 000 ms, disconnect, host clock +5 000 | `Stopped { lease_expiry }` | KC-36, RS-23 |
| `kc_36_attached_disconnect_ends_the_run` | Attached, disconnect | `Stopped { client_disconnect }` | KC-36 |
| `kc_37_run_child_is_refused` | `RunChild` | `Rejected`, `ezsdr.run_child` | KC-37 |
| `kf_01_the_delivered_events_are_readable_during_the_run` | a Session whose Provider emits one event; `events(from)` before and after it, past the end, after the Run ended | the event in delivery order; `from` skips; empty; equal to the Manifest's | KC-29a |
| `kf_02_wait_for_returns_at_the_round_that_delivered` | an event at 5 000; a kind that never comes | `Some(i)` standing at 5 000; `None` standing at the horizon | KC-29b |
| `kf_02_wait_for_stands_at_its_horizon` | no event | `None`, `now()` = the horizon | KC-29b |
| `kf_02_wait_for_finds_an_event_already_delivered` | the event already delivered; then from the next index | `Some` with no round; `None` at the horizon | KC-29b |
| `kf_03_a_child_run_is_admitted_logged_run_and_recorded` | `run_child` of a Spec Run on the parent's binding | admitted `RunChild` entry with the documents' hashes; the child's `run.parent`; the parent's time unchanged; the parent's `run.children` | KC-37a, KC-45 |
| `kf_03_rs_25a_refusals` | another selector; another Authority; the checked section dropped or changed; a Spec that does not parse; an unchecked section changed | the first five `Rejected` with `ezsdr.run_child` and logged; the last admitted | KC-37a, RS-25a |
| `kf_03_a_child_inherits_the_lease` | a Detached Session | the child's `lease` records the parent's mode | KC-37a, RS-25 |
| `kf_03_run_child_is_a_session_verb` | `run_child` on a Spec Run; `submit(RunChild)`; `run_child` after the end | `NotSession`; KC-37's reason; `Ended` | KC-37, KC-37a |
| `kc_39_every_instance_is_stopped_before_it_is_cleaned_up` | one Provider, one Executor, one Sink | each log has `stop` before `cleanup`; the Sink's artifacts are in the Manifest | KC-39, KA-12, MA-7 |
| `kc_39_orderly_cleanup_drains_the_tail` | a double that, after `stop(Orderly)`, schedules two more blocks | both blocks reach the test Sink before its `stop` | KC-39, KA-12, MA-13 |
| `kc_39_abort_cleanup_does_not_drain` | a double with a pending wakeup at 10, ended in `abort` mode at 0 | no `step` of any instance follows the first `stop` (under `orderly` the drain would step it at 10) | KC-39, KA-12 |
| `kc_41_a_failing_sink_stop_is_a_cleanup_failure` | a test Sink whose `stop` errs | `cleanup_failures` names its fragment; no artifact | KC-41, MA-26 |
| `kc_44_a_wedged_step_does_not_prevent_the_manifest` | a double wedged in `stop` | step 2 times out; the Manifest is sealed with a step-7 cleanup failure for that slot | KC-44, RS-8a, RS-11 |
| `kc_45_the_documents_are_recorded_as_parsed` | a Spec Run | `spec.body` and `binding.body` equal the parsed documents re-serialised; each `hash` is its body's | KC-45, RS-38, RS-45 |
| `kc_45_a_provider_section_outside_its_namespace_is_a_cleanup_failure` | a double whose `sections` hold `other.ns` | the section is absent; a step-7 `CleanupFailure` beginning `"KC-44"` | KC-44, KA-13 |
| `kc_45_manifest_fields` | a Spec Run with one Provider declaring a non-`NONE` fidelity, one Sink, one link | each field of KC-45 present and as described; `links` has one record naming `plan.links[0]` with its drops; the Provider's section is under its Module id and the Kernel wrote none; `lease` is exactly `{ mode, released }` | KC-45, KA-13, KA-18 |
| `kc_45_sample_clocks_and_domains_are_recorded` | a double declaring and registering one clock | `clocks.sample_clocks` has it; `clocks.domains` has every registered domain; `relations` is empty | KC-45, KA-2, KA-16 |

Phase 7's Kernel tests (KG-1…KG-12) are in `crates/ezsdr-kernel/tests/device_paced.rs` and are named by the rules they check; their inputs and expectations are spec 19's tables ([`plan/phase7/19-amendments.md`](../plan/phase7/19-amendments.md)). `kc_02_a_wall_paced_authority_is_refused` now expects a reason beginning `"KC-2: RealtimeEmulation"`.

## 13. Vision issues found

1. **§11 and §52 say `prepare` reports "effective configuration" as one map**; per-fragment is what a check can judge (KA-4). §52's "effective configuration overlaid with the proposed value" should say "each fragment's".
2. **§13 lists the TimingEnvelope as a structure the Kernel checks against**; Phase 2 makes it Provider capabilities plus one Kernel number (Y6, KA-7). §13 and §22 should say which.
3. **§22's "`validate()` is to check it at plan time"** is performed at `arm`, the first point at which T0 exists (KC-19). "Plan time" in §22 should read "before start".
4. **§32 says the Simulation Engine "is to decide drop policies in virtual time"**. In Phase 2 a drop is decided by the link, deterministically, because every publish and receive happens in the one logical thread in MA-30's fixed order; nothing is left for the Engine to decide. The sentence should say so.
5. **§53's cleanup list** names TX and RX steps that a Kernel cannot tell apart; KA-12's mapping should be summarised there.
6. **§14 says every Run records a relation to UTC**; a Simulation Run does not (KA-16).

## 13a. Deferred

The threaded driver for RealtimeEmulation, HardwareInLoop and Hardware. Child Runs and replay. A Sink completion signal (K12). Transfer-cost measurement (never Kernel work, SB-40).

## Changes

| date | rules | change | record |
|---|---|---|---|
| 2026-10-07 | UC-1…UC-6 | §10 points to spec 05 §5, the update classes' one home | [spec 22](../plan/maintenance/22-timing-simplification.md) |
| 2026-10-08 | KC-28 | step 3's `earliest` is not before the origin of the target stream's running clock (RS-19) | owner decision, 2026-10-08 |
| 2026-10-09 | KC-30, KC-46 | every failure of a round or a pass is handled on its own terms by one function both drivers call, so a panic after a `DeviceLost` in the same round still requests `Failed { run }` | [note 25](../plan/maintenance/25-fault-and-mark-notes.md), #65 |
| 2026-10-09 | KC-42 | the marks are recorded after the event path's last drain, which moves from step 7 into step 6 (RS-6) | [note 25](../plan/maintenance/25-fault-and-mark-notes.md), #66 |
| 2026-10-09 | KC-37a, KC-45 | the `spec` and `binding` sections and a `RunChild` entry's hashes are taken over the parsed documents (RS-45) | [audit item 5](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-09 | KC-7, KC-8, KC-10, KC-12a, KC-20, KC-22, KC-31, KC-32, KC-37a, KC-40, KC-42, KC-44, KC-45, KC-46 | the failure's reason is `Failed { stage, reason }` and `ezsdr.failure` is deleted; KC-32 records in `also`, as Terminations, every later request that escalates, fails or is a Policy reaction, once each, so a reaction a same-round failure overrode is no longer dropped; `ezsdr.links` and `ezsdr.children` become the typed `links` and `run.children`, and the Kernel writes no section; `execution_class` is absent without a plan; the Lease is recorded as `{ mode, released }`; `STEP_LIVELOCK` has one payload, `{ "rounds": … }`; the Policy cause's field is `event`; RS-6's renumbered steps (4–7 for 5–8); the cleanup drain records a `next_wakeup` panic as `Failed { run }` and the dispatch freeze tries every handle (F25 (a), (b)) | [audit item 4](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-10 | KC-32 | rewritten principle-first: a later request is recorded only when it says something the first end does not — an escalation, a failure or a Policy reaction; no change of rule | owner decision 2026-10-10 |
