//! The SimulationChannel (spec 11): the `sim.channel` section, its admission check, the
//! exact instants the channel works in, and the shared medium the simulated radios of
//! one Run transmit into and receive from.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use ezsdr_kernel::binding::{AdmissionCheck, CheckStage, Violation};
use ezsdr_kernel::id::RunId;
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::time::Rational;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::SimRng;

/// The environment section carrying the SimulationChannel (CH-1).
pub const CHANNEL_SECTION: &str = "sim.channel";

/// The largest `delay_ns` a coupling may carry (CH-1).
pub const MAX_DELAY_NS: u64 = 1 << 62;

/// The `sim.channel` document: the coupling matrix and the receivers' noise (CH-1).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelSpec {
    /// One entry per path from a transmit channel to a receive channel; a fragment may
    /// be both ends, which is its self-interference (CH-1, CH-4).
    #[serde(default)]
    pub couplings: Vec<Coupling>,
    /// Receiver noise power in dB relative to full scale, in `−400..=100`, per receiving
    /// fragment, added to every receive channel of that fragment (CH-1, CH-5).
    #[serde(default)]
    pub noise_dbfs: BTreeMap<Ident, f64>,
}

/// One path of the coupling matrix (CH-1).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Coupling {
    /// The transmitting fragment (CH-1).
    pub tx: Ident,
    /// The transmit channel of `tx` (CH-1).
    pub tx_channel: u16,
    /// The receiving fragment (CH-1).
    pub rx: Ident,
    /// The receive channel of `rx` (CH-1).
    pub rx_channel: u16,
    /// The path's amplitude gain in dB, in `−400..=400`; negative is a loss (CH-1, CH-4).
    pub gain_db: f64,
    /// The path's delay in nanoseconds, in `0..=2^62` (CH-1, CH-4).
    #[serde(default)]
    pub delay_ns: u64,
}

static CHANNEL_NAMESPACE: OnceLock<Namespace> = OnceLock::new();

fn channel_namespace() -> &'static Namespace {
    CHANNEL_NAMESPACE.get_or_init(|| Namespace::parse(CHANNEL_SECTION).expect("a valid section name"))
}

/// Reads `sim.channel`: `None` when the section is absent, an error when it is not a
/// [`ChannelSpec`] of JSON objects, when a gain is outside `−400..=400` dB, a noise power
/// outside `−400..=100` dBFS or a delay above 2^62 (CH-1).
pub fn read(
    environment: &BTreeMap<Namespace, serde_json::Value>,
) -> Result<Option<ChannelSpec>, String> {
    let Some(value) = environment.get(channel_namespace()) else {
        return Ok(None);
    };
    // serde also reads a struct from a JSON array; the document is an object of objects.
    let objects = value.is_object()
        && value.get("couplings").and_then(serde_json::Value::as_array).is_none_or(|couplings| couplings.iter().all(serde_json::Value::is_object));
    if !objects {
        return Err("CH-1: sim.channel and each of its couplings must be JSON objects".to_owned());
    }
    let spec: ChannelSpec =
        serde_json::from_value(value.clone()).map_err(|error| format!("CH-1: {error}"))?;
    if let Some(coupling) = spec.couplings.iter().find(|c| !(-400.0..=400.0).contains(&c.gain_db)) {
        return Err(format!("CH-1: the gain of {} -> {} is outside -400..=400 dB", coupling.tx, coupling.rx));
    }
    if let Some(coupling) = spec.couplings.iter().find(|c| c.delay_ns > MAX_DELAY_NS) {
        return Err(format!("CH-1: delay_ns {} exceeds 2^62", coupling.delay_ns));
    }
    if let Some((rx, _)) = spec.noise_dbfs.iter().find(|(_, p)| !(-400.0..=100.0).contains(*p)) {
        return Err(format!("CH-1: the noise of {rx} is outside -400..=100 dBFS"));
    }
    Ok(Some(spec))
}

/// The `sim.channel` admission check (CH-2).
pub struct ChannelCheck;

const VALIDATE: &[CheckStage] = &[CheckStage::Validate];

impl AdmissionCheck for ChannelCheck {
    fn section(&self) -> &Namespace {
        channel_namespace()
    }

    fn stages(&self) -> &[CheckStage] {
        VALIDATE
    }

    fn check(
        &self,
        section: &serde_json::Value,
        effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _stage: CheckStage,
    ) -> Vec<Violation> {
        let violation = |reason: String| Violation {
            check: channel_namespace().clone(),
            key: None,
            requested: None,
            reason,
        };
        let environment = BTreeMap::from([(channel_namespace().clone(), section.clone())]);
        match read(&environment) {
            Err(reason) => vec![violation(format!("CH-2: {reason}"))],
            Ok(None) => vec![],
            Ok(Some(spec)) => named(&spec)
                .into_iter()
                .filter(|name| !effective.contains_key(name))
                .map(|name| violation(format!("CH-2: {name} names no fragment of this Run")))
                .collect(),
        }
    }
}

/// Every fragment id the document names, as `tx`, `rx` or a noise key, each once and in
/// order (CH-2, CH-7).
pub fn named(spec: &ChannelSpec) -> Vec<Ident> {
    let mut names = BTreeSet::new();
    for coupling in &spec.couplings {
        names.insert(coupling.tx.clone());
        names.insert(coupling.rx.clone());
    }
    names.extend(spec.noise_dbfs.keys().cloned());
    names.into_iter().collect()
}

/// An exact instant in root ticks, `num / den` with `den > 0` (CH-3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RootInstant {
    /// The numerator (CH-3).
    pub num: i128,
    /// The denominator, always positive (CH-3).
    pub den: i128,
}

impl RootInstant {
    /// The instant of root tick `tick` (CH-3).
    pub fn tick(tick: i64) -> RootInstant {
        RootInstant { num: i128::from(tick), den: 1 }
    }

    /// The instant of sample `k` of a SampleClock with origin `origin` and `ratio` root
    /// ticks per sample: `origin + k · ratio` (CH-3).
    pub fn of_sample(origin: i64, ratio: Rational, k: i64) -> RootInstant {
        let den = i128::from(ratio.den());
        RootInstant { num: i128::from(origin) * den + i128::from(k) * i128::from(ratio.num()), den }
    }

    /// This instant moved earlier by `ticks` root ticks (CH-3).
    pub fn minus_ticks(self, ticks: i64) -> RootInstant {
        RootInstant { num: self.num - i128::from(ticks) * self.den, den: self.den }
    }

    /// The last sample of the clock `(origin, ratio)` whose instant is at or before this
    /// one: `floor((self − origin) / ratio)` (CH-3).
    pub fn sample_at_or_before(self, origin: i64, ratio: Rational) -> i64 {
        let numerator = (self.num - i128::from(origin) * self.den) * i128::from(ratio.den());
        let denominator = self.den * i128::from(ratio.num());
        i64::try_from(numerator.div_euclid(denominator)).unwrap_or(if numerator < 0 { i64::MIN } else { i64::MAX })
    }

    /// Whether root tick `tick` is at or before this instant (CH-3).
    pub fn is_at_or_after(self, tick: i64) -> bool {
        i128::from(tick) * self.den <= self.num
    }
}

/// One radiated sample: the complex baseband value a transmitter emits at an instant,
/// and the carrier frequency it emits it on (CH-8).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Radiated {
    /// The in-phase component (CH-8).
    pub re: f64,
    /// The quadrature component (CH-8).
    pub im: f64,
    /// The carrier frequency in hertz (CH-4, CH-8).
    pub frequency_hz: f64,
}

/// What a simulated radio exposes to the medium for its transmit side (CH-8).
pub trait Transmitter: Send + Sync {
    /// The sample `channel` radiates at `at`, held from its own instant until the next
    /// one (a zero-order hold), or `None` when nothing is radiated then (CH-8).
    fn radiated(&self, channel: u16, at: RootInstant) -> Option<Radiated>;
}

struct Path {
    tx: Ident,
    tx_channel: u16,
    rx: Ident,
    rx_channel: u16,
    amplitude: f64,
    delay_ticks: i64,
}

#[derive(Default)]
struct State {
    run: Option<RunId>,
    spec: Option<ChannelSpec>,
    seed: u64,
    root_rate_hz: u64,
    paths: Vec<Path>,
    transmitters: BTreeMap<Ident, Arc<dyn Transmitter>>,
    noise: BTreeMap<(Ident, u16), SimRng>,
}

/// The medium the simulated radios of one Run share: it holds the coupling matrix and
/// computes the field at a receiver (CH-6).
#[derive(Default)]
pub struct Medium {
    state: Mutex<State>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|error| error.into_inner())
}

impl Medium {
    /// An empty medium, which the runtime creates once per Run and hands to every
    /// simulated radio it constructs (CH-6).
    pub fn new() -> Arc<Medium> {
        Arc::new(Medium::default())
    }

    /// Joins fragment `fragment` of Run `run`. The first join fixes the Run, the
    /// document, the seed and the root rate; a later join that differs in any of them,
    /// or a second join of one fragment, is refused (CH-6).
    pub fn join(
        &self,
        run: &RunId,
        fragment: &Ident,
        spec: &ChannelSpec,
        seed: u64,
        root_rate_hz: u64,
        transmitter: Arc<dyn Transmitter>,
    ) -> Result<(), String> {
        let mut state = lock(&self.state);
        match &state.run {
            None => {
                if root_rate_hz == 0 {
                    return Err("CH-6: the root rate must be positive".to_owned());
                }
                let mut paths = Vec::with_capacity(spec.couplings.len());
                for coupling in &spec.couplings {
                    let ticks = (i128::from(coupling.delay_ns) * i128::from(root_rate_hz) + 999_999_999)
                        / 1_000_000_000;
                    paths.push(Path {
                        tx: coupling.tx.clone(),
                        tx_channel: coupling.tx_channel,
                        rx: coupling.rx.clone(),
                        rx_channel: coupling.rx_channel,
                        amplitude: 10f64.powf(coupling.gain_db / 20.0),
                        delay_ticks: i64::try_from(ticks)
                            .map_err(|_| "CH-6: a delay overflows the root".to_owned())?,
                    });
                }
                state.run = Some(run.clone());
                state.spec = Some(spec.clone());
                state.seed = seed;
                state.root_rate_hz = root_rate_hz;
                state.paths = paths;
            }
            Some(joined) => {
                if joined != run {
                    return Err(format!("CH-6: this medium serves Run {joined}, not {run}"));
                }
                if state.spec.as_ref() != Some(spec) || state.seed != seed || state.root_rate_hz != root_rate_hz {
                    return Err("CH-6: a join must carry the Run's one document, seed and root rate".to_owned());
                }
            }
        }
        if state.transmitters.contains_key(fragment) {
            return Err(format!("CH-6: {fragment} has already joined"));
        }
        state.transmitters.insert(fragment.clone(), transmitter);
        Ok(())
    }

    /// The fragments the document names that have not joined, in order (CH-7).
    pub fn missing(&self) -> Vec<Ident> {
        let state = lock(&self.state);
        let Some(spec) = &state.spec else { return Vec::new() };
        named(spec).into_iter().filter(|name| !state.transmitters.contains_key(name)).collect()
    }

    /// The field at receive channel `channel` of `rx` at antenna instant `at`, for a
    /// receiver tuned to `frequency_hz`: the sum over the paths into it of amplitude ×
    /// the transmitter's radiated sample at `at − delay`, counting a path only when the
    /// transmitter radiates on `frequency_hz`, plus one draw of the receiver's noise.
    /// A receiver calls it once per sample, in sample order (CH-4, CH-5, CH-9).
    pub fn field(&self, rx: &Ident, channel: u16, frequency_hz: f64, at: RootInstant) -> (f64, f64) {
        let (sources, sigma) = {
            let state = lock(&self.state);
            let sources: Vec<(Arc<dyn Transmitter>, u16, f64, i64)> = state
                .paths
                .iter()
                .filter(|path| path.rx == *rx && path.rx_channel == channel)
                .filter_map(|path| {
                    state
                        .transmitters
                        .get(&path.tx)
                        .map(|tx| (tx.clone(), path.tx_channel, path.amplitude, path.delay_ticks))
                })
                .collect();
            let sigma = state
                .spec
                .as_ref()
                .and_then(|spec| spec.noise_dbfs.get(rx))
                .map(|dbfs| (10f64.powf(dbfs / 10.0) / 2.0).sqrt());
            (sources, sigma)
        };
        let (mut re, mut im) = (0.0, 0.0);
        for (tx, tx_channel, amplitude, delay) in sources {
            if let Some(sample) = tx.radiated(tx_channel, at.minus_ticks(delay)) {
                if sample.frequency_hz == frequency_hz {
                    re += amplitude * sample.re;
                    im += amplitude * sample.im;
                }
            }
        }
        if let Some(sigma) = sigma {
            let mut state = lock(&self.state);
            let seed = state.seed;
            let rng = state
                .noise
                .entry((rx.clone(), channel))
                .or_insert_with(|| SimRng::new(seed, &format!("sim.channel/{rx}/{channel}")));
            let (g1, g2) = gaussian_pair(rng);
            re += sigma * g1;
            im += sigma * g2;
        }
        (re, im)
    }
}

/// Two independent standard normal values from two draws of `rng`, by the Box–Muller
/// transform: `u1 = (⌊x1 / 2^11⌋ + 1) · 2^−53`, `u2 = ⌊x2 / 2^11⌋ · 2^−53`,
/// `r = sqrt(−2 ln u1)`, `(r cos 2πu2, r sin 2πu2)` (CH-5).
pub fn gaussian_pair(rng: &mut SimRng) -> (f64, f64) {
    const SCALE: f64 = 1.0 / (1u64 << 53) as f64;
    let u1 = ((rng.next_u64() >> 11) as f64 + 1.0) * SCALE;
    let u2 = (rng.next_u64() >> 11) as f64 * SCALE;
    let r = (-2.0 * u1.ln()).sqrt();
    let theta = 2.0 * std::f64::consts::PI * u2;
    (r * theta.cos(), r * theta.sin())
}
