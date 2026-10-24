//! RM-16, RM-21 and RM-25 computed once (spec 22, VH-7): a stream's segments, each with
//! its origin, its cut and its configuration, from the stream's initial state, its
//! Provider's terms and its commands and faults. Pure and deterministic; every instant is
//! in root ticks, a sample instant is the root tick of its sample, rounded up (MR-7), and a
//! cut is a sample index of its segment, so that it never has to be read back from a tick.
//! RM-34's move of one channel from one stream to another is booked and refused here too, on
//! the two lines of one instance held together ([`Lines`]), so that neither Provider has its
//! own copy and every item of the destination at or after the move goes on waiting the
//! source's release as either plan changes (design 33 D3's9, D15). A move's release is read
//! off the segments the source runs ([`Line::released`]): the end of every segment that holds
//! the moving channel, not only the cut the move's own item makes, so that a segment a `Stop`,
//! a loss or a refusal ended later in the source's clock still floors the destination.

use ezsdr_kernel::stream::Direction;
use ezsdr_kernel::time::{Rational, TimeError};

/// The Kernel's `ChannelMask` limit: a logical channel number is 0…63, one bit each (S1).
const CHANNEL_LIMIT: u32 = 64;

/// The set of logical channels a stream holds (RM-21, RM-34): channel numbers 0…63, each
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

    /// The set with `channel` added (RM-34).
    pub fn with(self, channel: u32) -> Channels {
        Channels(self.0 | mask(channel))
    }

    /// The set without `channel` (RM-34).
    pub fn without(self, channel: u32) -> Channels {
        Channels(self.0 & !mask(channel))
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
/// (RM-21, RM-34).
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
    /// booking, or the end of the configuration an enable needs (VH-4). Never RM-34's
    /// release: what a transmit replay tells a command booked before its stream's end from
    /// one booked after (VH-2) is this instant, so only a Provider writes it.
    pub ready: i64,
    /// RM-34: the instant a move of this arrival order makes the stream this item's segment
    /// succeeds release the moved channel, so that the segment this item starts waits it. A
    /// booking writes [`NO_RELEASE`] here and only the instance's [`Lines`] writes the
    /// release; a stream that moves no channel keeps it.
    pub release: i64,
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

/// RM-34: no move's release floors an item.
pub const NO_RELEASE: i64 = i64::MIN;

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

/// RM-34: what a line's plan gives RM-34 besides its segments: the receive segments RM-25
/// leaves out of the plan, and the instant the last loss or `Provider::stop` took effect, which
/// a transmit clock runs on through.
type Planned = (Vec<Segment>, Vec<Omitted>, Option<i64>);

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
    /// RM-34: the segments RM-25 leaves out of the plan, by the item that began them: their
    /// configuration and the instant they end at, which the move that took the channel reads
    /// the release from.
    omitted: Vec<Omitted>,
    /// RM-34, VH-2: the instant the last loss or `Provider::stop` of the stream took effect, as
    /// the state the line is planned from holds it: a transmit clock runs on through it, so the
    /// segment it leaves running ends there.
    ended: Option<i64>,
}

impl Line {
    /// A stream with nothing booked: its first segment, if it has a channel.
    pub fn new(stream: Stream) -> Line {
        let start = State::new(&stream);
        let mut line = Line { stream, items: Vec::new(), plan: Vec::new(), start, omitted: Vec::new(), ended: None };
        let (plan, omitted, ended) = line.replan(&line.items).expect("a plan with no item");
        line.plan = plan;
        line.omitted = omitted;
        line.ended = ended;
        line
    }

    /// Books a command or fault and plans the stream again; returns its effective instant. A
    /// plan that cannot be represented leaves the line as it was.
    pub fn book(&mut self, item: Item) -> Result<i64, TimeError> {
        self.items.push(item);
        match self.replan(&self.items) {
            Ok((plan, omitted, ended)) => {
                self.plan = plan;
                self.omitted = omitted;
                self.ended = ended;
            }
            Err(error) => {
                self.items.pop();
                return Err(error);
            }
        }
        Ok(effective(&self.items, &item))
    }

    /// RM-25, VH-2: the booked command `seq` was refused at its instant, and the stream is
    /// planned again; false when no such command is booked.
    pub fn refuse(&mut self, seq: u64) -> Result<bool, TimeError> {
        let Some(index) = self.items.iter().position(|item| item.seq == seq) else { return Ok(false) };
        let mut items = self.items.clone();
        items[index].refused = true;
        let (plan, omitted, ended) = self.replan(&items)?;
        self.plan = plan;
        self.omitted = omitted;
        self.ended = ended;
        self.items = items;
        Ok(true)
    }

    /// RM-34: the instant this line releases `channel` for a move of it whose item is the
    /// booked command `seq` — the greatest end instant of the segments this line runs that
    /// hold the channel and began at or before that item (RM-16: a segment's end is the
    /// instant of its cut). Every such segment counts: the one the move cuts, one a `Stop`, a
    /// loss or a refusal ended, one that waited at its origin for a start, one RM-25 leaves
    /// out of the plan, one whose starting item the pruning has folded into the state the line
    /// is planned from, which began before every item the line still holds, and one a loss or
    /// `Provider::stop` left running, which ends at that end (`ended`). None when the line
    /// runs no such segment, so that no release floors the destination.
    ///
    /// # Panics
    ///
    /// Panics when the line holds no item of `seq`: a move's record and its items stand
    /// together. A segment of the source that holds the moved channel, began before the move
    /// and still runs is an error inside this rule too, unless the line has met a loss or
    /// `Provider::stop` (RM-25, VH-2: one leaves a transmit clock running): the move's item cuts
    /// the running segment otherwise.
    pub fn released(&self, channel: u32, seq: u64) -> Option<i64> {
        let moved = self.items.iter().find(|item| item.seq == seq).expect("RM-34: the line holds the item of a move it records");
        let before = (effective(&self.items, moved), moved.seq);
        let planned = self.plan.iter().map(|segment| (segment.by, segment.config, segment.cut.map(|cut| segment.instant(cut).expect("the plan made this cut"))));
        let omitted = self.omitted.iter().map(|omitted| (omitted.by, omitted.config, Some(omitted.end)));
        let mut released: Option<i64> = None;
        for (by, config, end) in planned.chain(omitted) {
            if !config.channels.contains(channel) || !self.began_before(by, before) {
                continue;
            }
            // RM-34, VH-2: a segment with no cut is one a loss or `Provider::stop` left running,
            // and the line gave the channel up when that end took effect.
            let end = end.unwrap_or_else(|| self.ended.expect("RM-34: a segment of the source that the move does not cut still runs"));
            released = Some(released.map_or(end, |was| was.max(end)));
        }
        released
    }

    /// RM-34: whether the segment a booking began takes effect at or before the instant and
    /// arrival order `before` — the move's item. A booking the pruning has folded away has no
    /// item left, and began before every item the line still holds.
    fn began_before(&self, began: Option<u64>, before: (i64, u64)) -> bool {
        match began {
            None => true,
            Some(seq) => match self.items.iter().find(|item| item.seq == seq) {
                None => true,
                Some(item) => (effective(&self.items, item), item.seq) < before,
            },
        }
    }

    /// RM-34: the booking `seq`'s release instant becomes `release` and the line is planned
    /// again; a plan that cannot be represented leaves the line as it was.
    fn set_release(&mut self, seq: u64, release: i64) -> Result<(), TimeError> {
        let mut items = self.items.clone();
        let item = items.iter_mut().find(|item| item.seq == seq).expect("RM-34: a line's item is booked on it");
        item.release = release;
        let (plan, omitted, ended) = self.replan(&items)?;
        self.plan = plan;
        self.omitted = omitted;
        self.ended = ended;
        self.items = items;
        Ok(())
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
        let made: Vec<Item> = self.items.iter().filter(|item| (effective(&self.items, item), item.seq) < (t, seq)).copied().collect();
        Ok(self.replan(&made)?.0.into_iter().find(|segment| segment.origin == origin && segment.config == config))
    }

    /// RM-25: a refusal in effect by `t` has halted the stream, and no `cold` change or
    /// `start_rx` has started it again by then; a pruned refusal counts (#64).
    pub fn halted(&self, t: i64) -> bool {
        let mut state = self.start.clone();
        let made: Vec<Item> = sorted(&self.items).into_iter().filter(|item| item.e <= t).collect();
        state.replay(&self.stream, &made).expect("the plan made these cuts already");
        state.halted
    }

    fn replan(&self, items: &[Item]) -> Result<Planned, TimeError> {
        let mut state = self.start.clone();
        state.replay(&self.stream, &sorted(items))?;
        state.finish(&self.stream)
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
        Some(line) => effective(&line.items, &Item { e: base, seq, ready: now, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(line.stream.config) }),
        None => base,
    };
    (e, asked < now || e > base)
}

/// RM-25 (design 33 D5): when a layout instance's `cold` change received at `now` with the
/// requested instant `at` takes effect: `at`, or `now` when it is absent or past; never
/// before the receive start `start`; and never before a still-pending `cold` change of the
/// instance, whichever direction, stream or line — `earlier` is those changes' effective
/// instants, and the greatest of them is the floor. The flag says whether that is later
/// than `at` asked, which makes the change late (UC-2). The one place MockRadio's layout
/// instance and the UHD Module compute this floor.
pub fn layout_instant(at: Option<i64>, now: i64, start: Option<i64>, earlier: impl IntoIterator<Item = i64>) -> (i64, bool) {
    let asked = at.unwrap_or(now);
    let base = asked.max(now).max(start.unwrap_or(i64::MIN));
    let e = earlier.into_iter().fold(base, |e, floor| e.max(floor));
    (e, asked < now || e > asked)
}

/// RM-21 (design 38 D2): when a `hardware_timed` update received at `now` with the requested
/// instant `at` takes effect, and whether that is later than `at` asked (which emits
/// `radio.LATE_COMMAND`): `at` or, when it is absent, the floor; never before the floor, which
/// is `now + lead` or the instant of a timed update booked earlier inside the release
/// `window`, whichever is later — whether that update was given, waits for a channel or the
/// device refused it. `earlier` is those booked instants. The one place both Providers compute
/// this floor.
pub fn timed_instant(at: Option<i64>, now: i64, lead: i64, window: i64, earlier: impl IntoIterator<Item = i64>) -> (i64, bool) {
    let horizon = now.saturating_add(window);
    let floor = earlier.into_iter().filter(|e| *e <= horizon).fold(now.saturating_add(lead), |floor, e| floor.max(e));
    let asked = at.unwrap_or(floor);
    (asked.max(floor), at.is_some_and(|at| at < floor))
}

/// RM-34: why a move cannot be booked; every variant leaves every line as it was.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveError {
    /// A line's plan cannot represent the move, as a booking that fails leaves its line.
    Time(TimeError),
    /// The number is not a logical channel: they are 0…63 (RM-21).
    NoSuchChannel,
    /// The source does not hold that channel at the move's instant.
    NotInSource,
    /// The destination already holds that channel at the move's instant.
    AlreadyInDestination,
    /// Both streams are one: moving a channel into its own stream changes nothing.
    SameStream,
    /// The two streams are of different directions, and a channel number names a channel of
    /// its own direction alone (RM-27).
    DifferentDirections,
}

/// One stream of an instance: its direction and its number, the key a [`Lines`] holds its
/// line under (RM-29, RM-34).
pub type StreamKey = (Direction, u32);

/// RM-34, D15: the lines of one Provider instance, so that a move's dependence is held for
/// the whole instance and not by the two lines that booked it: every item at or after a
/// move into a stream has its [`Item::release`] set to the instant the source's plan
/// releases the moved channel, so that its segment waits the release, and a booking that
/// changes either plan is answered, as often as it changes another move's source release.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Lines {
    /// The instance's lines, one per stream.
    lines: Vec<(StreamKey, Line)>,
    /// RM-34's moves, in the order booked.
    moves: Vec<Move>,
    /// RM-34: the release floor a pruning leaves on a stream, from the source's release of a
    /// move whose two items the pruning folded away; a later booking into that stream waits it.
    floors: Vec<(StreamKey, i64)>,
    /// RM-34: each stream's latest requested per-stream prune. A move's two items are folded
    /// only after both streams' requests have passed its instant.
    pruned: Vec<(StreamKey, i64)>,
}

/// RM-34: one move as its lines record it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Move {
    /// The arrival order of both lines' items, which names the move.
    seq: u64,
    /// The one instant both lines' items take effect at.
    e: i64,
    /// The channel the move takes from `from` to `to`, whose release the source's plan says.
    channel: u32,
    /// The channel leaves this stream.
    from: StreamKey,
    /// And joins this one.
    to: StreamKey,
}

impl Lines {
    /// An instance with no line.
    pub fn new() -> Lines {
        Lines::default()
    }

    /// Adds `line` as `stream`'s; a later call for the same stream replaces it. Setup only: a
    /// Provider inserts its instance's lines from its layout (RM-29) before it books anything,
    /// and no move is booked while a line is replaced.
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

    /// RM-25: books `item` on `stream`'s line, with RM-34's releases kept afterwards.
    pub fn book(&mut self, stream: StreamKey, item: Item) -> Result<i64, TimeError> {
        let mut next = self.clone();
        let e = next.line_mut(stream).book(item)?;
        next.settle()?;
        *self = next;
        Ok(e)
    }

    /// RM-25, VH-2: refuses `stream`'s booked command `seq`, with RM-34's releases kept
    /// afterwards; false when no such command is booked. A `seq` that names a move whose two
    /// lines `stream` is one of is refused on both ([`Lines::refuse_move`]).
    pub fn refuse(&mut self, stream: StreamKey, seq: u64) -> Result<bool, TimeError> {
        if let Some(moved) = self.moves.iter().find(|moved| moved.seq == seq && (moved.from == stream || moved.to == stream)).copied() {
            return self.refuse_move(moved.seq);
        }
        let mut next = self.clone();
        if !next.line_mut(stream).refuse(seq)? {
            return Ok(false);
        }
        next.settle()?;
        *self = next;
        Ok(true)
    }

    /// RM-25: prunes every line's items before `before`, and with them a move whose two items
    /// the pruning folds into the state its lines are planned from: no later booking can
    /// change the source's cut then, and the plan stays as it was. RM-34: the source's cut of
    /// a move so folded away stays as the destination stream's release floor, the greatest of
    /// them, which every item booked into that stream afterwards waits.
    pub fn prune(&mut self, before: i64) {
        let mut next = self.clone();
        let streams: Vec<StreamKey> = next.lines.iter().map(|(stream, _)| *stream).collect();
        for stream in streams {
            next.request_prune(stream, before);
        }
        next.prune_requested();
        *self = next;
    }

    /// RM-25: requests a prune of `stream`'s line before `before`; it does not request a prune
    /// of another stream. RM-34: an item of a move stays on both lines until both effective
    /// prune boundaries pass its instant, preserving later release changes and refusal.
    pub fn prune_stream(&mut self, stream: StreamKey, before: i64) {
        let mut next = self.clone();
        next.line_mut(stream);
        next.request_prune(stream, before);
        next.prune_requested();
        *self = next;
    }

    /// RM-34: books `channel` leaving `from`'s line for `to`'s, as one `Cold` item on both
    /// lines at one effective instant (design 33 D3's1). Returns that instant and whether it
    /// is later than `at` asked (UC-2). `now` is the receipt, `seq` the move's arrival order
    /// on both lines, and `ready` the instant the Provider is ready for the destination's
    /// segment; the source's release is [`Item::release`], which the lines hold apart from
    /// `ready` (VH-2). A move the lines cannot make is refused and changes neither line.
    ///
    /// # Panics
    ///
    /// Panics when the instance holds no line for a stream of a move it books: its lines are
    /// the streams its layout has (RM-29).
    #[allow(clippy::too_many_arguments)] // RM-34's operation: the two streams, the channel, its instant and terms
    pub fn book_move(
        &mut self,
        from: StreamKey,
        to: StreamKey,
        channel: u32,
        at: Option<i64>,
        now: i64,
        seq: u64,
        ready: i64,
    ) -> Result<(i64, bool), MoveError> {
        if from == to {
            return Err(MoveError::SameStream);
        }
        if from.0 != to.0 {
            return Err(MoveError::DifferentDirections);
        }
        if channel >= CHANNEL_LIMIT {
            return Err(MoveError::NoSuchChannel);
        }
        let mut next = self.clone();
        let asked = at.unwrap_or(now);
        // RM-25: the two lines' `cold` changes received before the move, and each receiving
        // line's start, floor the one instant both lines are booked at (D3's1).
        let floor = |line: &Line| {
            line.items
                .iter()
                .filter(|item| item.seq < seq && matches!(item.kind, Kind::Cold(_)))
                .fold(i64::MIN, |e, item| e.max(item.e))
        };
        let origin = |line: &Line| if line.stream.direction == Direction::Rx { line.stream.origin } else { i64::MIN };
        let base = asked.max(now).max(origin(next.line_mut(from))).max(origin(next.line_mut(to)));
        let e = base.max(floor(next.line_mut(from))).max(floor(next.line_mut(to)));
        let late = asked < now || e > base;
        let from_config = config_at(next.line_mut(from), e);
        let to_config = config_at(next.line_mut(to), e);
        // RM-34: a move the lines cannot make is refused, not booked: an unheld channel, one
        // the destination has already, or one number that is no channel would otherwise cut
        // both streams and restart them on new SampleClocks for nothing.
        if !from_config.channels.contains(channel) {
            return Err(MoveError::NotInSource);
        }
        if to_config.channels.contains(channel) {
            return Err(MoveError::AlreadyInDestination);
        }
        let from_item = Item {
            e,
            seq,
            ready: now,
            release: NO_RELEASE,
            delivered: None,
            refused: false,
            kind: Kind::Cold(Config { channels: from_config.channels.without(channel), ratio: from_config.ratio }),
        };
        let to_item = Item {
            e,
            seq,
            ready,
            release: NO_RELEASE,
            delivered: None,
            refused: false,
            kind: Kind::Cold(Config { channels: to_config.channels.with(channel), ratio: to_config.ratio }),
        };
        next.line_mut(from).book(from_item).map_err(MoveError::Time)?;
        next.line_mut(to).book(to_item).map_err(MoveError::Time)?;
        next.moves.push(Move { seq, e, from, to, channel });
        // RM-34: every item of the destination stream at or after the move is released no
        // earlier than the source's release, and a booking of either line later replans it
        // (D3's3, D3's7).
        next.settle().map_err(MoveError::Time)?;
        *self = next;
        Ok((e, late))
    }

    /// RM-34: refuses the move booked as `seq` on both lines and plans them again (RM-25's
    /// refusal, design 33 D3's4), so that they halt until their next successful `cold`
    /// change or `start_rx` and both record the configuration the move left. False, and no
    /// line changed, when `seq` names no move.
    pub fn refuse_move(&mut self, seq: u64) -> Result<bool, TimeError> {
        let Some(moved) = self.moves.iter().find(|moved| moved.seq == seq).copied() else { return Ok(false) };
        let mut next = self.clone();
        if !(next.line_mut(moved.from).refuse(seq)? && next.line_mut(moved.to).refuse(seq)?) {
            return Ok(false);
        }
        next.settle()?;
        *self = next;
        Ok(true)
    }

    /// The streams recorded by a move's `seq`, if it names one (RM-34).
    pub fn move_lines(&self, seq: u64) -> Option<(StreamKey, StreamKey)> {
        self.moves.iter().find(|moved| moved.seq == seq).map(|moved| (moved.from, moved.to))
    }

    /// RM-34: the configuration `stream`'s line leaves in effect at `e`, the state its
    /// pruned items have been folded into included; a Provider builds a later `cold`
    /// change's configuration from this.
    ///
    /// # Panics
    ///
    /// Panics when the instance holds no line for `stream`.
    pub fn config_at(&self, stream: StreamKey, e: i64) -> Config {
        config_at(self.line(stream).expect("RM-34: the instance holds a line for every stream"), e)
    }

    /// The line of `stream`.
    fn line_mut(&mut self, stream: StreamKey) -> &mut Line {
        self.lines
            .iter_mut()
            .find(|(key, _)| *key == stream)
            .map(|(_, line)| line)
            .unwrap_or_else(|| panic!("RM-34: the instance holds no line for {stream:?}"))
    }

    /// RM-34: the release floor `stream`'s line leaves, from a pruning that folded away a move
    /// into it; no floor, [`NO_RELEASE`], when the pruning folded none.
    fn floor(&self, stream: StreamKey) -> i64 {
        self.floors.iter().find(|(key, _)| *key == stream).map_or(NO_RELEASE, |(_, floor)| *floor)
    }

    /// RM-34: raises `stream`'s release floor to `cut`.
    fn raise_floor(&mut self, stream: StreamKey, cut: i64) {
        match self.floors.iter_mut().find(|(key, _)| *key == stream) {
            Some((_, floor)) => *floor = (*floor).max(cut),
            None => self.floors.push((stream, cut)),
        }
    }

    /// RM-34: remembers the greatest prune instant requested for one stream.
    fn request_prune(&mut self, stream: StreamKey, before: i64) {
        match self.pruned.iter_mut().find(|(key, _)| *key == stream) {
            Some((_, held)) => *held = (*held).max(before),
            None => self.pruned.push((stream, before)),
        }
    }

    /// The latest per-stream prune request, if any.
    fn prune_before(&self, stream: StreamKey) -> Option<i64> {
        self.pruned.iter().find(|(key, _)| *key == stream).map(|(_, before)| *before)
    }

    /// RM-34: completes requested pruning without folding one side of an unfinished move. The
    /// effective boundaries converge before a move's release is fixed as a destination floor.
    fn prune_requested(&mut self) {
        let mut boundaries: Vec<(StreamKey, i64)> = self.lines.iter().filter_map(|(stream, _)| {
            self.prune_before(*stream).map(|before| (*stream, before))
        }).collect();
        loop {
            let mut next = boundaries.clone();
            for moved in &self.moves {
                let before = |stream| boundaries.iter().find(|(key, _)| *key == stream).map(|(_, before)| *before);
                let from_passes = before(moved.from).is_some_and(|before| moved.e < before);
                let to_passes = before(moved.to).is_some_and(|before| moved.e < before);
                if from_passes && !to_passes {
                    if let Some((_, before)) = next.iter_mut().find(|(key, _)| *key == moved.from) {
                        *before = (*before).min(moved.e);
                    }
                } else if to_passes && !from_passes {
                    if let Some((_, before)) = next.iter_mut().find(|(key, _)| *key == moved.to) {
                        *before = (*before).min(moved.e);
                    }
                }
            }
            if next == boundaries { break; }
            boundaries = next;
        }
        let boundary = |stream| boundaries.iter().find(|(key, _)| *key == stream).map(|(_, before)| *before);
        let completed: Vec<(StreamKey, i64)> = self.moves.iter()
            .filter(|moved| boundary(moved.from).is_some_and(|before| moved.e < before)
                && boundary(moved.to).is_some_and(|before| moved.e < before)
                && self.line(moved.from).is_some_and(|line| line.items.iter().any(|item| item.seq == moved.seq))
                && self.line(moved.to).is_some_and(|line| line.items.iter().any(|item| item.seq == moved.seq)))
            .filter_map(|moved| self.line(moved.from).and_then(|line| line.released(moved.channel, moved.seq)).map(|cut| (moved.to, cut)))
            .collect();
        for (to, cut) in completed {
            self.raise_floor(to, cut);
        }
        for (stream, before) in boundaries {
            self.line_mut(stream).prune(before);
        }
        let lines = &self.lines;
        let holds = |stream: StreamKey, seq: u64| lines.iter().find(|(key, _)| *key == stream).is_some_and(|(_, line)| line.items.iter().any(|item| item.seq == seq));
        self.moves.retain(|moved| holds(moved.from, moved.seq) && holds(moved.to, moved.seq));
    }

    /// RM-34: after any operation, every item of every destination line at or after a move
    /// into it is released no earlier than the source's plan releases the moved channel then;
    /// a change plans the line again, which can change another move's source release, so this
    /// repeats until a round changes nothing.
    ///
    /// # Panics
    ///
    /// Panics when a round changes something after more rounds than there are moves plus
    /// one: the lines hold a dependence no plan settles, which is an error inside these two
    /// rules.
    fn settle(&mut self) -> Result<(), TimeError> {
        let mut rounds = 0;
        while self.settle_once()? {
            rounds += 1;
            assert!(rounds <= self.moves.len() + 1, "RM-34: the moves' release instants do not settle");
        }
        Ok(())
    }

    /// RM-34: one round over every line's items; whether any item's release changed. Each
    /// item's release is its stream's floor ([`Lines::prune`]) and the source's release of
    /// every move into that stream whose instant and arrival order are at or before the item's.
    fn settle_once(&mut self) -> Result<bool, TimeError> {
        let mut wanted: Vec<(usize, u64, i64)> = Vec::new();
        for index in 0..self.lines.len() {
            let (to, line) = (self.lines[index].0, &self.lines[index].1);
            let floor = self.floor(to);
            for item in &line.items {
                let at = effective(&line.items, item);
                let mut release = floor;
                for moved in &self.moves {
                    if moved.to != to || (at, item.seq) < (moved.e, moved.seq) {
                        continue;
                    }
                    let cut = self.lines.iter().find(|(key, _)| *key == moved.from).and_then(|(_, line)| line.released(moved.channel, moved.seq));
                    if let Some(cut) = cut {
                        release = release.max(cut);
                    }
                }
                if item.release != release {
                    wanted.push((index, item.seq, release));
                }
            }
        }
        let changed = !wanted.is_empty();
        for (index, seq, release) in wanted {
            self.lines[index].1.set_release(seq, release)?;
        }
        Ok(changed)
    }
}

/// RM-34: the configuration a line leaves in effect at `e`: the last of its booked `cold`
/// changes taking effect at or before it, else the state its pruned items have been folded
/// into (`Line::prune`), which starts from the stream's initial one.
fn config_at(line: &Line, e: i64) -> Config {
    let mut chosen: Option<(i64, u64, Config)> = None;
    for item in &line.items {
        let Kind::Cold(next) = item.kind else { continue };
        let at = effective(&line.items, item);
        if at <= e && chosen.is_none_or(|(was, seq, _)| (at, item.seq) > (was, seq)) {
            chosen = Some((at, item.seq, next));
        }
    }
    chosen.map_or(line.start.config, |(_, _, config)| config)
}

/// RM-14, RM-15: the instant a burst on a transmit clock with origin `origin` is decided at: `now`,
/// or, before that clock begins, `lead` before its origin, so that a burst decided on time
/// never starts before the origin and one that would is late.
pub fn decided_at(now: i64, origin: i64, lead: i64) -> i64 {
    now.max(origin.saturating_sub(lead))
}

/// RM-4, RM-25: a stream's start lead in nanoseconds, from the TimingEnvelope of its own
/// direction. The one place the two directions' leads are told apart.
pub fn start_lead_ns(timing: &crate::TimingEnvelope, direction: Direction) -> i64 {
    match direction {
        Direction::Rx => timing.rx_start_lead_ns,
        Direction::Tx => timing.tx_start_lead_ns,
    }
}

/// RM-25: the first lattice instant of `ratio` at or after `t`, a whole multiple of its
/// numerator in lowest terms.
pub fn lattice(t: i64, ratio: Rational) -> Result<i64, TimeError> {
    let (num, _) = parts(ratio);
    narrow(ceil_div(i128::from(t), num) * num)
}

/// RM-34: a receive segment RM-25 leaves out of the plan, cut with no sample: the item that
/// began it, the configuration it runs and the instant it ends at, which is its origin. A move
/// reads the release of the channel it moves from where such a segment ends too.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Omitted {
    /// The arrival order of the command that began it; none for the stream's first segment.
    by: Option<u64>,
    /// The configuration it runs.
    config: Config,
    /// The instant it ends at: the instant of its cut, which its origin is.
    end: i64,
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
    let mut state = State::new(stream);
    state.replay(stream, &sorted(items))?;
    Ok(state.finish(stream)?.0)
}

/// The items with their [`effective`] instants, in the order they take effect.
fn sorted(items: &[Item]) -> Vec<Item> {
    let mut order: Vec<Item> = items.iter().map(|item| Item { e: effective(items, item), ..*item }).collect();
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
    /// RM-21: a receive stream is on from the Run's start; a refusal halts either stream
    /// until its next `cold` change or `start_rx`.
    on: bool,
    halted: bool,
    /// Transmit only: the instant of a loss or `Provider::stop`, and of a refusal whose clock
    /// is still to end at its origin; an item booked at or after either is planned after it.
    ended: Option<i64>,
    refusal: Option<i64>,
    /// Receive only: a loss or `Provider::stop` has taken effect, and nothing after it does.
    over: bool,
    /// RM-34: the segments RM-25 leaves out of the plan, as [`Line::omitted`] holds them.
    omitted: Vec<Omitted>,
}

impl State {
    fn new(stream: &Stream) -> State {
        let config = stream.config;
        let running = (!config.channels.is_empty()).then_some(Segment { origin: stream.origin, cut: None, config, by: None });
        State { out: Vec::new(), running, previous: None, config, on: true, halted: false, ended: None, refusal: None, over: false, omitted: Vec::new() }
    }

    /// The items, sorted with their effective instants, take effect in turn.
    fn replay(&mut self, stream: &Stream, order: &[Item]) -> Result<(), TimeError> {
        let receive = stream.direction == Direction::Rx;
        for item in order {
            if self.over {
                break;
            }
            if self.refusal.is_some_and(|refused| item.ready >= refused) {
                self.end_at_origin(stream)?;
                self.refusal = None;
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
                Kind::Start if receive && (!self.on || self.halted) => {
                    self.on = true;
                    self.halted = item.refused;
                    if !self.halted && !self.config.channels.is_empty() {
                        self.running = Some(self.begin(stream, item, self.config)?);
                    }
                }
                Kind::Stop | Kind::Start => {}
                Kind::Cold(next) => {
                    self.cut(stream, item, false)?;
                    self.refusal = None;
                    self.config = next;
                    self.halted = item.refused;
                    if self.on && !next.channels.is_empty() {
                        let segment = self.begin(stream, item, next)?;
                        // RM-25: a refused change has no receive segment; the transmit clock
                        // registered for it ends at its origin.
                        if !item.refused || !receive {
                            self.running = Some(segment);
                        }
                        if item.refused && !receive {
                            self.refusal = Some(item.e);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The plan the state stands for, once the items after it are in; the segments RM-25 leaves
    /// out of it, and the instant the last loss or `Provider::stop` took effect (RM-34).
    fn finish(mut self, stream: &Stream) -> Result<Planned, TimeError> {
        if self.refusal.is_some() || self.running.zip(self.ended).is_some_and(|(segment, end)| segment.origin >= end) {
            // VH-2: a transmit clock orphaned by a loss, `Provider::stop` or a refusal.
            self.end_at_origin(stream)?;
        }
        self.out.extend(self.running);
        Ok((self.out, self.omitted, self.ended))
    }

    /// RM-16: the running segment ends at its cut, the first sample at or after the item's
    /// instant, never before sample 0 or the first sample not yet delivered; under `abort`,
    /// that sample itself when the Provider knows it, never before sample 0.
    fn cut(&mut self, stream: &Stream, item: &Item, abort: bool) -> Result<(), TimeError> {
        let Some(mut segment) = self.running.take() else { return Ok(()) };
        let cut = match (abort, item.delivered) {
            (true, Some(delivered)) => delivered.max(0),
            (_, delivered) => segment.sample_at_or_after(item.e)?.max(delivered.unwrap_or(i64::MIN)),
        };
        segment.cut = Some(cut);
        let instant = segment.instant(cut)?;
        self.previous = Some((instant, segment.config));
        if cut > 0 || stream.direction != Direction::Rx {
            self.out.push(segment);
        } else {
            // RM-25: a receive segment cut with no sample has no SampleClock and is left out of
            // the plan; RM-34 still reads the channel's release from where it ends.
            self.omitted.push(Omitted { by: segment.by, config: segment.config, end: instant });
        }
        Ok(())
    }

    /// Ends the running segment at its origin, with no sample (VH-2).
    fn end_at_origin(&mut self, stream: &Stream) -> Result<(), TimeError> {
        let Some(segment) = self.running else { return Ok(()) };
        let item = Item { e: segment.origin, seq: 0, ready: segment.origin, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Stop };
        self.cut(stream, &item, false)
    }

    /// RM-25's start rule: the first lattice instant at or after the item's effective
    /// instant, the previous cut plus the lead, and the ready instant plus the lead. RM-34
    /// adds the instant the item's stream released the moved channel, so that the segment of
    /// a move's destination, or of a stream the destination's Provider starts again after
    /// the move, waits the source's release the same way. After a cut, a receive segment is
    /// ready no earlier than that cut plus one receive call at the old rate and the
    /// allowance, the call the Provider has in progress there (VH-4).
    fn begin(&self, stream: &Stream, item: &Item, config: Config) -> Result<Segment, TimeError> {
        let lead = i128::from(stream.lead);
        let mut earliest = i128::from(item.e);
        let mut ready = i128::from(item.ready);
        if item.release != NO_RELEASE {
            earliest = earliest.max(i128::from(item.release) + lead);
        }
        if let Some((cut, old)) = self.previous {
            let cut = i128::from(cut);
            earliest = earliest.max(cut + lead);
            if stream.direction == Direction::Rx {
                let (num, den) = parts(old.ratio);
                let call = i128::from(stream.call).checked_mul(num).ok_or(TimeError::Overflow)?;
                ready = ready.max(cut + ceil_div(call, den) + i128::from(stream.allowance));
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
