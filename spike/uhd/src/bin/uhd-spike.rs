//! Lab driver for the spike. Every Run uses an acceptance-crate Spec unchanged except
//! for frequency and gain; only the BindingProfile differs from the Mock tests.
//!
//!   uhd-spike find    [ARGS]
//!   uhd-spike probe   ARGS
//!   uhd-spike rx      ARGS [--samples N] [--offset N]            (M1, §61 v3 behaviour 2)
//!   uhd-spike overflow ARGS [--stall-after-ms 500] [--stall-ms 300] (M2, §58 #6)
//!   uhd-spike txrx    ARGS [--offset N] [--burst N]               (M3, §61 v3 behaviour 3)
//!   uhd-spike repeat  ARGS [--seconds S]                          (M5, §61 v3 behaviour 1)
//!   uhd-spike leads   ARGS                                        (min lead sweep)
//!   uhd-spike session-rx ARGS [--at 12345] [--samples 100]         (Session capture at a sample index)
//!   uhd-spike loopback ARGS [--wait-ms 20] [--samples 5000]       (§57: tx.repeat(x); rx.capture(N))
//!
//! Common options: --rate 1e6 --freq 1e9 --gain 0 --tx-gain 0 --grid x310-like|ideal
//! --lead-ms 500 --rf-path cabled|over_the_air --clock internal --time internal --out DIR
//! ARGS is a UHD device string (`addr=192.168.40.2`, `type=usrp2`) or `fake`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ezsdr_acceptance::{experiments, rig};
use ezsdr_kernel::coordinator::{self, RunHandleError};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::time::TimePoint;
use ezsdr_uhd_spike::device::{self, Device, Dir};
use ezsdr_kernel::coordinator::RunHandle;
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::session::SessionAction;
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value as V};
use ezsdr_uhd_spike::{ProfileOpts, UhdDevice, assemble, open_devices, profile, session_profile};
use serde_json::{Value, json};

struct Opts {
    args: String,
    kv: BTreeMap<String, String>,
}

impl Opts {
    fn f(&self, k: &str, d: f64) -> f64 {
        self.kv.get(k).map_or(d, |v| v.parse().unwrap_or_else(|_| panic!("--{k} {v}: not a number")))
    }
    fn i(&self, k: &str, d: i64) -> i64 {
        self.f(k, d as f64) as i64
    }
    fn s<'a>(&'a self, k: &str, d: &'a str) -> &'a str {
        self.kv.get(k).map_or(d, String::as_str)
    }
}

fn parse(mut it: impl Iterator<Item = String>) -> Opts {
    let mut args = String::new();
    let mut kv = BTreeMap::new();
    while let Some(a) = it.next() {
        if let Some(k) = a.strip_prefix("--") {
            kv.insert(k.to_owned(), it.next().unwrap_or_else(|| panic!("--{k} needs a value")));
        } else {
            args = a;
        }
    }
    Opts { args, kv }
}

fn main() {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_default();
    let o = parse(it);
    match cmd.as_str() {
        "find" => match device::find(&o.args) {
            Ok(v) if v.is_empty() => println!("no UHD devices found for {:?}", o.args),
            Ok(v) => v.iter().for_each(|d| println!("{d}")),
            Err(e) => println!("find failed: {e}"),
        },
        "probe" => probe(&o),
        "rx" => rx(&o),
        "overflow" => overflow(&o),
        "txrx" => txrx(&o),
        "repeat" => repeat(&o),
        "leads" => leads(&o),
        "session-rx" => session_rx(&o),
        "loopback" => loopback(&o),
        _ => {
            eprintln!("usage: uhd-spike find|probe|rx|overflow|txrx|repeat|leads ARGS [--opt value]…  (see src/bin/uhd-spike.rs)");
            std::process::exit(2);
        }
    }
}

fn probe(o: &Opts) {
    let dev = UhdDevice::open(&o.args).unwrap_or_else(|e| panic!("open {:?}: {e}", o.args));
    println!("{}", dev.describe()["pp_string"].as_str().unwrap_or(""));
    println!("master clock rate: {} Hz", dev.tick_rate());
    let t1 = dev.time_now().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let t2 = dev.time_now().unwrap();
    println!("time advances {} ticks in 100 ms host time (expect ~{})", t2 - t1, dev.tick_rate() / 10);
    for rate in [1e6, 5e6, 19.5e6, 25e6] {
        let a = dev.apply(Dir::Rx, 0, &device::Settings { rate: Some(rate), freq: Some(o.f("freq", 1e9)), gain: Some(o.f("gain", 0.0)), antenna: None }, None);
        println!("rx rate {rate} → {a:?}");
    }
}

fn out_dir(o: &Opts, what: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let dir = PathBuf::from(o.s("out", "out")).join(format!("{what}-{stamp}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Sets frequency and gains on every radio resource of an acceptance Spec.
fn tune(spec: &mut Value, o: &Opts) {
    let req = &mut spec["resources"]["radio"]["requires"];
    let eq = |v: f64| json!({ "kind": "eq", "value": v });
    let tx = req.get("radio.tx.channels").is_some();
    req["radio.rx.frequency_hz"] = eq(o.f("freq", 1e9));
    req["radio.rx.gain_db"] = eq(o.f("gain", 0.0));
    if tx {
        req["radio.tx.frequency_hz"] = eq(o.f("freq", 1e9));
        req["radio.tx.gain_db"] = eq(o.f("tx-gain", 0.0));
    }
}

fn run(o: &Opts, what: &str, spec: &Value, radio: Value, inputs: BTreeMap<ContentHash, Vec<u8>>, seconds: f64) -> (Manifest, PathBuf) {
    let dir = out_dir(o, what);
    let po = ProfileOpts {
        args: &o.args,
        radio: {
            let mut r = json!({ "grid": o.s("grid", "x310-like") });
            r.as_object_mut().unwrap().extend(radio.as_object().cloned().unwrap_or_default());
            r
        },
        clock_source: o.s("clock", "internal"),
        time_source: o.s("time", "internal"),
        rf_path: o.s("rf-path", "cabled"),
        start_lead_ns: (o.f("lead-ms", 500.0) * 1e6) as u64,
    };
    let doc = profile(&po, &dir);
    std::fs::write(dir.join("spec.json"), serde_json::to_string_pretty(spec).unwrap()).unwrap();
    std::fs::write(dir.join("profile.json"), serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let binding = ezsdr_kernel::binding::BindingProfile::from_json(&doc).unwrap();
    let devices = open_devices(&binding).unwrap_or_else(|e| panic!("open device: {e}"));
    let assembly = assemble(&doc, inputs, &devices).unwrap_or_else(|e| panic!("assemble: {e}"));
    let started = std::time::Instant::now();
    let mut handle = coordinator::start_spec_run(spec, &doc, assembly).expect("Spec parses");
    if let Some(t0) = handle.start_instant() {
        let rate = devices.values().next().unwrap().tick_rate() as f64;
        let horizon = TimePoint::new(t0.domain, t0.ticks + (seconds * rate) as i64);
        match handle.run_until_end(horizon) {
            Ok(()) | Err(RunHandleError::Ended { .. }) => {}
            Err(e) => println!("run_until_end: {e}"),
        }
    }
    let m = handle.finish();
    println!("wall time {:.2} s", started.elapsed().as_secs_f64());
    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&m).unwrap()).unwrap();
    summary(&m, &dir);
    (m, dir)
}

fn summary(m: &Manifest, dir: &Path) {
    let v = serde_json::to_value(m).unwrap();
    println!("── {}", dir.display());
    println!("termination: {}", v["termination"]["reason"]);
    if let Some(f) = v["sections"].get("ezsdr.failure") {
        println!("FAILURE: {f}");
    }
    for c in &v["termination"]["cleanup_failures"].as_array().cloned().unwrap_or_default() {
        println!("cleanup failure: {c}");
    }
    println!("class: {}  fidelity: {}", v["run"]["execution_class"], v["run"]["fidelity"]);
    for (name, body) in v["sections"].as_object().unwrap() {
        if name.starts_with("ezsdr.radio.uhd.") && !name.ends_with(".device") {
            println!("{name}: {}", serde_json::to_string(body).unwrap());
        }
    }
    for e in &m.events.delivered {
        println!("event {} @{} {}: {}", e.kind, e.time.ticks, e.source.path, e.payload);
    }
    for a in &m.artifacts {
        println!("artifact {} {} ({} bytes, partial {})", a.id, a.uri, a.size_bytes, a.partial);
        for c in &a.continuity {
            println!("  continuity first {} end {} valid {:?} gaps {}", c.first.ticks, c.end.ticks,
                c.valid.iter().map(|ch| ch.iter().map(|r| (r.start.ticks, r.len)).collect::<Vec<_>>()).collect::<Vec<_>>(),
                serde_json::to_string(&c.gaps).unwrap());
        }
    }
}

fn capture(m: &Manifest) -> Option<Vec<(f32, f32)>> {
    let a = m.artifacts.iter().find(|a| a.kind.as_str() == "sink.capture")?;
    Some(rig::read_capture(a, 1).remove(0))
}

fn power_db(x: &[(f32, f32)]) -> f64 {
    let p = x.iter().map(|(r, i)| (*r as f64).powi(2) + (*i as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    10.0 * p.max(1e-20).log10()
}

fn rx(o: &Opts) {
    let n = o.i("samples", 100_000);
    let mut spec = experiments::receive(1, o.f("rate", 1e6), 1e9, None);
    spec["schedule"] = json!([]);
    let mut spec = experiments::with_timed_capture(spec, n, o.i("offset", 10_000));
    tune(&mut spec, o);
    let secs = (o.i("offset", 10_000) + n) as f64 / o.f("rate", 1e6) + 0.2;
    let (m, _) = run(o, "rx", &spec, json!({}), BTreeMap::new(), secs);
    if let Some(x) = capture(&m) {
        println!("captured {} samples, mean power {:.1} dBFS", x.len(), power_db(&x));
    }
}

fn overflow(o: &Opts) {
    let rate = o.f("rate", 10e6);
    let mut spec = experiments::receive(1, rate, 1e9, None);
    spec["schedule"] = json!([]);
    let n = (rate * 1.5) as i64;
    let mut spec = experiments::with_timed_capture(spec, n, 0);
    tune(&mut spec, o);
    let radio = json!({ "rx_stall_after_ms": o.i("stall-after-ms", 1000), "rx_stall_ms": o.i("stall-ms", 300) });
    run(o, "overflow", &spec, radio, BTreeMap::new(), 2.0);
}

/// ±0.5 BPSK from a 15-bit LFSR: sharp autocorrelation for locating the burst.
fn pn(n: usize, amp: f32) -> Vec<(f32, f32)> {
    let mut s: u16 = 0x5a5a;
    (0..n)
        .map(|_| {
            let b = ((s >> 14) ^ (s >> 13)) & 1;
            s = ((s << 1) | b) & 0x7fff;
            (if b == 1 { amp } else { -amp }, 0.0)
        })
        .collect()
}

fn txrx(o: &Opts) {
    let rate = o.f("rate", 1e6);
    let burst = o.i("burst", 1_000) as usize;
    let offset = o.i("offset", 20_000);
    let wave = pn(burst, o.f("amp", 0.5) as f32);
    let (bytes, waveform) = experiments::waveform_of(&wave);
    // Capture from 1 000 samples before the burst, for twice its length plus margin.
    let cap = 2 * burst as i64 + 2_000;
    let mut spec = experiments::with_timed_capture(
        experiments::transmit(rate, &waveform, false, "drop_and_flag", offset, None),
        cap,
        offset - 1_000,
    );
    tune(&mut spec, o);
    let secs = (offset + cap) as f64 / rate + 0.2;
    let (m, _) = run(o, "txrx", &spec, json!({}), BTreeMap::from([(waveform.hash.clone(), bytes)]), secs);
    let Some(x) = capture(&m) else { return };
    // Correlate; the burst was sent at capture index 1 000.
    let mut best = (0usize, 0.0f64);
    let mut noise = 0.0;
    for lag in 0..x.len().saturating_sub(burst) {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, p) in wave.iter().enumerate() {
            re += x[lag + i].0 as f64 * p.0 as f64;
            im += x[lag + i].1 as f64 * p.0 as f64;
        }
        let mag = (re * re + im * im).sqrt();
        noise += mag;
        if mag > best.1 {
            best = (lag, mag);
        }
    }
    noise /= x.len().saturating_sub(burst).max(1) as f64;
    println!(
        "correlation peak at capture index {} → TX→RX delay {} samples ({:.1} ns); peak/mean {:.1}; capture power {:.1} dBFS",
        best.0,
        best.0 as i64 - 1_000,
        (best.0 as f64 - 1_000.0) / rate * 1e9,
        best.1 / noise.max(1e-12),
        power_db(&x)
    );
    println!("(x310-like claims radio.tx.path_delay_samples = 45, INFERRED; a cabled loopback measures it)");
}

fn repeat(o: &Opts) {
    let rate = o.f("rate", 1e6);
    let wave = pn(o.i("burst", 10_000) as usize, o.f("amp", 0.3) as f32);
    let (bytes, waveform) = experiments::waveform_of(&wave);
    let mut spec = experiments::transmit(rate, &waveform, true, "send_asap_and_flag", 10_000, None);
    tune(&mut spec, o);
    run(o, "repeat", &spec, json!({}), BTreeMap::from([(waveform.hash.clone(), bytes)]), o.f("seconds", 3.0));
}

/// Shrinks the start lead until RX misses T0 and TX misses its target, with the
/// Provider's own lead checks off, so the device's floor shows.
fn leads(o: &Opts) {
    let rate = o.f("rate", 1e6);
    let wave = pn(1_000, 0.3);
    let (bytes, waveform) = experiments::waveform_of(&wave);
    for lead_ms in [100.0, 20.0, 10.0, 5.0, 2.0, 1.0, 0.5, 0.2] {
        let mut spec = experiments::transmit(rate, &waveform, false, "send_asap_and_flag", 0, Some(1_000));
        tune(&mut spec, o);
        let mut kv = o.kv.clone();
        kv.insert("lead-ms".into(), lead_ms.to_string());
        let oo = Opts { args: o.args.clone(), kv };
        println!("===== start lead {lead_ms} ms =====");
        run(&oo, &format!("lead-{lead_ms}ms"), &spec, json!({ "min_lead_ns": 0, "start_margin_ns": 0 }),
            BTreeMap::from([(waveform.hash.clone(), bytes.clone())]), 0.2);
    }
}

/// Opens a Session on the device; returns the handle, T0 and the output directory.
fn connect(o: &Opts, what: &str) -> (RunHandle, TimePoint, PathBuf, f64) {
    let dir = out_dir(o, what);
    let po = ProfileOpts {
        args: &o.args,
        radio: json!({ "grid": o.s("grid", "x310-like") }),
        clock_source: o.s("clock", "internal"),
        time_source: o.s("time", "internal"),
        rf_path: o.s("rf-path", "cabled"),
        start_lead_ns: (o.f("lead-ms", 500.0) * 1e6) as u64,
    };
    let doc = session_profile(&po, &dir);
    std::fs::write(dir.join("profile.json"), serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let devices = open_devices(&ezsdr_kernel::binding::BindingProfile::from_json(&doc).unwrap()).unwrap_or_else(|e| panic!("open: {e}"));
    let rate = devices.values().next().unwrap().tick_rate() as f64;
    let run = coordinator::connect(&doc, assemble(&doc, BTreeMap::new(), &devices).unwrap(), ezsdr_kernel::run::Lease::attached())
        .unwrap_or_else(|e| panic!("connect: {e}"));
    let Some(t0) = run.start_instant() else {
        let m = run.finish();
        summary(&m, &dir);
        panic!("the Session did not start");
    };
    let mut run = run;
    // Past T0, so the receive stream is running before any Action.
    if let Err(e) = run.advance_to(TimePoint::new(t0.domain, t0.ticks + (0.01 * rate) as i64)) {
        println!("advance_to T0: {e}");
    }
    (run, t0, dir, rate)
}

fn advance(run: &mut RunHandle, rate: f64, seconds: f64) {
    let now = run.now();
    if let Err(e) = run.advance_to(TimePoint::new(now.domain, now.ticks + (seconds * rate) as i64)) {
        println!("advance_to: {e}");
    }
}

fn finish_session(run: RunHandle, dir: &Path) -> Manifest {
    let m = run.finish();
    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&m).unwrap()).unwrap();
    summary(&m, dir);
    for e in &m.action_log {
        println!("log #{} {:?}", e.seq, e.outcome);
    }
    m
}

fn session_rx(o: &Opts) {
    let (mut run, _t0, dir, rate) = connect(o, "session-rx");
    let rx = run.sample_clocks().into_iter().find(|r| r.stream.path.ends_with("/rx")).expect("rx clock");
    let at = o.i("at", 50_000);
    let n = o.i("samples", 100);
    let e = run.submit(
        SessionAction::Vocabulary {
            ns: Namespace::parse("sink").unwrap(),
            verb: Ident::parse("capture").unwrap(),
            target: ResourceId::parse("rec").unwrap(),
            at: Some(TimePoint::new(rx.domain, at)),
            params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), V::Int(n))]),
        },
        None,
    );
    println!("capture at rx sample {at}: {e:?}");
    advance(&mut run, rate, (at + n) as f64 / o.f("rate", 1e6) + 0.2);
    let m = finish_session(run, &dir);
    if let Some(a) = m.artifacts.iter().find(|a| a.kind.as_str() == "sink.capture") {
        println!("capture continuity starts at rx sample {} (asked {at})", a.continuity[0].valid[0][0].start.ticks);
    }
}

fn loopback(o: &Opts) {
    let (mut run, _t0, dir, rate) = connect(o, "loopback");
    let wave = pn(o.i("burst", 10_000) as usize, o.f("amp", 0.3) as f32);
    let (bytes, _) = experiments::waveform_of(&wave);
    let a = run.submit(SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse("radio.tx.channels").unwrap(), value: V::Int(1) }, None);
    println!("set tx channels: {a:?}");
    // The Mock needs no wait: `submit` steps it before returning. This Provider applies
    // the change on its own thread, so the next Action waits for it (finding K5).
    advance(&mut run, rate, o.f("wait-ms", 20.0) / 1e3);
    let b = run.submit(SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: ResourceId::parse("radio/tx").unwrap(), at: None, params: Default::default() }, Some(&bytes));
    println!("start_repeat: {b:?}");
    advance(&mut run, rate, 0.05);
    let n = o.i("samples", 50_000);
    let c = run.submit(SessionAction::Vocabulary { ns: Namespace::parse("sink").unwrap(), verb: Ident::parse("capture").unwrap(), target: ResourceId::parse("rec").unwrap(), at: None, params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), V::Int(n))]) }, None);
    println!("capture: {c:?}");
    advance(&mut run, rate, n as f64 / o.f("rate", 1e6) + 0.3);
    let m = finish_session(run, &dir);
    if let Some(x) = capture(&m) {
        println!("captured {} samples, {:.1} dBFS", x.len(), power_db(&x));
    }
}
