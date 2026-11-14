# #101 — test plan v2 (AGENTS §9 step 2).

**v2 changes (Sol's plan review, owner's Q3, as implemented):** `FakeFault::LostOnCall(call, nth)` takes the nth call, so T1 (`ur_29_a_lost_reopen_while_a_stop_is_booked_is_published`) fires `LostOnCall("tx_open", 1)` after `owner_with`'s setup open and asserts the setup succeeded; Q9 adopted (tx.rs:855 publishes a lost error whatever a booked stop); Q5 rejected: every coordinator row asserts exactly one delivered `DEVICE_LOST` (it holds: the baseline and all five rows deliver one); Q3 decided by the owner: `rates_checked` is false after a lost read, asserted in the Manifest; the mutation rows are N101-01..12 (old 06 folded into the retargeted N67F-29, old 10-12 renumbered 08-10 and old 13 split into 11-12); the receive owner's open is tested at the owner (`rx.begin()`), not through `owner_configure` alone; §0's text stays as the implementation's shape.

(Original plan follows; its header said:) No code was written for this plan.

Tree: `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n101`, main 89dc141e. Paths below are under `crates/ezsdr-radio-uhd/` unless they start with `design/`, `plan/` or `KNOWN_LIMITATIONS.md`. Every claim about the code was read in this tree; "INFERRED" marks what was not run or not readable.

## 0. What the plan assumes about the code (so the rows have concrete anchors)

The decisions fix behaviour, not text. The mutation rows below are written against this shape; if the implementer's text differs, each row is re-derived as the smallest unique token sequence and `rules.py` must still find it exactly once.

```rust
// core.rs — the one decision "a DeviceError that is the loss becomes DEVICE_LOST" (new; replaces the copies listed in §1)
pub fn publish_loss(&self, error: &DeviceError) -> bool { if error.lost { self.device_lost(&error.message); } error.lost }
pub fn device_failed(&self, id, action, error) { if !self.publish_loss(error) { self.command_rejected(id, action, &error.message); } }

// mod.rs — `refuse_open` generalised (the only refusal conversion for prepare and arm)
fn refuse(core: &Core, error: DeviceError, text: impl FnOnce(DeviceError) -> String) -> ModuleError { core.publish_loss(&error); rejected(text(error)) }
//   prepare:    core.configure(dir, &entries, rate, k).map_err(|e| refuse(&core, e, |e| e.message))?;
//   ref_locked: Err(error) => return Err(refuse(&core, error, |e| format!("UR-13: {e}"))),
//   rx open:    core.open_streamer(Dir::Rx, stream.k, stream.channels).map_err(|e| refuse(&core, e, open_refusal))?;
//   tx open:    core.open_streamer(Dir::Tx, stream.k, stream.channels).map_err(|e| refuse(&core, e, open_refusal))?;
//   start:      core.publish_loss(&error);            // replaces `if error.lost { core.device_lost(&error.message); }`

// core.rs check_rates: a lost read is returned as the device's own error, before it can become the cached String
//   Err(error) if error.lost => return Err(error),     // new arm, before `Err(error) => Err(format!("UR-13: {}", error.message))`
//   `rates: Mutex<[Option<Result<(), String>>; 2]>` keeps its type, so a lost read stores nothing and `rates_checked` stays false.

// core.rs: the two copies of "close a streamer a refused comparison left open" become one
fn close_if_refused(&self, dir: Dir, k: usize, checked: &Result<(), DeviceError>) { if matches!(checked, Err(error) if !error.lost) && !self.is_lost() { self.close_device(dir, k); } }
//   called by open_streamer (core.rs:1549-1553) and reopen_tx_streamer (core.rs:1411-1419).

// uhd.rs check_with: `|| (code == UHD_ERROR_EXCEPT && matches!(call, "uhd_usrp_get_rx_stream" | "uhd_usrp_get_tx_stream"))`; new const UHD_ERROR_EXCEPT = 47.
```

Where publication happens: `check_rates` reads the device under `device_gate` and drops it before it returns (core.rs:1137-1146); `device_lost` takes that gate (core.rs:1079-1081), so the loss is published by the caller, after the read returns. Between the lost read and that publication the Provider must make **no device call**: today `open_streamer`/`reopen_tx_streamer` still pass the guard `!self.is_lost()` and call `rx_close`/`tx_close` there (the issue's "closes the streamer on a device it thinks is fine"). That is part of site 1 and is what `close_if_refused` and rows 10-12 pin.

## 1. The sites, and the greps that found them

Greps run over the non-test part of each file (test modules start: mod.rs 759, core.rs 1657, control.rs 670, rx.rs 706, tx.rs 1203):

```
grep -n "\.lost\b" src/provider/*.rs src/authority.rs src/host.rs          # readers of the bit
grep -n "DeviceError::failed(\|map_err(\|{error}\|{e}\|error\.message\|\.message\b\|Err(e)\|Err(error)\|\.ok()" <file>   # droppers, for mod.rs core.rs control.rs rx.rs tx.rs
grep -rn "check_rates\|check_opened\|open_streamer\|refuse_open\|open_refusal\|reopen_tx_streamer" src
```

| # | Site | Today | Disposition |
|---|---|---|---|
| 1 | `check_rates` core.rs:1139-1147 (`Err(error) => Err(format!("UR-13: {}", error.message))`, cached at :1145, replayed at :1128 as `DeviceError::failed`) | lost read becomes a cached non-lost String | **fix**: return the lost error unchanged, cache nothing |
| 1b | `open_streamer` core.rs:1549-1553 and `reopen_tx_streamer` core.rs:1411-1419 close the streamer after a refused check, lost or not (`!self.is_lost()` is still true: the loss is unpublished) | `rx_close`/`tx_close` on a device whose read just said lost | **fix**: `close_if_refused` (no close when `checked` is a loss) |
| 2 | `prepare` mod.rs:530 `map_err(|e| rejected(e.message))` | drops `lost` | **fix**: `refuse` |
| 3 | tx open at `arm` mod.rs:574 `map_err(|e| rejected(open_refusal(e)))` | drops `lost` | **fix**: `refuse` |
| 4 | `ref_locked` at `arm` mod.rs:556 `Err(error) => return Err(rejected(format!("UR-13: {error}")))` | drops `lost` | **fix**: `refuse` |
| R | rx open at `arm` mod.rs:568 `refuse_open` (#67) | publishes | becomes `refuse` (same behaviour) |
| E1 | `start`'s `rx_start` mod.rs:648-651 `if error.lost { core.device_lost(&error.message); }` | publishes, but a second copy of the decision | **rewire** to `publish_loss` (behaviour unchanged; pinned by `ur_29_a_lost_later_start_is_published_before_the_owners_are_stopped`, tests/fake.rs:6688, row P476) |
| E2 | `device_failed` core.rs:1103-1109 (reached from `refuse_segment` rx.rs:432, tx.rs:534/535/857, rx.rs:294/408/505) | publishes (the owners' opens and configures) | **rewire** its lost branch to `publish_loss` (behaviour unchanged) |
| 6b | the transmit owner's reopen, tx.rs:855-858: `if let Err(error) = &result { if !core.is_lost() && lock(&core.streams).stop.is_none() { device_failed(None, "tx_reopen", error) } }` | a **lost** error from the reopen (`tx_open` today, the `rates` read after the fix) is published only when no `Provider::stop` is booked; with a stop booked the outcome is `Reopen::Interrupted { lost: false }` and nothing is published, so `cleanup` then calls `close_streams` on the gone device | **found by the extra grep; decision needed (Q9)**: recommend the condition `error.lost \|\| (!core.is_lost() && stop.is_none())`; sequence T1, row 14 |
| E3 | branch sites that keep a non-lost arm of their own: `core.rs:1281/1371` (flush time/async reads), `core.rs:1319-1324` (flush send, already `device_failed`), `control.rs:641/652/657` (uhd-control reads, UR-29's 1 s rule), `rx.rs:509`, `tx.rs:1098` | publish on lost, count or record otherwise | **not touched**: these are streaming-loop `match` arms, not refusals made from a `DeviceError` |
| 5-7 | bind-time (the decisions' numbering): mod.rs:252 `serial`, :273 `set_subdev_spec`, :284 `front_end` (all `rejected(format!("UR-5: {}", e.message))`), `authority.rs:471` (`UR-7` first time read), `uhd_usrp_make` | refusal strings | **not touched**: no `Core` exists yet, so nothing to publish to; the loss is a refusal of `connect` (decision, out of scope) |
| F | spawn errors mod.rs:662/674/681, clock/lattice errors mod.rs:580/581 | not `DeviceError`s | none |

After the change the same greps must show: `if error.lost` / `Err(error) if error.lost` only in the branch sites E3 and in `publish_loss`; `refuse` is the only `rejected(...)` built from a `DeviceError` in `prepare`/`arm`; `refuse_open` and `open_refusal`'s direct `rejected(open_refusal(e))` callers are gone. The implementer runs the three greps again and puts the output in the report.

A loss that the open's own flush already published (core.rs:1319-1321, `device_failed` inside `tx_flush`) reaches `refuse` as an error whose loss is published: `device_lost` returns at once if lost (core.rs:1087, row N92-3), so the event stays one. Not a new test; each new lost test asserts the count is 1.

## 2. What the FakeDevice needs (smallest addition)

`FakeFault::Lost(t)` fails every call from an instant and, on the manual clock, needs a held clock to hit one call (and UR-33's list of the fake's faults, design/18 :379, names it, so F1 joins that list: §7); it cannot isolate "`rates` after a good open" or "tx open after a good rx open". The fake's one-shot non-lost refusals (`RxOpenFails`, `TxOpenFails(n)`, `RatesMismatch(n)`, `RxApplyFails`, `RefLockedFails`) have no lost twin. `LockSpy`'s scripted errors exist for `rx_stop` only (`refuse_rx_stop`, test_support.rs:432), are private to the lib, and the integration test for `prepare` (site 2) and any coordinator-level test cannot use them. Smallest addition that serves every site and both test levels:

**F1** `FakeFault::LostOnCall(&'static str)` (device.rs `FakeFault`, near `Lost`): the first call of the named `Device` method — one of `"apply"`, `"rates"`, `"ref_locked"`, `"rx_open"`, `"tx_open"` — fails with `DeviceError { lost: true, message: "fake: the device is gone (lost on <call>)" }`, and so does every later call that already checks `lost` (the link is gone, as `Lost(t)` models it). Implementation: an `AtomicBool` on `FakeDevice` read by `lost()` (device.rs:1157), set by a `trip(call)` placed after the `self.lost(&st)?` of those five methods; ~25 lines. No clock involvement, so no wall-clock oracle. The five methods are exactly the calls the sites make. VERIFIED by grep: `src/authority.rs` reads no `ref_locked` (it calls `set_sources` and `time_now` only), so `LostOnCall("ref_locked")` trips at `arm`, not at bind; nothing before `prepare` calls `apply`, `rates`, `rx_open` or `tx_open`.

**One fault, one rig**: the device stays gone after F1 trips, so every sequence below that makes more than one lost call (C2's three calls, C2' , A4's two directions, A5's cases, A2/A3 after their loss) builds a fresh rig (and fresh fake) per call; on a shared rig the second call would fail at its own open.

Rejected alternatives: (a) extend `LockSpy` with scripted one-shot errors for `rates`/`tx_open`/`ref_locked` (lib-only; `prepare` and the coordinator would still need a wrapper `Device` impl of ~90 lines, the eleventh hand-rolled copy in tests/fake.rs; two mechanisms); (b) `FakeFault::Lost(t)` + `apply_delay` + `set_time` (needs a held clock and an inference about virtual time at `prepare` entry). Recommended: F1.

**F-1 test** (device.rs, lib, `manual` helper): `ur_33_a_lost_on_call_fault_fails_that_call_and_every_call_after_it`: with `LostOnCall("rates")`, `rx_open` succeeds, the first `rates` fails `lost` with a message naming `rates`, a following `rx_open`, `apply` and `time_now` fail `lost`, and `calls()` holds no `rates`. Fails against: a fault that never trips; one that trips only once. Row 13.

## 3. Rigs

Arm rig (lib, mod.rs `tests`), following the #67 model test (`ur_29_a_loss_found_by_the_stop_on_open_at_arm_is_published_once_and_cleanup_closes_nothing`, mod.rs:1005): **one `LockSpy` is both the radio's device and the core's device** (`UhdRadio::from_binding(&binding, spy.clone())`, `test_support::rig_on_description(spy.clone(), …, radio.description.clone())`, `spy.watch(&core)`), so `ref_locked` and `cleanup`'s `close_streams` (both on `self.device`) and the opens, `rates` and `mark_lost` (on `core.device`) land in one `order()`. `prepared_rig` and `ur_25_a_refused_check_at_arm_leaves_the_set_unknown` use two fakes; "cleanup closes nothing" would pass vacuously there, so the new tests do not use them. The spy is built `LockSpy::new(FakeConfig { clock: Manual, faults: vec![…], ..default })`.

One helper `arm_rig(selector: (rx_links: bool, tx: bool, clock_source: &str), faults) -> (UhdRadio, Arc<LockSpy>, Arc<Core>, Arc<EventCollector>)` (the model test's ~15 inline lines, reused; the model test is moved onto it) and one assertion helper `assert_loss_published_at_arm(radio, spy, core, events, error: ModuleError, call: &str, message: &str)`:
1. `error.kind == Rejected` and `error.message == format!("UR-13: {message}")`;
2. **before `cleanup`**: `core.is_lost()`, `spy.seen("mark_lost") == 1` (published before arm returned: arm is one thread, so the state at return is the observable of "before the refusal");
3. `events.drain(..)` holds exactly one `DEVICE_LOST`, payload `message == message` (the device's own, no `UR-13:` prefix);
4. `radio.cleanup()` twice (idempotent): `spy.order()` after the lost `call` is exactly `["mark_lost"]` — no `close_streams`, no `rx_close`/`tx_close`, no second `mark_lost`.

A `ref_locked` call is not routed through `LockSpy::call` (test_support.rs:513), so it is absent from `order()`; its test asserts the whole `order()` is `["mark_lost"]`. Do not route it through `call` (it would add entries to other tests' orders).

## 4. Sequences, tests, rows

Test names are `ur_29_…` / `ur_13_…` so the design `*Test:*` lines can cite them. "Red" is what the test shows on the unfixed tree with only F1 and the helpers added; the implementer runs each new test on that tree first and records the symptom (a test that fails for another reason is rewritten).

### 4.1 Arm (lib, mod.rs `tests`, spy rig)

| # | Sequence | Test | Asserts (how observed) | Wrong behaviour caught | Red today |
|---|---|---|---|---|---|
| A1 | tx-only Run; `LostOnCall("tx_open")`; `arm`; `cleanup` ×2 | `ur_29_a_lost_transmit_open_at_arm_is_published_and_cleanup_closes_nothing` | helper; `order() == ["tx_open","mark_lost"]` | site 3 drops the loss (the issue's probe) | Rejected `UR-13`, no `DEVICE_LOST`, `order()` ends `close_streams` |
| A2 | Run with a receive link **and** a transmit stream (rx open first); `LostOnCall("tx_open")` | `ur_29_a_lost_transmit_open_after_the_receive_open_at_arm_is_published_once` | helper; `order() == ["rx_open","rx_stop","rates","tx_open","mark_lost"]` (receive open and its rate check completed, then the transmit loss, one event) | transmit side not routed; a receive-side close; a second event | `close_streams` after `tx_open`, no `mark_lost` |
| A3 | `clock_source: external`, links as A2; `LostOnCall("ref_locked")` | `ur_29_a_lost_reference_read_at_arm_is_published_before_any_open` | helper; whole `order() == ["mark_lost"]` (no open was tried) | site 4 drops the loss; `arm` goes on to open after it | `rx_open …` in `order()`, no `DEVICE_LOST` |
| A4 | open succeeds, the direction's first `rates` read is lost; for `Dir::Rx` (link) and `Dir::Tx` (tx-only), `LostOnCall("rates")` | `ur_29_a_lost_rate_read_at_arm_is_published_and_the_streamer_is_not_closed` | helper; `order()[after "rates"] == ["mark_lost"]` (so no `rx_close`/`tx_close`); after `cleanup`, `core.rates_checked(dir) == false` (see Q3) | site 1 collapses the loss; the streamer is closed on the lost device | `rx_close`/`tx_close` after `rates`; `close_streams`; no `DEVICE_LOST` |
| A5 | the same rig with a **non-lost** error at each site: (a) `RefLockedFails(Duration::ZERO)` with `external`; (b) `TxOpenFails(0)`; (c) `RatesMismatch(0)` on rx (and tx) | `ur_13_a_refusal_at_arm_that_is_not_a_loss_is_unchanged` (one test, three cases) | `kind == Rejected`; text exactly `UR-13: fake: the reference sensor read failed` / `UR-13: fake: scripted refusal of transmit open` / `UR-13: the rx rates the device reports are not the profile's rate set`; zero `DEVICE_LOST`; `seen("mark_lost") == 0`; after `cleanup` `close_streams` is in `order()` exactly once; (c) also `rx_close` after `rates` (the check-refused close stays) | a refusal that publishes a loss for a non-lost error; the close-after-refusal removed | green today (the unchanged-behaviour pin; it must stay green through the fix) |
| A6 | the #67 receive case (stop on open lost) | the existing model test, moved onto `arm_rig`/`assert_loss_published_at_arm` | helper | the rx site regresses to `rejected` | green today |

The non-lost read failure of `rates` at Session level is the existing `ur_13_a_rate_read_that_fails_refuses` (tests/fake.rs:5584; `RatesMisreported.failing`, a `DeviceError::failed`): it stays the non-lost twin of A4 and must stay green (it asserts `UR-13:`, `rates read failed`, `rx_open` before `close_streams`).

### 4.2 Core level (core.rs `tests`; t3.rs for the reopen), no publication: the caller's job

| # | Sequence | Test | Asserts | Wrong behaviour caught | Red today |
|---|---|---|---|---|---|
| C1 | `LostOnCall("rates")`: `check_rates(Rx,0)`; again; `core.device_lost("x")`; again | `ur_29_a_lost_rate_read_is_the_devices_own_error_and_is_not_cached` | 1st: `Err(e)` with `e.lost` and `e.message` the device's own (no `UR-13:`); `rates_checked(Rx) == false`; `spy.seen("rates") == 1`. 2nd: `Err` lost again and `seen("rates") == 2` (read again: nothing was cached as a refusal). 3rd: `Err` lost with the `UR-29: the device is lost before the rate comparison` text and `seen("rates")` still 2 (what a later open sees after the loss: no device call) | the loss cached as a permanent refusal (`UR-13:` String, non-lost, replayed at :1128); a published loss still reading the device | 1st returns non-lost `UR-13: …`; `rates_checked` true; 2nd returns the cached String with `seen == 1` |
| C2 | open ok, `rates` lost, nobody has published yet: `open_streamer(Rx,0)`; `open_streamer(Tx,0)`; `owner_configure(Rx,0,&planned)` (the owner's first open of a direction; `give_back` its claim) | `ur_29_an_open_whose_rate_read_is_lost_closes_no_streamer` | `Err` with `lost`; `spy.order().last() == "rates"` (no device call between the lost read and its publication: no `rx_close`/`tx_close`); the record is `DeviceSet::Unknown`, `configured None` (as for a refused comparison: `record_refused`); for `owner_configure` the returned `(error, claim)` has `error.lost` | close on a device the read found lost; the lost bit lost on the owner's path | `rx_close`/`tx_close` is the last call |
| C2' | the non-lost twin: `RatesMismatch(0)` on the same three calls | extends `ur_34_a_rate_refusal_leaves_the_stream_record_unknown` (core.rs ~:2303, which sets the cache by hand) with the real-read case, or a sibling | the streamer **is** closed (`rx_close` last) and the error is non-lost | the guard loosened to "never close" | green today |
| C3 | transmit reopen (the owner's UR-22 recovery): confirmed flush, then `LostOnCall("rates")`; `reopen_tx_streamer(0, first(1), i64::MIN)` (t3.rs: `LockSpy` rig with `clock_guard`, as the file's other spy tests) | `ur_29_a_reopen_whose_rate_read_is_lost_closes_no_streamer` | `Err` lost; `order()[after the last "rates"]` empty | `reopen_tx_streamer`'s copy of the close left behind | `tx_close` last |
| C3' | non-lost twin: `RatesMismatch(0)` (the existing t3.rs:571-577 case) | `ur_13_a_reopen_whose_comparison_refuses_closes_the_confirmed_streamer` (new; the existing test keeps its send count) | `tx_close` is the last call | row 13's killer | green today |

**T1** (only if Q9 is adopted; tx owner rig of tx.rs's `owner(1)`, `probe` at `tx_reopen_claimed`): `LostOnCall("tx_open")`, `tx.chain().mark_reopen_needed()`, the probe books `Provider::stop` (`streams.stop = Some(..)`) after the claim, `tx.pass()`: test `ur_29_a_lost_reopen_while_a_stop_is_booked_is_published`: one `DEVICE_LOST` (events), `spy`/device `mark_lost` once, and the calls after `tx_open` are exactly `["mark_lost"]`. Red today: no `DEVICE_LOST`, no `mark_lost`. Its non-lost twin (a refused reopen with a stop booked publishes nothing, `COMMAND_REJECTED` stays suppressed) is green today and keeps the guard's other half: `RxOpenFails`-style `TxOpenFails(n)` with the same probe.

The owner's publication is not re-tested: `refuse_segment` → `device_failed` publishes a lost error and is pinned by `ur_29_a_lost_receive_owner_configure_releases_stream_lock_before_reporting` (rx.rs:1163) and the tx reopen's handler (tx.rs:855-858); C1 and C2 make the owner's open hand them a lost error, which is the only new thing on that path.

### 4.3 Prepare (tests/fake.rs, `direct_prepare`, site 2)

`direct_prepare` discards its `EventCollector`. Add `direct_prepare_events(...) -> (Result<(), ModuleError>, Arc<EventCollector>)` and make `direct_prepare` return `.0` of it (3 lines, no copy).

| # | Sequence | Test | Asserts | Wrong behaviour caught | Red today |
|---|---|---|---|---|---|
| P1 | `fake(FakeConfig { faults: vec![LostOnCall("apply")], serial: "n101-prepare-lost-apply".. })`, the radio bound to it, `direct_prepare_events` with `("radio.rx.c0.stream", 0)` (RM-30: a stream needs a channel); then `radio.cleanup()` ×2 | `ur_29_a_lost_apply_in_prepare_is_published_and_refused_with_the_devices_text` | `kind == Rejected`, `message == "fake: the device is gone (lost on apply)"` (unchanged: no `UR-13:` prefix, as today's `e.message`); one `DEVICE_LOST` in the collector with that message; `device.calls()` holds `mark_lost` once and no `close_streams`; `cleanup` adds nothing. Not asserted: a `timing` row or `write_sections` (see below) | site 2 drops the loss | no `DEVICE_LOST`, no `mark_lost` |
| P2 | the non-lost twin: `FakeFault::RxApplyFails`, same constraints | `ur_12_a_refused_apply_in_prepare_that_is_not_a_loss_publishes_nothing` | `Rejected`, `message == "fake: scripted refusal of the first receive apply"`, no `DEVICE_LOST`, no `mark_lost` | publishing on every configure error | green today |

After a refused `prepare`, `self.prepared` is still `None`, so `cleanup` returns at `prepared.take()` (mod.rs:744) with no `close_streams` and no `write_sections` whether or not the loss was published. What publication adds at `prepare` is the event and `mark_lost` (F4: `UhdDevice` then frees nothing on a dead link). The existing read-back refusals (`ur_12_a_layout_a_rate_the_device_does_not_apply_is_refused`, `ur_12_a_layout_an_antenna_read_back_that_differs_is_refused`) are `DeviceError::failed` from `configure` itself and stay green. A loss published by another thread between two channels of one configure (`UR-29: … before the next channel's apply`, core.rs:1183) reaches `refuse` already published: once (idempotence, row N92-3), pinned below `prepare` by `ur_29_a_loss_between_two_channels_of_a_configure_applies_no_more` (rx.rs:2677); no `prepare`-level copy (the `probe` hook is lib-only).

### 4.4 Coordinator level: what a user sees (tests/fake.rs, `spec_run_ended`)

Nothing read shows what the Manifest holds when `DEVICE_LOST` is published during `prepare` or `arm` and the stage then refuses (INFERRED: a `Failed { stage }` Manifest; whether it carries the event is unknown). So:

`ur_29_a_loss_at_prepare_or_arm_ends_the_run_as_a_lost_receive_open_does` — one test over a table `[("rx_open", receive spec), ("tx_open", `with_tx(receive_spec(..))`, as `ur_13_arm_refuses_a_rate_set_the_description_does_not_have` builds its transmit case: this is the receive-link-first case; A1 keeps transmit-only), ("rates", receive), ("ref_locked", receive, `clock_source: external`), ("apply", receive; stage Prepare)]`, each `fake(FakeConfig { faults: vec![LostOnCall(call)], serial: unique })`. **Baseline first**: the `rx_open` row is the #67 behaviour (the receive open is published today) and must pass on the unfixed tree; the implementer records its observed `(Termination::Failed { stage }, DEVICE_LOST count in the Manifest, count of mark_lost and close_streams in inner.calls(), the failure text prefix)`, and every other row must equal that tuple except the stage and text (the apply row's stage is Prepare). If the Manifest of a refused stage carries no events, the event column is dropped from all rows (the lib tests carry the event). Wrong behaviour caught: a site that ends the Run differently from the receive open (no `mark_lost`, `close_streams` on a gone device). Red today: `tx_open`, `rates`, `ref_locked`, `apply` rows (`close_streams` present, no `mark_lost`).

### 4.5 Classification (src/uhd.rs, feature `uhd` only)

`UHD_ERROR_EXCEPT` (47) is lost only at `uhd_usrp_get_rx_stream` and `uhd_usrp_get_tx_stream`. Both go through `check_rx`/`check_tx` with `streaming = true` (uhd.rs:981, 1074), so **the mutant to kill is "47 is lost on every streaming call"**, and the negative cases must use `streaming = true` calls: `get_time_now` goes through `check_usrp` with `streaming = false` and discriminates nothing.

- **K1** `uhd_47_is_lost_only_at_the_two_stream_opens` (pure, calls `check_with` directly; a table of `(call, code, streaming) -> lost`): `("uhd_usrp_get_tx_stream", 47, true)` lost; `("uhd_usrp_get_rx_stream", 47, true)` lost; `("uhd_rx_streamer_max_num_samps", 47, true)` **not** lost (same closure as the open, `streaming = true`); `("uhd_tx_streamer_send", 47, true)`, `("uhd_tx_streamer_recv_async_msg", 47, true)`, `("uhd_rx_streamer_recv", 47, true)` not lost; `("uhd_usrp_get_time_now", 47, false)` not lost; unchanged: `44` with `streaming = true` lost at every one of those calls and not lost with `false`; `30`/`21`/`31` lost anywhere; `0` is `Ok`. Fails against: 47 lost everywhere (the broad mutant); 47 lost at only one of the two opens; RUNTIME narrowed away.
- **K2** `uhd_47_at_a_stream_open_is_a_lost_device` (shim): the shim's `uhd_usrp_get_rx_stream` stub is driven by `OPEN_STREAM_FAILS: bool` and its `uhd_usrp_get_tx_stream` stub returns `UHD_ERROR_RUNTIME` unconditionally (uhd.rs:1590-1599). The rx stub keeps its success default, and the tx stub its RUNTIME default (`uhd_close_drops_its_map_reference_but_an_in_flight_arc_keeps_the_streamer` and `uhd_stream_close_and_open_compete_under_the_control_mutex` unwrap `rx_open`; many tests want a failing tx open). Both read one settable code `OPEN_STREAM_CODE: AtomicI32` that is `None`-like (a sentinel such as `-1`) by default, in which case the rx stub answers `UHD_ERROR_NONE` and the tx stub `UHD_ERROR_RUNTIME` as now; a test sets it to 47, 44 or 21 under `TEST_LOCK` and resets it after. `uhd_a_failed_streamer_open_reads_its_text_from_the_streamer` (:1606) keeps `OPEN_STREAM_FAILS` for rx (so it is not touched) and its assertions. K2: with 47, `rx_open` and `tx_open` return `Err` with `lost == true` and `message` starting `uhd_usrp_get_rx_stream: UHD error 47: ` / `uhd_usrp_get_tx_stream: …`; with 44 still lost (unchanged); with 21 lost. Wiring guard: a misspelled call literal in `check_with` passes K1 only if K1 uses the same literals, so K2 goes through the real `rx_open`/`tx_open` that name them.
- Gate coverage: the gate's `uhd-test` step runs `cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api` (gate.py:334), so K1/K2 gate the change **as tests**; they have **no mutation rows** (the rules in `uhd.rs` are outside the mutation gate: the mutation image has no libuhd; handoff §3 "feature `uhd` の mutation"). Replacement evidence: a red run on the unfixed tree (`rdev.py -- cargo test -p ezsdr-radio-uhd --features uhd --lib uhd_47` (matches K1 and K2) fails on the 47 cases) and a green run after, both recorded in the report; never a mutant applied to the tree.
- `tests/uhd_api.rs` (real libuhd, no device) and `streamer_lifecycle` (uhd.rs:407) make no `get_*_stream` call: unaffected.

## 5. Mutation rows (new)

IDs `N101-nn`, in `plan/maintenance/tools/mutations.json`, written by the implementer with the code, run only by `remote_mutate.py` (rows go in only with the code, not now). `crate` `ezsdr-radio-uhd`; `test_bin` `lib` unless `fake`; `filter` is one test name. `old` is the planned text of §0, `new` the pre-fix behaviour.

| Row | file | old → new | killer (test_bin) |
|---|---|---|---|
| 01 | mod.rs | `.map_err(|e| refuse(&core, e, |e| e.message))?` → `.map_err(|e| rejected(e.message))?` (prepare) | `ur_29_a_lost_apply_in_prepare_is_published_and_refused_with_the_devices_text` (fake) |
| 02 | mod.rs | `Err(error) => return Err(refuse(&core, error, |e| format!("UR-13: {e}"))),` → `Err(error) => return Err(rejected(format!("UR-13: {error}"))),` | `ur_29_a_lost_reference_read_at_arm_is_published_before_any_open` |
| 03 | mod.rs | `core.open_streamer(Dir::Tx, stream.k, stream.channels).map_err(|e| refuse(&core, e, open_refusal))?;` → `….map_err(|e| rejected(open_refusal(e)))?;` | `ur_29_a_lost_transmit_open_at_arm_is_published_and_cleanup_closes_nothing` |
| 04 | mod.rs | the same for `Dir::Rx` | the model test (A6) |
| 05 | mod.rs | `fn refuse(…) -> ModuleError { core.publish_loss(&error);` → `… {` (call removed) | the model test (A6) |
| 06 | core.rs | `pub fn publish_loss(&self, error: &DeviceError) -> bool { if error.lost {` → `… if error.lost && false {` (retargets N67F-29) | the model test (A6) |
| 07 | core.rs | `if !self.publish_loss(error) {` → `if true {` in `device_failed` | `ur_29_a_lost_receive_owner_configure_releases_stream_lock_before_reporting` (killer to be confirmed by the runner; if it survives, a sibling in rx.rs/tx.rs that asserts the `DEVICE_LOST` count) |
| 08 | mod.rs | `core.publish_loss(&error);` in `start` removed (retargets P476) | `ur_29_a_lost_later_start_is_published_before_the_owners_are_stopped` (fake) |
| 09 | core.rs | `Err(error) if error.lost => return Err(error),` → `Err(error) if false => return Err(error),` | C1, A4 |
| 10 | core.rs | `matches!(checked, Err(error) if !error.lost)` → `checked.is_err()` | C2, A4 |
| 11 | core.rs | the `self.close_if_refused(dir, k, &checked);` call in `open_streamer` removed (retargets P500) | A5(c) rx case, `ur_13_arm_refuses_a_pre_chain_rate_set_after_the_open` (fake) |
| 12 | core.rs | the same call in `reopen_tx_streamer` removed | C3' |
| 13 | device.rs | `LostOnCall` trip never set (`self.gone.store(true, …)` removed) | F-1 |
| 14 | tx.rs | `if error.lost || (!core.is_lost() && lock(&core.streams).stop.is_none()) {` → `if !core.is_lost() && lock(&core.streams).stop.is_none() {` (only if Q9 is adopted) | T1 |

Existing rows that the change breaks (anchors move; the implementer re-anchors each so `rules.py` finds it once and the mutant is the same program as before):
- **T42** (mod.rs `core.open_streamer(Dir::Tx, …).map_err(|e| rejected(open_refusal(e)))?;`, killer `ur_13_arm_radiates_nothing`): re-anchor to the `refuse` line; `new` stays "append the `tx_send` after it".
- **P372** (mod.rs `…Dir::Rx… refuse_open(&core, e))?;`, killer `ur_25_a_receive_session_follows_its_recorded_plan`): re-anchor to the `refuse` line.
- **N67F-29** (the body of `refuse_open`, killer the model test): retarget as row 06 (and 05).
- **P476** (mod.rs `if error.lost { core.device_lost(&error.message); }` in `start`): retarget as row 08.
- **P500** (core.rs: the three-line `if opened.is_ok() && checked.is_err() && !self.is_lost() { self.close_device(dir, k); }` plus the `let checked` line, killer `ur_13_arm_refuses_a_pre_chain_rate_set_after_the_open`): retarget as row 11.
- **P296** (`let checked = if opened.is_ok() { … }` line): stays if that line is not rewritten; otherwise re-anchor.
- **P348, P334, P344, P345** (`check_rates`' `let outcome = match self.device.rates(dir, chan) {`, the `check_opened` body): unchanged by this plan; verify with `rules.py`.
- **P514, P331** (`rates` cache by hand, `rates_checked` insert): unchanged because the cache type is kept (Q3).
`rules.py` must pass with `--base 89dc141e`.

## 6. Hardware tests and rehearsals: audit (AGENTS §9)

Greps: `grep -n "UR-13\|\.lost\|lost:\|arm()\|Stage::Arm\|rates" tests/hardware.rs tests/common/*.rs`; `grep -rn "get_rx_stream\|get_tx_stream\|error 47\|EXCEPT" tests src/device.rs`. No `#[ignore]` hardware test asserts a value derived from these rules, so none needs a new FakeDevice rehearsal; K1/K2 are the rehearsal of the classification rule (the fake does not run `check_with`).

| Test | Expectation | Verdict |
|---|---|---|
| `hw_b9_unplug`, `hw_b9_unplug_transmit_only`, `hw_b9_unplug_and_reopen` (hardware.rs:197/216/233) | print-only: the Run ends `DEVICE_LOST` after an unplug during a Run | still right. If the cable is pulled during `prepare`/`arm` the outcome is now `Failed { prepare|arm }` plus a published loss instead of a plain refusal; none of the three asserts it |
| `hw_b9_long_receive`, `hw_b9_transmit_only` (hardware.rs:1192-1221) | `DEVICE_LOST` empty on a healthy device | still right, **INFERRED**: a healthy device returns 47 at `get_*_stream` only for the held-port refusal, which the Module never makes (fixed args) and which aborts the process on hardware (bench-results.md:73); nothing on the bench has shown a 47 there on a healthy link |
| `ur_13_a_rate_read_that_fails_refuses` (fake.rs:5584) | non-lost read failure: `UR-13:`, streamer opened then closed | still right; it is A4's non-lost twin |
| `ur_13_arm_refuses_a_rate_set_the_description_does_not_have`, `ur_13_arm_refuses_a_pre_chain_rate_set_after_the_open`, `ur_13_arm_cases` (unlocked reference) | non-lost mismatch/unlocked refusals, close after refusal | still right |
| `ur_34_a_rate_refusal_leaves_the_stream_record_unknown` (core.rs ~:2303) | cache seeded by hand with `Some(Err(String))` | still right (cache type kept) |
| `ur_25_a_refused_check_at_arm_leaves_the_set_unknown`, `ur_36_…_even_when_its_check_refuses` | non-lost refused comparison keeps the open and records `Unknown` | still right |
| t3.rs:566-577 (`RatesMismatch` on a reopen) | `UR-13`, two sends | still right |
| `ur_29_a_dead_link_that_fails_without_lost_is_a_lost_device`, `ur_29_a_failing_reference_sensor_alone_is_not_a_lost_device`, `ur_29_time_reads_that_fail_apart_are_not_a_lost_device` | uhd-control's reads (`time_now`, `ref_locked`), not the opens | still right: `check_with`'s change is limited to the two open calls |
| `ur_33_one_channel_is_opened_by_one_streamer` (fake.rs:2761), device.rs:2480-2511 | the fake's "reconnect port" refusals (code 47) are `DeviceError::failed` and set `aborted` | still right; **divergence accepted** (Q6): `UhdDevice` would now call the same 47 lost, but on hardware that open aborts the process, and the fake panics on drop unless a test takes the abort, so no Run outlives it |
| `uhd_a_failed_streamer_open_reads_its_text_from_the_streamer` (uhd.rs:1606) | text `UHD error 44: …` | still right (sets the code to 44 explicitly) |
| `probe_device.rs:276/295` (`lost` recorded in bench evidence) | time reads and receives only | unaffected |
| `tests/uhd_api.rs` | real libuhd, no device | unaffected |
| Fake with `FakeFault::Unreachable` | `rx_open`, `tx_open`, `rates` succeed on a dead link; the hardware's open would return 47 (now lost) | **cannot tell / gap accepted** (Q6): no test models a dead link at the opens; K1/K2 and the F1 sites cover the rule |

## 7. Design and records that change (the `design/` text is reviewed as the implementing diff; `rules.py --base 89dc141e` must pass; DA-1: normative text only, rationale goes in the Changes row)

- `design/18-uhd-radio.md` **UR-29** (:341): the lost list becomes "… the stop on open included, and a `DeviceError` with `lost: true` from an open's `rates` read, from a `ref_locked` read at `arm`, from the receive or transmit open at `arm` and from `prepare`'s configure: each is published before `arm` or `prepare` refuses, so `cleanup` makes no device call; a refusal that is not a loss publishes nothing". The parenthetical "(a dead link's control requests time out in UHD as `op_timeout`, which the C API returns as `UHD_ERROR_EXCEPT`, not counted lost: …)" gains "except at the two stream opens". The sentence at :343 "`UhdDevice` sets `lost` for IO, USB and OS, and for RUNTIME on a streaming call only" becomes "… for RUNTIME on a streaming call and for RUNTIME or `UHD_ERROR_EXCEPT` at `uhd_usrp_get_rx_stream` and `uhd_usrp_get_tx_stream`". Its `*Test:*` line (:344-345) gains A1-A4, C1-C3, P1 and the §4.4 test; K1/K2 get a second clause "(feature `uhd`)" beside `uhd_tx_async_reports_a_failed_call_as_an_error`.
- **UR-33** (:379): the fault list "a device loss (every later call fails with `lost: true`)" gains "or at a named call (`LostOnCall`: the first `apply`, `rates`, `ref_locked`, `rx_open` or `tx_open`, and every call after it)"; its `*Test:*` gains F-1.
- **UR-13** (:214): "refusing a mismatch or a read that fails with `UR-13:`" gains "a read whose error is the device's loss is published (UR-29) and refused with the same text, and holds no outcome"; adds "a `ref_locked` read that fails refuses `UR-13:` (a loss published first, UR-29)" (the text did not state it); "but never closes a lost device" gains "nor one whose comparison read found it lost". Its `*Test:*` (:215) gains A5.
- **UR-12** (:207, the configure paragraph): "a configure the device fails with its loss is published (UR-29) before `prepare` refuses with the error's own text"; `*Test:*` (:212) gains P1, P2.
- **§5 U9** (:448): "IO, USB, OS and RUNTIME errors on a streaming call, and `UHD_ERROR_EXCEPT` at the two stream opens, and 1 s of silence while streaming".
- **§2 INFERRED line** (:30) "which UHD errors mean a lost device (UR-29)": no edit needed; still true.
- **§6 tests table**: one row each for A1-A4, P1 and the §4.4 test (rows at :502-:640 are the model).
- **Changes table** (:703+): one row (date, UR-12/UR-13/UR-29/U9, the change, record = #101 and its decision file); the rationale (the source check: `error_c.cpp:20-35`, `usrp_c.cpp:257-288`, bench-results.md:73/1446/1535; the design-note answer) lives there, not in the rules.
- `plan/maintenance/failure-mechanisms.md` row `mech:failure-collapsed-into-value` (the orchestrator's file): ninth bug and its answer ("one conversion: `publish_loss`, one `refuse`, `close_if_refused`"). Read-only for the implementer.
- `KNOWN_LIMITATIONS.md`: grep (`-i "lost\|DEVICE_LOST\|UR-29\|UR-13"`, and `arm` with `refus`) finds no entry about a refusal at `arm` for a lost device. **KL-2** (line 24-29, "the link to the X300 goes down during a Run") is the nearest. Q7.

## 8. Order for the implementer (red first), and what runs where

1. Add F1, the `arm_rig`/`assert_loss_published_at_arm` helpers, `direct_prepare_events`, and the new tests; run them on the unfixed tree: A5/A6/C2'/C3'/P2/§4.4's `rx_open` row green; the others red with the symptoms of §4. Record the red output.
2. Make the code change of §0. Ordinary tests may run locally; the Python suites, the full mutation lists and the gate go to the orchestrator.
3. `rdev.py -- cargo test -p ezsdr-radio-uhd --features uhd --lib uhd_47` red before, green after (§4.5).
4. Rows N101-01..13 and the re-anchored rows into `mutations.json`; `rules.py --base 89dc141e`; rows run through `remote_mutate.py` only (touched rows now; the full lists at the sub-step commit, as AGENTS §7).
5. `cargo fmt --all` before reporting. No version bump, no `SCHEMA_CHANGELOG` entry. No test judges by wall-clock time: every new test is call-order or event-count on the manual clock, or pure.

## 9. Open questions (each with a recommendation)

- **Q1 F1 (`FakeFault::LostOnCall`) or spy-only scripted errors.** Recommend F1 (§2): one mechanism serves the lib, `prepare` and the coordinator; ~25 lines.
- **Q3 `rates_checked` for a lost read (Manifest content, so the owner's).** Recommend `false` (nothing was compared; the Manifest also carries the loss) by not caching a lost outcome; alternative: cache `Result<(), DeviceError>` and report `true`. Today's value, `true`, is the collapsed outcome (a cached refusal). A4 asserts the chosen value.
- **Q2 Scope of "one function".** Recommend `Core::publish_loss` with three callers (`refuse`, `device_failed`'s lost branch, `start`), so that no second copy of "if lost then `device_lost`" is left. This touches two existing, tested sites without changing their behaviour. Alternative: `refuse` alone, leaving `device_failed` and `start` as two copies of the decision.
- **Q4 Classification by call name inside `check_with`.** Recommend the string match (two literals already in the error prefix tests assert) pinned by K2's wiring; alternative: a code-class parameter replacing `streaming: bool` at ~20 call sites. A `shortcut:` comment names the limit (a renamed call literal).
- **Q5 §4.4's event assertion** depends on the Manifest of a refused stage carrying events (INFERRED). Recommend baseline first, as written; the lib tests carry the event either way.
- **Q6 Fake/hardware divergences** (the fake's two reconnect-47 refusals are `failed`, and `Unreachable` does not touch the opens). Recommend leaving both: the first aborts the process on hardware, the second would restate the decision. Record them in the report and handoff as accepted gaps.
- **Q7 KL-2.** Recommend no edit now. If §4.4 shows a user-visible change (a `DEVICE_LOST` event in a `Failed { arm }` Manifest), propose one sentence to KL-2's "When" (also during `connect`); owner decides (KNOWN_LIMITATIONS is the owner's).
- **Q9 The transmit owner's reopen with a stop booked (site 6b).** Recommend adopting the one-condition fix and T1: a lost error is published whenever it is the reopen's outcome, the booked stop only suppressing the non-lost report. It is an owner's open (decision 6 names those), reachable today with `tx_open` lost and a `Provider::stop` booked between the claim and the result, and it ends in `close_streams` on a gone device. Alternative: leave it and record it as a known gap (uhd-control's 500 ms read finds the loss later, UR-29), which keeps decision 6 not literally true.
- **Q8 Publishing at `open_streamer` itself.** Rejected: owners call it with the booking of the refusal still to be made before the loss is published (`owner_configure` contract, UR-25/RM-16), so the loss must stay the caller's.

INFERRED, not verified here: (i) what a `Failed { prepare | arm }` Manifest carries (§4.4); (ii) that `DEVICE_LOST` at `prepare` is observable to a user beyond `mark_lost` and the collector (`cleanup` is a no-op with no `prepared`); (iii) that the `uhd-test` step builds `--lib` tests with the shim stubs as read (uhd.rs:1560-1630); (iv) the killer of row 07; (v) the bench never showing 47 at `get_*_stream` on a healthy link (§6).
