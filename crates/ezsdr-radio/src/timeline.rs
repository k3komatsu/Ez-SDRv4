//! RM-16, RM-21 and RM-25 computed once (spec 22, VH-7): a stream's segments, each with
//! its origin, its cut and its configuration, from the stream's initial state, its
//! Provider's terms and its commands and faults. Pure and deterministic; every instant is
//! in root ticks, a sample instant is the root tick of its sample, rounded up (MR-7), and a
//! cut is a sample index of its segment, so that it never has to be read back from a tick.

use ezsdr_kernel::stream::Direction;
use ezsdr_kernel::time::{Rational, TimeError};

/// A stream's configuration: its channel count and its root ticks per sample (RM-21).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Config {
    /// `radio.{rx,tx}.channels`.
    pub channels: u16,
    /// Root ticks per sample, from `radio.{rx,tx}.sample_rate_hz`.
    pub ratio: Rational,
}

/// A stream's initial state and its Provider's terms (RM-25).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stream {
    /// Which stream: receive is on or off (RM-21), and drops a segment with no sample.
    pub direction: Direction,
    /// The first segment's origin: T0 for receive, the first lattice instant at or after
    /// `arm` for transmit (RM-25).
    pub origin: i64,
    /// The configuration from the Run's start.
    pub config: Config,
    /// `radio.timing.start_lead_ns`, in root ticks.
    pub lead: i64,
    /// One receive call, in samples: the longer of `block_len` and the device's packet; 0
    /// for a Provider whose ready instant is the receipt alone (VH-4).
    pub call: i64,
    /// The delivery allowance after that call, in root ticks.
    pub allowance: i64,
}

/// What a command or fault does to its stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A `cold` change: the configuration in effect from its instant (UC-3, RM-21).
    Cold(Config),
    /// A `Stop` of the stream or its device: receive turns off; transmit ends bursts, not
    /// its clock (RM-16).
    Stop,
    /// `start_rx`: receive turns on (RM-21).
    Start,
    /// A device loss or `Provider::stop`: nothing after it takes effect, and the receive
    /// segment ends, under `abort` at the first sample not yet delivered (RM-16).
    End {
        /// `Provider::stop(Abort)`.
        abort: bool,
    },
}

/// One command or fault of a stream (RM-25).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Item {
    /// The effective instant.
    pub e: i64,
    /// The arrival order; faults count as received at the Run's start.
    pub seq: u64,
    /// The instant the Provider is ready for a segment this item starts: the receipt or
    /// booking, or the end of the configuration an enable needs (VH-4).
    pub ready: i64,
    /// The first sample of the running segment not yet delivered when the Provider handled
    /// the item, if it knows it: every cut is at or after it (RM-16). Only for an item that
    /// takes effect when it is handled — a `Stop`, an abort, a loss — so that the index is
    /// of the segment the item cuts; a change booked ahead may cut a later segment.
    pub delivered: Option<i64>,
    /// Refused at its instant: the device did not apply the configuration (RM-25).
    pub refused: bool,
    /// What it does.
    pub kind: Kind,
}

/// A run of a stream on one SampleClock (RM-21, RM-25).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Segment {
    /// The root tick of sample 0, on the lattice (RM-25).
    pub origin: i64,
    /// The first sample not delivered, if the segment ends (RM-16); its SampleClock ends at
    /// that sample's instant.
    pub cut: Option<i64>,
    /// The configuration it runs.
    pub config: Config,
    /// The arrival order of the command that began it; none for the stream's first segment.
    pub by: Option<u64>,
}

impl Segment {
    /// The first sample at or after root tick `t`, never before sample 0 (RM-16).
    pub fn sample_at_or_after(&self, t: i64) -> Result<i64, TimeError> {
        let (num, den) = parts(self.config.ratio);
        let ticks = (i128::from(t) - i128::from(self.origin)).checked_mul(den).ok_or(TimeError::Overflow)?;
        narrow(ceil_div(ticks, num).max(0))
    }

    /// The root tick of sample `k`, rounded up (MR-7).
    pub fn instant(&self, k: i64) -> Result<i64, TimeError> {
        let (num, den) = parts(self.config.ratio);
        let ticks = i128::from(k).checked_mul(num).ok_or(TimeError::Overflow)?;
        narrow(i128::from(self.origin) + ceil_div(ticks, den))
    }
}

/// One stream's commands and faults, and their plan, as a Provider books them (RM-26).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Line {
    /// The stream's initial state and its Provider's terms.
    pub stream: Stream,
    /// Every command and fault booked, in the order booked.
    pub items: Vec<Item>,
    /// Their plan.
    pub plan: Vec<Segment>,
}

impl Line {
    /// A stream with nothing booked: its first segment, if it has a channel.
    pub fn new(stream: Stream) -> Line {
        let first = Segment { origin: stream.origin, cut: None, config: stream.config, by: None };
        Line { plan: (stream.config.channels > 0).then_some(first).into_iter().collect(), stream, items: Vec::new() }
    }

    /// Books a command or fault and plans the stream again; returns its effective instant.
    // ponytail: the items stay for the whole Run and each booking plans from its start, so a
    // Run's booking cost grows with its commands; prune the items wholly before the last
    // recorded cut, planning from that state, when a Run books thousands of them.
    pub fn book(&mut self, item: Item) -> Result<i64, TimeError> {
        self.items.push(item);
        self.plan = plan(&self.stream, &self.items)?;
        Ok(effective(&self.items, &item))
    }

    /// The planned segment from `origin` with `config`: a receive segment cut at its origin is
    /// not planned, and another may begin there.
    pub fn segment(&self, origin: i64, config: Config) -> Option<Segment> {
        self.plan.iter().find(|segment| segment.origin == origin && segment.config == config).copied()
    }

    /// That segment as the items in effect before `(t, seq)` have made it (RM-25): what has
    /// happened by `t`, a fault received with `seq` coming before an item at `t` received later.
    pub fn made(&self, origin: i64, config: Config, t: i64, seq: u64) -> Result<Option<Segment>, TimeError> {
        let made: Vec<Item> = self.items.iter().filter(|item| (effective(&self.items, item), item.seq) < (t, seq)).copied().collect();
        Ok(plan(&self.stream, &made)?.into_iter().find(|segment| segment.origin == origin && segment.config == config))
    }
}

/// The instant a command takes effect at (RM-25, VH-2): a `cold` change never before a
/// `cold` change of its stream that arrived earlier, its own effective instant floored at
/// that change's, so that at one instant the two keep their order of arrival; a Provider
/// applies it late, as UC-2 applies a timed update late. Other items take effect at their own.
pub fn effective(items: &[Item], item: &Item) -> i64 {
    let cold = |item: &Item| matches!(item.kind, Kind::Cold(_));
    if !cold(item) {
        return item.e;
    }
    items.iter().filter(|earlier| cold(earlier) && earlier.seq < item.seq).fold(item.e, |e, earlier| e.max(earlier.e))
}

/// When a command received at `now` with the requested instant `at` takes effect on its
/// stream (RM-25, VH-2): `at`, or `now` when it is absent or past; on receive never before
/// the stream's start; and a `cold` change never before one of its stream received earlier.
/// The flag says whether that is later than `at` asked, which makes a `cold` change late
/// (UC-2). `line` is the stream's, if it has one.
pub fn command_instant(line: Option<&Line>, at: Option<i64>, now: i64, seq: u64, cold: bool) -> (i64, bool) {
    let asked = at.unwrap_or(now);
    let floor = line.filter(|line| line.stream.direction == Direction::Rx).map_or(i64::MIN, |line| line.stream.origin);
    let base = asked.max(now).max(floor);
    let e = match line.filter(|_| cold) {
        Some(line) => effective(&line.items, &Item { e: base, seq, ready: now, delivered: None, refused: false, kind: Kind::Cold(line.stream.config) }),
        None => base,
    };
    (e, asked < now || e > base)
}

/// RM-15: the instant a burst on a transmit clock with origin `origin` is decided at: `now`,
/// or, before that clock begins, `lead` before its origin, so that a burst decided on time
/// never starts before the origin and one that would is late.
pub fn decided_at(now: i64, origin: i64, lead: i64) -> i64 {
    now.max(origin.saturating_sub(lead))
}

/// RM-25: the first lattice instant of `ratio` at or after `t`, a whole multiple of its
/// numerator in lowest terms.
pub fn lattice(t: i64, ratio: Rational) -> Result<i64, TimeError> {
    let (num, _) = parts(ratio);
    narrow(ceil_div(i128::from(t), num) * num)
}

/// The segments of one stream, in order (RM-16, RM-21, RM-25). Items take effect at their
/// [`effective`] instants, in that order and, at one instant, in the order of their arrival.
/// A receive segment cut with no sample has no SampleClock and is left out; a transmit one is
/// kept, since its clock was registered when its change was booked. For the same reason a
/// transmit clock's record stands once booked (VH-2): after a loss or `Provider::stop` at `f`,
/// the changes booked before `f` keep their cuts and the clock they began after `f` ends at
/// its origin; a refused change's clock ends at its origin unless a change booked before the
/// refusal, at the refused change's instant, already cut it.
pub fn plan(stream: &Stream, items: &[Item]) -> Result<Vec<Segment>, TimeError> {
    let mut order: Vec<Item> = items.iter().map(|item| Item { e: effective(items, item), ..*item }).collect();
    order.sort_by_key(|item| (item.e, item.seq));
    let receive = stream.direction == Direction::Rx;
    let mut planner = Planner { stream, receive, running: None, previous: None, out: Vec::new() };
    let mut config = stream.config;
    planner.running = (config.channels > 0).then_some(Segment { origin: stream.origin, cut: None, config, by: None });
    // RM-21: a receive stream is on from the Run's start; a refusal halts either stream
    // until its next `cold` change or `start_rx`.
    let (mut on, mut halted) = (true, false);
    // Transmit only: the instant of a loss or `Provider::stop`, and of a refusal whose clock
    // is still to end at its origin; an item booked at or after either is planned after it.
    let (mut ended, mut refusal) = (None::<i64>, None::<i64>);
    for item in &order {
        if refusal.is_some_and(|refused| item.ready >= refused) {
            planner.end_at_origin()?;
            refusal = None;
        }
        if ended.is_some_and(|end| item.ready >= end) {
            continue;
        }
        match item.kind {
            // RM-16, RM-21: receive ends at its cut, and nothing after it takes effect; on
            // transmit it ends the bursts, not the clock.
            Kind::End { abort } if receive => {
                planner.cut(item, abort)?;
                break;
            }
            Kind::End { .. } => ended = ended.or(Some(item.e)),
            Kind::Stop if receive && on => {
                on = false;
                planner.cut(item, false)?;
            }
            Kind::Start if receive && (!on || halted) => {
                on = true;
                halted = item.refused;
                if !halted && config.channels > 0 {
                    planner.running = Some(planner.begin(item, config)?);
                }
            }
            Kind::Stop | Kind::Start => {}
            Kind::Cold(next) => {
                planner.cut(item, false)?;
                refusal = None;
                config = next;
                halted = item.refused;
                if on && config.channels > 0 {
                    let segment = planner.begin(item, config)?;
                    // RM-25: a refused change has no receive segment; the transmit clock
                    // registered for it ends at its origin.
                    if !item.refused || !receive {
                        planner.running = Some(segment);
                    }
                    if item.refused && !receive {
                        refusal = Some(item.e);
                    }
                }
            }
        }
    }
    if refusal.is_some() || planner.running.zip(ended).is_some_and(|(segment, end)| segment.origin >= end) {
        // VH-2: a transmit clock orphaned by a loss, `Provider::stop` or a refusal.
        planner.end_at_origin()?;
    }
    let mut out = planner.out;
    out.extend(planner.running);
    Ok(out)
}

struct Planner<'a> {
    stream: &'a Stream,
    receive: bool,
    running: Option<Segment>,
    /// The last cut's instant and the configuration it ended (RM-25's start rule).
    previous: Option<(i64, Config)>,
    out: Vec<Segment>,
}

impl Planner<'_> {
    /// RM-16: the running segment ends at its cut, the first sample at or after the item's
    /// instant, never before sample 0 or the first sample not yet delivered; under `abort`,
    /// that sample itself when the Provider knows it, never before sample 0.
    fn cut(&mut self, item: &Item, abort: bool) -> Result<(), TimeError> {
        let Some(mut segment) = self.running.take() else { return Ok(()) };
        let cut = match (abort, item.delivered) {
            (true, Some(delivered)) => delivered.max(0),
            (_, delivered) => segment.sample_at_or_after(item.e)?.max(delivered.unwrap_or(i64::MIN)),
        };
        segment.cut = Some(cut);
        self.previous = Some((segment.instant(cut)?, segment.config));
        if cut > 0 || !self.receive {
            self.out.push(segment);
        }
        Ok(())
    }

    /// Ends the running segment at its origin, with no sample (VH-2).
    fn end_at_origin(&mut self) -> Result<(), TimeError> {
        let Some(segment) = self.running else { return Ok(()) };
        let item = Item { e: segment.origin, seq: 0, ready: segment.origin, delivered: None, refused: false, kind: Kind::Stop };
        self.cut(&item, false)
    }

    /// RM-25's start rule: the first lattice instant at or after the item's effective
    /// instant, the previous cut plus the lead, and the ready instant plus the lead. After a
    /// cut, a receive segment is ready no earlier than that cut plus one receive call at the
    /// old rate and the allowance, the call the Provider has in progress there (VH-4).
    fn begin(&self, item: &Item, config: Config) -> Result<Segment, TimeError> {
        let lead = i128::from(self.stream.lead);
        let mut earliest = i128::from(item.e);
        let mut ready = i128::from(item.ready);
        if let Some((cut, old)) = self.previous {
            let cut = i128::from(cut);
            earliest = earliest.max(cut + lead);
            if self.receive {
                let (num, den) = parts(old.ratio);
                let call = i128::from(self.stream.call).checked_mul(num).ok_or(TimeError::Overflow)?;
                ready = ready.max(cut + ceil_div(call, den) + i128::from(self.stream.allowance));
            }
        }
        earliest = earliest.max(ready + lead);
        let (num, _) = parts(config.ratio);
        Ok(Segment { origin: narrow(ceil_div(earliest, num) * num)?, cut: None, config, by: Some(item.seq) })
    }
}

fn parts(ratio: Rational) -> (i128, i128) {
    (i128::from(ratio.num()), i128::from(ratio.den()))
}

fn ceil_div(a: i128, b: i128) -> i128 {
    a.div_euclid(b) + i128::from(a.rem_euclid(b) != 0)
}

fn narrow(value: i128) -> Result<i64, TimeError> {
    i64::try_from(value).map_err(|_| TimeError::Overflow)
}
