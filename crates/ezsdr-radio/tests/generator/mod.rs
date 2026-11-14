//! VH-8's seeded sequences (spec 22): stops, `start_rx`, faults and a loss, with ties and with
//! effective instants out of arrival order. The timeline's own tests and both
//! Providers' comparisons read the same sequences: this file is included by path, so that
//! no crate takes another as a dev-dependency (PO-8) and none ships it.
#![allow(dead_code)]

use ezsdr_kernel::stream::Direction;
use ezsdr_radio::timeline::{Item, Kind, Stream};

/// The most a sequence exercises: a sequence holds its own class's operation or fault and
/// may hold any earlier class's. The order is how far today's Providers stray from the
/// timeline.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Class {
    /// `Stop` of the receive stream or of the device.
    Stop,
    /// `start_rx`.
    StartRx,
    /// A receive value (frequency or gain) set on stream 0's channel 0, accepted on a stopped stream and refused on a streaming one (RM-21).
    Value,
    /// Overruns and sequence errors, and half the time a device loss.
    Fault,
    /// A device loss, always.
    Loss,
}

/// Every class, in order.
pub const CLASSES: [Class; 5] = [Class::Stop, Class::StartRx, Class::Value, Class::Fault, Class::Loss];

/// One Action of a round.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Op {
    /// A `Stop` of `<device>/rx`, or of `<device>`.
    Stop {
        /// The device rather than its receive stream.
        device: bool,
    },
    /// `start_rx`, at the round's instant or, with `Some(ns)`, `ns` nanoseconds after it: its effective instant then differs from its arrival order
    /// (a `Stop` after it does not cancel it).
    StartRx {
        /// The `at` offset after the round's instant, a multiple of 100 ns.
        at: Option<i64>,
    },
    /// An `UpdateParameter` of receive channel 0's `leaf`, with value `v` (RM-21).
    Set {
        /// Which key.
        leaf: Leaf,
        /// The value's number.
        v: u8,
    },
}

/// The receive channel keys the generator sets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Leaf {
    /// `radio.rx.c0.frequency_hz`: `1e9 + 1e6 * v` Hz.
    Frequency,
    /// `radio.rx.c0.gain_db`: `v` dB.
    Gain,
}

impl Leaf {
    /// The value `v` stands for.
    pub fn value(self, v: u8) -> f64 {
        match self {
            Leaf::Frequency => 1.0e9 + 1.0e6 * f64::from(v),
            Leaf::Gain => f64::from(v),
        }
    }
}

/// What a start took, as the oracle computes it: the last accepted value of each leaf, none while a leaf is at the Run's own value.
pub type Snapshot = (Option<u8>, Option<u8>);

/// The oracle's account of a sequence's values (RM-21), written apart from `Staging` and `Line::streaming`.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Staged {
    /// The op indices of the refused `Set`s.
    pub refused: Vec<usize>,
    /// The op indices of the refused `start_rx`s: the stream was streaming (RM-21).
    pub refused_starts: Vec<usize>,
    /// The op index of each booked `start_rx` and the values it took.
    pub starts: Vec<(usize, Snapshot)>,
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
    /// The stream and its Provider's terms.
    pub stream: Stream,
}

impl Terms {
    /// Root ticks for `ns` nanoseconds.
    pub fn ticks(&self, ns: i64) -> i64 {
        (i128::from(ns) * i128::from(self.root_hz) / 1_000_000_000) as i64
    }
}

impl Sequence {
    /// The Actions in arrival order, each with its round's instant.
    pub fn ops(&self) -> impl Iterator<Item = (i64, &Op)> {
        self.rounds.iter().flat_map(|(at, ops)| ops.iter().map(move |op| (*at, op)))
    }

    /// Every operation and fault, sorted by effective instant and arrival. A `Stop` and a
    /// `start_rx` take effect at their receipt.
    pub fn schedule(&self, terms: &Terms) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self
            .faults
            .iter()
            .enumerate()
            .map(|(index, (at, _))| {
                let e = terms.t0 + terms.ticks(*at);
                Entry { e, seq: index as u64, receipt: e, event: Event::Fault(index) }
            })
            .collect();
        for (index, (round, op)) in self.ops().enumerate() {
            let receipt = terms.t0 + terms.ticks(round);
            // A `Stop` takes effect at its receipt and a `start_rx` at its `at`, or its receipt (RM-12, RM-25).
            let e = match op {
                Op::StartRx { at: Some(ns) } => receipt + terms.ticks(*ns),
                _ => receipt,
            };
            entries.push(Entry { e, seq: (self.faults.len() + index) as u64, receipt, event: Event::Op(index) });
        }
        entries.sort_by_key(|entry| (entry.e, entry.seq));
        entries
    }

    /// The timeline's items for `terms.stream`: each ready at its receipt. A `start_rx` the stream refused as streaming (the oracle's
    /// `Staged::refused_starts`) is no item: the line is unchanged by it.
    pub fn items(&self, terms: &Terms) -> Vec<Item> {
        let receive = terms.stream.direction == Direction::Rx;
        let refused = if receive { self.staged(terms, |_| true).refused_starts } else { Vec::new() };
        let ops: Vec<&Op> = self.ops().map(|(_, op)| op).collect();
        let mut items = Vec::new();
        for entry in self.schedule(terms) {
            let kind = match entry.event {
                Event::Fault(index) if self.faults[index].1 == Fault::Loss => Kind::End { abort: false },
                Event::Fault(_) => continue,
                Event::Op(index) => match *ops[index] {
                    Op::Stop { .. } if receive => Kind::Stop,
                    Op::StartRx { .. } if receive && !refused.contains(&index) => Kind::Start,
                    _ => continue,
                },
            };
            items.push(Item { e: entry.e, seq: entry.seq, ready: entry.receipt, delivered: None, refused: false, kind });
        }
        items
    }
}

impl Sequence {
    /// RM-21's staging as the oracle reads it, in arrival order: a `Set` is refused when the stream is streaming, which is the final state of the
    /// commands booked before it replayed in the order they take effect (a `Stop` turns it off, a `start_rx` on, each at its effective
    /// instant, ties by arrival); an accepted one replaces the leaf's value; a `start_rx` is refused on the same condition and otherwise takes
    /// the values at its booking, and a refused one is no command. `booked(index)` says whether the Provider took op `index` (none after a loss).
    pub fn staged(&self, terms: &Terms, booked: impl Fn(usize) -> bool) -> Staged {
        let schedule = self.schedule(terms);
        let effective = |index: usize| schedule.iter().find(|entry| entry.event == Event::Op(index)).map(|entry| (entry.e, entry.seq)).expect("every op is scheduled");
        let ops: Vec<&Op> = self.ops().map(|(_, op)| op).collect();
        let mut out = Staged::default();
        let mut current: Snapshot = (None, None);
        for (index, op) in ops.iter().enumerate() {
            if !booked(index) {
                continue;
            }
            // Streaming: the commands booked so far, in the order they take effect, ending on.
            let mut earlier: Vec<usize> = (0..index).filter(|i| booked(*i) && !out.refused_starts.contains(i) && matches!(ops[*i], Op::Stop { .. } | Op::StartRx { .. })).collect();
            earlier.sort_by_key(|i| effective(*i));
            let mut on = true;
            for i in earlier {
                match ops[i] {
                    Op::Stop { .. } => on = false,
                    _ => on = true,
                }
            }
            match op {
                Op::Set { .. } if on => out.refused.push(index),
                Op::Set { leaf: Leaf::Frequency, v } => current.0 = Some(*v),
                Op::Set { leaf: Leaf::Gain, v } => current.1 = Some(*v),
                Op::StartRx { .. } if on => out.refused_starts.push(index),
                Op::StartRx { .. } => out.starts.push((index, current)),
                Op::Stop { .. } => {}
            }
        }
        out
    }
}

/// SplitMix64: small, seeded and the same on every platform.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn below(&mut self, n: u64) -> u64 {
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
}

/// The sequence for `seed`: two to five rounds of one to three Actions, rounds sometimes at one
/// instant, and fault instants drawn from four shared instants half the time, so that ties
/// between Actions, rounds and faults are common.
pub fn sequence(seed: u64) -> Sequence {
    let mut rng = Rng(seed);
    let class = CLASSES[rng.below(CLASSES.len() as u64) as usize];
    let shared: Vec<i64> = (0..4).map(|_| rng.instant()).collect();
    let op = |rng: &mut Rng, kind: Class| match kind {
        Class::Stop => Op::Stop { device: rng.below(4) == 0 },
        Class::Value => Op::Set { leaf: if rng.below(2) == 0 { Leaf::Frequency } else { Leaf::Gain }, v: rng.below(8) as u8 },
        _ => Op::StartRx {
            at: match rng.below(3) {
                0 => None,
                1 => Some(100 * rng.below(100_000) as i64),
                _ => Some(1_000_000 * (1 + rng.below(120)) as i64),
            },
        },
    };
    // The Action kinds the class may hold; faults are drawn apart.
    let kinds: Vec<Class> = [Class::Stop, Class::StartRx, Class::Value].into_iter().filter(|kind| *kind <= class).collect();
    let mut instant = 0;
    let mut rounds: Vec<(i64, Vec<Op>)> = Vec::new();
    for _ in 0..2 + rng.below(4) {
        instant += match rng.below(3) {
            0 => 0,
            1 => 100 * rng.below(100) as i64,
            _ => 100_000 * rng.below(800) as i64,
        };
        let ops = (0..1 + rng.below(3))
            .map(|_| {
                let kind = kinds[rng.below(kinds.len() as u64) as usize];
                op(&mut rng, kind)
            })
            .collect();
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
    // The Loss class always holds a loss; the Fault class half the time, so a loss with faults still pending after it is generated (#77).
    if class == Class::Loss || (class == Class::Fault && rng.below(2) == 0) {
        let at = if rng.below(2) == 0 { shared[rng.below(4) as usize] } else { rng.instant() };
        faults.push((at, Fault::Loss));
    }
    Sequence { class, rounds, faults }
}
