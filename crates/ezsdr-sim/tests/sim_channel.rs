use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{AdmissionCheck, CheckStage};
use ezsdr_kernel::id::RunId;
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::time::Rational;
use ezsdr_sim::channel::{
    self, ChannelCheck, ChannelSpec, Coupling, Medium, Radiated, RootInstant, Transmitter,
};
use ezsdr_sim::SimRng;
use serde_json::{json, Value as JsonValue};

fn env(value: JsonValue) -> BTreeMap<Namespace, JsonValue> {
    BTreeMap::from([(Namespace::parse("sim.channel").unwrap(), value)])
}

fn id(name: &str) -> Ident {
    Ident::parse(name).unwrap()
}

fn coupling(tx: &str, tx_channel: u16, rx: &str, rx_channel: u16, gain_db: f64, delay_ns: u64) -> Coupling {
    Coupling { tx: id(tx), tx_channel, rx: id(rx), rx_channel, gain_db, delay_ns }
}

/// A transmitter whose channel `c` radiates `(t + c, −t)` at integer root tick `t` held
/// until the next tick, on 1 GHz, from tick 0 on; it records every instant it is asked.
struct Ramp {
    frequency_hz: f64,
    asked: Mutex<Vec<(u16, i128, i128)>>,
}

impl Ramp {
    fn new(frequency_hz: f64) -> Arc<Ramp> {
        Arc::new(Ramp { frequency_hz, asked: Mutex::new(Vec::new()) })
    }
}

impl Transmitter for Ramp {
    fn radiated(&self, channel: u16, at: RootInstant) -> Option<Radiated> {
        self.asked.lock().unwrap().push((channel, at.num, at.den));
        let tick = at.sample_at_or_before(0, Rational::new(1, 1).unwrap());
        (tick >= 0).then(|| Radiated {
            re: tick as f64 + f64::from(channel),
            im: -(tick as f64),
            frequency_hz: self.frequency_hz,
        })
    }
}

struct Silent;

impl Transmitter for Silent {
    fn radiated(&self, _channel: u16, _at: RootInstant) -> Option<Radiated> {
        None
    }
}

/// A transmitter whose channel `c` radiates the fixed `(re, im)` of that channel on
/// 1 GHz, so a test can choose the magnitudes a sum adds up.
struct Fixed(Vec<(f64, f64)>);

impl Transmitter for Fixed {
    fn radiated(&self, channel: u16, _at: RootInstant) -> Option<Radiated> {
        let (re, im) = self.0.get(usize::from(channel)).copied()?;
        Some(Radiated { re, im, frequency_hz: 1.0e9 })
    }
}

fn run() -> RunId {
    RunId::from_string("ch-run".to_owned())
}

#[test]
fn ch_01_reader() {
    assert_eq!(channel::read(&BTreeMap::new()).unwrap(), None);
    assert_eq!(channel::read(&env(json!({}))).unwrap(), Some(ChannelSpec::default()));
    let spec = channel::read(&env(json!({
        "couplings": [
            { "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 1, "gain_db": -6.0, "delay_ns": 1000 },
            { "tx": "b", "tx_channel": 1, "rx": "b", "rx_channel": 0, "gain_db": -40 }
        ],
        "noise_dbfs": { "b": -60.0 }
    })))
    .unwrap()
    .unwrap();
    assert_eq!(spec.couplings[0], coupling("a", 0, "b", 1, -6.0, 1000));
    assert_eq!(spec.couplings[1], coupling("b", 1, "b", 0, -40.0, 0));
    assert_eq!(spec.noise_dbfs[&id("b")], -60.0);
    assert!(channel::read(&env(json!({ "couplings": [], "noise_dbfs": {} }))).unwrap().is_some());
    for bad in [
        json!([]),
        json!({ "couplings": [], "extra": 1 }),
        json!({ "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0 }] }),
        json!({ "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0, "spare": 1 }] }),
        json!({ "couplings": [{ "tx": "a", "tx_channel": -1, "rx": "b", "rx_channel": 0, "gain_db": 0 }] }),
        json!({ "couplings": [{ "tx": "a", "tx_channel": 70000, "rx": "b", "rx_channel": 0, "gain_db": 0 }] }),
        json!({ "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0, "delay_ns": -1 }] }),
        json!({ "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0, "delay_ns": (1u64 << 62) + 1 }] }),
        json!({ "couplings": [{ "tx": "A b", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0 }] }),
        json!({ "noise_dbfs": { "b": "loud" } }),
        json!({ "couplings": [["a", 0, "b", 0, 0.0, 0]] }),
        json!({ "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 400.5 }] }),
        json!({ "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": -400.5 }] }),
        json!({ "noise_dbfs": { "b": 100.5 } }),
        json!({ "noise_dbfs": { "b": -400.5 } }),
    ] {
        let result = channel::read(&env(bad.clone()));
        assert!(result.is_err(), "{bad} must be refused");
        assert!(result.unwrap_err().starts_with("CH-1: "));
    }
    let edge = channel::read(&env(json!({
        "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0, "delay_ns": 1u64 << 62 }]
    })))
    .unwrap()
    .unwrap();
    assert_eq!(edge.couplings[0].delay_ns, 1u64 << 62);
}

#[test]
fn ch_02_check() {
    let effective = BTreeMap::from([
        (id("a"), BTreeMap::<Key, Value>::new()),
        (id("b"), BTreeMap::<Key, Value>::new()),
    ]);
    let empty = BTreeMap::new();
    let valid = json!({
        "couplings": [{ "tx": "a", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0 }],
        "noise_dbfs": { "b": -50 }
    });
    assert!(ChannelCheck.check(&valid, &effective, &empty, CheckStage::Validate).is_empty());
    assert_eq!(ChannelCheck.stages(), &[CheckStage::Validate]);
    assert_eq!(ChannelCheck.section(), &Namespace::parse("sim.channel").unwrap());

    let unknown = json!({
        "couplings": [
            { "tx": "x", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0 },
            { "tx": "a", "tx_channel": 0, "rx": "y", "rx_channel": 0, "gain_db": 0 },
            { "tx": "x", "tx_channel": 1, "rx": "y", "rx_channel": 1, "gain_db": 0 }
        ],
        "noise_dbfs": { "z": -50 }
    });
    let violations = ChannelCheck.check(&unknown, &effective, &empty, CheckStage::Validate);
    let reasons: Vec<&str> = violations.iter().map(|v| v.reason.as_str()).collect();
    assert_eq!(
        reasons,
        [
            "CH-2: x names no fragment of this Run",
            "CH-2: y names no fragment of this Run",
            "CH-2: z names no fragment of this Run",
        ]
    );
    assert!(violations.iter().all(|v| v.check == Namespace::parse("sim.channel").unwrap() && v.key.is_none()));

    let malformed = ChannelCheck.check(&json!({ "couplings": 3 }), &effective, &empty, CheckStage::Validate);
    assert_eq!(malformed.len(), 1);
    assert!(malformed[0].reason.starts_with("CH-2: CH-1: "));

    // CH-2: the violations come in `Ident` order, not in the document's order. Here
    // the document names `z` before `y`, so document order and `Ident` order differ.
    let unordered = json!({
        "couplings": [
            { "tx": "z", "tx_channel": 0, "rx": "b", "rx_channel": 0, "gain_db": 0 },
            { "tx": "a", "tx_channel": 0, "rx": "y", "rx_channel": 0, "gain_db": 0 }
        ]
    });
    let unordered_violations = ChannelCheck.check(&unordered, &effective, &empty, CheckStage::Validate);
    let unordered_reasons: Vec<&str> = unordered_violations.iter().map(|v| v.reason.as_str()).collect();
    assert_eq!(
        unordered_reasons,
        ["CH-2: y names no fragment of this Run", "CH-2: z names no fragment of this Run"]
    );
}

#[test]
fn ch_03_root_instants_are_exact() {
    let third = Rational::new(1_000, 3).unwrap();
    let k7 = RootInstant::of_sample(10, third, 7);
    assert_eq!((k7.num, k7.den), (10 * 3 + 7_000, 3));
    assert_eq!(k7.sample_at_or_before(10, third), 7);
    assert_eq!(k7.minus_ticks(1).sample_at_or_before(10, third), 6);
    assert_eq!(RootInstant::tick(9).sample_at_or_before(10, third), -1);
    assert_eq!(RootInstant::tick(10).sample_at_or_before(10, third), 0);
    assert_eq!(RootInstant::tick(343).sample_at_or_before(10, third), 0);
    assert_eq!(RootInstant::tick(344).sample_at_or_before(10, third), 1);
    assert!(k7.is_at_or_after(2343));
    assert!(!k7.is_at_or_after(2344));
    assert!(RootInstant::tick(5).is_at_or_after(5));
}

#[test]
fn ch_03_a_sample_at_or_before_saturates_with_the_sign_of_the_instant() {
    // CH-3: the quotient does not fit an i64, so it saturates to i64::MIN or i64::MAX
    // according to the sign of the instant, not to i64::MAX either way.
    let unit = Rational::new(1, 1).unwrap();
    let over = RootInstant { num: i128::from(i64::MAX) + 1, den: 1 };
    let under = RootInstant { num: i128::from(i64::MIN) - 1, den: 1 };
    assert_eq!(over.sample_at_or_before(0, unit), i64::MAX);
    assert_eq!(under.sample_at_or_before(0, unit), i64::MIN);
    // One tick inside the range the quotient is exact, which is what shows the
    // saturation is a fallback and not a clamp: a mutation that always answered
    // i64::MAX would return i64::MAX here instead.
    assert_eq!(RootInstant { num: i128::from(i64::MAX) - 1, den: 1 }.sample_at_or_before(0, unit), i64::MAX - 1);
    assert_eq!(RootInstant { num: i128::from(i64::MIN) + 1, den: 1 }.sample_at_or_before(0, unit), i64::MIN + 1);
}

#[test]
fn ch_04_the_field_sums_paths_with_gain_delay_and_the_frequency_gate() {
    let spec = ChannelSpec {
        couplings: vec![
            coupling("a", 0, "b", 0, -20.0, 100),
            coupling("a", 1, "b", 0, 0.0, 0),
            coupling("c", 0, "b", 0, 0.0, 0),
            coupling("a", 0, "b", 1, 6.0, 0),
            coupling("d", 0, "b", 0, 0.0, 0),
        ],
        noise_dbfs: BTreeMap::new(),
    };
    let medium = Medium::new();
    let a = Ramp::new(1.0e9);
    let c = Ramp::new(2.0e9);
    // the next f64 above 1 GHz: the gate is exact equality (C4)
    let d = Ramp::new(f64::from_bits(1.0e9_f64.to_bits() + 1));
    medium.join(&run(), &id("a"), &spec, 0, 1_000_000_000, a.clone()).unwrap();
    medium.join(&run(), &id("b"), &spec, 0, 1_000_000_000, Arc::new(Silent)).unwrap();
    medium.join(&run(), &id("c"), &spec, 0, 1_000_000_000, c.clone()).unwrap();
    medium.join(&run(), &id("d"), &spec, 0, 1_000_000_000, d.clone()).unwrap();

    let (re, im) = medium.field(&id("b"), 0, 1.0e9, RootInstant::tick(1_000));
    let expected_re = 0.1 * 900.0 + (1_000.0 + 1.0);
    let expected_im = 0.1 * -900.0 + -1_000.0;
    assert!((re - expected_re).abs() < 1e-9, "{re} vs {expected_re}");
    assert!((im - expected_im).abs() < 1e-9, "{im} vs {expected_im}");
    assert!(c.asked.lock().unwrap().iter().any(|(ch, num, den)| *ch == 0 && *num == 1_000 && *den == 1));
    assert!(d.asked.lock().unwrap().iter().any(|(ch, num, den)| *ch == 0 && *num == 1_000 && *den == 1), "one f64 step off the receiver's frequency is asked, and not counted");

    let (re1, im1) = medium.field(&id("b"), 1, 1.0e9, RootInstant::tick(50));
    let six = 10f64.powf(6.0 / 20.0);
    assert!((re1 - six * 50.0).abs() < 1e-9 && (im1 + six * 50.0).abs() < 1e-9);

    let (re_tuned, im_tuned) = medium.field(&id("b"), 0, 2.0e9, RootInstant::tick(1_000));
    assert_eq!((re_tuned, im_tuned), (1_000.0, -1_000.0));

    let (before, _) = medium.field(&id("b"), 0, 1.0e9, RootInstant::tick(50));
    assert_eq!(before, 50.0 + 1.0);
    assert_eq!(medium.field(&id("a"), 0, 1.0e9, RootInstant::tick(5)), (0.0, 0.0));
}

#[test]
fn ch_04_a_delay_rounds_up_to_a_root_tick() {
    let spec = ChannelSpec { couplings: vec![coupling("a", 0, "a", 0, 0.0, 1)], noise_dbfs: BTreeMap::new() };
    let medium = Medium::new();
    let a = Ramp::new(1.0e9);
    medium.join(&run(), &id("a"), &spec, 0, 250_000_000, a.clone()).unwrap();
    let _ = medium.field(&id("a"), 0, 1.0e9, RootInstant::tick(10));
    assert_eq!(a.asked.lock().unwrap().as_slice(), &[(0, 9, 1)]);
}

#[test]
fn ch_04_b_two_couplings_with_the_same_ends_are_two_paths() {
    // CH-4: a two-tap channel costs nothing to allow, so both couplings count. A join
    // that keyed the paths by their ends, or skipped a duplicate, would read only one.
    let spec = ChannelSpec {
        couplings: vec![
            coupling("a", 0, "b", 0, -20.0, 100),
            coupling("a", 0, "b", 0, 0.0, 0),
        ],
        noise_dbfs: BTreeMap::new(),
    };
    let medium = Medium::new();
    let a = Ramp::new(1.0e9);
    medium.join(&run(), &id("a"), &spec, 0, 1_000_000_000, a.clone()).unwrap();
    medium.join(&run(), &id("b"), &spec, 0, 1_000_000_000, Arc::new(Silent)).unwrap();

    let (re, im) = medium.field(&id("b"), 0, 1.0e9, RootInstant::tick(1_000));
    let (expected_re, expected_im) = (0.1 * 900.0 + 1_000.0, 0.1 * -900.0 + -1_000.0);
    assert!((re - expected_re).abs() < 1e-9, "{re} vs {expected_re}");
    assert!((im - expected_im).abs() < 1e-9, "{im} vs {expected_im}");
}

#[test]
fn ch_04_c_the_field_sums_the_paths_in_document_order() {
    // CH-4: "The sum is taken in `f64`, in document order." That is not the same answer as
    // every other order. The couplings are written in the order tx channel 2, 0, 1, and
    // `Fixed` gives channel c the term TERMS[c] = [1e16, 1, −1e16], so:
    //   document order (2, 0, 1):  −1e16 + 1e16 + 1  =  1.0   (the large terms cancel first)
    //   sorted by ends  (0, 1, 2):   1e16 + 1 + −1e16  =  0.0   (the small term is absorbed)
    //   reverse         (1, 0, 2):   1 + 1e16 + −1e16  =  0.0
    // So the field is 1.0 exactly in document order and 0.0 in the other two, which kills a
    // join that iterated the paths in ends order or in reverse, and kills any reordering
    // that moves the small term behind a large one.
    let spec = ChannelSpec {
        couplings: vec![
            coupling("a", 2, "b", 0, 0.0, 0),
            coupling("a", 0, "b", 0, 0.0, 0),
            coupling("a", 1, "b", 0, 0.0, 0),
        ],
        noise_dbfs: BTreeMap::new(),
    };
    let medium = Medium::new();
    let a = Fixed(vec![(1.0e16, 0.0), (1.0, 0.0), (-1.0e16, 0.0)]);
    medium.join(&run(), &id("a"), &spec, 0, 1_000_000_000, Arc::new(a)).unwrap();
    medium.join(&run(), &id("b"), &spec, 0, 1_000_000_000, Arc::new(Silent)).unwrap();

    let (re, im) = medium.field(&id("b"), 0, 1.0e9, RootInstant::tick(0));
    assert_eq!((re, im), (1.0, 0.0), "document order: -1e16 + 1e16 + 1");
    // the two orders it must differ from, spelled out so a reader sees why 1.0 discriminates
    assert_eq!((1.0e16 + 1.0) + -1.0e16, 0.0, "ends order absorbs the small term");
    assert_eq!((1.0 + 1.0e16) + -1.0e16, 0.0, "reverse order absorbs the small term");
}

#[test]
fn ch_05_noise_has_its_power_its_seed_and_one_stream_per_receive_channel() {
    let spec = ChannelSpec { couplings: vec![], noise_dbfs: BTreeMap::from([(id("b"), -20.0)]) };
    let draw = |seed: u64, channels: &[u16], count: usize| {
        let medium = Medium::new();
        medium.join(&run(), &id("b"), &spec, seed, 1_000_000_000, Arc::new(Silent)).unwrap();
        let mut out: BTreeMap<u16, Vec<(f64, f64)>> = BTreeMap::new();
        for _ in 0..count {
            for channel in channels {
                out.entry(*channel).or_default().push(medium.field(&id("b"), *channel, 1.0e9, RootInstant::tick(0)));
            }
        }
        out
    };
    let long = draw(7, &[0], 100_000);
    let power: f64 = long[&0].iter().map(|(re, im)| re * re + im * im).sum::<f64>() / 100_000.0;
    assert!((power - 0.01).abs() < 0.0005, "power {power}");
    let mean_re: f64 = long[&0].iter().map(|(re, _)| re).sum::<f64>() / 100_000.0;
    assert!(mean_re.abs() < 0.002, "mean {mean_re}");

    assert_eq!(draw(7, &[0], 50), draw(7, &[0], 50));
    assert_ne!(draw(7, &[0], 50)[&0], draw(8, &[0], 50)[&0]);
    assert_eq!(draw(7, &[0, 1], 50)[&0], draw(7, &[0], 50)[&0]);
    assert_ne!(draw(7, &[0, 1], 50)[&0], draw(7, &[0, 1], 50)[&1]);

    let mut rng = SimRng::new(7, "sim.channel/b/0");
    let (g1, g2) = channel::gaussian_pair(&mut rng);
    let sigma = (10f64.powf(-20.0 / 10.0) / 2.0).sqrt();
    assert_eq!(draw(7, &[0], 1)[&0][0], (sigma * g1, sigma * g2));
}

#[test]
fn ch_05_gaussian_pair_follows_its_formula() {
    let mut rng = SimRng::new(0, "");
    let mut copy = rng.clone();
    let (g1, g2) = channel::gaussian_pair(&mut rng);
    let x1 = copy.next_u64();
    let x2 = copy.next_u64();
    let u1 = ((x1 >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
    let u2 = (x2 >> 11) as f64 / (1u64 << 53) as f64;
    let r = (-2.0 * u1.ln()).sqrt();
    assert_eq!(g1, r * (2.0 * std::f64::consts::PI * u2).cos());
    assert_eq!(g2, r * (2.0 * std::f64::consts::PI * u2).sin());
    assert_eq!(rng.next_u64(), copy.next_u64());
}

#[test]
fn ch_06_join_rules_and_missing_fragments() {
    let spec = ChannelSpec {
        couplings: vec![coupling("a", 0, "b", 0, 0.0, 0)],
        noise_dbfs: BTreeMap::from([(id("c"), -30.0)]),
    };
    let medium = Medium::new();
    assert!(medium.missing().is_empty());
    medium.join(&run(), &id("a"), &spec, 1, 1_000_000_000, Arc::new(Silent)).unwrap();
    assert_eq!(medium.missing(), vec![id("b"), id("c")]);
    let refusals = [
        medium.join(&run(), &id("a"), &spec, 1, 1_000_000_000, Arc::new(Silent)),
        medium.join(&RunId::from_string("other".to_owned()), &id("b"), &spec, 1, 1_000_000_000, Arc::new(Silent)),
        medium.join(&run(), &id("b"), &ChannelSpec::default(), 1, 1_000_000_000, Arc::new(Silent)),
        medium.join(&run(), &id("b"), &spec, 2, 1_000_000_000, Arc::new(Silent)),
        medium.join(&run(), &id("b"), &spec, 1, 500_000_000, Arc::new(Silent)),
    ];
    for refusal in refusals {
        assert!(refusal.unwrap_err().starts_with("CH-6: "));
    }
    medium.join(&run(), &id("b"), &spec, 1, 1_000_000_000, Arc::new(Silent)).unwrap();
    medium.join(&run(), &id("c"), &spec, 1, 1_000_000_000, Arc::new(Silent)).unwrap();
    assert!(medium.missing().is_empty());
    assert!(Medium::new().join(&run(), &id("a"), &spec, 1, 0, Arc::new(Silent)).unwrap_err().starts_with("CH-6: "));
    let far = ChannelSpec { couplings: vec![coupling("a", 0, "b", 0, 0.0, 1 << 62)], noise_dbfs: BTreeMap::new() };
    assert_eq!(
        Medium::new().join(&run(), &id("a"), &far, 1, 2_000_000_000, Arc::new(Silent)).unwrap_err(),
        "CH-6: a delay overflows the root",
        "2^62 ns at a 2 GHz root is 2^63 root ticks"
    );
    assert!(Medium::new().join(&run(), &id("a"), &far, 1, 1_000_000_000, Arc::new(Silent)).is_ok());

    // CH-6: "A refusal changes nothing." The same medium is reused after the refused
    // first join above, so a join that recorded the Run or the document before it
    // validated the paths would refuse the good join that follows as a later one.
    let reused = Medium::new();
    assert_eq!(
        reused.join(&run(), &id("a"), &far, 1, 2_000_000_000, Arc::new(Silent)).unwrap_err(),
        "CH-6: a delay overflows the root"
    );
    assert!(reused.missing().is_empty(), "a refused first join leaves the medium unjoined");
    reused.join(&run(), &id("a"), &spec, 1, 1_000_000_000, Arc::new(Silent)).unwrap();
    assert_eq!(reused.missing(), vec![id("b"), id("c")], "the refused document was not adopted");
}
