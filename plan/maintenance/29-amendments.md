# Maintenance spec 29 — one admission path (pre-freeze item 3)

| Field | Value |
|---|---|
| Status | **Draft amended text, not yet in `design/`.** The plan is [`29-unified-admission.md`](29-unified-admission.md) (owner-approved 2026-10-11). This file is the revised spec the plan's §8 requires before implementation: KC-24's rows with their `reason`/`key`/`requested` settings, KC-28's front matter, and the per-rule amendments. It is reconciled with the code at the end of each stage and moved into `design/` by the acceptance step. |
| Basis | HEAD `a788875`, `main`, clean tree; [24-prefreeze-audit.md](24-prefreeze-audit.md) item 3, [28-forward-audit.md](28-forward-audit.md) decision 3, [27-role-typed-identities.md](27-role-typed-identities.md) judgment 4. |
| Stages | 3a declarations/targets/Spec checks, 3b class generation and runtime admission, 3c event source and ASCII, 3d memory domains. |

Notes for the reader: rule text is written in the accepted specs' voice so it can be moved into `design/` unchanged; Japanese notes mark what is still open until a stage lands. The mutations and tests named here are filled in per stage.

## 1. KC-24 — the one admission order and refusal table

Replacement text for design/06 (KC-24), replacing the current rule body. The parts not quoted here (the list of origins, the Abort clause, the front-matter pointer) are kept from the accepted text.

> **KC-24** Every Kernel Action passes one admission function before dispatch, whatever its origin (RS-16). This is the Kernel's **only** admission order and refusal table, and the function follows it step by step. A Session entry's front matter (KC-28 step 1: `Ended`/`NotSession`; `SessionLog::check_entry`; `session_earliest`; `session::compile`; the ControlOps) and the schedule's own resolution sit outside this order: an Action they refuse never reaches it, and an Action they pass arrives here as either a finished `Action` or a class-pending update request (KC-28 step 3, KC-17). The order is:
>
> 1. dispatch is not frozen (RS-6 step 1);
> 2. for a Session or Module origin, the Run is `Running` (`RunStateMachine::check_running`, logged as RS-18 says);
> 3. the target resolves in its own role (KC-23), as it is stated; an Action with no target (`Stop {}`, `Abort`) is not resolved; a Module's `Stop { target: None }` is refused here;
> 4. the payload's map keys are ASCII (SB-9a): an `UpdateParameter`'s value, a `TxBurst`'s `metadata`, a `Command`'s `params`;
> 5. the parameter's update class is declared and, for a Module's finished Action, the class the Action states equals it: `declared_class(spec, target, key)` gives a `{resource}`/`{output}` target its key's `KeyDecl.update_class` and a `{component}` target its `ParamDecl.update_class`, with **no fallback** from a component to a Vocabulary declaration (SB-2, item 3). A class-pending update request takes its class here and becomes a finished Action;
> 6. an `UpdateParameter` whose owner is a `KeyDecl` has a requested value of that declaration's kind (SB-2, SB-6). A component parameter has an opaque schema and no kind check;
> 7. for a `TxBurst`: a `RejectAtPlan` burst from a Module origin is refused (KC-19); the target must be a Provider fragment and the Assembly's clocks must hold a registered SampleClock record whose `stream` equals the resolved node and whose `ended_at` is absent, else refused (`ezsdr.target`, SC-23); `stream::admit_burst_target` converts `at` onto that clock (SC-23a, SC-23b) and sets `requested_at` when it advanced; and the waveform's hash must name bytes in the Run's input store whose length is its `size_bytes`, else refused (`ezsdr.input`, RS-44a; Phase 5, KE-2);
> 8. for an `UpdateParameter` to a Provider fragment from the Session or schedule origin, the value is a scalar (SB-5): a `List` or `Map` is refused;
> 9. the `matched` Provider the coerce below would use exists (KC-23);
> 10. for an `UpdateParameter` to a Provider fragment from the Session or schedule origin, KA-6's coerce (KC-26) runs: an error, a rejected constraint, or an applied value missing for the key is refused;
> 11. the value the coerce applied fits the declaration of step 6 as well, or is refused; a refused update is not recorded in the configuration (KC-25);
> 12. `Admitter::admit` runs the registered admission checks of SB-30's third point with the per-fragment configuration (KC-27) and `proposed = { fragment: { key: value } }` for an `UpdateParameter` — the value after step 10's coercion — and empty otherwise;
> 13. the coercion policy applies (SB-45, SB-46).
>
> At dispatch, not before, an `ActionId` comes from the Run's counter, which starts at 1, so that an entry some of whose Actions are refused consumes no id; the id travels with the Action into its queue and reaches the Module as `Dispatched.id` (MA-14). The refusals of steps 1–11 are these `Violation`s. **A refusal that would transcribe the payload sets `requested` only from step 4 on**: step 4's own refusal carries `requested: None`, because a Map key that is not ASCII must not enter a rejection record that the log and the sealed Manifest will hold.

| step | when | `check` | `key` | `requested` | `reason` |
|---|---|---|---|---|---|
| 1 | dispatch is frozen | `ezsdr.dispatch` | — | — | `"RS-6: dispatch is frozen"` |
| 2 | not `Running` | `ezsdr.run_state` | — | — | `"RS-18: the Run is not Running"` |
| 3 | the target does not resolve in its role | `ezsdr.target` | — | — | `"KC-23: <target> is not declared by this Spec"` |
| 3 | a Module's `Stop { target: None }` | `ezsdr.target` | — | — | `"KC-24: a Module ends a Run only with Abort"` |
| 4 | a payload map key is not ASCII | `ezsdr.value` | the key | **None** | the `KeyShape` message (SB-9a) |
| 5 | no update class is declared, or a Module's stated class differs | `ezsdr.update_class` | the key | the value | `"RS-52: <key> declares no update class"` / `"RS-52: update states <class> for <key>, which declares <declared>"` |
| 6 | the requested value's kind is not the `KeyDecl`'s | `ezsdr.value` | the key | the value | the kind refusal (SB-2, SB-6) |
| 7 | a Module's `RejectAtPlan` burst | `ezsdr.late_policy` | — | — | `"KC-19: SC-27: RejectAtPlan needs a statically known target"` |
| 7 | a burst whose target is not a Provider fragment | `ezsdr.target` | — | — | `"SC-23: <target> is not a Provider stream"` |
| 7 | no running SampleClock for the target | `ezsdr.target` | — | — | `"SC-23: <target> has no running transmit SampleClock"` |
| 7 | `admit_burst_target` fails | `ezsdr.target` | — | — | `"SC-23b: <the StreamError>"` |
| 7 | the waveform is not in the input store (Phase 5, KE-2) | `ezsdr.input` | — | — | `"RS-44a: <hash> is not an input of this Run"` |
| 7 | its stored length is not `size_bytes` (Phase 5, KE-2) | `ezsdr.input` | — | — | `"RS-44a: input <hash> holds <n> bytes, and the burst declares <m>"` |
| 8 | a Session or schedule Provider update is not a scalar | `ezsdr.value` | the key | the value | `"SB-5: <key> is a Provider parameter, whose value is a scalar"` |
| 9 | no matched Provider for the fragment | `ezsdr.target` | — | — | `"KC-23: no matched Provider for <fragment>"` |
| 10 | `coerce` errs, rejects, or applies nothing | `ezsdr.coercion` | the key (or the rejected key) | the requested value | `"RS-17: <fragment>: <the error, the rejection's reason, or 'its Provider applied no value for <key>'>"` |
| 11 | the coerced value's kind is not the declaration's | `ezsdr.value` | the key | the requested value | `"SB-2: the coercion of <key> applied a value whose kind is not the declaration's"` |

Steps 12–13 are the `Admitter`'s own violations, unchanged.

**Notes (Japanese; removed when this text is moved into `design/`).**
- KC-24 が「唯一」なのは Kernel Action admission の範囲である（§2 の前段・ControlOp 表）。KC-28 step 1 と ControlOp の拒否はここに含めない。
- `Admitter` から runtime の class 検査（現在の step 3）と `declared_classes` 欄を削除し、順序は KC-24 の参照に置き換える。validate/prepare の Admitter 用途（step 1 の登録チェックと step 2 の coerce policy）は維持する。
- step 5・6 は Module の Action と class 未確定要求の両方に効く。class 未確定要求では step 5 の比較がなく、宣言から class を導いて Action を完成させる（plan §5）。
- step 10–11: coerce 後の値が不正なら拒否し、configuration へ記録しない（KC-25 は dispatch で記録する）。
- ASCII 違反の拒否だけは `requested: None` とし、不正な Map key を拒否記録へ転記しない。他の payload 由来の拒否は step 4 の後にだけ置く。
- `Emit` の再帰 ASCII 検査は 3c で足す。そのときも「検査対象の追加」と「順序の変更」を分けて扱う。

## 2. KC-28 — Session entry front matter and the ControlOps

Additions to design/06 (KC-28 and KC-24's scope sentence):

> KC-24 is the Kernel Action admission's only order. A Session entry's own front matter and the ControlOps stand outside it: an Action they refuse never reaches KC-24, and a rejection they make is still a Session entry in the action log.

| position | condition, refusal | record |
|---|---|---|
| Session door (KC-28 step 1) | `Ended` after cleanup; `NotSession` on a Spec Run | the returned error |
| `SessionLog::check_entry` | a document/ASCII violation (`Malformed`) | the returned error; no sequence number is consumed |
| `session_earliest` | a time-resolution refusal, `ezsdr.time` | a `Rejected` Session entry |
| `session::compile` | a Vocabulary or artifact refusal, `ezsdr.vocabulary` / `ezsdr.artifact` | a `Rejected` Session entry |
| a ControlOp's `RunChild` | `ezsdr.run_child` | a `Rejected` Session entry |
| a ControlOp's `Adopt`/`Renew` | `ezsdr.lease` | a `Rejected` Session entry |

A class-pending update request is the one thing `session::compile` and `arm` hand to KC-24 instead of a finished Action (KC-28 step 3, KC-17) — this form lands in **3b**; 3a keeps the finished-Action form and the existing `class` fields, checked through `declared_class`. In 3b `session::compile` no longer refuses an update for a missing class: that check is KC-24 step 5. The compiler keeps its syntax and verb-conversion refusals.

## 3. Per-rule amendments (by stage)

### Stage 3a

- **SB-2** (design/03): the declaration of a key depends on its owner. A Spec's key positions are exactly five: a resource's `requires`, a `needs[].requires` under it, an output's `params`, a `policies.coercion` key, and a `schedule` entry's `UpdateParameter`. One iterator (`ExperimentSpec::keyed_entries`) feeds the prefix check and the declaration check over all five, so the two cannot look at different positions. At each position: an `ext.` key is checked only for its **owner** — its Module id must name a registered Module, with `ext.` stripped exactly once — and has no `KeyDecl`; any other key must be declared by a Vocabulary in `requirements.vocabularies`, and its constraint or value must be of that declaration's kind, including a List or Map judged by its outer kind. A `{component}` schedule target is the exception: its key is owned by that component's own `ParamDecl`, no Vocabulary prefix is required, no `KeyDecl` is consulted (a missing or fixed `ParamDecl` is never a reason to consult one), and no kind check applies; the component's `schema` stays opaque. `Command` params and a `TxBurst`'s `metadata` are not positions of this check and require no `KeyDecl`; they keep their own rules (SB-4, RS-44a). `Num` accepting an `Int` scalar is the kind rule below (SB-6), separate from the exact numeric comparison.
- **SB-6** (design/03): the kind-acceptance rule is one function: a `Num` declaration accepts an `Int` scalar too, and every other pair is the same kind; a List or Map is judged by its outer kind. The constraint-versus-capability comparison keeps its own exact numeric comparison and is not this rule.
- **SB-16** (design/03): a schedule entry's target is a `{resource}`, `{output}` or `{component}` of the Spec, checked by `classify_target` in `validate()`, which a `{resource}` sub-path also holds to SB-1's grammar. The runtime resolution reads the routing built from that validated plan, so it names no other role: no declaration check stands in the runtime resolver (review P2-4; an earlier call there was unkillable).
- **SB-17, SB-19** (design/03): an output's `params` value and a `policies.coercion` key now also need a Vocabulary `KeyDecl` (and, for the value, its kind); SB-2 carries the refusal, named `UnknownKeyPrefix`.
- **SB-30** (design/03): the `validate`-point configuration for a resource holds the values of its own `Eq` constraints only — a need's `Eq` values no longer join it; the need is a resolution, not a request of that fragment. The quote of SB-30 in design/06's Phase 2 amendment and the test text beside it still describe the joined form and are marked superseded by this sentence.
- **MA-34** (design/05): a component's parameter keys are owned by the component that declares them. The `ext.`-owner rule, the prefix check and the `KeyDecl` shape check apply to the five positions SB-2 lists. **MA-37**'s structural list gains one item of its own: a component declares each parameter key once; two ParamDecls with one key inside one component are refused, and different components may declare the same key.
- **MA-37 order** (design/05): a descriptor is validated **before** anything reads it, the schedule's `declared_class` included; a duplicate parameter key is therefore refused as MA-37 and not as RS-52 (review P2-5).
- **RS-52** (design/04): `ParamDecl.update_class: Option<UpdateClass>`; `None` means fixed for the Run, and the target's own declaration is the only source — a fixed or missing component declaration is refused, never falling back to a Vocabulary declaration (plan §2.2 item 2).
- **RS-17** (design/04): the runtime admission's class check runs at KC-24 step 5, before the coercion and the registered checks; the `Admitter` no longer carries `declared_classes` or a class check at any stage (the removal landed in 3a, one stage before plan §5 placed it: the class refusal it used to make is admission's own, and no other runtime `Admitter` exists). `validate()` and `prepare()` keep the registered checks and the coercion policy. The RS-17 test-table rows in design/04 that drove the `Admitter`'s class check are replaced by the runtime carriers named below.
- **RS-14 / RS-49a** (design/04): in 3a the existing `class` fields of `ActionTemplate` and `CompileRule` stay, and the class they state is checked through `declared_class` (the schedule path by `validate()`, a Session update by `session::compile`). The class-pending request form is 3b.
- **KC-17** (design/06): the timed entries keep their finished-Action form in 3a; the agenda holds only untimed `Stop`s, unchanged. `(instant, schedule index)` order and the working configuration are unchanged. The class-pending request, and the `arm`-time `Failed { arm }` for a declaration gap, are 3b.
- **KC-23** (design/06): a `{component}` target resolves through the Island that lists it (unchanged).
- **KC-28** (design/06): step 3 calls `session::compile` with the Spec, not a caller-built map of component classes (the signature change landed in 3a). The compile's own class lookups go through `declared_class`.
- **KC-26** (design/06): its "RS-17 step 0" reference becomes the coerce step of KC-24 (step 10 of §1).

- Landed (3a): `keyed_entries`, `classify_target`, `declared_class`, `kind_accepts` and `check_value_kind` exist; `key_decl` returns `Option`; the `Admitter`'s runtime class step and its `declared_classes` field are deleted; `session::compile` takes the Spec; the `ext.` owner strip is `strip_prefix` (review P1-1); `classify_target` is `validate`'s alone (review P2-4). Tests: `sb_16_a_schedule_entry_may_target_a_component` (component class, no-prefix key `proc.threshold`, missing-declaration no-fallback), `sb_02_one_key_declared_differently_by_two_components`, `sb_02_values_are_checked_against_their_key_decl`, `sb_02_a_component_value_is_not_checked_against_the_vocabulary_kind`, `sb_02_a_list_value_matches_only_a_list_declaration`, `sb_02_the_ext_owner_is_checked_at_every_key_position` (coercion, output, needs, schedule, doubled `ext.`, no-KeyDecl), `ma_37_a_component_declares_each_parameter_key_once` (MA-37 before RS-52), `rs_17_a_component_class_comes_from_its_own_declaration` (Session compile), `kc_24_component_update_classes_belong_to_the_target` (fixed and missing declarations at runtime), `kc_17_a_scheduled_update_to_a_component_reaches_its_executor`; `sb_16_spec_time_resolves_at_arm`, `sb_30_the_check_sees_each_fragment_s_own_value` and `sb_08_the_test_vocabulary_names_no_radio_word` follow the widened target, the removed need join and the `test.list` carrier. Mutations J177–J179 re-spelled (J222 retired with the deleted runtime call), J200–J223 added and J210–J221 pinning the review's fixes.

### Stage 3b

- **KC-28** (design/06): step 3 compiles to finished Actions and class-pending update requests; the class is not consulted at compile time; step 4 admits each through KC-24; `session::compile`, `Compiled` and `ActionTemplate::resolve` become internal to the Kernel crate.
- **RS-14 / RS-49a** (design/04): a class-pending update request is the internal form `session::compile` and the timed schedule produce; it holds the target, key, value and time and receives its class at KC-24 step 5.
- **KC-17** (design/06): the timed entries' admission goes through the same KC-24 with the class-pending request; the class comes from `declared_class`, and a declaration that is fixed or missing is `Failed { arm }` naming the entry (RS-52). `(instant, schedule index)` order and the working configuration are unchanged.
- **KC-24** (design/06): the rule and table of §1 above land here; the checked `SessionLog` path is unchanged.

### Stage 3c

- **RS-32 / RS-33** (design/04), **MA-6** (design/05): a Module receives a root-bound `EventSink` view; `resolve` is fallible; a source outside the instance's root, and `Kernel`/`Unforeseen` from a Module, are refused; the `EventHandle` carries one process-unique scope id; `emit_control` and `Action::Emit` check ASCII keys.
- **KC-8 / KC-30** (design/06): each instance's source root is the one `PrepareContext.source` states; an Executor's root comes from its parsed `IslandDecl`, and the routing-miss fallback to `Unforeseen` is deleted.

### Stage 3d

- **MA-10 / MA-25 / MA-39** (design/05): `Resource.memory_domains` per node, omitted means empty, and a node with ports must declare at least one; a graph link or a Sink feed touching a resource endpoint is checked like every other end (D31 closed). The quantification is the plan's §7 formula: for producer set `P` and consumer set `C`, every `p ∈ P` has a `c ∈ C` with `p == c` or a selected `Link.connects` pair `(p, c)` or `(c, p)`. Component sets are the Island's chosen domain's singleton, a Sink's the domains its descriptor reads, a Resource's the domains of the node it bound.
- **MA-6/MA-10** (design/05): `Resource` gains the field; the shared radio tree builder takes the host domain from its caller (`ezsdr_hostmem::HOST_MEMORY`) and declares it on the port-bearing root.

## 4. Schemas and surfaces

- Schema regeneration per stage: `action_template`, `vocabulary_descriptor`, `experiment_spec`, `component_descriptor`, `resource`, `provider_instance` (and embedded schemas).
- `Action::UpdateParameter.class` stays mandatory; old templates and `CompileRule`s that carry a removed `class` are refused, never reinterpreted (invariant 39).
- No versions are bumped and no `SCHEMA_CHANGELOG` entry is written before the freeze (AGENTS.md §6); the Kernel's public surface and `kernel_surface_allow.txt` are updated where a public signature changes (`ModuleRegistry::key_decl`, `Resource`, `session::compile`, `Compiled`, `ActionTemplate::resolve`).
