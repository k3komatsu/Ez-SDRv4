//! The device boundary (UR-2): everything the Provider and the Authority do to a USRP
//! goes through [`Device`], whose ticks are master-clock ticks since the device's time
//! was set. [`FakeDevice`] is its wall-clock double (UR-33), constructed only by Rust
//! code (GZ-9).

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ezsdr_kernel::module_api::Fidelity;

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
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Applied {
    /// The sample rate it runs at.
    pub rate: f64,
    /// The frequency it tuned to.
    pub freq: f64,
    /// The gain it applied.
    pub gain: f64,
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
/// Authority's threads.
pub trait Device: Send + Sync {
    /// Identity for the Manifest (UR-30).
    fn describe(&self) -> serde_json::Value;
    /// What a Run on this device may claim (UR-31).
    fn fidelity(&self) -> Fidelity;
    /// The master clock rate in whole hertz.
    fn master_clock_rate(&self) -> u64;
    /// Channels in one direction.
    fn channels(&self, dir: Dir) -> usize;
    /// UHD's name of the front end serving one channel, such as `UBX RX` (UR-5).
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError>;
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
    /// Opens the receive streamer for channels `0..channels`.
    fn rx_open(&self, channels: usize) -> Result<(), DeviceError>;
    /// A timed continuous start.
    fn rx_start(&self, at: i64) -> Result<(), DeviceError>;
    /// Samples a receive packet carries (UHD's `max_num_samps`); 0 when the receive
    /// streamer is not open or packets are not known (UR-25: a stop's delivery waits for
    /// the packet holding the cut).
    fn rx_packet_samples(&self) -> usize;
    /// Stops the continuous stream, at a device tick when given.
    fn rx_stop(&self, at: Option<i64>) -> Result<(), DeviceError>;
    /// Receives up to `n` samples per channel.
    fn rx_recv(&self, n: usize, timeout: Duration) -> RxRecv;
    /// Opens the transmit streamer for channels `0..channels`.
    fn tx_open(&self, channels: usize) -> Result<(), DeviceError>;
    /// Sends one buffer per channel; returns the samples accepted per channel.
    fn tx_send(
        &self,
        samples: &[&[Iq]],
        at: Option<i64>,
        sob: bool,
        eob: bool,
        timeout: Duration,
    ) -> Result<usize, DeviceError>;
    /// The next asynchronous transmit report, if one arrives within the timeout.
    fn tx_async(&self, timeout: Duration) -> Option<TxReport>;
    /// Frees both streamers; called only when no thread holds one (UR-16).
    fn close_streams(&self);
    /// The Provider has found the device gone (UR-29): from now on neither the device
    /// nor its streamers are freed, since UHD's teardown of an unreachable X300 ends the
    /// process (design-notes §17, F4).
    fn mark_lost(&self);
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
/// rate)`, 1 ≤ `N` ≤ 512, whose rate is `mcr / N` computed in `f64` — MockRadio's rule
/// (`Profile::decimation`), so every rate the profile advertises is one the Provider
/// accepts. `N` itself is exact whatever the `f64` rate's last bit: a SampleClock's
/// ratio is `N / 1` (K13's hazard was computing `N` by float division, not the rate).
pub fn decimation(mcr: u64, rate: f64) -> Option<u64> {
    if !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    let n = (mcr as f64 / rate).round();
    if !(1.0..=512.0).contains(&n) || mcr as f64 / n != rate {
        return None;
    }
    Some(n as u64)
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
    /// From this device time on the link is dead but no call says the device is lost:
    /// control calls wait 100 ms and fail as UHD's `op_timeout` does (`UHD_ERROR_EXCEPT`,
    /// not lost), receives time out, sends take nothing (Review S, TG-S1).
    Unreachable(Duration),
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
    /// One front end per channel, each direction (at most two): its UHD name without
    /// ` RX` or ` TX`, such as `UBX` or `CBX-120`.
    pub front_ends: Vec<&'static str>,
    /// How long after its last sample a receive packet reaches `rx_recv` (the host's
    /// delivery; the bench's orderly stops put it at 2–4 ms, INFERRED; Review N, T1).
    pub rx_latency: Duration,
    /// The samples of one receive packet (UHD's samples per packet; 1 996 on the bench's
    /// X300 at MTU 9000); `None` makes a whole request one packet.
    pub rx_packet: Option<usize>,
}

impl Default for FakeConfig {
    fn default() -> Self {
        FakeConfig {
            master_clock_rate: 200_000_000,
            drift_ppm: 0.0,
            apply_delay: Duration::ZERO,
            command_queue: 16,
            loopback_delay_samples: 0,
            faults: Vec::new(),
            front_ends: vec!["UBX", "UBX"],
            rx_latency: Duration::ZERO,
            rx_packet: None,
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

struct Fake {
    zero: Instant,
    offset: i64,
    settings: [[Channel; 2]; 2],
    commands: VecDeque<(i64, Dir, usize, Settings)>,
    last_command: i64,
    rx_channels: usize,
    rx_next: Option<i64>,
    /// The running stream's root ticks a sample, fixed when it starts.
    rx_ratio: i64,
    /// A stopped stream's end: its samples before it are still delivered (the device's
    /// in-flight tail), ahead of the next stream's.
    rx_end: Option<i64>,
    /// A start issued while a stopped stream's tail is still being delivered.
    rx_then: Option<i64>,
    /// Where the running stream's packets begin: its start (UHD's packets are the
    /// stream's, not a request's; Review O, N-2).
    rx_origin: i64,
    /// The shortest timeout an `rx_recv` was given.
    rx_min_timeout: Duration,
    rx_late: Option<i64>,
    rx_last: Option<i64>,
    tx_channels: usize,
    tx_cursor: i64,
    tx_in_burst: bool,
    tx_dropped: bool,
    /// The open burst ran out of samples and was reported (UR-33: an underflow).
    tx_starved: bool,
    tx_samples: BTreeMap<i64, (i64, Vec<Iq>)>,
    /// The `tx_send` calls carrying samples so far (`FakeFault::ShortSend`).
    tx_sends: usize,
    reports: VecDeque<TxReport>,
    restarts: u64,
    unended: u64,
    fired: Vec<bool>,
    rates_seen: Vec<f64>,
    calls: Vec<String>,
}

/// A wall-clock double of a USRP, faithful in the ways the tests depend on (UR-33).
pub struct FakeDevice {
    config: FakeConfig,
    state: Mutex<Fake>,
}

const OVERRUN_BEHIND: Duration = Duration::from_millis(100);
const TX_AHEAD: Duration = Duration::from_millis(50);

impl FakeDevice {
    /// A fake with one receive and one transmit channel per front end (UR-33).
    pub fn new(config: FakeConfig) -> FakeDevice {
        assert!(config.front_ends.len() <= 2, "the fake has at most two channels");
        let channel = |antenna: &str| Channel {
            rate: 1_000_000.0,
            freq: 1_000_000_000.0,
            gain: 0.0,
            antenna: antenna.to_owned(),
        };
        let fired = vec![false; config.faults.len()];
        FakeDevice {
            state: Mutex::new(Fake {
                zero: Instant::now(),
                offset: 0,
                settings: [
                    [channel("RX2"), channel("RX2")],
                    [channel("TX/RX"), channel("TX/RX")],
                ],
                commands: VecDeque::new(),
                last_command: i64::MIN,
                rx_channels: 0,
                rx_next: None,
                rx_ratio: 1,
                rx_end: None,
                rx_then: None,
                rx_origin: 0,
                rx_min_timeout: Duration::MAX,
                rx_late: None,
                rx_last: None,
                tx_channels: 0,
                tx_cursor: i64::MIN,
                tx_in_burst: false,
                tx_dropped: false,
                tx_starved: false,
                tx_samples: BTreeMap::new(),
                tx_sends: 0,
                reports: VecDeque::new(),
                restarts: 0,
                unended: 0,
                fired,
                rates_seen: Vec::new(),
                calls: Vec::new(),
            }),
            config,
        }
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
        let ratio = self.ratio(st.settings[1][0].rate);
        (0..n as i64)
            .map(|i| {
                let t = from + i * ratio;
                let (start, (r, samples)) = st.tx_samples.range(..=t).next_back()?;
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
        let ns = st.zero.elapsed().as_nanos() as f64;
        let ticks = ns * self.config.master_clock_rate as f64 / 1e9 * (1.0 + self.config.drift_ppm * 1e-6);
        ticks as i64 + st.offset
    }

    /// Applies the timed commands now in effect (UR-33's in-order command queue).
    fn settle(&self, st: &mut Fake) {
        let now = self.now(st);
        while st.commands.front().is_some_and(|(at, ..)| *at <= now) {
            let (_, dir, chan, s) = st.commands.pop_front().expect("checked");
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
        std::thread::sleep(Duration::from_millis(100));
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
            std::thread::sleep(Duration::from_nanos((ticks as i128 * 1_000_000_000 / self.config.master_clock_rate as i128) as u64));
        }
    }

    fn ratio(&self, rate: f64) -> i64 {
        ((self.mcr() as f64 / rate).round() as i64).max(1)
    }

    fn sample_at(&self, st: &Fake, chan: usize, t: i64, rx_ratio: i64) -> Iq {
        let rx = &st.settings[0][chan];
        let tx = &st.settings[1][0];
        let tx_ratio = self.ratio(tx.rate);
        if rx.freq == tx.freq && tx_ratio == rx_ratio {
            let source = t - self.config.loopback_delay_samples * rx_ratio;
            if let Some((start, (ratio, samples))) = st.tx_samples.range(..=source).next_back() {
                let offset = source - start;
                if offset % ratio == 0 {
                    if let Some(sample) = samples.get((offset / ratio) as usize) {
                        return *sample;
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
            "rx_channels": self.config.front_ends.len(),
            "tx_channels": self.config.front_ends.len(),
            "pp_string": "FakeDevice (UR-33)",
            "uhd_version": null,
            "fake": true,
        })
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::NONE
    }

    fn master_clock_rate(&self) -> u64 {
        self.config.master_clock_rate
    }

    fn channels(&self, _dir: Dir) -> usize {
        self.config.front_ends.len()
    }

    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> {
        let name = self.config.front_ends.get(chan).ok_or_else(|| DeviceError::failed(format!("no channel {chan}")))?;
        Ok(format!("{name} {}", if dir == Dir::Rx { "RX" } else { "TX" }))
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
        st.zero = Instant::now();
        st.offset = 0;
        // A new timeline: nothing queued on the old one survives (a device's queues are
        // in its own time).
        st.commands.clear();
        st.last_command = i64::MIN;
        st.tx_cursor = i64::MIN;
        st.tx_in_burst = false;
        st.tx_dropped = false;
        st.tx_samples.clear();
        st.rx_next = None;
        st.rx_end = None;
        st.rx_then = None;
        st.rx_late = None;
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
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut st = self.lock();
        self.lost(&st)?;
        for (index, fault) in self.config.faults.iter().enumerate() {
            match fault {
                FakeFault::TimeReadFails(at) if self.now(&st) >= self.ticks_of(*at) => {
                    return Err(DeviceError::failed("fake: the time read failed"));
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
        let unlocked = self.config.faults.iter().any(|f| matches!(f, FakeFault::Unlocked(at) if now >= self.ticks_of(*at)));
        Ok(Some(!unlocked))
    }

    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        if self.unreachable() {
            return Err(self.timed_out());
        }
        if chan >= 2 {
            return Err(DeviceError::failed(format!("fake: no {} channel {chan}", dir.name())));
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
                self.settle(&mut st);
                if st.commands.len() < self.config.command_queue {
                    let now = self.now(&st);
                    let effective = at.max(st.last_command).max(now);
                    st.last_command = effective;
                    st.commands.push_back((effective, dir, chan, s.clone()));
                    st.calls.push(format!("apply {} {chan} {} at={at} effective={effective}", dir.name(), describe(s)));
                    let c = &st.settings[dir as usize][chan];
                    return Ok(Applied { rate: c.rate, freq: c.freq, gain: c.gain });
                }
                drop(st);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        std::thread::sleep(self.config.apply_delay);
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("apply {} {chan} {} at=-", dir.name(), describe(s)));
        if let Some(rate) = s.rate {
            let seen = st.rates_seen.iter().filter(|seen| **seen == rate).count();
            st.rates_seen.push(rate);
            let wrong = self.config.faults.iter().find_map(|f| match f {
                FakeFault::WrongRate { claimed, applied, nth } if *claimed == rate && *nth == seen => Some(*applied),
                _ => None,
            });
            let n = (self.mcr() as f64 / rate).round().max(1.0);
            st.settings[dir as usize][chan].rate = wrong.unwrap_or(self.mcr() as f64 / n);
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
        }
        Ok(Applied { rate: channel.rate, freq: channel.freq, gain: channel.gain })
    }

    fn rx_open(&self, channels: usize) -> Result<(), DeviceError> {
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("rx_open {channels}"));
        st.rx_channels = channels;
        // A new streamer: nothing in flight on the old one is delivered.
        st.rx_next = None;
        st.rx_end = None;
        st.rx_then = None;
        Ok(())
    }

    fn rx_start(&self, at: i64) -> Result<(), DeviceError> {
        if self.unreachable() {
            return Err(self.timed_out());
        }
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("rx_start {at}"));
        let scripted = self.config.faults.iter().position(|f| *f == FakeFault::LateStart).filter(|i| !st.fired[*i]);
        // The device reports a late start once its time reaches the start (UR-33).
        if let Some(index) = scripted {
            st.fired[index] = true;
            st.rx_late = Some(at);
        } else if at < self.now(&st) {
            st.rx_late = Some(at);
        } else if st.rx_end.is_some() {
            // Behind the stopped stream's tail, which is delivered first.
            st.rx_then = Some(at);
        } else {
            st.rx_next = Some(at);
            st.rx_origin = at;
            st.rx_ratio = self.ratio(st.settings[0][0].rate);
        }
        Ok(())
    }

    fn rx_packet_samples(&self) -> usize {
        self.config.rx_packet.unwrap_or(0)
    }

    fn rx_stop(&self, at: Option<i64>) -> Result<(), DeviceError> {
        if self.unreachable() {
            return Err(self.timed_out());
        }
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("rx_stop {}", at.map_or("now".to_owned(), |t| t.to_string())));
        // The X3x0 stops a continuous stream at once, whatever the stop's time: UHD 4.10's
        // `radio_rx_core.v` takes STOP outside its command FIFO, "timed STOP commands are
        // not supported" (VERIFIED on the bench, design-notes §11 F1). What it produced
        // before the stop is still delivered (the bench: 1–2 blocks after it).
        let now = self.now(&st);
        st.rx_then = None;
        match st.rx_next {
            Some(next) if next < now => st.rx_end = Some(st.rx_end.map_or(now, |end| end.min(now))),
            _ => {
                st.rx_next = None;
                st.rx_end = None;
            }
        }
        Ok(())
    }

    fn rx_recv(&self, n: usize, timeout: Duration) -> RxRecv {
        if self.unreachable() {
            std::thread::sleep(timeout);
            return RxRecv::Timeout;
        }
        let mut st = self.lock();
        if let Err(error) = self.lost(&st) {
            drop(st);
            std::thread::sleep(timeout.min(Duration::from_millis(10)));
            return RxRecv::Failed(error);
        }
        st.rx_min_timeout = st.rx_min_timeout.min(timeout);
        let now = self.now(&st);
        if st.rx_end.is_some_and(|end| st.rx_next.is_none_or(|next| next >= end)) {
            // The stopped stream's tail is delivered; the next stream, if one was started.
            st.rx_end = None;
            st.rx_next = None;
            if let Some(at) = st.rx_then.take() {
                if at < now {
                    st.rx_late = Some(at);
                } else {
                    st.rx_next = Some(at);
                    st.rx_origin = at;
                    st.rx_ratio = self.ratio(st.settings[0][0].rate);
                }
            }
        }
        let (mut next, ratio) = match (st.rx_late, st.rx_next) {
            (Some(at), _) if now >= at => {
                st.rx_late = None;
                return RxRecv::LateCommand;
            }
            (None, Some(next)) => (next, st.rx_ratio),
            _ => {
                // UHD's `recv` on a stopped stream waits its whole timeout.
                st.rx_next = None;
                drop(st);
                std::thread::sleep(timeout);
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
                    st.rx_next = Some(next + skip);
                    return RxRecv::Overflow { out_of_sequence: false };
                }
                FakeFault::SequenceError(_) => {
                    st.rx_next = Some(next + block);
                    return RxRecv::Overflow { out_of_sequence: true };
                }
                FakeFault::Alignment(_) => {
                    st.rx_next = Some(next + block);
                    return RxRecv::Alignment;
                }
                FakeFault::Jump(_) => {
                    next += block;
                    jumped = true;
                }
                FakeFault::Repeat(_) => {
                    if let Some(last) = st.rx_last {
                        next = last;
                    }
                }
                FakeFault::RxPanic(_) => panic!("fake: rx_recv panics"),
                FakeFault::Silence(_) => {
                    drop(st);
                    std::thread::sleep(timeout);
                    return RxRecv::Timeout;
                }
                _ => unreachable!(),
            }
        }
        let behind = self.ticks_of(OVERRUN_BEHIND);
        let mut n = n as i64;
        if let Some(stop) = st.rx_end {
            n = n.min((stop - next + ratio - 1) / ratio).max(1);
        }
        let end = next + n * ratio;
        if now - end > behind {
            st.rx_next = Some(next + ((now - next) / ratio + 1) * ratio);
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
        let origin = self.lock().rx_origin;
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
                (self.now(&st), st.rx_end)
            };
            if let Some(stop) = stop {
                // Stopped meanwhile: nothing produced after the stop ever comes.
                take = take.min(((stop - next + ratio - 1) / ratio - got).max(0));
                if take == 0 {
                    std::thread::sleep(timeout);
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
                    std::thread::sleep(timeout);
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
        if let Some(stop) = st.rx_end {
            // Stopped during the wait: nothing produced after the stop.
            got = got.min(((stop - next + ratio - 1) / ratio).max(0));
        }
        if got == 0 {
            return RxRecv::Timeout;
        }
        self.settle(&mut st);
        let channels = st.rx_channels.max(1);
        let samples = (0..channels)
            .map(|chan| (0..got).map(|i| self.sample_at(&st, chan.min(1), next + i * ratio, ratio)).collect())
            .collect();
        let _ = jumped;
        st.rx_last = Some(next);
        if st.rx_next.is_some() {
            st.rx_next = Some(next + got * ratio);
        }
        let horizon = next + got * ratio - 2 * self.mcr();
        st.tx_samples.retain(|start, (ratio, v)| start + v.len() as i64 * *ratio > horizon);
        RxRecv::Samples { first_tick: next, samples }
    }

    fn tx_open(&self, channels: usize) -> Result<(), DeviceError> {
        let mut st = self.lock();
        self.lost(&st)?;
        st.calls.push(format!("tx_open {channels}"));
        st.tx_channels = channels;
        Ok(())
    }

    fn tx_send(
        &self,
        samples: &[&[Iq]],
        at: Option<i64>,
        sob: bool,
        eob: bool,
        timeout: Duration,
    ) -> Result<usize, DeviceError> {
        if self.unreachable() {
            std::thread::sleep(timeout.min(Duration::from_millis(100)));
            return Ok(0);
        }
        if self.config.faults.contains(&FakeFault::TxBlocks) {
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        for fault in &self.config.faults {
            if let FakeFault::SlowSend(delay) = fault {
                std::thread::sleep(*delay);
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
            let ratio = self.ratio(st.settings[1][0].rate);
            st.calls.push(format!(
                "tx_send n={n} at={} sob={sob} eob={eob}",
                at.map_or("-".to_owned(), |t| t.to_string())
            ));
            if sob {
                if at.is_some() && st.tx_in_burst {
                    st.unended += 1;
                }
                match at {
                    // At the end of the samples queued is late too: the X300 reports a
                    // timed start-of-burst at the tick its previous burst ended as late
                    // and drops it (design-notes §11 F3).
                    Some(t) if t < now || t <= st.tx_cursor => {
                        // The X300 reports a tick at or after the late start (Review N, N2).
                        let tick = Some(t.max(now));
                        st.reports.push_back(TxReport { code: TxCode::TimeError, tick, channel: 0 });
                        st.tx_dropped = true;
                        st.tx_in_burst = false;
                        return Ok(n);
                    }
                    Some(t) => st.tx_cursor = t,
                    None if st.tx_in_burst => st.restarts += 1,
                    None => st.tx_cursor = st.tx_cursor.max(now),
                }
                st.tx_in_burst = true;
                st.tx_dropped = false;
                st.tx_starved = false;
            } else if st.tx_dropped {
                return Ok(n);
            } else if !st.tx_in_burst {
                st.tx_cursor = st.tx_cursor.max(now);
                st.tx_in_burst = true;
            }
            if st.tx_in_burst && n > 0 && now > st.tx_cursor && !st.tx_starved {
                // The burst ran dry before these samples came (UR-33: an underflow).
                let tick = Some(st.tx_cursor);
                st.reports.push_back(TxReport { code: TxCode::Underflow, tick, channel: 0 });
            }
            if n > 0 && st.tx_in_burst {
                st.tx_cursor = st.tx_cursor.max(now);
                st.tx_starved = false;
            }
            let start = st.tx_cursor;
            if n > 0 {
                st.tx_samples.insert(start, (ratio, samples[0].to_vec()));
            }
            st.tx_cursor = start + n as i64 * ratio;
            if eob {
                let tick = Some(st.tx_cursor);
                st.reports.push_back(TxReport { code: TxCode::BurstAck, tick, channel: 0 });
                st.tx_in_burst = false;
            }
            st.tx_cursor
        };
        // Flow control: at most 50 ms ahead of the device's time (UR-33).
        let deadline = Instant::now() + timeout;
        loop {
            let now = {
                let st = self.lock();
                self.now(&st)
            };
            let ahead = cursor - now - self.ticks_of(TX_AHEAD);
            if ahead <= 0 || Instant::now() >= deadline {
                break;
            }
            self.sleep_ticks(ahead.min(self.ticks_of(Duration::from_millis(5))));
        }
        if fail {
            return Err(DeviceError::failed("fake: the send failed after half its samples"));
        }
        Ok(n)
    }

    fn tx_async(&self, timeout: Duration) -> Option<TxReport> {
        {
            let mut st = self.lock();
            for (index, fault) in self.config.faults.iter().enumerate() {
                if let FakeFault::TxReport(at, code) = fault {
                    if self.due(&st, index, *at) {
                        st.fired[index] = true;
                        let tick = Some(self.now(&st));
                        st.reports.push_back(TxReport { code: *code, tick, channel: 0 });
                    }
                }
            }
            let now = self.now(&st);
            if st.tx_in_burst && !st.tx_dropped && !st.tx_starved && now > st.tx_cursor {
                // The open burst ran out of samples before its end-of-burst (UR-33).
                st.tx_starved = true;
                let tick = Some(st.tx_cursor);
                st.reports.push_back(TxReport { code: TxCode::Underflow, tick, channel: 0 });
            }
            // In order, and a burst's acknowledgement only once its end has played: the
            // device sends it then, and judges a later burst only after it
            // (`radio_tx_core.v:348–374`), so that burst's report waits behind (Review Q,
            // TG-Q1).
            let due = st.reports.front().is_some_and(|r| r.code != TxCode::BurstAck || r.tick.is_none_or(|t| t <= now));
            if due {
                return st.reports.pop_front();
            }
        }
        if !timeout.is_zero() {
            std::thread::sleep(timeout);
        }
        None
    }

    fn close_streams(&self) {
        self.lock().calls.push("close_streams".to_owned());
    }

    fn mark_lost(&self) {
        self.lock().calls.push("mark_lost".to_owned());
    }
}
