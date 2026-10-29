//! The fixtures `tests/fake.rs` and `tests/hardware.rs` share: a Run assembled as the
//! server's catalogue assembles one, on any `Device` (spec 18 §6, UR-34).
#![allow(dead_code, unused_imports)]

use ezsdr_kernel::spec::Scalar;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, BindingProfile};
use ezsdr_kernel::contract::ContractRegistry;
use ezsdr_kernel::coordinator::{Assembly, RunHandle, RunHandleError, connect, start_spec_run};
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector, EventKind, EventSource, Target};
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
use ezsdr_radio_uhd::{Device, DeviceAuthority, FakeConfig, FakeDevice, FakeFault, TxCode, UhdRadio};
use serde_json::{Value as Json, json};

// ------------------------------------------------ the per-test wall watchdog (R05)

mod watchdog;
pub use watchdog::watch;
pub mod d9;
pub mod measurement;
pub mod observations;
pub mod probe_device;
pub mod tone;

// ---------------------------------------------------------------- fixtures

pub const MCR: i64 = 200_000_000;
pub const ARGS: &str = "type=fake-rig";
pub const SAFE_TX_HZ: std::ops::RangeInclusive<f64> = 999_500_000.0..=1_000_500_000.0;

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
    json!({ "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } })
}

pub fn x3xx() -> Json {
    json!({ "name": "x3xx", "version": { "major": 0, "minor": 1, "patch": 0 } })
}

/// The board of the device's first receive front end, which decides the bench's frequency
/// and the profile's front ends (UR-5, UR-9).
pub fn board_of(device: &dyn Device) -> ezsdr_radio::x3xx::Board {
    let name = device.front_end(ezsdr_radio_uhd::Dir::Rx, 0).unwrap_or_default();
    ezsdr_radio::x3xx::Board::of_front_end(&name).expect("the board of the device's first rx front end")
}

/// The `x3xx` layout of the device's front ends: each receive and transmit channel, in slot
/// order from slot A, whose front end is a board of slot A's kind (design 33 D5; UR-5: one
/// device carries one kind, so a UBX in A and a CBX in B lays out slot A alone).
pub fn layout_of(device: &dyn Device) -> (Json, Json) {
    let first = board_of(device);
    let elements = |dir: ezsdr_radio_uhd::Dir| -> Json {
        let boards: Vec<&'static str> = (0..device.channels(dir))
            .map_while(|chan| device.front_end(dir, chan).ok().and_then(|name| ezsdr_radio::x3xx::Board::of_front_end(&name)))
            .take_while(|board| *board == first)
            .map(|board| board.name())
            .collect();
        Json::Array(
            boards
                .iter()
                .enumerate()
                .map(|(index, board)| {
                    let slot = if index == 0 { "A" } else { "B" };
                    json!(format!("0:{slot}:{board}:0"))
                })
                .collect(),
        )
    };
    (elements(ezsdr_radio_uhd::Dir::Rx), elements(ezsdr_radio_uhd::Dir::Tx))
}

/// The keys `EZSDR_MEASUREMENT_SELECTOR` may carry (stage 3c-3a): the layout, the image and the links.
pub const SELECTOR_KEYS: [&str; 5] = ["rx", "tx", "image", "links", "link_gbps"];

/// The selector `EZSDR_MEASUREMENT_SELECTOR` holds: a JSON object of `SELECTOR_KEYS` only. Whether the
/// layout fits the device is the Provider's to judge (UR-5, RM-28), not this function's.
pub fn parse_selector(text: &str) -> Result<Json, String> {
    let value: Json = serde_json::from_str(text).map_err(|error| format!("EZSDR_MEASUREMENT_SELECTOR is not JSON: {error}"))?;
    let object = value.as_object().ok_or("EZSDR_MEASUREMENT_SELECTOR must be a JSON object")?;
    if let Some(key) = object.keys().find(|key| !SELECTOR_KEYS.contains(&key.as_str())) {
        return Err(format!("EZSDR_MEASUREMENT_SELECTOR has the key `{key}`; it may carry only {SELECTOR_KEYS:?}"));
    }
    // A selector that names one direction's layout names both: the other would be inherited from the device, and the record's
    // transmitting channels would not be the ones the attenuator confirmation names.
    if object.contains_key("rx") != object.contains_key("tx") {
        return Err("EZSDR_MEASUREMENT_SELECTOR must give both `rx` and `tx`, or neither".to_owned());
    }
    Ok(value)
}

thread_local! {
    static SELECTOR_OVERRIDE: std::cell::RefCell<Option<Json>> = const { std::cell::RefCell::new(None) };
}

/// The selector every `bench_profile` of this thread overlays, if one was installed.
pub fn selector_override() -> Option<Json> {
    SELECTOR_OVERRIDE.with(|cell| cell.borrow().clone())
}

/// Installs `selector` for this thread (`None` removes it); the hardware record installs the environment's once.
pub fn set_selector_override(selector: Option<Json>) {
    SELECTOR_OVERRIDE.with(|cell| *cell.borrow_mut() = selector);
}

/// Runs `body` with `selector` installed, and removes it again, also on a panic.
pub fn with_selector<T>(selector: Json, body: impl FnOnce() -> T) -> T {
    struct Remove;
    impl Drop for Remove {
        fn drop(&mut self) { set_selector_override(None); }
    }
    set_selector_override(Some(selector));
    let _remove = Remove;
    body()
}

/// The layout elements the bench profile names: the installed selector's `rx` and `tx` where it has them, else the device's.
pub fn effective_layout(device: &dyn Device) -> (Json, Json) {
    let selector = selector_override();
    let named = |key: &str| selector.as_ref().and_then(|selector| selector.get(key)).filter(|value| value.is_array()).cloned();
    match (named("rx"), named("tx")) {
        (Some(rx), Some(tx)) => (rx, tx),
        (rx, tx) => {
            let (device_rx, device_tx) = layout_of(device);
            (rx.unwrap_or(device_rx), tx.unwrap_or(device_tx))
        }
    }
}

/// The default frequency of channel `chan`'s front end in `dir` (RM-5; a Basic/LF board's is 0 Hz), or the
/// device's first board's where the layout has no such channel.
pub fn channel_hz(device: &dyn Device, dir: ezsdr_radio_uhd::Dir, chan: usize) -> f64 {
    let (rx, tx) = effective_layout(device);
    let elements = if dir == ezsdr_radio_uhd::Dir::Rx { rx } else { tx };
    elements.get(chan).and_then(Json::as_str).and_then(|element| element.split(':').nth(2))
        .and_then(ezsdr_radio::x3xx::FrontEnd::parse)
        .map_or_else(|| board_of(device).default_frequency_hz(), ezsdr_radio::x3xx::FrontEnd::default_frequency_hz)
}

/// A fake of `x3xx`'s device, the bench's: an OBX in slot A and slot B empty, which
/// UHD reports as a second channel on its unknown board (`db_unknown.cpp`).
pub fn one_obx() -> FakeConfig {
    FakeConfig { front_ends: vec!["OBX", "Unknown (0xffff) - 0"], ..FakeConfig::default() }
}

/// A fake of `x3xx`'s device: a CBX-120 in slot A and slot B empty, which UHD
/// reports as a second channel on its unknown board (`db_unknown.cpp`).
pub fn one_cbx() -> FakeConfig {
    FakeConfig { front_ends: vec!["CBX-120", "Unknown (0xffff) - 0"], ..FakeConfig::default() }
}

/// The frequency the steps receive and transmit at: the default of the device's board
/// (UR-9), so a Session that sets none (B7) stays inside the RF envelope around it.
pub fn bench_hz(device: &dyn Device) -> f64 {
    channel_hz(device, ezsdr_radio_uhd::Dir::Rx, 0)
}

/// bench.md's RF envelope around the bench frequency: every step that transmits carries
/// it (RM-19), so that nothing but the envelope holds the transmitter to the band. It has a band around each
/// transmit channel's frequency and one `tx_enabled` entry each (RM-33).
pub fn bench_envelope(device: &dyn Device) -> Json {
    let channels = effective_layout(device).1.as_array().map_or(1, Vec::len).max(1);
    let mut bands: Vec<(f64, f64)> = Vec::new();
    for chan in 0..channels {
        let hz = channel_hz(device, ezsdr_radio_uhd::Dir::Tx, chan);
        let band = ((hz - 1e6).max(0.0), hz + 1e6);
        if !bands.contains(&band) { bands.push(band); }
    }
    let bands: Vec<Json> = bands.into_iter().map(|(lo, hi)| json!({ "lo_hz": lo, "hi_hz": hi })).collect();
    json!({ "radio.rf_envelope": { "allowed_bands": bands, "max_gain_db": 0.0, "tx_enabled": vec![true; channels] } })
}

/// `profile` with the radio binding's profile the `x3xx` one and its layout the device's
/// front ends (UR-5, design 33 D5); the installed selector (`EZSDR_MEASUREMENT_SELECTOR`) is laid over it.
pub fn bench_profile(device: &dyn Device, dir: &TempDir, selector: Json, environment: Json, session: bool) -> Json {
    let mut doc = profile(dir, selector, environment, session);
    doc["bindings"]["radio"]["profile"] = x3xx();
    let (rx, tx) = effective_layout(device);
    doc["bindings"]["radio"]["selector"]["rx"] = rx;
    doc["bindings"]["radio"]["selector"]["tx"] = tx;
    if let Some(Json::Object(overlay)) = selector_override() {
        for (key, value) in overlay { doc["bindings"]["radio"]["selector"][key] = value; }
    }
    // The bench's HG image streams over its 10 GbE port (bench.md B0.2); the HG default is 1 GbE.
    if doc["bindings"]["radio"]["selector"].get("link_gbps").is_none() {
        doc["bindings"]["radio"]["selector"]["link_gbps"] = json!([10]);
    }
    doc
}

/// The bench profile of `bench.md` with the fake's args (Vision §59: only the radio
/// binding, the Authority and `ezsdr.rf_path` differ from the Mock profile).
pub fn profile(dir: &TempDir, selector: Json, environment: Json, session: bool) -> Json {
    let mut radio_selector = json!({
        "args": ARGS, "id": "usrp",
        "rx": ["0:A:ubx:0"],
        "tx": ["0:A:ubx:0"]
    });
    radio_selector.as_object_mut().unwrap().extend(selector.as_object().cloned().unwrap_or_default());
    let mut rec = json!({
        "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 2, "patch": 0 } },
        "selector": { "dir": dir.0.to_string_lossy() }
    });
    if session {
        rec["feed"] = json!({ "port": { "component": "radio", "port": "rx0" }, "policy": "drop_oldest", "capacity": 64 });
    }
    let mut env = json!({
        "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 2_000_000_000u64 },
        "ezsdr.rf_path": { "path": "cabled" }
    });
    env.as_object_mut().unwrap().extend(environment.as_object().cloned().unwrap_or_default());
    json!({
        "version": 1,
        "bindings": {
            "radio": { "module": uhd_module(), "profile": x3xx(), "selector": radio_selector },
            "rec": rec
        },
        "authority": "radio",
        "placements": { "links": [{
            "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
            "from": { "component": "radio", "port": "rx0" },
            "to": { "component": "rec", "port": "in" }
        }] },
        "environment": env
    })
}

pub fn receive_spec(channels: i64, rate: f64, frequency: f64, capture: Option<i64>) -> Json {
    assert!(matches!(channels, 0 | 1), "receive_spec supports one channel, got {channels}");
    let mut output = json!({
        "id": "rec", "kind": "sink.capture", "params": {},
        "feed": { "port": { "component": "radio", "port": "rx0" }, "policy": "drop_oldest", "capacity": 64 }
    });
    if let Some(n) = capture {
        output["params"]["sink.capture_samples"] = json!(n);
    }
    let mut requires = json!({ "radio.rx.s0.sample_rate_hz": { "kind": "eq", "value": rate } });
    requires["radio.rx.c0.stream"] = json!({ "kind": "eq", "value": if channels > 0 { 0 } else { -1 } });
    requires["radio.rx.c0.frequency_hz"] = json!({ "kind": "eq", "value": frequency });
    json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 2 }, { "id": "sink", "major": 1 }] },
        "resources": { "radio": { "kind": "radio.device", "requires": requires } },
        "outputs": [output]
    })
}

pub fn with_tx(mut spec: Json, rate: f64) -> Json {
    let requires = &mut spec["resources"]["radio"]["requires"];
    requires["radio.tx.c0.stream"] = json!({ "kind": "eq", "value": 0 });
    requires["radio.tx.s0.sample_rate_hz"] = json!({ "kind": "eq", "value": rate });
    requires["radio.tx.c0.frequency_hz"] = requires["radio.rx.c0.frequency_hz"].clone();
    spec
}

pub fn with_burst(mut spec: Json, waveform: &ArtifactRef, repeat: bool, late: &str, offset: i64) -> Json {
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": offset },
        "action": {
            "kind": "tx_burst",
            "target": serde_json::to_value(resource("radio/tx/s0")).unwrap(),
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

/// A waveform for the transmissions added in stage 3c-3a (duplex stages and companions): `pn` scaled so that every sample's complex
/// magnitude is within bench.md's 0.5; the bound is asserted before anything is sent. (The earlier steps keep sending `pn` itself.)
pub fn transmit_waveform(n: usize) -> (Vec<u8>, ArtifactRef) {
    waveform_of(&transmit_samples(n))
}

/// The samples of `transmit_waveform`, for a probe that compares what it receives with them.
pub fn transmit_samples(n: usize) -> Vec<(f32, f32)> {
    let samples: Vec<(f32, f32)> = pn(n).into_iter().map(|(re, im)| (re * 0.875, im * 0.875)).collect();
    assert!(samples.iter().all(|(re, im)| re.hypot(*im) <= 0.5), "bench.md RF safety: a transmit waveform's magnitude is at most 0.5");
    samples
}

pub fn tone(n: usize) -> Vec<(f32, f32)> {
    (0..n).map(|i| (0.25 + 0.5 * (i as f32 / n as f32), -0.125)).collect()
}

/// A Run's Assembly as the server's catalogue builds one (EA-7): the radio, the
/// Authority on the same device, the capture Sink and the host Link.
pub fn assembly(profile_doc: &Json, device: Arc<dyn Device>, inputs: BTreeMap<ContentHash, Vec<u8>>, adjust: impl FnOnce(UhdRadio) -> UhdRadio) -> Assembly {
    // A Run on the manual clock is the one a frozen clock can hang (R05); a real-clock device
    // is its own wall bound. The Assembly is built before the Run starts, so the watchdog is
    // armed before any wait a mutant could freeze.
    if device.host().is_manual() {
        watch();
    }
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
        Some(Value::Scalar(Scalar::Str(s))) => s.clone(),
        _ => "internal".to_owned(),
    };
    let authority_args = match profile.bindings[&profile.authority].selector.get(&Ident::parse("authority_args").unwrap()) {
        Some(Value::Scalar(Scalar::Str(s))) => s.clone(),
        _ => selector("args"),
    };
    // UR-7: the reference PLL misses its lock on about 4% of opens (handoff §6.2); try up to three more times, a second apart.
    let authority = open_with_pll_retries(
        || DeviceAuthority::new(device.clone(), clocks.clone(), &selector("clock_source"), &selector("time_source"), &authority_args),
        3, Wall::from_secs(1),
    ).unwrap();
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
    let mut assembly = Assembly::new(Box::new(authority), clocks);
    assembly.registry = registry;
    assembly.checks = checks;
    assembly.kinds = kinds;
    assembly.providers = providers;
    assembly.sinks = sinks;
    assembly.links = links;
    assembly.inputs = inputs;
    assembly
}

/// Runs `open`, and again up to `retries` times after the reference clock's PLL failing to lock
/// (UR-7), `pause` apart. Any other error is returned at once, so that it cannot disappear into a
/// run that then succeeds.
pub fn open_with_pll_retries<T>(mut open: impl FnMut() -> Result<T, String>, retries: usize, pause: Wall) -> Result<T, String> {
    let mut opened = open();
    for retry in 1..=retries {
        let Err(error) = &opened else { break };
        if !error.contains(ezsdr_radio_uhd::REFERENCE_DID_NOT_LOCK) { break; }
        println!("UR-7 retry {retry}: {error}");
        std::thread::sleep(pause);
        opened = open();
    }
    // The retries used up on a PLL that stays unlocked: inside a record, nothing after this can use the device (stage 3e).
    if let Err(error) = &opened { measurement::latch_record_on_pll_failure(error); }
    opened
}

/// `set_sources` for a probe that opens the device itself, with the same UR-7 retries as an Authority's open: UHD's PLL text is
/// turned into `REFERENCE_DID_NOT_LOCK` as `DeviceAuthority::new` does (authority.rs), so the retry helper recognises it.
pub fn set_sources_with_pll_retries(device: &dyn Device, clock: &str, time: &str) -> Result<(), String> {
    set_sources_with_pll_retries_every(device, clock, time, Wall::from_secs(1))
}

/// `set_sources_with_pll_retries` with the retries `pause` apart.
pub fn set_sources_with_pll_retries_every(device: &dyn Device, clock: &str, time: &str, pause: Wall) -> Result<(), String> {
    open_with_pll_retries(|| device.set_sources(clock, time).map_err(|e| {
        if e.message.contains("Reference Clock PLL failed to lock") { format!("{} ({e})", ezsdr_radio_uhd::REFERENCE_DID_NOT_LOCK) } else { format!("UR-7: {e}") }
    }), 3, pause)
}

pub fn fake(config: FakeConfig) -> Arc<FakeDevice> {
    // The smoke set runs on the manual clock (plan 30 §4.4): the device's time stops while
    // a Provider thread is busy, so host load cannot miss a device deadline.
    watch();
    Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..config }))
}

/// A fake on the real host clock (plan 30 §4.4): the Authority's own diagnostics and the
/// tests that must keep a real host deadline (the wedged-thread set).
pub fn fake_real(config: FakeConfig) -> Arc<FakeDevice> {
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
    TimePoint::new(now.domain(), now.ticks_in(now.domain()).unwrap() + ticks)
}

/// Advances past T0, so that a Session's streams run (UR-15's start-up).
pub fn past_t0(run: &mut RunHandle, extra: i64) {
    let t0 = run.start_instant().unwrap();
    run.advance_to(TimePoint::new(t0.domain(), t0.ticks_in(t0.domain()).unwrap() + extra)).unwrap();
}

/// Advances past the origin of the transmit clock a change from 0 channels registered,
/// the start lead after it (RM-25; spec 22, VH-4), so that a burst sent then is on time.
pub fn past_tx_origin(run: &mut RunHandle) {
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.path() == "usrp/tx/s0" && r.ended_at.is_none()).expect("a transmit clock");
    let _ = run.advance_to(TimePoint::new(clock.origin.domain(), clock.origin.ticks_in(clock.root).unwrap() + ms(1)));
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

/// Check the measured prerequisites before interpreting a continuous waveform or
/// an exact capture index. Failure remains a test failure, with raw evidence;
/// it does not prove the host is at fault or rule out a runtime regression.
pub fn require_stream_timing(manifest: &Manifest, trial: &str, tx: bool) {
    let mut evidence: Vec<_> = manifest.events.delivered.iter().filter(|e|
        (e.kind == kind("radio.LATE_COMMAND") && e.payload["key"].is_null())
        || e.kind == kind("radio.RX_OVERFLOW")
        || (tx && e.kind == kind("radio.TX_UNDERFLOW"))
        || (tx && e.kind == kind("radio.TIME_ERROR") && matches!(
            e.payload["outcome"].as_str(), Some("drop" | "plan_violation" | "late_at_device"))))
        .map(|e| json!({ "kind": e.kind, "time": e.time, "payload": e.payload })).collect();
    if tx {
        evidence.extend(section(manifest, "rejected").as_array().unwrap().iter()
            .filter(|r| r["reason"].as_str().is_some_and(|s| s.contains("transmit timeline expired"))).cloned());
    }
    assert!(evidence.is_empty(),
        "TIMING_PRECONDITION_UNMET ({trial}): exact indices/uninterrupted playback require on-time, loss-free streams. Evidence: {evidence:?}; async: {}. Host scheduling or a runtime regression may cause this; see tests/README.md. The trial remains a test failure.",
        section(manifest, "async"));
}

pub fn section<'a>(manifest: &'a Manifest, suffix: &str) -> &'a Json {
    manifest.section(&usrp(), &format!("ezsdr.radio.uhd.{suffix}")).unwrap_or_else(|| panic!("no section {suffix}"))
}

/// The first artifact the Sink of `output` returned (RS-44).
pub fn capture_of(manifest: &Manifest, output: &str) -> ArtifactRef {
    manifest.artifacts.iter().find(|(o, _)| o.as_str() == output).and_then(|(_, list)| list.first()).unwrap_or_else(|| panic!("no artifact of {output}: {:?}", manifest.artifacts)).clone()
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
    match &manifest.termination.reason {
        ezsdr_kernel::run::Termination::Failed { reason, .. } => reason.clone(),
        _ => String::new(),
    }
}

/// The UHD instance's source: its node (KC-8), under which its sections are filed (RS-39).
pub fn usrp() -> EventSource {
    EventSource::Node { node: ResourceId::parse("usrp").unwrap() }
}

/// A resource target, `<resource>[/<path>]` (KC-23).
pub fn resource(path: &str) -> Target {
    let (resource, path) = path.split_once('/').unwrap_or((path, ""));
    Target::Resource { resource: Ident::parse(resource).unwrap(), path: path.to_owned() }
}

/// An output target (KC-23).
pub fn output(name: &str) -> Target {
    Target::Output { output: Ident::parse(name).unwrap() }
}

pub fn verb(verb: &str, target: Target, at: Option<TimePoint>, params: &[(&str, Value)]) -> SessionAction {
    SessionAction::Vocabulary {
        ns: Namespace::parse(if verb == "capture" { "sink" } else { "radio" }).unwrap(),
        verb: Ident::parse(verb).unwrap(),
        target,
        at,
        params: params.iter().map(|(k, v)| (Key::parse(k).unwrap(), v.clone())).collect(),
    }
}

pub fn set(key: &str, value: Value) -> SessionAction {
    SessionAction::SetParameter { target: resource("radio"), key: Key::parse(key).unwrap(), value }
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

/// T3's untimed two-zero SOB/EOB send is setup, not experiment traffic.
pub fn tx_experiment_sends(device: &FakeDevice, prefix: &str) -> Vec<String> {
    calls(device, prefix).into_iter().filter(|call| !call.contains(" at=- sob=true eob=true")).collect()
}

pub fn tx_clock_origin(manifest: &Manifest, nth: usize) -> i64 {
    manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/tx/s0").unwrap()).nth(nth).unwrap().origin.ticks_in(manifest.clocks.sample_clocks.iter().filter(|r| r.stream == ResourceId::parse("usrp/tx/s0").unwrap()).nth(nth).unwrap().root).unwrap()
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

/// The complex correlation of `wave` with `samples` at every offset: `Σ conj(w)·s`.
fn correlation(samples: &[(f32, f32)], wave: &[(f32, f32)]) -> Vec<(f32, f32)> {
    (0..samples.len().saturating_sub(wave.len()))
        .map(|at| {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for ((wr, wi), (sr, si)) in wave.iter().zip(&samples[at..]) {
                re += wr * sr + wi * si;
                im += wr * si - wi * sr;
            }
            (re, im)
        })
        .collect()
}

/// How far the correlation's magnitude must peak above its median, the capture's noise
/// floor, for `correlate` to call it the waveform (noise alone peaks about 3.5 times over
/// a few thousand offsets; the bench's loop peaked about 70 times).
pub const CORRELATION_PEAK_OVER_MEDIAN: f32 = 8.0;

/// Where `wave` first appears in `samples`: the peak of their complex correlation's
/// magnitude, since a cabled loop turns the waveform by its LOs' phase offset, accepted
/// when it stands `CORRELATION_PEAK_OVER_MEDIAN` times above the median. Not a threshold
/// on the loop's gain, which is the attenuator's and the front ends' (the bench's was
/// −36.4 dB, bench-results.md session 2).
pub fn correlate(samples: &[(f32, f32)], wave: &[(f32, f32)]) -> Option<usize> {
    let magnitudes: Vec<f32> = correlation(samples, wave).into_iter().map(|(re, im)| re.hypot(im)).collect();
    let mut sorted = magnitudes.clone();
    sorted.sort_by(f32::total_cmp);
    let median = *sorted.get(sorted.len() / 2)?;
    let (at, peak) = magnitudes.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    (*peak > CORRELATION_PEAK_OVER_MEDIAN * median).then_some(at)
}

/// The correlation with `wave` at `first` and every whole period after it, as (offset,
/// gain, phase °), the gain a fraction of the waveform's energy.
pub fn back_to_back(samples: &[(f32, f32)], wave: &[(f32, f32)], first: usize) -> Vec<(usize, f32, f32)> {
    let energy: f32 = wave.iter().map(|(re, im)| re * re + im * im).sum();
    (first..=samples.len().saturating_sub(wave.len()))
        .step_by(wave.len())
        .map(|at| {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for ((wr, wi), (sr, si)) in wave.iter().zip(&samples[at..]) {
                re += wr * sr + wi * si;
                im += wr * si - wi * sr;
            }
            (at, re.hypot(im) / energy, im.atan2(re).to_degrees())
        })
        .collect()
}

/// What a bench capture holds against `wave`: its RMS, the correlation's peak (offset,
/// magnitude and phase, the magnitude as a fraction of the waveform's energy: the loop's
/// amplitude gain) and how far it stands above the median (what `correlate` thresholds).
pub fn diagnose(what: &str, samples: &[(f32, f32)], wave: &[(f32, f32)]) {
    let energy: f32 = wave.iter().map(|(re, im)| re * re + im * im).sum();
    let rms = (samples.iter().map(|(re, im)| re * re + im * im).sum::<f32>() / samples.len().max(1) as f32).sqrt();
    let dots = correlation(samples, wave);
    let mut sorted: Vec<f32> = dots.iter().map(|(re, im)| re.hypot(*im)).collect();
    sorted.sort_by(f32::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or_default();
    let peak = dots.iter().enumerate().max_by(|a, b| a.1.0.hypot(a.1.1).total_cmp(&b.1.0.hypot(b.1.1)));
    println!(
        "{what}: {} samples, rms {rms:.5}; correlation peak {:?} (offset, gain, phase °); peak over median {:.1} (needs > {CORRELATION_PEAK_OVER_MEDIAN})",
        samples.len(),
        peak.map(|(at, (re, im))| (at, re.hypot(*im) / energy, im.atan2(*re).to_degrees())),
        peak.map_or(0.0, |(_, (re, im))| re.hypot(*im) / median)
    );
}

/// B3: `experiments::receive(1, 1e6, f, Some(10_000))` at the bench frequency `f` under the bench profile.
pub fn rehearse_receive_at_t0(device: Arc<dyn Device>) -> Manifest {
    let dir = TempDir::new();
    let manifest = captured(spec_run(&receive_spec(1, 1e6, bench_hz(&*device), Some(10_000)), &bench_profile(&*device, &dir, json!({}), json!({}), false), device, BTreeMap::new()));
    require_stream_timing(&manifest, "B3 receive at T0", false);
    assert!(matches!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} }), "{:?}", manifest.termination);
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.first.ticks_in(map.domain).unwrap(), 0, "the capture starts at receive sample 0");
    assert!(map.gaps.is_empty(), "{:?}", map.gaps);
    let first = section(&manifest, "timing").as_array().unwrap().iter().find(|r| r["what"] == "first_rx_block").cloned().unwrap();
    assert_eq!(first["index"], 0);
    let rate = section(&manifest, "applied").as_array().unwrap().iter().find(|r| r["key"] == "radio.rx.s0.sample_rate_hz").cloned().unwrap();
    assert_eq!(rate["claimed"], rate["read_back"]);
    manifest
}

/// B4: a capture of 10 000 samples at receive sample 50 000 (v3 behaviour 2).
pub fn rehearse_capture_at_a_sample_index(device: Arc<dyn Device>) -> Manifest {
    let dir = TempDir::new();
    let mut spec = receive_spec(1, 1e6, bench_hz(&*device), None);
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": 50_000 },
        "action": { "kind": "update_parameter", "target": serde_json::to_value(output("rec")).unwrap(),
                    "key": "sink.capture_samples", "value": 10_000 }
    }]);
    let manifest = captured(spec_run(&spec, &bench_profile(&*device, &dir, json!({}), json!({}), false), device, BTreeMap::new()));
    require_stream_timing(&manifest, "B4 capture at an index", false);
    let map = &capture_of(&manifest, "rec").continuity[0];
    assert_eq!(map.first.ticks_in(map.domain).unwrap(), 50_000);
    assert_eq!(map.end.ticks_in(map.domain).unwrap(), 60_000);
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
            "action": { "kind": "update_parameter", "target": serde_json::to_value(output("rec")).unwrap(),
                        "key": "sink.capture_samples", "value": n }
        }));
    };
    let dir = TempDir::new();
    let mut burst = with_burst(with_tx(receive_spec(1, 1e6, bench_hz(&*device), None), 1e6), &waveform, false, "drop_and_flag", 10_000);
    capture(&mut burst, 10_000, 5_000);
    let inputs = BTreeMap::from([(waveform.hash.clone(), bytes.clone())]);
    let heard = captured(spec_run(&burst, &bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false), device.clone(), inputs.clone()));
    require_stream_timing(&heard, "B6 single-burst loopback", true);
    let samples = read_capture(&capture_of(&heard, "rec"), 1).remove(0);
    if !exact {
        diagnose("B6 burst", &samples, &wave);
    }
    let at = if exact { exactly(&samples, &wave) } else { correlate(&samples, &wave) }.unwrap_or_else(|| {
        panic!("the burst's correlation peak: async {} TIME_ERROR {:?}", section(&heard, "async"), events_of(&heard, "radio.TIME_ERROR"))
    });
    if exact {
        assert_eq!(at, 0, "the burst at T0 + 10 000 samples is received at sample 10 000");
    }
    println!("B6: transmit-to-receive delay {at} samples");
    let dir = TempDir::new();
    let mut repeat = with_burst(with_tx(receive_spec(1, 1e6, bench_hz(&*device), None), 1e6), &waveform, true, "send_asap_and_flag", 1_000);
    capture(&mut repeat, 1_000, 4_000);
    let looped = captured(spec_run(&repeat, &bench_profile(&*device, &dir, json!({}), bench_envelope(&*device), false), device, inputs));
    require_stream_timing(&looped, "B6 repeat loopback", true);
    let samples = read_capture(&capture_of(&looped, "rec"), 1).remove(0);
    if !exact {
        diagnose("B6 repeat", &samples, &wave);
    }
    let start = if exact { 0 } else { correlate(&samples, &wave).expect("the repeat is heard") };
    for (i, sample) in samples[start..samples.len() - wave.len()].iter().enumerate() {
        if exact {
            assert_eq!(*sample, wave[i % wave.len()], "sample {i}: async {} TIME_ERROR {:?}", section(&looped, "async"), events_of(&looped, "radio.TIME_ERROR"));
        }
    }
    if !exact {
        // Back to back through a cable: every period after the first peak correlates with
        // the waveform at the same gain and phase (a gap or a slip would break one).
        let periods = back_to_back(&samples, &wave, start % wave.len());
        let first = periods[0];
        println!("B6 repeat: {} whole periods from sample {} (offset, gain, phase °): {periods:?}", periods.len(), start % wave.len());
        for (at, gain, phase) in &periods {
            assert!(*gain > 0.5 * first.1 && (phase - first.2 + 540.0).rem_euclid(360.0) - 180.0 < 20.0 && (phase - first.2 + 540.0).rem_euclid(360.0) - 180.0 > -20.0, "the repeat is not back to back at sample {at}: {periods:?}");
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
    let envelope = bench_envelope(&*device);
    let mut run = session(&bench_profile(&*device, &dir, json!({}), envelope, true), device);
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.tx.c0.stream", Value::from(0)), None).unwrap()));
    past_tx_origin(&mut run);
    let wave = pn(1_000);
    let (bytes, _) = waveform_of(&wave);
    assert!(admitted(&run.submit(verb("start_repeat", resource("radio/tx/s0"), None, &[]), Some(&bytes)).unwrap()));
    let at = after_ticks(&run, ms(50));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(at), &[("sink.capture_samples", Value::from(5_000))]), None).unwrap()));
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let refused = run.submit(set("radio.tx.c0.frequency_hz", Value::num(hz + 100e6).unwrap()), None).unwrap();
    assert!(matches!(&refused.outcome, Outcome::Rejected { violations } if violations.iter().any(|v| v.check.as_str() == "radio.rf_envelope")), "{refused:?}");
    if !exact {
        println!("B7 refused: {:?}", refused.outcome);
    }
    let manifest = run.finish();
    require_stream_timing(&manifest, "B7 session loopback", true);
    let samples = read_capture(&capture_of(&manifest, "rec"), 1).remove(0);
    if !exact {
        diagnose("B7 capture", &samples, &wave);
        println!("B7 capture asked at {at:?}; artifact {:?}", capture_of(&manifest, "rec").continuity);
        println!("B7 sample clocks: {:?}", manifest.clocks.sample_clocks);
    }
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
    TimePoint::new(now.domain(), now.ticks_in(now.domain()).unwrap() + ticks)
}

/// The transmit sample `lead` root ticks from now, on the running transmit clock.
pub fn tx_at(run: &RunHandle, lead: i64) -> TimePoint {
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx/s0") && r.ended_at.is_none()).unwrap();
    let n = clock.root_ticks_per_tick.num() as i64;
    TimePoint::new(clock.domain, (run.now().ticks_in(clock.root).unwrap() + lead - clock.origin.ticks_in(clock.root).unwrap() + n - 1).div_euclid(n))
}

/// The phase of the one 1 ms burst of `wave` in the `rec` capture of `manifest`, in degrees, or why none was read (INCONCLUSIVE):
/// a gap, a late burst, a missing or mistimed waveform, a weak loop or a phase that moves across the burst. The capture is
/// 10 000 samples and the burst begins 5 000 samples into it, the loop's delay (35 to 44 samples, B6) later, give or take the instants' rounding.
pub fn ma1_phase(manifest: &Manifest, wave: &[(f32, f32)]) -> Result<f64, String> {
    let artifact = capture_of(manifest, "rec");
    if artifact.continuity.len() != 1 { return Err(format!("the capture has {} stretches, not one (a gap)", artifact.continuity.len())); }
    if !events_of(manifest, "radio.TIME_ERROR").is_empty() { return Err("the burst was late (TIME_ERROR)".to_owned()); }
    let samples = read_capture(&artifact, 1).remove(0);
    if samples.len() < 10_000 { return Err(format!("the capture holds {} of 10 000 samples", samples.len())); }
    let offset = correlate(&samples, wave).ok_or("no waveform peak in the capture (the loopback is weak or the burst is missing)")?;
    if !(4_950..5_150).contains(&offset) { return Err(format!("the waveform is at sample {offset}, not at the burst's time (4 950 to 5 150)")); }
    let phase = tone::burst_phase(&samples, wave, offset).ok_or("the capture does not hold the whole burst")?;
    tone::phase_verdict(&phase)
}

/// One run of the MA-1 probe: the receive and the transmit frequency are changed to the bench frequency plus 300 kHz (the LO
/// is rewritten), then one 1 ms burst is sent and captured, and its phase against the known waveform is returned (`ma1_phase`).
/// `timed_after_start` false: a Session sets both before T0, where the receive one is applied untimed (UR-24, D9a) and the
/// transmit one by the enable's configuration. True: a Spec Run sets both again at one instant, 100 ms in, while the
/// receive stream runs (the receive one timed, as D9a's covering rule gives it; the transmit one between bursts, away from
/// F3′), and its burst begins 10 ms after it. The two measure one thing, the burst's phase, by two harnesses, because a Session
/// action has no `at` of its own.
pub fn ma1_run(device: Arc<dyn Device>, timed_after_start: bool, wave: &[(f32, f32)], bytes: &[u8]) -> Result<f64, String> {
    let dir = TempDir::new();
    let hz = bench_hz(&*device);
    let f_new = hz + 300e3;
    assert!(SAFE_TX_HZ.contains(&f_new), "bench.md RF safety: {f_new} Hz is outside SAFE_TX_HZ (the probe is for slot A's OBX or UBX at 1 GHz)");
    let envelope = bench_envelope(&*device);
    let manifest = if timed_after_start {
        let (_, waveform) = waveform_of(wave);
        let mut spec = with_burst(with_tx(receive_spec(1, 1e6, hz, None), 1e6), &waveform, false, "send_asap_and_flag", 110_000);
        let at = |offset: i64| json!({ "clock": "radio", "offset_ticks": offset });
        let update = |direction: &str| json!({ "at": at(100_000), "action": { "kind": "update_parameter", "target": serde_json::to_value(resource("radio")).unwrap(),
            "key": format!("radio.{direction}.c0.frequency_hz"), "value": f_new } });
        spec["schedule"].as_array_mut().expect("the burst's schedule").extend([update("rx"), update("tx"), json!({ "at": at(105_000), "action": {
            "kind": "update_parameter", "target": serde_json::to_value(output("rec")).unwrap(), "key": "sink.capture_samples", "value": 10_000 } })]);
        let inputs = BTreeMap::from([(waveform.hash.clone(), bytes.to_vec())]);
        captured(spec_run(&spec, &bench_profile(&*device, &dir, json!({}), envelope, false), device, inputs))
    } else {
        let mut run = session(&bench_profile(&*device, &dir, json!({}), envelope, true), device);
        for direction in ["rx", "tx"] {
            let entry = run.submit(set(&format!("radio.{direction}.c0.frequency_hz"), Value::num(f_new).unwrap()), None).unwrap();
            assert!(admitted(&entry), "{entry:?}");
        }
        past_t0(&mut run, ms(1));
        assert!(admitted(&run.submit(set("radio.tx.c0.stream", Value::from(0)), None).unwrap()));
        past_tx_origin(&mut run);
        // Both instants are read before either is submitted, so that a slow submit does not move the burst in the capture.
        let (at, capture_at) = (tx_at(&run, ms(30)), after_ticks(&run, ms(25)));
        assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(at), &[]), Some(bytes)).unwrap()));
        assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", Value::from(10_000))]), None).unwrap()));
        let horizon = after_ticks(&run, ms(3_000));
        let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
        run.finish()
    };
    ma1_phase(&manifest, wave)
}
