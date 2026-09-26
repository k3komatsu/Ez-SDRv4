//! Ez-SDR v4 server `ezsdr-server` 0.1.0: the Runtime a client drives over
//! `ezsdr.protocol` 1 (design/16-easy-api.md).
//!
//! It compiles the Modules in (EA-7), runs one Session through the Kernel's
//! `RunHandle` and answers one request at a time. It is a frontend, not a Module: it
//! holds no Run semantics of its own beyond what every client must do alike (EA-1).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod catalogue;
pub mod protocol;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ezsdr_exec_native::Implementation;
use ezsdr_kernel::coordinator::{self, RunHandle, RunHandleError};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::run::{Lease, RunState};
use ezsdr_kernel::time::{ClockRegistry, TimePoint};

pub use catalogue::{assemble, default_profile};
use protocol::{
    ErrorKind, MAX_HEADER_BYTES, ProtocolError, Reply, ReplyFrame, Request, RequestFrame, Response,
    SUPPORTED,
};

/// What a server is started with (EA-7, EA-8).
#[derive(Clone)]
pub struct Config {
    /// Where the Session directory is created (EA-8).
    pub runs_dir: PathBuf,
    /// The component implementations handed to the native Executor (EA-7).
    pub implementations: Vec<Implementation>,
}

/// One reply: the frame's content, its body, and whether the server then exits.
pub struct Handled {
    /// The reply.
    pub reply: Reply,
    /// The body that follows it.
    pub body: Vec<u8>,
    /// Whether the server stops after sending it (EA-3, EA-5, EA-10, EA-15).
    pub exit: bool,
}

impl Handled {
    fn ok(response: Response) -> Handled {
        Handled { reply: Reply::Result(response), body: Vec::new(), exit: false }
    }

    fn error(error: ProtocolError) -> Handled {
        Handled { reply: Reply::Error(error), body: Vec::new(), exit: false }
    }

    fn exiting(mut self) -> Handled {
        self.exit = true;
        self
    }
}

fn fail(kind: ErrorKind, message: impl Into<String>) -> Handled {
    Handled::error(ProtocolError::new(kind, message))
}

/// The live Session: the Run, its clocks and its profile document.
struct Live {
    run: RunHandle,
    clocks: Arc<ClockRegistry>,
    profile: serde_json::Value,
}

/// A server's state between requests (EA-4).
pub struct Server {
    config: Config,
    greeted: bool,
    dir: Option<PathBuf>,
    live: Option<Live>,
    /// The URIs this server's Runs reported (EA-13).
    readable: BTreeSet<String>,
    /// How far `readable` has scanned the Session's events.
    scanned: usize,
}

impl Server {
    /// A server that has not yet been greeted.
    pub fn new(config: Config) -> Server {
        Server { config, greeted: false, dir: None, live: None, readable: BTreeSet::new(), scanned: 0 }
    }

    /// Whether `hello` has been answered (EA-3).
    pub fn greeted(&self) -> bool {
        self.greeted
    }

    /// The Session directory, once created (EA-8).
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Carries out one request (EA-4).
    pub fn handle(&mut self, request: Request, body: Vec<u8>) -> Handled {
        if !self.greeted {
            return match request {
                Request::Hello { protocol } if SUPPORTED.contains(&protocol) => {
                    self.greeted = true;
                    Handled::ok(Response::Hello {
                        protocol,
                        server: format!("ezsdr-server {}", env!("CARGO_PKG_VERSION")),
                        kernel_api: ezsdr_kernel::module_api::KERNEL_API.to_string(),
                    })
                }
                Request::Hello { protocol } => Handled::error(ProtocolError {
                    supported: Some(SUPPORTED.to_vec()),
                    ..ProtocolError::new(ErrorKind::UnsupportedProtocol, format!("EA-3: protocol {protocol} is not supported"))
                })
                .exiting(),
                _ => fail(ErrorKind::Protocol, "EA-3: the first request must be hello").exiting(),
            };
        }
        let needs_body = matches!(request, Request::Submit { .. } | Request::RunChild { .. });
        if !needs_body && !body.is_empty() {
            return fail(ErrorKind::Protocol, "EA-4: this request carries no body");
        }
        match request {
            Request::Hello { .. } => fail(ErrorKind::Protocol, "EA-4: hello was already said"),
            Request::Connect { profile, lease } => self.connect(profile, lease),
            request => {
                let Some(live) = self.live.as_mut() else {
                    return fail(ErrorKind::Protocol, "EA-4: no Session is connected");
                };
                let handled = match request {
                    Request::Submit { action } => {
                        let waveform = (!body.is_empty()).then_some(body.as_slice());
                        match live.run.submit(action, waveform) {
                            Ok(entry) => Handled::ok(Response::Submitted { entry, now: live.run.now(), events: count(&live.run) }),
                            Err(error) => run_error(error),
                        }
                    }
                    Request::Advance { to, by_ns } => advance(live, to, by_ns),
                    Request::WaitFor { kinds, from, within_ns, until } => wait_for(live, &kinds, from, within_ns, until),
                    Request::Events { from } => {
                        let events = live.run.events(from);
                        Handled::ok(Response::Events { next: count(&live.run), events })
                    }
                    Request::Status {} => Handled::ok(Response::Status {
                        run: live.run.id(),
                        state: live.run.state(),
                        now: live.run.now(),
                        effective: live.run.effective(),
                        events: count(&live.run),
                    }),
                    Request::Read { uri } => return self.read(&uri),
                    Request::RunChild { spec, profile, inputs, duration_ns } => {
                        return self.run_child(spec, profile, inputs, duration_ns, body);
                    }
                    Request::Finish {} => return self.finish(),
                    Request::Hello { .. } | Request::Connect { .. } => unreachable!("handled above"),
                };
                self.scan();
                handled
            }
        }
    }

    /// The end of the client's stream: an Attached Lease ends the Run (EA-15).
    pub fn disconnect(&mut self) {
        if let Some(mut live) = self.live.take() {
            live.run.disconnect();
            let manifest = live.run.finish();
            let _ = self.write_manifest("manifest.json", &manifest);
        }
    }

    fn connect(&mut self, profile: Option<serde_json::Value>, lease: Option<Lease>) -> Handled {
        if self.live.is_some() {
            return fail(ErrorKind::Protocol, "EA-4: a Session is already connected");
        }
        let dir = match self.session_dir() {
            Ok(dir) => dir,
            Err(error) => return fail(ErrorKind::Io, format!("EA-8: {error}")),
        };
        let profile = profile.unwrap_or_else(|| default_profile(&dir.to_string_lossy()));
        let assembly = match assemble(&profile, BTreeMap::new(), self.config.implementations.clone()) {
            Ok(assembly) => assembly,
            Err(message) => return fail(ErrorKind::Refused, message),
        };
        let clocks = assembly.clocks.clone();
        let mut run = match coordinator::connect(&profile, assembly, lease.unwrap_or_default()) {
            Ok(run) => run,
            Err(error) => return fail(ErrorKind::Refused, error.to_string()),
        };
        if let (RunState::Running {}, Some(t0)) = (run.state(), run.start_instant()) {
            // A Run that ends on its way to T0 falls through to the `ended` reply below with
            // its Manifest written (Review H, P0-4); any other refusal is impossible for T0,
            // which is on the primary root by construction.
            let _ = run.advance_to(t0);
        }
        let (RunState::Running {}, Some(start_instant)) = (run.state(), run.start_instant()) else {
            let manifest = run.finish();
            let termination = manifest.termination.reason.clone();
            let _ = self.write_manifest("manifest.json", &manifest);
            return Handled::error(ProtocolError {
                termination: Some(termination),
                ..ProtocolError::new(ErrorKind::Ended, "EA-10: the Session ended while it was being connected")
            })
            .exiting();
        };
        let response = Response::Connected {
            run: run.id(),
            now: run.now(),
            start_instant,
            dir: dir.to_string_lossy().into_owned(),
            profile: profile.clone(),
            effective: run.effective(),
        };
        self.live = Some(Live { run, clocks, profile });
        Handled::ok(response)
    }

    fn read(&mut self, uri: &str) -> Handled {
        self.scan();
        let path = uri.strip_prefix("file://").filter(|_| self.readable.contains(uri));
        let Some(path) = path else {
            return fail(ErrorKind::NotFound, format!("EA-13: {uri} is no artifact of this server's Runs"));
        };
        match std::fs::read(path) {
            Ok(bytes) => Handled { reply: Reply::Result(Response::Read { size: bytes.len() as u64 }), body: bytes, exit: false },
            Err(error) => fail(ErrorKind::Io, format!("EA-13: {path}: {error}")),
        }
    }

    fn run_child(
        &mut self,
        spec: serde_json::Value,
        profile: Option<serde_json::Value>,
        sizes: Vec<u64>,
        duration_ns: Option<u64>,
        body: Vec<u8>,
    ) -> Handled {
        let Some(live) = self.live.as_mut() else {
            return fail(ErrorKind::Protocol, "EA-4: no Session is connected");
        };
        let total = sizes.iter().try_fold(0u64, |sum, size| sum.checked_add(*size));
        if total != Some(body.len() as u64) {
            return fail(ErrorKind::Protocol, "EA-14: the inputs' sizes do not add up to the body");
        }
        let mut inputs = BTreeMap::new();
        let mut rest = body.as_slice();
        for size in sizes {
            let (bytes, tail) = rest.split_at(size as usize);
            inputs.insert(ContentHash::of_bytes(bytes), bytes.to_vec());
            rest = tail;
        }
        if duration_ns.is_none() && !schedules_a_stop(&spec) {
            return fail(ErrorKind::Refused, "EA-14: a child Run needs duration_ns or a scheduled Stop {}, or it would never end");
        }
        let profile = profile.unwrap_or_else(|| without_feeds(&live.profile));
        let assembly = match assemble(&profile, inputs, self.config.implementations.clone()) {
            Ok(assembly) => assembly,
            Err(message) => return fail(ErrorKind::Refused, message),
        };
        let clocks = assembly.clocks.clone();
        if let Some(ns) = duration_ns {
            // A duration that does not fit the child's clock would run nothing (Review H, P2-3).
            let root = assembly.authority.time().primary_root();
            // T0 + duration must be an instant: T0 is the start lead (seconds of a root that
            // counts from 0), so half the tick range leaves it room (Review I, P2-4).
            let fits = ticks(&clocks, TimePoint::new(root, 0), ns).is_some_and(|ticks| ticks <= i64::MAX / 2);
            if !fits {
                return fail(ErrorKind::Refused, "EA-14: duration_ns does not fit the child's clock");
            }
        }
        let mut drive = |child: &mut RunHandle| {
            let Some(t0) = child.start_instant() else {
                return;
            };
            let horizon = match duration_ns {
                Some(ns) => match ticks(&clocks, t0, ns) {
                    Some(ticks) => TimePoint::new(t0.domain, t0.ticks.saturating_add(ticks)),
                    None => return,
                },
                None => TimePoint::new(t0.domain, i64::MAX),
            };
            let _ = child.run_until_end(horizon);
        };
        let (entry, manifest) = match live.run.run_child(&spec, &profile, assembly, &mut drive) {
            Ok(result) => result,
            Err(error) => return run_error(error),
        };
        let path = match &manifest {
            Some(manifest) => {
                self.readable.extend(manifest.artifacts.iter().map(|artifact| artifact.uri.clone()));
                match self.write_manifest(&format!("child-{}.manifest.json", entry.seq), manifest) {
                    Ok(path) => Some(path),
                    Err(error) => return fail(ErrorKind::Io, format!("EA-8: {error}")),
                }
            }
            None => None,
        };
        self.scan();
        Handled::ok(Response::Ran { entry, manifest: manifest.map(Box::new), path })
    }

    fn finish(&mut self) -> Handled {
        let Some(live) = self.live.take() else {
            return fail(ErrorKind::Protocol, "EA-4: no Session is connected");
        };
        let manifest = live.run.finish();
        // A Manifest that cannot be written is still returned (Review I, P2-7).
        let path = self.write_manifest("manifest.json", &manifest).ok();
        Handled::ok(Response::Finished { manifest: Box::new(manifest), path }).exiting()
    }

    /// Adds the URIs of newly delivered `sink.CAPTURE_WRITTEN` events (EA-13).
    fn scan(&mut self) {
        let Some(live) = self.live.as_ref() else {
            return;
        };
        let events = live.run.events(self.scanned);
        self.scanned += events.len();
        for event in events {
            if event.kind.as_str() == ezsdr_sink::CAPTURE_WRITTEN {
                if let Ok(payload) = serde_json::from_value::<ezsdr_sink::CaptureWrittenPayload>(event.payload) {
                    self.readable.insert(payload.artifact.uri);
                }
            }
        }
    }

    /// Creates the Session directory once: `session-<pid>-<n>` with the first free `n` (EA-8).
    fn session_dir(&mut self) -> io::Result<PathBuf> {
        if let Some(dir) = &self.dir {
            return Ok(dir.clone());
        }
        std::fs::create_dir_all(&self.config.runs_dir)?;
        let pid = std::process::id();
        for n in 0u32.. {
            let candidate = self.config.runs_dir.join(format!("session-{pid}-{n}"));
            match std::fs::create_dir(&candidate) {
                Ok(()) => {
                    let dir = candidate.canonicalize()?;
                    self.dir = Some(dir.clone());
                    return Ok(dir);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!("the directory numbers are unbounded")
    }

    fn write_manifest(&self, name: &str, manifest: &Manifest) -> io::Result<String> {
        let dir = self.dir.as_ref().ok_or_else(|| io::Error::other("no Session directory"))?;
        let path = dir.join(name);
        let text = serde_json::to_string_pretty(manifest).map_err(io::Error::other)?;
        std::fs::write(&path, text)?;
        Ok(path.to_string_lossy().into_owned())
    }
}

impl Drop for Server {
    /// Every exit — a reply that cannot be written, a panic unwinding — still finishes a
    /// live Session and writes its Manifest (EA-15; Review H, P0-4).
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// The number of events the Run has delivered.
// ponytail: clones every event to count them; a count accessor if Sessions grow long event logs.
fn count(run: &RunHandle) -> usize {
    run.events(0).len()
}

fn run_error(error: RunHandleError) -> Handled {
    match error {
        RunHandleError::Ended { termination } => Handled::error(ProtocolError {
            termination: Some(termination.clone()),
            ..ProtocolError::new(ErrorKind::Ended, format!("the Run has ended: {termination:?}"))
        }),
        RunHandleError::Malformed { error } => fail(ErrorKind::Malformed, error.to_string()),
        RunHandleError::NotOnPrimaryRoot { t } => fail(ErrorKind::NotOnPrimaryRoot, format!("{t} is not on the Authority's primary root")),
        RunHandleError::NotSession => fail(ErrorKind::Protocol, "the request is available only on a Session"),
    }
}

/// Nanoseconds as ticks of `at`'s domain, rounded up (EA-12).
pub fn ticks(clocks: &ClockRegistry, at: TimePoint, ns: u64) -> Option<i64> {
    let rate = clocks.nominal_rate(at.domain).ok()?;
    let numerator = u128::from(ns) * u128::from(rate.num());
    let denominator = u128::from(rate.den()) * 1_000_000_000;
    i64::try_from(numerator.div_ceil(denominator)).ok()
}

fn advance(live: &mut Live, to: Option<TimePoint>, by_ns: Option<u64>) -> Handled {
    let target = match (to, by_ns) {
        (Some(to), None) => to,
        (None, Some(ns)) => {
            let now = live.run.now();
            match ticks(&live.clocks, now, ns) {
                Some(ticks) => TimePoint::new(now.domain, now.ticks.saturating_add(ticks)),
                None => return fail(ErrorKind::NotOnPrimaryRoot, "EA-12: the duration does not fit the Run's clock"),
            }
        }
        _ => return fail(ErrorKind::Protocol, "EA-4: advance takes exactly one of to and by_ns"),
    };
    match live.run.advance_to(target) {
        Ok(()) => Handled::ok(Response::Advanced { now: live.run.now(), events: count(&live.run) }),
        Err(error) => run_error(error),
    }
}

fn wait_for(
    live: &mut Live,
    kinds: &[ezsdr_kernel::event::EventKind],
    from: usize,
    within_ns: Option<u64>,
    until: Option<TimePoint>,
) -> Handled {
    let horizon = match (within_ns, until) {
        (Some(ns), None) => {
            let now = live.run.now();
            let Some(within) = ticks(&live.clocks, now, ns) else {
                return fail(ErrorKind::NotOnPrimaryRoot, "EA-12: the duration does not fit the Run's clock");
            };
            TimePoint::new(now.domain, now.ticks.saturating_add(within))
        }
        (None, Some(until)) => until,
        _ => return fail(ErrorKind::Protocol, "EA-4: wait_for takes exactly one of within_ns and until"),
    };
    match live.run.wait_for(kinds, from, horizon) {
        Ok(index) => Handled::ok(Response::Waited {
            index,
            event: index.and_then(|index| live.run.events(index).into_iter().next()),
            now: live.run.now(),
            horizon,
            events: count(&live.run),
        }),
        Err(error) => run_error(error),
    }
}

/// Whether a Spec document schedules `Stop {}`, which ends a Run (EA-14, KC-33).
fn schedules_a_stop(spec: &serde_json::Value) -> bool {
    spec["schedule"].as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry["action"]["kind"] == "stop" && entry["action"].get("target").is_none_or(serde_json::Value::is_null)
        })
    })
}

/// The Session's profile with every `feed` removed: a child's profile (EA-14).
pub fn without_feeds(profile: &serde_json::Value) -> serde_json::Value {
    let mut child = profile.clone();
    if let Some(bindings) = child["bindings"].as_object_mut() {
        for binding in bindings.values_mut() {
            if let Some(binding) = binding.as_object_mut() {
                binding.remove("feed");
            }
        }
    }
    child
}

/// Why the server stopped reading.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exit {
    /// It sent a reply after which it exits (`finish`, a refused handshake, a failed connect).
    Replied,
    /// Its input ended.
    EndOfInput,
    /// A frame could not be read (EA-5).
    BadFrame,
}

/// Serves one client: reads frames from `input`, writes replies to `output` (EA-2).
pub fn serve(input: impl BufRead, mut output: impl Write, config: Config) -> io::Result<Exit> {
    let mut input = input;
    let mut server = Server::new(config);
    loop {
        let mut line = Vec::new();
        let read = (&mut input).take(MAX_HEADER_BYTES as u64 + 1).read_until(b'\n', &mut line)?;
        if read == 0 {
            server.disconnect();
            return Ok(Exit::EndOfInput);
        }
        if line.last() != Some(&b'\n') {
            let message = if line.len() > MAX_HEADER_BYTES { "EA-2: the header is longer than 16 MiB" } else { "EA-2: the stream ended inside a header" };
            write_reply(&mut output, &fail(ErrorKind::Protocol, message))?;
            server.disconnect();
            return Ok(Exit::BadFrame);
        }
        // The frame in two steps: a JSON object whose `body_bytes` is readable frames the
        // stream, so a request that then fails to decode costs only that request
        // (Review H, P1-1); a header that is not such an object loses the framing.
        let header: serde_json::Value = match serde_json::from_slice(&line) {
            Ok(header @ serde_json::Value::Object(_)) => header,
            Ok(_) | Err(_) => {
                write_reply(&mut output, &fail(ErrorKind::Protocol, "EA-2: the header is not a JSON object"))?;
                server.disconnect();
                return Ok(Exit::BadFrame);
            }
        };
        let body_bytes = match header.get("body_bytes") {
            None => 0,
            Some(value) => match value.as_u64() {
                Some(n) => n,
                None => {
                    write_reply(&mut output, &fail(ErrorKind::Protocol, "EA-2: body_bytes is not a count"))?;
                    server.disconnect();
                    return Ok(Exit::BadFrame);
                }
            },
        };
        // ponytail: no limit on body_bytes; Phase 7's remote listener needs one.
        let mut body = Vec::new();
        let complete = (&mut input).take(body_bytes).read_to_end(&mut body).is_ok_and(|n| n as u64 == body_bytes);
        if !complete {
            write_reply(&mut output, &fail(ErrorKind::Protocol, "EA-2: the stream ended inside a body"))?;
            server.disconnect();
            return Ok(Exit::BadFrame);
        }
        let frame: RequestFrame = match serde_json::from_value(header) {
            Ok(frame) => frame,
            Err(error) => {
                write_reply(&mut output, &fail(ErrorKind::Protocol, format!("EA-2: not a request frame: {error}")))?;
                // Before `hello`, anything but `hello` ends the handshake (EA-3; Review I, P1-B).
                if !server.greeted() {
                    return Ok(Exit::BadFrame);
                }
                continue;
            }
        };
        let handled = server.handle(frame.request, body);
        write_reply(&mut output, &handled)?;
        if handled.exit {
            server.disconnect();
            return Ok(Exit::Replied);
        }
    }
}

fn write_reply(output: &mut impl Write, handled: &Handled) -> io::Result<()> {
    let frame = ReplyFrame { reply: handled.reply.clone(), body_bytes: handled.body.len() as u64 };
    let mut line = serde_json::to_vec(&frame).map_err(io::Error::other)?;
    line.push(b'\n');
    output.write_all(&line)?;
    output.write_all(&handled.body)?;
    output.flush()
}
