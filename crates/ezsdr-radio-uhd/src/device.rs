//! The device boundary (UR-2): everything the Provider and the Authority do to a USRP
//! goes through [`Device`], whose ticks are master-clock ticks since the device's time
//! was set. [`FakeDevice`] is its wall-clock double (UR-33), constructed only by Rust
//! code (GZ-9).

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use ezsdr_kernel::module_api::Fidelity;

use crate::host::{ClockMode, HostClock};

/// One complex `fc32` sample, UHD's `std::complex<float>` (UR-2).
pub type Iq = [f32; 2];

/// A stream direction (UR-2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// Receive.
    Rx,
    /// Transmit.
    Tx,
}

impl Dir {
    /// `rx` or `tx`, as the Radio Model's keys spell it.
    pub fn name(self) -> &'static str {
        match self {
            Dir::Rx => "rx",
            Dir::Tx => "tx",
        }
    }
}

impl From<ezsdr_kernel::stream::Direction> for Dir {
    /// The device direction of a Stream Contract direction (RM-29).
    fn from(direction: ezsdr_kernel::stream::Direction) -> Dir {
        match direction {
            ezsdr_kernel::stream::Direction::Rx => Dir::Rx,
            ezsdr_kernel::stream::Direction::Tx => Dir::Tx,
        }
    }
}

/// What to change on one channel; `None` leaves a setting alone (UR-2).
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Settings {
    /// Sample rate in S/s.
    pub rate: Option<f64>,
    /// Centre frequency in Hz.
    pub freq: Option<f64>,
    /// Gain in dB.
    pub gain: Option<f64>,
    /// Antenna port.
    pub antenna: Option<String>,
}

/// What the device reports back after an untimed change (UR-2).
#[derive(Clone, PartialEq, Debug)]
pub struct Applied {
    /// The sample rate it runs at.
    pub rate: f64,
    /// The frequency it tuned to.
    pub freq: f64,
    /// The gain it applied.
    pub gain: f64,
    /// The antenna read back after a change that named one, untimed; `None` when the change
    /// named none or was timed (UR-2, design 33 D16).
    pub antenna: Option<String>,
}

/// A device call's failure; `lost` means the device is gone (UR-2, UR-29).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceError {
    /// The device is gone.
    pub lost: bool,
    /// What happened.
    pub message: String,
}

impl DeviceError {
    /// A failure that does not mean the device is gone.
    pub fn failed(message: impl Into<String>) -> DeviceError {
        DeviceError { lost: false, message: message.into() }
    }
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// One `rx_recv` outcome (UR-2; UHD's receive metadata codes, spec 18 §2).
#[derive(Clone, PartialEq, Debug)]
pub enum RxRecv {
    /// Samples, one vector per channel, all of one length ≥ 1.
    Samples {
        /// The device tick of the first sample.
        first_tick: i64,
        /// Planar per channel.
        samples: Vec<Vec<Iq>>,
    },
    /// Samples were lost; `out_of_sequence` distinguishes a sequence error.
    Overflow {
        /// A sequence error, not an overrun.
        out_of_sequence: bool,
    },
    /// A timed command (a start) arrived late.
    LateCommand,
    /// The device discarded samples to keep the channels aligned.
    Alignment,
    /// A malformed packet.
    BadPacket,
    /// Nothing arrived within the timeout.
    Timeout,
    /// The call failed.
    Failed(DeviceError),
}

/// UHD's asynchronous transmit event codes (UR-2; spec 18 §2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TxCode {
    /// A burst's end-of-burst was played.
    BurstAck,
    /// The device ran out of samples.
    Underflow,
    /// The device ran out of samples inside a packet.
    UnderflowInPacket,
    /// A sequence error.
    SeqError,
    /// A sequence error inside a burst.
    SeqErrorInBurst,
    /// A timed packet arrived late.
    TimeError,
    /// Any other code, raw.
    Other(i32),
}

/// One asynchronous transmit report (UR-2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TxReport {
    /// What happened.
    pub code: TxCode,
    /// When, in device ticks, when the device says.
    pub tick: Option<i64>,
    /// Which channel.
    pub channel: usize,
}

/// The device boundary (UR-2). Object-safe and shared by the Provider's and the
/// Authority's threads. Streams are numbered from 0 in each direction (design 33 D5), and
/// the channels of a call are the device's own channel numbers of its current subdev spec.
pub trait Device: Send + Sync {
    /// Identity for the Manifest (UR-30).
    fn describe(&self) -> serde_json::Value;
    /// What a Run on this device may claim (UR-31).
    fn fidelity(&self) -> Fidelity;
    /// The master clock rate in whole hertz.
    fn master_clock_rate(&self) -> u64;
    /// Channels in one direction: the size of its current subdev spec.
    fn channels(&self, dir: Dir) -> usize;
    /// UHD's name of the front end serving one channel, such as `UBX RX` (UR-5).
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError>;
    /// The motherboard's serial, the identity of the device itself (UR-33; design 33 D5
    /// refuses a second binding of one serial).
    fn serial(&self) -> Result<String, DeviceError>;
    /// Selects a direction's front ends as UHD's subdev spec markup, e.g. `"A:0 B:0"`:
    /// after it, `channels` and `front_end` name that spec's channels, and that direction's
    /// open streamers are gone with the channel map UHD rebuilds (UR-2, UR-33). Called with
    /// no thread holding one of that direction's streamers.
    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError>;
    /// The rates one channel can run at, as UHD's `(start, stop, step)` ranges (UR-2).
    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError>;
    /// Selects the 10 MHz and PPS sources, the clock source first (UR-7).
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError>;
    /// Sets the device's time to zero, at the next PPS edge when asked (UR-7).
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError>;
    /// Master-clock ticks since the time was set.
    fn time_now(&self) -> Result<i64, DeviceError>;
    /// Whether the reference is locked; `None` when the device has no such sensor.
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError>;
    /// Applies settings to one channel, as a timed command when `at` is given.
    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError>;
    /// Opens receive stream `stream` for the device channels `channels` (UR-2, UR-3;
    /// design 33 D5's `uhd_stream_args_t.channel_list`).
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError>;
    /// A timed continuous start of receive stream `stream`.
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError>;
    /// Samples a receive packet of stream `stream` carries (UHD's `max_num_samps`); 0 when
    /// that streamer is not open or packets are not known (UR-25: a stop's delivery waits
    /// for the packet holding the cut).
    fn rx_packet_samples(&self, stream: usize) -> usize;
    /// Stops stream `stream`'s continuous stream, at a device tick when given.
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError>;
    /// Receives up to `n` samples per channel on stream `stream`.
    fn rx_recv(&self, stream: usize, n: usize, timeout: Duration) -> RxRecv;
    /// Opens transmit stream `stream` for the device channels `channels`.
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError>;
    /// Sends one buffer per channel of stream `stream`; returns the samples accepted per
    /// channel.
    fn tx_send(
        &self,
        stream: usize,
        samples: &[&[Iq]],
        at: Option<i64>,
        sob: bool,
        eob: bool,
        timeout: Duration,
    ) -> Result<usize, DeviceError>;
    /// Stream `stream`'s next asynchronous transmit report, if one arrives within the
    /// timeout; a call that fails is an error, not the absence of a report (UR-3).
    fn tx_async(&self, stream: usize, timeout: Duration) -> Result<Option<TxReport>, DeviceError>;
    /// Frees every streamer of both directions (UR-2, UR-16); called only when no thread
    /// holds one.
    fn close_streams(&self);
    /// Everything else the Provider and the Authority read the host's time with (UR-2;
    /// spec 18 UR-33): the real clock, or a manual one a `FakeDevice` may be built with.
    fn host(&self) -> HostClock {
        real_host()
    }
    /// The Provider has found the device gone (UR-29): from now on neither the device
    /// nor its streamers are freed, since UHD's teardown of an unreachable X300 ends the
    /// process (design-notes §17, F4).
    fn mark_lost(&self);
}

/// The one real host clock per process: `Device::host`'s default and a `ClockMode::Real`
/// `FakeDevice` share an origin, instead of measuring from their own creation — so a wrapper
/// that forgets to forward `host()` still reads the same axis (UR-2, UR-33).
pub(crate) fn real_host() -> HostClock {
    static REAL: std::sync::OnceLock<HostClock> = std::sync::OnceLock::new();
    REAL.get_or_init(HostClock::real).clone()
}

/// A tick as UHD's `(full_secs, frac_secs)` (UR-3, TM-20).
pub fn to_time_spec(tick: i64, mcr: u64) -> (i64, f64) {
    let rate = mcr as i64;
    (tick.div_euclid(rate), tick.rem_euclid(rate) as f64 / mcr as f64)
}

/// A time spec as a tick, rounding to nearest with ties away from zero; an overflow
/// is a `DeviceError` (UR-3, TM-20).
pub fn from_time_spec(full: i64, frac: f64, mcr: u64) -> Result<i64, DeviceError> {
    let overflow = || DeviceError::failed("UR-3: a time spec overflows the tick count");
    let fraction = (frac * mcr as f64).round();
    if !fraction.is_finite() || fraction.abs() > i64::MAX as f64 / 2.0 {
        return Err(overflow());
    }
    full.checked_mul(mcr as i64)
        .and_then(|whole| whole.checked_add(fraction as i64))
        .ok_or_else(overflow)
}

/// UR-12 as Review L (P0-2) corrected it: the integer decimation `N = round(mcr /
/// rate)` that is one of UHD's decimations (`ezsdr_radio::x3xx::decimations`, design 33
/// D5), whose rate is `mcr / N` computed in `f64` — MockRadio's rule
/// (`Profile::decimation`), so every rate the profile advertises is one the Provider
/// accepts. `N` itself is exact whatever the `f64` rate's last bit: a SampleClock's
/// ratio is `N / 1` (K13's hazard was computing `N` by float division, not the rate).
pub fn decimation(mcr: u64, rate: f64) -> Option<u64> {
    ezsdr_radio::x3xx::decimation(mcr, rate)
}

// ---------------------------------------------------------------- the fake

/// A fault `FakeDevice` produces on command (UR-33). Instants are device time since
/// its time was set.
#[derive(Clone, Debug, PartialEq)]
pub enum FakeFault {
    /// An overrun: the receive stream skips 50 ms and reports `Overflow`.
    Overflow(Duration),
    /// A sequence error: one block lost, reported `Overflow { out_of_sequence }`.
    SequenceError(Duration),
    /// An alignment error: one block lost, reported `Alignment`.
    Alignment(Duration),
    /// One block lost with no report.
    Jump(Duration),
    /// The block after this instant is delivered twice.
    Repeat(Duration),
    /// The first receive start is reported late.
    LateStart,
    /// `rx_recv` panics.
    RxPanic(Duration),
    /// `rx_recv` returns only `Timeout`.
    Silence(Duration),
    /// Every `tx_send` blocks forever.
    TxBlocks,
    /// Every `tx_send` takes this long before the device sees it.
    SlowSend(Duration),
    /// An asynchronous transmit report.
    TxReport(Duration, TxCode),
    /// Every call fails with `lost: true`.
    Lost(Duration),
    /// `time_now` fails without the device being lost.
    TimeReadFails(Duration),
    /// `tx_async` fails from this instant on, with this `lost` (UR-28).
    TxAsyncFails(Duration, bool),
    /// `ref_locked` reports an unlocked reference.
    Unlocked(Duration),
    /// The `nth` untimed `apply` of the rate `claimed` (0-based) applies `applied`.
    WrongRate {
        /// The rate asked for.
        claimed: f64,
        /// The rate applied instead.
        applied: f64,
        /// Which occurrence.
        nth: usize,
    },
    /// The clock steps back by this many ticks, once.
    StepBack(Duration, i64),
    /// From this device time on, `time_now` takes 5 ms before it reads (UR-7's 1 ms
    /// bracket rule).
    SlowTimeRead(Duration),
    /// The `nth` `tx_send` carrying samples (0-based) takes only its first half and
    /// returns their count, its end-of-burst not sent (UHD's `send` timing out part way;
    /// Review P, NB-1).
    ShortSend(usize),
    /// The same `tx_send` takes its first half (nothing of one sample), then fails without
    /// the device being lost (UHD failing after some packets went, which is not known to
    /// happen; Review Q, NB-Q1).
    FailSend(usize),
    /// The same `tx_send` takes nothing and returns 0 (UHD's `send` timing out on its first
    /// packet; Review Q, TG-Q2).
    StalledSend(usize),
    /// `set_sources` fails as the X300 does when its reference PLL does not lock within
    /// UHD's 30 s (`x300_mb_controller.cpp:243–255`), with UHD's own text.
    ReferenceDoesNotLock,
    /// The `nth` untimed `apply` that names the antenna `claimed` (0-based) makes the
    /// channel read that back as `applied` — a front end whose getter disagrees with its
    /// setter (UR-33; design 33 D16's read-back check).
    WrongAntenna {
        /// The antenna asked for.
        claimed: &'static str,
        /// The antenna the read-back reports instead.
        applied: &'static str,
    },
    /// From this device time on the link is dead but no call says the device is lost:
    /// control calls wait 100 ms and fail as UHD's `op_timeout` does (`UHD_ERROR_EXCEPT`,
    /// not lost), receives time out, sends take nothing (Review S, TG-S1).
    Unreachable(Duration),
    /// `time_now` fails, not lost, from the first device time for the second's length, and
    /// succeeds again after it (Review T, TG-T1).
    TimeReadFailsFor(Duration, Duration),
    /// From this device time on, every `ref_locked` fails, not lost, while the time reads
    /// succeed (Review T, NB-T1).
    RefLockedFails(Duration),
    /// The first `rx_open` is refused, once: uhd-control's enable from no stream fails at its
    /// open, before any rate comparison (UR-25).
    RxOpenFails,
    /// The first receive `apply` is refused, once: uhd-control's configuration of an enable
    /// from no stream fails after its open (UR-13).
    RxApplyFails,
}

/// How [`FakeDevice`] behaves (UR-33).
#[derive(Clone, Debug)]
pub struct FakeConfig {
    /// Integer hertz.
    pub master_clock_rate: u64,
    /// How fast the fake's clock runs against the host's, in parts per million.
    pub drift_ppm: f64,
    /// The wall time an untimed `apply` takes.
    pub apply_delay: Duration,
    /// Timed commands not yet in effect that the device queue holds before it blocks.
    pub command_queue: usize,
    /// The loopback's delay in receive samples.
    pub loopback_delay_samples: i64,
    /// The scripted faults.
    pub faults: Vec<FakeFault>,
    /// One board per slot, each direction (at most two slots), the name without ` RX` or
    /// ` TX`, such as `UBX` or `CBX-120`; a Basic/LF board has its own front ends (UR-33).
    pub front_ends: Vec<&'static str>,
    /// The motherboard's serial (UR-33; design 33 D5 tells two bindings of one apart). The
    /// default is one value per `FakeDevice`, so two devices of a test binary never look
    /// like one device (UR-33).
    pub serial: String,
    /// Whether `rates` reports UHD's pre-chain answer `[(1.0, 1.0, 0.0)]` for a channel no
    /// streamer has opened yet (INFERRED, design 33 D5; BP0 measures the hardware), instead
    /// of the profile's rate grid.
    pub rates_before_open: bool,
    /// How long after its last sample a receive packet reaches `rx_recv` (the host's
    /// delivery; the bench's orderly stops put it at 2–4 ms, INFERRED; Review N, T1).
    pub rx_latency: Duration,
    /// The samples of one receive packet (UHD's samples per packet; 1 996 on the bench's
    /// X300 at MTU 9000); `None` makes a whole request one packet.
    pub rx_packet: Option<usize>,
    /// Whether its host clock is the real one or a manual one (spec 18 UR-33, plan 30 §4.4).
    pub clock: ClockMode,
}

impl Default for FakeConfig {
    fn default() -> Self {
        // UR-33: one serial per instance, so that two default `FakeDevice`s of one test
        // binary are two devices (a test binary's tests run in parallel; design 33 D5
        // refuses a second binding of one live serial).
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let serial = format!("FAKE{:07}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        FakeConfig {
            master_clock_rate: 200_000_000,
            drift_ppm: 0.0,
            apply_delay: Duration::ZERO,
            command_queue: 16,
            loopback_delay_samples: 0,
            faults: Vec::new(),
            front_ends: vec!["UBX", "UBX"],
            serial,
            rates_before_open: false,
            rx_latency: Duration::ZERO,
            rx_packet: None,
            clock: ClockMode::Real,
        }
    }
}

#[derive(Clone, Debug)]
struct Channel {
    rate: f64,
    freq: f64,
    gain: f64,
    antenna: String,
}

impl Channel {
    /// A front end's settings before anything is applied (UR-33).
    fn default_of(dir: Dir) -> Channel {
        Channel {
            rate: 1_000_000.0,
            freq: 1_000_000_000.0,
            gain: 0.0,
            antenna: if dir == Dir::Rx { "RX2" } else { "TX/RX" }.to_owned(),
        }
    }
}

/// The front ends UHD's front end numbers name on a board in `dir` (UR-33): a receive-side
/// Basic/LF board has `0` and `1`, its transmit side (`db_basic_and_lf.hpp`'s `rx_frontends`
/// and `tx_frontends`) and every other board one front end `0` per direction.
fn board_front_ends(board: &str, dir: Dir) -> Vec<u32> {
    let basic_rx = board.starts_with("BasicRX") || board.starts_with("LFRX");
    let basic_tx = board.starts_with("BasicTX") || board.starts_with("LFTX");
    match dir {
        Dir::Rx if basic_rx => vec![0, 1],
        Dir::Rx if basic_tx => Vec::new(),
        Dir::Tx if basic_tx => vec![0],
        Dir::Tx if basic_rx => Vec::new(),
        _ => vec![0],
    }
}

/// The front end table of a fake whose boards are `front_ends` (UR-33): every `(slot, front
/// end)` pair of its boards, in slot order.
fn table_of(front_ends: &[&'static str], dir: Dir) -> Vec<(usize, u32)> {
    front_ends
        .iter()
        .enumerate()
        .flat_map(|(slot, board)| board_front_ends(board, dir).into_iter().map(move |fe| (slot, fe)))
        .collect()
}

/// UHD's name of the front end `fe` of `board` in `dir` (UR-33): `<board> RX` or
/// `<board> TX` as the boards whose drivers name their subtree so (`db_ubx.cpp:562`,
/// `db_sbx_common.cpp:154`), and `BasicRX (0)` for a Basic/LF board, whose name carries the
/// front end's number (`db_basic_and_lf.cpp:101-111`).
fn front_end_name(board: &str, dir: Dir, fe: u32) -> String {
    let basic = ["BasicRX", "LFRX", "BasicTX", "LFTX"].iter().any(|prefix| board.starts_with(prefix));
    if basic {
        format!("{board} ({fe})")
    } else {
        format!("{board} {}", if dir == Dir::Rx { "RX" } else { "TX" })
    }
}

/// One open receive streamer of the fake (UR-33): its own channel list and its own stream
/// state, as UHD's streamers have.
struct RxStreamer {
    /// The device channels it was opened for, each an index into the direction's current
    /// subdev spec.
    channels: Vec<usize>,
    next: Option<i64>,
    /// The running stream's root ticks a sample, fixed when it starts.
    ratio: i64,
    /// A stopped stream's end: its samples before it are still delivered (the device's
    /// in-flight tail), ahead of the next stream's.
    end: Option<i64>,
    /// A start issued while a stopped stream's tail is still being delivered.
    then: Option<i64>,
    /// Where the running stream's packets begin: its start (UHD's packets are the
    /// stream's, not a request's; Review O, N-2).
    origin: i64,
    late: Option<i64>,
    last: Option<i64>,
}

impl RxStreamer {
    fn new(channels: Vec<usize>) -> RxStreamer {
        RxStreamer { channels, next: None, ratio: 1, end: None, then: None, origin: 0, late: None, last: None }
    }
}

/// One open transmit streamer of the fake (UR-33): its own channel list, timeline and
/// report queue (UHD's asynchronous reports are per streamer).
struct TxStreamer {
    channels: Vec<usize>,
    cursor: i64,
    in_burst: bool,
    dropped: bool,
    /// The open burst ran out of samples and was reported (UR-33: an underflow).
    starved: bool,
    reports: VecDeque<TxReport>,
}

impl TxStreamer {
    fn new(channels: Vec<usize>) -> TxStreamer {
        TxStreamer { channels, cursor: i64::MIN, in_burst: false, dropped: false, starved: false, reports: VecDeque::new() }
    }
}

struct Fake {
    zero: Duration,
    offset: i64,
    /// Each direction's current front ends (UHD's subdev spec), in channel order: a
    /// `(slot, front end)` pair per channel.
    spec: [Vec<(usize, u32)>; 2],
    /// Each direction's settings, parallel to its spec.
    settings: [Vec<Channel>; 2],
    /// Each direction's channels some streamer has been opened on (UR-33's `rates`
    /// pre-chain path and the duplicate-open refusal).
    opened: [std::collections::BTreeSet<usize>; 2],
    /// The timed commands not yet in effect, in order: the instant, the direction, the
    /// `(slot, front end)` the command was booked on — UHD's timed commands are the radio
    /// block's, so a spec change moves or drops the destination, never re-points it — and
    /// the settings.
    commands: VecDeque<(i64, Dir, (usize, u32), Settings)>,
    last_command: i64,
    rx: BTreeMap<usize, RxStreamer>,
    tx: BTreeMap<usize, TxStreamer>,
    /// The shortest timeout an `rx_recv` was given.
    rx_min_timeout: Duration,
    /// What the device transmitted per transmit channel: at each start instant, its ratio
    /// and samples.
    tx_samples: BTreeMap<usize, BTreeMap<i64, (i64, Vec<Iq>)>>,
    /// The `tx_send` calls carrying samples so far (`FakeFault::ShortSend`).
    tx_sends: usize,
    restarts: u64,
    unended: u64,
    fired: Vec<bool>,
    rates_seen: Vec<f64>,
    calls: Vec<String>,
}

/// A wall-clock double of a USRP, faithful in the ways the tests depend on (UR-33).
pub struct FakeDevice {
    config: FakeConfig,
    host: HostClock,
    state: Mutex<Fake>,
    #[cfg(test)]
    pub(crate) eob_error: Mutex<Option<DeviceError>>,
    /// The next `time_now` first sleeps this long on the host clock, once, as
    /// `FakeFault::SlowTimeRead` does from its instant on: a test's one wide bracket (UR-7).
    #[cfg(test)]
    pub(crate) slow_next_read: Mutex<Option<Duration>>,
}

const OVERRUN_BEHIND: Duration = Duration::from_millis(100);
const TX_AHEAD: Duration = Duration::from_millis(50);

impl FakeDevice {
    /// A fake with the front ends `FakeConfig::front_ends` names (UR-33): one channel per
    /// front end of each slot's board, each direction, in slot order.
    pub fn new(config: FakeConfig) -> FakeDevice {
        assert!(config.front_ends.len() <= 2, "the fake has at most two slots");
        let fired = vec![false; config.faults.len()];
        let host = match config.clock {
            ClockMode::Manual => HostClock::manual(),
            ClockMode::Real => real_host(),
        };
        let spec = [table_of(&config.front_ends, Dir::Rx), table_of(&config.front_ends, Dir::Tx)];
        let settings = [
            (0..spec[0].len()).map(|_| Channel::default_of(Dir::Rx)).collect(),
            (0..spec[1].len()).map(|_| Channel::default_of(Dir::Tx)).collect(),
        ];
        // The device's own zero (item 7): a real clock's origin is the process's, so a device
        // built later would otherwise start with time already past its creation and a
        // `Lost(d)`-style fault deadline already met. Count from this creation instead.
        let zero = host.now();
        FakeDevice {
            state: Mutex::new(Fake {
                zero,
                offset: 0,
                spec,
                settings,
                opened: [std::collections::BTreeSet::new(), std::collections::BTreeSet::new()],
                commands: VecDeque::new(),
                last_command: i64::MIN,
                rx: BTreeMap::new(),
                tx: BTreeMap::new(),
                rx_min_timeout: Duration::MAX,
                tx_samples: BTreeMap::new(),
                tx_sends: 0,
                restarts: 0,
                unended: 0,
                fired,
                rates_seen: Vec::new(),
                calls: Vec::new(),
            }),
            config,
            host,
            #[cfg(test)]
            eob_error: Mutex::new(None),
            #[cfg(test)]
            slow_next_read: Mutex::new(None),
        }
    }

    /// The device's front end table for `dir`: every `(slot, front end)` pair of its
    /// boards, in slot order (UR-33).
    fn table(&self, dir: Dir) -> Vec<(usize, u32)> {
        table_of(&self.config.front_ends, dir)
    }

    /// UHD's name of one entry of the front end table (UR-33).
    fn front_end_name(&self, dir: Dir, entry: (usize, u32)) -> String {
        front_end_name(self.config.front_ends[entry.0], dir, entry.1)
    }

    /// Parses a subdev spec markup for `dir` against the device's front end table (UR-33):
    /// UHD's `<slot>:<front end>` pairs, space-separated, each naming one table entry; an
    /// entry named twice and an empty spec are refused too, so a spec the fake takes is one
    /// UHD could stream with.
    fn parse_spec(&self, dir: Dir, markup: &str) -> Result<Vec<(usize, u32)>, DeviceError> {
        let call = format!("uhd_usrp_set_{}_subdev_spec", dir.name());
        let mut spec: Vec<(usize, u32)> = Vec::new();
        // UHD tokenizes on ASCII spaces alone (`boost::char_separator<char>(" ")`,
        // `subdev_spec.cpp:19-20`), dropping empty tokens: a tab is part of its token, so
        // `"A:0\tB:0"` is one three-part token below, not two pairs.
        for pair in markup.split(' ').filter(|pair| !pair.is_empty()) {
            // `subdev_spec_t` parses a token with one or two `:`-separated parts, a bare
            // front end having no slot, and refuses three or more (`subdev_spec.cpp:58-59`).
            let parts: Vec<&str> = pair.split(':').collect();
            let (db, sd) = match parts.len() {
                1 => ("", parts[0]),
                2 => (parts[0], parts[1]),
                _ => {
                    // That parse is `uhd_subdev_spec_make`'s, which is where the call name
                    // comes from (UR-3, UR-33).
                    return Err(DeviceError::failed(format!(
                        "uhd_subdev_spec_make: UHD error 43: ValueError: invalid subdev-spec markup string: {markup}"
                    )));
                }
            };
            let entry = self.table(dir).into_iter().find(|(slot, fe)| (if *slot == 0 { "A" } else { "B" }) == db && fe.to_string() == sd);
            // UHD searches its radios for the pair and throws a `lookup_error` when none
            // matches (`multi_usrp_rfnoc.cpp:1226-1254`).
            let Some(entry) = entry else {
                return Err(DeviceError::failed(format!(
                    "{call}: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev {db}:{sd}"
                )));
            };
            if spec.contains(&entry) {
                // INFERRED: UHD 4.10's RFNoC `set_rx_subdev_spec` registers a repeated pair
                // twice instead of refusing it (`multi_usrp_rfnoc.cpp:2796-2850`); the fake
                // refuses it so that no test streams one front end twice.
                return Err(DeviceError::failed(format!("{call}: UHD error 43: ValueError: duplicate subdev {db}:{sd}")));
            }
            spec.push(entry);
        }
        if spec.is_empty() {
            // INFERRED: UHD 4.10's RFNoC `set` would leave the device with no channel
            // instead of refusing the empty spec (`multi_usrp_rfnoc.cpp:2796`).
            return Err(DeviceError::failed(format!("{call}: UHD error 43: ValueError: empty subdev-spec markup string")));
        }
        Ok(spec)
    }

    /// Every call the fake saw, in order (UR-33).
    pub fn calls(&self) -> Vec<String> {
        self.lock().calls.clone()
    }

    /// The shortest timeout any `rx_recv` was given (UHD truncates it to whole
    /// milliseconds, so less than 1 ms would be a zero wait).
    pub fn min_recv_timeout(&self) -> Duration {
        self.lock().rx_min_timeout
    }

    /// Starts of burst without a time spec inside a burst (UR-22, RM-13's CORDIC reset).
    pub fn restarts(&self) -> u64 {
        self.lock().restarts
    }

    /// What the device transmitted at `n` transmit sample instants from root tick `from`
    /// on, channel 0, `None` where it transmitted nothing (Review P, TG-1). Kept for 2 s
    /// behind a receive stream.
    pub fn transmitted(&self, from: i64, n: usize) -> Vec<Option<Iq>> {
        let st = self.lock();
        let Some(channel) = st.settings[1].first() else { return vec![None; n] };
        let ratio = self.ratio(channel.rate);
        let empty = BTreeMap::new();
        let a_channel = st.tx_samples.get(&0).unwrap_or(&empty);
        (0..n as i64)
            .map(|i| {
                let t = from + i * ratio;
                let (start, (r, samples)) = a_channel.range(..=t).next_back()?;
                let offset = t - start;
                if offset % r == 0 { samples.get((offset / r) as usize).copied() } else { None }
            })
            .collect()
    }

    /// Timed starts of burst sent while a burst was still open: the device would start
    /// a burst inside one that never ended (UR-23; Review L, P1-4).
    pub fn unended_bursts(&self) -> u64 {
        self.lock().unended
    }

    fn lock(&self) -> MutexGuard<'_, Fake> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn mcr(&self) -> i64 {
        self.config.master_clock_rate as i64
    }

    fn ticks_of(&self, duration: Duration) -> i64 {
        (duration.as_nanos() as i128 * self.config.master_clock_rate as i128 / 1_000_000_000) as i64
    }

    fn now(&self, st: &Fake) -> i64 {
        let ns = self.host.now().saturating_sub(st.zero).as_nanos() as i128;
        // Exact integer arithmetic at the nominal rate, so that the device's count and the
        // Authority's extrapolation are the same floor of the same product, tick for tick (a
        // `f64` product truncated to `i64` can land one tick low, which the Authority's
        // integer arithmetic then sees as a one-tick offset it cannot account for). The drift
        // is a separate, small term: exactly zero at the default `drift_ppm`.
        let base = ns * self.config.master_clock_rate as i128 / 1_000_000_000;
        let drift = (base as f64 * self.config.drift_ppm * 1e-6) as i128;
        (base + drift) as i64 + st.offset
    }

    /// Applies the timed commands now in effect (UR-33's in-order command queue): each to the
    /// channel its front end serves under the spec now, so a spec change that kept that front
    /// end applies it to its new channel, and one that dropped it applies nothing.
    fn settle(&self, st: &mut Fake) {
        let now = self.now(st);
        while st.commands.front().is_some_and(|(at, ..)| *at <= now) {
            let (_, dir, dest, s) = st.commands.pop_front().expect("checked");
            let Some(chan) = st.spec[dir as usize].iter().position(|entry| *entry == dest) else { continue };
            let channel = &mut st.settings[dir as usize][chan];
            if let Some(freq) = s.freq {
                channel.freq = freq;
            }
            if let Some(gain) = s.gain {
                channel.gain = (gain * 2.0).round() / 2.0;
            }
        }
    }

    /// Whether the fault at `index` is due: `at` has passed and it has not fired.
    fn due(&self, st: &Fake, index: usize, at: Duration) -> bool {
        !st.fired[index] && self.now(st) >= self.ticks_of(at)
    }

    /// `FakeFault::Unreachable`: whether the link is dead now.
    fn unreachable(&self) -> bool {
        let st = self.lock();
        let now = self.now(&st);
        self.config.faults.iter().any(|f| matches!(f, FakeFault::Unreachable(at) if now >= self.ticks_of(*at)))
    }

    /// A control call on a dead link: it waits and fails, not lost.
    fn timed_out(&self) -> DeviceError {
        self.host.sleep(Duration::from_millis(100));
        DeviceError::failed("fake: control operation timed out waiting for ACK (UHD error 47)")
    }

    fn lost(&self, st: &Fake) -> Result<(), DeviceError> {
        let now = self.now(st);
        let gone = self.config.faults.iter().any(|f| matches!(f, FakeFault::Lost(at) if now >= self.ticks_of(*at)));
        if gone {
            Err(DeviceError { lost: true, message: "fake: the device is gone".to_owned() })
        } else {
            Ok(())
        }
    }

    fn sleep_ticks(&self, ticks: i64) {
        if ticks > 0 {
            self.host.sleep(Duration::from_nanos((ticks as i128 * 1_000_000_000 / self.config.master_clock_rate as i128) as u64));
        }
    }

    fn ratio(&self, rate: f64) -> i64 {
        ((self.mcr() as f64 / rate).round() as i64).max(1)
    }

    /// The sample the device produces on receive channel `chan` at tick `t` (UR-33): the
    /// signal transmitted on transmit channel `chan`, when the two channels' frequencies
    /// and the transmit channel's rate match, else a ramp.
    fn sample_at(&self, st: &Fake, chan: usize, t: i64, rx_ratio: i64) -> Iq {
        if let (Some(rx), Some(tx)) = (st.settings[0].get(chan), st.settings[1].get(chan)) {
            let tx_ratio = self.ratio(tx.rate);
            if rx.freq == tx.freq && tx_ratio == rx_ratio {
                let source = t - self.config.loopback_delay_samples * rx_ratio;
                if let Some((start, (ratio, samples))) = st.tx_samples.get(&chan).and_then(|map| map.range(..=source).next_back()) {
                    let offset = source - start;
                    if offset % ratio == 0 {
                        if let Some(sample) = samples.get((offset / ratio) as usize) {
                            return *sample;
                        }
                    }
                }
            }
        }
        [((t / rx_ratio).rem_euclid(65_536)) as f32 / 65_536.0, 0.0]
    }
}

impl Device for FakeDevice {
    fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "args": "fake",
            "master_clock_rate": self.config.master_clock_rate,
            "rx_channels": self.channels(Dir::Rx),
            "tx_channels": self.channels(Dir::Tx),
            "pp_string": "FakeDevice (UR-33)",
            "uhd_version": null,
            "fake": true,
        })
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::NONE
    }

    fn host(&self) -> HostClock {
        self.host.clone()
    }

    fn master_clock_rate(&self) -> u64 {
        self.config.master_clock_rate
    }

    fn channels(&self, dir: Dir) -> usize {
        self.lock().spec[dir as usize].len()
    }

    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> {
        let st = self.lock();
        let entry = st.spec[dir as usize].get(chan).copied().ok_or_else(|| DeviceError::failed(format!("no channel {chan}")))?;
        Ok(self.front_end_name(dir, entry))
    }

    fn serial(&self) -> Result<String, DeviceError> {
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push("serial".to_owned());
        Ok(self.config.serial.to_owned())
    }

    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> {
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("set_subdev_spec {} {markup}", dir.name()));
        let spec = self.parse_spec(dir, markup)?;
        // UHD rebuilds the direction's channel map: its open streamers are disconnected with
        // it (`_set_subdev_spec`'s "Disconnect and clear existing chains",
        // `multi_usrp_rfnoc.cpp:2822-2834`; the other direction's stay). No thread holds one
        // (the trait's precondition).
        match dir {
            Dir::Rx => st.rx.clear(),
            Dir::Tx => st.tx.clear(),
        }
        // A rebuilt channel map's chains are new: its channels count as unconnected again
        // (UR-33's `rates_before_open`).
        st.opened[dir as usize].clear();
        // The commands due up to now are settled under the spec this change is leaving: one
        // whose front end it drops is settled then, and the channel that front end comes back
        // on does not inherit it (UR-33).
        self.settle(&mut st);
        // A front end that stays keeps its settings; a new one starts at its defaults
        // (UR-33). The device's channels are the spec's, in its order.
        let old = std::mem::replace(&mut st.spec[dir as usize], spec);
        let old_settings = std::mem::take(&mut st.settings[dir as usize]);
        st.settings[dir as usize] = st.spec[dir as usize]
            .iter()
            .map(|entry| {
                old.iter().position(|o| o == entry).map_or_else(|| Channel::default_of(dir), |i| old_settings[i].clone())
            })
            .collect();
        Ok(())
    }

    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> {
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("rates {} {chan}", dir.name()));
        if chan >= st.spec[dir as usize].len() {
            return Err(DeviceError::failed(format!("fake: no {} channel {chan}", dir.name())));
        }
        if self.config.rates_before_open && !st.opened[dir as usize].contains(&chan) {
            // INFERRED (design 33 D5): UHD's X3x0 reports `[1.0, 1.0, 0.0]` before the
            // DDC's chain is connected; BP0 measures the hardware.
            return Ok(vec![(1.0, 1.0, 0.0)]);
        }
        Ok(ezsdr_radio::x3xx::rates(self.config.master_clock_rate).into_iter().map(|rate| (rate, rate, 0.0)).collect())
    }

    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> {
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("set_sources {clock} {time}"));
        if self.config.faults.contains(&FakeFault::ReferenceDoesNotLock) {
            return Err(DeviceError::failed(format!(
                "uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to {clock} source."
            )));
        }
        Ok(())
    }

    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> {
        let wait = {
            let st = self.lock();
            self.lost(&st)?;
            let now = self.now(&st);
            if at_next_pps { (now.div_euclid(self.mcr()) + 1) * self.mcr() - now } else { 0 }
        };
        self.sleep_ticks(wait);
        let mut st = self.lock();
        st.calls.push(format!("set_time_zero {}", if at_next_pps { "pps" } else { "now" }));
        st.zero = self.host.now();
        st.offset = 0;
        // A new timeline: nothing queued on the old one survives (a device's queues are
        // in its own time).
        st.commands.clear();
        st.last_command = i64::MIN;
        for streamer in st.tx.values_mut() {
            streamer.cursor = i64::MIN;
            streamer.in_burst = false;
            streamer.dropped = false;
        }
        st.tx_samples.clear();
        for streamer in st.rx.values_mut() {
            streamer.next = None;
            streamer.end = None;
            streamer.then = None;
            streamer.late = None;
        }
        Ok(())
    }

    fn time_now(&self) -> Result<i64, DeviceError> {
        if self.unreachable() {
            return Err(self.timed_out());
        }
        let slow = {
            let st = self.lock();
            self.config.faults.iter().any(|f| matches!(f, FakeFault::SlowTimeRead(at) if self.now(&st) >= self.ticks_of(*at)))
        };
        if slow {
            self.host.sleep(Duration::from_millis(5));
        }
        #[cfg(test)]
        {
            let once = self.slow_next_read.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(d) = once {
                self.host.sleep(d);
            }
        }
        let mut st = self.lock();
        self.lost(&st)?;
        for (index, fault) in self.config.faults.iter().enumerate() {
            match fault {
                FakeFault::TimeReadFails(at) if self.now(&st) >= self.ticks_of(*at) => {
                    return Err(DeviceError::failed("fake: the time read failed"));
                }
                FakeFault::TimeReadFailsFor(at, length)
                    if self.now(&st) >= self.ticks_of(*at) && self.now(&st) < self.ticks_of(*at + *length) =>
                {
                    return Err(DeviceError::failed("fake: the time read failed for a while"));
                }
                FakeFault::StepBack(at, ticks) if self.due(&st, index, *at) => {
                    st.fired[index] = true;
                    st.offset -= ticks;
                }
                _ => {}
            }
        }
        Ok(self.now(&st))
    }

    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> {
        if self.unreachable() {
            return Err(self.timed_out());
        }
        let st = self.lock();
        self.lost(&st)?;
        let now = self.now(&st);
        if self.config.faults.iter().any(|f| matches!(f, FakeFault::RefLockedFails(at) if now >= self.ticks_of(*at))) {
            return Err(DeviceError::failed("fake: the reference sensor read failed"));
        }
        let unlocked = self.config.faults.iter().any(|f| matches!(f, FakeFault::Unlocked(at) if now >= self.ticks_of(*at)));
        Ok(Some(!unlocked))
    }

    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        if self.unreachable() {
            return Err(self.timed_out());
        }
        let describe = |s: &Settings| {
            format!(
                "rate={} freq={} gain={} antenna={}",
                s.rate.map_or("-".to_owned(), |v| v.to_string()),
                s.freq.map_or("-".to_owned(), |v| v.to_string()),
                s.gain.map_or("-".to_owned(), |v| v.to_string()),
                s.antenna.as_deref().unwrap_or("-")
            )
        };
        if let Some(at) = at {
            // The in-order, back-pressuring queue: it blocks while full (UR-33).
            loop {
                let mut st = self.lock();
                self.lost(&st)?;
                if chan >= st.spec[dir as usize].len() {
                    return Err(DeviceError::failed(format!("fake: no {} channel {chan}", dir.name())));
                }
                self.settle(&mut st);
                if st.commands.len() < self.config.command_queue {
                    let now = self.now(&st);
                    let effective = at.max(st.last_command).max(now);
                    st.last_command = effective;
                    // The destination is the front end the channel names now — the radio
                    // block's —, not the channel number: the block outlives a spec change,
                    // the channel number does not (UR-33).
                    let dest = st.spec[dir as usize][chan];
                    st.commands.push_back((effective, dir, dest, s.clone()));
                    st.calls.push(format!("apply {} {chan} {} at={at} effective={effective}", dir.name(), describe(s)));
                    let c = &st.settings[dir as usize][chan];
                    return Ok(Applied { rate: c.rate, freq: c.freq, gain: c.gain, antenna: None });
                }
                drop(st);
                self.host.sleep(Duration::from_millis(1));
            }
        }
        self.host.sleep(self.config.apply_delay);
        let mut st = self.lock();
        self.lost(&st)?;
        if chan >= st.spec[dir as usize].len() {
            return Err(DeviceError::failed(format!("fake: no {} channel {chan}", dir.name())));
        }
        st.calls.push(format!("apply {} {chan} {} at=-", dir.name(), describe(s)));
        if dir == Dir::Rx {
            if let Some(index) = self.config.faults.iter().position(|f| *f == FakeFault::RxApplyFails).filter(|i| !st.fired[*i]) {
                st.fired[index] = true;
                return Err(DeviceError::failed("fake: scripted refusal of the first receive apply"));
            }
        }
        if let Some(rate) = s.rate {
            let seen = st.rates_seen.iter().filter(|seen| **seen == rate).count();
            st.rates_seen.push(rate);
            let wrong = self.config.faults.iter().find_map(|f| match f {
                FakeFault::WrongRate { claimed, applied, nth } if *claimed == rate && *nth == seen => Some(*applied),
                _ => None,
            });
            let n = ezsdr_radio::x3xx::clip_decimation(self.mcr() as u64, rate);
            st.settings[dir as usize][chan].rate = wrong.unwrap_or(self.mcr() as f64 / n as f64);
        }
        let channel = &mut st.settings[dir as usize][chan];
        if let Some(freq) = s.freq {
            channel.freq = freq;
        }
        if let Some(gain) = s.gain {
            channel.gain = (gain * 2.0).round() / 2.0;
        }
        if let Some(antenna) = &s.antenna {
            channel.antenna = antenna.clone();
            // UR-33 (design 33 D16's read-back): a scripted front end whose getter answers
            // another name than the one it was set to.
            if let Some(wrong) = self.config.faults.iter().find_map(|f| match f {
                FakeFault::WrongAntenna { claimed, applied } if *claimed == antenna.as_str() => Some(*applied),
                _ => None,
            }) {
                channel.antenna = wrong.to_owned();
            }
        }
        // The antenna read back after an untimed change that named one (UR-2, design 33 D16).
        let antenna = s.antenna.as_ref().map(|_| channel.antenna.clone());
        Ok(Applied { rate: channel.rate, freq: channel.freq, gain: channel.gain, antenna })
    }

    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("rx_open {stream} {channels:?}"));
        // UHD reads an empty channel list as channel 0 (`sanitize_stream_args`, UR-33), and
        // `UhdDevice` sends it as channel 0: the streamer the fake opens is the device's.
        let channels: &[usize] = if channels.is_empty() { &[0] } else { channels };
        // UR-25, as `UhdDevice` does: this stream's old streamer goes before the new one is
        // made, so a refused reopen leaves the stream with none.
        st.rx.remove(&stream);
        if let Some(index) = self.config.faults.iter().position(|f| *f == FakeFault::RxOpenFails).filter(|i| !st.fired[*i]) {
            st.fired[index] = true;
            return Err(DeviceError::failed("fake: uhd_usrp_get_rx_stream: scripted refusal of the first open"));
        }
        // UHD's graph refuses a second streamer on one front end — the radio block's output
        // port is already connected (`graph.cpp:179-184`, "Attempting to reconnect output
        // port!"; UR-33) —, the channel twice in one streamer included.
        let mut seen: Vec<usize> = Vec::new();
        for chan in channels {
            if chan >= &st.spec[Dir::Rx as usize].len() {
                return Err(DeviceError::failed(format!("fake: no rx channel {chan}")));
            }
            if seen.contains(chan) || st.rx.values().any(|streamer| streamer.channels.contains(chan)) {
                return Err(DeviceError::failed("fake: uhd_usrp_get_rx_stream: UHD error 47: RfnocError: Attempting to reconnect output port!"));
            }
            seen.push(*chan);
        }
        st.rx.insert(stream, RxStreamer::new(channels.to_vec()));
        st.opened[Dir::Rx as usize].extend(channels.iter().copied());
        Ok(())
    }

    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> {
        if self.unreachable() {
            return Err(self.timed_out());
        }
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("rx_start {stream} {at}"));
        if !st.rx.contains_key(&stream) {
            return Err(DeviceError::failed(format!("fake: receive stream {stream} is not open")));
        }
        let now = self.now(&st);
        // The running stream's ratio: its first channel's rate (UHD's one rate per streamer).
        let ratio = st.rx[&stream]
            .channels
            .first()
            .and_then(|chan| st.settings[0].get(*chan))
            .map_or(1, |channel| self.ratio(channel.rate));
        let scripted = self.config.faults.iter().position(|f| *f == FakeFault::LateStart).filter(|i| !st.fired[*i]);
        // The device reports a late start once its time reaches the start (UR-33).
        match scripted {
            Some(index) => {
                st.fired[index] = true;
                st.rx.get_mut(&stream).expect("checked").late = Some(at);
            }
            None if at < now => st.rx.get_mut(&stream).expect("checked").late = Some(at),
            None if st.rx[&stream].end.is_some() => {
                // Behind the stopped stream's tail, which is delivered first.
                st.rx.get_mut(&stream).expect("checked").then = Some(at);
            }
            None => {
                let streamer = st.rx.get_mut(&stream).expect("checked");
                streamer.next = Some(at);
                streamer.origin = at;
                streamer.ratio = ratio;
            }
        }
        Ok(())
    }

    fn rx_packet_samples(&self, stream: usize) -> usize {
        if !self.lock().rx.contains_key(&stream) {
            return 0;
        }
        self.config.rx_packet.unwrap_or(0)
    }

    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> {
        if self.unreachable() {
            return Err(self.timed_out());
        }
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("rx_stop {stream} {}", at.map_or("now".to_owned(), |t| t.to_string())));
        // The X3x0 stops a continuous stream at once, whatever the stop's time: UHD 4.10's
        // `radio_rx_core.v` takes STOP outside its command FIFO, "timed STOP commands are
        // not supported" (VERIFIED on the bench, design-notes §11 F1). What it produced
        // before the stop is still delivered (the bench: 1–2 blocks after it).
        let now = self.now(&st);
        let Some(streamer) = st.rx.get_mut(&stream) else {
            return Err(DeviceError::failed(format!("fake: receive stream {stream} is not open")));
        };
        streamer.then = None;
        match streamer.next {
            Some(next) if next < now => streamer.end = Some(streamer.end.map_or(now, |end| end.min(now))),
            _ => {
                streamer.next = None;
                streamer.end = None;
            }
        }
        Ok(())
    }

    fn rx_recv(&self, stream: usize, n: usize, timeout: Duration) -> RxRecv {
        if self.unreachable() {
            self.host.sleep(timeout);
            return RxRecv::Timeout;
        }
        let mut st = self.lock();
        if let Err(error) = self.lost(&st) {
            drop(st);
            self.host.sleep(timeout.min(Duration::from_millis(10)));
            return RxRecv::Failed(error);
        }
        if !st.rx.contains_key(&stream) {
            // As on `UhdDevice`, no streamer means no call; the wait keeps a caller that
            // measures clock time across a loop on the clock (UR-33, plan 30 §4.7).
            drop(st);
            self.host.sleep(timeout.min(Duration::from_millis(10)));
            return RxRecv::Failed(DeviceError::failed(format!("fake: receive stream {stream} is not open")));
        }
        st.rx_min_timeout = st.rx_min_timeout.min(timeout);
        let now = self.now(&st);
        // The stream's ratio: its first channel's rate (UHD's one rate per streamer).
        let ratio0 = st.rx[&stream]
            .channels
            .first()
            .and_then(|chan| st.settings[0].get(*chan))
            .map_or(1, |channel| self.ratio(channel.rate));
        {
            let streamer = st.rx.get_mut(&stream).expect("checked");
            if streamer.end.is_some_and(|end| streamer.next.is_none_or(|next| next >= end)) {
                // The stopped stream's tail is delivered; the next stream, if one was started.
                streamer.end = None;
                streamer.next = None;
                if let Some(at) = streamer.then.take() {
                    if at < now {
                        streamer.late = Some(at);
                    } else {
                        streamer.next = Some(at);
                        streamer.origin = at;
                        streamer.ratio = ratio0;
                    }
                }
            }
        }
        let (mut next, ratio) = match (st.rx[&stream].late, st.rx[&stream].next) {
            (Some(at), _) if now >= at => {
                st.rx.get_mut(&stream).expect("checked").late = None;
                return RxRecv::LateCommand;
            }
            (None, Some(next)) => (next, st.rx[&stream].ratio),
            _ => {
                // UHD's `recv` on a stopped stream waits its whole timeout.
                st.rx.get_mut(&stream).expect("checked").next = None;
                drop(st);
                self.host.sleep(timeout);
                return RxRecv::Timeout;
            }
        };
        let mut jumped = false;
        for (index, fault) in self.config.faults.iter().enumerate() {
            let at = match fault {
                FakeFault::Overflow(at)
                | FakeFault::SequenceError(at)
                | FakeFault::Alignment(at)
                | FakeFault::Jump(at)
                | FakeFault::Repeat(at)
                | FakeFault::RxPanic(at)
                | FakeFault::Silence(at) => self.ticks_of(*at),
                _ => continue,
            };
            let silent = matches!(fault, FakeFault::Silence(_));
            if next < at || (st.fired[index] && !silent) {
                continue;
            }
            st.fired[index] = true;
            let block = n as i64 * ratio;
            match fault {
                FakeFault::Overflow(_) => {
                    let skip = (self.ticks_of(Duration::from_millis(50)) + ratio - 1) / ratio * ratio;
                    st.rx.get_mut(&stream).expect("checked").next = Some(next + skip);
                    return RxRecv::Overflow { out_of_sequence: false };
                }
                FakeFault::SequenceError(_) => {
                    st.rx.get_mut(&stream).expect("checked").next = Some(next + block);
                    return RxRecv::Overflow { out_of_sequence: true };
                }
                FakeFault::Alignment(_) => {
                    st.rx.get_mut(&stream).expect("checked").next = Some(next + block);
                    return RxRecv::Alignment;
                }
                FakeFault::Jump(_) => {
                    next += block;
                    jumped = true;
                }
                FakeFault::Repeat(_) => {
                    if let Some(last) = st.rx[&stream].last {
                        next = last;
                    }
                }
                FakeFault::RxPanic(_) => panic!("fake: rx_recv panics"),
                FakeFault::Silence(_) => {
                    drop(st);
                    self.host.sleep(timeout);
                    return RxRecv::Timeout;
                }
                _ => unreachable!(),
            }
        }
        let behind = self.ticks_of(OVERRUN_BEHIND);
        let mut n = n as i64;
        if let Some(stop) = st.rx[&stream].end {
            n = n.min((stop - next + ratio - 1) / ratio).max(1);
        }
        let end = next + n * ratio;
        if now - end > behind {
            st.rx.get_mut(&stream).expect("checked").next = Some(next + ((now - next) / ratio + 1) * ratio);
            return RxRecv::Overflow { out_of_sequence: false };
        }
        drop(st);
        // Whole packets as they reach the host, each waited for up to the timeout, as
        // UHD's `recv` does; one that does not come in time cuts the call short, and the
        // next call waits as usual (UHD caches no timeout: `rx_streamer_impl.hpp`). The
        // packets are the stream's, on its own grid from its start: a sample is there
        // once the packet holding it is, and the rest of a packet a call left comes at once
        // (Review O, N-2).
        let latency = self.ticks_of(self.config.rx_latency);
        let origin = self.lock().rx[&stream].origin;
        let patience = self.ticks_of(timeout);
        let mut got = 0i64;
        while got < n {
            let k = (next - origin) / ratio + got;
            let (mut take, packet_end) = match self.config.rx_packet {
                Some(p) => {
                    let p = p.max(1) as i64;
                    let end = (k / p + 1) * p;
                    ((end - k).min(n - got), origin + end * ratio)
                }
                None => (n - got, next + n * ratio),
            };
            let (waiting, stop) = {
                let st = self.lock();
                (self.now(&st), st.rx.get(&stream).and_then(|streamer| streamer.end))
            };
            if let Some(stop) = stop {
                // Stopped meanwhile: nothing produced after the stop ever comes.
                take = take.min(((stop - next + ratio - 1) / ratio - got).max(0));
                if take == 0 {
                    self.host.sleep(timeout);
                    if got == 0 {
                        return RxRecv::Timeout;
                    }
                    break;
                }
            }
            // A stopped stream's last packet ends at the stop.
            let ready = stop.map_or(packet_end, |stop| packet_end.min(stop.max(next + (got + take) * ratio))) + latency;
            if ready - waiting > patience {
                if got == 0 {
                    self.host.sleep(timeout);
                    return RxRecv::Timeout;
                }
                break;
            }
            loop {
                let st = self.lock();
                let now = self.now(&st);
                drop(st);
                if now >= ready {
                    break;
                }
                self.sleep_ticks((ready - now).min(self.ticks_of(Duration::from_millis(5))));
            }
            got += take;
        }
        let mut st = self.lock();
        if let Some(stop) = st.rx.get(&stream).and_then(|streamer| streamer.end) {
            // Stopped during the wait: nothing produced after the stop.
            got = got.min(((stop - next + ratio - 1) / ratio).max(0));
        }
        if got == 0 {
            return RxRecv::Timeout;
        }
        self.settle(&mut st);
        let channels = st.rx[&stream].channels.clone();
        let samples = channels
            .iter()
            .map(|chan| (0..got).map(|i| self.sample_at(&st, *chan, next + i * ratio, ratio)).collect())
            .collect();
        let _ = jumped;
        {
            let streamer = st.rx.get_mut(&stream).expect("checked");
            streamer.last = Some(next);
            if streamer.next.is_some() {
                streamer.next = Some(next + got * ratio);
            }
        }
        let horizon = next + got * ratio - 2 * self.mcr();
        for channel in st.tx_samples.values_mut() {
            channel.retain(|start, (ratio, v)| start + v.len() as i64 * *ratio > horizon);
        }
        RxRecv::Samples { first_tick: next, samples }
    }

    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("tx_open {stream} {channels:?}"));
        // As on the receive side: an empty channel list is channel 0 (UR-33).
        let channels: &[usize] = if channels.is_empty() { &[0] } else { channels };
        // As on the receive side, this stream's old streamer goes before the new one is
        // made, and the graph refuses a second streamer on one front end: "Attempting to
        // reconnect input port!" (`graph.cpp:188-197`; UR-33).
        st.tx.remove(&stream);
        let mut seen: Vec<usize> = Vec::new();
        for chan in channels {
            if chan >= &st.spec[Dir::Tx as usize].len() {
                return Err(DeviceError::failed(format!("fake: no tx channel {chan}")));
            }
            if seen.contains(chan) || st.tx.values().any(|streamer| streamer.channels.contains(chan)) {
                return Err(DeviceError::failed("fake: uhd_usrp_get_tx_stream: UHD error 47: RfnocError: Attempting to reconnect input port!"));
            }
            seen.push(*chan);
        }
        st.tx.insert(stream, TxStreamer::new(channels.to_vec()));
        st.opened[Dir::Tx as usize].extend(channels.iter().copied());
        Ok(())
    }

    fn tx_send(
        &self,
        stream: usize,
        samples: &[&[Iq]],
        at: Option<i64>,
        sob: bool,
        eob: bool,
        timeout: Duration,
    ) -> Result<usize, DeviceError> {
        #[cfg(test)]
        if eob {
            if let Some(error) = self.eob_error.lock().unwrap().clone() { return Err(error); }
        }
        if self.unreachable() {
            self.host.sleep(timeout.min(Duration::from_millis(100)));
            return Ok(0);
        }
        if self.config.faults.contains(&FakeFault::TxBlocks) {
            loop {
                // A permanent host stop, not a clock wait: the participant stays busy and
                // the manual clock stays frozen (the wedged test keeps the real clock).
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        for fault in &self.config.faults {
            if let FakeFault::SlowSend(delay) = fault {
                self.host.sleep(*delay);
            }
        }
        // As `UhdDevice` refuses them before the C call (UR-3, UR-33): a send carries one
        // buffer per channel of its streamer, none shorter than the first.
        {
            let st = self.lock();
            let Some(streamer) = st.tx.get(&stream) else {
                return Err(DeviceError::failed(format!("fake: transmit stream {stream} is not open")));
            };
            if samples.len() != streamer.channels.len() {
                return Err(DeviceError::failed(format!("fake: {} buffers for {} channels", samples.len(), streamer.channels.len())));
            }
            let n = samples.first().map_or(0, |s| s.len());
            if samples.iter().any(|samples| samples.len() < n) {
                return Err(DeviceError::failed(format!("fake: a transmit channel buffer is shorter than {n} samples")));
            }
        }
        // UHD sends an empty end-of-burst as one zero sample: a CHDR packet carries at
        // least one (`tx_streamer_impl.hpp:266–276`; the bench: `hw_b8_raw_empty_eob_gap`).
        let zero: Vec<Iq> = vec![[0.0, 0.0]];
        let padded: Vec<&[Iq]> = vec![&zero; samples.len().max(1)];
        let samples = if eob && samples.first().is_none_or(|s| s.is_empty()) { &padded[..] } else { samples };
        let n = samples.first().map_or(0, |s| s.len());
        // `ShortSend`, `FailSend`, `StalledSend`: what this send takes, if it is cut.
        let cut = if n >= 1 {
            let mut st = self.lock();
            st.tx_sends += 1;
            let nth = st.tx_sends - 1;
            self.config.faults.iter().find_map(|fault| match fault {
                FakeFault::ShortSend(k) if *k == nth => Some((n / 2, false)),
                FakeFault::FailSend(k) if *k == nth => Some((n / 2, true)),
                FakeFault::StalledSend(k) if *k == nth => Some((0, false)),
                _ => None,
            })
        } else {
            None
        };
        let (short, fail, took) = match cut {
            Some((took, fail)) => (true, fail, took),
            None => (false, false, n),
        };
        if short && took == 0 {
            // `FailSend` fails even with nothing taken, a one-sample send included (Review R,
            // NB-R3).
            let what = if fail { "failed" } else { "stalled" };
            self.lock().calls.push(format!("tx_send n=0 {what} sob={sob} eob={eob}"));
            return if fail { Err(DeviceError::failed("fake: the send failed after half its samples")) } else { Ok(0) };
        }
        let half: Vec<&[Iq]> = samples.iter().map(|s| &s[..took]).collect();
        let (samples, eob) = if short { (&half[..], false) } else { (samples, eob) };
        let n = samples.first().map_or(0, |s| s.len());
        let cursor = {
            let mut st = self.lock();
            self.lost(&st)?;
            self.settle(&mut st);
            let now = self.now(&st);
            // The stream's ratio: its first channel's rate (UHD's one rate per streamer).
            let ratio = st.tx.get(&stream)
                .and_then(|streamer| streamer.channels.first().copied())
                .and_then(|chan| st.settings[1].get(chan))
                .map_or(1, |channel| self.ratio(channel.rate));
            let Fake { tx, tx_samples, restarts, unended, calls, .. } = &mut *st;
            calls.push(format!(
                "tx_send {stream} n={n} at={} sob={sob} eob={eob}",
                at.map_or("-".to_owned(), |t| t.to_string())
            ));
            let Some(streamer) = tx.get_mut(&stream) else {
                return Err(DeviceError::failed(format!("fake: transmit stream {stream} is not open")));
            };
            if sob {
                if at.is_some() && streamer.in_burst {
                    *unended += 1;
                }
                match at {
                    // At the end of the samples queued is late too: the X300 reports a
                    // timed start-of-burst at the tick its previous burst ended as late
                    // and drops it (design-notes §11 F3).
                    Some(t) if t < now || t <= streamer.cursor => {
                        // The X300 reports a tick at or after the late start (Review N, N2).
                        let tick = Some(t.max(now));
                        for channel in 0..streamer.channels.len() {
                            streamer.reports.push_back(TxReport { code: TxCode::TimeError, tick, channel });
                        }
                        streamer.dropped = true;
                        streamer.in_burst = false;
                        return Ok(n);
                    }
                    Some(t) => streamer.cursor = t,
                    None if streamer.in_burst => *restarts += 1,
                    None => streamer.cursor = streamer.cursor.max(now),
                }
                streamer.in_burst = true;
                streamer.dropped = false;
                streamer.starved = false;
            } else if streamer.dropped {
                return Ok(n);
            } else if !streamer.in_burst {
                streamer.cursor = streamer.cursor.max(now);
                streamer.in_burst = true;
            }
            if streamer.in_burst && n > 0 && now > streamer.cursor && !streamer.starved {
                // The burst ran dry before these samples came (UR-33: an underflow).
                let tick = Some(streamer.cursor);
                streamer.reports.push_back(TxReport { code: TxCode::Underflow, tick, channel: 0 });
            }
            if n > 0 && streamer.in_burst {
                streamer.cursor = streamer.cursor.max(now);
                streamer.starved = false;
            }
            let start = streamer.cursor;
            if n > 0 {
                for (position, chan) in streamer.channels.iter().enumerate() {
                    if let Some(samples) = samples.get(position) {
                        tx_samples.entry(*chan).or_default().insert(start, (ratio, samples.to_vec()));
                    }
                }
            }
            streamer.cursor = start + n as i64 * ratio;
            if eob {
                let tick = Some(streamer.cursor);
                for channel in 0..streamer.channels.len() {
                    streamer.reports.push_back(TxReport { code: TxCode::BurstAck, tick, channel });
                }
                streamer.in_burst = false;
            }
            streamer.cursor
        };
        // Flow control: at most 50 ms ahead of the device's time (UR-33).
        let deadline = self.host.now() + timeout;
        loop {
            let now = {
                let st = self.lock();
                self.now(&st)
            };
            let ahead = cursor - now - self.ticks_of(TX_AHEAD);
            if ahead <= 0 || self.host.now() >= deadline {
                break;
            }
            self.sleep_ticks(ahead.min(self.ticks_of(Duration::from_millis(5))));
        }
        if fail {
            return Err(DeviceError::failed("fake: the send failed after half its samples"));
        }
        Ok(n)
    }

    fn tx_async(&self, stream: usize, timeout: Duration) -> Result<Option<TxReport>, DeviceError> {
        {
            let mut st = self.lock();
            // As on `UhdDevice`, no streamer means no call, and no call fails; its queue is
            // the streamer's, so there is none to read (UR-33).
            if st.tx.contains_key(&stream) {
                self.lost(&st)?;
                let now = self.now(&st);
                for fault in &self.config.faults {
                    if let FakeFault::TxAsyncFails(at, lost) = fault {
                        if now >= self.ticks_of(*at) {
                            return Err(DeviceError { lost: *lost, message: "fake: uhd_tx_streamer_recv_async_msg failed".to_owned() });
                        }
                    }
                }
                let mut scripted = Vec::new();
                for (index, fault) in self.config.faults.iter().enumerate() {
                    if let FakeFault::TxReport(at, code) = fault {
                        if self.due(&st, index, *at) {
                            st.fired[index] = true;
                            scripted.push(*code);
                        }
                    }
                }
                let now = self.now(&st);
                let streamer = st.tx.get_mut(&stream).expect("checked");
                for code in scripted {
                    streamer.reports.push_back(TxReport { code, tick: Some(now), channel: 0 });
                }
                if streamer.in_burst && !streamer.dropped && !streamer.starved && now > streamer.cursor {
                    // The open burst ran out of samples before its end-of-burst (UR-33).
                    streamer.starved = true;
                    let tick = Some(streamer.cursor);
                    streamer.reports.push_back(TxReport { code: TxCode::Underflow, tick, channel: 0 });
                }
                // In order, and a burst's acknowledgement only once its end has played: the
                // device sends it then, and judges a later burst only after it
                // (`radio_tx_core.v:348–374`), so that burst's report waits behind (Review Q,
                // TG-Q1).
                let due = streamer.reports.front().is_some_and(|r| r.code != TxCode::BurstAck || r.tick.is_none_or(|t| t <= now));
                if due {
                    return Ok(streamer.reports.pop_front());
                }
            }
        }
        // A read that finds no report waits its timeout, the shutdown's ACK window's
        // (`Tx::shutdown`): no streamer is no reason for a caller to spin (UR-33, plan 30
        // §4.7; `Tx::reports`' failed read does the same).
        if !timeout.is_zero() {
            self.host.sleep(timeout);
        }
        Ok(None)
    }

    fn close_streams(&self) {
        // UR-2, UR-16: every streamer of both directions, as UHD frees each of a device's.
        let mut st = self.lock();
        st.calls.push("close_streams".to_owned());
        st.rx.clear();
        st.tx.clear();
    }

    fn mark_lost(&self) {
        self.lock().calls.push("mark_lost".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::mpsc::channel;

    #[test]
    fn ur_33_a_real_fake_s_time_counts_from_its_own_creation() {
        // R07: a real-clock fake's zero — the origin of its time and of its faults' deadlines
        // — is the process-wide real clock's reading at its creation, bracketed by readings
        // just before and after it, not that clock's origin. A bracket, not a bound: it holds
        // however long this thread is descheduled (R12).
        let host = real_host();
        std::thread::sleep(Duration::from_millis(2));
        let before = host.now();
        let fake = FakeDevice::new(FakeConfig::default());
        let after = host.now();
        let zero = fake.lock().zero;
        assert!(before <= zero && zero <= after, "zero {zero:?} is not its creation, within [{before:?}, {after:?}]");
    }

    // The fake's receive strictness (UR-33; Review N, T2), each scenario run on participants of
    // the fake's manual clock (M1): the device's time moves only while they wait on it, so its
    // instants do not depend on the host's scheduling, and the wall clock is only the guard
    // (plan 30 D2: these were integration tests whose plain test threads a loaded host delayed).

    /// Root ticks of `n` milliseconds on the fake's 200 MHz master clock.
    fn ms(n: i64) -> i64 {
        n * 200_000
    }

    /// A fake on a manual host clock.
    fn manual(config: FakeConfig) -> Arc<FakeDevice> {
        Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..config }))
    }

    /// The receive link as the bench measured it (INFERRED from its stops: 2–4 ms), with the
    /// X300's 1 996-sample packets.
    fn bench_link(latency_ms: u64) -> FakeConfig {
        FakeConfig { rx_latency: Duration::from_millis(latency_ms), rx_packet: Some(1_996), ..FakeConfig::default() }
    }

    /// Runs `f` on a participant of `device`'s clock and returns what it returns, failing after a
    /// wall bound instead of hanging on a clock that freezes.
    fn participant<T: Send + 'static>(device: &Arc<FakeDevice>, f: impl FnOnce(Arc<FakeDevice>) -> T + Send + 'static) -> T {
        const BOUND: Duration = Duration::from_secs(10);
        let (done, result) = channel();
        let receiver = device.clone();
        device.host().spawn("receiver", move || {
            let _ = done.send(f(receiver));
        }).unwrap();
        result.recv_timeout(BOUND).unwrap_or_else(|e| panic!("the receiver's result within {BOUND:?}: {e}"))
    }

    /// Opens channel 0 at 1 MS/s as stream 0 and starts it 5 000 samples ahead; returns the
    /// start.
    fn start_rx(device: &FakeDevice) -> i64 {
        device.set_time_zero(false).unwrap();
        device.apply(Dir::Rx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        device.rx_open(0, &[0]).unwrap();
        let start = (device.time_now().unwrap() / 200 + 5_000) * 200;
        device.rx_start(0, start).unwrap();
        start
    }

    /// Every block's (first tick, length) until `until` or two timeouts in a row.
    fn blocks_until(device: &FakeDevice, until: impl Fn(&[(i64, usize)]) -> bool) -> Vec<(i64, usize)> {
        let mut blocks = Vec::new();
        let mut timeouts = 0;
        while timeouts < 2 && !until(&blocks) {
            match device.rx_recv(0, 2_000, Duration::from_millis(50)) {
                RxRecv::Samples { first_tick, samples } => {
                    timeouts = 0;
                    blocks.push((first_tick, samples[0].len()));
                }
                RxRecv::Timeout => timeouts += 1,
                other => panic!("{other:?}"),
            }
        }
        blocks
    }

    /// The tick after the last sample of `blocks` (at 1 MS/s), 0 when there are none.
    fn end_of(blocks: &[(i64, usize)]) -> i64 {
        blocks.last().map_or(0, |(t, n)| t + *n as i64 * 200)
    }

    #[test]
    fn ur_33_a_timed_receive_stop_stops_the_stream_at_once() {
        // design-notes §11 F1: the X3x0 ignores the stop's time. What was produced before the
        // stop is still delivered; nothing after it. The receiver falls 5 ms of the device's time
        // behind before it issues a stop timed 100 ms ahead, so the stream holds samples produced
        // before the stop.
        let device = manual(FakeConfig::default());
        let (behind, issued, tail) = participant(&device, |device| {
            let start = start_rx(&device);
            let blocks = blocks_until(&device, |b| end_of(b) >= start + ms(20));
            device.host().sleep(Duration::from_millis(5));
            let issued = device.time_now().unwrap();
            device.rx_stop(0, Some(issued + ms(100))).unwrap();
            (end_of(&blocks), issued, blocks_until(&device, |_| false))
        });
        assert_eq!(tail.first().map(|(t, _)| *t), Some(behind), "the tail begins where the stream was: {tail:?}");
        assert_eq!(end_of(&tail), issued, "the tail ends at the stop's issue, not at its time: {tail:?}");
    }

    #[test]
    fn ur_33_a_stopped_stream_s_tail_comes_before_the_next_stream() {
        // The bench: 1–2 blocks after an untimed stop. The stop's tail, then the next stream from
        // its start. On the bench's 3 ms link the receiver is behind the device when it stops.
        let device = manual(bench_link(3));
        let (stopped, next, blocks) = participant(&device, |device| {
            let start = start_rx(&device);
            let _ = blocks_until(&device, |b| end_of(b) >= start + ms(20));
            let stopped = device.time_now().unwrap();
            device.rx_stop(0, None).unwrap();
            let next = (stopped / 200 + 30_000) * 200;
            device.rx_start(0, next).unwrap();
            (stopped, next, blocks_until(&device, |b| b.iter().any(|(t, _)| *t >= next)))
        });
        let tail: Vec<_> = blocks.iter().filter(|(t, _)| *t < stopped).collect();
        assert!(!tail.is_empty(), "no tail: {blocks:?}");
        let first_new = blocks.iter().position(|(t, _)| *t >= next).expect("the next stream");
        assert!(blocks[..first_new].iter().all(|(t, _)| *t < stopped), "{blocks:?}");
        assert_eq!(blocks[first_new].0, next);
    }

    #[test]
    fn ur_33_a_stopped_stream_s_recv_waits_its_whole_timeout() {
        // design-notes §11 F2: UHD's `recv` on a stopped stream waits its whole timeout, here on
        // the device's own clock.
        let device = manual(FakeConfig::default());
        let (got, waited) = participant(&device, |device| {
            let _ = start_rx(&device);
            device.rx_stop(0, None).unwrap();
            let _ = blocks_until(&device, |_| false);
            let begun = device.host().now();
            let got = device.rx_recv(0, 2_000, Duration::from_millis(60));
            (got, device.host().now() - begun)
        });
        assert_eq!(got, RxRecv::Timeout);
        assert!(waited >= Duration::from_millis(60), "the stopped stream's call waited {waited:?} of the device's clock");
    }

    #[test]
    fn ur_33_a_recv_cut_short_by_a_packet_s_timeout_returns_what_it_has() {
        // UHD's `recv` waits for each packet up to its timeout and returns what it has; it caches
        // no timeout, so the next call waits as usual (`rx_streamer_impl.hpp:25–47`; Review O,
        // N-1). A stream on the bench's link stopped 8 ms into a 20 ms request, by a second
        // participant: the stop lands 8 ms of device time after the request began, inside it —
        // a plain thread's stopper let a loaded host finish the request first (plan 30 D2).
        let device = manual(bench_link(1));
        let (from, stop, cut, next, waited) = participant(&device, |device| {
            let start = start_rx(&device);
            let from = end_of(&blocks_until(&device, |b| end_of(b) >= start + ms(10)));
            // This participant is busy, so the clock stands still until both the stopper's sleep
            // and the request's first packet wait have begun: the stop is 8 ms after this read.
            let stop = device.time_now().unwrap() + ms(8);
            let (stopper, clock) = (device.clone(), device.host());
            let stopping = device.host().spawn("stopper", move || {
                clock.sleep(Duration::from_millis(8));
                stopper.rx_stop(0, None).unwrap();
            }).unwrap();
            let cut = device.rx_recv(0, 20_000, Duration::from_millis(5));
            stopping.join().unwrap();
            let begun = device.host().now();
            let next = device.rx_recv(0, 2_000, Duration::from_millis(50));
            (from, stop, cut, next, device.host().now() - begun)
        });
        let RxRecv::Samples { first_tick, samples } = cut else { panic!("the request cut short returned {cut:?}") };
        let end = first_tick + samples[0].len() as i64 * 200;
        // Every sample produced before the stop, from where the stream was, and none after it.
        assert!(first_tick == from && end >= stop && end - 200 < stop, "the request cut short returned [{first_tick}, {end}) of the stream at {from} stopped at {stop}");
        assert_eq!(next, RxRecv::Timeout, "the call after it");
        assert!(waited >= Duration::from_millis(50), "the call after it waited {waited:?} of the device's clock, not its whole timeout");
    }

    #[test]
    fn ur_33_a_command_due_while_its_front_end_was_out_never_applies() {
        // UR-33: a spec change settles the commands due up to it under the spec it is leaving,
        // so a command whose instant passes while its front end is out is dropped — the channel
        // that front end comes back on starts at its defaults —, while one not yet due at the
        // return still applies to that front end's channel at its instant. One participant, so
        // the host's load cannot move the clock between the booking and the spec changes.
        let dir = Dir::Rx;
        // Due while slot B was out of the spec: settled under "A:0" and dropped.
        let device = manual(FakeConfig::default());
        let dropped = participant(&device, move |device| {
            let set = |freq: f64| Settings { freq: Some(freq), ..Settings::default() };
            device.set_time_zero(false).unwrap();
            device.apply(dir, 1, &set(2e9), Some(device.time_now().unwrap() + ms(5))).unwrap();
            device.set_subdev_spec(dir, "A:0").unwrap();
            device.host().sleep(Duration::from_millis(10));
            device.set_subdev_spec(dir, "A:0 B:0").unwrap();
            // A no-op timed change runs the queue's settle first.
            device.apply(dir, 0, &Settings::default(), Some(device.time_now().unwrap() + ms(50))).unwrap();
            device.apply(dir, 1, &Settings::default(), None).unwrap().freq
        });
        assert_eq!(dropped, 1e9, "a command due while its front end was out applied after its return");
        // Not yet due at the return: it stays in the queue and applies under the spec then.
        let device = manual(FakeConfig::default());
        let kept = participant(&device, move |device| {
            let set = |freq: f64| Settings { freq: Some(freq), ..Settings::default() };
            device.set_time_zero(false).unwrap();
            device.apply(dir, 1, &set(2e9), Some(device.time_now().unwrap() + ms(200))).unwrap();
            device.set_subdev_spec(dir, "A:0").unwrap();
            device.set_subdev_spec(dir, "A:0 B:0").unwrap();
            device.host().sleep(Duration::from_millis(210));
            device.apply(dir, 0, &Settings::default(), Some(device.time_now().unwrap() + ms(50))).unwrap();
            device.apply(dir, 1, &Settings::default(), None).unwrap().freq
        });
        assert_eq!(kept, 2e9, "a command not yet due did not apply to the front end that came back");
    }

    #[test]
    fn ur_33_a_recv_on_a_stream_with_no_streamer_waits_its_10_ms() {
        // UR-33: a call naming a stream no streamer is open for does not return at once as
        // `UhdDevice`'s does — `rx_recv` waits at most 10 ms of its host clock and then fails
        // —, so a caller that measures clock time across a loop waits on the clock in every
        // pass (plan 30 §4.7). The manual clock makes the wait exact.
        let device = manual(FakeConfig::default());
        let (got, waited) = participant(&device, |device| {
            let begun = device.host().now();
            let got = device.rx_recv(0, 2_000, Duration::from_millis(60));
            (got, device.host().now() - begun)
        });
        assert!(matches!(got, RxRecv::Failed(_)), "{got:?}");
        assert_eq!(waited, Duration::from_millis(10), "the failed call waited {waited:?} of the device's clock, not its 10 ms");
    }

    #[test]
    fn ur_33_a_report_read_on_a_stream_with_no_streamer_waits_its_timeout() {
        // UR-33: `tx_async` with no streamer is a read that finds no report after its whole
        // timeout, as the shutdown's ACK reads do (UR-26, plan 30 §4.7).
        let device = manual(FakeConfig::default());
        let (report, waited) = participant(&device, |device| {
            let begun = device.host().now();
            let report = device.tx_async(0, Duration::from_millis(60)).unwrap();
            (report, device.host().now() - begun)
        });
        assert!(report.is_none(), "{report:?}");
        assert_eq!(waited, Duration::from_millis(60), "the read waited {waited:?} of the device's clock, not its timeout");
    }

    #[test]
    fn ur_33_a_packet_size_read_of_a_stream_with_no_streamer_is_zero() {
        // UR-33: `rx_packet_samples` answers 0 for a stream with no streamer, as `UhdDevice`'s
        // does, and the configured packet size once one is open.
        let device = manual(FakeConfig { rx_packet: Some(1_996), ..FakeConfig::default() });
        assert_eq!(device.rx_packet_samples(0), 0, "no streamer, no packet size");
        device.rx_open(0, &[0]).unwrap();
        assert_eq!(device.rx_packet_samples(0), 1_996, "the open streamer's packet size");
    }
}
