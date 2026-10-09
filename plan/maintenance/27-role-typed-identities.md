# Maintenance spec 27 — Role-typed identities (pre-freeze item 2)

| Field | Value |
|---|---|
| Status | **Draft for the owner**, written by the orchestrator (Claude Opus) on 2026-10-09. It implements the owner's decision on audit item 2 ([24-prefreeze-audit.md](24-prefreeze-audit.md), Owner decisions, "item 2"; findings F13, F14, F15, F16, F17 (#54), F18, F24, F25 f, F39's source half) and opens with the design note AGENTS.md §6 asks for. Not implemented. It lands after spec 24 and spec 26, and before item 3, which builds on the typed targets. |
| Mechanisms | **`string-composed-identity`**, four bugs: #1, #15, #16, #26. **`correlation-by-partial-key`**, four bugs: #12, #17, #30, #31. See [failure-mechanisms.md](failure-mechanisms.md). |
| Stages | **2a** addresses (one per mechanism), then **2b** correlation. Each stage goes through implementation, independent review, the mutation gate and a commit, and leaves the tree green. |
| Versions | None are bumped and no SCHEMA_CHANGELOG entry is written (AGENTS.md §6). Both stages regenerate schemas. |

## 1. Design note (AGENTS.md §6)

**What keeps failing.** Two shapes of the same habit: a fact the Kernel holds is turned into a string or a count, and then has to be recovered from it.

*Addresses composed as strings.* One `ResourceId` string type stands for six roles:
- a Provider node;
- a Spec-relative resource target;
- `sink/<output>`;
- `island_<n>`;
- `kernel`;
- `unforeseen`.

The first path segment tells them apart. That is the source of:
- the reserved names: the `sink` name (SB-22a) and the four prefixes of SB-22h, which KA-14 already had to widen once;
- #16, an output whose `sink/<id>` does not parse;
- #26, `sink/radio` reaching the radio.

Need resolutions share `matched` with resources under `<resource>_<need>`. That join is not injective, so #15 added a refusal of legal name pairs. Artifacts are named `<output>_<n>`, so #1's overwrite became possible, and a Manifest reader can only learn an artifact's output by parsing the prefix.

*Records matched by count or by label.* A Module receives an Action without its ActionId, so:
- the Sink numbers its captures (HD-16);
- the Python client predicts the number (`_captures`);
- any Action the client did not count shifts it.

The same habit appears in two more places:
- **A report's self-declared label:** `PrepareReport.fragment` is a label the Kernel must check against the fragment it invoked (#17).
- **A queue without ids:** the dispatch queue drops the ActionId, so an Action discarded at the freeze leaves no record (F24).

**Can the concept be removed or merged?**
- **The reserved names and prefixes, removed.** These are the `sink` name, SB-22h's list, `need_key` with SB-36's collision refusal, the Sink's request counter, the client's `_captures` and `_capture_recorder`, and the label check on PrepareReport. Each existed only to recover a role or a correlation that a type now carries.
- **`ResourceId`, kept** for what it names: a node in a Provider's resource tree.
- **The Spec-relative target, kept.** KC-23's rewriting is the rule that lets a Spec stay free of bindings (§3, "intent and binding are separate"). Only its encoding changes.

**Is a rule implemented twice?** Yes, in each case:
- **The `island_<n>` derivation:** four copies, three in the Kernel and one in the native Executor.
- **The Sink address:** in the capture Sink, the native Executor and the Python client.
- **The single-recorder rule of #54:** in `session.rs` and in Python's `_capture_recorder`, and the two copies already disagree.

**Is the spec text too complex?** SB-22h, SB-36's collision sentences, HD-16's counting and RS-14's recorder clause all describe the workarounds. They are deleted rather than reworded.

**Outcome.** Each role gets its own type, and the Kernel hands each fact to the party that needs it instead of letting it be rebuilt.
- **Rejected 1:** keep the strings and refuse the collisions, which is today's design. Every new Kernel-side source needs another reserved segment, and a frozen refusal cannot be loosened (D85).
- **Rejected 2:** use a separator outside the Ident grammar for need keys. One map would still hold two roles.
- **Rejected 3:** identify artifacts by their hash. Two silent captures have one hash.

## 2. Stage 2a — addresses (`string-composed-identity`)

**Targets.** `Target = {resource: Ident, path: String} | {output: Ident} | {component: Ident}`, tagged by `kind` as every Kernel tag is after spec 24.
- Every Action's target is a `Target`, and `Stop`'s is optional as today. This covers Session, schedule and Module Actions alike: KC-23 says every origin uses Spec-relative targets.
- `path` is the sub-path below the Spec resource and may be empty. It keeps SB-1's segment grammar.
- **Resolution (KC-23).** A `{resource}` target is resolved to `matched[resource]`'s node path plus `path`. `{output}` goes to that output's Sink, and `{component}` to the Island that lists it. A name the Spec does not declare in that role is refused (`ezsdr.target`). The prefix parsing in `rewrite_spec_target` goes away.
- **What a Module receives (2b's `Dispatched`).** The Action keeps its authored target, and `Dispatched.node: Option<ResourceId>` is the node path a `{resource}` target was resolved to. A Provider matches on `node`, which is what it compares today. The action log records the authored target, so the log reads in the Spec's own names.
- **#54 is settled.** An UpdateParameter verb compiles to the target the verb names, like every other target. `sink.capture` names `{output: rec}`. The implicit single-recorder choice is deleted, along with `session.rs`'s recorder branch, Python's `_capture_recorder`, and RS-14's recorder clause apart from one sentence. Close #54.

**Event sources.** `EventSource = {node: ResourceId} | {output: Ident} | {island: IslandId} | {kernel} | {unforeseen}`, used in `Event.source`, `CounterRow.source` and wherever a source is recorded (`StopCause::Policy`, `ArtifactMark`).
- Each instance receives its source root through `PrepareContext.source`, which the Kernel already computes (`state.rs`, `source_root`):
  - a Provider gets its node root and names sub-nodes below it;
  - a Sink gets `{output}`;
  - an Executor gets `{island}`.
- No Module formats a source any more.
- Deleted: SB-22h, the `sink` clause of SB-22a, the `sink/<output>` parse check in validation (#16), and the native Executor's `island_<n>`.

**`matched` and `needs`.** `matched: {<resource>: ResourceId}` holds resources only. `needs: {<resource>: {<need>: ResourceId}}` holds the need resolutions. Deleted: `need_key`, SB-36's collision refusal, SB-T1's need-key row, and the guard in `links.rs`. SB-39's check walks the nested map.

**Python.** The wire carries the typed forms. The Easy API keeps its user-facing arguments, the radio and recorder names, and builds the typed target itself. The low-level `set_parameter` and `stop` take the typed target dict, with helpers `resource(name, path="")`, `output(name)` and `component(name)`. The Python client stops parsing `sink/` prefixes.

**Spec text and schemas.**
- design/03: SB-16, SB-22a, SB-22h deleted, SB-36, SB-39, SB-T1.
- design/04: RS-14, RS-31, RS-49.
- design/06: KC-8, KC-23, KA-14.
- design/05: MA-5, `PrepareContext`.
- design/10: HD-16's addressing.
- design/16: the protocol's target and source forms.
- Changes rows in each.
- Regenerate the schemas: event, action, action_template, admission_result, manifest, log_entry, and the server frames.

**Tests:**
- a resource, an output and a component that share one name are each addressed without ambiguity;
- a target naming an undeclared name in its role is refused;
- `sink` as a resource name and a Provider node under `kernel/` are accepted;
- `a`/`b_c` and `a_b`/`c` needs no longer collide;
- a capture addressed `{output: rec}` with two Sinks bound reaches `rec` (#54);
- each Module's events carry the source the context gave it.

**Mutation rows:** KC-23's resolution per role, the source each kind of instance is given, and the `needs` walk of SB-39.

## 3. Stage 2b — correlation (`correlation-by-partial-key`)

**The ActionId reaches the receiver.**
- `ActionReceiver::recv() -> Option<Dispatched>`, with `Dispatched { id: ActionId, action: Action, node: Option<ResourceId> }`. It is a document type with a schema, as MA-6 requires of a role signature. The id stays out of `Action`, which is also the Spec's template.
- Vocabulary events that answer an Action carry `action: ActionId`: `sink.CAPTURE_WRITTEN`, `sink.REQUEST_REJECTED`, and a Provider's per-command refusal under MA-14.
- The Python client waits for the event whose `payload.action` is in its entry's `dispatched`.
- Deleted: the Sink's `received` and `requests` counters, HD-16's counting text, and Python's `_captures`.

**Artifacts by output.** `Manifest.artifacts: {<output>: [ArtifactRef]}`, filled by the Kernel from the instance whose `stop` returned the artifacts.
- The Sink names its captures without the output prefix.
- A capture's artifact id is the Sink's choice; numbering by ActionId is the obvious one.
- No uniqueness rule is added, by the owner's decision.

**Undelivered Actions are recorded.**
1. First, a test pins the window that F24 inferred: in a device-paced class, a Session Action admitted just as an end is requested.
2. Then the dispatch queues hold `(ActionId, Action)`, and `Queue::clear` returns the discarded ids.
3. FreezeDispatch records them in the termination section as `undelivered: [ActionId]`.

**PrepareReport without `fragment`.**
- The Kernel keys the reports by the fragment it invoked.
- `collect_prepare` takes `(fragment, Result)` pairs, so an error names its fragment.
- Deleted: the duplicate, unexpected and label checks, and `report_fragments`.

**`submit(Abort)` gets a real id.** A Module's Abort is logged with a real ActionId like every other Action (RS-15). The `ActionId(0)` sentinel is deleted.

**Spec text and schemas.**
- design/05: MA-6, MA-14, MA-14a, MA-46.
- design/04: RS-15, RS-16.
- design/06: KC-9, KC-21a, KC-24a.
- design/10: HD-16.
- design/03: the PrepareReport shape.
- Regenerate the schemas: manifest, prepare_report, dispatched (new), and the server frames.

**Tests:**
- two clients capture on one recorder, and each receives its own `CAPTURE_WRITTEN`;
- a Reactor's capture does not shift the client's;
- the F24 window records `undelivered`;
- a Module's Abort appears in the log with its id;
- a prepare error names its fragment;
- artifacts are listed under the output that produced them.

**Mutation rows:**
- the id carried into `Dispatched` and into an answering payload;
- `Queue::clear`'s returned ids;
- the output an artifact is filed under.

## 4. Size and risk

Item 2 is the widest change across crates:
- it changes the `recv` signature and the event source in every Module (MockRadio, UHD, capture Sink, native Executor);
- it changes the client protocol;
- it touches seven schemas or more.

Splitting it into 2a and 2b keeps each step reviewable. 2a changes what things are called, and 2b changes how records find their owner. Both need the mutation gate on all six lists, because the Modules' tests are where most of the edits land.

## 5. Not in this spec

- **A check that a Module's event source lies within its root.** Today the shared collector takes any source from any Module. With typed sources and the root known, this is one check at the collector. It is a question of admission, so it is offered to item 3 instead of being added here.
- **The Kernel's event kinds under `ezsdr.`:** item 4, spec 24.
- **`Fragment.content` typing and `key_decl`:** F25 c and d, under items 4 and 3.
- **A uniqueness rule for artifact ids:** not adopted.
