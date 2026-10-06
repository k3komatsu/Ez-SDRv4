//! Phase 5's carriers (plan/phase5/00-overview.md §8): the Mini Reactive Radio, a PING
//! answered by a Reactor with a timed PONG, run through the coordinator with the real
//! Modules — two MockRadios on a SimulationChannel with a coupling each way, the responder
//! on an Island of the native Executor, and the recorder on the pinging radio.
//!
//! The timing every test relies on (00-overview.md §7): at 1 Msps on `x310-like` a PING at
//! transmit sample 10 000 is radiated 45 samples late (MR-3) and crosses a 1 µs path, so its
//! first sample reaches the responder at receive sample 10 046; a 5 ms turnaround targets
//! 15 046, which reaches the pinger at 15 092. The block holding 10 046 is published at
//! 12 000 µs (MR-14), and with MR-3's 2 ms lead a target before 14 000 is late.

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use ezsdr_acceptance::{experiments, responder, rig};
use ezsdr_exec_native::{Component, ComponentContext, Implementation, NativeExecutor};
use ezsdr_kernel::event::Action;
use ezsdr_kernel::manifest::{ArtifactRef, Manifest};
use ezsdr_kernel::module_api::{ModuleError, StepOutcome, StopMode};
use ezsdr_kernel::run::{StopCause, Termination};
use ezsdr_kernel::spec::Ident;
use ezsdr_kernel::stream::BurstRecord;
use ezsdr_kernel::time::TimePoint;
use serde_json::json;
use support::{artifact, finish_at, root, section, T0};

/// The transmit clock starts at arm and the receive clock at T0, 2 s later (MR-9, MR-11),
/// so transmit tick `2 000 000 + k` is receive sample `k` at 1 Msps.
const TX_AT_T0: i64 = 2_000_000;
const TURNAROUND_NS: i64 = 5_000_000;
/// With block-length jitter a block holds up to 4 000 samples (MR-12), so the responder may
/// learn of a PING up to 4 ms after it began, and a 5 ms turnaround can then be late (the
/// radio's verdict, as on hardware; the decision itself does not move). The carriers that
/// jitter use 7 ms, which no block length makes late: 4 ms + MR-3's 2 ms lead < 7 ms.
const JITTER_TURNAROUND_NS: i64 = 7_000_000;

fn constant(len: usize, value: (f32, f32)) -> Vec<(f32, f32)> {
    vec![value; len]
}

/// The PING: 1 000 samples of amplitude 0.5. The PONG: 500 samples of `0.5j`.
fn waveforms() -> ((Vec<u8>, ArtifactRef), (Vec<u8>, ArtifactRef)) {
    let ping = experiments::waveform_of(&constant(1_000, (0.5, 0.0)));
    let (pong_bytes, mut pong) = experiments::waveform_of(&constant(500, (0.0, 0.5)));
    pong.id = Ident::parse("pong").unwrap();
    (ping, (pong_bytes, pong))
}

/// Both couplings at −6 dB over 1 µs, with the same noise at both receivers when asked.
fn couplings(a: &str, b: &str, noise_dbfs: Option<f64>, seed: u64) -> serde_json::Value {
    let mut channel = json!({ "couplings": [
        { "tx": a, "tx_channel": 0, "rx": b, "rx_channel": 0, "gain_db": -6.0, "delay_ns": 1_000 },
        { "tx": b, "tx_channel": 0, "rx": a, "rx_channel": 0, "gain_db": -6.0, "delay_ns": 1_000 }
    ] });
    if let Some(noise) = noise_dbfs {
        channel["noise_dbfs"] = json!({ a: noise, b: noise });
    }
    json!({ "sim.seed": seed, "sim.channel": channel })
}

struct Ping<'a> {
    pinger: &'a str,
    responder: &'a str,
    pings: &'a [i64],
    turnaround_ns: i64,
    late_policy: &'a str,
    jitter: bool,
    noise_dbfs: Option<f64>,
    seed: u64,
    /// Where the Run is stopped: `None` runs to T0 + 25 ms as the other carriers do.
    stop_at: Option<i64>,
    /// The responder's implementation, when a test wraps it.
    implementation: Option<Implementation>,
}

impl Default for Ping<'_> {
    fn default() -> Self {
        Ping {
            pinger: "a",
            responder: "b",
            pings: &[10_000],
            turnaround_ns: TURNAROUND_NS,
            late_policy: "drop_and_flag",
            jitter: false,
            noise_dbfs: None,
            seed: 0,
            stop_at: None,
            implementation: None,
        }
    }
}

/// Runs the Mini Reactive Radio. Runs compared through `determinism_projection` must
/// share `temp`, because the capture directory is in the recorded BindingProfile.
fn ping_pong(temp: &rig::TempDir, ping: Ping<'_>) -> (Manifest, Vec<(f32, f32)>) {
    let ((ping_bytes, ping_ref), (pong_bytes, pong_ref)) = waveforms();
    let hash = responder::impl_hash();
    let mut spec = experiments::ping_pong(
        ping.pinger, ping.responder, 1.0e6, &ping_ref, &pong_ref, ping.pings[0], ping.turnaround_ns, ping.late_policy, 20_000,
        (responder::IMPL_ID, &hash),
    );
    for offset in &ping.pings[1..] {
        let mut entry = spec["schedule"][0].clone();
        entry["at"]["offset_ticks"] = json!(offset);
        spec["schedule"].as_array_mut().unwrap().push(entry);
    }
    let environment = couplings(ping.pinger, ping.responder, ping.noise_dbfs, ping.seed);
    let profile = rig::ping_pong_profile("x310-like", ping.pinger, ping.responder, ping.jitter, &temp.0, environment);
    let inputs = BTreeMap::from([(ping_ref.hash.clone(), ping_bytes), (pong_ref.hash.clone(), pong_bytes)]);
    let mut assembly = rig::assemble(&profile, inputs);
    if let Some(implementation) = ping.implementation {
        assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(NativeExecutor::new(vec![implementation]).unwrap()));
    }
    let mut run = ezsdr_kernel::coordinator::start_spec_run(&spec, &profile, assembly).expect("Spec Run starts");
    let clock = root(&run);
    let manifest = match ping.stop_at {
        Some(tick) => {
            run.advance_to(TimePoint::new(clock, tick)).expect("the Run is live");
            run.finish()
        }
        None => finish_at(run, clock, T0 + 25_000_000),
    };
    let capture = rig::read_capture(artifact(&manifest, "rec"), 1).remove(0);
    (manifest, capture)
}

fn bursts(manifest: &Manifest, device: &str) -> Vec<BurstRecord> {
    serde_json::from_value(section(manifest, &format!("ezsdr.radio.mock.{device}.bursts")).clone()).unwrap()
}

fn time_errors(manifest: &Manifest) -> Vec<serde_json::Value> {
    manifest.events.delivered.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).map(|e| e.payload.clone()).collect()
}

fn heard(capture: &[(f32, f32)]) -> Option<usize> {
    capture.iter().position(|sample| *sample != (0.0, 0.0))
}

/// A burst record without its clock domains, whose local ids depend on allocation order.
fn decision(record: &BurstRecord) -> (i64, Option<i64>, Option<i64>, u64, u32) {
    (record.target.ticks, record.requested_target.map(|t| t.ticks), record.late_by.map(|d| d.ticks), record.samples, record.blocks)
}

fn clean_stop(manifest: &Manifest) {
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert!(manifest.termination.also.is_empty(), "{:?}", manifest.termination.also);
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn v58_09_a_reactor_answers_a_ping_with_a_timed_pong() {
    let (manifest, capture) = ping_pong(&rig::TempDir::new("v58-09-pong"), Ping::default());
    clean_stop(&manifest);
    assert!(!serde_json::to_string(&manifest.spec.body).unwrap().contains("sim."), "the Spec carries no channel");

    // The decision, as the responder radio recorded it (SC-28): one burst at receive sample
    // 15 046 = 10 046 + 5 000, on time, not moved.
    let answered = bursts(&manifest, "dev_b");
    assert_eq!(answered.len(), 1, "{answered:?}");
    assert_eq!(answered[0].target.ticks, TX_AT_T0 + 15_046);
    assert_eq!(answered[0].requested_target, None);
    assert_eq!(answered[0].late_by, None);
    assert_eq!(answered[0].samples, 500);
    assert!(time_errors(&manifest).is_empty());

    // The pinger hears the PONG from sample 15 092 for 500 samples, at −6 dB; x310-like
    // rotates it by the two radios' LO phases (MR-34), so its magnitude is what is exact.
    assert_eq!(heard(&capture), Some(15_092));
    let expected = 0.5 * 10f64.powf(-6.0 / 20.0);
    for (k, (re, im)) in capture.iter().enumerate() {
        let magnitude = (f64::from(*re).powi(2) + f64::from(*im).powi(2)).sqrt();
        if (15_092..15_592).contains(&k) {
            assert!((magnitude - expected).abs() < 1e-6, "sample {k}: {magnitude}");
        } else {
            assert_eq!((*re, *im), (0.0, 0.0), "sample {k}");
        }
    }

    // Provenance: the listed input first, then the scheduled one (KC-9), and the
    // component's implementation hash (KC-45).
    let ((_, ping), (_, pong)) = waveforms();
    assert_eq!(manifest.inputs, vec![pong, ping]);
    assert_eq!(manifest.components.get(&Ident::parse("responder").unwrap()), Some(&responder::impl_hash()));
}

#[test]
fn v58_09_every_ping_gets_one_pong() {
    // Two PINGs 2.5 ms apart: two decisions, two closed bursts (SC-24), heard in turn.
    let (manifest, capture) = ping_pong(&rig::TempDir::new("v58-09-two"), Ping { pings: &[10_000, 12_500], ..Ping::default() });
    clean_stop(&manifest);
    let answered = bursts(&manifest, "dev_b");
    assert_eq!(answered.iter().map(|b| b.target.ticks - TX_AT_T0).collect::<Vec<_>>(), vec![15_046, 17_546]);
    assert!(answered.iter().all(|b| b.end == ezsdr_kernel::stream::BurstEnd::Eob && b.samples == 500), "{answered:?}");
    let loud: Vec<usize> = capture.iter().enumerate().filter(|(_, s)| **s != (0.0, 0.0)).map(|(k, _)| k).collect();
    assert_eq!(loud.len(), 1_000);
    assert_eq!((loud[0], loud[499], loud[500], loud[999]), (15_092, 15_591, 17_592, 18_091));
}

#[test]
fn v58_09_a_pong_short_of_the_lead_is_late_as_on_hardware() {
    // §22: the radio decides a runtime burst's lead against the same envelope as hardware,
    // counted from when it delivered the samples the Reactor reacts to (MR-14, MR-17).
    let late = |policy: &'static str, turnaround_ns: i64, test: &str| {
        ping_pong(&rig::TempDir::new(test), Ping { late_policy: policy, turnaround_ns, ..Ping::default() })
    };

    // A 1 ms turnaround targets 11 046, before the device's 14 000: dropped and flagged.
    let (dropped, capture) = late("drop_and_flag", 1_000_000, "v58-09-drop");
    clean_stop(&dropped);
    assert!(bursts(&dropped, "dev_b").is_empty());
    let errors = time_errors(&dropped);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!((errors[0]["cause"].as_str(), errors[0]["outcome"].as_str()), (Some("late"), Some("drop")));
    assert_eq!(errors[0]["late_by_ns"], json!(2_954_000));
    assert_eq!(errors[0]["target"]["ticks"], json!(TX_AT_T0 + 11_046));
    assert_eq!(section(&dropped, "ezsdr.radio.mock.dev_b.rejected")[0]["reason"], json!("MR-17: the late policy dropped the burst"));
    assert_eq!(heard(&capture), None);

    // The same under send_asap_and_flag: sent at 14 000, both targets recorded (SC-28).
    let (moved, capture) = late("send_asap_and_flag", 1_000_000, "v58-09-asap");
    let sent = bursts(&moved, "dev_b");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].target.ticks, TX_AT_T0 + 14_000);
    assert_eq!(sent[0].requested_target.map(|t| t.ticks), Some(TX_AT_T0 + 11_046));
    assert_eq!(sent[0].late_by.map(|d| d.ticks), Some(2_954_000));
    assert_eq!(time_errors(&moved)[0]["outcome"], json!("send_asap"));
    assert_eq!(heard(&capture), Some(14_046));

    // The boundary is where the arithmetic puts it: a target at 14 000 is on time, one
    // sample earlier is late.
    let (on_time, _) = late("drop_and_flag", 3_954_000, "v58-09-edge-on");
    assert_eq!(bursts(&on_time, "dev_b").iter().map(decision).collect::<Vec<_>>(), vec![(TX_AT_T0 + 14_000, None, None, 500, 1)]);
    let (one_early, _) = late("drop_and_flag", 3_953_000, "v58-09-edge-late");
    assert!(bursts(&one_early, "dev_b").is_empty());
    assert_eq!(time_errors(&one_early)[0]["late_by_ns"], json!(1_000));
    // A turnaround between two samples is rounded up to the next one, so 3 953.5 µs is
    // on time where 3 953 µs is not (Review F, P2-8).
    let (rounded, _) = late("drop_and_flag", 3_953_500, "v58-09-edge-rounded");
    assert_eq!(bursts(&rounded, "dev_b").iter().map(decision).collect::<Vec<_>>(), vec![(TX_AT_T0 + 14_000, None, None, 500, 1)]);
}

#[test]
fn v58_03_reactor_decisions_reproduce_with_their_seed() {
    let temp = rig::TempDir::new("v58-03-reactor");
    let noisy = |seed: u64, pinger: &'static str, responder: &'static str| {
        ping_pong(&temp, Ping { pinger, responder, noise_dbfs: Some(-30.0), seed, jitter: true, turnaround_ns: JITTER_TURNAROUND_NS, ..Ping::default() })
    };
    let (first, first_capture) = noisy(7, "a", "b");
    let (second, _) = noisy(7, "a", "b");
    let (other, other_capture) = noisy(8, "a", "b");
    // One seed, one Manifest projection: events, decisions and capture alike.
    assert_eq!(rig::determinism_projection(&first), rig::determinism_projection(&second));
    // Another seed is other noise and other block lengths, and the same decision.
    assert_ne!(first_capture, other_capture);
    let decisions = |manifest: &Manifest, device: &str| bursts(manifest, device).iter().map(decision).collect::<Vec<_>>();
    assert_eq!(decisions(&first, "dev_b"), vec![(TX_AT_T0 + 17_046, None, None, 500, 1)]);
    assert_eq!(decisions(&other, "dev_b"), decisions(&first, "dev_b"));
    // The radios' names do not decide it either: the responder sorts first here (MA-30).
    let (swapped, swapped_capture) = noisy(7, "z", "a");
    assert_eq!(decisions(&swapped, "dev_a"), decisions(&first, "dev_b"));
    // What the pinger hears first is the PONG, not noise above the capture's zero: with
    // noise every sample is nonzero, so compare the loud stretch instead.
    let loud = |capture: &[(f32, f32)]| capture.iter().position(|(re, im)| f64::from(*re).powi(2) + f64::from(*im).powi(2) > 0.02);
    assert_eq!(loud(&first_capture), Some(17_092));
    assert_eq!(loud(&swapped_capture), Some(17_092));
}

#[test]
fn v58_12_a_reactor_answers_whatever_the_block_lengths() {
    let (plain, plain_capture) = ping_pong(&rig::TempDir::new("v58-12-reactor-plain"), Ping { turnaround_ns: JITTER_TURNAROUND_NS, ..Ping::default() });
    let (jittered, jittered_capture) = ping_pong(&rig::TempDir::new("v58-12-reactor-jitter"), Ping { jitter: true, turnaround_ns: JITTER_TURNAROUND_NS, ..Ping::default() });
    // §58 #12 for a component: the decision and what the pinger hears do not depend on
    // the blocks the responder was handed.
    assert_eq!(heard(&plain_capture), Some(17_092));
    assert_eq!(bursts(&plain, "dev_b").iter().map(decision).collect::<Vec<_>>(), bursts(&jittered, "dev_b").iter().map(decision).collect::<Vec<_>>());
    assert_eq!(plain_capture, jittered_capture);
    // … and the blocks really differed, or the equality proves nothing.
    let blocks = |manifest: &Manifest| section(manifest, "ezsdr.radio.mock.dev_b.stats")["rx_blocks"].clone();
    assert_ne!(blocks(&plain), blocks(&jittered));

    // A PING that straddles a block boundary is one PING, whatever seed: at 11 500 its
    // first sample reaches the responder at 11 546 and its last at 12 545, across the
    // boundary at 12 000, and it is answered once (Review F, P1-3).
    let (straddling, _) = ping_pong(&rig::TempDir::new("v58-12-reactor-straddle"), Ping { pings: &[11_500], turnaround_ns: JITTER_TURNAROUND_NS, ..Ping::default() });
    assert_eq!(bursts(&straddling, "dev_b").iter().map(decision).collect::<Vec<_>>(), vec![(TX_AT_T0 + 18_546, None, None, 500, 1)]);
}

/// How many Actions the wrapped responder decided, for `ke_03`, the only test that wraps it
/// (a cleanup step runs on a thread of its own, RS-8a, so a thread-local would miss it).
static DECIDED: AtomicUsize = AtomicUsize::new(0);

struct Counting(Box<dyn Component>);

impl Component for Counting {
    fn prepare(&mut self, ctx: ComponentContext) -> Result<(), ModuleError> {
        self.0.prepare(ctx)
    }
    fn step(&mut self, until: TimePoint, out: &mut Vec<Action>) -> Result<StepOutcome, ModuleError> {
        let before = out.len();
        let outcome = self.0.step(until, out);
        DECIDED.fetch_add(out.len() - before, Ordering::SeqCst);
        outcome
    }
    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> {
        self.0.stop(mode)
    }
    fn cleanup(&mut self) {
        self.0.cleanup()
    }
}

#[test]
fn ke_03_a_decision_after_the_stop_is_not_an_abort() {
    // The PING starts at 25 960 µs and the Run is stopped at 25 990 µs: the pinger radiates
    // 30 samples, which reach the responder from receive sample 26 006, in a block it is
    // handed only in the orderly drain (KA-12). Dispatch is frozen by then, admit() refuses
    // the PONG with ezsdr.dispatch, and that refusal must not become an abort (KE-3, NX-6).
    let counting = Implementation {
        id: responder::IMPL_ID.to_owned(),
        hash: responder::impl_hash(),
        make: || Box::new(Counting((responder::implementation().make)())),
    };
    let (manifest, _) = ping_pong(
        &rig::TempDir::new("ke-03-drain"),
        Ping { pings: &[25_960], stop_at: Some(T0 + 25_990_000), implementation: Some(counting), ..Ping::default() },
    );
    // The responder did decide — in the drain, the one place it heard the PING — and the
    // refusal left the stop clean (Review F, P2-6: without the count the test would pass
    // with no decision at all).
    assert_eq!(DECIDED.load(Ordering::SeqCst), 1);
    clean_stop(&manifest);
    let pinged = bursts(&manifest, "dev_a");
    assert_eq!((pinged.len(), pinged[0].samples), (1, 30), "{pinged:?}");
    assert!(section(&manifest, "ezsdr.radio.mock.dev_b.stats")["rx_samples"].as_u64().unwrap() > 26_006, "the PING's first sample was delivered");
    assert!(bursts(&manifest, "dev_b").is_empty());
}

#[test]
fn v58_09_a_reactor_runs_in_a_child_run_of_a_session() {
    // Phase 5 deferred "a Reactor in a Session" to Phase 6's child Runs (KF-3): a Session
    // binds both radios, and the ping-pong Spec runs as its child, with the responder on
    // the native Executor, through the server (spec 16 EA-14).
    use ezsdr_server::protocol::{Reply, Request, Response};
    let temp = rig::TempDir::new("v58-09-child");
    let ((ping_bytes, ping_ref), (pong_bytes, pong_ref)) = waveforms();
    let hash = responder::impl_hash();
    let spec = experiments::ping_pong("a", "b", 1.0e6, &ping_ref, &pong_ref, 10_000, TURNAROUND_NS, "drop_and_flag", 20_000, (responder::IMPL_ID, &hash));
    let child_profile = rig::ping_pong_profile("x310-like", "a", "b", false, &temp.0, couplings("a", "b", None, 0));
    // The Session's own profile: the same radios and Authority, the recorder fed from `a`,
    // and no Executor, which only the child places.
    let mut session_profile = child_profile.clone();
    session_profile["bindings"].as_object_mut().unwrap().remove("exec");
    session_profile["bindings"]["rec"]["feed"] = json!({ "port": { "component": "a", "port": "rx" }, "policy": "drop_oldest", "capacity": 64 });
    session_profile["placements"] = json!({ "links": [child_profile["placements"]["links"][0].clone()] });

    let mut server = ezsdr_server::Server::new(ezsdr_server::Config { implementations: vec![responder::implementation()], ..ezsdr_server::Config::new(temp.0.join("runs")) });
    let result = |handled: ezsdr_server::Handled| match handled.reply {
        Reply::Result(response) => response,
        Reply::Error(error) => panic!("{error:?}"),
    };
    result(server.handle(Request::Hello { protocol: 2 }, Vec::new()));
    let Response::Connected { run: session, .. } = result(server.handle(Request::Connect { profile: Some(session_profile), lease: None }, Vec::new())) else { panic!() };
    let inputs = vec![ping_bytes.len() as u64, pong_bytes.len() as u64];
    let request = Request::RunChild { spec, profile: Some(child_profile), inputs, duration_ns: Some(25_000_000) };
    let Response::Ran { entry, manifest: Some(child), .. } = result(server.handle(request, [ping_bytes, pong_bytes].concat())) else { panic!("the child was refused") };
    assert!(matches!(entry.outcome, ezsdr_kernel::session::Outcome::Admitted { .. }));
    assert_eq!(child.run.parent, Some(session));
    clean_stop(&child);
    // The same decision as `v58_09_a_reactor_answers_a_ping_with_a_timed_pong`'s.
    let answered = bursts(&child, "dev_b");
    assert_eq!(answered.iter().map(decision).collect::<Vec<_>>(), vec![(TX_AT_T0 + 15_046, None, None, 500, 1)]);
    let capture = rig::read_capture(artifact(&child, "rec"), 1).remove(0);
    assert_eq!(heard(&capture), Some(15_092));
    let Response::Finished { manifest, .. } = result(server.handle(Request::Finish {}, Vec::new())) else { panic!() };
    assert_eq!(manifest.sections[&ezsdr_kernel::spec::Namespace::parse("ezsdr.children").unwrap()].as_array().unwrap().len(), 1);
}
