# Phase 6 spec 16 — The Easy API: the server, its protocol and the Python package

| Field | Value |
|---|---|
| Status | Draft, implemented under the owner's delegation; awaiting Gate X ([`00-overview.md`](00-overview.md) §11). Moves to `design/16-easy-api.md` at Step X. |
| Scope | The frontend that makes Vision §3's Easy API real: the server `ezsdr-server` 0.1.0 (a Rust binary that compiles the Modules in and runs one Session), the protocol `ezsdr.protocol` 1 between a client and the server, and the Python package `ezsdr` 0.1.0. |
| Not in scope | A remote listener, server-owned profiles and authentication (Phase 7); Session replay; a Spec builder; a CLI or MCP client ([`00-overview.md`](00-overview.md) §3). |
| Depends on | Specs 01–11 and 14 as amended; spec 17 (KF-1 `events`, KF-2 `wait_for`, KF-3 `run_child`, VD-1 `sink.CAPTURE_WRITTEN`). |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Rules are `EA-n`; decisions are `A1`…`A8` (§6).

---

## 1. The three parts

- **EA-1** The Easy API has three parts. The **server** `ezsdr-server` (crate `crates/ezsdr-server`) is the Runtime: it compiles the Modules in (Vision §62), builds each Run's `Assembly`, and drives one Session through the Kernel's `RunHandle`. The **protocol** `ezsdr.protocol` 1 carries the Kernel's own documents — `SessionAction`, `LogEntry`, `Event`, `Manifest`, `TimePoint`, `Lease`, BindingProfile and ExperimentSpec documents — between a client and the server. The **Python package** `ezsdr` (directory `python/`) is a client: it spawns the server, names Radio Model keys in Python terms and composes a capture from protocol requests. The Python package holds no Run semantics: every change it asks for is a `SessionAction` the Kernel admits and logs, and every advance of time is the server's (Vision §3: "nothing bypasses the log"). *Checked by the Python carriers of [`00-overview.md`](00-overview.md) §8, all of which read the logged entries back.*

## 2. The protocol

- **EA-2** **Transport and framing.** The protocol runs over one reliable, ordered byte stream in each direction; in Phase 6 that is the server's standard input (client to server) and standard output (server to client), and the server writes nothing else to its standard output. A **frame** is a header line — one JSON object in UTF-8, with no line feed inside it, terminated by one line feed (`0x0A`) — followed by exactly `body_bytes` raw bytes (none when it is 0). A client's header is `{ "request": <Request>, "body_bytes": <u64> }`, the server's `{ "reply": <Reply>, "body_bytes": <u64> }`; `body_bytes` may be omitted when 0, and an unknown field anywhere is refused (Vision §9). A header longer than 16 MiB is refused. The client sends one request and reads its reply before it sends the next. *Checked by `ea_02_framing`.*
- **EA-3** **Handshake and version.** The first request must be `hello { protocol }`. A server supports protocol 1 only: to `hello { protocol: 1 }` it replies `hello { protocol: 1, server: "ezsdr-server <version>", kernel: "<ezsdr-kernel version>" }`; to another version it replies the error `unsupported_protocol` with `supported: [1]` and exits; to any other first request it replies the error `protocol` and exits. A later protocol version is a new integer, and a server that speaks several lists them (Vision §9: refused, never reinterpreted). *Checked by `ea_03_handshake`.*
- **EA-4** **Requests and replies.** After `hello`, the requests and their replies are:

  | Request | Body | Reply | Rule |
  |---|---|---|---|
  | `connect { profile?, lease? }` | — | `connected { run, now, start_instant, dir, profile, effective }` | EA-10 |
  | `submit { action }` | the waveform, if the action carries one | `submitted { entry, now, events }` | EA-11 |
  | `advance { to? , by_ns? }` (exactly one) | — | `advanced { now, events }` | EA-12 |
  | `wait_for { kinds, from, within_ns }` | — | `waited { index?, event?, now, events }` | EA-12 |
  | `events { from }` | — | `events { events, next }` | EA-12 |
  | `status {}` | — | `status { run, state, now, effective, events }` | EA-12 |
  | `read { uri }` | — | `read { size }` | EA-13; the reply's body is the bytes |
  | `run_child { spec, profile?, inputs, duration_ns? }` | the inputs' bytes, concatenated in the order of `inputs` | `ran { entry, manifest?, path? }` | EA-14 |
  | `finish {}` | — | `finished { manifest, path }` | EA-15 |

  `events` in a reply is the number of events the Run has delivered so far, the index the next one will take. `connect` is valid once, after `hello`, until a `connect` succeeds; every other request needs a connected Session. *Checked by `ea_04_requests`.*
- **EA-5** **Errors.** A request that cannot be carried out gets `{ "reply": { "error": { "kind", "message", … } } }` and changes nothing, except as noted. The kinds are: `protocol` (a header that is not a valid frame, an unknown request, a body of the wrong length, a request out of order); `unsupported_protocol` (EA-3); `refused` (a `connect` or `run_child` whose documents the Kernel or the server refuses before a Run exists, with the Kernel's reason; a `connect` may be retried); `malformed` (`RunHandleError::Malformed`); `ended` (`RunHandleError::Ended`, with the `termination`); `not_on_primary_root`; `not_found` (EA-13); `io` (the server could not read or write a file). After a `protocol` error in the framing itself — a header that does not parse, or a stream that ends inside a body — the server cannot find the next frame and exits; after any other error the Session is as it was. *Checked by `ea_05_errors`.*
- **EA-6** **Schemas.** The frame types are Rust types in `ezsdr-server` with generated JSON Schemas committed at `schemas/server/request_frame.v1.json` and `schemas/server/reply_frame.v1.json`, frozen byte for byte by `ea_06_schema_freeze` (regenerated with `EZSDR_UPDATE_SCHEMAS=1`). A Kernel document embedded in a frame has the Kernel's schema; a BindingProfile or ExperimentSpec travels as the JSON document itself, because the Kernel hashes and parses the document as sent (KC-1). *Checked by `ea_06_schema_freeze`.*

## 3. The server

- **EA-7** **The catalogue.** The server compiles in and registers exactly: the Vocabularies `radio`, `sim`, `sink`; the Modules `ezsdr.sim-engine`, `ezsdr.radio.mock`, `ezsdr.link.host`, `ezsdr.sink.capture` and `ezsdr.exec.native`; and hands the native Executor the component implementations its embedding supplies (the binary supplies none; a test embedding may supply the Phase 5 responder). `ezsdr_server::assemble(profile, inputs, implementations)` builds a Run's `Assembly` from a BindingProfile document; a binding that names another Module, or a Module the catalogue cannot build from its binding, is refused with its reason. The acceptance rig uses the same function. *Checked by `ea_07_a_profile_naming_an_unknown_module_is_refused` and every carrier that assembles a Run.*
- **EA-8** **The Session directory.** The server takes `--runs-dir <path>` (default `ezsdr-runs`, relative to its working directory) and creates in it a fresh directory `session-<pid>-<n>` for its Session, `n` the smallest number from 0 that names no existing entry. The Session's Manifest is written there as `manifest.json` and each child's as `child-<seq>.manifest.json`, `seq` the parent's log entry. *Checked by `ea_08_the_manifests_are_written_to_the_session_directory`.*
- **EA-9** **The default profile.** `connect` without a profile uses this BindingProfile, with `<dir>` the Session directory:

  ```json
  { "version": 1,
    "bindings": {
      "radio": { "module": { "id": "ezsdr.radio.mock", "version": { "major": 1, "minor": 2, "patch": 0 } },
                 "selector": { "id": "radio" },
                 "profile": { "name": "x310-like", "version": { "major": 1, "minor": 1, "patch": 0 } } },
      "rec":   { "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 2, "patch": 0 } },
                 "selector": { "dir": "<dir>" },
                 "feed": { "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 64 } },
      "sim":   { "module": { "id": "ezsdr.sim-engine", "version": { "major": 1, "minor": 0, "patch": 0 } }, "selector": {} } },
    "authority": "sim",
    "placements": { "links": [ { "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
                                 "from": { "component": "radio", "port": "rx" }, "to": { "component": "rec", "port": "in" } } ] },
    "environment": {
      "ezsdr.time": { "class": "simulation", "start_lead_ns": 2000000000 },
      "sim.seed": 0,
      "sim.channel": { "couplings": [ { "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 } ] } } }
  ```

  It is data in the server, not in any client (§58 #10). *Checked by `ea_09_the_default_profile_loops_back`.*
- **EA-10** **`connect`.** The server builds the Assembly (EA-7) and calls `coordinator::connect(profile, assembly, lease)` (`lease` defaults to `Lease::attached()`). A `SpecError` is the error `refused`. If the Run is `Running`, the server advances it to `start_instant()` — T0, the instant its devices start — and replies `connected` with the Run's id, `now`, T0, the Session directory, the profile document it used and `effective()`. If the Run ended during its pipeline (a stage failed), the server finishes it, writes `manifest.json`, replies the error `ended` with its termination and exits. *Checked by `ea_10_connect_stands_at_t0` and `ea_10_a_failed_connect_writes_its_manifest`.*
- **EA-11** **`submit`.** `RunHandle::submit(action, body)` with the body as the waveform when `body_bytes` > 0 and `None` otherwise; the reply carries the `LogEntry`, admitted or rejected (a rejection is a result, not an error: the Kernel logged it). *Checked by `ea_11_submit_returns_the_logged_entry`.*
- **EA-12** **Time and events.** Durations on the wire are nanoseconds (`by_ns`, `within_ns`, `duration_ns`), converted onto the Run's primary root with its nominal rate and rounded up. `advance { to }` is `advance_to(to)`; `advance { by_ns }` is `advance_to(now + by_ns)`. `wait_for { kinds, from, within_ns }` is `RunHandle::wait_for(kinds, from, now + within_ns)` (KF-2), and its reply carries the index and the event when one matched. `events { from }` is `RunHandle::events(from)` (KF-1). `status` reports the Run's id, state, `now`, `effective()` and the event count. *Checked by `ea_12_time_and_events`.*
- **EA-13** **`read`.** The server serves the bytes of an artifact only if its Runs reported it: a `file://` URI that appeared as the `artifact.uri` of a delivered `sink.CAPTURE_WRITTEN` event (VD-1) or among the `artifacts` of a child's Manifest. Any other URI is `not_found`, whatever file it names: a client reads what its Session produced and nothing else. *Checked by `ea_13_read_serves_only_reported_artifacts`.*
- **EA-14** **`run_child`.** The server splits the body into the inputs' bytes by the sizes `inputs` lists and keys each by its `ContentHash`. The child's profile is `profile` when given; otherwise it is the parent's profile document with the `feed` member removed from every binding (decision S6 of the overview). It needs `duration_ns` or a schedule entry `Stop {}` in the Spec, and without either the request is refused before the Kernel sees it (a child with neither would never end: a MockRadio always has a next wakeup). It builds the child's Assembly (EA-7) and calls `RunHandle::run_child(spec, profile, assembly, drive)` (KF-3), where `drive` runs the child with `run_until_end(T0 + duration_ns)` or, with no duration, until it ends. An admitted child's Manifest is written as `child-<seq>.manifest.json` and its artifacts become readable (EA-13); the reply carries the parent's log entry, the child's Manifest and the file's path. A rejected child has no Manifest. *Checked by `ea_14_run_child` and the Python sweep.*
- **EA-15** **The end.** `finish` calls `RunHandle::finish()`, writes `manifest.json`, replies `finished` and exits with status 0. When its input ends before `finish`, the server calls `disconnect()` (an Attached Lease ends the Run `Stopped { client_disconnect }`, RS-23), finishes, writes `manifest.json` and exits. After the Run has ended by itself (a Policy abort, a failed stage), every request that needs a live Run gets `ended`, and `finish` still returns the Manifest. *Checked by `ea_15_finish_and_disconnect`.*

## 4. The Python package

- **EA-16** **The API.** The package `ezsdr` requires Python ≥ 3.9 and numpy, and exports:

  | Name | Meaning |
  |---|---|
  | `connect(profile=None, *, server=None, runs_dir=None, lease=None) -> Session` | spawns the server (`server`, else the environment variable `EZSDR_SERVER`, else `ezsdr-server` on `PATH`) with `--runs-dir` when given, says `hello`, then `connect` |
  | `Session` | a context manager: leaving it calls `close()`, which sends `finish` and keeps the Manifest in `manifest` and its path in `manifest_path` |
  | `Session.radio(name=None) -> Radio` | the named radio binding, or the profile's only binding that has a transmit or receive port fed to a recorder — in practice the one radio; `Session.rx` and `Session.tx` are `radio().rx` and `radio().tx` |
  | `Radio.rx`, `Radio.tx` | the receive and transmit sides, with the attributes `frequency`, `sample_rate`, `gain`, `antenna`, `channels`: reading one returns the effective value (KC-27), setting one submits `SetParameter` on the radio for `radio.<rx or tx>.frequency_hz`, `sample_rate_hz`, `gain_db`, `antenna`, `channels` and raises `Rejected` if the entry is rejected |
  | `Radio.tx.repeat(x)` | sets `radio.tx.channels` to the waveform's channel count when the effective value differs, then submits `radio.start_repeat` on `<radio>/tx` with `x`'s bytes; raises `Rejected` on a rejected entry; returns the entry |
  | `Radio.tx.stop()`, `Radio.rx.stop()`, `Session.stop(target=None)` | submits `Stop { target }` (`<radio>/tx`, `<radio>/rx`, the given target, or none) |
  | `Radio.rx.capture(n, at=None, timeout=None) -> numpy.ndarray` | EA-17 |
  | `Session.sleep(seconds)`, `Session.wait_until(t)`, `Session.now` | `advance { by_ns }`, `advance { to }`, the Run's current `TimePoint` (Vision §54: the client never uses `time.sleep` for Run time) |
  | `Session.wait_for(kinds, timeout)`, `Session.events(start=0)` | `wait_for` from the first event the client has not yet waited past, `events { from }` |
  | `Session.set(target, key, value)`, `Session.submit(action, waveform=None)` | a `SetParameter` that raises `Rejected`; any `SessionAction` document, returning the entry without raising (the descent of Vision §3: explicit targets, timed operations, any Vocabulary verb) |
  | `Session.run(spec, profile=None, inputs=(), duration=None) -> RunResult` | `run_child`; `inputs` are `(ArtifactRef, bytes)` pairs, as `waveform()` makes; `RunResult` has `entry`, `manifest`, `path`, `termination` and `capture(output_id)`, which reads that output's artifact as EA-17 does; raises `Rejected` on a rejected entry |
  | `Session.read(artifact) -> bytes` | `read` of an `ArtifactRef`'s URI |
  | `waveform(x, id="waveform") -> (dict, bytes)` | the `ArtifactRef` document (kind `ezsdr.input`, uri `mem:<hash>`) and the cf32 bytes of `x` (EA-17), for a Spec's `inputs` or schedule |
  | `Error`, `Rejected(entry)`, `RunEnded(termination)`, `ProtocolError`, `CaptureTimeout` | the exceptions; `Rejected` carries the logged entry and its violations |

  *Checked by the Python carriers.*
- **EA-17** **Samples.** A waveform `x` is a 1-D array (one channel) or a 2-D array of shape `(channels, n)`; it is sent as little-endian `cf32`, channel-interleaved (sample `k` of channel `c` at byte `(k · channels + c) · 8`), which is RM-13's layout and the capture Sink's. `capture(n, at, timeout)` finds the recorder whose `feed` starts at this radio's receive port, submits `sink.capture` on it with `sink.capture_samples = n` (and `at` when given) and raises `Rejected` on a rejected entry; then waits (`wait_for`) for `sink.CAPTURE_WRITTEN` or `sink.REQUEST_REJECTED` from source `sink/<recorder>`, within `timeout` seconds of Run time (default `n / rate + 1`, `rate` the effective receive rate); raises `CaptureTimeout` when none arrives and `Rejected` on `REQUEST_REJECTED`; reads the artifact's bytes (`read`); and returns them as `complex64`, of shape `(n,)` for one channel and `(channels, n)` otherwise, the channel count taken from the artifact's `continuity`. Captures are served in order (HD-10), and `capture` returns only after its own artifact, so the next `CAPTURE_WRITTEN` from that recorder is always its own. *Checked by `test_v57_repeat_then_capture_in_software` and `test_ea_17_two_channels_round_trip`.*
- **EA-18** **Names.** The package and its examples name no Module id, device profile, simulation section or other binding content: what they name are the Radio Model's keys and verb (`radio.*`, `radio.start_repeat`) and the Sink Vocabulary's key, verb and event kinds (`sink.capture_samples`, `sink.capture`, `sink.CAPTURE_WRITTEN`, `sink.REQUEST_REJECTED`) — Vocabulary names, which are the same on hardware (§58 #10). *Checked by `v58_10_experiments_name_no_mock_type` (GY-6).*
- **EA-19** **Examples.** `python/examples/minimal.py` is Vision §3's first snippet and `python/examples/sweep.py` §54's loop; both run against the default profile. *Checked by `test_ea_19_the_examples_run`.*

## 5. Tests

| test | input | expected | rules |
|---|---|---|---|
| `ea_02_framing` (server) | a header without a line feed before the stream ends; a header that is not JSON; one with an unknown field; a body shorter than `body_bytes`; a header over 16 MiB | the error `protocol`, and the server stops reading | EA-2, EA-5 |
| `ea_03_handshake` | `hello { protocol: 1 }`; `hello { protocol: 2 }`; `status` first | `hello` with the versions; `unsupported_protocol` with `supported: [1]`; `protocol` | EA-3 |
| `ea_04_requests` | `submit` before `connect`; `connect` twice | `protocol` each time | EA-4 |
| `ea_05_errors` | `advance { to }` in a domain unrelated to the root; `submit` of an action that fails `check_entry`; a request after the Run ended | `not_on_primary_root`; `malformed`; `ended` with the termination | EA-5 |
| `ea_06_schema_freeze` | the frame types | equal to `schemas/server/*.v1.json` | EA-6 |
| `ea_07_a_profile_naming_an_unknown_module_is_refused` | a profile binding `ezsdr.radio.nonexistent` | `refused`, naming the Module | EA-7 |
| `ea_08_the_manifests_are_written_to_the_session_directory` | a Session with one child, finished | `session-<pid>-0/manifest.json` and `child-<seq>.manifest.json`, equal to the replies' Manifests | EA-8 |
| `ea_09_the_default_profile_loops_back` | `connect {}`, `radio.tx.channels = 1`, a repeat, a capture | the capture holds the waveform at some offset, rotated by one unit phasor (MR-34's LO phases) | EA-9 |
| `ea_10_connect_stands_at_t0`, `ea_10_a_failed_connect_writes_its_manifest` | a default `connect`; a profile whose Sink directory cannot be created | `now == start_instant`; `ended` with `Failed { prepare }` and a written Manifest | EA-10 |
| `ea_11_submit_returns_the_logged_entry` | an admitted and a rejected action | both entries, dense sequence numbers | EA-11 |
| `ea_12_time_and_events` | `advance { by_ns: 1_000_000 }`; a capture's `wait_for`; `events { from }` | `now` moved by exactly 1 ms on the root; the event and its index; the same event read back | EA-12 |
| `ea_13_read_serves_only_reported_artifacts` | `read` of a reported capture; of the Session directory's `manifest.json`; of `file:///etc/hosts` | the bytes; `not_found`; `not_found` | EA-13 |
| `ea_14_run_child` | a child with `duration_ns`; one with neither duration nor stop; one whose profile binds a device the parent does not | a Manifest with `run.parent`; `refused`; an entry `Rejected` with `ezsdr.run_child` and no Manifest | EA-14 |
| `ea_15_finish_and_disconnect` | `finish`; a stream that ends after `connect` | `finished` and exit 0; `Stopped { client_disconnect }` in the written Manifest | EA-15 |
| `ea_binary_speaks_the_protocol` (server) | the built binary, spawned | `hello`, `connect`, `finish` over its standard streams; exit status 0 | EA-2, EA-8, EA-15 |
| Python carriers | [`00-overview.md`](00-overview.md) §8 | as listed there | EA-1, EA-16…EA-19 |

## 6. Decisions

| # | Decision | Choice | Rejected (one line each) |
|---|---|---|---|
| A1 | Frame format | A JSON header line plus a raw binary body | Samples base64-encoded inside JSON (a third larger, and a parse of megabytes of text); a length-prefixed binary header (unreadable in a trace, and one more format); MessagePack or protobuf (an external package on both sides for what JSON and a length do) |
| A2 | Envelope | `{ "request": … }` / `{ "reply": … }` with `body_bytes` beside it, every level `deny_unknown_fields` | The request's fields flattened beside `body_bytes` (serde cannot refuse unknown fields through `flatten`, and Vision §9 wants them refused) |
| A3 | A rejected Session Action on the wire | A normal `submitted` reply carrying the `Rejected` entry; Python turns it into `Rejected` | A protocol error (the Kernel logged it: it is the result the user asked for, and an MCP client may want the entry, not an exception) |
| A4 | What `read` may serve | Only URIs the server's Runs reported (EA-13) | Any file under the Session directory (a user profile may name another capture directory, and the Session directory holds Manifests, not artifacts); any path (a client could read any file the server's user can) |
| A5 | Where Manifests go | The Session directory, `manifest.json` and `child-<seq>.manifest.json` | Only in the replies (a Session's provenance would live only as long as the client's variable); the capture directory (a user profile may place it elsewhere, and the Manifest is the Session's, not the Sink's) |
| A6 | The shape of a capture in Python | `(n,)` for one channel, `(channels, n)` otherwise | Always 2-D (§3's `y = sdr.rx.capture(N)` reads as a vector); `(n, channels)` (numpy code indexes a channel as `y[c]`) |
| A7 | Python version floor | 3.9: the oldest the machines at hand run (macOS's `/usr/bin/python3`), with `from __future__ import annotations` | 3.10 or later for `match` and `X | Y` (nothing in the package needs them) |
| A8 | How Python finds the server | `server=`, else `EZSDR_SERVER`, else `ezsdr-server` on `PATH`; a clear error naming `cargo build -p ezsdr-server` otherwise | Building it from Python (a toolchain dependency); a wheel bundling it (packaging is later work) |
