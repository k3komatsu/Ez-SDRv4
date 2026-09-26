# Phase 6 spec 17 — Amendments: a Run a client can watch, and child Runs

| Field | Value |
|---|---|
| Status | Draft, implemented under the owner's delegation; awaiting Gate X ([`00-overview.md`](00-overview.md) §11). Its text reaches `design/` in the commit that implements it (GY-5); this file is the record. |
| Scope | Four Kernel amendments: KF-1 (a client reads the delivered events during the Run), KF-2 (`wait_for`: wait in Run time for an event), KF-3 (child Runs, with the environment rule RS-25a lacked), KF-4 (SB-14's dangling reference). One Vocabulary amendment: VD-1 (`sink.CAPTURE_WRITTEN`). |
| Amends | `design/03-spec-and-binding.md` (SB-14), `design/04-run-and-session.md` (RS-6's coordinator paragraph, RS-25, RS-25a), `design/06-kernel-coordinator.md` (KC-28 step 5, new KC-29a and KC-29b, KC-37, new KC-37a, KC-45), `design/10-host-data-path.md` (HD-6, HD-7, new HD-16). |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Each amendment gives the problem, the rule text as it reads after the amendment, the rejected alternatives and the tests. Appendix A lists the mutations that show each test guards its rule (GY-4).

---

## KF-1 — A client reads the delivered events during the Run

**Problem.** After each round the coordinator appends the drained events to `delivered` (KC-31), and the Manifest records them (KC-45) — at the end. Nothing lets a client see them while the Run runs, so a client cannot learn that a capture was written (`00-overview.md` §2, hole 1), and Vision §15's `run.wait_for(event)` has nothing to wait on.

**KC-29a** (new):

> `RunHandle::events(from)` returns the events delivered so far (KC-31) from index `from` on, in delivery order, and an empty list when `from` is at or past their number. Index `i` names the same event for the whole Run and in the Manifest's `events.delivered`. It runs no round and is available in every state, after cleanup included, until `finish` consumes the handle (Phase 6, KF-1). *Checked by `kf_01_the_delivered_events_are_readable_during_the_run`.*

**Rejected.** A callback or channel that pushes events to the client (the coordinator runs on the client's thread in the Simulation class, and a push API would need a second thread or a re-entrant call; a pull by index is enough for every Phase 6 client). Returning only events of registered Vocabulary kinds (the Kernel's own events — `STEP_LIVELOCK`, `DEVICE_LOST` — matter to a client too).

**Code.** `coordinator/stepping.rs` (or `mod.rs`): `pub fn events(&self, from: usize) -> Vec<Event>`. A method, not an item (`kernel_surface` unchanged, GY-2).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `kf_01_the_delivered_events_are_readable_during_the_run` (coordinator) | a Session whose test Provider emits one event per round; `events(0)` after two rounds, `events(1)`, `events(99)`; `events(0)` after the Run ended | two events in delivery order; the second alone; empty; all of them, equal to the Manifest's `events.delivered` | KC-29a |

---

## KF-2 — `wait_for`: wait in Run time for an event

**Problem.** Vision §15: "Python has `run.wait_until(t)` and `run.wait_for(event)`". `advance_to` is the first; nothing is the second. A client that steps `advance_to` in quanta overshoots the event by up to a quantum, and the instant at which it logs its next Action then depends on the quantum it chose (`00-overview.md` §2, hole 2).

**KC-29b** (new):

> `RunHandle::wait_for(kinds, from, horizon)` returns the index of the first delivered event at or after index `from` whose kind is one of `kinds`. If one is already delivered it returns at once without running a round. Otherwise it places `horizon` on the primary root as KC-29 does (else `NotOnPrimaryRoot`), schedules a no-op callback at it as `advance_to` does, and runs KC-20's loop one round at a time, checking the events each round delivered: it returns `Ok(Some(i))` after the round that delivered the first match, standing at that round's instant; `Ok(None)` after a round has run at an instant at or after `horizon` with no match, standing at `horizon`; and `Err(Ended)` if the Run ended first, whether or not the round that ended it delivered a match (the events stay readable, KC-29a). With `kinds` empty it is `advance_to(horizon)` (Phase 6, KF-2). *Checked by `kf_02_wait_for_returns_at_the_round_that_delivered`, `kf_02_wait_for_stands_at_its_horizon` and `kf_02_wait_for_finds_an_event_already_delivered`.*

**Rejected.** A predicate over the event instead of a list of kinds (a closure cannot cross the protocol, and every Phase 6 wait is by kind; a client filters by source itself). Returning the event instead of its index (the index is what the next `from` needs, and `events(i)` returns the event).

**Code.** `coordinator/stepping.rs`: `run_loop` gains an optional stop check evaluated after each round; `pub fn wait_for(&mut self, kinds: &[EventKind], from: usize, horizon: TimePoint) -> Result<Option<usize>, RunHandleError>`.

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `kf_02_wait_for_returns_at_the_round_that_delivered` (coordinator) | a test Provider that emits kind `test.MARK` at its round at instant 5 000 and `test.OTHER` at 3 000; `wait_for([test.MARK], 0, 10 000)` | `Some(i)` with event `i` the `test.MARK`; `now()` is 5 000, not later | KC-29b |
| `kf_02_wait_for_stands_at_its_horizon` | the same Provider emitting nothing; `wait_for([test.MARK], 0, 7 000)` | `None`; `now()` is 7 000 | KC-29b |
| `kf_02_wait_for_finds_an_event_already_delivered` | the `test.MARK` already delivered; `wait_for` from its index, then from the index after it with a horizon | `Some` at once with `now()` unchanged; then `None` at the horizon | KC-29b |

---

## KF-3 — Child Runs

**Problem.** Vision §3 and §54: `sdr.run(spec)` creates a child Run under the Session's Lease. RS-25 and RS-25a describe it and KC-37 refuses it. Even admitted, `SessionAction::RunChild { spec_hash, binding_hash }` names its documents by hash with no store to find them in, and the Kernel builds no Module (KC-4), so it has nothing to run them with (`00-overview.md` §2, hole 3). And RS-25a ties a child's devices and Authority to its parent's but not its `environment`, so a child profile without the parent's `radio.rf_envelope` would escape the RF safety envelope (§52; hole 4).

**KC-37 amended**:

> `submit` refuses `RunChild`, logged `Rejected` with the violation `ezsdr.run_child` and the reason "RS-25a: a child Run is created with `run_child`, which carries its documents and Modules" (Phase 6, KF-3).

(KC-28 step 5's "`RunChild` is refused (KC-37)" stands.)

**KC-37a** (new):

> `RunHandle::run_child(spec_doc, profile_doc, assembly, drive)` on a Session, at the current instant `now`:
>
> 1. refuses with `Ended` after cleanup and `NotSession` on a Spec Run; the entry's action is `RunChild { spec_hash: ContentHash::of_value(spec_doc), binding_hash: ContentHash::of_value(profile_doc) }`, and it refuses with `Malformed` when a hash cannot be computed or `SessionLog::check_entry(now, &action)` fails (RS-15: no sequence number);
> 2. admits the child, the first failing check giving the entry `Rejected` with one violation `ezsdr.run_child`: the Run is `Running` (reason "RS-18: …"); `spec_doc` parses as an ExperimentSpec and `profile_doc` as a BindingProfile (the parse error); each binding `assembly.providers` names has a binding description — `(module, selector, profile)`, SB-3 — equal to one of the parent profile's bindings ("RS-25a: <name> binds an instance its parent does not"); the child's `authority` binding has the parent's `authority` binding's description ("RS-25a: the Authority …"); and every `environment` section of the parent's profile that an admission check registered for the parent reads (SB-29) is in the child's profile with an equal value ("RS-25a: section <ns> …");
> 3. a rejected entry is appended (RS-15) and the call returns it with no Manifest;
> 4. an admitted entry is appended as `Admitted { coercions: [], warnings: [], dispatched: [] }`; the child is started as `start_spec_run` starts a Run, with kind `Spec`, the parent's id as its parent and a copy of the parent's current Lease as its own (RS-25); `drive(&mut child)` runs it; the child is finished (`finish`, KC-34), which cleans it up if `drive` left it running; the parent records `{ seq, run, manifest }` — the entry's sequence number, the child's id and its Manifest's hash (null if unsealed) — for `ezsdr.children` (KC-45); the parent then checks its Lease (KC-36), so a Detached Lease that expired while the child ran ends the child first and the parent after (RS-25);
> 5. the call returns the entry and the child's Manifest.
>
> The parent's time does not advance while the child runs: the child has its own Authority and clocks, from its own Assembly. A child is never live when `run_child` returns, so RS-6's step 0 finds none (Phase 6, KF-3). *Checked by `kf_03_a_child_run_is_admitted_logged_run_and_recorded`, `kf_03_rs_25a_refusals`, `kf_03_a_child_inherits_the_lease` and `kf_03_run_child_is_a_session_verb`.*

**KC-45 amended**: `run.parent` is the parent's id for a child Run (KC-37a) and `None` otherwise; `sections` also holds `ezsdr.children` when the Run created a child: an array of `{ "seq": u64, "run": RunId, "manifest": ContentHash | null }` in creation order.

**RS-6's coordinator paragraph amended** (step 0): "**0** nothing: a child Run is cleaned up inside `run_child` before it returns, so none is live at its parent's cleanup (KC-37a)."

**RS-25 amended** (its marker): "*Step 0 is checked in Phase 1 by `rs_25_child_inherits_the_lease`; the inherited Lease in Phase 6 by `kf_03_a_child_inherits_the_lease` (KC-37a).*"

**RS-25a amended** (a sentence added before the marker, and the marker):

> The child's profile also carries, with an equal value, every `environment` section of its parent's that an admission check registered for the parent reads (SB-29): a child may add a section and may not drop or change one the parent's checks judge, so it cannot leave its parent's RF safety envelope (Vision §52) (Phase 6, KF-3). *Checked in Phase 6 by `kf_03_rs_25a_refusals` (KC-37a).*

**Rejected.**

- *A live child handle returned to the caller.* RS-6's step 0 has the parent clean up its children; a handle the client holds could outlive the parent's cleanup, and the Kernel would need a registry of live children and a way to reach into a handle it gave away.
- *The Kernel fetching the documents and Modules by hash.* There is no store (Phase 2 Y12), and the Kernel builds no Module (KC-4). The documents travel with the call and the child's Manifest records both, so the hashes in the parent's log always resolve through `ezsdr.children`.
- *Environment equal in full.* A sweep over a simulated channel — §54's "for snr in snrs" — changes the child's `sim.channel`, which no admission check reads; only the sections a check judges bound the child.
- *Checking the child's sections against the envelope's meaning (a narrower band is allowed).* The Kernel does not read a section's content (SB-29); equality is what it can judge.
- *Running the child beside the parent.* The Simulation class steps one Run on the client's thread (KC-2); the parent standing still is what a paused Session is.

**Ceiling.** On a wall-paced class the parent's devices keep running while the child uses them; the first hardware Provider decides whether `run_child` quiesces the parent's streams (Phase 7). A Session replayed later (RS-20) re-creates a child from the child's Manifest, which holds both documents; replay itself is out of Phase 6 (`00-overview.md` §3).

**Code.** `coordinator/mod.rs`: the parent id and the children list on `RunHandle`; `pub fn run_child(&mut self, spec_doc: &serde_json::Value, profile_doc: &serde_json::Value, assembly: Assembly, drive: &mut dyn FnMut(&mut RunHandle)) -> Result<(LogEntry, Option<Manifest>), RunHandleError>`; `coordinator/pipeline.rs`: the admission and KC-37's reason; `coordinator/ending.rs`: `run.parent` and `ezsdr.children`.

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `kf_03_a_child_run_is_admitted_logged_run_and_recorded` (coordinator) | a Session with a test Provider; `run_child` of a Spec Run on the same binding description, `drive` running it to a horizon | the entry is `Admitted` with action `RunChild` and the documents' hashes; the child's Manifest has `run.parent` = the Session's id and `kind` spec; the parent's `now()` is unchanged; the parent's Manifest's `ezsdr.children` is `[{ seq, run, manifest }]` with the child's id and Manifest hash | KC-37a, KC-45 |
| `kf_03_rs_25a_refusals` | a child profile whose Provider binding has another selector; whose Authority binding has another selector; that drops the parent's checked section; that changes it; that changes an unchecked section; a Spec that does not parse | the first four and the last `Rejected` with `ezsdr.run_child` and no Manifest, the entry appended; the fifth admitted | KC-37a, RS-25a |
| `kf_03_a_child_inherits_the_lease` | a Detached Session Lease; the child's Manifest | the child's `lease` equals the parent's at the call | KC-37a, RS-25 |
| `kf_03_run_child_is_a_session_verb` | `run_child` on a Spec Run; after the Session ended; `submit(RunChild)` | `NotSession`; `Ended`; `Rejected` with KC-37's reason | KC-37, KC-37a |

---

## KF-4 — SB-14's dangling reference

**Problem.** SB-14: "parametrisation is the client-side builder of Vision §9, whose source hash the Manifest records beside the Spec hash (spec 04)". Spec 04 defines no such field, and `SpecSection` has `hash`, `body`, `original_version` and `original_hash` only (`00-overview.md` §2, hole 5). A reader of SB-14 would look for a field that is not there; a Kernel field added after v4.0 would be a Manifest schema change.

**SB-14 amended** (its last clause):

> …parametrisation is the client-side builder of Vision §9. *Forward obligation: the builder's source hash beside the Spec hash (Vision §9) needs a Manifest field that spec 04 does not yet define; it is to be added before the v4.0 freeze, with the first Spec builder (Phase 6, KF-4).*

No code. The owner decides at Gate X whether the field is added before the freeze or §9's sentence is withdrawn (`00-overview.md` §11).

---

## VD-1 — The capture Sink says when it has written a capture

**Problem.** A client cannot learn that a capture is written until the Session ends (`00-overview.md` §2, hole 1). The Sink knows the moment it records the `ArtifactRef` (HD-10); the event path (§29) is how a Module tells the Run and its client what happened.

**HD-6 amended**: the Vocabulary's version is 1.1.0 and `event_kinds` is `[sink.REQUEST_REJECTED { default: continue, severity: warning }, sink.CAPTURE_WRITTEN { default: continue, severity: info }]`; `register` registers both. `ezsdr_sink::CAPTURE_WRITTEN = "sink.CAPTURE_WRITTEN"`.

**HD-7 amended**: the Module's version is 1.2.0, its `vocabularies` `[{ sink, ^1.1.0 }]` and its `impl_hash` `ContentHash::of_bytes(b"ezsdr.sink.capture 1.2.0")` (Phase 6, VD-1).

**HD-16** (new):

> Each time the Sink records an `ArtifactRef` — a capture completed (HD-10), finished partial by a `Stop` (HD-11) or finished by `stop` (HD-13) — it emits `sink.CAPTURE_WRITTEN` with `emit_control`, source `sink/<output id>`, time the current instant in the primary root, and payload `serde_json::to_value(CaptureWrittenPayload { artifact })`, after the data file is closed and its metadata written (HD-15), so that a client reading the artifact finds both. `CaptureWrittenPayload { artifact: ArtifactRef }` (`deny_unknown_fields`) is defined in `ezsdr_sink` and its schema is committed as `schemas/sink/capture_written_payload.v1.json` (PO-7). The event recorded by `stop` reaches the Manifest's events only as far as cleanup still drains the event path (KA-12); a client never needs it, since `finish` returns the artifact itself (Phase 6, VD-1). *Checked by `hd_16_a_written_capture_is_announced`.*

Every profile that names the capture Sink names 1.2.0 (HD-8 refuses another version); the acceptance rig's and the server's do.

**Rejected.** Reporting through a Manifest section at the end (a client needs it during the Session). A Kernel event kind (the artifact is the Sink's, and its payload's schema belongs to the Vocabulary that owns the kind, RS-31).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `hd_06_vocabulary` (amended) | the descriptor | 1.1.0 with both kinds | HD-6 |
| `hd_07_descriptor` (amended) | the descriptor | 1.2.0, `sink ^1.1.0` | HD-7 |
| `hd_16_a_written_capture_is_announced` (sink-capture) | a request of 1 000 samples in the harness; a second request finished partial by a `Stop` | one `sink.CAPTURE_WRITTEN` per capture, source `sink/rec`, payload equal to the `ArtifactRef` `stop` later returns, the second with `partial: true`; the `.sigmf-meta` of the first exists when its event is emitted | HD-16 |
| `hd_14_schema_freeze` (amended) | the payload types | equal to `schemas/sink/*.v1.json` | HD-16 |

---

## Appendix A — Mutations (GY-4)

Each mutation disables one rule's code; the listed test must fail. The list is `tools/mutations.json`, run by `tools/mutate.py` (Phase 5's tool).

| # | Rule | Mutation | Killed by |
|---|---|---|---|
| F01 | KC-29a | `events` returns nothing | `kf_01_*` |
| F02 | KC-29a | `events` ignores `from` | `kf_01_*` |
| F03 | KC-29b | `wait_for` does not look at events already delivered | `kf_02_wait_for_finds_an_event_already_delivered` |
| F04 | KC-29b | `wait_for` matches any kind | `kf_02_wait_for_returns_at_the_round_that_delivered` |
| F05 | KC-29b | `wait_for` schedules nothing at its horizon | `kf_02_wait_for_stands_at_its_horizon` |
| F06 | KC-29b | `wait_for` runs to the horizon before looking | `kf_02_wait_for_returns_at_the_round_that_delivered` |
| F07 | KC-37a | the Provider description check skipped | `kf_03_rs_25a_refusals` |
| F08 | KC-37a | the Authority description check skipped | `kf_03_rs_25a_refusals` |
| F09 | RS-25a | the environment check skipped | `kf_03_rs_25a_refusals` |
| F10 | RS-25a | the environment check also binds unchecked sections | `kf_03_rs_25a_refusals` |
| F11 | KC-37a | the child's parent not set | `kf_03_a_child_run_is_admitted_logged_run_and_recorded` |
| F12 | KC-37a | the child gets a fresh Attached Lease | `kf_03_a_child_inherits_the_lease` |
| F13 | KC-45 | `ezsdr.children` not written | `kf_03_a_child_run_is_admitted_logged_run_and_recorded` |
| F14 | KC-37a | a rejected child not appended to the log | `kf_03_rs_25a_refusals` |
| F15 | KC-37 | `submit(RunChild)` keeps the Phase 2 reason | `kf_03_run_child_is_a_session_verb` |
| F16 | HD-16 | no `CAPTURE_WRITTEN` | `hd_16_*` |
| F17 | HD-16 | no `CAPTURE_WRITTEN` for a partial capture | `hd_16_*` |
| F18 | EA-13 | `read` serves any `file://` URI | `ea_13_*` |
| F19 | EA-14 | the derived child profile keeps `feed` | `ea_14_run_child` |
| F20 | EA-10 | `connect` does not advance to T0 | `ea_10_connect_stands_at_t0` |
| F21 | EA-12 | durations rounded down | `ea_12_time_and_events` |
| F22 | EA-2 | `body_bytes` not checked against the stream | `ea_02_framing` |
| F23 | EA-3 | any protocol version accepted | `ea_03_handshake` |
| F24 | EA-13 | a reported capture is not made readable | `ea_13_*` |
| F25 | EA-14 | a child without duration or stop is not refused, and one with a stop is | `ea_14_run_child` |
| F26 | KC-37a | the Provider description check skipped, through the server | `ea_14_run_child` |
| F27 | HD-16 | no `CAPTURE_WRITTEN`, through the Python client | `test_v57_repeat_then_capture_in_software` |
| P01 | EA-16 | `repeat` does not set the channel count | `test_v57_repeat_then_capture_in_software` |
| P02 | EA-17 | captured frames not transposed | `test_ea_17_two_channels_round_trip` |
| P03 | EA-17 | a 2-D waveform sent without interleaving | `test_ea_17_two_channels_round_trip` |
| P04 | EA-16 | `sleep` converts seconds wrongly | `test_v54_sleep_is_run_time` |
| P05 | EA-16 | `wait_for` returns one event twice | `test_ea_16_events_and_wait_for` |
| P06 | EA-16 | a rejected entry does not raise | `test_v58_16_a_rejected_call_raises_and_is_logged` |
| P07 | EA-16 | a capture timeout is not raised | `test_ea_16_errors` |
| P08 | EA-16 | `run` does not pass the duration | `test_v54_a_sweep_of_child_runs` |

F24 onwards and P01–P08 were added while running the list (spec 16's rules have code in the server and the Python package too). The Python mutations (`python` in `mutations.json`) run their unittest against the scratch copy's server. The list may grow with the reviews; the final count is `mutations.json`'s.
