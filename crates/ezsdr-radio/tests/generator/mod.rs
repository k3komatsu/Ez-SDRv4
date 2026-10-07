//! VH-8's seeded sequences (spec 22): stops, `start_rx`, `cold` changes of rate and of
//! channel count (to and from 0), `hardware_timed` updates, faults and a loss, with ties
//! and with effective instants out of arrival order. The timeline's own tests and both
//! Providers' comparisons read the same sequences: this file is included by path, so that
//! no crate takes another as a dev-dependency (PO-8) and none ships it.
#![allow(dead_code)]

use ezsdr_kernel::stream::Direction;
use ezsdr_kernel::time::Rational;
use ezsdr_radio::timeline::{Item, Kind, Stream};

/// The most a sequence exercises: a sequence holds its own class's operation or fault and
/// may hold any earlier class's. The order is how far today's Providers stray from the
/// timeline.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Class {
    /// `hardware_timed` gain updates.
    Timed,
    /// Transmit `cold` changes.
    TxCold,
    /// Overruns and sequence errors.
    Fault,
    /// Receive `cold` changes.
    RxCold,
    /// `Stop` of the receive stream or of the device.
    Stop,
    /// A device loss.
    Loss,
    /// `start_rx`.
    StartRx,
}

/// Every class, in order.
pub const CLASSES: [Class; 7] =
    [Class::Timed, Class::TxCold, Class::Fault, Class::RxCold, Class::Stop, Class::Loss, Class::StartRx];

/// The rates a change picks on a device whose rates are decimations of 200 MHz.
pub const RATES: [u64; 5] = [400_000, 500_000, 1_000_000, 2_000_000, 5_000_000];

/// `RATES` and 3 MS/s, for a 1 GHz root where it is 1 000/3 ticks a sample (MR-7).
pub const FRACTIONAL_RATES: [u64; 6] = [400_000, 500_000, 1_000_000, 2_000_000, 3_000_000, 5_000_000];

/// One `cold` change.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Change {
    /// `radio.{rx,tx}.channels`.
    Channels(u16),
    /// `radio.{rx,tx}.sample_rate_hz`.
    Rate(u64),
}

/// One Action of a round.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Op {
    /// A `Stop` of `<device>/rx`, or of `<device>`.
    Stop {
        /// The device rather than its receive stream.
        device: bool,
    },
    /// `start_rx` at the current instant.
    StartRx,
    /// A `cold` update, `at` nanoseconds after T0 when timed.
    Cold {
        /// Its stream.
        direction: Direction,
        /// Its key and value.
        change: Change,
        /// Its `at`.
        at_ns: Option<i64>,
    },
    /// A `hardware_timed` gain update.
    Timed {
        /// Its stream.
        direction: Direction,
        /// Its value.
        gain_db: f64,
        /// Its `at`.
        at_ns: Option<i64>,
    },
}

/// One scheduled fault (`sim.faults`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fault {
    /// `rx_overflow`.
    Overrun,
    /// `rx_sequence_error`.
    Sequence,
    /// `device_lost`.
    Loss,
}

/// One sequence; every instant is nanoseconds after T0, a multiple of 100 ns.
#[derive(Clone, PartialEq, Debug)]
pub struct Sequence {
    /// Its class.
    pub class: Class,
    /// The rounds in arrival order, each its instant and its Actions.
    pub rounds: Vec<(i64, Vec<Op>)>,
    /// The faults, each its instant.
    pub faults: Vec<(i64, Fault)>,
}

/// What may happen in a sequence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    /// The Action at this index of the flattened rounds.
    Op(usize),
    /// The fault at this index.
    Fault(usize),
}

/// One operation or fault with its effective instant and arrival order (RM-25's order).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The effective instant, in root ticks.
    pub e: i64,
    /// The arrival order: the faults first, then the Actions as sent.
    pub seq: u64,
    /// When the Provider receives it, in root ticks; T0 plus its offset for a fault.
    pub receipt: i64,
    /// Which.
    pub event: Event,
}

/// How a Provider runs a sequence on one of its streams.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Terms {
    /// T0 in root ticks.
    pub t0: i64,
    /// Root ticks per second, a multiple of 10 MHz.
    pub root_hz: u64,
    /// The lead a `hardware_timed` update without `at` takes, and a late one is moved to,
    /// in root ticks (MR-18, UR-24).
    pub timed_lead: i64,
    /// The stream and its Provider's terms.
    pub stream: Stream,
}

impl Terms {
    /// Root ticks for `ns` nanoseconds.
    pub fn ticks(&self, ns: i64) -> i64 {
        (i128::from(ns) * i128::from(self.root_hz) / 1_000_000_000) as i64
    }

    /// Root ticks per sample at `rate_hz`.
    pub fn ratio(&self, rate_hz: u64) -> Rational {
        Rational::new(self.root_hz, rate_hz).expect("a positive rate")
    }
}

impl Sequence {
    /// The Actions in arrival order, each with its round's instant.
    pub fn ops(&self) -> impl Iterator<Item = (i64, &Op)> {
        self.rounds.iter().flat_map(|(at, ops)| ops.iter().map(move |op| (*at, op)))
    }

    /// Every operation and fault, sorted by effective instant and arrival. A `cold` update
    /// takes effect at `at`, or at its receipt when absent or past, and never before a `cold`
    /// update of its stream received earlier (VH-2); a `hardware_timed` one at `at`, or at
    /// its receipt plus the lead when absent or earlier (MR-18, UR-24, UR-25); a `Stop` and a
    /// `start_rx` at their receipt.
    pub fn schedule(&self, terms: &Terms) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self.faults.iter().enumerate().map(|(index, (at, _))| {
            let e = terms.t0 + terms.ticks(*at);
            Entry { e, seq: index as u64, receipt: e, event: Event::Fault(index) }
        }).collect();
        let mut floor = [i64::MIN; 2];
        for (index, (round, op)) in self.ops().enumerate() {
            let receipt = terms.t0 + terms.ticks(round);
            let at = |at_ns: &Option<i64>| at_ns.map(|ns| terms.t0 + terms.ticks(ns));
            let e = match op {
                Op::Stop { .. } | Op::StartRx => receipt,
                Op::Cold { direction, at_ns, .. } => {
                    let floor = &mut floor[usize::from(*direction == Direction::Tx)];
                    *floor = at(at_ns).unwrap_or(receipt).max(receipt).max(*floor);
                    *floor
                }
                Op::Timed { at_ns, .. } => at(at_ns).unwrap_or(i64::MIN).max(receipt + terms.timed_lead),
            };
            entries.push(Entry { e, seq: (self.faults.len() + index) as u64, receipt, event: Event::Op(index) });
        }
        entries.sort_by_key(|entry| (entry.e, entry.seq));
        entries
    }

    /// The timeline's items for `terms.stream`: each `cold` change carries the stream's
    /// configuration projected to its instant, and its ready instant is its receipt.
    pub fn items(&self, terms: &Terms) -> Vec<Item> {
        let receive = terms.stream.direction == Direction::Rx;
        let ops: Vec<&Op> = self.ops().map(|(_, op)| op).collect();
        let mut config = terms.stream.config;
        let mut items = Vec::new();
        for entry in self.schedule(terms) {
            let kind = match entry.event {
                Event::Fault(index) if self.faults[index].1 == Fault::Loss => Kind::End { abort: false },
                Event::Fault(_) => continue,
                Event::Op(index) => match *ops[index] {
                    Op::Stop { .. } if receive => Kind::Stop,
                    Op::StartRx if receive => Kind::Start,
                    Op::Cold { direction, change, .. } if direction == terms.stream.direction => {
                        match change {
                            Change::Channels(channels) => config.channels = channels,
                            Change::Rate(rate) => config.ratio = terms.ratio(rate),
                        }
                        Kind::Cold(config)
                    }
                    _ => continue,
                },
            };
            items.push(Item { e: entry.e, seq: entry.seq, ready: entry.receipt, delivered: None, refused: false, kind });
        }
        items
    }
}

/// SplitMix64: small, seeded and the same on every platform.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % n
    }

    /// An instant up to 200 ms after T0, a multiple of 100 ns.
    fn instant(&mut self) -> i64 {
        100 * self.below(2_000_001) as i64
    }

    fn direction(&mut self) -> Direction {
        if self.below(2) == 0 { Direction::Rx } else { Direction::Tx }
    }
}

/// The sequence for `seed`, its rates drawn from `rates`: two to five rounds of one to three Actions, rounds sometimes
/// at one instant, and `at`s drawn from four shared instants half the time, so that ties
/// between Actions, rounds and faults are common.
pub fn sequence(seed: u64, rates: &[u64]) -> Sequence {
    let mut rng = Rng(seed);
    let class = CLASSES[rng.below(CLASSES.len() as u64) as usize];
    let shared: Vec<i64> = (0..4).map(|_| rng.instant()).collect();
    let at = |rng: &mut Rng| match rng.below(4) {
        0 => None,
        1 | 2 => Some(shared[rng.below(4) as usize]),
        _ => Some(rng.instant() - 2_000_000),
    };
    let op = |rng: &mut Rng, kind: Class| match kind {
        Class::Timed => Op::Timed { direction: rng.direction(), gain_db: 0.5 * rng.below(64) as f64, at_ns: at(rng) },
        Class::TxCold | Class::RxCold => Op::Cold {
            direction: if kind == Class::TxCold { Direction::Tx } else { Direction::Rx },
            change: if rng.below(2) == 0 {
                Change::Channels(rng.below(3) as u16)
            } else {
                Change::Rate(rates[rng.below(rates.len() as u64) as usize])
            },
            at_ns: at(rng),
        },
        Class::Stop => Op::Stop { device: rng.below(4) == 0 },
        _ => Op::StartRx,
    };
    // The Action kinds the class may hold; faults are drawn apart.
    let kinds: Vec<Class> = [Class::Timed, Class::TxCold, Class::RxCold, Class::Stop, Class::StartRx]
        .into_iter()
        .filter(|kind| *kind <= class)
        .collect();
    let mut instant = 0;
    let mut rounds: Vec<(i64, Vec<Op>)> = Vec::new();
    for _ in 0..2 + rng.below(4) {
        instant += match rng.below(3) {
            0 => 0,
            1 => 100 * rng.below(100) as i64,
            _ => 100_000 * rng.below(800) as i64,
        };
        let ops = (0..1 + rng.below(3)).map(|_| {
            let kind = kinds[rng.below(kinds.len() as u64) as usize];
            op(&mut rng, kind)
        }).collect();
        rounds.push((instant, ops));
    }
    if kinds.contains(&class) {
        let round = rng.below(rounds.len() as u64) as usize;
        let own = op(&mut rng, class);
        rounds[round].1.push(own);
    }
    let mut faults = Vec::new();
    if class >= Class::Fault {
        for _ in 0..u64::from(class == Class::Fault) + rng.below(2) {
            let fault = if rng.below(2) == 0 { Fault::Overrun } else { Fault::Sequence };
            let at = if rng.below(2) == 0 { shared[rng.below(4) as usize] } else { rng.instant() };
            faults.push((at, fault));
        }
    }
    if class == Class::Loss || (class > Class::Loss && rng.below(4) == 0) {
        let at = if rng.below(2) == 0 { shared[rng.below(4) as usize] } else { rng.instant() };
        faults.push((at, Fault::Loss));
    }
    Sequence { class, rounds, faults }
}
