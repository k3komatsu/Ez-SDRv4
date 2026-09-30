//! The fixtures `tests/fake.rs` and `tests/hardware.rs` share: a Run assembled as the
//! server's catalogue assembles one, on any `Device` (spec 18 §6, UR-34).
#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, BindingProfile};
use ezsdr_kernel::contract::ContractRegistry;
use ezsdr_kernel::coordinator::{Assembly, RunHandle, RunHandleError, connect, start_spec_run};
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector, EventKind};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ResourceId, RunId};
use ezsdr_kernel::manifest::{ArtifactRef, Manifest};
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, Authority, AttachedPort, Endpoint, ExecutionClass, Link,
    ModuleErrorKind, ModuleRegistry, PrepareContext, Provider, Role, Sink,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::run::{Lease, RunState, Stage, StopCause, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BurstEnd, BurstRecord, DataLinkDecl, GapCause};
use ezsdr_kernel::time::{ClockRegistry, Duration, RelativeBudget, TimePoint};
use ezsdr_radio::payloads::{RxOverflowCause, RxOverflowPayload, TimeErrorOutcome, TimeErrorPayload};
use ezsdr_radio_uhd::profile::Profile;
use ezsdr_radio_uhd::{Device, DeviceAuthority, FakeConfig, FakeDevice, FakeFault, TxCode, UhdRadio};
use serde_json::{Value as Json, json};

// ---------------------------------------------------------------- fixtures

pub const MCR: i64 = 200_000_000;
pub const ARGS: &str = "type=fake-rig";

pub fn ms(n: i64) -> i64 {
    n * MCR / 1_000
}

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ezsdr-uhd-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn uhd_module() -> Json {
    json!({ "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 1, "patch": 0 } })
}

pub fn x310_ubx() -> Json {
    json!({ "name": "x310-ubx", "version": { "major": 0, "minor": 1, "patch": 0 } })
}

/// The profile of the device's front ends: `x310-obx` on an OBX (the bench, bench.md),
/// `x310-cbx` on a CBX, else `x310-ubx` (the fake's default).
pub fn profile_of(device: &dyn Device) -> Profile {
    let name = device.front_end(ezsdr_radio_uhd::Dir::Rx, 0).unwrap_or_default();
    [Profile::X310Obx, Profile::X310Cbx].into_iter().find(|p| name.starts_with(p.front_end())).unwrap_or(Profile::X310Ubx)
}

/// A fake of `x310-obx`'s device, the bench's: an OBX in slot A and slot B empty, which
/// UHD reports as a second channel on its unknown board (`db_unknown.cpp`).
pub fn one_obx() -> FakeConfig {
    FakeConfig { front_ends: vec!["OBX", "Unknown (0xffff) - 0"], ..FakeConfig::default() }
}

/// A fake of `x310-cbx`'s device: a CBX-120 in slot A and slot B empty, which UHD
/// reports as a second channel on its unknown board (`db_unknown.cpp`).
pub fn one_cbx() -> FakeConfig {
    FakeConfig { front_ends: vec!["CBX-120", "Unknown (0xffff) - 0"], ..FakeConfig::default() }
}

/// The frequency the steps receive and transmit at: the default of the device's
/// profile, so a Session that sets none (B7) stays inside the RF envelope around it.
pub fn bench_hz(device: &dyn Device) -> f64 {
    match profile_of(device).description(2_000).defaults[&Key::parse(ezsdr_radio::keys::RX_FREQUENCY_HZ).unwrap()] {
        Value::Num(hz) => hz,
        ref other => panic!("a default frequency, not {other:?}"),
    }
}

/// `profile` with the radio binding's profile the device's (UR-5).
pub fn bench_profile(device: &dyn Device, dir: &TempDir, selector: Json, environment: Json, session: bool) -> Json {
    let mut doc = profile(dir, selector, environment, session);
    doc["bindings"]["radio"]["profile"] = serde_json::to_value(profile_of(device).profile_ref()).unwrap();
    doc
}

/// The bench profile of `bench.md` with the fake's args (Vision §59: only the radio
/// binding, the Authority and `ezsdr.rf_path` differ from the Mock profile).
pub fn profile(dir: &TempDir, selector: Json, environment: Json, session: bool) -> Json {
    let mut radio_selector = json!({ "args": ARGS, "id": "usrp" });
    radio_selector.as_object_mut().unwrap().extend(selector.as_object().cloned().unwrap_or_default());
    let mut rec = json!({
        "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 2, "patch": 0 } },
        "selector": { "dir": dir.0.to_string_lossy() }
    });
    if session {
        rec["feed"] = json!({ "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 64 });
    }
    let mut env = json!({
        "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 2_000_000_000u64 },
        "ezsdr.rf_path": { "path": "cabled" }
    });
    env.as_object_mut().unwrap().extend(environment.as_object().cloned().unwrap_or_default());
    json!({
        "version": 1,
        "bindings": {
            "radio": { "module": uhd_module(), "profile": x310_ubx(), "selector": radio_selector },
            "rec": rec
        },
        "authority": "radio",
        "placements": { "links": [{
            "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
            "from": { "component": "radio", "port": "rx" },
            "to": { "component": "rec", "port": "in" }
        }] },
        "environment": env
    })
}

pub fn receive_spec(channels: i64, rate: f64, frequency: f64, capture: Option<i64>) -> Json {
    let mut output = json!({
        "id": "rec", "kind": "sink.capture", "params": {},
        "feed": { "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 64 }
    });
    if let Some(n) = capture {
        output["params"]["sink.capture_samples"] = json!(n);
    }
    json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 1 }, { "id": "sink", "major": 1 }] },
        "resources": { "radio": { "kind": "radio.device", "requires": {
            "radio.rx.channels": { "kind": "eq", "value": channels },
            "radio.rx.sample_rate_hz": { "kind": "eq", "value": rate },
            "radio.rx.frequency_hz": { "kind": "eq", "value": frequency }
        } } },
        "outputs": [output]
    })
}

pub fn with_tx(mut spec: Json, rate: f64) -> Json {
    let requires = &mut spec["resources"]["radio"]["requires"];
    requires["radio.tx.channels"] = json!({ "kind": "eq", "value": 1 });
    requires["radio.tx.sample_rate_hz"] = json!({ "kind": "eq", "value": rate });
    requires["radio.tx.frequency_hz"] = requires["radio.rx.frequency_hz"].clone();
    spec
}

pub fn with_burst(mut spec: Json, waveform: &ArtifactRef, repeat: bool, late: &str, offset: i64) -> Json {
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": offset },
        "action": {
            "kind": "tx_burst",
            "target": serde_json::to_value(ResourceId::parse("radio/tx").unwrap()).unwrap(),
            "waveform": waveform, "repeat": repeat, "late_policy": late, "metadata": {}
        }
    }]);
    spec
}

pub fn waveform_of(samples: &[(f32, f32)]) -> (Vec<u8>, ArtifactRef) {
    let mut bytes = Vec::with_capacity(samples.len() * 8);
    for (re, im) in samples {
        bytes.extend_from_slice(&re.to_le_bytes());
        bytes.extend_from_slice(&im.to_le_bytes());
    }
    let hash = ContentHash::of_bytes(&bytes);
    let reference = ezsdr_kernel::manifest::ingest_input(
        Ident::parse("waveform").unwrap(),
        Namespace::parse("ezsdr.input").unwrap(),
        format!("mem:{hash}"),
        &bytes,
    );
    (bytes, reference)
}

pub fn tone(n: usize) -> Vec<(f32, f32)> {
    (0..n).map(|i| (0.25 + 0.5 * (i as f32 / n as f32), -0.125)).collect()
}

/// A Run's Assembly as the server's catalogue builds one (EA-7): the radio, the
/// Authority on the same device, the capture Sink and the host Link.
pub fn assembly(profile_doc: &Json, device: Arc<dyn Device>, inputs: BTreeMap<ContentHash, Vec<u8>>, adjust: impl FnOnce(UhdRadio) -> UhdRadio) -> Assembly {
    let profile = BindingProfile::from_json(profile_doc).unwrap();
    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    ezsdr_sink::register(&mut registry, &mut checks, &mut kinds).unwrap();
    registry.register(ezsdr_radio_uhd::descriptor(), ezsdr_radio_uhd::factories()).unwrap();
    let link = ezsdr_kernel::module_api::Factories { link: true, ..Default::default() };
    registry.register(ezsdr_link_host::descriptor(), link).unwrap();
    registry.register_link_descriptor(ezsdr_link_host::link_descriptor()).unwrap();
    let sink = ezsdr_kernel::module_api::Factories { sink: true, ..Default::default() };
    registry.register(ezsdr_sink_capture::descriptor(), sink).unwrap();
    let clocks = Arc::new(ClockRegistry::new());
    let radio = &profile.bindings[&Ident::parse("radio").unwrap()];
    let selector = |name: &str| match radio.selector.get(&Ident::parse(name).unwrap()) {
        Some(Value::Str(s)) => s.clone(),
        _ => "internal".to_owned(),
    };
    let authority_args = match profile.bindings[&profile.authority].selector.get(&Ident::parse("authority_args").unwrap()) {
        Some(Value::Str(s)) => s.clone(),
        _ => selector("args"),
    };
    let authority = DeviceAuthority::new(device.clone(), clocks.clone(), &selector("clock_source"), &selector("time_source"), &authority_args).unwrap();
    let mut clean = radio.clone();
    clean.selector.remove(&Ident::parse("authority_args").unwrap());
    let mut providers: BTreeMap<Ident, Box<dyn Provider>> = BTreeMap::new();
    providers.insert(Ident::parse("radio").unwrap(), Box::new(adjust(UhdRadio::from_binding(&clean, device).unwrap())));
    let mut sinks: BTreeMap<Ident, Box<dyn Sink>> = BTreeMap::new();
    for (name, binding) in &profile.bindings {
        if binding.module.id.as_str() == "ezsdr.sink.capture" {
            sinks.insert(name.clone(), Box::new(ezsdr_sink_capture::CaptureSink::from_binding(binding).unwrap()));
        }
    }
    let mut links: BTreeMap<_, Box<dyn Link>> = BTreeMap::new();
    for placement in &profile.placements.links {
        links.insert(placement.link.clone(), Box::new(ezsdr_link_host::HostLinkModule::new()) as Box<dyn Link>);
    }
    Assembly {
        registry,
        checks,
        kinds,
        contracts: ContractRegistry::with_standard_contracts(),
        clocks,
        host_clock: Arc::new(ezsdr_kernel::run::SystemHostClock::new()),
        providers,
        executors: BTreeMap::new(),
        sinks,
        authority: Box::new(authority),
        links,
        inputs,
    }
}

pub fn fake(config: FakeConfig) -> Arc<FakeDevice> {
    Arc::new(FakeDevice::new(config))
}

pub fn spec_run(spec: &Json, profile: &Json, device: Arc<dyn Device>, inputs: BTreeMap<ContentHash, Vec<u8>>) -> RunHandle {
    start_spec_run(spec, profile, assembly(profile, device, inputs, |r| r)).unwrap()
}

pub fn session(profile: &Json, device: Arc<dyn Device>) -> RunHandle {
    let run = connect(profile, assembly(profile, device, BTreeMap::new(), |r| r), Lease::attached()).unwrap();
    assert!(matches!(run.state(), RunState::Running {}), "{:?}", run.state());
    run
}

pub fn running(run: &RunHandle) {
    assert!(matches!(run.state(), RunState::Running {}), "{:?}", run.state());
}

pub fn after(run: &RunHandle, ticks: i64) -> TimePoint {
    let now = run.now();
    TimePoint::new(now.domain, now.ticks + ticks)
}

/// Advances past T0, so that a Session's streams run (UR-15's start-up).
pub fn past_t0(run: &mut RunHandle, extra: i64) {
    let t0 = run.start_instant().unwrap();
    run.advance_to(TimePoint::new(t0.domain, t0.ticks + extra)).unwrap();
}

pub fn wait(run: &mut RunHandle, ticks: i64) {
    let to = after(run, ticks);
    let _ = run.advance_to(to);
}

pub fn kind(name: &str) -> EventKind {
    EventKind::parse(name).unwrap()
}

pub fn events_of(manifest: &Manifest, name: &str) -> Vec<Event> {
    manifest.events.delivered.iter().filter(|e| e.kind == kind(name)).cloned().collect()
}

pub fn section<'a>(manifest: &'a Manifest, suffix: &str) -> &'a Json {
    &manifest.sections[&Namespace::parse(&format!("ezsdr.radio.uhd.usrp.{suffix}")).unwrap()]
}

pub fn capture_of(manifest: &Manifest, id: &str) -> ArtifactRef {
    manifest.artifacts.iter().find(|a| a.id.as_str() == id || a.id.as_str().starts_with(&format!("{id}_"))).unwrap_or_else(|| panic!("no artifact {id}: {:?}", manifest.artifacts)).clone()
}

pub fn read_capture(artifact: &ArtifactRef, channels: usize) -> Vec<Vec<(f32, f32)>> {
    let bytes = std::fs::read(artifact.uri.strip_prefix("file://").unwrap()).unwrap();
    let mut out = vec![Vec::new(); channels];
    for frame in bytes.chunks_exact(8 * channels) {
        for (c, channel) in out.iter_mut().enumerate() {
            let at = c * 8;
            channel.push((
                f32::from_le_bytes(frame[at..at + 4].try_into().unwrap()),
                f32::from_le_bytes(frame[at + 4..at + 8].try_into().unwrap()),
            ));
        }
    }
    out
}

/// Runs a Spec until its capture is written, then finishes it.
pub fn captured(mut run: RunHandle) -> Manifest {
    let horizon = after(&run, ms(10_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    run.finish()
}

pub fn failure(manifest: &Manifest) -> String {
    manifest.sections.get(&Namespace::parse("ezsdr.failure").unwrap()).map_or(String::new(), |f| f["reason"].as_str().unwrap_or_default().to_owned())
}

pub fn verb(verb: &str, target: &str, at: Option<TimePoint>, params: &[(&str, Value)]) -> SessionAction {
    SessionAction::Vocabulary {
        ns: Namespace::parse(if verb == "capture" { "sink" } else { "radio" }).unwrap(),
        verb: Ident::parse(verb).unwrap(),
        target: ResourceId::parse(target).unwrap(),
        at,
        params: params.iter().map(|(k, v)| (Key::parse(k).unwrap(), v.clone())).collect(),
    }
}

pub fn set(key: &str, value: Value) -> SessionAction {
    SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse(key).unwrap(), value }
}

pub fn admitted(entry: &ezsdr_kernel::session::LogEntry) -> bool {
    matches!(entry.outcome, Outcome::Admitted { .. })
}

pub fn bursts(manifest: &Manifest) -> Vec<BurstRecord> {
    serde_json::from_value(section(manifest, "bursts").clone()).unwrap()
}

pub fn calls(device: &FakeDevice, prefix: &str) -> Vec<String> {
    device.calls().into_iter().filter(|c| c.starts_with(prefix)).collect()
}

pub fn tx_clock_origin(manifest: &Manifest, nth: usize) -> i64 {
    manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/tx").unwrap()).nth(nth).unwrap().origin.ticks
}


// ---------------------------------------------------------------- the rehearsal (UR-34)
//
// The bench steps B3–B7 as functions of a device: `tests/fake.rs` calls them with a
// `FakeDevice` and `tests/hardware.rs` with a `UhdDevice`. `exact` says whether the
// loopback is sample-exact (the fake) or has to be found by correlation (a cable).

/// A QPSK pseudo-noise waveform of amplitude 0.4, for correlation (B6, B7).
pub fn pn(n: usize) -> Vec<(f32, f32)> {
    let mut state: u32 = 0x1234_5678;
    (0..n)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let bit = |b: u32| if state >> b & 1 == 1 { 0.4 } else { -0.4 };
            (bit(30), bit(29))
        })
        .collect()
}

/// Where `wave` appears exactly in `samples` (a loopback without a channel).
pub fn exactly(samples: &[(f32, f32)], wave: &[(f32, f32)]) -> Option<usize> {
    (0..=samples.len().saturating_sub(wave.len())).find(|at| samples[*at..].starts_with(wave))
}

/// Where `wave` first appears in `samples`, by the peak of their correlation.
pub fn correlate(samples: &[(f32, f32)], wave: &[(f32, f32)]) -> Option<usize> {
    let energy: f32 = wave.iter().map(|(re, im)| re * re + im * im).sum();
    (0..samples.len().saturating_sub(wave.len()))
        .map(|at| {
            let dot: f32 = wave.iter().zip(&samples[at..]).map(|((wr, wi), (sr, si))| wr * sr + wi * si).sum();
            (at, dot)
        })
        .filter(|(_, dot)| *dot > 0.5 * energy * 0.0316)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(at, _)| at)
}

/// What a bench capture holds against `wave`: its RMS, and the peaks of the complex
/// correlation's magnitude and of its real part (what `correlate` thresholds), each as a
/// fraction of the waveform's energy (the loop's amplitude gain at the peak).
pub fn diagnose(what: &str, samples: &[(f32, f32)], wave: &[(f32, f32)]) {
    let energy: f32 = wave.iter().map(|(re, im)| re * re + im * im).sum();
    let rms = (samples.iter().map(|(re, im)| re * re + im * im).sum::<f32>() / samples.len().max(1) as f32).sqrt();
    let dots: Vec<(usize, f32, f32)> = (0..samples.len().saturating_sub(wave.len()))
        .map(|at| {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for ((wr, wi), (sr, si)) in wave.iter().zip(&samples[at..]) {
                re += wr * sr + wi * si;
                im += wr * si - wi * sr;
            }
            (at, re, im)
        })
        .collect();
    let by_abs = dots.iter().max_by(|a, b| a.1.hypot(a.2).total_cmp(&b.1.hypot(b.2)));
    let by_re = dots.iter().max_by(|a, b| a.1.total_cmp(&b.1));
    println!(
        "{what}: {} samples, rms {rms:.5}; |corr| peak {:?}; Re(corr) peak {:?}; threshold {:.5} (fractions of the energy {energy})",
        samples.len(),
        by_abs.map(|(at, re, im)| (*at, re.hypot(*im) / energy, im.atan2(*re).to_degrees())),
        by_re.map(|(at, re, _)| (*at, re / energy)),
        0.5 * 0.0316
    );
}

/// B3: `experiments::receive(1, 1e6, f, Some(10_000))` at the bench frequency `f` under the bench profile.
pub fn rehearse_receive_at_t0(device: Arc<dyn Device>) -> Manifest {
    let dir = TempDir::new();
    let manifest = captured(spec_run(&receive_spec(1, 1e6, bench_hz(&*device), Some(10_000)), &bench_profile(&*device, &dir, json!({}), json!({}), false), device, BTreeMap::new()));
    assert!(matches!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} }), "{:?}", manifest.termination);
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.first.ticks, 0, "the capture starts at receive sample 0");
    assert!(map.gaps.is_empty(), "{:?}", map.gaps);
    let first = section(&manifest, "timing").as_array().unwrap().iter().find(|r| r["what"] == "first_rx_block").cloned().unwrap();
    assert_eq!(first["index"], 0);
    let rate = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.sample_rate_hz").cloned().unwrap();
    assert_eq!(rate["claimed"], rate["read_back"]);
    manifest
}

/// B4: a capture of 10 000 samples at receive sample 50 000 (v3 behaviour 2).
pub fn rehearse_capture_at_a_sample_index(device: Arc<dyn Device>) -> Manifest {
    let dir = TempDir::new();
    let mut spec = receive_spec(1, 1e6, bench_hz(&*device), None);
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": 50_000 },
        "action": { "kind": "update_parameter", "target": serde_json::to_value(ResourceId::parse("sink/rec").unwrap()).unwrap(),
                    "key": "sink.capture_samples", "value": 10_000, "class": "block_boundary" }
    }]);
    let manifest = captured(spec_run(&spec, &bench_profile(&*device, &dir, json!({}), json!({}), false), device, BTreeMap::new()));
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.first.ticks, 50_000);
    assert_eq!(map.end.ticks, 60_000);
    manifest
}

/// B5: a receive Run at 10 Msps whose reader stalls once: RM-17's overrun gap.
pub fn rehearse_overflow(device: Arc<dyn Device>, after: Wall, stall: Wall) -> Manifest {
    let dir = TempDir::new();
    let doc = bench_profile(&*device, &dir, json!({}), json!({}), false);
    let spec = receive_spec(1, 10e6, bench_hz(&*device), Some(30_000_000));
    let mut run = start_spec_run(&spec, &doc, assembly(&doc, device, BTreeMap::new(), |r| r.with_rx_stall(after, stall))).unwrap();
    let horizon = after_ticks(&run, ms(2_000) + ms((after + stall).as_millis() as i64 + 500));
    let _ = run.advance_to(horizon);
    let manifest = run.finish();
    let overflows = events_of(&manifest, "radio.RX_OVERFLOW");
    assert!(!overflows.is_empty(), "no overflow; the host's socket buffer absorbed the stall");
    let payload = ezsdr_radio::payloads::RxOverflowPayload::from_payload(&overflows[0].payload).unwrap();
    assert_eq!(payload.cause, ezsdr_radio::payloads::RxOverflowCause::Overrun);
    let map = &capture_of(&manifest, "rec").continuity[0];
    let gap = map.gaps.iter().find(|g| g.cause == ezsdr_kernel::stream::GapCause::OverflowRestart {}).expect("an overflow_restart gap");
    assert_eq!(gap.lost, Some(gap.len));
    assert_eq!(payload.lost, gap.len);
    manifest
}

/// B6: a timed burst heard by a timed capture on one device (v3 behaviour 3), then a
/// repeat captured back to back across its wraps (v3 behaviour 1).
pub fn rehearse_txrx_and_repeat(device: Arc<dyn Device>, exact: bool) -> (Manifest, Manifest) {
    let wave = pn(1_000);
    let (bytes, waveform) = waveform_of(&wave);
    let capture = |spec: &mut Json, offset: i64, n: i64| {
        spec["schedule"].as_array_mut().unwrap().push(json!({
            "at": { "clock": "radio", "offset_ticks": offset },
            "action": { "kind": "update_parameter", "target": serde_json::to_value(ResourceId::parse("sink/rec").unwrap()).unwrap(),
                        "key": "sink.capture_samples", "value": n, "class": "block_boundary" }
        }));
    };
    let dir = TempDir::new();
    let mut burst = with_burst(with_tx(receive_spec(1, 1e6, bench_hz(&*device), None), 1e6), &waveform, false, "drop_and_flag", 10_000);
    capture(&mut burst, 10_000, 5_000);
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes.clone())]);
    let heard = captured(spec_run(&burst, &bench_profile(&*device, &dir, json!({}), json!({}), false), device.clone(), inputs.clone()));
    let samples = read_capture(&capture_of(&heard, "rec"), 1).remove(0);
    if !exact {
        diagnose("B6 burst", &samples, &wave);
    }
    let at = if exact { exactly(&samples, &wave) } else { correlate(&samples, &wave) }.expect("the burst's correlation peak");
    if exact {
        assert_eq!(at, 0, "the burst at T0 + 10 000 samples is received at sample 10 000");
    }
    println!("B6: transmit-to-receive delay {at} samples");
    let dir = TempDir::new();
    let mut repeat = with_burst(with_tx(receive_spec(1, 1e6, bench_hz(&*device), None), 1e6), &waveform, true, "send_asap_and_flag", 1_000);
    capture(&mut repeat, 1_000, 4_000);
    let looped = captured(spec_run(&repeat, &bench_profile(&*device, &dir, json!({}), json!({}), false), device, inputs));
    let samples = read_capture(&capture_of(&looped, "rec"), 1).remove(0);
    if !exact {
        diagnose("B6 repeat", &samples, &wave);
    }
    let start = if exact { 0 } else { correlate(&samples, &wave).expect("the repeat is heard") };
    for (i, sample) in samples[start..samples.len() - wave.len()].iter().enumerate() {
        if exact {
            assert_eq!(*sample, wave[i % wave.len()], "sample {i}");
        }
    }
    assert!(events_of(&looped, "radio.TX_UNDERFLOW").is_empty());
    (heard, looped)
}

/// B7's shape from Rust: a Session that enables transmit, repeats a waveform, captures
/// it ahead of the Run's time (EA-17), and is refused a retune outside the RF envelope
/// (§58 #16).
pub fn rehearse_session_loopback(device: Arc<dyn Device>, exact: bool) -> Manifest {
    let dir = TempDir::new();
    let hz = bench_hz(&*device);
    let envelope = json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": hz - 1e6, "hi_hz": hz + 1e6 }], "max_gain_db": 0.0, "tx_enabled": [true] } });
    let mut run = session(&bench_profile(&*device, &dir, json!({}), envelope, true), device);
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.tx.channels", Value::Int(1)), None).unwrap()));
    let wave = pn(1_000);
    let (bytes, _) = waveform_of(&wave);
    assert!(admitted(&run.submit(verb("start_repeat", "radio/tx", None, &[]), Some(&bytes)).unwrap()));
    let at = after_ticks(&run, ms(50));
    assert!(admitted(&run.submit(verb("capture", "sink/rec", Some(at), &[("sink.capture_samples", Value::Int(5_000))]), None).unwrap()));
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let refused = run.submit(set("radio.tx.frequency_hz", Value::Num(hz + 100e6)), None).unwrap();
    assert!(matches!(&refused.outcome, Outcome::Rejected { violations } if violations.iter().any(|v| v.check.as_str() == "radio.rf_envelope")), "{refused:?}");
    let manifest = run.finish();
    let samples = read_capture(&capture_of(&manifest, "rec"), 1).remove(0);
    let found = if exact { exactly(&samples, &wave) } else { correlate(&samples, &wave) };
    assert!(found.is_some(), "the loopback holds the waveform");
    let errors = events_of(&manifest, "radio.TIME_ERROR");
    if exact {
        // On the fake, under a test binary's parallel load, the untimed repeat can miss
        // the delivery allowance and move (Review L, P1-8; Review M's runs saw 2.09 ms).
        // K6 itself is `ur_21_an_untimed_send_is_on_time_after_its_delivery`'s; here a
        // device-late burst, or a move beyond the whole 5 ms lead, fails.
        let payloads: Vec<TimeErrorPayload> = errors.iter().map(|e| serde_json::from_value(e.payload.clone()).unwrap()).collect();
        let lead = ezsdr_radio_uhd::profile::DEVICE_LEAD_NS + ezsdr_radio_uhd::profile::DELIVERY_ALLOWANCE_NS;
        assert!(
            payloads.iter().all(|p| p.outcome != TimeErrorOutcome::LateAtDevice && p.late_by_ns <= lead),
            "spike K6: {payloads:?}"
        );
    } else {
        assert!(errors.is_empty(), "no TIME_ERROR (spike K6): {errors:?}");
    }
    manifest
}

pub fn after_ticks(run: &RunHandle, ticks: i64) -> TimePoint {
    let now = run.now();
    TimePoint::new(now.domain, now.ticks + ticks)
}
