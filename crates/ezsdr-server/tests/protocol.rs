//! Spec 16's server tests (plan/phase6/16-easy-api.md §5).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ezsdr_kernel::event::EventKind;
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::run::{Stage, StopCause, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::time::{ClockDomain, ClockRegistry, EpochRef, Rational, TimePoint};
use ezsdr_server::protocol::{ErrorKind, ProtocolError, Reply, ReplyFrame, Request, Response};
use ezsdr_server::{Config, Exit, Handled, Server, serve};
use serde_json::json;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("ezsdr-server-{}-{name}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config(dir: &Path) -> Config {
    Config { runs_dir: dir.to_path_buf(), implementations: Vec::new() }
}

fn ok(handled: Handled) -> Response {
    match handled.reply {
        Reply::Result(response) => response,
        Reply::Error(error) => panic!("expected a result, got {error:?}"),
    }
}

fn err(handled: Handled) -> ProtocolError {
    match handled.reply {
        Reply::Error(error) => error,
        Reply::Result(response) => panic!("expected an error, got {response:?}"),
    }
}

fn greeted(dir: &Path) -> Server {
    let mut server = Server::new(config(dir));
    ok(server.handle(Request::Hello { protocol: 1 }, Vec::new()));
    server
}

fn connected(dir: &Path) -> (Server, TimePoint) {
    let mut server = greeted(dir);
    let Response::Connected { now, .. } = ok(server.handle(Request::Connect { profile: None, lease: None }, Vec::new())) else { panic!("connect") };
    (server, now)
}

fn submit(server: &mut Server, action: SessionAction, body: Vec<u8>) -> ezsdr_kernel::session::LogEntry {
    match ok(server.handle(Request::Submit { action }, body)) {
        Response::Submitted { entry, .. } => entry,
        other => panic!("{other:?}"),
    }
}

fn set(key: &str, value: Value) -> SessionAction {
    SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse(key).unwrap(), value }
}

fn repeat() -> SessionAction {
    SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: ResourceId::parse("radio/tx").unwrap(), at: None, params: BTreeMap::new() }
}

fn capture(n: i64) -> SessionAction {
    SessionAction::Vocabulary { ns: Namespace::parse("sink").unwrap(), verb: Ident::parse("capture").unwrap(), target: ResourceId::parse("rec").unwrap(), at: None, params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::Int(n))]) }
}

fn written() -> EventKind {
    EventKind::parse(ezsdr_sink::CAPTURE_WRITTEN).unwrap()
}

/// One channel of little-endian cf32: sample `k` is `(k + 1, -(k + 1)) / 4096`.
fn ramp(len: usize) -> Vec<u8> {
    (0..len).flat_map(|k| {
        let v = (k + 1) as f32 / 4096.0;
        v.to_le_bytes().into_iter().chain((-v).to_le_bytes())
    }).collect()
}

/// Captures `n` samples and returns the artifact's URI and the Run's time after the wait.
fn capture_uri(server: &mut Server, n: i64) -> (String, TimePoint) {
    let Response::Status { events, .. } = ok(server.handle(Request::Status {}, Vec::new())) else { panic!() };
    let entry = submit(server, capture(n), Vec::new());
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "{:?}", entry.outcome);
    let Response::Waited { event: Some(event), now, .. } = ok(server.handle(Request::WaitFor { kinds: vec![written()], from: events, within_ns: 1_000_000_000 }, Vec::new())) else { panic!("no capture") };
    (event.payload["artifact"]["uri"].as_str().unwrap().to_owned(), now)
}

fn read(server: &mut Server, uri: &str) -> Vec<u8> {
    let handled = server.handle(Request::Read { uri: uri.to_owned() }, Vec::new());
    let body = handled.body.clone();
    let Response::Read { size } = ok(handled) else { panic!() };
    assert_eq!(size as usize, body.len());
    body
}

fn finish(server: &mut Server) -> (Manifest, String) {
    let handled = server.handle(Request::Finish {}, Vec::new());
    assert!(handled.exit);
    let Response::Finished { manifest, path } = ok(handled) else { panic!() };
    (*manifest, path)
}

/// Runs `serve` over `input` and returns the reply frames, their bodies and the exit.
fn serve_bytes(dir: &Path, input: Vec<u8>) -> (Vec<(ReplyFrame, Vec<u8>)>, Exit) {
    let mut output = Vec::new();
    let exit = serve(Cursor::new(input), &mut output, config(dir)).unwrap();
    let mut reader = BufReader::new(Cursor::new(output));
    let mut frames = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        let frame: ReplyFrame = serde_json::from_str(&line).unwrap();
        let mut body = vec![0; frame.body_bytes as usize];
        reader.read_exact(&mut body).unwrap();
        frames.push((frame, body));
    }
    (frames, exit)
}

fn error_of(frame: &ReplyFrame) -> &ProtocolError {
    match &frame.reply {
        Reply::Error(error) => error,
        Reply::Result(result) => panic!("expected an error, got {result:?}"),
    }
}

const HELLO: &str = "{\"request\":{\"op\":\"hello\",\"protocol\":1}}\n";

#[test]
fn ea_02_framing() {
    let temp = TempDir::new("framing");
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (HELLO.trim_end().as_bytes().to_vec(), "EA-2: the stream ended inside a header"),
        (b"not json\n".to_vec(), "EA-2: not a request frame"),
        (b"{\"request\":{\"op\":\"hello\",\"protocol\":1},\"extra\":1}\n".to_vec(), "EA-2: not a request frame"),
        (b"{\"request\":{\"op\":\"hello\",\"protocol\":1,\"extra\":1}}\n".to_vec(), "EA-2: not a request frame"),
        ([HELLO.as_bytes(), b"{\"request\":{\"op\":\"status\"},\"body_bytes\":10}\nabc"].concat(), "EA-2: the stream ended inside a body"),
    ];
    for (input, message) in cases {
        let (frames, exit) = serve_bytes(&temp.0, input);
        assert_eq!(exit, Exit::BadFrame, "{message}");
        let error = error_of(&frames.last().unwrap().0);
        assert_eq!(error.kind, ErrorKind::Protocol);
        assert!(error.message.starts_with(message), "{} does not start with {message}", error.message);
    }
    // A header over 16 MiB: whitespace is valid JSON, so only the limit refuses it.
    let mut huge = b"{\"request\":{\"op\":\"hello\",\"protocol\":1}".to_vec();
    huge.resize(16 * 1024 * 1024 + 8, b' ');
    huge.extend_from_slice(b"}\n");
    let (frames, exit) = serve_bytes(&temp.0, huge);
    assert_eq!(exit, Exit::BadFrame);
    assert_eq!(error_of(&frames[0].0).message, "EA-2: the header is longer than 16 MiB");
}

#[test]
fn ea_03_handshake() {
    let temp = TempDir::new("handshake");
    let mut server = Server::new(config(&temp.0));
    let handled = server.handle(Request::Hello { protocol: 1 }, Vec::new());
    assert!(!handled.exit);
    let Response::Hello { protocol, server: name, kernel_api } = ok(handled) else { panic!() };
    assert_eq!((protocol, name.as_str(), kernel_api.as_str()), (1, "ezsdr-server 0.1.0", "4.0.0"));

    let handled = Server::new(config(&temp.0)).handle(Request::Hello { protocol: 2 }, Vec::new());
    assert!(handled.exit);
    let error = err(handled);
    assert_eq!(error.kind, ErrorKind::UnsupportedProtocol);
    assert_eq!(error.supported, Some(vec![1]));

    let handled = Server::new(config(&temp.0)).handle(Request::Status {}, Vec::new());
    assert!(handled.exit);
    assert_eq!(err(handled).kind, ErrorKind::Protocol);
}

#[test]
fn ea_04_requests() {
    let temp = TempDir::new("requests");
    let mut server = greeted(&temp.0);
    assert_eq!(err(server.handle(Request::Submit { action: set("radio.tx.channels", Value::Int(1)) }, Vec::new())).kind, ErrorKind::Protocol);
    assert_eq!(err(server.handle(Request::Hello { protocol: 1 }, Vec::new())).kind, ErrorKind::Protocol);
    ok(server.handle(Request::Connect { profile: None, lease: None }, Vec::new()));
    assert_eq!(err(server.handle(Request::Connect { profile: None, lease: None }, Vec::new())).kind, ErrorKind::Protocol);
    assert_eq!(err(server.handle(Request::Status {}, vec![1])).kind, ErrorKind::Protocol, "a request without a body refuses one");
    assert_eq!(err(server.handle(Request::Advance { to: None, by_ns: None }, Vec::new())).kind, ErrorKind::Protocol);
    finish(&mut server);
}

#[test]
fn ea_05_errors() {
    let temp = TempDir::new("errors");
    let (mut server, now) = connected(&temp.0);
    let unrelated = TimePoint::new(ezsdr_kernel::id::ClockDomainId::local(999), now.ticks + 10);
    assert_eq!(err(server.handle(Request::Advance { to: Some(unrelated), by_ns: None }, Vec::new())).kind, ErrorKind::NotOnPrimaryRoot);
    let deep = set("radio.tx.gain_db", Value::List(vec![Value::List(vec![Value::Int(1)])]));
    assert_eq!(err(server.handle(Request::Submit { action: deep }, Vec::new())).kind, ErrorKind::Malformed);
    let stop = submit(&mut server, SessionAction::Stop { target: None }, Vec::new());
    assert!(matches!(stop.outcome, Outcome::Admitted { .. }));
    let ended = err(server.handle(Request::Submit { action: set("radio.tx.channels", Value::Int(1)) }, Vec::new()));
    assert_eq!(ended.kind, ErrorKind::Ended);
    assert_eq!(ended.termination, Some(Termination::Stopped { cause: StopCause::Client {} }));
    let (manifest, _) = finish(&mut server);
    assert_eq!(manifest.action_log.len(), 1, "the malformed action took no sequence number");
}

#[test]
fn ea_06_schema_freeze() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/server");
    let update = std::env::var("EZSDR_UPDATE_SCHEMAS").is_ok();
    let schemas = ezsdr_server::protocol::document_schemas();
    let mut stale = Vec::new();
    if update {
        std::fs::create_dir_all(&dir).unwrap();
    } else {
        for entry in std::fs::read_dir(&dir).expect("schemas/server is readable") {
            let file = entry.unwrap().file_name().to_string_lossy().into_owned();
            if file.ends_with(".json") && !schemas.keys().any(|name| ezsdr_kernel::schema::file_name(name) == file) {
                stale.push(format!("{file}: no frame type generates it"));
            }
        }
    }
    for (name, schema) in schemas {
        let path = dir.join(ezsdr_kernel::schema::file_name(name));
        let rendered = ezsdr_kernel::schema::render(&schema);
        if update {
            std::fs::write(&path, &rendered).unwrap();
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(committed) if committed == rendered => {}
            Ok(_) => stale.push(format!("{name}: differs from the committed schema")),
            Err(_) => stale.push(format!("{name}: no committed schema at {}", path.display())),
        }
    }
    assert!(stale.is_empty(), "EA-6: schema freeze failed:\n{}", stale.join("\n"));
}

#[test]
fn ea_07_a_profile_naming_an_unknown_module_is_refused() {
    let temp = TempDir::new("unknown-module");
    let mut profile = ezsdr_server::default_profile(&temp.0.to_string_lossy());
    profile["bindings"]["radio"]["module"]["id"] = json!("ezsdr.radio.nonexistent");
    let mut server = greeted(&temp.0);
    let error = err(server.handle(Request::Connect { profile: Some(profile), lease: None }, Vec::new()));
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(error.message, "EA-7: radio: no Module ezsdr.radio.nonexistent in this server");
    ok(server.handle(Request::Connect { profile: None, lease: None }, Vec::new()));
    finish(&mut server);
}

/// A Spec Run on the default profile's radio recording `n` samples from its receive port.
fn receive_spec(n: i64) -> serde_json::Value {
    json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 1 }, { "id": "sink", "major": 1 }] },
        "resources": { "radio": { "kind": "radio.device", "requires": {
            "radio.rx.channels": { "kind": "eq", "value": 1 },
            "radio.rx.sample_rate_hz": { "kind": "eq", "value": 1.0e6 }
        } } },
        "outputs": [{
            "id": "rec", "kind": "sink.capture", "params": { "sink.capture_samples": n },
            "feed": { "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 64 }
        }]
    })
}

fn run_child(server: &mut Server, spec: serde_json::Value, profile: Option<serde_json::Value>, duration_ns: Option<u64>) -> Handled {
    server.handle(Request::RunChild { spec, profile, inputs: Vec::new(), duration_ns }, Vec::new())
}

#[test]
fn ea_08_the_manifests_are_written_to_the_session_directory() {
    let temp = TempDir::new("manifests");
    let (mut server, _) = connected(&temp.0);
    let Response::Ran { entry, manifest: Some(child), path: Some(child_path) } = ok(run_child(&mut server, receive_spec(1_000), None, Some(10_000_000))) else { panic!() };
    let dir = server.dir().unwrap().to_path_buf();
    assert_eq!(dir.file_name().unwrap().to_string_lossy(), format!("session-{}-0", std::process::id()));
    assert_eq!(PathBuf::from(&child_path), dir.join(format!("child-{}.manifest.json", entry.seq)));
    let written: Manifest = serde_json::from_str(&std::fs::read_to_string(&child_path).unwrap()).unwrap();
    assert_eq!(written, *child);
    let (manifest, path) = finish(&mut server);
    assert_eq!(PathBuf::from(&path), dir.join("manifest.json"));
    let written: Manifest = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written, manifest);
    // A second server in the same runs directory takes the next free number.
    let (mut second, _) = connected(&temp.0);
    assert_eq!(second.dir().unwrap().file_name().unwrap().to_string_lossy(), format!("session-{}-1", std::process::id()));
    finish(&mut second);
}

#[test]
fn ea_09_the_default_profile_loops_back() {
    let temp = TempDir::new("loopback");
    let (mut server, _) = connected(&temp.0);
    let waveform = ramp(1_000);
    assert!(matches!(submit(&mut server, set("radio.tx.channels", Value::Int(1)), Vec::new()).outcome, Outcome::Admitted { .. }));
    assert!(matches!(submit(&mut server, repeat(), waveform.clone()).outcome, Outcome::Admitted { .. }));
    server.handle(Request::Advance { to: None, by_ns: Some(5_000_000) }, Vec::new());
    let (uri, _) = capture_uri(&mut server, 3_000);
    let bytes = read(&mut server, &uri);
    assert_eq!(bytes.len(), 3_000 * 8);
    // The repeat is continuous, so the capture is the waveform at some offset, rotated by
    // the two LOs' phases (`x310-like` draws them, MR-34): one unit phasor for every sample.
    let complex = |b: &[u8]| (f32::from_le_bytes(b[..4].try_into().unwrap()) as f64, f32::from_le_bytes(b[4..8].try_into().unwrap()) as f64);
    let captured: Vec<_> = bytes.chunks_exact(8).map(complex).collect();
    let sent: Vec<_> = waveform.chunks_exact(8).map(complex).collect();
    let norm = |(re, im): (f64, f64)| (re * re + im * im).sqrt();
    let offset = (0..1_000).find(|k| (norm(captured[0]) - norm(sent[*k])).abs() < 1e-6).expect("the capture holds the waveform");
    let ratio = |(a, b): ((f64, f64), (f64, f64))| {
        let d = b.0 * b.0 + b.1 * b.1;
        ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
    };
    let phasor = ratio((captured[0], sent[offset]));
    assert!((norm(phasor) - 1.0).abs() < 1e-5, "a 0 dB loopback keeps the amplitude");
    for (index, sample) in captured.iter().enumerate() {
        let r = ratio((*sample, sent[(offset + index) % 1_000]));
        assert!((r.0 - phasor.0).abs() < 1e-4 && (r.1 - phasor.1).abs() < 1e-4, "captured sample {index}");
    }
    finish(&mut server);
}

#[test]
fn ea_10_connect_stands_at_t0() {
    let temp = TempDir::new("t0");
    let mut server = greeted(&temp.0);
    let Response::Connected { now, start_instant, dir, profile, .. } = ok(server.handle(Request::Connect { profile: None, lease: None }, Vec::new())) else { panic!() };
    assert_eq!(now, start_instant);
    assert_eq!(now.ticks, 2_000_000_000, "x310-like's 2 s start lead on a nanosecond root");
    assert_eq!(profile, ezsdr_server::default_profile(&dir));
    finish(&mut server);
}

#[test]
fn ea_10_a_failed_connect_writes_its_manifest() {
    let temp = TempDir::new("failed-connect");
    let blocker = temp.0.join("a-file");
    std::fs::write(&blocker, b"").unwrap();
    let profile = ezsdr_server::default_profile(&blocker.join("captures").to_string_lossy());
    let mut server = greeted(&temp.0);
    let handled = server.handle(Request::Connect { profile: Some(profile), lease: None }, Vec::new());
    assert!(handled.exit);
    let error = err(handled);
    assert_eq!(error.kind, ErrorKind::Ended);
    assert_eq!(error.termination, Some(Termination::Failed { stage: Stage::Prepare }));
    let written: Manifest = serde_json::from_str(&std::fs::read_to_string(server.dir().unwrap().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(written.termination.reason, Termination::Failed { stage: Stage::Prepare });
}

#[test]
fn ea_11_submit_returns_the_logged_entry() {
    let temp = TempDir::new("submit");
    let (mut server, _) = connected(&temp.0);
    let admitted = submit(&mut server, set("radio.tx.channels", Value::Int(1)), Vec::new());
    let rejected = submit(&mut server, set("radio.rx.sample_rate_hz", Value::Num(1.0e12)), Vec::new());
    assert!(matches!(admitted.outcome, Outcome::Admitted { .. }));
    assert!(matches!(rejected.outcome, Outcome::Rejected { .. }), "a rejection is a result, not an error");
    assert_eq!((admitted.seq, rejected.seq), (0, 1));
    let (manifest, _) = finish(&mut server);
    assert_eq!(manifest.action_log, vec![admitted, rejected]);
}

#[test]
fn ea_12_time_and_events() {
    let temp = TempDir::new("time");
    let (mut server, now) = connected(&temp.0);
    let Response::Advanced { now: later, .. } = ok(server.handle(Request::Advance { to: None, by_ns: Some(1_000_000) }, Vec::new())) else { panic!() };
    assert_eq!(later.ticks - now.ticks, 1_000_000, "1 ms on a nanosecond root");
    let (uri, at) = capture_uri(&mut server, 2_000);
    let Response::Events { events, next } = ok(server.handle(Request::Events { from: 0 }, Vec::new())) else { panic!() };
    let index = events.iter().position(|event| event.kind == written()).unwrap();
    assert_eq!(next, events.len());
    assert_eq!(events[index].payload["artifact"]["uri"], json!(uri));
    assert_eq!(events[index].time, at, "the wait ended at the round that delivered the event");
    let Response::Events { events: tail, .. } = ok(server.handle(Request::Events { from: index }, Vec::new())) else { panic!() };
    assert_eq!(tail[0], events[index]);
    let Response::Waited { index: none, now: horizon, .. } = ok(server.handle(Request::WaitFor { kinds: vec![written()], from: next, within_ns: 3_000_000 }, Vec::new())) else { panic!() };
    assert_eq!(none, None);
    assert_eq!(horizon.ticks, at.ticks + 3_000_000);
    finish(&mut server);
}

#[test]
fn ea_12_durations_round_up() {
    let clocks = ClockRegistry::new();
    let root = clocks.allocate_id();
    clocks.register(ClockDomain::root(root, Rational::new(3, 1).unwrap(), EpochRef::Arbitrary { set_by: "test".to_owned() })).unwrap();
    let at = TimePoint::new(root, 0);
    assert_eq!(ezsdr_server::ticks(&clocks, at, 1), Some(1), "a nanosecond on a 3 Hz clock is one tick, rounded up");
    assert_eq!(ezsdr_server::ticks(&clocks, at, 1_000_000_000), Some(3));
    assert_eq!(ezsdr_server::ticks(&clocks, at, 0), Some(0));
}

#[test]
fn ea_13_read_serves_only_reported_artifacts() {
    let temp = TempDir::new("read");
    let (mut server, _) = connected(&temp.0);
    let (uri, _) = capture_uri(&mut server, 100);
    assert_eq!(read(&mut server, &uri).len(), 800);
    let manifest = format!("file://{}", server.dir().unwrap().join("manifest.json").display());
    for uri in [manifest, "file:///etc/hosts".to_owned(), uri.replace(".sigmf-data", ".sigmf-meta")] {
        let error = err(server.handle(Request::Read { uri: uri.clone() }, Vec::new()));
        assert_eq!(error.kind, ErrorKind::NotFound, "{uri}");
    }
    finish(&mut server);
}

#[test]
fn ea_14_run_child() {
    let temp = TempDir::new("child");
    let (mut server, now) = connected(&temp.0);
    // The derived profile: the Session's without `feed`, which a Spec Run refuses (SB-22g).
    let Response::Ran { entry, manifest: Some(child), .. } = ok(run_child(&mut server, receive_spec(1_000), None, Some(10_000_000))) else { panic!() };
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    assert_eq!(child.termination.reason, Termination::Stopped { cause: StopCause::Client {} }, "{:?}", child.sections.get(&Namespace::parse("ezsdr.failure").unwrap()));
    let artifact = child.artifacts.iter().find(|artifact| artifact.id.as_str() == "rec").unwrap();
    assert_eq!(artifact.size_bytes, 8_000);
    assert_eq!(read(&mut server, &artifact.uri).len(), 8_000, "a child's artifact is readable");
    let Response::Status { now: after, .. } = ok(server.handle(Request::Status {}, Vec::new())) else { panic!() };
    assert_eq!(after, now, "the Session stood still while its child ran");

    // A Spec that schedules its own stop needs no duration.
    let mut stopping = receive_spec(1_000);
    stopping["schedule"] = json!([{ "at": { "clock": "radio", "offset_ticks": 5_000 }, "action": { "kind": "stop" } }]);
    let Response::Ran { manifest: Some(stopped), .. } = ok(run_child(&mut server, stopping, None, None)) else { panic!() };
    assert_eq!(stopped.termination.reason, Termination::Completed {});

    // Neither: refused before the Kernel sees it (and before it could run forever).
    let refused = err(run_child(&mut server, receive_spec(1_000), None, None));
    assert_eq!(refused.kind, ErrorKind::Refused);
    assert!(refused.message.starts_with("EA-14: a child Run needs duration_ns or a scheduled Stop"));

    let mut elsewhere = ezsdr_server::without_feeds(&ezsdr_server::default_profile(&server.dir().unwrap().to_string_lossy()));
    elsewhere["bindings"]["radio"]["selector"] = json!({ "id": "another" });
    let Response::Ran { entry, manifest: None, path: None } = ok(run_child(&mut server, receive_spec(1_000), Some(elsewhere), Some(10_000_000))) else { panic!() };
    let Outcome::Rejected { violations } = entry.outcome else { panic!() };
    assert_eq!(violations[0].check, Namespace::parse("ezsdr.run_child").unwrap());

    let (manifest, _) = finish(&mut server);
    assert_eq!(manifest.sections[&Namespace::parse("ezsdr.children").unwrap()].as_array().unwrap().len(), 2);
}

#[test]
fn ea_15_finish_and_disconnect() {
    let temp = TempDir::new("finish");
    let (frames, exit) = serve_bytes(&temp.0, format!("{HELLO}{{\"request\":{{\"op\":\"connect\"}}}}\n{{\"request\":{{\"op\":\"finish\"}}}}\n{{\"request\":{{\"op\":\"status\"}}}}\n").into_bytes());
    assert_eq!(exit, Exit::Replied);
    assert_eq!(frames.len(), 3, "nothing is read after finish");
    assert!(matches!(frames[2].0.reply, Reply::Result(Response::Finished { .. })));

    let (frames, exit) = serve_bytes(&temp.0, format!("{HELLO}{{\"request\":{{\"op\":\"connect\"}}}}\n").into_bytes());
    assert_eq!(exit, Exit::EndOfInput);
    let Reply::Result(Response::Connected { dir, .. }) = &frames[1].0.reply else { panic!() };
    let written: Manifest = serde_json::from_str(&std::fs::read_to_string(Path::new(dir).join("manifest.json")).unwrap()).unwrap();
    assert_eq!(written.termination.reason, Termination::Stopped { cause: StopCause::ClientDisconnect {} });
}

#[test]
fn ea_binary_speaks_the_protocol() {
    let temp = TempDir::new("binary");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ezsdr-server"))
        .arg("--runs-dir")
        .arg(&temp.0)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |request: &str| {
        stdin.write_all(request.as_bytes()).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let frame: ReplyFrame = serde_json::from_str(&line).unwrap();
        frame.reply
    };
    assert!(matches!(ask(HELLO), Reply::Result(Response::Hello { protocol: 1, .. })));
    assert!(matches!(ask("{\"request\":{\"op\":\"connect\"}}\n"), Reply::Result(Response::Connected { .. })));
    let Reply::Result(Response::Finished { path, .. }) = ask("{\"request\":{\"op\":\"finish\"}}\n") else { panic!() };
    assert!(Path::new(&path).starts_with(temp.0.canonicalize().unwrap()));
    assert!(child.wait().unwrap().success());
}
