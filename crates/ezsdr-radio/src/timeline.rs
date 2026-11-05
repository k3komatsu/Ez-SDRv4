//! RM-16, RM-21 and RM-25 computed once (spec 22, VH-7): a stream's segments, each with
//! its origin, its cut and its configuration, from the stream's initial state, its
//! Provider's terms and its commands and faults. Pure and deterministic; every instant is
//! in root ticks, a sample instant is the root tick of its sample, rounded up (MR-7), and a
//! cut is a sample index of its segment, so that it never has to be read back from a tick.
//! A stream's settings change only at the start of the Run (note 47), so a stream's items are
//! its `Stop`, `start_rx` and the Run's end.

use ezsdr_kernel::stream::{Direction, LateOutcome, LatePolicy};
use ezsdr_kernel::time::{Duration, Rational, TimeError};

/// The Kernel's `ChannelMask` limit: a logical channel number is 0…63, one bit each (S1).
const CHANNEL_LIMIT: u32 = 64;

/// The set of logical channels a stream holds (RM-21): channel numbers 0…63, each
/// held at most once. Two configurations with the same number of channels are the same
/// configuration only when their sets are (design 33 D3's2).
///
/// Ceiling: 64 channels per stream, the Kernel's `ChannelMask` limit (S1); a number at or
/// above 64 is no channel, and a set of 64 or more is all of them.
#[derive(Clone, Copy, Debug)]
pub struct Channels(u64);

impl Channels {
    /// The set `{0, …, count - 1}`, the channels of a description that numbers its channels
    /// by position: one direction's `radio.{rx,tx}.channels` (RM-4, RM-27).
    pub fn first(count: u32) -> Channels {
        Channels(if count >= CHANNEL_LIMIT { u64::MAX } else { (1 << count) - 1 })
    }

    /// The number of channels in the set.
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    /// Whether the set holds no channel.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether the set holds `channel`; a number at or above 64 is no channel.
    pub fn contains(self, channel: u32) -> bool {
        self.0 & mask(channel) != 0
    }

    /// The set with `channel` added.
    pub fn with(self, channel: u32) -> Channels {
        Channels(self.0 | mask(channel))
    }

    /// The channels in the set, smallest first.
    pub fn iter(self) -> impl Iterator<Item = u32> {
        (0..CHANNEL_LIMIT).filter(move |channel| self.contains(*channel))
    }
}

/// The bit of `channel`, 0 for a number that is not a channel.
fn mask(channel: u32) -> u64 {
    if channel < CHANNEL_LIMIT { 1 << channel } else { 0 }
}

impl PartialEq for Channels {
    fn eq(&self, other: &Channels) -> bool {
        self.0 == other.0
    }
}

impl Eq for Channels {}

/// A stream's configuration: the set of channels it holds and its root ticks per sample
/// (RM-21).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Config {
    /// The channels the stream holds, from `radio.{rx,tx}.channels` (RM-4) or from the
    /// `stream` membership of a channel of a layout description (RM-27).
    pub channels: Channels,
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
    /// The stream's own direction's `radio.timing.rx_start_lead_ns` or
    /// `radio.timing.tx_start_lead_ns`, in root ticks.
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
    /// booking (VH-4). What a transmit replay tells a command booked before its stream's end
    /// from one booked after (VH-2) is this instant, so only a Provider writes it.
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

/// One stream's commands and faults, and their plan, as a Provider books them (RM-25).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Line {
    /// The stream's initial state and its Provider's terms.
    pub stream: Stream,
    /// Every command and fault booked and not pruned, in the order booked.
    pub items: Vec<Item>,
    /// Their plan, the segments of the pruned items included.
    pub plan: Vec<Segment>,
    /// The planner's state once the pruned items have taken effect (#63).
    start: State,
}

impl Line {
    /// A stream with nothing booked: its first segment, if it has a channel.
    pub fn new(stream: Stream) -> Line {
        let start = State::new(&stream);
        let mut line = Line { stream, items: Vec::new(), plan: Vec::new(), start };
        line.plan = line.replan(&line.items).expect("a plan with no item");
        line
    }

    /// Books a command or fault and plans the stream again; returns its effective instant. A
    /// plan that cannot be represented leaves the line as it was.
    pub fn book(&mut self, item: Item) -> Result<i64, TimeError> {
        self.items.push(item);
        match self.replan(&self.items) {
            Ok(plan) => self.plan = plan,
            Err(error) => {
                self.items.pop();
                return Err(error);
            }
        }
        Ok(item.e)
    }

    /// RM-25, VH-2: the booked command `seq` was refused at its instant, and the stream is
    /// planned again; false when no such command is booked.
    pub fn refuse(&mut self, seq: u64) -> Result<bool, TimeError> {
        let Some(index) = self.items.iter().position(|item| item.seq == seq) else { return Ok(false) };
        let mut items = self.items.clone();
        items[index].refused = true;
        self.plan = self.replan(&items)?;
        self.items = items;
        Ok(true)
    }

    /// Prunes the items whose effective instant is before `before`, folding them into the
    /// state the stream is planned from, so that the plan stays as it was (#63). The caller
    /// guarantees that every item booked later takes effect at or after `before` and that
    /// no pruned item is refused later.
    pub fn prune(&mut self, before: i64) {
        let mut order = sorted(&self.items);
        let kept = order.iter().position(|item| item.e >= before).unwrap_or(order.len());
        let keep: Vec<u64> = order.split_off(kept).iter().map(|item| item.seq).collect();
        self.start.replay(&self.stream, &order).expect("the plan made these cuts already");
        self.items.retain(|item| keep.contains(&item.seq));
    }

    /// The planned segment from `origin` with `config`: a receive segment cut at its origin is
    /// not planned, and another may begin there.
    pub fn segment(&self, origin: i64, config: Config) -> Option<Segment> {
        self.plan.iter().find(|segment| segment.origin == origin && segment.config == config).copied()
    }

    /// That segment as the items in effect before `(t, seq)` have made it (RM-25): what has
    /// happened by `t`, a fault received with `seq` coming before an item at `t` received later.
    pub fn made(&self, origin: i64, config: Config, t: i64, seq: u64) -> Result<Option<Segment>, TimeError> {
        let made: Vec<Item> = self.items.iter().filter(|item| (item.e, item.seq) < (t, seq)).copied().collect();
        Ok(self.replan(&made)?.into_iter().find(|segment| segment.origin == origin && segment.config == config))
    }

    /// RM-21: the stream is streaming: the line, replayed over every item booked so far in the
    /// order they take effect, ends on. A booked `start_rx` turns it on at its effective instant,
    /// and a later-booked `Stop` that takes effect before that instant does not cancel it; a start
    /// the device refuses leaves it on (RM-25's halt). A stream that holds no channel streams
    /// nothing, so it is never streaming and takes a start as accepted and carried out by no one.
    pub fn streaming(&self) -> bool {
        let mut state = self.start.clone();
        state.replay(&self.stream, &sorted(&self.items)).expect("the plan made these cuts already");
        state.on && !self.stream.config.channels.is_empty()
    }

    fn replan(&self, items: &[Item]) -> Result<Vec<Segment>, TimeError> {
        let mut state = self.start.clone();
        state.replay(&self.stream, &sorted(items))?;
        state.finish(&self.stream)
    }
}

/// When a command received at `now` with the requested instant `at` takes effect on its
/// stream (RM-25, VH-2): `at`, or `now` when it is absent or past; on receive never before
/// the stream's start. `line` is the stream's, if it has one.
pub fn command_instant(line: Option<&Line>, at: Option<i64>, now: i64) -> i64 {
    let floor = line.filter(|line| line.stream.direction == Direction::Rx).map_or(i64::MIN, |line| line.stream.origin);
    at.unwrap_or(now).max(now).max(floor)
}

/// One stream of an instance: its direction and its number, the key a [`Lines`] holds its
/// line under (RM-29).
pub type StreamKey = (Direction, u32);

/// The lines of one Provider instance, one per stream (RM-25, RM-29).
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Lines {
    /// The instance's lines, one per stream.
    lines: Vec<(StreamKey, Line)>,
}

impl Lines {
    /// An instance with no line.
    pub fn new() -> Lines {
        Lines::default()
    }

    /// Adds `line` as `stream`'s; a later call for the same stream replaces it. Setup only: a
    /// Provider inserts its instance's lines from its layout (RM-29) before it books anything.
    pub fn insert(&mut self, stream: StreamKey, line: Line) {
        match self.lines.iter_mut().find(|(key, _)| *key == stream) {
            Some((_, held)) => *held = line,
            None => self.lines.push((stream, line)),
        }
    }

    /// The line of `stream`.
    pub fn line(&self, stream: StreamKey) -> Option<&Line> {
        self.lines.iter().find(|(key, _)| *key == stream).map(|(_, line)| line)
    }

    /// RM-25: books `item` on `stream`'s line.
    ///
    /// # Panics
    ///
    /// Panics when the instance holds no line for `stream`: its lines are the streams its layout
    /// has (RM-29).
    pub fn book(&mut self, stream: StreamKey, item: Item) -> Result<i64, TimeError> {
        self.line_mut(stream).book(item)
    }

    /// RM-25, VH-2: refuses `stream`'s booked command `seq`; false when no such command is
    /// booked.
    ///
    /// # Panics
    ///
    /// Panics when the instance holds no line for `stream`.
    pub fn refuse(&mut self, stream: StreamKey, seq: u64) -> Result<bool, TimeError> {
        self.line_mut(stream).refuse(seq)
    }

    /// RM-25: prunes `stream`'s line before `before` (#63).
    ///
    /// # Panics
    ///
    /// Panics when the instance holds no line for `stream`.
    pub fn prune_stream(&mut self, stream: StreamKey, before: i64) {
        self.line_mut(stream).prune(before);
    }

    fn line_mut(&mut self, stream: StreamKey) -> &mut Line {
        self.lines.iter_mut().find(|(key, _)| *key == stream).map(|(_, line)| line).unwrap_or_else(|| panic!("RM-29: the instance holds no line for {stream:?}"))
    }
}

/// RM-14, RM-15: the instant a burst on a transmit clock with origin `origin` is decided at: `now`,
/// or, before that clock begins, `lead` before its origin, so that a burst decided on time
/// never starts before the origin and one that would is late.
pub fn decided_at(now: i64, origin: i64, lead: i64) -> i64 {
    now.max(origin.saturating_sub(lead))
}

/// RM-15: the later of the ordinary start bound and an optional same-clock hand-over front.
pub fn later_bound(current: i64, front: Option<i64>) -> i64 {
    front.map_or(current, |front| current.max(front))
}

/// RM-15: applies a burst's late policy when a later handed front passes its target.
pub fn late_policy_front_outcome<E>(policy: LatePolicy, target: i64, current: i64, front: Option<i64>, late_by: impl FnOnce(i64) -> Result<Duration, E>) -> Result<Option<LateOutcome>, E> {
    let Some(front) = front.filter(|front| *front > current && target < *front) else { return Ok(None) };
    let late_by = late_by(front - target)?;
    Ok(Some(match policy {
        LatePolicy::RejectAtPlan => LateOutcome::PlanViolation { late_by },
        LatePolicy::SendAsapAndFlag => LateOutcome::SendAsap { late_by },
        LatePolicy::DropAndFlag => LateOutcome::Drop { late_by },
    }))
}

/// RM-4, RM-25: a stream's start lead in nanoseconds, from the TimingEnvelope of its own
/// direction. The one place the two directions' leads are told apart.
pub fn start_lead_ns(timing: &crate::TimingEnvelope, direction: Direction) -> i64 {
    match direction {
        Direction::Rx => timing.rx_start_lead_ns,
        Direction::Tx => timing.tx_start_lead_ns,
    }
}

/// RM-25, RM-6: the time a receive start's configure left before the planned `origin`, when that
/// is less than the declared command lead `min_lead` (`min_timed_command_lead_ns`): the stream
/// command can then no longer be issued with the device's own lead. A configure that ended at or
/// after the origin has missed the start, which the device's own late report names, so it is not
/// reported here. All in root ticks.
pub fn lead_left(origin: i64, completion: i64, min_lead: i64) -> Option<i64> {
    let left = origin.saturating_sub(completion);
    (completion < origin && left < min_lead).then_some(left)
}

/// RM-25: the first lattice instant of `ratio` at or after `t`, a whole multiple of its
/// numerator in lowest terms.
pub fn lattice(t: i64, ratio: Rational) -> Result<i64, TimeError> {
    let (num, _) = parts(ratio);
    narrow(ceil_div(i128::from(t), num) * num)
}

/// The segments of one stream, in order (RM-16, RM-21, RM-25). Items take effect at their
/// instants, in that order and, at one instant, in the order of their arrival. A receive
/// segment cut with no sample has no SampleClock and is left out; a transmit one is kept. A
/// transmit clock's record stands once booked (VH-2): after a loss or `Provider::stop` at `f`,
/// a clock that begins after `f` ends at its origin.
pub fn plan(stream: &Stream, items: &[Item]) -> Result<Vec<Segment>, TimeError> {
    let mut state = State::new(stream);
    state.replay(stream, &sorted(items))?;
    state.finish(stream)
}

/// The items in the order they take effect.
fn sorted(items: &[Item]) -> Vec<Item> {
    let mut order = items.to_vec();
    order.sort_by_key(|item| (item.e, item.seq));
    order
}

/// The planner's state between two items.
#[derive(Clone, PartialEq, Eq, Debug)]
struct State {
    /// The segments cut so far, and the running one.
    out: Vec<Segment>,
    running: Option<Segment>,
    /// The last cut's instant and the configuration it ended (RM-25's start rule).
    previous: Option<(i64, Config)>,
    config: Config,
    /// RM-21: a receive stream is on from the Run's start, and a refused `start_rx` leaves it on:
    /// the refusal only keeps that segment out of the plan (RM-25).
    on: bool,
    /// Transmit only: the instant of a loss or `Provider::stop`; an item booked at or after it is
    /// planned after it.
    ended: Option<i64>,
    /// Receive only: a loss or `Provider::stop` has taken effect, and nothing after it does.
    over: bool,
    /// RM-25: `running` is the receive segment a refused `start_rx` would have begun: planned
    /// from as one that runs, never in the plan.
    refused_running: bool,
}

impl State {
    fn new(stream: &Stream) -> State {
        let config = stream.config;
        let running = (!config.channels.is_empty()).then_some(Segment { origin: stream.origin, cut: None, config, by: None });
        State { out: Vec::new(), running, previous: None, config, on: true, ended: None, over: false, refused_running: false }
    }

    /// The items, in the order they take effect, take effect in turn.
    fn replay(&mut self, stream: &Stream, order: &[Item]) -> Result<(), TimeError> {
        let receive = stream.direction == Direction::Rx;
        for item in order {
            if self.over {
                break;
            }
            if self.ended.is_some_and(|end| item.ready >= end) {
                continue;
            }
            match item.kind {
                // RM-16, RM-21: receive ends at its cut, and nothing after it takes effect; on
                // transmit it ends the bursts, not the clock.
                Kind::End { abort } if receive => {
                    self.cut(stream, item, abort)?;
                    self.over = true;
                }
                Kind::End { .. } => self.ended = self.ended.or(Some(item.e)),
                Kind::Stop if receive && self.on => {
                    self.on = false;
                    self.cut(stream, item, false)?;
                }
                Kind::Start if receive && !self.on => {
                    self.on = true;
                    if !self.config.channels.is_empty() {
                        self.running = Some(self.begin(stream, item, self.config)?);
                        self.refused_running = item.refused;
                    }
                }
                Kind::Stop | Kind::Start => {}
            }
        }
        Ok(())
    }

    /// The plan the state stands for, once the items after it are in.
    fn finish(mut self, stream: &Stream) -> Result<Vec<Segment>, TimeError> {
        if self.running.zip(self.ended).is_some_and(|(segment, end)| segment.origin >= end) {
            // VH-2: a transmit clock orphaned by a loss or `Provider::stop`.
            self.end_at_origin(stream)?;
        }
        if !self.refused_running {
            self.out.extend(self.running);
        }
        Ok(self.out)
    }

    /// RM-16: the running segment ends at its cut, the first sample at or after the item's
    /// instant, never before sample 0 or the first sample not yet delivered; under `abort`,
    /// that sample itself when the Provider knows it, never before sample 0.
    fn cut(&mut self, stream: &Stream, item: &Item, abort: bool) -> Result<(), TimeError> {
        let Some(mut segment) = self.running.take() else { return Ok(()) };
        // RM-25: a segment a refusal left running has delivered nothing, so `delivered` is not its.
        let refused = std::mem::take(&mut self.refused_running);
        let cut = match (abort, item.delivered) {
            _ if refused => segment.sample_at_or_after(item.e)?,
            (true, Some(delivered)) => delivered.max(0),
            (_, delivered) => segment.sample_at_or_after(item.e)?.max(delivered.unwrap_or(i64::MIN)),
        };
        segment.cut = Some(cut);
        let instant = segment.instant(cut)?;
        self.previous = Some((instant, segment.config));
        // RM-25: a receive segment cut with no sample, or one a refusal left running, has no
        // SampleClock and is left out of the plan.
        if !refused && (cut > 0 || stream.direction != Direction::Rx) {
            self.out.push(segment);
        }
        Ok(())
    }

    /// Ends the running segment at its origin, with no sample (VH-2).
    fn end_at_origin(&mut self, stream: &Stream) -> Result<(), TimeError> {
        let Some(segment) = self.running else { return Ok(()) };
        let item = Item { e: segment.origin, seq: 0, ready: segment.origin, delivered: None, refused: false, kind: Kind::Stop };
        self.cut(stream, &item, false)
    }

    /// RM-25's start rule: the first lattice instant at or after the item's effective
    /// instant, the previous cut plus the lead, and the ready instant plus the lead. Only a
    /// receive `start_rx` begins a segment. After a cut, it is ready no earlier than one
    /// receive call at the old rate and the allowance after that cut, the call the Provider
    /// has in progress there (VH-4).
    fn begin(&self, stream: &Stream, item: &Item, config: Config) -> Result<Segment, TimeError> {
        let lead = i128::from(stream.lead);
        let mut earliest = i128::from(item.e);
        let mut ready = i128::from(item.ready);
        if let Some((cut, old)) = self.previous {
            let cut = i128::from(cut);
            earliest = earliest.max(cut + lead);
            let (num, den) = parts(old.ratio);
            let call = i128::from(stream.call).checked_mul(num).ok_or(TimeError::Overflow)?;
            ready = ready.max(cut + ceil_div(call, den) + i128::from(stream.allowance));
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
