//! EA-15's watch on the client's input (#89): its signals, their removal and the order against
//! a fire in progress (N1), through the `test-support` seams.

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::run::{StopCause, Termination};
use ezsdr_server::protocol::{ErrorKind, Reply, Request, Response};
use ezsdr_server::test_support::{Watch, unwatched, watched_server};
use ezsdr_server::{Config, Handled, Server};
use support::{BOUND, eventually};

fn counter() -> (Arc<AtomicUsize>, impl Fn() + Send + Sync + 'static) {
    let count = Arc::new(AtomicUsize::new(0));
    let held = count.clone();
    (count, move || {
        held.fetch_add(1, Ordering::SeqCst);
    })
}

/// A signal that says it was entered, then waits until the returned sender drops.
fn gated() -> (mpsc::Receiver<()>, mpsc::Sender<()>, impl Fn() + Send + Sync + 'static) {
    let (entered, entered_rx) = mpsc::channel();
    let (gate, gate_rx) = mpsc::channel::<()>();
    let gate_rx = Mutex::new(gate_rx);
    (entered_rx, gate, move || {
        entered.send(()).unwrap();
        let _ = gate_rx.lock().unwrap().recv();
    })
}

/// Fires `watch` on a thread named `input` once the signal `entered_rx` stands for is running.
fn fire_held(watch: &Arc<Watch>, entered_rx: &mpsc::Receiver<()>) -> std::thread::JoinHandle<()> {
    let firing = watch.clone();
    let input = std::thread::Builder::new().name("input".into()).spawn(move || firing.fire()).unwrap();
    entered_rx.recv_timeout(BOUND).expect("the fire is in progress");
    input
}

/// Opens `gate` once a removal has found the watch's lock held by the fire; whether it had.
fn open_when_contended(watch: &Arc<Watch>, gate: mpsc::Sender<()>) -> std::thread::JoinHandle<bool> {
    let probe = watch.clone();
    std::thread::spawn(move || {
        let contended = eventually(|| (probe.contended() >= 1).then_some(())).is_some();
        drop(gate);
        contended
    })
}

#[test]
fn ea_15_the_watch_calls_every_signal_it_holds_once_and_each_added_after_it_fired() {
    let watch = Arc::new(Watch::default());
    let ((a, signal_a), (b, signal_b), (c, signal_c)) = (counter(), counter(), counter());
    let _a = watch.add(signal_a);
    let held_b = watch.add(signal_b);
    unwatched(Some(held_b), ());
    watch.fire();
    watch.fire();
    let _c = watch.add(signal_c);
    assert_eq!((a.load(Ordering::SeqCst), b.load(Ordering::SeqCst), c.load(Ordering::SeqCst)), (1, 0, 1));
    assert!(watch.fired());
}

#[test]
fn ea_15_a_signal_is_removed_when_its_holder_is_dropped() {
    // A panic between `add` and the removal (in `connect` or `drive`) leaves no signal behind.
    let watch = Arc::new(Watch::default());
    let (count, signal) = counter();
    let held = watch.clone();
    assert!(
        std::panic::catch_unwind(move || {
            let _watched = held.add(signal);
            panic!("a panic while the signal is held");
        })
        .is_err()
    );
    assert_eq!(watch.len(), 0);
    watch.fire();
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn ea_15_a_removal_waits_for_a_fire_in_progress_so_the_owner_drops_last() {
    // The owner's last reference stands for the Run a signal holds while it runs (N1).
    struct Token(Arc<Mutex<Option<String>>>);
    impl Drop for Token {
        fn drop(&mut self) {
            *self.0.lock().unwrap() = std::thread::current().name().map(str::to_owned);
        }
    }
    let dropped_on = Arc::new(Mutex::new(None));
    let token = Arc::new(Token(dropped_on.clone()));
    let (entered_rx, gate, wait) = gated();
    let held = token.clone();
    let watch = Arc::new(Watch::default());
    let watched = watch.add(move || {
        let _held = held.clone();
        wait();
    });
    let input = fire_held(&watch, &entered_rx);
    let owner = std::thread::Builder::new()
        .name("owner".into())
        .spawn(move || {
            let token = unwatched(Some(watched), token);
            drop(token);
        })
        .unwrap();
    let opener = open_when_contended(&watch, gate);
    assert!(opener.join().unwrap(), "the owner's removal does not wait for the fire in progress");
    input.join().unwrap();
    owner.join().unwrap();
    // The removal dropped the signal, and its clone of the token, on the owner's thread before
    // the owner dropped its own.
    assert_eq!(dropped_on.lock().unwrap().as_deref(), Some("owner"));
}

/// A greeted server over `watch`, with its own runs directory under the temporary directory.
fn server(name: &str, watch: &Arc<Watch>) -> (Server, PathBuf) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let runs = std::env::temp_dir().join(format!("ezsdr-server-watch-{}-{name}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let mut server = watched_server(Config::new(runs.clone()), watch.clone());
    result(server.handle(request(serde_json::json!({ "op": "hello", "protocol": 2 })), Vec::new()));
    (server, runs)
}

fn request(value: serde_json::Value) -> Request {
    serde_json::from_value(value).unwrap()
}

/// `connect` to the default profile, receiving on c0 (EA-10a).
fn connect() -> Request {
    request(serde_json::json!({ "op": "connect", "initial": { "radio": { "rx": [{ "number": 0, "channels": [0] }] } } }))
}

fn finish() -> Request {
    request(serde_json::json!({ "op": "finish" }))
}

fn result(handled: Handled) -> Response {
    match handled.reply {
        Reply::Result(response) => response,
        Reply::Error(error) => panic!("expected a result, got {error:?}"),
    }
}

fn client_disconnect() -> Termination {
    Termination::Stopped { cause: StopCause::ClientDisconnect {} }
}

/// A child Spec receiving on c0 at `rate`, capturing 1 000 samples for 10 ms.
fn child(rate: f64) -> Request {
    request(serde_json::json!({ "op": "run_child", "duration_ns": 10_000_000u64, "spec": {
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 2 }, { "id": "sink", "major": 1 }] },
        "resources": { "radio": { "kind": "radio.device", "requires": {
            "radio.rx.c0.stream": { "kind": "eq", "value": 0 },
            "radio.rx.s0.sample_rate_hz": { "kind": "eq", "value": rate }
        } } },
        "outputs": [{
            "id": "rec", "kind": "sink.capture", "params": { "sink.capture_samples": 1_000 },
            "feed": { "port": { "component": "radio", "port": "rx0" }, "policy": "drop_oldest", "capacity": 64 }
        }]
    } }))
}

#[test]
fn ea_15_every_end_of_a_run_removes_its_signal_from_the_watch() {
    let watch = Arc::new(Watch::default());
    let (mut server, runs) = server("unwatched", &watch);
    result(server.handle(connect(), Vec::new()));
    assert_eq!(watch.len(), 1, "connect adds the Session's signal");
    let Response::Ran { manifest: Some(ran), .. } = result(server.handle(child(1.0e6), Vec::new())) else { panic!("a child ran") };
    assert!(matches!(ran.termination.reason, Termination::Stopped { .. }), "{:?}", ran.termination.reason);
    assert_eq!(watch.len(), 1, "the end of drive removes the child's signal");
    // A child that fails before T0 has no start instant: drive returns before it adds a signal.
    let Response::Ran { manifest: Some(failed), .. } = result(server.handle(child(1.0e12), Vec::new())) else { panic!("a child ran") };
    assert!(matches!(failed.termination.reason, Termination::Failed { .. }), "{:?}", failed.termination.reason);
    assert_eq!(watch.len(), 1, "a child that never started leaves no signal");
    result(server.handle(finish(), Vec::new()));
    assert_eq!(watch.len(), 0, "finish removes the Session's signal");

    let (mut server, second) = self::server("unwatched-disconnect", &watch);
    result(server.handle(connect(), Vec::new()));
    assert_eq!(watch.len(), 1);
    server.disconnect();
    assert_eq!(watch.len(), 0, "disconnect removes the Session's signal");
    for dir in [runs, second] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn ea_15_a_connect_after_the_input_ended_ends_on_its_way_to_t0_and_leaves_no_signal() {
    let watch = Arc::new(Watch::default());
    watch.fire();
    let (mut server, runs) = server("ended-connect", &watch);
    let handled = server.handle(connect(), Vec::new());
    assert!(handled.exit);
    let Reply::Error(error) = handled.reply else { panic!("the Run ended on its way to T0, got {:?}", handled.reply) };
    assert_eq!((error.kind, error.termination), (ErrorKind::Ended, Some(client_disconnect())));
    let written: Manifest = serde_json::from_str(&std::fs::read_to_string(server.dir().unwrap().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(written.termination.reason, client_disconnect());
    assert_eq!(watch.len(), 0, "the ended path removes the Session's signal");
    let _ = std::fs::remove_dir_all(runs);
}

#[test]
fn ea_15_finish_removes_the_session_s_signal_before_it_finishes_the_run() {
    // A fire held in a signal added before the Session's: `finish`'s removal waits for it, so the
    // Session's signal is called while the Run is still live only if the removal comes first.
    let watch = Arc::new(Watch::default());
    let (entered_rx, gate, wait) = gated();
    let _held = watch.add(wait);
    let (mut server, runs) = server("finish-order", &watch);
    result(server.handle(connect(), Vec::new()));
    let input = fire_held(&watch, &entered_rx);
    let opener = open_when_contended(&watch, gate);
    let Response::Finished { manifest, .. } = result(server.handle(finish(), Vec::new())) else { panic!("finished") };
    assert!(opener.join().unwrap(), "finish's removal did not wait for the fire in progress");
    input.join().unwrap();
    assert_eq!(manifest.termination.reason, client_disconnect());
    let _ = std::fs::remove_dir_all(runs);
}
