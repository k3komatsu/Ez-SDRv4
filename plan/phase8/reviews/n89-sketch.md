# #89 design sketch: a client that goes away ends its Session, also while a request waits

## Owner decisions (override the rest where they differ)

1. Q1 = **R** (owner 2026-11-11): a Kernel `RunHandle::disconnect_signal()` (KC-36), the server's input-watching thread, Python `start_new_session` and abandon-on-interrupted-call. B (server-side slicing) rejected.
2. Q2 = **(a)** (owner 2026-11-11): Simulation Runs are cut too; one rule in every class; the end instant is recorded in `termination.at`.
3. Q3 = **(a)** (owner 2026-11-11): the stdio server refuses a Detached Lease at `connect` (it cannot honour one: nothing can reconnect to it). The Kernel's Detached/Adopt/TTL stay for a future long-lived server.
4. Q4 = **both** (owner 2026-11-11): Python starts the server with `start_new_session=True`, and an exception while a reply is awaited abandons the connection, so the Session ends `client_disconnect` (no keep-the-Session-after-interrupt; a cancel-only token stays a later option).
5. Q5 = **(a)** (owner 2026-11-11): KL-43 for silent ssh partitions and signals/death of the server itself; no heartbeat, no signal handler. The lab sshd is NOT configured now: the server runs from many PCs, so KL-43's workaround is written generically (`ClientAliveInterval`/`ClientAliveCountMax` in the sshd of whichever host runs the server over ssh).
6. Q6 (owner 2026-11-11): bench after the fix, in one session on (i)'s splitter wiring: `EZSDR_DROP_WAIT=sleep` at `8dd1016` (FAIL expected) and at the new commit (PASS), plus (a) `docker kill` of the server during a repeat (measures E9). (b) `kill -STOP` of the ssh client is not run.
7. After the Fable 5.2 review (fable-design.md): **A2 adopted** (owner 2026-11-11): once the server's input has ended, it serves no further request (removes F2: a read-ahead `tx.repeat` is not dispatched after the departure).
8. **A1 deferred** (owner 2026-11-11): decide the server's fail-stop backstop (exit N s after input end) after the Q6(a) `docker kill` measurement of E9; until then F4 (a hung server with `start_new_session` is not killed by Ctrl-C) goes into KL-43.
9. **F3** (owner 2026-11-11): when `finish` and a departure overlap, the first end requested wins (KC-32): `Stopped { client }` or `Stopped { client_disconnect }`; the contract table says so. **A3 deferred**: child propagation stays in the server's watch (children run only in Simulation, KC-37a).
10. All other Fable findings (F1, F2 via A2, F4–F7, N1) are adopted into the brief and test plan.

AGENTS §9 step 0 (design). Draft by an Opus subagent (effort max), read-only. Nothing in the tree was built, run, formatted or edited; two probes ran in the session scratchpad (Appendix B).

**Base.** `main` = `8dd1016`, clean. Every `path:line` is at that commit.

**Marks.** VERIFIED: I read the code or record at the cited line, or ran the probe. INFERRED: my reasoning or a document I could not check here. Sizes are estimates.

---

## 0. Recommendation in one paragraph, and what it beats

**Recommendation (R).** Three small pieces that ship together:

1. **Kernel**: `RunHandle::disconnect_signal()` returns a `Send + Sync` function that any thread may call to report that an *Attached* Lease's client has gone. It requests `Stopped { client_disconnect }` (orderly) and, in a device-paced class, wakes the control loop exactly as the data thread already does (KC-46a). The cleanup stays on the control thread: the waiting call (`advance_to`, `wait_for`, KC-21a's wait inside `submit`) returns, runs RS-6 and answers `Err(Ended)`. About 15 lines; one KC-36 sentence; no new Kernel type.
2. **Server**: the binary reads its input on a thread of its own and, at end of input, fires the Session's signal (and a running child's). The request in progress is cut and answered `ended`; the existing EOF path (`disconnect` → `finish` → `manifest.json`) then runs as today. A Detached Lease, which this server cannot honour, is refused at `connect`.
3. **Python**: start the server with `start_new_session=True` (a terminal's Ctrl-C reaches only the client); an exception while a reply is awaited (`KeyboardInterrupt` included) closes the server's input and output at once, which *is* the departure; `close()` then sends no `finish` and reads `manifest.json` after the server exits. No stale reply can ever be read.

Bound for RF on a visible departure (local pipe, ssh drop that sshd sees): detection ≈ 0 + one wake (µs; ≤ 10 ms inside KC-21a's wait) + `Provider::stop` (the burst ends about 10 ms after the stop is booked; the whole stop ≤ about 3 s worst case, < 1 s measured end to end on the X300). What stays outside the bound — a silent network partition under ssh, a signal sent to the server itself — goes to a KNOWN_LIMITATIONS entry with a configuration workaround (§3, §6).

**Rejected (details in §4).**
- **B. Server slices device-paced waits (≤ 100 ms) and polls a flag** (the audit's fix): no Kernel change, but it re-implements KC-29/KC-29b's wait in the server, adds a control round per slice, leaves KC-21a's 5 s wait unsliced, never cuts Simulation, and every future frontend must copy it.
- **C. A Kernel cancellation token that ends a wait without ending the Run**: a new outcome for `advance_to`/`wait_for` that #89 does not need; additive later if an interrupt that keeps the Session is ever wanted.
- **D. Signal handlers in the server** (SIGINT ignored, SIGTERM → disconnect): needs `unsafe` or a new dependency, covers only (b), and `start_new_session` already covers (b).
- **E. A protocol heartbeat**: the only fix for silent partitions, at the cost of a timer thread in every client and a frame outside the request/reply alternation; ssh already has `ClientAliveInterval`.
- **F. Python-only fixes** (slicing `sleep`, closing stdin): cannot protect an ssh drop (audit, agreed). **G. Fail-stop at EOF** (exit the server at once): no EOB, no Manifest, RF stop left to starvation (INFERRED). **H. TTL on Attached Leases**: changes the Lease's meaning (audit, agreed).

**Risk class: risky** (a lifecycle — how a Run ends, from another thread — and a Kernel contract, KC-36). Step 0 applies (Kernel contract, user-facing API meaning of EA-15/EA-16). One step in three sub-steps (§10).

---

## 1. The gap, in the code

**Where the server sees its client.** One loop reads a frame, handles it, writes the reply (`crates/ezsdr-server/src/transport.rs:24-83`); end of input is seen only at the read (`:26-30`). The binary hands it `BufReader::new(io::stdin().lock())` (`crates/ezsdr-server/src/main.rs:30`). VERIFIED.

**Where a request waits.**
- `advance` and `wait_for` call `RunHandle::advance_to` / `wait_for` synchronously (`crates/ezsdr-server/src/lib.rs:494, 509`) → `advance_to_with` schedules a no-op at the target and runs `run_loop` (`crates/ezsdr-kernel/src/coordinator/stepping.rs:394-425`), which checks the Run's end only at the top of each iteration (`:313`) and otherwise blocks in `Authority::next_wakeup()` (`:316`). On the UHD Authority that is a host-clock wait until the target (`crates/ezsdr-radio-uhd/src/authority.rs:523-555`). VERIFIED.
- `submit` on a device-paced Run waits for the Provider to book (KC-21a), up to 5 s, rechecking every 10 ms whether an end was requested (`crates/ezsdr-kernel/src/coordinator/paced.rs:22, 177-206`; called from `pipeline.rs:1033-1036`). VERIFIED.
- The Python waits are each one request: `sleep`/`wait_until` (`python/ezsdr/session.py:696-704`), `wait_for` (`:706-715`), `result` (`:433-456`, a loop of `wait_for`), `capture` = `request` + `result` (`:406-414`). VERIFIED.

**Python's side.** `Popen` without `start_new_session` (`python/ezsdr/_client.py:67`); `call` keeps no "reply awaited" state (`:74-95`); `Session.close` sends `finish` and reads the next reply as its answer (`session.py:826-841`), so after an interrupted call it reads the stale reply: `KeyError: 'manifest'` (`:831`). `close()`'s fallback reads `manifest.json` *before* waiting for the server to exit (`:834-838` then `:840`). VERIFIED.

**What already works and is reused.**
- KC-46a's wake: `paced::wake(shared, unconditional)` schedules a no-op at the current instant (`paced.rs:153-173`). The UHD Authority's `schedule` calls `host.notify_all()` (`authority.rs:295, 307`), and `next_wakeup` reads the notify generation before the queue and waits only while it is unchanged (`authority.rs:525-554`, `crates/ezsdr-radio-uhd/src/host.rs:336-372`): a schedule from any thread ends the wait at once. VERIFIED (code).
- `ending::request` keeps the first end and drops a later disconnect that does not escalate (`crates/ezsdr-kernel/src/coordinator/ending.rs:210-226`). VERIFIED.
- `Drop for Server` finishes a live Session and writes `manifest.json` on any exit, a failed reply write included (`lib.rs:395-401`). VERIFIED.
- A child Run inherits the Session's Lease and the parent settles after the child (`pipeline.rs:1089-1094`). VERIFIED.

**A Detached Lease through this server is never honoured.** At end of input the server calls `disconnect()` (which, for a Detached Lease, only starts the TTL) and then `finish()` (`lib.rs:185-191`), which ends the Run `Stopped { client }` (`crates/ezsdr-kernel/src/coordinator/mod.rs:269-273`, `ending.rs:197-202`): a misattributed cause, and the same #89 gap while a request waits. No server or Python test uses a Detached Lease (search in Appendix A). VERIFIED.

---

## 2. The design

### 2.1 Kernel: `RunHandle::disconnect_signal()` (KC-36)

```rust
impl RunHandle {
    /// KC-36: a function any thread may call to report that the client has gone.
    pub fn disconnect_signal(&self) -> impl Fn() + Send + Sync + 'static {
        // One decision of what a disconnect means: Lease::on_disconnect's, on a copy (RS-23).
        let cause = self.lease.clone().on_disconnect(&*self.shared.ctx.host_clock);
        let shared = Arc::downgrade(&self.shared);
        move || {
            let (Some(cause), Some(shared)) = (cause.clone(), shared.upgrade()) else { return };
            if ending::request(&shared, Termination::Stopped { cause }, CleanupMode::Orderly) && shared.device_paced() {
                paced::wake(&shared, true);
            }
        }
    }
}
```

Semantics (normative, §6.2):
- **Attached Lease**: requests `Stopped { client_disconnect }`, orderly. KC-32 keeps an earlier end; a later disconnect says nothing new and is dropped (RS-10). When this request set the end and the class is device-paced, it wakes the control loop unconditionally (KC-46a).
- **Detached Lease**: does nothing (`on_disconnect` returns `None` for it; the TTL starts only at `disconnect()` on the control thread).
- **Runs no cleanup.** The control thread does, inside the call that was waiting or the next call (KC-38): `run_loop` returns at its next iteration (`stepping.rs:313`), `advance_to_with` settles (`:423`) and answers `Err(Ended)`; KC-21a's wait sees the end within 10 ms (`paced.rs:186`), `submit` settles (`pipeline.rs:1048`).
- **Holds the Run weakly**: it never keeps a Run alive (the data thread's last-holder backstop, `paced.rs:62`, is unaffected) and does nothing once the Run is gone.
- In the **Simulation class** it needs no wake: the loop never blocks in host time and reads the end every round (Q2 decides whether Simulation uses it at all).
- No thread is spawned, no wall clock read: PO-11/GZ-4 unaffected. No new module-level public item (OV-23): the return type is `impl Fn`. A named type is the alternative if reviewers prefer discoverability (one `NEW:` line in `kernel_surface_allow.txt`).

Why the control thread and not KC-46b's data thread: KC-46b performs steps 1–2 on the data thread only for ends it requested itself, and its own text records the cost — receive samples an orderly client end would have delivered can be lost while the data thread sits in `Provider::stop` (`design/06-kernel-coordinator.md:440`). The server always follows the signal with a control-thread call (the waiting one, then `disconnect`/`finish` at EOF), so nothing needs a third thread inside cleanup (rejected variant K-c, §4).

### 2.2 Server: the binary watches its input

- `transport.rs`: `pub fn serve_watched(input: impl Read + Send + 'static, output: impl Write, config: Config) -> io::Result<Exit>`. A thread (`ezsdr-input`) reads `input` in chunks and forwards them over an `mpsc` channel to the unchanged framing loop (a ~15-line `Read` over the receiver, wrapped in `BufReader`); at end of input or a read error it **fires the watch** and then drops the sender, so the loop sees the end in order and runs today's EOF path (`transport.rs:27-30`). `main.rs` calls `serve_watched(io::stdin(), io::stdout().lock(), config)`.
- **The watch** is a sticky list of disconnect signals behind a mutex: `fire()` marks it fired and calls every signal; adding a signal to a fired watch calls it at once. The Server holds it (`Option<Arc<Watch>>`, `None` under plain `serve`).
- **Registration**: `connect()` adds the Session's signal once `coordinator::connect` has returned (`lib.rs:225-228`); `run_child`'s `drive` closure (`lib.rs:308-320`) adds the child's signal before `run_until_end` and removes it after, so a child in progress ends `client_disconnect` too (RS-25, Vision §53: "ending the Session ends its children"). Device-paced Sessions run no child (KC-37a).
- **`serve` (scripted input) keeps today's semantics**: it reads on the calling thread, sees the end only after the frames before it, and has no watch. Every test that feeds `serve` a byte slice (`crates/ezsdr-server/tests/protocol.rs:169-171`, `:1299`) is unchanged. Two entry points rather than one, because a byte slice reaches its end before the first request is served: a watched read of it would end every scripted Session at once.
- **Detached Lease refused** (Q3): `connect` refuses `lease` whose mode is not Attached with `refused` ("EA-10: this server ends its Session when its input ends (EA-15), so nothing could adopt a Detached Lease"), before any directory or Run exists.
- Not changed: framing, replies, `Drop for Server`. A reply written to a client that closed its output fails with `BrokenPipe` (Rust ignores SIGPIPE: probe B2), `serve` returns the error and `Drop for Server` finishes the Session as today (`lib.rs:395-401`). Today the binary then prints `ezsdr-server: Broken pipe (os error 32)` on the user's terminal and exits 1 (`main.rs:33-36`), which after every Ctrl-C reads like a failure. One line in `main.rs`: a `BrokenPipe` is the client's departure, exit 0 without a message. (Doing it in `serve` instead would change `ea_15_every_exit_writes_the_manifest`'s `is_err()` assertion, `protocol.rs:1299`.)

Estimate: ~70 lines of production code.

### 2.3 Python: one session for the server, and "an interrupted exchange is a departure"

- `Connection.__init__`: `subprocess.Popen(args, stdin=PIPE, stdout=PIPE, start_new_session=True)` (`_client.py:67`).
- `Connection.call`: any exception between writing the request and reading the whole reply (`KeyboardInterrupt`, `SystemExit` from a user's SIGTERM handler, a dead server's EOF, a malformed reply, `MemoryError`) **abandons** the connection: close the server's stdin *and* stdout at once, mark it, re-raise. Stdout too, because a server writing a reply larger than the pipe buffer (a `read` body, a long `events`) would otherwise block in `write` while the client waits for its exit, and the cleanup would not run (that thread is the one that runs it). Error *replies* (`_raise`, `_client.py:92-94`) come after the whole reply is read and abandon nothing. Later calls raise `ProtocolError` ("the connection was closed when a call was interrupted; the Session has ended").
- `Session.close`: after an abandon it sends no `finish`. It waits for the server to exit (≤ 30 s, today's `Connection.close`, `_client.py:97-111`), then keeps `<dir>/manifest.json` when it can read it (a server on the same host). After the client's own interruption it does not raise when the file is not readable (a server on another host: the Manifest is on that host); after the server's death it raises `ProtocolError` as today. Fix the order in the existing fallback too: wait for the exit, then read (`session.py:834-840`).
- `connect()` already closes on any exception (`session.py:978-982`); with the abandon, an interrupted `connect` leaves no Session object and a Manifest `client_disconnect` in `runs_dir`.

Consequence to document (Q4): the server and an ssh wrapper no longer share the terminal, so the wrapper must not prompt (BatchMode, an ssh agent, known hosts) — the bench wrapper already runs `ssh -T -o BatchMode=yes` (`plan/phase8/tools/ssh-drop-tx.sh:136`).

Estimate: ~40 lines.

### 2.4 Bench tooling

`ssh-drop-driver.py`: `EZSDR_DROP_WAIT=time|sleep` (default `time`, today's `time.sleep(hold)` at `:68`); `sleep` calls `sdr.sleep(hold)`. `ssh-drop-tx.sh` passes it through. Verdict lines unchanged (C1–C3). Run at `8dd1016` (expected: C1 FAIL, server still in `advance` 15 s after the drop; C2 FAIL, tone on) and then at the new commit (expected: PASS), on (i)'s splitter wiring, after the owner confirms the attenuator. Optional rows in Q6.

### 2.5 Documentation

EA-10, EA-15, EA-16, KC-36, KC-46a, the two test tables and Changes tables (§6.2); a new KNOWN_LIMITATIONS entry KL-43 for the departures the server cannot see (§6.3); the (i) procedure in `plan/phase8/bench.md`. A mechanism label for #89 (none of `plan/maintenance/failure-mechanisms.md`'s fits; suggestion: `mech:client-absence-unseen-while-blocked` — "the frontend learns of its client only where its one thread reads the client, so a request that waits in Run time hides the departure").

---

## 3. The user-facing contract

Terms: **D** = the time until the server's input ends after the client goes; **W** = the time until the waiting call returns after the signal; **S** = `Provider::stop` (UHD: books the end at its instant, sends end of burst, waits ≤ 100 ms of device host clock for the last `BURST_ACK`, joins its threads ≤ 1 s each — UR-26, UR-16, `design/18-uhd-radio.md:220, 310`; "about 3 s at worst", `design/06-kernel-coordinator.md:440`). The tone stops about 10 ms (the in-flight window) after an end is booked: 10.27–11.35 ms from a `Stop`'s submission to silence (`plan/phase8/bench-results.md:50`), VERIFIED for a `Stop` Action; INFERRED for `Provider::stop`, which books its end the same way (UR-26).

"Manifest" below means `<dir>/manifest.json` with `termination.reason`, the Lease `{ attached, released: true }`, and the UHD Provider's `timing` row `{ what: stop, at, done }` (`crates/ezsdr-radio-uhd/src/provider/mod.rs:717`).

| Way of leaving | Idle (between requests) | In a waiting request: `advance`/`sleep`/`wait_until`, `wait_for`, `capture`/`result`, `submit` (KC-21a) | In `connect` | In `finish` |
|---|---|---|---|---|
| **The input ends**: pipe closed, client exits or is killed (SIGTERM's default, SIGKILL, a crash), an ssh drop sshd sees | Ends at once. Bound D + S. Manifest `Stopped { client_disconnect }`; exit 0. (As today.) | The request is cut. Bound D + W + S, W ≈ µs (`next_wakeup`) or ≤ 10 ms (KC-21a). Reply `ended` with the termination (nobody reads it; a client that closed its output makes the write fail, and the binary exits 0 all the same, §2.2). Manifest `client_disconnect`; `termination.at` before the request's target. **New.** | `connect` completes (no burst yet, so no RF, INFERRED), then the Session ends. Bound: the rest of `connect` (device open, KC-12a's budgets, the start lead). Manifest `client_disconnect`. | `finish` completes. Manifest `Stopped { client }` (the client asked first). |
| **Ctrl-C in a terminal** (SIGINT to the foreground process group) | Reaches only Python (the server is in its own session, probe B1). Then as the next row. Today: the server dies at once, no cleanup, no Manifest. | same | same | same |
| **SIGINT to Python only** (`KeyboardInterrupt`; an IDE's interrupt, `kill -INT`) | Not inside a call: the exception leaves the `with` block; `close()` sends `finish`. Manifest `client`, returned in-band. (As today.) | Inside a call: the client closes the server's pipes → "the input ends" row. The user sees `KeyboardInterrupt`; `close()` sends no `finish`, waits ≤ 30 s, and `manifest` holds the Manifest (`client_disconnect`) when the server is on the same host, else `None`. No `KeyError`. **New.** | `connect()` re-raises; no Session object; Manifest `client_disconnect` in `runs_dir`. | `close()` re-raises; the server completes `finish`: Manifest `client` on disk, `manifest` stays `None`. |
| **Python exception inside `with`** (not inside a call) | `close()` → `finish` → Manifest `client`; the exception then propagates. (As today.) An exception *inside* a call is the row above. | | | |
| **SIGTERM to Python** | Default: Python dies → "the input ends". With a user handler that raises: as `KeyboardInterrupt`. | | | |
| **SIGKILL of the client** | "The input ends" (the server, in its own session, survives and cleans up). | | | |
| **Ctrl-Z** (SIGTSTP) | Stops only Python; the Session lives (the client has not left); it ends when the client resumes and leaves, or dies. | | | |
| **SIGKILL or crash of the server**; a signal sent to the server itself (SIGTERM, SIGINT, SIGHUP; `docker stop` of its container) | No cleanup, no Manifest. The transmit stops when the device runs out of samples (INFERRED: a repeat is host-fed, KL-5, `KNOWN_LIMITATIONS.md:47`); LO leakage may remain (INFERRED, `plan/phase8/bench.md:366`). Python: the call in flight or the next one raises `ProtocolError`; `close()` raises `ProtocolError` (no Manifest). Unchanged; KL-43. | | | |
| **Silent network partition over ssh** (laptop asleep, Wi-Fi or VPN lost: no FIN/RST) | No end of input until sshd gives up on the client: `ClientAliveInterval × ClientAliveCountMax` when set, otherwise TCP keepalive, about 2 h 11 min with Linux defaults (INFERRED, sshd_config(5)). Then "the input ends". The transmit goes on meanwhile, idle or not. Unchanged; KL-43. | | | |

**Simulation class** (Q2, recommended): the same rows, with W = one round (the loop reads the end every iteration) and a child Run in progress cut too (its Manifest `client_disconnect`). No RF. The instant a Session abandoned mid-request ends at depends on when the server saw the end; it is recorded in `termination.at`.

---

## 4. Alternatives

| | (a) ssh drop during `sleep` | (b) group Ctrl-C | (c) SIGINT to Python only | Lives in | Cost | Verdict |
|---|---|---|---|---|---|---|
| **R** Kernel disconnect signal + server input thread + Python session/abandon | ✓ at once | ✓ | ✓ | Kernel (1 method), server, Python | ~125 lines production, ~450 tests (estimates) | **recommended** |
| **B** Server slices device-paced `advance`/`wait_for` into ≤ 100 ms and polls a flag between slices (audit) | ✓ ≤ 100 ms, device-paced only | needs Python's part | needs Python's part | server | ~90 lines; a second copy of KC-29/KC-29b's wait (the horizon kept across slices, `until` semantics, the final `now`); a control round and an Authority `schedule`/`cancel` every slice; KC-21a's wait (≤ 5 s) not sliced; Simulation never cut; each future frontend (CLI, MCP, a listener) copies it | rejected: same reader thread, more code, a rule implemented twice (AGENTS §6) |
| **C** Kernel cancellation token: a wait returns early without ending the Run | ✓ | Python's part | Python's part | Kernel | new outcome "returned early, standing before its target" in KC-29/KC-29b, new API, the server still ends the Run itself | rejected now: an interrupt that keeps the Session is not asked for; additive later (§8) |
| **D** Server signal handlers (ignore SIGINT; SIGTERM/SIGHUP → disconnect) | ✗ | ✓ | ✗ | server | `unsafe` in a `forbid(unsafe_code)` crate, or `signal-hook`; still needs the cross-thread end of R or B | rejected for #89: `start_new_session` covers (b) with no server code; KL-43 names signals to the server |
| **E** Protocol heartbeat: the client pings every T, the server ends the Session after 3T of silence | ✓, silent partitions too | Python's part | Python's part | protocol, every client, server | a ping frame outside the request/reply alternation, a client timer thread with a lock on stdin, a server timer; every client carries it | rejected now (Q5): ssh has this already (`ClientAliveInterval`); local pipes never fail silently |
| **F** Python only: slice `sleep`, `start_new_session`, close stdin on interrupt | ✗ | ✓ | the server still transmits until the request ends (it does not read its input) | Python | small | rejected alone (the audit's reason, agreed); its two cheap parts are in R |
| **G** Fail-stop: the input thread exits the process at EOF | ✓ (starvation) | — | ✓ | server | ~5 lines | rejected: no end of burst, no Manifest (EA-15's promise), RF stop by starvation INFERRED |
| **H** TTL on Attached Leases | ✓ after the TTL | | | Kernel | the Lease's meaning changes | rejected (the audit's reason, agreed) |
| **K-c** The signal itself performs RS-6 steps 1–2 on the calling thread (KC-46b-like) | ✓ | | | Kernel | a third thread inside cleanup; KC-46b's receive-sample loss | rejected: the server always follows with a control-thread call, so no case needs it |

Challenge to the audit: its own fix is B. R moves the one decision ("the client has gone, end the Run") to where the Lease and the end live (KC-32, KC-36), reuses the existing wake, needs no slicing, and serves every frontend once.

---

## 5. Principles that decide the cases

1. **The client owns the end while it is present; its absence is a Lease event the Kernel acts on** (RS-23, KC-36). The server only reports the absence (EA-1: it holds no Run semantics).
2. **The server's only departure signal is the end of its input** (and a reply it cannot write). A signal is not a departure: a signal to the client becomes the client's departure (Python closes the connection); a signal to the server is the server's death, outside the contract (KL-43).
3. **An exchange that cannot complete is a departure.** A client that lost a reply leaves; it never resynchronises. This is what makes a stale reply impossible.
4. **Runs end on the control thread** (KC-38). Other threads only request the end and wake the control thread (KC-46a): one cleanup path, the one a client's `finish` takes.
5. **One rule in every class** (Q2): an absent client ends the Session at the next round. In Simulation the end instant is recorded, not reproduced; the Run's outputs up to it are as deterministic as before.
6. **The RF bound has one unbounded term, the transport's detection** (an ssh silent partition). It is configured (sshd), not coded, and stated (KL-43).
7. **Never trade the orderly stop for speed** (rejects G): the end of burst and the Manifest are the point of EA-15.

---

## 6. Against the existing promises

### 6.1 What holds unchanged

- **EA-2** one request at a time: unchanged; the input thread reads ahead only to see the end, and a frame that arrives during a request waits its turn. **EA-5**: a cut request is the existing `ended` kind. **EA-11**: a cut `submit` already answers `ended` (`lib.rs:414-436`). **EA-12/EA-17**: `advance`, `wait_for`, `capture`, `result` cut → `ended`. **EA-13**: unaffected.
- **RS-6…RS-11a, RS-8a**: the same cleanup, the same 5 s step deadline (`crates/ezsdr-kernel/src/run.rs:545, 572-591`). **RS-23**: holds, now also during a request. **RS-25**: a child in progress ends with its parent's client.
- **KC-21a** stops its wait on an end request (`paced.rs:186`). **KC-29/KC-29b** already return `Err(Ended)` when the Run ends during the wait. **KC-32/KC-33/RS-10**: the first end stands; `client_disconnect` is already a listed end. **KC-46b/KC-46c**: unchanged.
- **UR-16/UR-26**: `Provider::stop` unchanged.
- **PO-11/GZ-4**: no Kernel thread, no wall clock. **OV-23**: no new module-level item.
- **Vision §53** "TX must not continue indefinitely merely because a client disappears" (`design/vision/09-provenance-validation-and-ownership.md:121`): now true for every departure the server can see; KL-43 names the rest.
- **RS-42** (`design/04-run-and-session.md:304`): a Simulation Session cut mid-request keeps `deterministic` (Q2); its outputs up to `termination.at` are reproducible, and `termination.at` is recorded.

### 6.2 Every rule text that changes (drafts; English as the specs)

Format constraints: `design/06` is in DA-1 form (rule text normative only, an indented `*Test:*` line, history in its Changes table); `design/16` keeps its inline history, and `plan/maintenance/tools/rules.py` fails if its lines over 1 500 characters or its history markers (`(owner, …)`, `#89`, `(Review …)`) grow outside the Changes table (`rules.py:124-128, 45-47`). EA-15 is 870 characters (`design/16-easy-api.md:81`): the new text goes on an indented continuation line, and `#89` only in the Changes rows.

1. **KC-36** (`design/06-kernel-coordinator.md:396`), after "`check_lease()` performs the check alone.":
   > `disconnect_signal()` returns a function that any thread may call any number of times and that does not keep the Run alive. For an Attached Lease it requests `Stopped { client_disconnect }` in orderly mode and, in a device-paced class, when that request set the Run's end, wakes the control loop unconditionally (KC-46a); for a Detached Lease, or once the Run is gone, it does nothing. It runs no cleanup: the `RunHandle` call in progress, whose wait then returns, or the next call does (KC-38).

   *Test:* line adds `kc_36_a_disconnect_signal_ends_a_waiting_advance`, `kc_36_a_disconnect_signal_ends_a_submit_waiting_for_its_provider`, `kc_36_a_disconnect_signal_ends_a_simulated_advance`, `kc_36_a_disconnect_signal_does_nothing_for_a_detached_lease_or_an_ended_run`.
2. **KC-46a** (`:436`), append: "KC-36's disconnect signal wakes the control loop the same way, unconditionally, from the thread that reports the disconnect."
3. **design/06** test table rows for the four tests; Changes row (#89).
4. **EA-15** (`design/16-easy-api.md:81`), new continuation line:
   > The binary reads its input on a thread of its own (`serve_watched`), so it sees the input end while it serves a request too: it calls the disconnect signal (KC-36) of the Session's Run and of a child Run it is running, the request in progress returns as for an ended Run (`ended` with `Stopped { client_disconnect }`; a submit as EA-11 says; `run_child` with the child's Manifest), and the server then finishes as above. `serve` reads on the calling thread, for an input that holds every request (a test's): it sees the end after the requests before it.

   *Checked by* adds `ea_15_an_input_that_ends_during_an_advance_ends_the_session`, `ea_15_an_input_that_ends_during_a_simulated_advance_ends_the_session`, `ea_15_an_input_that_ends_during_a_child_run_ends_the_child`, `ea_15_a_watched_input_that_ends_between_requests_ends_the_session`.
5. **EA-10** (`:69`), in the `lease` clause: "`lease` defaults to `Lease::attached()`; a Detached Lease is refused with `refused` before any Run exists: this server ends its Session when its input ends (EA-15)." Test `ea_10_a_detached_lease_is_refused` (Q3).
6. **EA-16** (`:89-90`): the `connect(...)` row adds "in a session of its own, so that a signal to the client's process group (a terminal's Ctrl-C) does not reach the server; a server wrapper must not prompt"; the `Session` row adds "A call that ends while its reply is awaited — `KeyboardInterrupt` or any other exception — closes the server's input and output at once, which ends the Session (EA-15), and re-raises; later calls raise `ProtocolError`, and `close()` sends no `finish`: it waits for the server to exit and keeps `<dir>/manifest.json` in `manifest` when it can read it, else leaves `None`." Tests named in §9.
7. **design/16** §5 test table rows and Changes row.
8. **KNOWN_LIMITATIONS.md**: KL-43 (§6.3).
9. **plan/phase8/bench.md** (i): the `EZSDR_DROP_WAIT=sleep` variant and its expected FAIL/PASS.

Not changed: RS-21…RS-25, RS-42 text, KC-29/KC-29b/KC-33/KC-46b/KC-46c, UR-*, MA-*, the Vision.

### 6.3 KL-43 (draft)

> ### KL-43 Departures the server cannot see
> - **Symptom:** the transmit goes on after the client has gone, until the server sees its input end: over ssh when the network fails silently (a laptop asleep, Wi-Fi or a VPN lost); a server process that is itself killed or signalled (`kill`, `docker stop`, the OOM killer) ends with no cleanup and no Manifest, and the device stops when it runs out of samples. After an interrupted call to a server on another host, `manifest` is `None`: the Manifest is `dir/manifest.json` on that host.
> - **When:** remote use with sshd's default `ClientAliveInterval 0` (then TCP keepalive decides, about 2 h 11 min with Linux defaults); any signal sent to `ezsdr-server` rather than to its client.
> - **Workaround:** on the server's host set `ClientAliveInterval 5` and `ClientAliveCountMax 3` (sshd_config); end a Session by ending its client (Ctrl-C, `kill <client>`), never by signalling the server.
> - **Evidence:** EA-15 (`design/16-easy-api.md`), KC-36 (`design/06-kernel-coordinator.md`); `plan/phase8/bench-results.md` "2026-11-09" (i) for a drop sshd sees.
> - **Status:** Ceiling (a transport's liveness is the transport's). INFERRED: sshd's behaviour and the stop by starvation.

---

## 7. Evidence

| # | Fact | Mark | Source |
|---|---|---|---|
| E1 | The server reads its input only between requests; `advance`/`wait_for` block in `next_wakeup`; KC-21a's wait rechecks the end every 10 ms | VERIFIED (code) | §1 citations |
| E2 | A `schedule` from any thread ends the UHD Authority's `next_wakeup` wait at once (notify generation) | VERIFIED (code) | `authority.rs:284-310, 523-555`; `host.rs:336-372` |
| E3 | Disconnect → server exit < 1 s and the independent receiver quiet at +0 s (1 s resolution), **idle server**, #1 OBX, one trial | VERIFIED | `plan/phase8/bench-results.md:385`; `bench/2026-11-09-stage5/results/verdicts.tsv:106-108` |
| E4 | From a `Stop`'s submission to silence: 10.27–11.35 ms; device stop within −22…+56 µs of the recorded end | VERIFIED (Stop Action) | `plan/phase8/bench-results.md:50, 384` |
| E5 | `Provider::stop`'s own duration on the X300 | **not read here**: every UHD Manifest records `timing { what: stop, at, done }` (`provider/mod.rs:717`); the 2026-11-09 (i) Manifest is in the private raw archive (`bench/2026-11-09-stage5/raw.txt`). Cheap to read before the brief. | — |
| E6 | `Provider::stop` bound: ≤ 100 ms ACK window + 1 s per join; "about 3 s at worst" | VERIFIED (spec) | UR-16, UR-26 (`design/18-uhd-radio.md:220, 310`); KC-46b (`design/06-kernel-coordinator.md:440`) |
| E7 | A SIGINT to the client's process group kills a same-group child that has no handler (exit −2) and does not reach a child started with `start_new_session=True` (it exits 0 at end of input) | VERIFIED (probe B1, Linux, Python 3.14) | Appendix B |
| E8 | A Rust reader thread sees stdin's end while the main thread is busy; a reply written to a closed stdout returns `BrokenPipe` (no SIGPIPE death) | VERIFIED (probe B2, rustc 1.99.1) | Appendix B |
| E9 | When the host process dies uncleanly, the X300 stops a repeat once it runs out of host samples; LO leakage may remain | **INFERRED**: a repeat is streamed from host memory (KL-5); no bench record of an unclean death (searches, Appendix A); "a stopped tone does not separate a clean end of burst from FPGA starvation" (`plan/phase8/bench.md:366`) | Q6 offers a row that measures it |
| E10 | sshd gives a non-pty command end of input on a drop it sees | VERIFIED for the bench's `ssh -T … docker run -i` wrapper (E3) | `ssh-drop-tx.sh:136-139, 215` |
| E11 | On a silent partition sshd notices only through `ClientAlive*` (default off) or TCP keepalive (Linux default 7200 s + 9 × 75 s) | INFERRED (sshd_config(5), Linux sysctl defaults; not run) | Q6 offers a `kill -STOP` row |
| E12 | Jupyter's interrupt signals the kernel's process group, so today it kills the server too | INFERRED (`jupyter_client` is not installed here) | — |
| E13 | Nothing transmits during `connect` before the first burst | INFERRED (no burst is submitted; UR-15 starts the streamers only) | — |

---

## 8. What the choice makes impossible or harder later

- **A Detached Lease through the stdio server** (Q3 refuses it). A server that outlives its client (a daemon, a listener) can accept it; the Kernel keeps Detached, Adopt and Renew unchanged.
- **Keeping a Session across an interrupt** (Jupyter "interrupt" that only stops waiting): not impossible — the input thread can see a future `interrupt` frame, and alternative C adds the Kernel side — but it is a protocol change then, not now.
- **Pipelining**: a client that sends requests ahead (outside EA-2) and closes its input may find the unserved ones answered `ended`.
- **Interactive ssh prompts** from the server wrapper: impossible once the server has its own session (Q4); the wrapper must be non-interactive.
- **Simulation**: a Session abandoned mid-request ends at an instant that depends on host timing (recorded; Q2).
- **Multiple clients per Session**: not made harder; a multi-client server fires the signal when the Lease holder leaves.
- **Signals to the server**: adding a handler later (alternative D) needs only to fire the same watch.

---

## 9. Test approach

No verdict depends on wall-clock time: each asserts the cause, the reply kind, and `termination.at` against the request's target in **Run time**. A broken cut shows up as a hang, which the existing guards turn into failures (the server tests' watchdog, `crates/ezsdr-server/tests/protocol.rs:32-34`; a `recv_timeout` around the waiting call in the Kernel tests, as `device_paced.rs:933` does; `timeout=` on Python subprocesses). Where a test fires the signal "while the call waits", the order does not change the verdict: a signal before the call makes the call answer `Ended` at once.

**Kernel** (`crates/ezsdr-kernel/tests/`):
- `kc_36_a_disconnect_signal_ends_a_waiting_advance` (`device_paced.rs`, `WallAuthority` + `ThreadedProvider`): `advance_to(T0 + 1 h)`; another thread fires the signal → `Err(Ended { Stopped { client_disconnect } })`; `radio:stop` in the probe; `termination.at` < target.
- `kc_36_a_disconnect_signal_ends_a_submit_waiting_for_its_provider` (`ThreadedProvider::…deaf()`, as `rs_16_an_action_the_freeze_discards_is_undelivered`, `device_paced.rs:811-824`): the signal during KC-21a's wait → the entry logged, `termination.undelivered` = its ids, termination `client_disconnect` (not KC-21a's `Failed` after 5 s).
- `kc_36_a_disconnect_signal_ends_a_simulated_advance` (a Session whose Provider always has a next wakeup): same verdict, `termination.at` < target.
- `kc_36_a_disconnect_signal_does_nothing_for_a_detached_lease_or_an_ended_run`: Detached → `advance_to(near)` is `Ok`; after `finish` and after the handle is dropped, calling it is a no-op (no panic).

**Server** (`crates/ezsdr-server/tests/protocol.rs`, through `serve_watched` over `std::os::unix::net::UnixStream::pair()`; the test writes frames, `shutdown(Write)`s, and still reads the replies):
- `ea_15_an_input_that_ends_during_an_advance_ends_the_session` (FakeDevice through `Config.open_device`, keeping the `Arc<FakeDevice>`): `connect` with a transmit stream, `start_repeat`, `advance { by: 3600 s }`, end the input → the reply is `ended` with `client_disconnect`; `serve_watched` returns `EndOfInput`; the Manifest says `client_disconnect` with `termination.at` < target; the fake's `calls()` (`crates/ezsdr-radio-uhd/src/device.rs:1014`) show the end-of-burst send and `close_streams` after the repeat. This is the FakeDevice rehearsal of the bench row (AGENTS §9, hardware tests).
- `ea_15_an_input_that_ends_during_a_simulated_advance_ends_the_session` (default profile with rx0 streaming, so the advance cannot finish by itself).
- `ea_15_an_input_that_ends_during_a_child_run_ends_the_child` (a child with a huge `duration_ns`): the child's Manifest and the Session's say `client_disconnect`.
- `ea_15_a_watched_input_that_ends_between_requests_ends_the_session` (today's EOF case, through the watched path).
- `ea_10_a_detached_lease_is_refused` (Q3): `refused`, no Session directory.
- Unchanged: every `serve`-over-bytes test (`ea_02_framing`, `ea_05_…`, `ea_15_finish_and_disconnect`, `ea_15_every_exit_writes_the_manifest`); `ea_binary_speaks_the_protocol` (`protocol.rs:1223`) now exercises the watched binary and still passes (request, reply, `finish`).

**Python** (`python/tests/test_easy_api.py`):
- `test_ea_16_an_interrupted_call_closes_both_pipes_and_close_sends_no_finish` (a stand-in process whose `stdout.readline` raises `KeyboardInterrupt`): both pipes closed, no `finish` written, later calls raise `ProtocolError`, `close()` waits and loads the `manifest.json` the test placed.
- `test_ea_16_an_interrupted_sleep_ends_the_session` (real server, `connect(rx={0: [0]})`, stdout wrapped so the first `readline` after an `advance` raises `KeyboardInterrupt`): `close()` returns a Manifest `client_disconnect`, no `KeyError`, the server exited 0 (with the `main.rs` line of §2.2).
- `test_ea_16_the_server_runs_in_a_session_of_its_own`: `os.getsid(server pid) != os.getsid(0)`.
- `test_ea_16_ctrl_c_to_the_client_group_does_not_kill_the_server`: a child script in its own session connects, prints `dir`, calls `sdr.sleep(10**6)` with rx streaming; the test `os.killpg(SIGINT)`s the child's group; `manifest.json` exists with `client_disconnect` (or `client`, if the signal came before the request was sent — accepted; what matters is that the server lived to write it).
- Unchanged: `test_ea_16_close_after_the_server_exited` (`:954-961`) passes with the wait-then-read order.

**Mutation rows** (examples; token anchors for Rust, exact substrings for Python): the signal requests nothing; no wake; acts on a Detached Lease; the input thread does not fire; the child's signal not added; Detached not refused; `start_new_session` dropped; no abandon on exception; `finish` sent after an abandon; stdout left open on abandon.

**Bench** (owner, 2026-11-11): (i) with `EZSDR_DROP_WAIT=sleep` at `8dd1016` (C1/C2 FAIL expected) and at the new commit (PASS). Optional rows in Q6.

---

## 10. Risk and staging

**Risky** (AGENTS §9 step 1: a lifecycle — the end of a Run requested from another thread). Steps 2–3 apply (test plan, Sol's plan review).

**Sequences for the brief** (each with its expected result):
1. Device-paced `advance_to(far)` waiting in `next_wakeup`; signal → `Err(Ended{client_disconnect})`, `Provider::stop` once, Manifest, `termination.at` < far.
2. Device-paced `submit` waiting in KC-21a (Provider deaf); signal → entry logged, ids in `undelivered`, `client_disconnect`.
3. Idle Session; signal → nothing until the next call; that call (or `disconnect`/`finish`) cleans up; cause `client_disconnect`.
4. Another end first (Policy abort, scheduled `Stop {}`), then the signal → the first end stands, nothing in `also`.
5. Signal during `finish` → `Stopped { client }`.
6. Signal on a Detached Lease; after cleanup; after the handle is dropped; twice → no effect.
7. Simulation `advance_to(unreachable)` + signal → `Err(Ended{client_disconnect})`; a child in progress + both signals → the child ends `client_disconnect`, then the parent.
8. Watched server: input ends during `advance` on the FakeDevice (repeat playing) → `ended`, Manifest, EOB and `close_streams` in `calls()`.
9. Watched server: input ends during `connect` → `connected` (or `ended` on the way to T0), Manifest `client_disconnect`.
10. Watched server: `finish` replied, then input ends (Python's normal `close`) → `Stopped { client }`.
11. Python: exception while a reply is awaited → both pipes closed, no `finish`, Manifest loaded; exception outside a call → `finish`, `client`.
12. Python: group SIGINT → the server survives and writes its Manifest.
13. Bench: ssh drop during `sdr.sleep` → C1–C3 PASS (FAIL at `8dd1016`).

Hardware behaviour relied on: E2 (code, VERIFIED), E3/E4 (bench, VERIFIED), E6 (spec), E9/E11 (INFERRED, not relied on for a verdict).

**Staging** on `work/n89`, one local commit per sub-step (AGENTS §9):
- **S1 Kernel**: `disconnect_signal`, KC-36/KC-46a text, the four Kernel tests, rows.
- **S2 Server**: `serve_watched`, the watch, registration (Session and child), the Detached refusal, `main.rs` (the watched input, a `BrokenPipe` exit 0), EA-10/EA-15 text, the server tests, rows.
- **S3 Python and tooling**: `start_new_session`, abandon, `close`, EA-16 text, KL-43, the Python tests, the driver variant, `bench.md` (i).
Then the full mutation lists, Sol's code review, the gate, and the bench pair. Estimated size: ~125 lines of production code and ~450 of tests.

---

## 11. Owner decisions (one at a time; recommended answer first)

1. **Direction.** Kernel disconnect signal + server input thread (R), or server-side slicing with no Kernel change (B)? **Recommended: R** — one decision in the place that owns the Lease and the end, no second copy of the wait, prompt in every wait; B's only advantage is "no Kernel change", and the change is one method with no new type.
2. **Simulation.** Cut a Simulation request too (the end instant then depends on host timing, recorded), or end Simulation Sessions only between requests (as today)? **Recommended: cut** — one rule for every class, no CPU left running for a client that has gone (KL-19: a coupled simulation runs at about real time), and the cost is confined to the recorded end instant of an abandoned Session. If not: the signal is a no-op in Simulation and the child registration is dropped.
3. **Detached Lease through this server.** Refuse it at `connect` (`refused`), keep accepting it (it ends `Stopped { client }` at end of input, and #89's gap remains for it), or delete `lease` from `connect` and Python? **Recommended: refuse** — one check; the field can be deleted later if nothing needs it.
4. **Python.** `start_new_session=True` (a terminal's Ctrl-C reaches only the client; an ssh wrapper must not prompt) and "an interrupted call ends the Session" (no interrupt that keeps it)? **Recommended: yes to both.**
5. **What stays outside the bound.** A silent ssh partition and signals sent to the server itself go to KL-43, with `ClientAliveInterval 5` / `ClientAliveCountMax 3` on the lab host's sshd (applied as a setup script) — no protocol heartbeat, no server signal handler now? **Recommended: yes.**
6. **Bench.** Besides the requested `sdr.sleep` pair (FAIL at `8dd1016`, PASS after): add (a) `docker kill` of the server during a repeat — it measures E9, the X300 after an unclean host death, the one INFERRED fact behind the SIGKILL row; (b) `kill -STOP` of the local ssh client during `sdr.sleep`, with and without sshd's `ClientAlive*` — it measures KL-43's bound? **Recommended: (a) yes, same rig, a few minutes; (b) once sshd is configured.**

---

## Appendix A. Searches (for the negative results)

- Detached Leases in the server and Python: `grep -rn "detached\|Detached" crates/ezsdr-server/ python/ --include=*.rs --include=*.py` → no hits. Kernel-only uses: `grep -rln "Detached\|detached(" crates/*/tests crates/*/src` → `crates/ezsdr-kernel/{src/run.rs, src/session.rs, src/coordinator/state.rs, src/coordinator/mod.rs, tests/device_paced.rs, tests/run_session.rs, tests/coordinator.rs, tests/kernel_surface_allow.txt}`.
- Unclean host death on the bench: `grep -n "SIGKILL\|kill -9\|kill\b\|ssh 切断\|ssh drop\|ssh-drop\|starv\|LO leak\|ATR\|(i)\|C1\b\|C2\b\|2026-11-09" plan/phase8/bench-results.md` → only (i) on 2026-11-09 (idle server, PASS) and the unrun (f)/(g) of 2026-11-05/06; `grep -n "kill\|SIGKILL\|プロセスが落\|process.*died\|host.*dies\|飢え\|starv\|DmaFIFO\|DMA FIFO\|dma_fifo" plan/phase7/bench-results.md plan/phase7/bench.md plan/phase8/bench.md plan/spikes/*.md` → no record of the device after the host process dies (the hits are a SIGSEGV of two tests opening one device, a SIGABRT on a lost link, and the "stopped tone ≠ EOB vs starvation" caution).
- A KL entry for #89: `grep -n "89\|disconnect\|Ctrl-C\|interrupt\|sleep" KNOWN_LIMITATIONS.md` → none (GitHub-tracked defects are not listed there, `KNOWN_LIMITATIONS.md:3`).
- Signals anywhere in the code: `grep -rln "SIGKILL\|kill -9\|kill -KILL\|SIGINT\|SIGTERM\|start_new_session\|KeyboardInterrupt" --include=*.md --include=*.py --include=*.rs --include=*.sh .` (excluding `v3/`) → only bench tools and review records; no handler in the server, nothing in `python/`.
- Jupyter: `import jupyter_client` fails in `~/.cache/ezsdr-venv/bin/python` and `python3` (E12 stays INFERRED).
- A provider-stop duration on the bench: `grep -n "cleanup took\|stop took\|finish took\|Provider::stop.*ms\|stop.* ms\b\|finish.* s\b" plan/phase7/bench-results.md plan/phase8/bench-results.md` → only `hw_b8_stop_end` (a `Stop` Action, E4).

## Appendix B. Probes (session scratchpad, not the tree)

**B1** `group_sigint.py`: a client (in its own session, so the probe's runner is not hit) starts `cat` as its "server", with and without `start_new_session`, and receives `os.killpg(SIGINT)`:
```
--- old: client pid=211064 pgid=211064 sid=211064; server pid=211065 pgid=211064 sid=211064
client: KeyboardInterrupt; server alive after the group SIGINT: True (returncode None)
server exit after its input ended: -2
--- new: client pid=211066 pgid=211066 sid=211066; server pid=211067 pgid=211067 sid=211067
client: KeyboardInterrupt; server alive after the group SIGINT: True (returncode None)
server exit after its input ended: 0
```
(-2: killed by the group SIGINT; the `poll()` just after the signal had not yet reaped it.)

**B2** `probe.rs` (rustc 1.99.1) + `drive.py`: a stdin reader thread sets a flag at end of input while the main thread waits in a 60 s "request" checking it every 10 ms; the client closes both pipes after 0.3 s:
```
server exited 0 0.007 s after the client closed its pipes
probe: request ended after 306.517637ms, gone=true
probe: reply write -> Err(BrokenPipe)
```
