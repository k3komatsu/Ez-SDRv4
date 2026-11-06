//! Spec 16's server tests (design/16-easy-api.md §5).

// The shared per-test wall watchdog (plan 30 §7, R05): a manual clock a mutant freezes would
// hang the test until the harness's timeout; this aborts instead. One implementation, shared
// with `ezsdr-radio-uhd`'s `tests/common` and `ezsdr-acceptance`'s `tests/uhd.rs`.
#[path = "../../ezsdr-radio-uhd/tests/common/watchdog.rs"]
mod watchdog;

use ezsdr_kernel::spec::Scalar;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ezsdr_kernel::event::{EventKind, EventSource, Target};
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::run::{Stage, StopCause, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::time::{ClockDomain, ClockRegistry, Duration, EpochRef, Rational, TimePoint};
use ezsdr_radio_uhd::{Device, FakeConfig, FakeDevice};
use ezsdr_server::protocol::{ChannelRef, DirectionNames, ErrorKind, ProtocolError, Reply, ReplyFrame, Request, ResourceInit, Response, StreamInit};
use ezsdr_server::{Config, Exit, Handled, Server, serve};
use serde_json::json;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        // Every test of this file builds its server with `FakeDevice` (the manual clock,
        // plan 30 §4.4) and starts with a `TempDir`: arm the shared watchdog here (R05).
        watchdog::watch();
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
    Config::new(dir.to_path_buf())
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
    ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
    server
}

/// RM-30: no channel is in a stream unless a Session asks; most runs receive on c0.
fn rx0() -> Initial {
    Some(BTreeMap::from([("radio".to_owned(), ResourceInit { rx: vec![StreamInit { number: Some(0), channels: vec![ChannelRef::Number(0)], ..StreamInit::default() }], tx: Vec::new() })]))
}

/// As [`rx0`], and transmit channel c0 in stream 0 too: a loopback.
fn loopback() -> Initial {
    Some(BTreeMap::from([(
        "radio".to_owned(),
        ResourceInit {
            rx: vec![StreamInit { number: Some(0), channels: vec![ChannelRef::Number(0)], ..StreamInit::default() }],
            tx: vec![StreamInit { number: Some(0), channels: vec![ChannelRef::Number(0)], ..StreamInit::default() }],
        },
    )]))
}

fn connected(dir: &Path) -> (Server, TimePoint) {
    connected_with(dir, rx0())
}

fn connected_with(dir: &Path, initial: Initial) -> (Server, TimePoint) {
    let mut server = greeted(dir);
    let Response::Connected { now, .. } = ok(server.handle(Request::Connect { profile: None, lease: None, initial }, Vec::new())) else { panic!("connect") };
    (server, now)
}

fn submit(server: &mut Server, action: SessionAction, body: Vec<u8>) -> ezsdr_kernel::session::LogEntry {
    match ok(server.handle(Request::Submit { action }, body)) {
        Response::Submitted { entry, .. } => entry,
        other => panic!("{other:?}"),
    }
}

fn resource(name: &str, path: &str) -> Target {
    Target::Resource { resource: Ident::parse(name).unwrap(), path: path.to_owned() }
}

fn set(key: &str, value: Value) -> SessionAction {
    SessionAction::SetParameter { target: resource("radio", ""), key: Key::parse(key).unwrap(), value }
}

fn repeat() -> SessionAction {
    SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio", "tx/s0"), at: None, params: BTreeMap::new() }
}

fn capture(n: i64) -> SessionAction {
    SessionAction::Vocabulary {
        ns: Namespace::parse("sink").unwrap(),
        verb: Ident::parse("capture").unwrap(),
        target: Target::Output { output: Ident::parse("rec").unwrap() },
        at: None,
        params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(n))]),
    }
}

fn written() -> EventKind {
    EventKind::parse(ezsdr_sink::CAPTURE_WRITTEN).unwrap()
}

/// One channel of little-endian cf32: sample `k` is `(k + 1, -(k + 1)) / 4096`.
fn ramp(len: usize) -> Vec<u8> {
    (0..len)
        .flat_map(|k| {
            let v = (k + 1) as f32 / 4096.0;
            v.to_le_bytes().into_iter().chain((-v).to_le_bytes())
        })
        .collect()
}

/// Captures `n` samples and returns the artifact's URI and the Run's time after the wait.
fn capture_uri(server: &mut Server, n: i64) -> (String, TimePoint) {
    let Response::Status { events, now, .. } = ok(server.handle(Request::Status {}, Vec::new())) else { panic!() };
    let entry = submit(server, capture(n), Vec::new());
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "{:?}", entry.outcome);
    let within = Duration::new(now.domain(), 1_000_000_000);
    let Response::Waited { event: Some(event), now, .. } = ok(server.handle(Request::WaitFor { kinds: vec![written()], from: events, within: Some(within), until: None }, Vec::new())) else {
        panic!("no capture")
    };
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
    (*manifest, path.expect("the Manifest was written"))
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

const HELLO: &str = "{\"request\":{\"op\":\"hello\",\"protocol\":2}}\n";

#[test]
fn ea_02_framing() {
    let temp = TempDir::new("framing");
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (HELLO.trim_end().as_bytes().to_vec(), "EA-2: the stream ended inside a header"),
        (b"not json\n".to_vec(), "EA-2: the header is not a JSON object"),
        (b"[1]\n".to_vec(), "EA-2: the header is not a JSON object"),
        (b"{\"request\":{\"op\":\"status\"},\"body_bytes\":-1}\n".to_vec(), "EA-2: body_bytes is not a count"),
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
    let handled = server.handle(Request::Hello { protocol: 2 }, Vec::new());
    assert!(!handled.exit);
    let Response::Hello { protocol, server: name, kernel_api } = ok(handled) else { panic!() };
    assert_eq!((protocol, name.as_str(), kernel_api.as_str()), (2, "ezsdr-server 0.5.0", "4.0.0"));

    let handled = Server::new(config(&temp.0)).handle(Request::Hello { protocol: 1 }, Vec::new());
    assert!(handled.exit);
    let error = err(handled);
    assert_eq!(error.kind, ErrorKind::UnsupportedProtocol);
    assert_eq!(error.supported, Some(vec![2]));

    let handled = Server::new(config(&temp.0)).handle(Request::Status {}, Vec::new());
    assert!(handled.exit);
    assert_eq!(err(handled).kind, ErrorKind::Protocol);

    // A first frame that does not decode ends the handshake as well (Review I, P1-B).
    for first in ["{\"request\":{\"op\":\"nope\"}}\n", "{\"request\":{\"op\":\"hello\",\"protocol\":2,\"x\":1}}\n"] {
        let (frames, exit) = serve_bytes(&temp.0, format!("{first}{HELLO}").into_bytes());
        assert_eq!(exit, Exit::BadFrame, "{first}");
        assert_eq!(frames.len(), 1);
        assert_eq!(error_of(&frames[0].0).kind, ErrorKind::Protocol);
    }
}

#[test]
fn ea_04_requests() {
    let temp = TempDir::new("requests");
    let mut server = greeted(&temp.0);
    assert_eq!(err(server.handle(Request::Submit { action: set("radio.tx.c0.stream", Value::from(0)) }, Vec::new())).kind, ErrorKind::Protocol);
    assert_eq!(err(server.handle(Request::Hello { protocol: 2 }, Vec::new())).kind, ErrorKind::Protocol);
    ok(server.handle(Request::Connect { profile: None, lease: None, initial: None }, Vec::new()));
    assert_eq!(err(server.handle(Request::Connect { profile: None, lease: None, initial: None }, Vec::new())).kind, ErrorKind::Protocol);
    assert_eq!(err(server.handle(Request::Status {}, vec![1])).kind, ErrorKind::Protocol, "a request without a body refuses one");
    assert_eq!(err(server.handle(Request::Advance { to: None, by: None }, Vec::new())).kind, ErrorKind::Protocol);
    finish(&mut server);
}

#[test]
fn ea_05_errors() {
    let temp = TempDir::new("errors");
    let (mut server, now) = connected(&temp.0);
    let unrelated = TimePoint::new(ezsdr_kernel::id::ClockDomainId::local(999), now.ticks_in(now.domain()).unwrap() + 10);
    assert_eq!(err(server.handle(Request::Advance { to: Some(unrelated), by: None }, Vec::new())).kind, ErrorKind::NotOnPrimaryRoot);
    let non_ascii = set("radio.tx.c0.gain_db", Value::Map(std::collections::BTreeMap::from([("é".to_owned(), Scalar::from(1))])));
    assert_eq!(err(server.handle(Request::Submit { action: non_ascii }, Vec::new())).kind, ErrorKind::Malformed);
    let stop = submit(&mut server, SessionAction::Stop { target: None }, Vec::new());
    assert!(matches!(stop.outcome, Outcome::Admitted { .. }));
    let ended = err(server.handle(Request::Submit { action: set("radio.tx.c0.stream", Value::Scalar(Scalar::Int(0))) }, Vec::new()));
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
            Ok(committed) => {
                if let Some(how) = ezsdr_kernel::schema::drift(&committed, &rendered) {
                    stale.push(format!("{name}: differs from the committed schema {how}"));
                }
            }
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
    let error = err(server.handle(Request::Connect { profile: Some(profile), lease: None, initial: None }, Vec::new()));
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(error.message, "EA-7: radio: no Module ezsdr.radio.nonexistent in this server");
    ok(server.handle(Request::Connect { profile: None, lease: None, initial: None }, Vec::new()));
    finish(&mut server);
}

/// A Spec Run on the default profile's radio recording `n` samples from its receive port.
fn receive_spec(n: i64) -> serde_json::Value {
    json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 2 }, { "id": "sink", "major": 1 }] },
        "resources": { "radio": { "kind": "radio.device", "requires": {
            "radio.rx.c0.stream": { "kind": "eq", "value": 0 },
            "radio.rx.s0.sample_rate_hz": { "kind": "eq", "value": 1.0e6 }
        } } },
        "outputs": [{
            "id": "rec", "kind": "sink.capture", "params": { "sink.capture_samples": n },
            "feed": { "port": { "component": "radio", "port": "rx0" }, "policy": "drop_oldest", "capacity": 64 }
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
    let (mut server, _) = connected_with(&temp.0, loopback());
    let waveform = ramp(1_000);
    assert!(matches!(submit(&mut server, repeat(), waveform.clone()).outcome, Outcome::Admitted { .. }));
    // The untimed repeat goes out 2 ms after it is admitted (RS-19), and 60 ms of Run time
    // puts the waveform on the air (RM-25).
    let (now, _, _) = status(&mut server);
    server.handle(Request::Advance { to: None, by: Some(Duration::new(now.domain(), 60_000_000)) }, Vec::new());
    let (uri, _) = capture_uri(&mut server, 3_000);
    let bytes = read(&mut server, &uri);
    assert_eq!(bytes.len(), 3_000 * 8);
    // The repeat is continuous, so the capture is the waveform at some offset, rotated by
    // the two LOs' phases (the `x3xx` profile draws them, MR-34): one unit phasor per sample.
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
    let (manifest, _) = finish(&mut server);
    assert!(!manifest.events.delivered.iter().any(|event| event.kind.as_str() == "radio.TIME_ERROR"));
}

#[test]
fn ea_10_connect_stands_at_t0() {
    let temp = TempDir::new("t0");
    let mut server = greeted(&temp.0);
    let Response::Connected { now, start_instant, dir, profile, root_epoch, .. } = ok(server.handle(Request::Connect { profile: None, lease: None, initial: None }, Vec::new())) else { panic!() };
    assert_eq!(now, start_instant);
    assert_eq!(now.ticks_in(now.domain()).unwrap(), 2_000_000_000, "the 2 s start lead on a nanosecond root");
    // The simulated root's epoch is no PPS edge (EA-10).
    assert!(matches!(root_epoch, ezsdr_kernel::time::EpochRef::Arbitrary { .. }), "{root_epoch:?}");
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
    let handled = server.handle(Request::Connect { profile: Some(profile), lease: None, initial: None }, Vec::new());
    assert!(handled.exit);
    let error = err(handled);
    assert_eq!(error.kind, ErrorKind::Ended);
    assert!(matches!(error.termination, Some(Termination::Failed { stage: Stage::Prepare, .. })));
    let written: Manifest = serde_json::from_str(&std::fs::read_to_string(server.dir().unwrap().join("manifest.json")).unwrap()).unwrap();
    assert!(matches!(written.termination.reason, Termination::Failed { stage: Stage::Prepare, .. }));
}

// ---------------------------------------------------------------- connect's initial streams (EA-10, RS-12)

type Initial = Option<BTreeMap<String, ResourceInit>>;
/// The radio's effective values and the stream names the reply gave.
type Joined = (BTreeMap<Key, Value>, BTreeMap<String, DirectionNames>);

/// `initial` for the default profile's `radio`.
fn radio_init(rx: Vec<StreamInit>, tx: Vec<StreamInit>) -> Initial {
    Some(BTreeMap::from([("radio".to_owned(), ResourceInit { rx, tx })]))
}

fn numbered(number: u32, channels: &[u32]) -> StreamInit {
    StreamInit { number: Some(number), channels: channels.iter().map(|c| ChannelRef::Number(*c)).collect(), ..StreamInit::default() }
}

fn named(name: &str, channels: &[u32]) -> StreamInit {
    StreamInit { number: None, name: Some(name.to_owned()), ..numbered(0, channels) }
}

/// `connect` with `initial`; the reply's effective values of `radio` and its stream names.
fn connect_with(server: &mut Server, initial: Initial) -> Result<Joined, ProtocolError> {
    let handled = server.handle(Request::Connect { profile: None, lease: None, initial }, Vec::new());
    match handled.reply {
        Reply::Result(Response::Connected { mut effective, streams, .. }) => Ok((effective.remove(&Ident::parse("radio").unwrap()).unwrap(), streams)),
        Reply::Result(other) => panic!("{other:?}"),
        Reply::Error(error) => Err(error),
    }
}

fn membership(effective: &BTreeMap<Key, Value>, direction: &str) -> Vec<i64> {
    (0..2)
        .map(|n| match &effective[&Key::parse(&format!("radio.{direction}.c{n}.stream")).unwrap()] {
            Value::Scalar(Scalar::Int(k)) => *k,
            other => panic!("{other:?}"),
        })
        .collect()
}

/// That no Run was made: the server wrote no Manifest, which every failed Run it makes does (EA-10).
fn assert_no_run(server: &Server) {
    assert!(!server.dir().unwrap().join("manifest.json").exists(), "a Run was created and sealed");
}

#[test]
fn ea_09_the_default_profile_connects_with_no_stream() {
    // RM-30: no channel is in a stream unless the Session asks; the default profile's recorder
    // is still linked to the radio's receive port `rx0`, and the Session connects and nothing flows.
    let temp = TempDir::new("no-stream");
    let mut server = greeted(&temp.0);
    let (effective, streams) = connect_with(&mut server, None).expect("connects");
    assert_eq!((membership(&effective, "rx"), membership(&effective, "tx")), (vec![-1, -1], vec![-1, -1]));
    assert!(streams.get("radio").is_none_or(|names| names.rx.is_empty() && names.tx.is_empty()));
    let (now, rate, _) = status(&mut server);
    ok(server.handle(Request::Advance { to: None, by: Some(seconds(now, rate, 1)) }, Vec::new()));
    let Response::Status { sample_clocks, .. } = ok(server.handle(Request::Status {}, Vec::new())) else { panic!() };
    assert!(sample_clocks.is_empty(), "no stream, no SampleClock: {sample_clocks:?}");
    let (manifest, _) = finish(&mut server);
    assert!(manifest.artifacts.is_empty(), "nothing was recorded");
    assert!(manifest.clocks.sample_clocks.is_empty());
}

#[test]
fn ea_10_a_direction_list_is_the_whole_channel_set() {
    let temp = TempDir::new("whole-set");
    let mut server = greeted(&temp.0);
    let (effective, _) = connect_with(&mut server, radio_init(Vec::new(), vec![numbered(0, &[1])])).expect("connects");
    assert_eq!(membership(&effective, "tx"), vec![-1, 0], "the one channel listed, and not c0");
    assert_eq!(membership(&effective, "rx"), vec![-1, -1], "a direction left out is in no stream");
    let (manifest, _) = finish(&mut server);
    let requires = &manifest.spec.body["resources"]["radio"]["requires"];
    assert_eq!(requires["radio.tx.c1.stream"], json!({ "kind": "eq", "value": 0 }), "the request reaches the Spec: {requires}");
    assert_eq!(manifest.spec.body["requirements"]["vocabularies"], json!([{ "id": "radio", "major": 2 }]));
}

#[test]
fn ea_10_two_streams_and_an_empty_direction() {
    let temp = TempDir::new("two-streams");
    let mut server = greeted(&temp.0);
    let (effective, _) = connect_with(&mut server, radio_init(vec![numbered(1, &[0]), numbered(0, &[1])], Vec::new())).expect("connects");
    assert_eq!(membership(&effective, "rx"), vec![1, 0], "streams by number, whatever order they are listed in");
    let (manifest, _) = finish(&mut server);
    assert_eq!(manifest.spec.body["resources"]["radio"]["requires"].as_object().unwrap().len(), 2, "only what was listed is asked");
    // `[]` and an absent direction are the same Session: every channel in no stream.
    let mut empty = greeted(&TempDir::new("empty").0);
    let (effective, _) = connect_with(&mut empty, radio_init(Vec::new(), Vec::new())).expect("connects");
    assert_eq!((membership(&effective, "rx"), membership(&effective, "tx")), (vec![-1, -1], vec![-1, -1]));
}

#[test]
fn ea_10_a_named_stream_gets_a_number_from_the_server() {
    let temp = TempDir::new("named");
    let mut server = greeted(&temp.0);
    let (effective, streams) = connect_with(&mut server, radio_init(vec![named("main", &[0]), numbered(0, &[1])], vec![named("out", &[0]), named("aux", &[1])])).expect("connects");
    assert_eq!(membership(&effective, "rx"), vec![1, 0], "the explicit 0 is kept and the name takes the smallest number left");
    assert_eq!(membership(&effective, "tx"), vec![0, 1], "names are numbered in request order");
    let names = &streams["radio"];
    assert_eq!((names.rx.get("main"), names.tx.get("out"), names.tx.get("aux")), (Some(&1), Some(&0), Some(&1)));
    finish(&mut server);
}

#[test]
fn ea_10_a_stream_name_is_used_once_across_both_directions() {
    let temp = TempDir::new("names");
    let mut server = greeted(&temp.0);
    let error = connect_with(&mut server, radio_init(vec![named("main", &[0])], vec![named("main", &[0])])).expect_err("one name for an rx and a tx stream");
    assert_eq!(error.kind, ErrorKind::Refused);
    assert!(error.message.contains("stream name main is used twice"), "{}", error.message);
    assert_no_run(&server);
    // One rule in both languages: non-empty, not only the digits 0-9, no ':', '#', white space or control character.
    for bad in ["", "3", "0123456789", "a:b", "a#b", "a b", "a\tb", "\u{1c}", "\u{3000}", "a\u{85}", "a\u{7f}"] {
        let error = connect_with(&mut server, radio_init(vec![named(bad, &[0])], Vec::new())).expect_err("not a stream name");
        assert!(error.message.contains("is not a stream name"), "{bad:?}: {}", error.message);
    }
    let (_, streams) =
        connect_with(&mut server, radio_init(vec![named("main", &[0]), named("\u{ff11}\u{ff12}", &[1])], vec![named("out", &[0])])).expect("distinct names connect, full-width digits among them");
    assert_eq!((streams["radio"].rx["main"], streams["radio"].tx["out"]), (0, 0));
    finish(&mut server);
}

#[test]
fn ea_10_a_request_the_layout_cannot_meet_is_refused_before_any_run() {
    let temp = TempDir::new("refused");
    let mut server = greeted(&temp.0);
    let (rx, tx) = (|streams: Vec<StreamInit>| radio_init(streams, Vec::new()), |streams: Vec<StreamInit>| radio_init(Vec::new(), streams));
    let cases: Vec<(&str, Initial, &str)> = vec![
        ("a channel past the last", rx(vec![numbered(0, &[2])]), "RM-4: no such channel c2"),
        ("the last channel is c1, not c0", tx(vec![numbered(0, &[0, 1, 2])]), "RM-4: no such channel c2"),
        ("a stream past the last", rx(vec![numbered(2, &[0])]), "RM-4: no such stream s2"),
        ("a name takes a number past the last", rx(vec![named("a", &[0]), named("b", &[1]), named("c", &[])]), "RM-4: no such stream s2"),
        ("a channel in two streams", rx(vec![numbered(0, &[0]), numbered(1, &[0])]), "channel c0 is given twice"),
        ("a channel twice in one stream", rx(vec![numbered(0, &[1, 1])]), "channel c1 is given twice"),
        ("a stream number twice", rx(vec![numbered(0, &[0]), numbered(0, &[1])]), "rx stream s0 is given twice"),
        ("a number and a name", rx(vec![StreamInit { name: Some("x".to_owned()), ..numbered(0, &[0]) }]), "exactly one of number and name"),
        ("a channel name this profile does not give", rx(vec![StreamInit { channels: vec![ChannelRef::Name("loop-rx".to_owned())], ..numbered(0, &[]) }]), "no channel is named \"loop-rx\""),
        ("a value for a channel the stream lacks", rx(vec![StreamInit { gain: BTreeMap::from([("1".to_owned(), 5.0)]), ..numbered(0, &[0]) }]), "c1 has a value but is not a channel of its stream"),
        (
            "a value key written \"00\" beside \"0\" is no number and no legal name",
            rx(vec![StreamInit { gain: BTreeMap::from([("0".to_owned(), 5.0), ("00".to_owned(), 6.0)]), ..numbered(0, &[0]) }]),
            "\"00\" is not a channel name",
        ),
        (
            "a value key written \"+0\" is a name, and this profile gives none",
            rx(vec![StreamInit { gain: BTreeMap::from([("0".to_owned(), 5.0), ("+0".to_owned(), 6.0)]), ..numbered(0, &[0]) }]),
            "no channel is named \"+0\"",
        ),
        ("a value keyed by a name this profile does not give", rx(vec![StreamInit { gain: BTreeMap::from([("loop-rx".to_owned(), 5.0)]), ..numbered(0, &[0]) }]), "no channel is named \"loop-rx\""),
        ("a rate that is not finite", rx(vec![StreamInit { sample_rate: Some(f64::NAN), ..numbered(0, &[0]) }]), "sample_rate is not a finite number"),
        ("a frequency that is not finite", rx(vec![StreamInit { frequency: BTreeMap::from([("0".to_owned(), f64::INFINITY)]), ..numbered(0, &[0]) }]), "frequency_hz is not a finite number"),
        ("a resource that is no radio", Some(BTreeMap::from([("rec".to_owned(), ResourceInit::default())])), "no radio of this profile"),
    ];
    for (what, initial, message) in cases {
        let error = connect_with(&mut server, initial).expect_err(what);
        assert_eq!(error.kind, ErrorKind::Refused, "{what}: {}", error.message);
        assert!(error.message.contains(message), "{what}: {}", error.message);
        assert_no_run(&server);
    }
    // The boundary the refusals sit on: the last channel and the last stream of each direction.
    let (effective, _) = connect_with(&mut server, radio_init(vec![numbered(1, &[1])], vec![numbered(1, &[0, 1])])).expect("the last stream and channel exist");
    assert_eq!((membership(&effective, "rx"), membership(&effective, "tx")), (vec![-1, 1], vec![1, 1]));
    finish(&mut server);
}

// ---------------------------------------------------------------- channel names (RM-28, EA-10a)

type Tables = BTreeMap<String, DirectionNames>;

/// The default profile with the radio's layout lists replaced.
fn profile_with(dir: &Path, rx: &[&str], tx: &[&str]) -> serde_json::Value {
    let mut profile = ezsdr_server::default_profile(&dir.to_string_lossy());
    profile["bindings"]["radio"]["selector"]["rx"] = json!(rx);
    profile["bindings"]["radio"]["selector"]["tx"] = json!(tx);
    profile
}

/// The owner's named layout: rx c0 `loop-rx`, c1 `side`; tx c0 `loop-tx`, c1 unnamed.
fn named_profile(dir: &Path) -> serde_json::Value {
    profile_with(dir, &["0:A:ubx:0#loop-rx", "0:B:ubx:0#side"], &["0:A:ubx:0#loop-tx", "0:B:ubx:0"])
}

/// A second Mock radio, `second`, with one channel each way, named `r2-rx` and `r2-tx`.
fn two_radios(dir: &Path) -> serde_json::Value {
    let mut profile = named_profile(dir);
    let mut second = profile["bindings"]["radio"].clone();
    second["selector"]["id"] = json!("second");
    second["selector"]["rx"] = json!(["0:A:ubx:0#r2-rx"]);
    second["selector"]["tx"] = json!(["0:A:ubx:0#r2-tx"]);
    profile["bindings"]["second"] = second;
    profile
}

struct Named {
    effective: BTreeMap<Ident, BTreeMap<Key, Value>>,
    streams: Tables,
    channels: Tables,
}

fn connect_profile(server: &mut Server, profile: serde_json::Value, initial: Initial) -> Result<Named, ProtocolError> {
    match server.handle(Request::Connect { profile: Some(profile), lease: None, initial }, Vec::new()).reply {
        Reply::Result(Response::Connected { effective, streams, channels, .. }) => Ok(Named { effective, streams, channels }),
        Reply::Result(other) => panic!("{other:?}"),
        Reply::Error(error) => Err(error),
    }
}

fn radio_values<'a>(connected: &'a Named, radio: &str) -> &'a BTreeMap<Key, Value> {
    &connected.effective[&Ident::parse(radio).unwrap()]
}

fn name_ref(name: &str) -> ChannelRef {
    ChannelRef::Name(name.to_owned())
}

fn init_of(radio: &str, rx: Vec<StreamInit>, tx: Vec<StreamInit>) -> Initial {
    Some(BTreeMap::from([(radio.to_owned(), ResourceInit { rx, tx })]))
}

fn stream_of(number: u32, channels: Vec<ChannelRef>) -> StreamInit {
    StreamInit { number: Some(number), channels, ..StreamInit::default() }
}

fn bad_names() -> Vec<String> {
    let cases: serde_json::Value = serde_json::from_str(include_str!("../../ezsdr-radio/tests/fixtures/names.json")).unwrap();
    cases["bad"].as_array().unwrap().iter().map(|name| name.as_str().unwrap().to_owned()).collect()
}

#[test]
fn ea_10_a_named_selector_connects_as_the_unnamed_one_and_the_record_holds_numbers() {
    // RM-28: the same layout with and without names, the same call by numbers: the same effective
    // values, Spec, admission and log; the names are only in the recorded profile, never in a key,
    // a target or an event.
    let run = |profile_of: &dyn Fn(&Path) -> serde_json::Value| {
        let temp = TempDir::new("named-same");
        let mut server = greeted(&temp.0);
        let initial = init_of("radio", vec![StreamInit { gain: BTreeMap::from([("1".to_owned(), 7.0)]), sample_rate: Some(2.0e6), ..numbered(0, &[0, 1]) }], vec![numbered(0, &[0])]);
        let connected = connect_profile(&mut server, profile_of(&temp.0), initial).expect("connects");
        // A transmit gain on a stopped stream is admitted and logged by its numbered key.
        let entry = submit(&mut server, set("radio.tx.c0.gain_db", Value::num(9.0).unwrap()), Vec::new());
        assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "{:?}", entry.outcome);
        submit(&mut server, repeat(), Vec::new());
        let (manifest, _) = finish(&mut server);
        (connected.effective, manifest)
    };
    let (plain_effective, plain) = run(&|dir| profile_with(dir, &["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]));
    let (named_effective, named) = run(&|dir| named_profile(dir));
    assert_eq!(plain_effective, named_effective, "the effective values");
    let section = |m: &Manifest, name: &str| serde_json::to_value(m).unwrap()[name].clone();
    for name in ["spec", "admission", "action_log", "events", "clocks"] {
        assert_eq!(section(&plain, name), section(&named, name), "{name}");
        let text = section(&named, name).to_string();
        assert!(!text.contains("loop-rx") && !text.contains("side") && !text.contains("loop-tx") && !text.contains('#'), "{name} holds a name: {text}");
    }
    let radio_keys = named.action_log.iter().map(|entry| format!("{:?}", entry.action)).collect::<String>();
    assert!(radio_keys.contains("radio.tx.c0.gain_db"), "the log's key is the numbered one: {radio_keys}");
    assert!(section(&named, "binding").to_string().contains("0:A:ubx:0#loop-rx"), "the recorded profile carries the names");
    assert!(!section(&plain, "binding").to_string().contains('#'));
    assert_ne!(section(&plain, "binding"), section(&named, "binding"));
}

#[test]
fn ea_09_a_bad_selector_name_is_refused_before_any_run() {
    let temp = TempDir::new("bad-selector-name");
    let mut server = greeted(&temp.0);
    let mut refuse = |rx: &[&str], tx: &[&str], needle: &str| {
        let error = connect_profile(&mut server, profile_with(&temp.0, rx, tx), None).err().expect("refused");
        assert_eq!(error.kind, ErrorKind::Refused, "{rx:?} {tx:?}");
        assert!(error.message.starts_with("EA-7: radio: ") && error.message.contains(needle), "{rx:?} {tx:?}: {}", error.message);
    };
    for name in bad_names() {
        refuse(&[&format!("0:A:ubx:0#{name}")], &[], "has the name");
    }
    refuse(&["0:A:ubx:0#a", "0:B:ubx:0#a"], &[], "repeats the name `a`");
    refuse(&["0:A:ubx:0#a"], &["0:A:ubx:0#a"], "repeats the name `a`");
    assert_no_run(&server);
}

#[test]
fn ea_09_a_name_is_unique_in_the_whole_profile() {
    let temp = TempDir::new("name-unique");
    let mut server = greeted(&temp.0);
    let clash = |rx: &str, tx: &str| {
        let mut profile = two_radios(&temp.0);
        profile["bindings"]["second"]["selector"]["rx"] = json!([rx]);
        profile["bindings"]["second"]["selector"]["tx"] = json!([tx]);
        profile
    };
    // Whether or not the call uses names, and in either direction.
    for (profile, initial) in [
        (clash("0:A:ubx:0#loop-rx", "0:A:ubx:0#r2-tx"), None),
        (clash("0:A:ubx:0#r2-rx", "0:A:ubx:0#loop-tx"), None),
        (clash("0:A:ubx:0#loop-tx", "0:A:ubx:0#r2-tx"), init_of("radio", vec![numbered(0, &[0])], Vec::new())),
        (clash("0:A:ubx:0#side", "0:A:ubx:0#r2-tx"), init_of("second", Vec::new(), vec![numbered(0, &[0])])),
    ] {
        let error = connect_profile(&mut server, profile, initial).err().expect("refused");
        assert_eq!(error.kind, ErrorKind::Refused);
        assert!(error.message.starts_with("EA-7: the channel name ") && error.message.contains("radio and by second") || error.message.contains("second and by radio"), "{}", error.message);
        assert_no_run(&server);
    }
    let connected = connect_profile(&mut server, two_radios(&temp.0), None).expect("distinct names connect");
    assert_eq!(connected.channels.len(), 2);
    finish(&mut server);
}

#[test]
fn ea_10_a_channel_name_is_resolved_to_its_number() {
    let temp = TempDir::new("name-resolve");
    let mut server = greeted(&temp.0);
    let at = |connected: &Named, key: &str| radio_values(connected, "radio")[&Key::parse(key).unwrap()].clone();
    // A number beside a name, and value keys by name or by number: `loop-rx` is c0, held here as the number 0, and `side` is c1.
    let initial = init_of(
        "radio",
        vec![StreamInit {
            gain: BTreeMap::from([("side".to_owned(), 5.0), ("0".to_owned(), 6.0)]),
            frequency: BTreeMap::from([("loop-rx".to_owned(), 2.4e9)]),
            ..stream_of(0, vec![ChannelRef::Number(0), name_ref("side")])
        }],
        vec![StreamInit { antenna: BTreeMap::from([("loop-tx".to_owned(), "TX/RX".to_owned())]), ..stream_of(0, vec![name_ref("loop-tx")]) }],
    );
    let connected = connect_profile(&mut server, named_profile(&temp.0), initial).expect("connects");
    assert_eq!((membership(radio_values(&connected, "radio"), "rx"), membership(radio_values(&connected, "radio"), "tx")), (vec![0, 0], vec![0, -1]));
    assert_eq!(at(&connected, "radio.rx.c1.gain_db"), Value::num(5.0).unwrap(), "`side` is c1");
    assert_eq!(at(&connected, "radio.rx.c0.gain_db"), Value::num(6.0).unwrap(), "the number 0 is c0");
    assert_eq!(at(&connected, "radio.rx.c0.frequency_hz"), Value::num(2.4e9).unwrap(), "`loop-rx` is c0");
    assert_eq!(at(&connected, "radio.tx.c0.antenna"), Value::from("TX/RX".to_owned()));
    finish(&mut server);
    // A number for a channel its stream holds by name, and a name for one it holds by number.
    let mut server = greeted(&TempDir::new("name-resolve-mixed").0);
    let temp = TempDir::new("name-resolve-b");
    let initial = init_of(
        "radio",
        vec![
            StreamInit { gain: BTreeMap::from([("0".to_owned(), 3.0)]), ..stream_of(0, vec![name_ref("loop-rx")]) },
            StreamInit { gain: BTreeMap::from([("side".to_owned(), 4.0)]), ..stream_of(1, vec![ChannelRef::Number(1)]) },
        ],
        Vec::new(),
    );
    let connected = connect_profile(&mut server, named_profile(&temp.0), initial).expect("both spellings of one channel are the one channel");
    assert_eq!(at(&connected, "radio.rx.c0.gain_db"), Value::num(3.0).unwrap());
    assert_eq!(at(&connected, "radio.rx.c1.gain_db"), Value::num(4.0).unwrap());
    finish(&mut server);
}

#[test]
fn ea_10_a_key_is_a_number_only_when_it_is_a_canonical_ascii_decimal() {
    // `+0`, `-0` and full-width digits are legal names; with them on c1, a number-looking parse would pick c0.
    let temp = TempDir::new("name-spelling");
    let mut server = greeted(&temp.0);
    for name in ["+0", "-0", "\u{ff11}", "00a"] {
        let profile = profile_with(&temp.0, &["0:A:ubx:0", &format!("0:B:ubx:0#{name}")], &["0:A:ubx:0", "0:B:ubx:0"]);
        let initial = init_of("radio", vec![StreamInit { gain: BTreeMap::from([(name.to_owned(), 5.0)]), ..stream_of(0, vec![name_ref(name)]) }], Vec::new());
        let connected = connect_profile(&mut server, profile, initial).unwrap_or_else(|e| panic!("{name:?}: {}", e.message));
        assert_eq!(membership(radio_values(&connected, "radio"), "rx"), vec![-1, 0], "{name:?} is c1");
        assert_eq!(radio_values(&connected, "radio")[&Key::parse("radio.rx.c1.gain_db").unwrap()], Value::num(5.0).unwrap(), "{name:?}");
        assert_eq!(connected.channels["radio"].rx[name], 1);
        finish(&mut server);
        server = greeted(&TempDir::new("name-spelling-next").0);
    }
    // Digits only is never a name: written with a leading zero, past u32 or full of zeros, it is refused.
    for text in ["00", "01", "4294967296", "18446744073709551616"] {
        let initial = init_of("radio", vec![StreamInit { gain: BTreeMap::from([(text.to_owned(), 5.0)]), ..stream_of(0, vec![ChannelRef::Number(0)]) }], Vec::new());
        let error = connect_profile(&mut server, named_profile(&temp.0), initial).err().expect(text);
        assert!(error.message.contains("is not a channel name"), "{text}: {}", error.message);
        assert_no_run(&server);
    }
    // A channel written as a name that is digits only is refused the same way.
    let error = connect_profile(&mut server, named_profile(&temp.0), init_of("radio", vec![stream_of(0, vec![name_ref("1")])], Vec::new())).err().unwrap();
    assert!(error.message.contains("is not a channel name"), "{}", error.message);
}

#[test]
fn ea_10_a_name_that_cannot_be_resolved_is_refused() {
    let temp = TempDir::new("name-refused");
    let mut server = greeted(&temp.0);
    let rx = |stream: StreamInit| init_of("radio", vec![stream], Vec::new());
    let tx = |stream: StreamInit| init_of("radio", Vec::new(), vec![stream]);
    let cases: Vec<(&str, Initial, &str)> = vec![
        ("an unknown name", rx(stream_of(0, vec![name_ref("nowhere")])), "no channel is named \"nowhere\""),
        ("a transmit name in rx", rx(stream_of(0, vec![name_ref("loop-tx")])), "is a transmit channel, not a receive channel"),
        ("a receive name in tx", tx(stream_of(0, vec![name_ref("side")])), "is a receive channel, not a transmit channel"),
        ("a name twice", rx(stream_of(0, vec![name_ref("side"), name_ref("side")])), "channel c1 is given twice"),
        ("a name and its number", rx(stream_of(0, vec![name_ref("side"), ChannelRef::Number(1)])), "channel c1 is given twice"),
        (
            "a name in two streams",
            rx(stream_of(0, vec![name_ref("side")]))
                .map(|mut i| {
                    i.get_mut("radio").unwrap().rx.push(stream_of(1, vec![name_ref("side")]));
                    i
                })
                .unwrap()
                .into(),
            "channel c1 is given twice",
        ),
        (
            "two value keys for one channel",
            rx(StreamInit { gain: BTreeMap::from([("1".to_owned(), 5.0), ("side".to_owned(), 6.0)]), ..stream_of(0, vec![ChannelRef::Number(1)]) }),
            "gain_db has two values for c1",
        ),
        (
            "two antenna keys for one channel",
            rx(StreamInit { antenna: BTreeMap::from([("1".to_owned(), "RX2".to_owned()), ("side".to_owned(), "TX/RX".to_owned())]), ..stream_of(0, vec![name_ref("side")]) }),
            "antenna has two values for c1",
        ),
        (
            "a value for a name not in the stream",
            rx(StreamInit { gain: BTreeMap::from([("side".to_owned(), 5.0)]), ..stream_of(0, vec![name_ref("loop-rx")]) }),
            "c1 has a value but is not a channel of its stream",
        ),
        (
            "a value keyed by a name of the other direction",
            rx(StreamInit { gain: BTreeMap::from([("loop-tx".to_owned(), 5.0)]), ..stream_of(0, vec![ChannelRef::Number(0)]) }),
            "is a transmit channel, not a receive channel",
        ),
    ];
    for (what, initial, message) in cases {
        let error = connect_profile(&mut server, named_profile(&temp.0), initial).err().expect(what);
        assert_eq!(error.kind, ErrorKind::Refused, "{what}: {}", error.message);
        assert!(error.message.contains(message), "{what}: {}", error.message);
        assert_no_run(&server);
    }
}

#[test]
fn ea_10_a_stream_of_names_goes_to_the_radio_that_owns_them() {
    let temp = TempDir::new("name-owner");
    let mut server = greeted(&temp.0);
    // Written under `radio`, a radio that exists, and under a key that names no radio at all.
    for key in ["radio", "nowhere"] {
        let initial = init_of(key, vec![StreamInit { name: Some("main".to_owned()), number: None, ..stream_of(0, vec![name_ref("r2-rx")]) }], vec![stream_of(0, vec![name_ref("r2-tx")])]);
        let connected = connect_profile(&mut server, two_radios(&temp.0), initial).unwrap_or_else(|e| panic!("{key}: {}", e.message));
        assert_eq!(connected.effective[&Ident::parse("second").unwrap()][&Key::parse("radio.rx.c0.stream").unwrap()], Value::from(0), "{key}");
        assert_eq!(membership(radio_values(&connected, "radio"), "rx"), vec![-1, -1], "{key}: the radio of the key keeps its defaults");
        assert_eq!(connected.streams["second"].rx["main"], 0, "{key}: the stream name is the owner's");
        assert!(!connected.streams.contains_key("radio"), "{key}");
        finish(&mut server);
        server = greeted(&TempDir::new("name-owner-next").0);
    }
    // Stream numbers are the owner's: two streams numbered 0 on two radios are two streams; two on one radio are not.
    let initial = init_of("radio", vec![stream_of(0, vec![name_ref("side")]), stream_of(0, vec![name_ref("r2-rx")])], Vec::new());
    let connected = connect_profile(&mut server, two_radios(&temp.0), initial).expect("one stream 0 on each radio");
    assert_eq!(
        (membership(radio_values(&connected, "radio"), "rx"), connected.effective[&Ident::parse("second").unwrap()][&Key::parse("radio.rx.c0.stream").unwrap()].clone()),
        (vec![-1, 0], Value::from(0))
    );
    finish(&mut server);
    let mut server = greeted(&TempDir::new("name-owner-twice").0);
    let initial = init_of("radio", vec![stream_of(0, vec![name_ref("side")]), stream_of(0, vec![name_ref("loop-rx")])], Vec::new());
    let error = connect_profile(&mut server, two_radios(&temp.0), initial).err().expect("refused");
    assert!(error.message.contains("rx stream s0 is given twice"), "{}", error.message);
    // A stream of numbers is not moved: under a key naming no radio it is refused as before.
    let error = connect_profile(&mut server, two_radios(&temp.0), init_of("nowhere", vec![numbered(0, &[0])], Vec::new())).err().expect("refused");
    assert!(error.message.contains("no radio of this profile"), "{}", error.message);
    assert_no_run(&server);
}

#[test]
fn ea_10_a_stream_of_two_radios_is_refused() {
    let temp = TempDir::new("name-mixed");
    let mut server = greeted(&temp.0);
    let rx = |stream: StreamInit| init_of("radio", vec![stream], Vec::new());
    let cases: Vec<(&str, Initial, &str)> = vec![
        ("names of two radios", rx(stream_of(0, vec![name_ref("loop-rx"), name_ref("r2-rx")])), "mixes the channels of radios"),
        ("a number of radio, a name of second", rx(stream_of(0, vec![ChannelRef::Number(0), name_ref("r2-rx")])), "mixes channel numbers of radio with channel names of radio second"),
        (
            "a value key by number beside a name of second",
            rx(StreamInit { gain: BTreeMap::from([("0".to_owned(), 1.0)]), ..stream_of(0, vec![name_ref("r2-rx")]) }),
            "mixes channel numbers of radio with channel names of radio second",
        ),
        (
            "a value key by name of second beside a number",
            rx(StreamInit { gain: BTreeMap::from([("r2-rx".to_owned(), 1.0)]), ..stream_of(0, vec![ChannelRef::Number(0)]) }),
            "mixes channel numbers of radio with channel names of radio second",
        ),
        (
            "a number written under the other radio",
            init_of("second", vec![stream_of(0, vec![ChannelRef::Number(0), name_ref("side")])], Vec::new()),
            "mixes channel numbers of second with channel names of radio radio",
        ),
    ];
    for (what, initial, message) in cases {
        let error = connect_profile(&mut server, two_radios(&temp.0), initial).err().expect(what);
        assert_eq!(error.kind, ErrorKind::Refused, "{what}");
        assert!(error.message.contains(message), "{what}: {}", error.message);
        assert_no_run(&server);
    }
    // The same number and name on the radio that owns both is one stream.
    let connected =
        connect_profile(&mut server, two_radios(&temp.0), init_of("second", vec![stream_of(0, vec![ChannelRef::Number(0)])], vec![stream_of(0, vec![name_ref("r2-tx")])])).expect("one radio");
    assert_eq!(connected.effective[&Ident::parse("second").unwrap()][&Key::parse("radio.tx.c0.stream").unwrap()], Value::from(0));
    finish(&mut server);
}

#[test]
fn ea_10_connected_carries_the_channel_name_table() {
    let temp = TempDir::new("name-table");
    let mut server = greeted(&temp.0);
    let table = |pairs: &[(&str, u32)]| pairs.iter().map(|(name, n)| ((*name).to_owned(), *n)).collect::<BTreeMap<_, _>>();
    // With no `initial`, and an unnamed second tx entry: the table lists only named entries, by position.
    let connected = connect_profile(&mut server, named_profile(&temp.0), None).expect("connects");
    assert_eq!(connected.channels["radio"].rx, table(&[("loop-rx", 0), ("side", 1)]));
    assert_eq!(connected.channels["radio"].tx, table(&[("loop-tx", 0)]));
    assert_eq!(connected.channels.len(), 1);
    finish(&mut server);
    // A name on the second entry only keeps its position; an entry per radio, empty for an unnamed one.
    let mut server = greeted(&TempDir::new("name-table-two").0);
    let mut profile = two_radios(&temp.0);
    profile["bindings"]["radio"]["selector"]["rx"] = json!(["0:A:ubx:0", "0:B:ubx:0#late"]);
    profile["bindings"]["second"]["selector"]["rx"] = json!(["0:A:ubx:0"]);
    profile["bindings"]["second"]["selector"]["tx"] = json!(["0:A:ubx:0"]);
    let connected = connect_profile(&mut server, profile, init_of("radio", vec![numbered(0, &[0])], Vec::new())).expect("connects");
    assert_eq!(connected.channels["radio"].rx, table(&[("late", 1)]));
    assert_eq!(connected.channels["second"], DirectionNames::default(), "a radio that names nothing has an empty table");
    assert_eq!(connected.channels.len(), 2, "the sink and the Authority are no radios");
    finish(&mut server);
    // An `ideal` radio names nothing.
    let mut server = greeted(&TempDir::new("name-table-ideal").0);
    let mut profile = named_profile(&temp.0);
    profile["bindings"]["radio"]["profile"] = json!({ "name": "ideal", "version": { "major": 2, "minor": 0, "patch": 0 } });
    profile["bindings"]["radio"]["selector"] = json!({ "id": "radio", "rx": 2, "tx": 2 });
    let connected = connect_profile(&mut server, profile, None).expect("connects");
    assert_eq!(connected.channels["radio"], DirectionNames::default());
    finish(&mut server);
}

#[test]
fn ea_14_a_child_run_repeats_its_parent_s_selector_names_included() {
    // A name is part of the binding (RM-28): the Kernel keeps the selector opaque, so a child whose
    // binding drops or changes a name binds another instance than its parent (RS-25a).
    let temp = TempDir::new("name-child");
    let mut server = greeted(&temp.0);
    let parent = named_profile(&temp.0);
    connect_profile(&mut server, parent.clone(), init_of("radio", vec![numbered(0, &[0])], Vec::new())).expect("connects");
    let child = |profile: serde_json::Value| ezsdr_server::without_feeds(&profile);
    let derived = ok(run_child(&mut server, receive_spec(1_000), None, Some(10_000_000)));
    let Response::Ran { entry, .. } = derived else { panic!() };
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "the derived profile repeats the names");
    let Response::Ran { entry, .. } = ok(run_child(&mut server, receive_spec(1_000), Some(child(parent.clone())), Some(10_000_000))) else { panic!() };
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "the same names: {:?}", entry.outcome);
    let mut stripped = parent.clone();
    stripped["bindings"]["radio"]["selector"]["rx"] = json!(["0:A:ubx:0", "0:B:ubx:0"]);
    let mut renamed = parent.clone();
    renamed["bindings"]["radio"]["selector"]["rx"] = json!(["0:A:ubx:0#loop-in", "0:B:ubx:0#side"]);
    for (what, profile) in [("names removed", stripped), ("a name changed", renamed)] {
        let Response::Ran { entry, manifest: None, .. } = ok(run_child(&mut server, receive_spec(1_000), Some(child(profile)), Some(10_000_000))) else { panic!("{what}") };
        let Outcome::Rejected { violations } = entry.outcome else { panic!("{what}: admitted") };
        assert!(violations[0].reason.starts_with("RS-25a: radio binds an instance its parent does not"), "{what}: {}", violations[0].reason);
    }
    // A child's own profile with a duplicate name is refused at assembly, as a connect's is.
    let mut clash = two_radios(&temp.0);
    clash["bindings"]["second"]["selector"]["rx"] = json!(["0:A:ubx:0#loop-rx"]);
    let refused = err(run_child(&mut server, receive_spec(1_000), Some(child(clash)), Some(10_000_000)));
    assert!(refused.message.contains("a name is used once in a profile"), "{}", refused.message);
    finish(&mut server);
}

#[test]
fn ea_10_a_wrong_typed_initial_is_a_protocol_error() {
    // The frame's types are the schema's: a string where a list of channels goes never reaches a Session.
    let temp = TempDir::new("typed");
    let connect = "{\"request\":{\"op\":\"connect\",\"initial\":{\"radio\":{\"rx\":[{\"number\":0,\"channels\":\"all\"}]}}}}\n";
    let (frames, _) = serve_bytes(&temp.0, [HELLO.as_bytes(), connect.as_bytes()].concat());
    assert_eq!(error_of(&frames[1].0).kind, ErrorKind::Protocol, "{:?}", frames[1].0.reply);
    let unknown = "{\"request\":{\"op\":\"connect\",\"initial\":{\"radio\":{\"rx\":[{\"number\":0,\"channels\":[0],\"bandwidth\":1}]}}}}\n";
    let (frames, _) = serve_bytes(&temp.0, [HELLO.as_bytes(), unknown.as_bytes()].concat());
    assert_eq!(error_of(&frames[1].0).kind, ErrorKind::Protocol, "an unknown member is refused, not ignored: {:?}", frames[1].0.reply);
}

#[test]
fn ea_10_initial_requests_name_their_resource() {
    // Two radios: the numbered maps configure the one the resource name gives, and the other keeps RM-30's defaults.
    let temp = TempDir::new("two-radios");
    let mut profile = ezsdr_server::default_profile(&temp.0.to_string_lossy());
    let mut second = profile["bindings"]["radio"].clone();
    second["selector"]["id"] = json!("second");
    // One channel each way, so a channel c1 is the first radio's and no one else's.
    second["selector"]["rx"] = json!(["0:A:ubx:0"]);
    second["selector"]["tx"] = json!(["0:B:ubx:0"]);
    profile["bindings"]["second"] = second;
    let mut server = greeted(&temp.0);
    let initial = |channel| Some(BTreeMap::from([("second".to_owned(), ResourceInit { rx: vec![numbered(0, &[channel])], tx: Vec::new() })]));
    let error = err(server.handle(Request::Connect { profile: Some(profile.clone()), lease: None, initial: initial(1) }, Vec::new()));
    assert!(error.message.contains("RM-4: no such channel c1"), "`second` has one receive channel: {}", error.message);
    assert_no_run(&server);
    let initial = initial(0);
    let Response::Connected { effective, .. } = ok(server.handle(Request::Connect { profile: Some(profile), lease: None, initial }, Vec::new())) else { panic!() };
    let rx_c0 = |name: &str| effective[&Ident::parse(name).unwrap()][&Key::parse("radio.rx.c0.stream").unwrap()].clone();
    assert_eq!((rx_c0("second"), rx_c0("radio")), (Value::from(0), Value::from(-1)));
    finish(&mut server);
}

#[test]
fn ea_09_initial_requests_mean_the_same_under_every_profile_source() {
    // The built-in default, the same profile sent explicitly and one a configured file holds ask the same.
    let temp = TempDir::new("sources");
    let initial = || radio_init(vec![numbered(0, &[1])], vec![numbered(0, &[0])]);
    let reply = |config: Config, profile: Option<serde_json::Value>| {
        let mut server = Server::new(config);
        ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
        let Response::Connected { effective, .. } = ok(server.handle(Request::Connect { profile, lease: None, initial: initial() }, Vec::new())) else { panic!() };
        finish(&mut server);
        effective
    };
    let profile = ezsdr_server::default_profile(&temp.0.to_string_lossy());
    let path = temp.0.join("lab.json");
    std::fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
    let built_in = reply(config(&temp.0), None);
    assert_eq!(built_in, reply(config(&temp.0), Some(profile)), "the profile sent explicitly");
    assert_eq!(built_in, reply(Config { default_profile: Some(path), ..config(&temp.0) }, None), "the configured default profile");
    let radio = &built_in[&Ident::parse("radio").unwrap()];
    assert_eq!((membership(radio, "rx"), membership(radio, "tx")), (vec![-1, 0], vec![0, -1]));
}

#[test]
fn ea_06_connect_initial_is_in_the_schema() {
    // The frames' committed schemas carry the initial streams, with a channel an integer or a
    // name (so that channel names add no schema change) and the server's name table in the reply.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/server");
    let request: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("request_frame.v1.json")).unwrap()).unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("reply_frame.v1.json")).unwrap()).unwrap();
    let text = request.to_string();
    assert!(text.contains("\"initial\"") && text.contains("\"StreamInit\"") && text.contains("\"ChannelRef\""), "{text}");
    let channel = &request["$defs"]["ChannelRef"];
    let kinds: Vec<_> = channel["anyOf"].as_array().unwrap().iter().map(|v| v["type"].as_str().unwrap().to_owned()).collect();
    assert_eq!(kinds, ["integer", "string"], "{channel}");
    assert!(reply.to_string().contains("\"streams\"") && reply.to_string().contains("\"DirectionNames\"") && reply.to_string().contains("\"channels\""));
}

#[test]
fn ea_11_submit_returns_the_logged_entry() {
    let temp = TempDir::new("submit");
    let (mut server, _) = connected_with(&temp.0, loopback());
    let admitted = submit(&mut server, repeat(), ramp(1_000));
    let rejected = submit(&mut server, set("radio.rx.s0.sample_rate_hz", Value::num(1.0e12).unwrap()), Vec::new());
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
    let root = status(&mut server).0.domain();
    let Response::Advanced { now: later, .. } = ok(server.handle(Request::Advance { to: None, by: Some(Duration::new(root, 1_000_000)) }, Vec::new())) else { panic!() };
    assert_eq!(later.ticks_in(later.domain()).unwrap() - now.ticks_in(now.domain()).unwrap(), 1_000_000, "1 ms on a nanosecond root");
    // A wait that returns early echoes the horizon it would have stood at, not `now`
    // (Review I, P2-5).
    let Response::Status { events: before, now: asked, .. } = ok(server.handle(Request::Status {}, Vec::new())) else { panic!() };
    submit(&mut server, capture(500), Vec::new());
    let Response::Waited { index: Some(_), now: early, horizon: far, .. } =
        ok(server.handle(Request::WaitFor { kinds: vec![written()], from: before, within: Some(Duration::new(root, 1_000_000_000)), until: None }, Vec::new()))
    else {
        panic!()
    };
    assert_eq!(far.ticks_in(far.domain()).unwrap(), asked.ticks_in(asked.domain()).unwrap() + 1_000_000_000);
    assert!(early.ticks_in(early.domain()).unwrap() < far.ticks_in(far.domain()).unwrap());
    let (uri, at) = capture_uri(&mut server, 2_000);
    let Response::Events { events, next } = ok(server.handle(Request::Events { from: 0 }, Vec::new())) else { panic!() };
    let index = events.iter().rposition(|event| event.kind == written()).unwrap();
    assert_eq!(next, events.len());
    assert_eq!(events[index].payload["artifact"]["uri"], json!(uri));
    assert_eq!(events[index].time, at, "the wait ended at the round that delivered the event");
    let Response::Events { events: tail, .. } = ok(server.handle(Request::Events { from: index }, Vec::new())) else { panic!() };
    assert_eq!(tail[0], events[index]);
    let Response::Waited { index: none, now: horizon, .. } =
        ok(server.handle(Request::WaitFor { kinds: vec![written()], from: next, within: Some(Duration::new(root, 3_000_000)), until: None }, Vec::new()))
    else {
        panic!()
    };
    assert_eq!(none, None);
    assert_eq!(horizon.ticks_in(horizon.domain()).unwrap(), at.ticks_in(at.domain()).unwrap() + 3_000_000);
    // `until` keeps a deadline across waits: the reply's `horizon` is where it would stand.
    let deadline = TimePoint::new(horizon.domain(), horizon.ticks_in(horizon.domain()).unwrap() + 2_000_000);
    let Response::Waited { index: none, now: stood, horizon: echoed, .. } = ok(server.handle(Request::WaitFor { kinds: vec![written()], from: next, within: None, until: Some(deadline) }, Vec::new()))
    else {
        panic!()
    };
    assert_eq!((none, stood, echoed), (None, deadline, deadline));
    assert_eq!(err(server.handle(Request::WaitFor { kinds: vec![], from: 0, within: Some(Duration::new(root, 1)), until: Some(deadline) }, Vec::new())).kind, ErrorKind::Protocol);
    finish(&mut server);
}

#[test]
fn ea_12_durations_round_up() {
    let clocks = ClockRegistry::new();
    let root = clocks.allocate_id().unwrap();
    clocks.register(ClockDomain::root(root, Rational::new(3, 1).unwrap(), EpochRef::Arbitrary { set_by: "test".to_owned() })).unwrap();
    let at = TimePoint::new(root, 0);
    assert_eq!(ezsdr_server::ticks(&clocks, at, 1), Some(1), "a nanosecond on a 3 Hz clock is one tick, rounded up");
    assert_eq!(ezsdr_server::ticks(&clocks, at, 1_000_000_000), Some(3));
    assert_eq!(ezsdr_server::ticks(&clocks, at, 0), Some(0));
}

#[test]
fn ea_12_a_duration_counts_the_primary_root() {
    // EA-12 (spec 20, VF-2): `by` and `within` are the Kernel's `Duration` on the Session's
    // primary root, added to `now` without rounding; another domain and a negative count
    // are refused, and the Run does not move.
    let temp = TempDir::new("duration-root");
    let (mut server, t0) = connected(&temp.0);
    let elsewhere = Duration::new(ezsdr_kernel::id::ClockDomainId::local(999), 7);
    let error = err(server.handle(Request::Advance { to: None, by: Some(elsewhere) }, Vec::new()));
    assert_eq!(error.kind, ErrorKind::NotOnPrimaryRoot);
    assert_eq!(error.message, format!("EA-12: the duration counts ticks of {}, not of the primary root {}", elsewhere.domain(), t0.domain()));
    let backwards = Duration::new(t0.domain(), -1);
    assert_eq!(err(server.handle(Request::WaitFor { kinds: vec![written()], from: 0, within: Some(backwards), until: None }, Vec::new())).kind, ErrorKind::Protocol);
    let error = err(server.handle(Request::Advance { to: None, by: Some(Duration::new(t0.domain(), i64::MAX)) }, Vec::new()));
    assert_eq!((error.kind, error.message.as_str()), (ErrorKind::NotOnPrimaryRoot, "EA-12: the duration does not fit the Run's clock"));
    let Response::Advanced { now, .. } = ok(server.handle(Request::Advance { to: None, by: Some(Duration::new(t0.domain(), 7)) }, Vec::new())) else { panic!() };
    assert_eq!(now, TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + 7));
    finish(&mut server);
}

#[test]
fn ea_13_read_serves_only_reported_artifacts() {
    let temp = TempDir::new("read");
    let (mut server, _) = connected(&temp.0);
    let (uri, _) = capture_uri(&mut server, 100);
    assert_eq!(read(&mut server, &uri).len(), 800);
    let manifest = format!("file://{}", server.dir().unwrap().join("manifest.json").display());
    for uri in [manifest, "file:///etc/hosts".to_owned(), uri.replace(".sigmf-data", ".sigmf-meta"), format!("{uri}/../x"), format!("{uri}x")] {
        let error = err(server.handle(Request::Read { uri: uri.clone() }, Vec::new()));
        assert_eq!(error.kind, ErrorKind::NotFound, "{uri}");
    }
    finish(&mut server);
}

#[test]
fn ea_14_a_child_manifest_that_cannot_be_written_is_still_returned() {
    let temp = TempDir::new("child-write");
    let (mut server, _) = connected(&temp.0);
    let dir = server.dir().unwrap().to_path_buf();
    std::fs::create_dir(dir.join("child-0.manifest.json")).unwrap();
    let Response::Ran { entry, manifest: Some(child), path: None } = ok(run_child(&mut server, receive_spec(1_000), None, Some(10_000_000))) else { panic!() };
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    let artifact = child.artifacts.values().flatten().find(|artifact| artifact.id.as_str() == "rec").unwrap();
    assert_eq!(read(&mut server, &artifact.uri).len(), 8_000, "its artifacts are still readable");
    let (parent, _) = finish(&mut server);
    assert_eq!(parent.action_log.len(), 1);
}

#[test]
fn ea_14_run_child() {
    let temp = TempDir::new("child");
    let (mut server, now) = connected(&temp.0);
    // The derived profile: the Session's without `feed`, which a Spec Run refuses (SB-22g).
    let Response::Ran { entry, manifest: Some(child), .. } = ok(run_child(&mut server, receive_spec(1_000), None, Some(10_000_000))) else { panic!() };
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    assert_eq!(child.termination.reason, Termination::Stopped { cause: StopCause::Client {} }, "{:?}", child.termination.reason);
    let artifact = child.artifacts.values().flatten().find(|artifact| artifact.id.as_str() == "rec").unwrap();
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
    elsewhere["bindings"]["radio"]["selector"]["id"] = json!("another");
    let Response::Ran { entry, manifest: None, path: None } = ok(run_child(&mut server, receive_spec(1_000), Some(elsewhere), Some(10_000_000))) else { panic!() };
    let Outcome::Rejected { violations } = entry.outcome else { panic!() };
    assert_eq!(violations[0].check, Namespace::parse("ezsdr.run_child").unwrap());

    let (manifest, _) = finish(&mut server);
    assert_eq!(manifest.run.children.len(), 2);
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
    let mut child =
        std::process::Command::new(env!("CARGO_BIN_EXE_ezsdr-server")).arg("--runs-dir").arg(&temp.0).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
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
    assert!(matches!(ask(HELLO), Reply::Result(Response::Hello { protocol: 2, .. })));
    assert!(matches!(ask("{\"request\":{\"op\":\"connect\"}}\n"), Reply::Result(Response::Connected { .. })));
    let Reply::Result(Response::Finished { path: Some(path), .. }) = ask("{\"request\":{\"op\":\"finish\"}}\n") else { panic!() };
    assert!(Path::new(&path).starts_with(temp.0.canonicalize().unwrap()));
    assert!(child.wait().unwrap().success());
}

#[test]
fn ea_05_a_request_that_does_not_decode_costs_only_itself() {
    // The header is framed before the request is decoded, so an unknown field, an unknown
    // request or a malformed Kernel document is a `protocol` error and the Session goes on
    // (Review H, P1-1); its body is consumed with it.
    let temp = TempDir::new("undecodable");
    let requests = [
        HELLO.to_owned(),
        "{\"request\":{\"op\":\"connect\"}}\n".to_owned(),
        "{\"request\":{\"op\":\"status\"},\"extra\":1}\n".to_owned(),
        "{\"request\":{\"op\":\"nope\"},\"body_bytes\":3}\nabc".to_owned(),
        "{\"request\":{\"op\":\"submit\",\"action\":{\"kind\":\"set_paramter\"}}}\n".to_owned(),
        "{\"request\":{\"op\":\"status\"}}\n".to_owned(),
        "{\"request\":{\"op\":\"finish\"}}\n".to_owned(),
    ];
    let (frames, exit) = serve_bytes(&temp.0, requests.concat().into_bytes());
    assert_eq!(exit, Exit::Replied);
    for frame in &frames[2..5] {
        let error = error_of(&frame.0);
        assert_eq!(error.kind, ErrorKind::Protocol);
        assert!(error.message.starts_with("EA-2: not a request frame"), "{}", error.message);
    }
    assert!(matches!(frames[5].0.reply, Reply::Result(Response::Status { .. })));
    let Reply::Result(Response::Finished { manifest, .. }) = &frames[6].0.reply else { panic!() };
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
}

#[test]
fn ea_15_every_exit_writes_the_manifest() {
    // Dropping a server with a live Session — an unwritable reply, a panic — finishes it
    // and writes its Manifest (Review H, P0-4).
    let temp = TempDir::new("dropped");
    let (server, _) = connected(&temp.0);
    let dir = server.dir().unwrap().to_path_buf();
    drop(server);
    let written: Manifest = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(written.termination.reason, Termination::Stopped { cause: StopCause::ClientDisconnect {} });

    // A reader that stops after a few kilobytes: a reply cannot be written, and the
    // Manifest still is.
    struct Closing(usize);
    impl Write for Closing {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0 < bytes.len() {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.0 -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let runs = TempDir::new("closed");
    let input = format!("{HELLO}{{\"request\":{{\"op\":\"connect\"}}}}\n{}", "{\"request\":{\"op\":\"status\"}}\n".repeat(50)).into_bytes();
    assert!(serve(Cursor::new(input), Closing(8_000), config(&runs.0)).is_err());
    let session = std::fs::read_dir(&runs.0).unwrap().next().unwrap().unwrap().path();
    let written: Manifest = serde_json::from_str(&std::fs::read_to_string(session.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(written.termination.reason, Termination::Stopped { cause: StopCause::ClientDisconnect {} });
    let mut greeted = Server::new(config(&runs.0));
    ok(greeted.handle(Request::Hello { protocol: 2 }, Vec::new()));
    ok(greeted.handle(Request::Connect { profile: None, lease: None, initial: None }, Vec::new()));
    let dir = greeted.dir().unwrap().to_path_buf();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _held = greeted;
        panic!("a panic while the Session is live");
    }));
    assert!(result.is_err());
    assert!(dir.join("manifest.json").exists(), "the unwinding server wrote its Manifest");
}

#[test]
fn ea_10_a_run_that_ends_on_its_way_to_t0_writes_its_manifest() {
    let temp = TempDir::new("lost-at-start");
    let mut profile = ezsdr_server::default_profile(&temp.0.join("captures").to_string_lossy());
    profile["environment"]["sim.faults"] = json!([{ "at_ns": 0, "fault": "device_lost", "target": "radio" }]);
    let mut server = greeted(&temp.0);
    let handled = server.handle(Request::Connect { profile: Some(profile), lease: None, initial: None }, Vec::new());
    assert!(handled.exit);
    let error = err(handled);
    assert_eq!(error.kind, ErrorKind::Ended);
    let written: Manifest = serde_json::from_str(&std::fs::read_to_string(server.dir().unwrap().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(Some(written.termination.reason), error.termination);
}

#[test]
fn ea_14_refusals_before_a_child_runs() {
    let temp = TempDir::new("child-refusals");
    let (mut server, _) = connected(&temp.0);
    let refuse = |server: &mut Server, spec: serde_json::Value, inputs: Vec<u64>, duration_ns: Option<u64>, body: Vec<u8>| {
        err(server.handle(Request::RunChild { spec, profile: None, inputs, duration_ns }, body))
    };
    // Sizes that overflow, or do not add up, are refused and change nothing (Review H, P0-3).
    let overflow = refuse(&mut server, receive_spec(10), vec![u64::MAX, 1], Some(1_000_000), Vec::new());
    assert_eq!((overflow.kind, overflow.message.as_str()), (ErrorKind::Protocol, "EA-14: the inputs' sizes do not add up to the body"));
    assert_eq!(refuse(&mut server, receive_spec(10), vec![3], Some(1_000_000), vec![0; 4]).kind, ErrorKind::Protocol);
    // A targeted Stop does not end a Run: without a duration it is refused (KC-33).
    let mut targeted = receive_spec(10);
    targeted["schedule"] = json!([{ "at": { "clock": "radio", "offset_ticks": 5_000 }, "action": { "kind": "stop", "target": { "kind": "resource", "resource": "radio", "path": "tx/s0" } } }]);
    assert!(refuse(&mut server, targeted, Vec::new(), None, Vec::new()).message.starts_with("EA-14: a child Run needs duration_ns"));
    // A duration that does not fit the child's clock (Review H, P2-3).
    assert_eq!(refuse(&mut server, receive_spec(10), Vec::new(), Some(u64::MAX), Vec::new()).message, "EA-14: duration_ns does not fit the child's clock");
    // Half the tick range is the bound: 5·10^18 ns on a nanosecond root is over it (Review I, P2-4).
    assert_eq!(refuse(&mut server, receive_spec(10), Vec::new(), Some(5_000_000_000_000_000_000), Vec::new()).message, "EA-14: duration_ns does not fit the child's clock");
    let (manifest, _) = finish(&mut server);
    assert!(manifest.action_log.is_empty(), "no refused request reached the log");
}

#[test]
fn ea_15_a_manifest_that_cannot_be_written_is_still_returned() {
    let temp = TempDir::new("unwritable");
    let (mut server, _) = connected(&temp.0);
    let dir = server.dir().unwrap().to_path_buf();
    let mut permissions = std::fs::metadata(&dir).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&dir, permissions.clone()).unwrap();
    let handled = server.handle(Request::Finish {}, Vec::new());
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&dir, permissions).unwrap();
    assert!(handled.exit);
    let Response::Finished { manifest, path } = ok(handled) else { panic!() };
    assert_eq!(path, None, "the write failed (Review I, P2-7)");
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
}

// ---------------------------------------------------------------- Phase 7 (VE-5, VE-6)

/// A Session profile on one USRP whose binding is also the Authority (VE-5, UR-6).
fn uhd_profile(dir: &Path) -> serde_json::Value {
    json!({
        "version": 1,
        "bindings": {
            "radio": {
                "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } },
                "profile": { "name": "x3xx", "version": { "major": 0, "minor": 1, "patch": 0 } },
                "selector": {
                    "args": "addr=192.0.2.1",
                    "rx": ["0:A:ubx:0", "0:B:ubx:0"],
                    "tx": ["0:A:ubx:0", "0:B:ubx:0"]
                }
            },
            "rec": {
                "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 2, "patch": 0 } },
                "selector": { "dir": dir.to_string_lossy() },
                "feed": { "port": { "component": "radio", "port": "rx0" }, "policy": "drop_oldest", "capacity": 64 }
            }
        },
        "authority": "radio",
        "placements": { "links": [{
            "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
            "from": { "component": "radio", "port": "rx0" },
            "to": { "component": "rec", "port": "in" }
        }] },
        "environment": {
            "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 2_000_000_000u64 },
            "ezsdr.rf_path": { "path": "cabled" },
            "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999_000_000.0, "hi_hz": 1_001_000_000.0 }], "max_gain_db": 0.0, "tx_enabled": [true] }
        }
    })
}

/// A server whose devices are `FakeDevice`s, counting the opens (VE-5; GZ-9).
fn fake_server(dir: &Path) -> (Server, Arc<AtomicU64>) {
    let opened = Arc::new(AtomicU64::new(0));
    let count = opened.clone();
    let open: ezsdr_server::OpenDevice = Arc::new(move |_args: &str| {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..FakeConfig::default() })) as Arc<dyn Device>)
    });
    let mut server = Server::new(Config { open_device: Some(open), ..config(dir) });
    ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
    (server, opened)
}

fn fake_session(dir: &Path) -> (Server, Arc<AtomicU64>) {
    fake_session_with(dir, rx0())
}

fn fake_session_with(dir: &Path, initial: Initial) -> (Server, Arc<AtomicU64>) {
    let (mut server, opened) = fake_server(dir);
    let Response::Connected { .. } = ok(server.handle(Request::Connect { profile: Some(uhd_profile(dir)), lease: None, initial }, Vec::new())) else { panic!("connect") };
    (server, opened)
}

fn status(server: &mut Server) -> (TimePoint, Rational, usize) {
    let Response::Status { now, root_rate, events, .. } = ok(server.handle(Request::Status {}, Vec::new())) else { panic!() };
    (now, root_rate, events)
}

/// `s` seconds as a `Duration` on the primary root, counted from its `root_rate` (EA-12).
fn seconds(now: TimePoint, rate: Rational, s: u64) -> Duration {
    Duration::new(now.domain(), (s * rate.num()).div_ceil(rate.den()) as i64)
}

/// Captures `n` samples at `at` and returns the written artifact (EA-17).
fn capture_at(server: &mut Server, n: i64, at: TimePoint) -> ezsdr_kernel::manifest::ArtifactRef {
    let (now, rate, events) = status(server);
    let SessionAction::Vocabulary { ns, verb, target, params, .. } = capture(n) else { unreachable!() };
    let entry = submit(server, SessionAction::Vocabulary { ns, verb, target, at: Some(at), params }, Vec::new());
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "{:?}", entry.outcome);
    let within = seconds(now, rate, 3);
    let Response::Waited { event: Some(event), .. } = ok(server.handle(Request::WaitFor { kinds: vec![written()], from: events, within: Some(within), until: None }, Vec::new())) else {
        panic!("no capture")
    };
    serde_json::from_value::<ezsdr_sink::CaptureWrittenPayload>(event.payload).unwrap().artifact
}

/// The instant on the primary root of an artifact's first sample (TM-13d).
fn first_on_root(manifest: &Manifest, artifact: &ezsdr_kernel::manifest::ArtifactRef) -> i64 {
    let map = &artifact.continuity[0];
    let clock = manifest.clocks.sample_clocks.iter().find(|record| record.domain == map.domain).expect("the capture's SampleClock");
    assert_eq!(clock.root_ticks_per_tick.den(), 1);
    clock.origin.ticks_in(clock.origin.domain()).unwrap() + map.first.ticks_in(map.first.domain()).unwrap() * clock.root_ticks_per_tick.num() as i64
}

fn complex(bytes: &[u8]) -> Vec<(f32, f32)> {
    bytes.chunks_exact(8).map(|b| (f32::from_le_bytes(b[..4].try_into().unwrap()), f32::from_le_bytes(b[4..].try_into().unwrap()))).collect()
}

#[test]
fn ea_12_a_duration_is_whole_root_ticks() {
    // EA-12 (spec 20, VF-2): on the FakeDevice's 200 MHz root a `Duration` counts root
    // ticks, not nanoseconds: 2 000 000 000 ticks are 10 s, so a wait that returns at once
    // has its horizon at least 1 000 000 000 ticks after its `now` (as nanoseconds, 400 000 000).
    let temp = TempDir::new("duration-ticks");
    let (mut server, _) = fake_server(&temp.0);
    let Response::Connected { now, root_rate, .. } = ok(server.handle(Request::Connect { profile: Some(uhd_profile(&temp.0)), lease: None, initial: rx0() }, Vec::new())) else { panic!("connect") };
    assert_eq!(root_rate, Rational::new(200_000_000, 1).unwrap());
    let (at, _, _) = status(&mut server);
    capture_at(&mut server, 100, TimePoint::new(at.domain(), at.ticks_in(at.domain()).unwrap() + 20_000_000));
    let within = Duration::new(now.domain(), 2_000_000_000);
    let Response::Waited { index: Some(_), now, horizon, .. } = ok(server.handle(Request::WaitFor { kinds: vec![written()], from: 0, within: Some(within), until: None }, Vec::new())) else {
        panic!("the capture was delivered")
    };
    assert!(horizon.ticks_in(horizon.domain()).unwrap() - now.ticks_in(now.domain()).unwrap() >= 1_000_000_000, "horizon {horizon}, now {now}");
    finish(&mut server);
}

#[test]
fn ea_07_a_uhd_profile_without_the_feature_is_refused() {
    let temp = TempDir::new("uhd-refused");
    let mut server = greeted(&temp.0);
    let error = err(server.handle(Request::Connect { profile: Some(uhd_profile(&temp.0)), lease: None, initial: rx0() }, Vec::new()));
    assert_eq!(error.kind, ErrorKind::Refused);
    if cfg!(feature = "uhd") {
        // UHD's own refusal: nothing answers at TEST-NET-1.
        assert!(error.message.starts_with("EA-7: radio: uhd_usrp_make: "), "{}", error.message);
    } else {
        assert_eq!(error.message, "EA-7: ezsdr.radio.uhd: this server was built without UHD; rebuild ezsdr-server with --features uhd");
    }
}

#[test]
fn ea_07_a_session_on_the_fake_device() {
    let temp = TempDir::new("uhd-session");
    let (mut server, opened) = fake_session_with(&temp.0, loopback());
    assert_eq!(opened.load(Ordering::SeqCst), 1);
    let wave: Vec<u8> = ramp(1_000);
    assert!(matches!(submit(&mut server, repeat(), wave.clone()).outcome, Outcome::Admitted { .. }));
    let (now, _, _) = status(&mut server);
    // The repeat starts within a restart lead (50 ms); a capture 100 ms ahead is inside it.
    let artifact = capture_at(&mut server, 3_000, TimePoint::new(now.domain(), now.ticks_in(now.domain()).unwrap() + 20_000_000));
    let captured = complex(&read(&mut server, &artifact.uri));
    let sent = complex(&wave);
    assert_eq!(captured.len(), 3_000);
    // The fake loops transmit to receive sample for sample (UR-33): the waveform, repeated.
    let offset = (0..1_000).find(|k| captured[0] == sent[*k]).expect("the capture holds the waveform");
    for (index, sample) in captured.iter().enumerate() {
        assert_eq!(*sample, sent[(offset + index) % 1_000], "captured sample {index}");
    }
    let (manifest, _) = finish(&mut server);
    assert_eq!(manifest.run.execution_class, Some(ezsdr_kernel::module_api::ExecutionClass::HardwareInLoop));
}

#[test]
fn ea_07_the_uhd_authority_takes_the_binding_s_sources() {
    // UR-6, UR-7: the server builds the Authority with the binding's sources; the time
    // source sets the device's time at the next PPS (Review L, L13).
    use ezsdr_kernel::time::EpochRef;
    let set_by = "ezsdr.radio.uhd.set_time_unknown_pps:addr=192.0.2.1".to_owned();
    for (clock_source, epoch) in [("external", EpochRef::Pps { set_by: set_by.clone() }), ("internal", EpochRef::Arbitrary { set_by: set_by.clone() })] {
        let temp = TempDir::new("uhd-sources");
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..FakeConfig::default() }));
        let held = device.clone();
        let open: ezsdr_server::OpenDevice = Arc::new(move |_args: &str| Ok(held.clone() as Arc<dyn Device>));
        let mut server = Server::new(Config { open_device: Some(open), ..config(&temp.0) });
        ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
        let mut profile = uhd_profile(&temp.0);
        profile["bindings"]["radio"]["selector"]["time_source"] = json!("external");
        profile["bindings"]["radio"]["selector"]["clock_source"] = json!(clock_source);
        // UR-6 accepts the Authority's own root, a PPS root only with a locked clock, and
        // `connected` names its epoch (EA-10).
        let Response::Connected { root_epoch, .. } = ok(server.handle(Request::Connect { profile: Some(profile), lease: None, initial: None }, Vec::new())) else { panic!() };
        assert_eq!(root_epoch, epoch, "{clock_source}");
        let calls = device.calls();
        assert!(calls.iter().any(|c| *c == format!("set_sources {clock_source} external")), "{calls:?}");
        assert!(calls.iter().any(|c| c == "set_time_zero pps"), "{calls:?}");
        finish(&mut server);
    }
}

#[test]
fn ea_07_the_device_is_released_when_the_session_ends() {
    // D-1 (ezsdr-radio-uhd's design-notes §15): once the Session has ended no thread holds
    // the device — no Provider thread and no `uhd-clock` — so the server's exit, on the
    // thread that served it, cannot race UHD's static teardown with a device call or the
    // device's free.
    let temp = TempDir::new("uhd-released");
    let made = Arc::new(std::sync::Mutex::new(std::sync::Weak::<FakeDevice>::new()));
    let keep = made.clone();
    let open: ezsdr_server::OpenDevice = Arc::new(move |_args: &str| {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..FakeConfig::default() }));
        *keep.lock().unwrap() = Arc::downgrade(&device);
        Ok(device as Arc<dyn Device>)
    });
    let mut server = Server::new(Config { open_device: Some(open), ..config(&temp.0) });
    ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
    ok(server.handle(Request::Connect { profile: Some(uhd_profile(&temp.0)), lease: None, initial: loopback() }, Vec::new()));
    assert!(matches!(submit(&mut server, repeat(), ramp(1_000)).outcome, Outcome::Admitted { .. }));
    finish(&mut server);
    assert!(made.lock().unwrap().upgrade().is_none(), "a thread still holds the device after the Session ended");
}

/// A server whose `n`th open (0-based) gives a device whose reference does not lock,
/// for each `n` in `unlocked`; counts the opens.
fn unlocking_server(dir: &Path, unlocked: &'static [u64]) -> (Server, Arc<AtomicU64>) {
    let opened = Arc::new(AtomicU64::new(0));
    let count = opened.clone();
    let open: ezsdr_server::OpenDevice = Arc::new(move |_args: &str| {
        let n = count.fetch_add(1, Ordering::SeqCst);
        let faults = if unlocked.contains(&n) { vec![ezsdr_radio_uhd::FakeFault::ReferenceDoesNotLock] } else { Vec::new() };
        Ok(Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, faults, ..FakeConfig::default() })) as Arc<dyn Device>)
    });
    let mut server = Server::new(Config { open_device: Some(open), ..config(dir) });
    ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
    (server, opened)
}

const UNLOCKED: &str = "EA-7: radio: UR-7: the reference clock did not lock to its internal source within UHD's 30 s; connecting again usually succeeds (";

#[test]
fn ea_07_a_reference_that_does_not_lock_refuses_the_connect_with_its_reason() {
    // UR-7 (ezsdr-radio-uhd's design-notes §18): opened again once, and the second open's
    // reference does not lock either: the client is told so.
    let temp = TempDir::new("uhd-unlocked");
    let (mut server, opened) = unlocking_server(&temp.0, &[0, 1]);
    let error = err(server.handle(Request::Connect { profile: Some(uhd_profile(&temp.0)), lease: None, initial: rx0() }, Vec::new()));
    assert_eq!(error.kind, ErrorKind::Refused);
    assert!(error.message.starts_with(UNLOCKED), "{}", error.message);
    assert_eq!(opened.load(Ordering::SeqCst), 2);
}

#[test]
fn ea_07_a_reference_that_locks_on_the_reopen_connects() {
    // UR-7's reopen: the first open's reference does not lock, the second's does; the
    // Session runs, and its Manifest records the reopen with the first error.
    let temp = TempDir::new("uhd-reopened");
    let (mut server, opened) = unlocking_server(&temp.0, &[0]);
    let Response::Connected { .. } = ok(server.handle(Request::Connect { profile: Some(uhd_profile(&temp.0)), lease: None, initial: rx0() }, Vec::new())) else { panic!("connect") };
    assert_eq!(opened.load(Ordering::SeqCst), 2);
    let (manifest, _) = finish(&mut server);
    let usrp = EventSource::Node { node: ResourceId::parse("usrp").unwrap() };
    let timing = manifest.section(&usrp, "ezsdr.radio.uhd.timing").expect("the UHD timing section");
    let reopened = timing.as_array().unwrap().iter().find(|r| r["what"] == "reopened_on_unlock").unwrap_or_else(|| panic!("{timing}"));
    assert!(reopened["first_error"].as_str().unwrap().starts_with("UR-7: the reference clock did not lock"), "{reopened}");
}

#[test]
fn ea_07_the_reopen_of_one_device_is_not_a_second_binding() {
    // UR-7, UR-5: the reopen is a new handle of the same physical device — the same serial
    // — taken after the first `UhdRadio` was dropped, so the one-live-instance rule does
    // not refuse it; an instance whose registration outlived it would.
    let temp = TempDir::new("uhd-reopen-serial");
    let opened = Arc::new(AtomicU64::new(0));
    let count = opened.clone();
    let open: ezsdr_server::OpenDevice = Arc::new(move |_args: &str| {
        let n = count.fetch_add(1, Ordering::SeqCst);
        let faults = if n == 0 { vec![ezsdr_radio_uhd::FakeFault::ReferenceDoesNotLock] } else { Vec::new() };
        Ok(Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, serial: "REOPEN5".to_owned(), faults, ..FakeConfig::default() })) as Arc<dyn Device>)
    });
    let mut server = Server::new(Config { open_device: Some(open), ..config(&temp.0) });
    ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
    let Response::Connected { .. } = ok(server.handle(Request::Connect { profile: Some(uhd_profile(&temp.0)), lease: None, initial: rx0() }, Vec::new())) else { panic!("connect") };
    assert_eq!(opened.load(Ordering::SeqCst), 2);
    let (manifest, _) = finish(&mut server);
    let usrp = EventSource::Node { node: ResourceId::parse("usrp").unwrap() };
    let timing = manifest.section(&usrp, "ezsdr.radio.uhd.timing").expect("the UHD timing section");
    assert!(timing.as_array().unwrap().iter().any(|r| r["what"] == "reopened_on_unlock"), "{timing}");
}

#[test]
fn ea_07_the_reopen_can_be_switched_off() {
    // UR-7's reopen is the binding's to refuse: `reopen_on_unlock: false` refuses at the
    // first open's failure.
    let temp = TempDir::new("uhd-no-reopen");
    let (mut server, opened) = unlocking_server(&temp.0, &[0]);
    let mut profile = uhd_profile(&temp.0);
    profile["bindings"]["radio"]["selector"]["reopen_on_unlock"] = json!(false);
    let error = err(server.handle(Request::Connect { profile: Some(profile), lease: None, initial: None }, Vec::new()));
    assert!(error.message.starts_with(UNLOCKED), "{}", error.message);
    assert_eq!(opened.load(Ordering::SeqCst), 1);
}

#[test]
fn ea_07_a_lost_device_ends_the_session_with_its_manifest() {
    // Review S, TG-S3 (F4): a device lost during a Session stops the Run by Policy; Finish
    // still returns the Manifest and writes it, and the device was marked lost first.
    let temp = TempDir::new("uhd-lost");
    let device = Arc::new(FakeDevice::new(FakeConfig {
        clock: ezsdr_radio_uhd::ClockMode::Manual,
        faults: vec![ezsdr_radio_uhd::FakeFault::Lost(std::time::Duration::from_millis(3_000))],
        ..FakeConfig::default()
    }));
    let held = device.clone();
    let open: ezsdr_server::OpenDevice = Arc::new(move |_args: &str| Ok(held.clone() as Arc<dyn Device>));
    let mut server = Server::new(Config { open_device: Some(open), ..config(&temp.0) });
    ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
    let Response::Connected { .. } = ok(server.handle(Request::Connect { profile: Some(uhd_profile(&temp.0)), lease: None, initial: rx0() }, Vec::new())) else { panic!("connect") };
    let lost = EventKind::parse(EventKind::DEVICE_LOST).unwrap();
    let (now, rate, _) = status(&mut server);
    let _ = server.handle(Request::WaitFor { kinds: vec![lost], from: 0, within: Some(seconds(now, rate, 5)), until: None }, Vec::new());
    let (manifest, path) = finish(&mut server);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: EventKind::parse(EventKind::DEVICE_LOST).unwrap() } });
    assert!(Path::new(&path).exists(), "{path}");
    assert!(device.calls().iter().any(|c| c == "mark_lost"), "{:?}", device.calls());
}

#[test]
fn ea_07_two_uhd_bindings_are_refused() {
    let temp = TempDir::new("uhd-two");
    let (mut server, opened) = fake_server(&temp.0);
    let mut profile = uhd_profile(&temp.0);
    profile["bindings"]["second"] = profile["bindings"]["radio"].clone();
    let error = err(server.handle(Request::Connect { profile: Some(profile), lease: None, initial: None }, Vec::new()));
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(error.message, "EA-7: one USRP per Run in this server (Phase 7)");
    assert_eq!(opened.load(Ordering::SeqCst), 0, "refused before any device opens");
}

#[test]
fn ea_09_the_configured_default_profile_is_used() {
    let temp = TempDir::new("configured-default");
    let mut profile = ezsdr_server::default_profile(&temp.0.to_string_lossy());
    profile["environment"]["sim.seed"] = json!(7);
    let path = temp.0.join("lab.json");
    std::fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
    let mut server = Server::new(Config { default_profile: Some(path), ..config(&temp.0) });
    ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
    let Response::Connected { profile: used, .. } = ok(server.handle(Request::Connect { profile: None, lease: None, initial: None }, Vec::new())) else { panic!() };
    assert_eq!(used, profile);
    finish(&mut server);

    let missing = temp.0.join("missing.json");
    let mut server = Server::new(Config { default_profile: Some(missing.clone()), ..config(&temp.0) });
    ok(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
    let error = err(server.handle(Request::Connect { profile: None, lease: None, initial: None }, Vec::new()));
    assert_eq!(error.kind, ErrorKind::Refused);
    assert!(error.message.starts_with(&format!("EA-9: {}: ", missing.display())), "{}", error.message);
}

#[test]
fn ea_09_the_binary_reads_ezsdr_profile() {
    let temp = TempDir::new("binary-profile");
    let mut profile = ezsdr_server::default_profile(&temp.0.to_string_lossy());
    profile["environment"]["sim.seed"] = json!(11);
    let path = temp.0.join("lab.json");
    std::fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ezsdr-server"))
        .arg("--runs-dir")
        .arg(&temp.0)
        .env("EZSDR_PROFILE", &path)
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
        serde_json::from_str::<ReplyFrame>(&line).unwrap().reply
    };
    assert!(matches!(ask(HELLO), Reply::Result(Response::Hello { .. })));
    let Reply::Result(Response::Connected { profile: used, .. }) = ask("{\"request\":{\"op\":\"connect\"}}\n") else { panic!() };
    assert_eq!(used, profile);
    assert!(matches!(ask("{\"request\":{\"op\":\"finish\"}}\n"), Reply::Result(Response::Finished { .. })));
    assert!(child.wait().unwrap().success());
}

#[test]
fn ea_12_status_carries_the_root_rate() {
    let temp = TempDir::new("root-rate");
    let (mut server, _) = connected(&temp.0);
    let (now, rate, _) = status(&mut server);
    assert_eq!(rate, Rational::new(1_000_000_000, 1).unwrap());
    // Past the receive stream's first sample, so that the Run has a SampleClock at another rate.
    let _ = server.handle(Request::WaitFor { kinds: vec![written()], from: 0, within: Some(Duration::new(now.domain(), 3_000_000)), until: None }, Vec::new());
    let Reply::Result(response) = server.handle(Request::Status {}, Vec::new()).reply else { panic!() };
    assert_eq!(serde_json::to_value(response).unwrap()["root_rate"], json!({ "num": 1_000_000_000u64, "den": 1 }));
    finish(&mut server);
}

#[test]
fn ea_14_a_child_of_a_device_paced_session_opens_no_device() {
    let temp = TempDir::new("uhd-child");
    let (mut server, opened) = fake_session(&temp.0);
    let Response::Ran { entry, manifest: None, path: None } = ok(run_child(&mut server, receive_spec(1_000), None, Some(10_000_000))) else { panic!() };
    let Outcome::Rejected { violations } = entry.outcome else { panic!("{:?}", entry.outcome) };
    assert!(violations[0].reason.contains("KG-12"), "{}", violations[0].reason);
    assert_eq!(opened.load(Ordering::SeqCst), 1, "only the parent's device was opened");
    finish(&mut server);
}

#[test]
fn ea_17_a_capture_ahead_starts_at_its_instant() {
    let temp = TempDir::new("uhd-ahead");
    let (mut server, _) = fake_session(&temp.0);
    let (now, rate, _) = status(&mut server);
    // 50 ms of the root, from its rate: a client's `after(0.05)` (VE-6).
    let ahead = i64::try_from((50 * u128::from(rate.num())).div_ceil(1_000 * u128::from(rate.den()))).unwrap();
    let at = TimePoint::new(now.domain(), now.ticks_in(now.domain()).unwrap() + ahead);
    let artifact = capture_at(&mut server, 1_000, at);
    let (manifest, _) = finish(&mut server);
    let first = first_on_root(&manifest, &artifact);
    // A test thread delayed between the instant and the request makes the capture start
    // later; it never starts before it, and it holds its 1 000 samples. The exact start is
    // pinned on a Spec by `uhd_59_receive_capture_starts_at_the_requested_index`.
    assert!(first >= at.ticks_in(at.domain()).unwrap(), "the first sample at {first}, asked {}", at.ticks_in(at.domain()).unwrap());
    let map = &artifact.continuity[0];
    assert_eq!(map.end.ticks_in(map.end.domain()).unwrap() - map.first.ticks_in(map.first.domain()).unwrap(), 1_000, "the map names the samples it holds");
}

#[test]
fn ea_17_a_capture_at_a_passed_instant_says_where_it_started() {
    let temp = TempDir::new("uhd-passed");
    let (mut server, _) = fake_session(&temp.0);
    let (now, rate, _) = status(&mut server);
    let back = i64::try_from((20 * u128::from(rate.num())).div_ceil(1_000 * u128::from(rate.den()))).unwrap();
    let at = TimePoint::new(now.domain(), now.ticks_in(now.domain()).unwrap() - back);
    let artifact = capture_at(&mut server, 1_000, at);
    let (manifest, _) = finish(&mut server);
    let first = first_on_root(&manifest, &artifact);
    // The samples at `at` were delivered before the request arrived: it starts later.
    assert!(first > at.ticks_in(at.domain()).unwrap(), "the first sample at {first}, asked {}", at.ticks_in(at.domain()).unwrap());
    assert_eq!(
        artifact.continuity[0].end.ticks_in(artifact.continuity[0].end.domain()).unwrap() - artifact.continuity[0].first.ticks_in(artifact.continuity[0].first.domain()).unwrap(),
        1_000,
        "the map names the samples it holds"
    );
}

#[test]
fn kc_23_an_output_target_cannot_address_a_provider() {
    let temp = TempDir::new("audit-target");
    let (mut server, _) = connected(&temp.0);
    let entry = submit(
        &mut server,
        SessionAction::SetParameter { target: Target::Output { output: Ident::parse("radio").unwrap() }, key: Key::parse("radio.rx.c0.gain_db").unwrap(), value: Value::num(3.0).unwrap() },
        vec![],
    );
    assert!(
        matches!(entry.outcome, Outcome::Rejected { ref violations }
        if violations.iter().any(|v| v.reason.starts_with("KC-23: output radio"))),
        "radio is not an output: {entry:?}"
    );
}

// ---------------------------------------------------------------- note 47 stage 3, O2: a Provider's refusal reaches the submit reply (EA-11)

/// The reply of a submit: the entry and the Provider's refusals.
fn submitted(server: &mut Server, action: SessionAction) -> (ezsdr_kernel::session::LogEntry, Vec<ezsdr_kernel::event::Event>) {
    match ok(server.handle(Request::Submit { action }, Vec::new())) {
        Response::Submitted { entry, refused, .. } => (entry, refused),
        other => panic!("{other:?}"),
    }
}

fn dispatched_of(entry: &ezsdr_kernel::session::LogEntry) -> Vec<ezsdr_kernel::event::ActionId> {
    match &entry.outcome {
        Outcome::Admitted { dispatched, .. } => dispatched.clone(),
        other => panic!("{other:?}"),
    }
}

fn start_rx(at: Option<TimePoint>) -> SessionAction {
    SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_rx").unwrap(), target: resource("radio", "rx/s0"), at, params: BTreeMap::new() }
}

fn rx_stop() -> SessionAction {
    SessionAction::Stop { target: Some(resource("radio", "rx/s0")) }
}

#[test]
fn ea_11_a_provider_refusal_is_in_the_submit_reply_on_a_stepped_run() {
    // The Kernel admits a value for a streaming stream; the Mock refuses it as the Action is booked, and the reply carries its event, matched by the Action's id.
    let temp = TempDir::new("refused-stepped");
    let (mut server, _) = connected(&temp.0);
    let (entry, refused) = submitted(&mut server, set("radio.rx.c0.frequency_hz", Value::num(1.1e9).unwrap()));
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(refused[0].kind.as_str(), ezsdr_radio::kinds::COMMAND_REJECTED);
    assert_eq!(serde_json::from_value::<ezsdr_radio::payloads::CommandRejectedPayload>(refused[0].payload.clone()).unwrap().action, Some(dispatched_of(&entry)[0]));
    // Another Action's refusal is not in this one's reply: the next submit is accepted (a Stop) and its list is empty; a second refused value has only its own event.
    let (stop, none) = submitted(&mut server, rx_stop());
    assert!(matches!(stop.outcome, Outcome::Admitted { .. }));
    assert!(none.is_empty(), "{none:?}");
    let (accepted, none) = submitted(&mut server, set("radio.rx.c0.frequency_hz", Value::num(1.2e9).unwrap()));
    assert!(matches!(accepted.outcome, Outcome::Admitted { .. }) && none.is_empty(), "a stopped stream stages the value: {none:?}");
    let (_, again) = submitted(&mut server, start_rx(None));
    assert!(again.is_empty());
    let (second, refused) = submitted(&mut server, set("radio.rx.c0.frequency_hz", Value::num(1.3e9).unwrap()));
    assert_eq!(refused.len(), 1, "the stream streams again: {refused:?}");
    assert_eq!(serde_json::from_value::<ezsdr_radio::payloads::CommandRejectedPayload>(refused[0].payload.clone()).unwrap().action, Some(dispatched_of(&second)[0]));
    finish(&mut server);
}

#[test]
fn ea_11_a_kernel_rejection_and_a_capture_have_an_empty_refused_list() {
    let temp = TempDir::new("refused-empty");
    let (mut server, _) = connected_with(&temp.0, loopback());
    let (rejected, none) = submitted(&mut server, set("radio.rx.s0.sample_rate_hz", Value::num(1.0e12).unwrap()));
    assert!(matches!(rejected.outcome, Outcome::Rejected { .. }) && none.is_empty());
    let (_, none) = submitted(&mut server, capture(100));
    assert!(none.is_empty(), "a Sink's request is not a Provider refusal: {none:?}");
    finish(&mut server);
}

#[test]
fn ea_11_a_provider_refusal_is_in_the_submit_reply_on_a_device_paced_run() {
    // The same on a device-paced Run: submit waits for uhd-control to book the Action (KC-21a), so its refusal exists when the reply is built.
    let temp = TempDir::new("refused-paced");
    let (mut server, _) = fake_session(&temp.0);
    let (entry, refused) = submitted(&mut server, set("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap()));
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(serde_json::from_value::<ezsdr_radio::payloads::CommandRejectedPayload>(refused[0].payload.clone()).unwrap().action, Some(dispatched_of(&entry)[0]));
    let (_, none) = submitted(&mut server, rx_stop());
    assert!(none.is_empty(), "{none:?}");
    finish(&mut server);
}

#[test]
fn ea_11_a_start_of_a_streaming_stream_is_refused_in_the_submit_reply_on_both_kinds_of_run() {
    // RM-21: the Run begins streaming, so a `start_rx` before a `Stop` is refused by the Provider, and the reply names it by the Action's id; after a
    // `Stop` the same start is accepted (empty list), and a second one while that start is booked is refused again.
    for name in ["stepped", "device-paced"] {
        let temp = TempDir::new(&format!("start-refused-{name}"));
        let mut server = if name == "stepped" { connected(&temp.0).0 } else { fake_session(&temp.0).0 };
        let (entry, refused) = submitted(&mut server, start_rx(None));
        assert_eq!(refused.len(), 1, "{name}: {refused:?}");
        let payload = serde_json::from_value::<ezsdr_radio::payloads::CommandRejectedPayload>(refused[0].payload.clone()).unwrap();
        assert_eq!((payload.action, payload.kind.as_str()), (Some(dispatched_of(&entry)[0]), "command"), "{name}");
        assert!(payload.reason.contains("streaming") && payload.reason.contains("Stop"), "{name}: {}", payload.reason);
        let (_, none) = submitted(&mut server, rx_stop());
        assert!(none.is_empty(), "{name}: {none:?}");
        let (_, none) = submitted(&mut server, start_rx(None));
        assert!(none.is_empty(), "{name}: {none:?}");
        let (second, refused) = submitted(&mut server, start_rx(None));
        assert_eq!(refused.len(), 1, "{name}: {refused:?}");
        assert_eq!(serde_json::from_value::<ezsdr_radio::payloads::CommandRejectedPayload>(refused[0].payload.clone()).unwrap().action, Some(dispatched_of(&second)[0]), "{name}");
        finish(&mut server);
    }
}

#[test]
fn ea_11_a_start_the_line_cannot_represent_is_refused_in_the_submit_reply_with_its_id() {
    // RM-25: an `at` whose lattice instant overflows is refused at the Provider's booking; the refusal carries the Action's id, so the reply lists it
    // (without the id the Python call would return as a success).
    let temp = TempDir::new("start-overflow");
    let (mut server, _) = fake_session(&temp.0);
    let (now, _, _) = status(&mut server);
    let (_, none) = submitted(&mut server, rx_stop());
    assert!(none.is_empty(), "{none:?}");
    let (entry, refused) = submitted(&mut server, start_rx(Some(TimePoint::new(now.domain(), i64::MAX))));
    assert_eq!(refused.len(), 1, "{refused:?}");
    let payload = serde_json::from_value::<ezsdr_radio::payloads::CommandRejectedPayload>(refused[0].payload.clone()).unwrap();
    assert_eq!((payload.action, payload.kind.as_str()), (Some(dispatched_of(&entry)[0]), "command"));
    assert!(payload.reason.starts_with("RM-25:"), "{}", payload.reason);
    finish(&mut server);

    // The same on a stepped Run (MockRadio): a refusal with the id, not a failed Run; the Run continues.
    let temp = TempDir::new("start-overflow-stepped");
    let (mut server, now) = connected(&temp.0);
    let (_, none) = submitted(&mut server, rx_stop());
    assert!(none.is_empty(), "{none:?}");
    let (entry, refused) = submitted(&mut server, start_rx(Some(TimePoint::new(now.domain(), i64::MAX))));
    assert_eq!(refused.len(), 1, "stepped: {refused:?}");
    let payload = serde_json::from_value::<ezsdr_radio::payloads::CommandRejectedPayload>(refused[0].payload.clone()).unwrap();
    assert_eq!((payload.action, payload.kind.as_str()), (Some(dispatched_of(&entry)[0]), "command"), "stepped");
    assert!(payload.reason.starts_with("RM-25:"), "stepped: {}", payload.reason);
    let (_, none) = submitted(&mut server, start_rx(None));
    assert!(none.is_empty(), "the Run continues and takes a later start: {none:?}");
    finish(&mut server);
}
