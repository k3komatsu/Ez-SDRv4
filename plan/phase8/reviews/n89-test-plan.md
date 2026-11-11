# #89 test plan — a client that goes away ends its Session, also while a request waits

Implementer's test plan (AGENTS §9 step 2). Base `main` = `8dd1016`, branch `work/n89`. No code is written yet.
Design: `sketch.md` (Owner decisions 1–10 override), `fable-design.md` (F1–F7, N1–N5, A2 adopted; A1, A3 deferred).
Every `path:line` is at `8dd1016`. VERIFIED = I read the code; INFERRED = reasoning.

## Changes after Sol's review

Sol's review (`sol-plan.md`, FAIL, 2 majors) and the orchestrator's dispositions (`plan-dispositions.md`, all adopted) and answers (`plan-adjudication.md`, Q1–Q4) are applied in the sections below. This list is the summary; each item's full row is in its sub-step.

| # | Finding | Test (file) | Asserts | Observes by | Rows it kills |
|---|---|---|---|---|---|
| 1 | Major 1: X1 never showed that the repeat was transmitted before the departure | **X1 changed**, `ea_15_an_input_that_ends_during_an_advance_ends_the_session` (`protocol.rs`) | Unchanged assertions, plus a precondition: before `advance` is written, `device.calls()` holds a `tx_send … n=<k>` with `k > 0` (`device.rs:1857`). Then, after it, `tx_send … eob=true` and `close_streams`. | It polls `device.calls()` (bounded, 10 s) for a positive-payload send; the input is closed only after that. | C89-2-04 (EOF) |
| 2 | Major 2: N1 (the device's final drop on the input thread) had no concurrency test | **W2 new**, `ea_15_a_removal_waits_for_a_fire_in_progress_so_the_owner_drops_last` (server `lib` unit test) | While `fire` is held inside a signal (a gated test closure that holds a clone of an `Arc<Token>`), a removal on an "owner" thread blocks on the watch. Once the gate opens, the owner's removal returns after `fire`'s closure has returned. The owner then drops its `Arc<Token>`, and `Token`'s `Drop` runs on the owner thread, not the firing thread. | A narrow `#[cfg(test)]` hook: the watch's removal and clear take `try_lock` first, and count a contended acquisition in `contended: AtomicUsize` before blocking (precedent: `ClockMutex`'s `#[cfg(test)] waiting`, `crates/ezsdr-radio-uhd/src/host.rs`). The test waits (bounded) for `contended == 1`, which is positive proof that the owner is blocked behind the fire, then opens the gate. `Token::drop` records `std::thread::current().name()`. | C89-2-11 (`fire` calls its signals outside the mutex): the `try_lock` succeeds, so `contended` never reaches 1 and the bounded wait fails. |
| 2 | Major 2: the four removal sites | **W3 new**, `ea_15_every_end_of_a_run_removes_its_signal_from_the_watch` (server `lib` unit test) | A `Server` built with a watch (crate-private constructor `Server::watched(config, Arc<Watch>)`, the narrow hook). `#[cfg(test)] Watch::len()`: 1 after `connect`, still 1 after `run_child` returns (the child's signal removed at the end of `drive`), 0 after `handle(Finish)`. In a second Server, 0 after `disconnect()`. | Direct reads of `Watch::len()` between calls. | C89-2-12 (`finish` keeps the signal), C89-2-13 (`disconnect` keeps it), C89-2-15 (the end of `drive` does not remove the child's). If `finish` and `disconnect` share one removal helper, C89-2-12 and C89-2-13 become one row on that helper. |
| 2 + minor "connect" | Connect registers before its T0 advance, and its ended path unregisters | **W4 new**, `ea_15_a_connect_after_the_input_ended_ends_on_its_way_to_t0_and_leaves_no_signal` (server `lib` unit test) | The watch is fired first; then `hello` and `connect` (the default Simulation profile) are handled. The reply is the error `ended`, with `termination` `Stopped { client_disconnect }`, and `exit` set (the EA-10 path, `lib.rs:235-239`). `manifest.json` in the Session dir says `client_disconnect`. `Watch::len() == 0` afterwards. | The pre-fired watch is the gate: an `add` on a fired watch calls the signal at once (W1), so only a registration made **before** `advance_to(t0)` can end the Run on its way to T0. | C89-2-06 (no registration at connect), C89-2-14 (connect's ended path keeps the signal), C89-2-16 (registration moved after `advance_to(t0)`: the reply becomes `connected`). |
| 3 | Minor: `finish` first, then a signal during its cleanup | **K6 new**, `kc_36_a_disconnect_signal_during_finish_s_cleanup_does_not_change_its_end` (`device_paced.rs`) | A worker runs `run.finish()`. The test waits for `radio:stop` (cleanup has begun; `finish` requested `client`), calls `signal()`, then opens the stop gate. The returned Manifest: reason `Stopped { client }`, `also` empty, transitions contain `Stopping { Orderly }` only. | New support `ThreadedProvider::gated_stop(gate)`: `stop` records `stop` and then waits for the gate (same form as `hold_after_take`). The probe line orders the signal after `finish`'s request. | C89-1-03 (an `Abort` request escalates the orderly cleanup, so `also` gains an entry and the mode changes). |
| 4 | Minor: read errors | **X7 new**, `ea_15_a_read_error_during_an_advance_ends_the_session` (`protocol.rs`) | `serve_watched` over a test reader fed by a channel: `Ok(bytes)` for each frame, then on command `Err(io::ErrorKind::Other)`. Simulation with `rx0()`: `hello`, `connect`, `submit(capture(1_000))`, `advance { by: 10^6 s }`. Wait for the `*.sigmf-meta`, then send the error. One more reply, `ended` with `client_disconnect`. The serve thread returns `Ok(Exit::EndOfInput)`. The Manifest says `client_disconnect`. | The capture file proves the advance is served; the reader's error is the only departure. | C89-2-17 (the input thread fires on EOF only; a read error ends the forwarding without firing): the advance is never cut and the bounded read fails. C89-2-04 is now EOF-only and keeps X1 as its killer. |
| 5 | Minor: Python, interruption while the reply body is read | **P6 new**, `Departure.test_ea_16_an_interruption_while_the_reply_body_is_read_abandons` (`test_easy_api.py`) | After a capture, its artifact URI is taken from `sdr.events()` (`sink.CAPTURE_WRITTEN`). A stdout wrapper raises `KeyboardInterrupt` on its first `read` (the body read, `_read_exactly`, `_client.py:114-122`). `sdr.read(uri)` raises `KeyboardInterrupt`. The real stdin and stdout are then `closed`, and `sdr.close()` returns a Manifest with `client_disconnect`. | The wrapper; the server is idle after its full reply, so the abandon's EOF is the departure. | C89-3-12 (the abandoned region ends after `readline`, so the body read is outside it): the pipes stay open. |
| 5 | Minor: Python, the 30 s kill fallback | **P7 new**, `DepartureClose.test_ea_16_a_server_that_does_not_exit_is_killed_and_close_keeps_no_manifest` | A `Connection` built with `object.__new__`, with `_process` a Mock whose `wait.side_effect = [TimeoutExpired("ezsdr-server", 30), -9]`, and with `abandoned = True`. A Session over it, with an empty temporary `dir`. `sdr.close()` returns `None` and does not raise. `_process.kill` was called once, `wait` twice, and `stdout.close` once. | Mock calls; no real 30 s wait. | C89-3-13 (`kill()` dropped from the `TimeoutExpired` branch). |
| 6 | C89-3-06 is equivalent | — | The production code has no separate `abandoned` check in `call`: the abandon closes stdin, and today's `stdin.closed` refusal (`_client.py:79-81`) refuses every later call. P1's "a later call raises `ProtocolError`" pins that existing refusal. | — | **Row C89-3-06 dropped.** |
| 7 | Bench: no evidence that the request was entered | Tooling (no test verdict) | See §S3 Tooling: the wrapper records the client-to-server bytes with times, and a run without the evidence reads `INCONCLUSIVE`. | — | — |
| 8 | EA-15: exit 0 for `BrokenPipe` only | Text | X6 already asserts it with a `BrokenPipe`; every other write error keeps exit 1 and its message. | — | C89-2-09 (unchanged). |
| 9 | EA-16 and KL-43 wording | Text | The distinction "an earlier abandoned call" vs "`close()`'s own `finish` failed", plus: a missing or partial Manifest, the 30 s kill, inherited pipes (F5a), a hung server (F4). | — | — |
| 10 | Mocks: no production `abandoned is True` | Changed helpers (not tests) | Production tests `abandoned` by truthiness. `DurationRequests.session` (`test_easy_api.py:1255`) and `ConnectRequests.frame`/`refused` (`:1107-1121`) set `connection.abandoned = False` (and `connection.return_value.abandoned = False`) explicitly. P3a–c and P7 set it themselves. | — | — |
| Q1–Q4 | Answers | §S3 Tooling (Q1: `EZSDR_DROP_CUT=link|kill`); §S3 close rules (Q2); §S2 dropped tests (Q3); §0 bounds (Q4) | — | — | Rows killed only by a bound are listed in §Counts. |

### Changes after the re-check

All items of the Opus re-check (`plan-dispositions.md`, last section) and of `plan-adjudication.md` "After Sol's review" are applied below. One bounds set is used throughout: **10 s** for every Rust bounded wait, and the **60 s** faulthandler guard in Python (§0).

| Item | Test | Asserts | Observes by | Rows |
|---|---|---|---|---|
| X1's payload poll | **X1** | It waits for a **timed** payload send, `tx_send 0 n=<k> at=<digits>` with `k > 0`. The 4-zero untimed flush every transmit stream sends when it opens (`crates/ezsdr-radio-uhd/src/provider/core.rs:1205-1215`, `at=-`) does not count. The `eob=true` send must be at a strictly later index in `calls()`, and `close_streams` later still. | `device.calls()` polled (10 s), then indices compared. | C89-2-04 |
| X1's race (**chosen: soften the claim**, not retry) | **X1** | Its assertions hold in both orders: the advance served and cut (`ended` reply), or skipped by A2 (no reply). The failure message names the branch that ran. | — | — |
| W2 robustness | **W2** | It waits for `contended >= 1`, not `== 1`. The gate is an `mpsc` channel: the gated closure blocks in `recv()`, and the test's sender drops on unwind, so a failing test never leaves the closure blocked. The owner removes through the **same helper the sites use** (the one that removes under the lock and hands back the `Live`, adjudication Q2), not a bare `Watch::remove`. | `contended` (`#[cfg(test)]` only, see §S2 hooks) | C89-2-11 |
| W5 (new): the order at `finish` | **W5** `ea_15_finish_removes_the_session_s_signal_before_it_finishes_the_run` (server `lib` unit test) | A gated test signal G is registered in the watch **before** `connect`. Then `hello` and `connect` (default Simulation) register the Session's signal after G. Thread F runs `watch.fire()`, which calls G first, and G blocks. The owner thread runs `handle(Finish)`. The test waits for `contended >= 1`, then opens G's gate. Expected: the reply is `finished` with reason `Stopped { client_disconnect }`. | The fire, released only after `finish`'s removal is blocked behind it, calls the Session's signal while the Run is still live. That happens only if the removal comes before the finish. A finish-then-remove order instead ends the Run first, and the late signal is a no-op, so the reason is `client`. | **C89-2-18** (the helper reversed at `finish`: finish, then remove). The order at `disconnect` and on `connect`'s ended path stays with review: there the reason is `client_disconnect` either way. |
| `drive`'s early returns (`lib.rs:309-316`) | **W3 extended** | The child's signal is added only after the two early returns (no `start_instant`; a duration that does not fit). W3 adds a child that fails before T0 (a `requires` of `radio.rx.s0.sample_rate_hz` `eq 1e12`, refused at validate, so the child has no `start_instant`). The reply is `ran` with the child `Failed`, and `Watch::len() == 1` afterwards. | `#[cfg(test)] Watch::len()` | **C89-2-19** (the add moved before the early returns) |
| X7: `Interrupted` | **X7 extended** | Before the `submit`, the controlled reader returns `Err(io::ErrorKind::Interrupted)` once. The `submit` is still answered `Admitted`, and the advance is served and later cut by the real read error, as before. The input thread retries `Interrupted` and does not treat it as a departure. | The reply to the `submit` after the injected `Interrupted`. | **C89-2-20** (`Interrupted` treated as an end of input): the `submit` frame is skipped by A2 and its reply never comes. |
| P6's `read` mode | **P6** | `_Interrupting` gets `on="read"`: the first `read` raises `KeyboardInterrupt`. | — | C89-3-12 |
| One bounds set | §0, the tests, §Counts | 10 s for every Rust wait or timeout, K2 included; 60 s Python guard. | — | — |
| C89-3-02 | §Counts | It is killed by P1's assertion that the real stdin is `closed`, not by a bound. | — | — |
| Adjudication: the hook | §S2 | `contended` and `len()` exist only under `#[cfg(test)]`. In production the removal path is a plain `lock()`; under `#[cfg(test)]` it tries `try_lock` first and counts a contended acquisition. | — | — |
| Adjudication: the helper | §S2 | One helper removes the signal under the watch's lock, then hands back the `Live` (or the child) to finish. `disconnect`, `finish`, `connect`'s ended path and the end of `drive` all use it. W5 now tests the order at `finish` (this revises Q2). | — | — |

---

## 0. Ground rules for every test below

- **No verdict depends on the wall clock.** A test waits for an observable state (a probe line, a park on the test Authority, a file in the Session directory) with a bounded wait whose expiry is the failure. Then it asserts causes, reply kinds and Run-time instants.
- **A hang fails the test.** That is what kills the "nothing ends the wait" mutants.
  - **One bounds set (re-check):** every Rust bounded wait or timeout is 10 s. That covers the Kernel's `recv_timeout` (as `device_paced.rs:933` does), probe and park waits, the server's socket read timeout, the binary's `try_wait` loop, and the waits for a capture file, a timed `tx_send` or `contended`. A correct run passes each in well under a second (Q4).
  - Server: the client socket's `set_read_timeout(10 s)` and a bounded `try_wait` loop (10 s) for the binary. The shared watchdog (120 s abort, `crates/ezsdr-radio-uhd/tests/common/watchdog.rs`) is armed by `TempDir::new`.
  - Python: the new class arms `faulthandler.dump_traceback_later(60, exit=True)` in `setUp` and cancels it in `tearDown`. A hung mutant then ends the process (mutate.py also counts its 180 s timeout as killed, `mutate.py:146-148`).
- **"Huge" in Simulation** means `10^6 s` (`10^15` ticks on the 1 GHz root). Simulation runs faster than the wall clock (`test_v58_02_virtual_time_runs_faster_than_the_wall_clock`), so 3600 s is not enough to hang a mutant.
- **How a served request is proven in Simulation.** A Simulation Run steps only inside a call (`kg_02_the_simulation_class_steps_on_the_callers_thread`). So a capture's `*.sigmf-meta` appearing in the Session directory proves that the control thread is inside the waiting request.
  - The capture Sink creates it when a capture completes: `crates/ezsdr-sink-capture/src/lib.rs:296-346`, VERIFIED.
  - Its `dir` is the Session directory: `lib.rs:207`, `catalogue.rs:219`, VERIFIED.
  - This replaces any `sleep` before the input is closed.

---

## S1 — Kernel: `RunHandle::disconnect_signal()`

### Test support (not tests; added in S1)

- **`WallAuthority` park probe** (`crates/ezsdr-kernel/tests/support/paced.rs`). This is F1's requirement.
  - `WallState` gets `parked_until: Option<i64>`. `next_wakeup` sets it to the tick it is about to wait for, and calls `changed.notify_all()`, just before `changed.wait_timeout`. It clears it after waking.
  - `WallAuthority::park_probe(&self) -> WallPark` clones the `Arc<WallTime>`.
  - `WallPark::wait_parked_until(tick, timeout) -> bool` waits on `changed` until `parked_until == Some(tick)`.
  - It changes nothing the existing doubles do.
- **`ThreadedProvider::gated_stop(gate)`**: `stop` records `stop` and then waits for `gate`, like `hold_after_take` (`paced.rs:410-415`). For K6.
- **`Probe::holders(&self) -> usize`** = `Arc::strong_count` of the probe's inner `Arc` (`run_doubles.rs:32`). It shows that a dropped Run released its Provider.

### Tests (all new)

| # | Test (file) | Sequence | Setup and operation | Asserts | Observes by | Wrong behaviour that fails it |
|---|---|---|---|---|---|---|
| K1 | `kc_36_a_disconnect_signal_ends_a_waiting_advance` (`device_paced.rs`) | 1, F1 | `paced()` + a plain `ThreadedProvider` (no publishing, no marks: no event, so the data thread never wakes the loop, `paced.rs:75-76`). `session(...)`, `let signal = run.disconnect_signal()`, `park = park_probe()`. A worker thread runs `run.advance_to(now + 3600 s)` and sends the result and `run`. The test waits `park.wait_parked_until(target, 10 s)` (expiry fails), then calls `signal()` from the test thread. | `recv_timeout(10 s)` returns `Err(Ended { Stopped { client_disconnect } })`. `count("radio:stop") == 1`. The Manifest (`run.finish()`): `termination.reason` is `client_disconnect`, `also` is empty, `termination.at < target`, and `run.transitions` contains `Stopping { mode: Orderly }`. | The park probe proves the control thread is in `next_wakeup` before the signal. The channel carries the worker's return. | No request: hangs until the timeout. No wake (= #89): stays parked for 1 h, so the timeout fires. Abort mode: the `Stopping { Orderly }` check fails. |
| K2 | `kc_36_a_disconnect_signal_ends_a_submit_waiting_for_its_provider` (`device_paced.rs`) | 2 | `ThreadedProvider::hold_after_take(gate)`, with the gate closed. A worker runs `run.submit(gain(2.0))`. The test waits for `probe.wait_for("radio:took:UpdateParameter", 10 s)`, i.e. the Action is taken, so the control thread is in KC-21a's wait (`pipeline.rs:1034-1036`). Then `signal()`. The gate opens only after the worker has returned. | Returns within `recv_timeout(10 s)` (KC-21a's own budget is 5 s). The entry is `Admitted`. `run.state()` is not `Running`. The Manifest's reason is `Stopped { client_disconnect }`, not `Failed { "KC-21a: …" }`, and `also` is empty. | The probe line before the signal. The termination cause. | No request: the wait runs its 5 s and fails the Run `Failed { KC-21a }`, so the cause check fails. |
| K3 | `kc_36_a_disconnect_signal_ends_a_simulated_advance` (`device_paced.rs`, next to `simulation_run`) | 7 (Q2) | `SimAuthority` with FreeRunning pacing. `SteppedProvider` `.publishing_every(10)` (it always has a next wakeup), plus `.with_wakeups(&[1_000]).blocking_step_at(1_000, gate)` as `ka_12_…` does (`coordinator.rs:2423`). `start_spec_run(output_docs)` with empty `environment`, as `simulation_run` does. A worker runs `advance_to(TimePoint(root, 10^12))`. The test waits `probe.wait_for("p:step:1000", 10 s)`, calls `signal()` from the test thread, then opens the gate. | Within `recv_timeout(10 s)` the result is `Err(Ended { Stopped { client_disconnect } })`. `termination.at < 10^12` (it ends at about 1000). | The blocked step proves the control thread is inside the advance when the signal fires. | No request: runs on toward 10^12, so the timeout fires. `device_paced()` checked before the request (Simulation never requested): the same. |
| K4 | `kc_36_a_disconnect_signal_does_nothing_for_a_detached_lease_or_an_ended_run` (`device_paced.rs`) | 6 | `detached_session(&probe, 60_000)` (`device_paced.rs:479`). `signal(); signal();`, then `run.advance_to(after(&run, 10 ms))`. Then `let m = run.finish()`, then `signal()` again. | The advance returns `Ok(())` and the Run is `Running`. `m.termination.reason` is `Stopped { client }` (finish asked first), and `also` is empty. The signal after the handle is gone does not panic. `probe.holders() == 1` while `signal` is still alive. | Return values. The probe's strong count shows the Run, and so its Provider, was dropped. | Acting on a Detached Lease (signal treated as Attached): the advance answers `Ended`. A strong hold on the Run: `holders() > 1`. |
| K5 | `kc_36_the_first_end_requested_stands_against_a_disconnect_signal` (`device_paced.rs`) | 3, 4, 5 (F3) | (a) `session(profile_one)`, idle. `signal()`. Then `running(&run)` and no `radio:stop` line yet. Then `run.finish()`. (b) `lost_device_session` (`:527`): wait for `radio:stop` (the Policy end, KC-46b), then `signal()`, then `finish()`. | (a) The Run is still `Running` after the signal (it runs no cleanup, KC-38). The Manifest says `Stopped { client_disconnect }`: the signal's end came first, and `finish`'s `client` is dropped (KC-32). `also` is empty. (b) The reason is `Stopped { policy: DEVICE_LOST }` and `also` is empty. | State and probe lines read before `finish`. Manifest fields. | A signal that cleans up on the caller's thread: (a)'s `running` fails. A signal that overrides an earlier end: (b) fails. |

Sequence 4's "nothing in `also`" relies on `ending::request` (`ending.rs:210-226`), which `kc_32_a_disconnect_after_a_policy_stop_is_dropped` already pins for `disconnect()`. K5(b) pins it for the signal.

| K6 | `kc_36_a_disconnect_signal_during_finish_s_cleanup_does_not_change_its_end` (`device_paced.rs`) | 5 (F3, `finish` first) | `session(...)` with `ThreadedProvider::gated_stop(gate)` (new support). A worker runs `run.finish()`. The test waits `probe.wait_for("radio:stop", 10 s)`, calls `signal()`, then opens the gate. | Within `recv_timeout(10 s)` after the gate opens, the Manifest: reason `Stopped { client }`, `also` empty, and `Stopping { Orderly }` is the only `Stopping` transition. | The probe line proves `finish` had requested its end and cleanup had begun. | A signal that escalates or overrides an earlier end: `also` gains `client_disconnect`, or the reason changes. |

### Mutation rows (S1)

All rows: `file` = `crates/ezsdr-kernel/src/coordinator/mod.rs`, `crate` = `ezsdr-kernel`, `test_bin` = `device_paced`. Anchors are written against sketch §2.1's shape and re-anchored to the final text. Where `CleanupMode::Orderly` is not unique in `mod.rs` (`disconnect()` spells it `crate::run::CleanupMode::Orderly`), `scope` = `pub fn disconnect_signal`.

| Id | Mutant (old → new) | Killer (`filter`) | Why the killer runs the mutated path |
|---|---|---|---|
| C89-1-01 | `if ending::request(` → `if false && ending::request(`: the signal requests no end | `kc_36_a_disconnect_signal_ends_a_waiting_advance` | K1 calls `signal()` while parked, so the request is the only end. |
| C89-1-02 | `paced::wake(&shared, true);` → (deleted): no wake | `kc_36_a_disconnect_signal_ends_a_waiting_advance` | K1 fires only after the park probe saw `next_wakeup` waiting for the 1 h tick, and nothing else wakes it. |
| C89-1-03 | `CleanupMode::Orderly` → `CleanupMode::Abort` (scoped) | `kc_36_a_disconnect_signal_ends_a_waiting_advance` | K1 asserts `Stopping { Orderly }` in `run.transitions`. K6 also kills it (escalation during `finish`'s cleanup records `also`). |
| C89-1-04 | `ending::request(&shared, Termination::Stopped { cause }, CleanupMode::Orderly) && shared.device_paced()` → `shared.device_paced() && ending::request(&shared, Termination::Stopped { cause }, CleanupMode::Orderly)`: Simulation never requested (Q2) | `kc_36_a_disconnect_signal_ends_a_simulated_advance` | K3 is a Simulation Run. |
| C89-1-05 | `self.lease.clone().on_disconnect(&*self.shared.ctx.host_clock)` → `Some(crate::run::StopCause::ClientDisconnect {})`: Detached treated as Attached | `kc_36_a_disconnect_signal_does_nothing_for_a_detached_lease_or_an_ended_run` | K4's Lease is Detached. |
| C89-1-06 | `Arc::downgrade(&self.shared)` → `self.shared.clone()`, with `old2` `shared.upgrade()` → `Some(shared.clone())`: holds the Run strongly | `kc_36_a_disconnect_signal_does_nothing_for_a_detached_lease_or_an_ended_run` | K4 keeps `signal` alive after `finish` and reads `holders()`. |

No row is planned for:
- `wake(&shared, true)` → `false`. Equivalent: a conditional wake is skipped only while a data-thread wake is pending, and that pending wake ends the same `next_wakeup` (KC-46a).
- Removing the `device_paced()` condition (also waking in Simulation). Equivalent: a no-op at `now` on a loop that reads the end every round. N2 is a text change (§S1 text).

### Text (S1)

- **KC-36.** Add the `disconnect_signal()` sentence, corrected for F2/A2. It must not say "the next call cleans up before dispatching": a `submit` that is already read can still admit and dispatch, then settle (`pipeline.rs:1033-1048`).
- **N2.** The wake is described as "in a class whose control loop waits in host time". The code keeps `device_paced()`: RealtimeEmulation never reaches `Running`, because KC-2 refuses it at plan (`pipeline.rs:295-296`).
- **KC-36's `*Test:*`** line gets K1–K6. **KC-46a**: one sentence.
- design/06 test table and Changes row.

---

## S2 — Server: watched input, the watch, A2, Detached refusal, `main.rs`

### Test helper (in `crates/ezsdr-server/tests/protocol.rs`)

`watched(dir, config) -> (UnixStream, BufReader<UnixStream>, JoinHandle<io::Result<Exit>>)`:
- `UnixStream::pair()`. The server half runs `serve_watched(s.try_clone(), s, config)` on a thread.
- The client half has `set_read_timeout(10 s)`.
- `send(&mut client, request_json, body)` writes `{"request":…, "body_bytes":n}\n` + body. Requests are serialized from `Request` (`Serialize`, `protocol.rs:111`).
- `reply(&mut reader) -> Option<(ReplyFrame, Vec<u8>)>` returns `None` at EOF.
- `meta_appears(dir, 10 s) -> bool` polls for any `*.sigmf-meta`.

### Tests

| # | Test | Sequence | Setup and operation | Asserts | Observes by | Wrong behaviour that fails it |
|---|---|---|---|---|---|---|
| W1 | `ea_15_the_watch_calls_every_signal_it_holds_once_and_each_added_after_it_fired` (unit test in the server crate's `lib`, beside the watch) | 9 (the connect-time path), N1's API | Three counting closures (`Arc<AtomicUsize>`). Add `a` and `b`, remove `b`, `fire()`, `fire()` again, then add `c`. | `a == 1`, `b == 0`, and `c == 1` at once on being added. | The counters. | `fire` calls nothing. A signal added to a fired watch is not called (a Session connected while its client left would never end). `remove` is ignored. |
| X1 | `ea_15_an_input_that_ends_during_an_advance_ends_the_session` (`protocol.rs`) | 8; the bench rehearsal | FakeDevice (manual clock) through `Config.open_device`, keeping the `Arc<FakeDevice>` as `ea_07_the_uhd_authority_takes_the_binding_s_sources` does (`protocol.rs:1527`). `uhd_profile` + `loopback()`. Over `watched`: `hello`, `connect` and `submit(repeat(), ramp(1000))`, reading each reply (`Admitted`). **Then it waits (bounded, 10 s) until `device.calls()` holds a timed payload send `tx_send 0 n=<k> at=<digits>` with `k > 0`; the untimed 4-zero flush at stream open (`core.rs:1205-1215`, `at=-`) does not count** (Sol major 1: the `submitted` reply proves a booking, not a send, `lib.rs:409-423`, `tx.rs:288-305`). Then `advance { by: 3600 s }` and `status {}` are written and `shutdown(Write)`. All replies are read to EOF. | 1. At most one reply follows the submit's, and it is `ended` with `Stopped { client_disconnect }`. Whether the advance was served before the end is a race; the verdict holds either way. 2. No `status` result is ever sent (A2). 3. The serve thread returns `Ok(Exit::EndOfInput)`. 4. `manifest.json`: reason `client_disconnect`, Lease `released`, `termination.at < connected.now + 3600 s`. 5. In `device.calls()`, the `tx_send … eob=true` is at a strictly later index than that timed payload send, and `close_streams` at a later index still. The failure message names the branch that ran (served and cut, or skipped by A2). **The race is not retried** (the re-check let me choose): X1 rehearses the bench's end state, which is the same Manifest, end of burst and `close_streams` in both branches. The served-and-cut path on a device-paced Run is rehearsed by K1 (Kernel, paced Authority, park probe) and by the UHD Authority's existing `ur_07_schedule_wakes_a_waiting_next_wakeup`. (`device.rs:1833, 1857, 2007`). | Reply frames, the Manifest file, the fake's call log. | No fire at end of input: the advance is always served (A2 never skips it) and never cut, so the 10 s read timeout fires. |
| X2 | `ea_15_an_input_that_ends_during_a_simulated_advance_ends_the_session_and_nothing_after_it_is_served` | 8 (Simulation, Q2), A2 | Default profile with `rx0()`, over `watched`: `hello` and `connect`. `submit(capture(1_000))`: read `Admitted`. Write `advance { by: 10^6 s }`. Wait `meta_appears(dir, 10 s)`: the control thread is now inside the advance. Then write `status {}` and `shutdown(Write)`. | 1. Exactly one more reply, `ended` with `client_disconnect`. 2. No `status` result. 3. `Ok(Exit::EndOfInput)`. 4. The Manifest says `client_disconnect` and `termination.at < connected.now + 10^6 s`. | The capture file proves the advance was served. In the input thread, `status` is forwarded before EOF fires, and the control thread can read it only after the cut, so A2 must skip it. | A2 removed: `status` is answered. Session signal not registered: the advance is never cut (timeout). |
| X3 | `ea_15_an_input_that_ends_during_a_child_run_ends_the_child` | 7 (child), RS-25 | Default profile with `rx0()`: `hello` and `connect`. Write `run_child { spec: receive_spec(1_000), duration_ns: 10^15 }`. Wait `meta_appears(dir, 10 s)`: the child's capture is done and the child is in `run_until_end`, its signal registered by `drive`. Then `shutdown(Write)`. | 1. One reply, `ran`. Its child Manifest says `Stopped { client_disconnect }` with `path` `child-0.manifest.json`. 2. `Ok(Exit::EndOfInput)`. 3. The Session's `manifest.json` says `client_disconnect`, with `run.children.len() == 1`. | The capture file; the replies; the Manifest files. | Child signal not registered: the child runs toward 10^15 ns, so the read timeout fires. |
| X5 | `ea_10_a_detached_lease_is_refused` | Q3 | `greeted(dir)`. Then `handle(Connect { lease: Some(Lease::detached(5_000, false, "t", &SystemHostClock::new())) })`. Then a plain `connect`. | The first is error `refused`, message starting `"EA-10:"`. `server.dir()` is `None` before the second connect, and `runs_dir` holds no `session-*`, so the check runs before `session_dir()` (`lib.rs:197`). The second connect is `connected`. | `Server::handle` directly. | Detached accepted: `connected`. The refusal placed after the directory is made: `dir()` is `Some`. |
| X6 | `ea_15_the_binary_exits_0_when_its_client_closes_both_pipes_during_a_request` | main.rs, F5 | Spawn `CARGO_BIN_EXE_ezsdr-server` with stdin, stdout and stderr piped. `hello`, then `connect` with `initial` rx0 (raw JSON). `submit` capture(1_000): read the reply. Write `advance { by: 10^6 s }`. Wait `meta_appears`. Drop stdout's reader first, then stdin. Wait with a bounded `try_wait` loop (10 s). | 1. Exit status 0. 2. stderr is empty. 3. The Session's `manifest.json` says `client_disconnect`. | The cut `ended` reply is written after stdout is closed, which gives a deterministic `BrokenPipe` (Rust ignores SIGPIPE, sketch probe B2). Exit status and stderr. | `BrokenPipe` not mapped: exit 1 and `ezsdr-server: Broken pipe…`. The binary on plain `serve`: the advance is never cut (timeout). |

| W2 | `ea_15_a_removal_waits_for_a_fire_in_progress_so_the_owner_drops_last` (server `lib` unit test) | N1 (Sol major 2) | Signal `s` is a test closure that holds a clone of `Arc<Token>` (where `Token::drop` records the dropping thread's name), says "entered", and waits for a gate. A thread named `input` runs `watch.fire()`. The test waits for "entered". A thread named `owner` runs `watch.remove(id)` and then drops its own `Arc<Token>`. The owner removes through the sites' helper. The gate is an `mpsc` channel: the closure blocks in `recv()`, and the test's sender drops on unwind. The test waits (bounded, 10 s) for `#[cfg(test)] watch.contended() >= 1`, then drops the sender (opens the gate) and joins both threads. | `Token` was dropped on `owner`. `contended >= 1`. | The hook: removal and clear try `try_lock` first and count a contended acquisition before they block (as `ClockMutex`'s `#[cfg(test)] waiting`, `host.rs`). This is positive proof that the owner is blocked behind the fire. | `fire` calling its signals outside the mutex (a cloned list): removal never contends, so the bounded wait fails. Without that wait it would also show as `Token` dropped on `input`. |
| W3 | `ea_15_every_end_of_a_run_removes_its_signal_from_the_watch` (server `lib` unit test) | N1's sites | `Server::watched(config, watch)` (crate-private): `hello`, `connect` (default Simulation, `rx0`), then `len() == 1`. `run_child(receive_spec(1_000), duration 10 ms)` returns `ran`, and `len() == 1`. A child whose `requires` has `radio.rx.s0.sample_rate_hz` `eq 1e12` fails at validate (no `start_instant`, `drive`'s early return, `lib.rs:309-311`): `ran` with the child `Failed`, and still `len() == 1`. `handle(Finish)`, and `len() == 0`. A second Server: `connect`, `disconnect()`, `len() == 0`. | Lengths as stated. | `#[cfg(test)] Watch::len()`. | A site that does not remove its signal: the end of `drive`, `finish`, `disconnect`. An add before `drive`'s early returns. |
| W4 | `ea_15_a_connect_after_the_input_ended_ends_on_its_way_to_t0_and_leaves_no_signal` (server `lib` unit test) | 9; Sol minor "connect" | `Server::watched` with a watch fired before anything. `hello`, then `connect` (default Simulation). | The reply is the error `ended`, `termination: Stopped { client_disconnect }`, `exit == true`. `<dir>/manifest.json` says `client_disconnect`. `len() == 0`. | The pre-fired watch: `add` calls the signal at once (W1), so only a registration before `advance_to(t0)` (`lib.rs:229-234`) makes the T0 advance end. | No registration: `connected`. Registration after the T0 advance: `connected`, with the end pending. The ended path keeping its signal: `len() == 1`. |
| W5 | `ea_15_finish_removes_the_session_s_signal_before_it_finishes_the_run` (server `lib` unit test) | N1's order at `finish` (re-check) | `Server::watched(config, watch)`. A gated test signal G is added to the watch first. `hello` and `connect` (default Simulation) add the Session's signal after G. Thread F runs `watch.fire()`, which calls G first (insertion order), and G blocks on its channel. The owner thread runs `server.handle(Finish)`. The test waits for `contended >= 1` (10 s), then drops G's sender, and joins both threads. | The owner's reply is `finished` with `manifest.termination.reason == Stopped { client_disconnect }`. | The fire, released after `finish`'s removal is blocked behind it, calls the Session's signal while the Run is still live only if the removal precedes the finish. Finish-then-remove ends the Run `client` first, and the late signal is a no-op. Requires `fire` to call signals in insertion order (stated in the watch's contract). | Finish before removal: reason `client`. |
| X7 | `ea_15_a_read_error_during_an_advance_ends_the_session` (`protocol.rs`) | Sol minor "read errors" | `serve_watched(reader, output, config)`, where the reader is test-controlled: it returns queued `Ok` chunks and, on command, `Err(io::ErrorKind::Other)`. Default Simulation, `rx0()`: `hello`, `connect`; then one injected `Err(Interrupted)`; then `submit(capture(1_000))` (read each reply: the `submit` is still `Admitted`, because `Interrupted` is retried and is not a departure), `advance { by: 10^6 s }`. Wait for the `*.sigmf-meta` (10 s), then send the error. | The `submit` after the `Interrupted` is answered. One more reply, `ended` with `client_disconnect`. `Ok(Exit::EndOfInput)`. The Manifest says `client_disconnect`. | The capture file proves the advance was served; the error is the only end of input. | The input thread not firing on a read error: the bounded read fails. `Interrupted` taken as a departure: the `submit` goes unanswered. |

Existing tests whose meaning changes, with no edit:
- `ea_binary_speaks_the_protocol` (`protocol.rs:1223`) now runs the watched binary. Its `finish` is answered while stdin is still open, and `child.wait()` returns only if `serve_watched` does not join the input thread. So it also pins sequence 10 ("`finish` replied, then the input ends → `client`") and EA-15's "replies `finished` and exits".
- Every `serve`-over-bytes test is unchanged: `serve` has no watch. That covers `ea_02_framing`, `ea_05_…`, `ea_15_finish_and_disconnect` and `ea_15_every_exit_writes_the_manifest`, whose `is_err()` stays: the `BrokenPipe` mapping is in `main.rs` only.

Sketch tests dropped as redundant (Q3, accepted):
- `ea_15_a_watched_input_that_ends_between_requests_ends_the_session`: today's EOF tests (`ea_15_finish_and_disconnect`, `ea_15_every_exit_writes_the_manifest`) cover the EOF path, and X2 and X3 need the watched loop to see EOF after their last reply, or they hang.
- An end-to-end "input ends during `connect`" through `serve_watched`: when the input ends relative to the frame being taken is not observable from outside. W4 now covers it at the `Server` level with a pre-fired watch (Sol minor).

### Mutation rows (S2)

Files: the watch's module (`crates/ezsdr-server/src/transport.rs`, or wherever the implementation puts it), `lib.rs`, `main.rs`. `crate` = `ezsdr-server`.

| Id | Mutant | `test_bin` / killer | Why the killer runs the path |
|---|---|---|---|
| C89-2-01 | `fire` calls no signal (the loop over signals deleted) | `lib` / `ea_15_the_watch_calls_every_signal_it_holds_once_and_each_added_after_it_fired` | W1 fires with `a` held. |
| C89-2-02 | `add` on a fired watch does not call the signal | same | W1 adds `c` after `fire`. |
| C89-2-03 | `remove` does nothing | same | W1 removes `b` before `fire`. |
| C89-2-04 | The input thread does not fire at **end of input** (EOF) | `protocol` / `ea_15_an_input_that_ends_during_an_advance_ends_the_session` | X1's advance is always served when nothing fires, and never ends. |
| C89-2-05 | A2's check deleted: the framing loop serves a frame read after the watch fired | `protocol` / `ea_15_an_input_that_ends_during_a_simulated_advance_ends_the_session_and_nothing_after_it_is_served` | X2's `status` is read after the fire, by construction. |
| C89-2-06 | `connect` does not add the Session's signal | `protocol` / same X2 | X2's advance is proven served, so only the Session's signal can end it. |
| C89-2-07 | `run_child`'s `drive` does not add the child's signal | `protocol` / `ea_15_an_input_that_ends_during_a_child_run_ends_the_child` | X3 fires only once the child is running. |
| C89-2-08 | The Detached check deleted | `protocol` / `ea_10_a_detached_lease_is_refused` | Direct. |
| C89-2-09 | `main.rs`: the `BrokenPipe` arm removed, so it falls to `FAILURE` with the message | `protocol` / `ea_15_the_binary_exits_0_when_its_client_closes_both_pipes_during_a_request` | X6's reply is written to a closed stdout. |
| C89-2-11 | `fire` calls its signals outside the watch mutex (the signal list cloned, the guard dropped, then the calls) | `lib` / `ea_15_a_removal_waits_for_a_fire_in_progress_so_the_owner_drops_last` | W2 holds a fire inside a signal and removes concurrently. |
| C89-2-18 | The helper at `finish` reversed: finish the Run, then remove its signal | `lib` / `ea_15_finish_removes_the_session_s_signal_before_it_finishes_the_run` | W5 releases a fire held in G after `finish`'s removal blocks, so the order decides the reason. |
| C89-2-19 | `drive` adds the child's signal before its early returns | `lib` / `ea_15_every_end_of_a_run_removes_its_signal_from_the_watch` | W3's child fails at validate and has no `start_instant`. |
| C89-2-20 | The input thread treats `ErrorKind::Interrupted` as an end of input | `protocol` / `ea_15_a_read_error_during_an_advance_ends_the_session` | X7 injects one `Interrupted` before the `submit`. |
| C89-2-12 | `finish` does not remove the Session's signal | `lib` / `ea_15_every_end_of_a_run_removes_its_signal_from_the_watch` | W3 reads `len()` after `Finish`. |
| C89-2-13 | `disconnect` does not remove it (merged with -12 if one helper does both) | same W3 | W3's second Server. |
| C89-2-14 | `connect`'s ended path does not remove it | `lib` / `ea_15_a_connect_after_the_input_ended_ends_on_its_way_to_t0_and_leaves_no_signal` | W4 takes the ended path. |
| C89-2-15 | The end of `drive` does not remove the child's signal | same W3 | W3's `run_child`. |
| C89-2-16 | Registration moved after `run.advance_to(t0)` in `connect` (`old`: the add and the advance; `new`: swapped) | same W4 | W4's watch is fired before `connect`. |
| C89-2-17 | The input thread fires on EOF only; a read error ends the forwarding without firing | `protocol` / `ea_15_a_read_error_during_an_advance_ends_the_session` | X7's departure is a read error. |
| C89-2-10 | `main.rs` calls `serve(BufReader::new(io::stdin().lock()), …)` instead of `serve_watched(io::stdin(), …)` (`old2`/`new2` for the `use`) | `protocol` / same X6 | X6's advance can end only through the watched input. |

No row is planned for:
- **N1's ordering at `disconnect` and on `connect`'s ended path** (the removal **before** the RunHandle is finished): there the reason is `client_disconnect` either way, so it stays with review. One helper removes under the watch's lock and then hands back the `Live` or the child; every site uses it (adjudication). W5 tests the order at `finish`; W2 that a removal waits for a fire in progress; W3/W4 that each site removes.
- **Hooks (adjudication):** a crate-private `Server::watched(config, Arc<Watch>)`. `Watch::len()` and the `contended` counter are `#[cfg(test)]` only. The removal path is a plain `lock()` in production, and under `#[cfg(test)]` a `try_lock` that counts a contended acquisition before it blocks.
- **Lock order** (Sol's scenario): the input thread holds the watch mutex while the signal takes `end`/`also` and, through `paced::wake`, `wake_handle` and the Authority's `schedule`. The control thread takes the watch mutex only from server code, holding no Kernel lock. There is no reverse order, and KC-46a bounds `schedule`'s block. This is by review; no row.
- **"`serve_watched` does not join the input thread"**: it is pinned by `ea_binary_speaks_the_protocol` (above). A mutant has to add code (a `join`), which is not a meaningful anchor.

### Text (S2)

- **EA-10**: the `lease` clause (Q3).
- **EA-15** continuation line: the watched input (EOF or a read error), A2's "once its input has ended it serves no further request", and F5's exit status, for `BrokenPipe` only: a reply write that fails with `BrokenPipe` is the client's departure, exit 0 with no message; any other write error keeps exit 1 and its message (Sol).
  - Its "Checked by" names X1, X2, X3, X6, X7, W2, W3, W4, W5 and `ea_binary_speaks_the_protocol`. EA-10 names X5.
  - `#89` goes in the Changes row only (`rules.py` markers).

---

## S3 — Python, tooling, documents

All Python tests are in `python/tests/test_easy_api.py`. They run as gate.py runs them: `PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=<target>/debug/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`, and under Python 3.9 in `python:3.9-slim`.
- Everything used is in 3.9: `start_new_session`, `os.getsid`, `os.killpg`, `faulthandler.dump_traceback_later`.
- A new class `Departure(unittest.TestCase)` holds the real-server tests, with `EasyApi`'s `setUp` (`find_server`, temporary `runs`) plus the faulthandler guard.
- The Mock tests go in a class `DepartureClose`, built like `DurationRequests.session` (`test_easy_api.py:1255`).

**Stream wrappers** (test helpers):
- `_Interrupting(stream, on)`: a pass-through to the real `BufferedReader`/`BufferedWriter`, with `closed`, `close`, `read`, `readline`, `write` and `flush`.
  - On its first `readline` (`on="readline"`) it raises `KeyboardInterrupt`.
  - On its first `write` (`on="write"`) it writes the first half of the bytes to the real stream and then raises `KeyboardInterrupt`.
  - On its first `read` (`on="read"`, for P6's body read) it raises `KeyboardInterrupt`.
- A test installs it in `sdr._connection._process.stdout` or `.stdin`, and keeps the real object to assert on.

### Tests

| # | Test | Sequence | Operation | Asserts | Wrong behaviour that fails it |
|---|---|---|---|---|---|
| P1 | `Departure.test_ea_16_an_interrupted_call_closes_both_pipes_and_ends_the_session` | 11, F6 (repeatable close) | `connect(rx={0:[0]})` (Simulation, streaming). Install a stdout wrapper that raises on `readline`. `assertRaises(KeyboardInterrupt): sdr.sleep(10**6)`. | 1. The real stdin and stdout are `closed` right after the exception. 2. `sdr.events()` raises `ProtocolError`, not `ValueError` or a hang. 3. `sdr.close()` returns a Manifest with `{"kind":"stopped","cause":{"kind":"client_disconnect"}}`, `manifest_path == dir/manifest.json`, and `_process.returncode == 0`. 4. A second `sdr.close()` returns the same dict (F6: repeatable). | No abandon, or stdin left open: `close()` sends `finish` and reads behind a server stuck in a 10^6 s advance, so the guard ends it. Stdout left open: assertion 1. A later call not refused: assertion 2. |
| P2 | `Departure.test_ea_16_an_exception_while_the_request_is_written_abandons_too` | F6 (write) | `connect(rx={0:[0]})`. Install a stdin wrapper that raises on `write` after half the bytes. `assertRaises(KeyboardInterrupt): sdr.sleep(1)`. | 1. Real stdin and stdout are `closed` right after. 2. `sdr.close()` gives a Manifest with `client_disconnect`: the server saw half a header and then EOF, which is `BadFrame` → `disconnect()`. The exit status is not asserted: 2 or 0, depending on whether its error reply met a closed stdout. | The `try` does not cover the write: the pipes are still open after the exception (assertion 1). |
| P3a | `DepartureClose.test_ea_16_close_after_an_abandon_sends_no_finish_and_reads_after_the_exit` | 11, F6 | Mock connection: `abandoned = True`. `close.side_effect` writes `manifest.json` (reason `client_disconnect`) into a temporary `dir`. | `sdr.close()` returns that Manifest. `connection.call.assert_not_called()`. `connection.close` was called once. | `finish` sent after an abandon: `call` is called and its Mock result used. Reading before the wait: the file does not exist yet, so the result is `None`. |
| P3b | `DepartureClose.test_ea_16_close_reads_the_manifest_after_the_server_exited` | the existing fallback's order | Mock: `abandoned = False`. `call.side_effect = ProtocolError(…)`. `close.side_effect` writes `manifest.json`. | `sdr.close()` returns it. | Today's order (read, then wait, `session.py:834-840`) raises `ProtocolError`, because the file appears only at the exit. |
| P3c | `DepartureClose.test_ea_16_close_after_an_abandon_leaves_no_manifest_when_none_is_readable` | F6 (missing or partial; a server on another host) | Mock: `abandoned = True`. (i) `close` writes nothing; (ii) `close` writes `"{"`. | Both (i) and (ii) return `None` without raising, and `manifest_path` stays `None`. `close()` again also returns `None`, with no exception. | A missing file raises (`FileNotFoundError`, or the fallback's `ProtocolError`). A partial file raises `JSONDecodeError`. |
| P4 | `Departure.test_ea_16_the_server_runs_in_a_session_of_its_own` | 12 (mechanism) | `connect()`, then `pid = sdr._connection._process.pid`. | `os.getsid(pid) == pid` (it leads its own session). | `start_new_session` dropped: the server shares the test's session. |
| P5 | `Departure.test_ea_16_ctrl_c_to_the_client_group_does_not_kill_the_server` | 12 (end to end; sketch probe B1) | A child Python, `Popen(start_new_session=True)` with `EZSDR_SERVER` inherited: `with ezsdr.connect(rx={0:[0]}, runs_dir=…) as sdr: print(sdr.dir, flush=True); sdr.sleep(10**6)`. The test reads the dir line, then `os.killpg(child.pid, SIGINT)`, then `child.wait(timeout=60)`. | `dir/manifest.json` exists, with reason `client_disconnect`, or `client` if the SIGINT came before the advance was written (accepted). The server lived to write it. | Without its own session the server dies by SIGINT: no Manifest (today's issue (b)). |
| P6 | `Departure.test_ea_16_an_interruption_while_the_reply_body_is_read_abandons` | 11, Sol minor (body) | `connect(rx={0:[0]})`, `sdr.rx.capture(100)`, and the artifact URI taken from the `sink.CAPTURE_WRITTEN` event in `sdr.events()`. A stdout wrapper raises `KeyboardInterrupt` on its first `read` (`_read_exactly`). `assertRaises(KeyboardInterrupt): sdr.read(uri)`. | Real stdin and stdout `closed` right after. `sdr.close()` gives `client_disconnect`. | The abandoned region ending after `readline`: the pipes stay open (assertion 1). |
| P7 | `DepartureClose.test_ea_16_a_server_that_does_not_exit_is_killed_and_close_keeps_no_manifest` | Sol minor (30 s fallback) | `conn = object.__new__(Connection)`; `conn._process = Mock()` with `wait.side_effect = [subprocess.TimeoutExpired("ezsdr-server", 30), -9]`; `conn.abandoned = True`. A Session over `conn` with an empty temporary `dir`. `sdr.close()`. | Returns `None` without raising. `kill` called once, `wait` called twice (the first with `timeout=30`), `stdout.close` called. | `kill` dropped: assertion on `kill`. |
| changed | `EasyApi.test_ea_16_errors` (`:487`) | 11 (an error reply is not an interruption) | After the `RunEnded` from `sdr.set(...)`, add `self.assertIsInstance(sdr.events(), list)`. | An error reply (`_raise`, `_client.py:92-94`) comes after the whole reply is read and leaves the connection usable. | Error replies abandon: `events()` raises `ProtocolError`. No existing test calls the Session again after an error reply (search: `grep -n "assertRaises(ezsdr.ServerError\|…ProtocolError\|…RunEnded" python/tests/test_easy_api.py` → `:488, 495, 932, 941, 1573`, none followed by another call). |

`test_ea_16_close_after_the_server_exited` (`:954`) is unchanged and must still pass:
- `finish`'s write meets `BrokenPipe`, which now abandons and raises `ProtocolError`.
- The fallback then waits, and reads the Manifest the dead server wrote.

**Implementer's notes the tests depend on:**
- `Session.close` tests `self._connection.abandoned` by plain truthiness; no `is True` in production (Sol). The test doubles set it explicitly: `DurationRequests.session` (`:1255`) and `ConnectRequests.frame`/`refused` (`:1107-1121`) set `abandoned = False`, and P3a–c and P7 set their own value. These are changed helpers, not changed tests.
- `close()` rules (Q2, accepted): after any abandon (an earlier call ended while its reply was awaited), `close()` sends no `finish`, waits for the exit (30 s, then `kill`), reads `manifest.json` if it can, and otherwise returns `None` without raising. When `close()`'s own `finish` fails, the fallback still raises `ProtocolError` if no Manifest is readable, as today, now after the wait.
- There is no separate `abandoned` check in `call`: the closed stdin refuses later calls through today's check (`_client.py:79-81`).
- `stdin.close()` in the abandon swallows `BrokenPipeError`, as `Connection.close` does (`_client.py:100-103`).

### Mutation rows (S3)

`python` rows, exact substrings, anchored to the final code. Ids `C89-3-…`.

| Id | Mutant (intent) | Killer (`python`) |
|---|---|---|
| C89-3-01 | `start_new_session=True` → `start_new_session=False` | `test_easy_api.Departure.test_ea_16_the_server_runs_in_a_session_of_its_own` |
| C89-3-02 | The abandon does not close stdin | `…Departure.test_ea_16_an_interrupted_call_closes_both_pipes_and_ends_the_session` |
| C89-3-03 | The abandon does not close stdout | same |
| C89-3-04 | The `try` begins after the write (write and flush outside it) | `…Departure.test_ea_16_an_exception_while_the_request_is_written_abandons_too` |
| C89-3-05 | Error replies abandon (`_raise` inside the handled region) | `test_easy_api.EasyApi.test_ea_16_errors` |
| C89-3-07 | `close()` after an abandon still calls `finish` | `…DepartureClose.test_ea_16_close_after_an_abandon_sends_no_finish_and_reads_after_the_exit` |
| C89-3-08 | Abandon path: reads `manifest.json` before `connection.close()` | same P3a |
| C89-3-09 | Fallback path: reads before `connection.close()` (today's order restored) | `…DepartureClose.test_ea_16_close_reads_the_manifest_after_the_server_exited` |
| C89-3-10 | A partial Manifest is not tolerated (`json.JSONDecodeError`/`ValueError` dropped from the `except`) | `…DepartureClose.test_ea_16_close_after_an_abandon_leaves_no_manifest_when_none_is_readable` |
| C89-3-11 | A missing Manifest after an abandon raises (`OSError` dropped from the `except`) | same P3c |
| C89-3-12 | The abandoned region ends after `readline` (the body read `_read_exactly` outside it) | `…Departure.test_ea_16_an_interruption_while_the_reply_body_is_read_abandons` |
| C89-3-13 | `self._process.kill()` dropped from the `TimeoutExpired` branch | `…DepartureClose.test_ea_16_a_server_that_does_not_exit_is_killed_and_close_keeps_no_manifest` |

C89-3-06 is dropped (Sol): with no separate `abandoned` check in `call`, the closed stdin's existing refusal is the only one. P1's assertion 2 pins it, and it is not this step's code.

### Tooling and documents (S3; no test verdict)

- **`ssh-drop-driver.py`**: `EZSDR_DROP_WAIT=time|sleep`, default `time` (today's `time.sleep(hold)`, `:68`). With `sleep` it calls `sdr.sleep(hold)`. Before the wait it writes the marker `wait-start` with `time.time_ns()`.
- **Evidence that the request was entered** (Sol): the ssh wrapper (`ssh-drop-tx.sh:130-138`) puts a small line-timestamping relay between the client and `ssh`. It writes each client-to-server header line with `date +%s%N` into `$OUT/to-server.log` (headers only; bodies are counted, not stored). The script then judges "entered" when:
  - the last header before the drop is the `advance` frame;
  - it was written ≥ 1 s before the drop;
  - every earlier request had its reply. The driver's `tx-started` marker follows the `repeat` reply, and `wait-start` comes just before `sleep`.

  Otherwise C1 and C2 read `INCONCLUSIVE: no evidence that the server was inside advance at the drop`. Entry itself is INFERRED from the gap: an idle server takes a frame within µs, and A2 skips only a frame read after the end. At `8dd1016`, C1 FAIL ("still running at +15 s") is direct evidence.
- **`EZSDR_DROP_CUT=link|kill`** (Q1, accepted): `kill` runs `docker kill "$DROP_NAME"` instead of killing the ssh client. Instead of C1/C2 it prints `VERDICT E9 the tone on #2 ended at +N s after the kill` or `… still on at +60 s`, a measurement. C3 runs only after quiet, as today.
- **`ssh-drop-tx.sh`**:
  - It passes `EZSDR_DROP_WAIT` through.
  - **F7's hold margin**: under `sleep`, the drop happens after `EZSDR_DROP_SETTLE`. If `hold − (DROP_NS − wait-start)/1e9 < 15 + 15` s, C1 and C2 read `INCONCLUSIVE: the sleep had only N s left at the drop`. The margin is 15 s beyond the 15 s deadline. With the defaults (hold 120, settle 5) about 115 s are left.
  - The verdict lines are unchanged, except for the new `INCONCLUSIVE` reason (entry evidence) and the E9 line in `kill` mode.
  - The script builds the server from `COMMIT`, and the Python package from `$OUT/src/python` of that commit (`ssh-drop-tx.sh:145-148`). The driver comes from the running tree. So the run at `8dd1016` uses the old server and Python with the new driver, which is exactly the pair this run compares.
- **`plan/phase8/bench.md`**, new section "#89 departure during a wait (2026-11-xx)":
  - (i-a) `EZSDR_DROP_WAIT=sleep` at `8dd1016`: expected C1 FAIL, server still in `advance` at +15 s; C2 FAIL, tone on.
  - (i-b) The same at the new commit: expected C1, C2, C3 PASS.
  - (ii) `EZSDR_DROP_CUT=kill`: a `docker kill` of the server during a repeat with #2 listening, to measure E9. F7: the emergency `docker stop -t 2` after (i-a)'s verdicts is already an unclean death (SIGTERM to PID 1, then SIGKILL), so keep reading the receiver after it, for E9's first data point.
  - The attenuator confirmation before any transmit.
  - E5 cited: `Provider::stop` about 104 ms on X300 #1, 2026-11-09 (i), VERIFIED, one sample.
- **Rehearsal of the tooling**: `bash -n plan/phase8/tools/ssh-drop-tx.sh` and `python3 -m py_compile plan/phase8/tools/ssh-drop-driver.py`. The relay is exercised once locally against the Simulation server (`EZSDR_SERVER` = a wrapper with the relay, no ssh, driver logic only through the relay): `to-server.log` holds `hello`, `connect` and `advance`. The server behaviour the bench verdicts rest on is rehearsed by X1 (device-paced, FakeDevice, a timed payload send observed before the departure, served or skipped: `ended`/no reply, Manifest `client_disconnect`, EOB and `close_streams`) and by P1 (an interrupted `sleep` through the real binary).
- **EA-16 text and docstrings**: connect's own session; the abandon on any exception while a reply is awaited, error replies excepted; `close()` after an earlier abandon (no `finish`, wait, read, else `None`) vs `close()`'s own `finish` failing (`ProtocolError` when no Manifest); a missing or partial Manifest; the 30 s wait then `kill`. **KL-43**: Q5, F4 (a hung server is not killed by Ctrl-C), F5's three additions (inherited pipes keep the input open, the 30 s kill leaves no Manifest, F4), the Manifest only when the server had begun the request (Findings 2), and the sshd workaround stated generically.

---

## Hardware audit (AGENTS §9 "Hardware tests and bench values")

| Test or tool | Depends on the changed behaviour? | Rehearsal |
|---|---|---|
| `crates/ezsdr-radio-uhd/tests/hardware.rs` (`#[ignore]`, the only ignored hardware tests: `grep -rln '#\[ignore' crates/*/tests` → `hardware.rs`, plus a string in `governance.rs`) | No. They drive `RunHandle` and Providers directly, never the server. `disconnect_signal` is additive. No rule they compute expectations from changes (`grep -n disconnect crates/ezsdr-radio-uhd/tests/*.rs` → only `fake.rs:2706`, an unrelated UR-33 test). | Not needed. |
| `plan/phase8/tools/ssh-drop-tx.sh` + `ssh-drop-driver.py` (bench (i), (ii)) | Yes. C1 and C2 rest on the server ending a Session whose client went away during `sdr.sleep`, with the tone already sent. | X1 (FakeDevice, device-paced: a timed payload send observed, then the departure, EOB and `close_streams`, whether the advance was served or skipped by A2), K1 with `ur_07_schedule_wakes_a_waiting_next_wakeup` for the served-and-cut wake on a paced Authority, X7 (read error), and P1 (the real binary, `sleep` interrupted). The scripts get `bash -n`/`py_compile`, and the relay a local run. |
| `python/examples/*.py` (`test_ea_19_the_examples_run`) | No. They end through `close()`/`finish`. | Existing test. |

No bench value is fixed from new data by this step. E9 is measured in (ii) and feeds A1's later decision, not a rule here.

---

## Counts

| Sub-step | Tests added | Changed | Deleted | Mutation rows |
|---|---|---|---|---|
| S1 Kernel | 6 (K1–K6), plus 3 support helpers (park probe, `gated_stop`, `Probe::holders`) | 0 | 0 | 6 |
| S2 Server | 6 integration (X1, X2, X3, X5, X6, X7) + 5 unit (W1–W5) = 11 | 0 | 0 | 20 |
| S3 Python | 9 (P1, P2, P3a, P3b, P3c, P4, P5, P6, P7) | 1 (`test_ea_16_errors`); 2 helpers set `abandoned = False` | 0 | 12 |
| **Total** | **26** | **1** | **0** | **38** |

**Rows killed only by a bound** (Q4; every Rust bound is 10 s): a correct run passes well inside it, and the mutant is killed by its expiry.
- C89-1-01: K1's `recv_timeout`; also K3.
- C89-1-02: K1; only with the park probe.
- C89-1-04: K3.
- C89-2-04: X1's socket read timeout.
- C89-2-06: X2.
- C89-2-07: X3.
- C89-2-10: X6's `try_wait` loop.
- C89-2-11: W2's wait for `contended >= 1`.
- C89-2-17: X7.

**S2 fix round (implemented, supersedes the S2 row table above where they differ).** `Watched` has a `Drop` that removes its signal, so rows C89-2-13, -14, -15 and -19 are removed: 15 and 19 are equivalent (the drop removes at the same point, or on the early return), and 13 and 14 became order-only mutants at `disconnect` and on `connect`'s ended path, which stay with review. C89-2-12 is an N1-order mutant at `finish`, killed by W5. New C89-2-21 (the `Drop` removes nothing), killed by the new W6 `ea_15_a_signal_is_removed_when_its_holder_is_dropped`. The W tests moved to `crates/ezsdr-server/tests/watch.rs` (`test_bin` `watch`). W3 now kills no row and W4 only C89-2-16. S2 has 17 rows; those killed only by a bound are C89-2-04, -06, -07, -10, -11 and -17.

C89-2-20 is killed by an assertion, not a bound: with `Interrupted` taken as the end, the server's output ends and X7's read of the `submit` reply finds no reply (corrected in the S2 fix round).
C89-3-02 is killed by P1's assertion that the real stdin is `closed` (corrected; the guard is only the backstop for a hang that would follow).
All other rows are killed by an assertion.

---

## Findings: where the sketch or the review disagrees with the code

1. **A2 makes several of the sketch's server tests racy** (sketch §9, written before A2).
   - The input thread forwards a frame and then fires at EOF, and the control thread may take that frame before or after the fire. So "write a frame, then `shutdown(Write)`" leaves it open whether the frame is served at all.
   - The sketch expected `ended` for `ea_15_an_input_that_ends_during_an_advance_…` and a cut child for `ea_15_…_child_run_…`. Either can instead get no reply.
   - Fix used here: X1's assertions hold in both orders. X2, X3 and X6 first prove that the request is in progress through the capture file. That works only in Simulation, where steps happen only inside a call (`kg_02_the_simulation_class_steps_on_the_callers_thread`).
2. **The sketch's contract table, under A2 and F3:**
   - "Ctrl-C during `connect` → Manifest `client_disconnect` in `runs_dir`" holds only if the server took the `connect` frame before the end. Otherwise no Session directory and no Manifest are ever made.
   - "SIGINT in `finish` → Manifest `client`" can also be `client_disconnect`: A2 skips a `finish` read after the fire (F3, decision 9).
   - The EA-16 and KL-43 wording should say "a Manifest when the server had begun the request".
3. **The Detached refusal must come before `session_dir()`.** Q3 / sketch §2.2 say "before any directory or Run exists", but `Server::connect` creates the directory first (`lib.rs:197`), before the profile is even read. X5 pins the order.
4. **N1 covers more places than `disconnect`/`finish`.** The watch must also be emptied or removed under its mutex at two more points:
   - `connect`'s ended-on-the-way-to-T0 path, which calls `run.finish()` after the signal was registered (`lib.rs:233-239`);
   - the end of `drive`, because the Kernel finishes and drops the child right after `drive` returns (`pipeline.rs:1085-1086`).
   Without the second, removing the child's signal is not just tidiness: it is N1's guard for the child.
5. **Sketch §9's KC-21a test uses `deaf()`.** With it the test cannot know that the signal came during the wait: the verdict is the same if it came first, because `submit` admits and dispatches on a Run whose end is pending, `pipeline.rs:1033-1048`. K2 uses `hold_after_take` and waits for `took:`. The sketch's `undelivered` assertion is dropped (a taken Action is not undelivered), and RS-16 keeps its own tests.
6. **Sketch §9's Simulation Kernel test also does not observe the "during" path.** K3 uses `blocking_step_at`, as `ka_12_…` does.
7. **N2 against the code.** `device_paced()` (`state.rs:343`) is exactly HardwareInLoop | Hardware, and a RealtimeEmulation plan fails at KC-2 before `Running` (`pipeline.rs:295-296`). So the condition is correct today, and only the KC-36 text needs N2's wording. A code change to "not Simulation" would be equivalent and untestable.
8. **`kc_36_a_disconnect_signal_ends_a_simulated_advance` needs a Spec Run, not "a Session".** The double that always has a wakeup and can block a step (`SteppedProvider`) is used with `start_spec_run`. The signal works on any `RunHandle`, which the child (a Spec Run) needs anyway. Not a design error; a note on the sketch's wording.
9. **No existing test kills "error replies abandon".** No Python test calls the Session again after an error reply (search in the S3 table). The plan changes `test_ea_16_errors` for it.

Nothing else in sketch §1–§2 or in Fable's §1 table was contradicted by the code I read:
- `paced.rs:22, 62, 75-76, 153-206`; `stepping.rs:311-425`; `ending.rs:15-47, 197-226`;
- `mod.rs:250-296`; `run.rs:412-440`; `pipeline.rs:1033-1095`;
- server `transport.rs`, `main.rs`, `lib.rs:185-258, 338-400`;
- `_client.py`, `session.py:629-633, 826-841, 978-982`.

## Questions for the orchestrator

Q1–Q4 of the first round are answered (`plan-adjudication.md`) and applied. New questions:

1. **W2–W4 need two narrow hooks in production code:**
   - a crate-private `Server::watched(config, Arc<Watch>)` constructor (also what `serve_watched` uses);
   - two `#[cfg(test)]` items on the watch, `len()` and the `contended` counter, with removal written as `try_lock`, then `lock`.

   The disposition allows hooks "only where necessary", and I see no hook-free way to prove positively that the owner is blocked (otherwise only a timed negative). Accept them?
2. **Order at each removal site is not mutation-pinned** (§S2 "No row"): removal before vs after `finish`. A probe would need the Kernel's signal to expose its Run's liveness, which is new Kernel surface. Recommendation: make the order structural (one helper that removes, then hands back `Live`), and leave it to review. Agree, or do you want the Kernel surface?
3. **Bench entry evidence is a client-side relay plus a ≥ 1 s gap, so entry is INFERRED.** The alternative is a positive server-side record: one stderr line per request taken, behind an environment variable. That is new production code for the bench. Is the INFERRED evidence enough?
