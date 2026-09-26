//! The device underneath the Provider and the Authority: a real USRP through the UHD
//! C API, or a wall-clock fake for checking the Kernel wiring without hardware.
//!
//! Ticks are the device's master-clock ticks since its time was set; every
//! conversion to UHD's `(full_secs, frac_secs)` happens here and nowhere else.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::ffi::*;

/// One complex `fc32` sample, laid out as UHD's `std::complex<float>`.
pub type Iq = [f32; 2];

/// A stream direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// Receive.
    Rx,
    /// Transmit.
    Tx,
}

/// What to change on one channel; `None` leaves a setting alone.
#[derive(Clone, Default, Debug)]
pub struct Settings {
    /// Sample rate in S/s.
    pub rate: Option<f64>,
    /// Centre frequency in Hz.
    pub freq: Option<f64>,
    /// Overall gain in dB.
    pub gain: Option<f64>,
    /// Antenna port name.
    pub antenna: Option<String>,
}

/// What the device reports back after a change.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    /// Actual sample rate.
    pub rate: f64,
    /// Actual centre frequency (RF LO plus DSP shift).
    pub freq: f64,
    /// Actual gain.
    pub gain: f64,
}

/// One `recv` outcome.
pub enum RxRecv {
    /// Samples, planar per channel, starting at a device tick.
    Samples {
        /// Device tick of the first sample.
        first_tick: i64,
        /// One vector per channel, all the same length.
        samples: Vec<Vec<Iq>>,
    },
    /// UHD's `OVERFLOW` (the `O`): samples were lost; the next packet's time jumps.
    Overflow {
        /// UHD's out-of-sequence flag: `true` is a sequence error (`D`), not an overrun.
        out_of_sequence: bool,
    },
    /// Nothing arrived within the timeout.
    Timeout,
    /// Any other error code (late command, broken chain, alignment, bad packet).
    Error(String),
}

/// An asynchronous transmit report.
#[derive(Clone, Debug, PartialEq)]
pub struct TxEvent {
    /// UHD's event code name.
    pub code: &'static str,
    /// When, in device ticks, if the device said.
    pub tick: Option<i64>,
    /// Which channel.
    pub channel: usize,
}

/// The operations the spike needs from a device.
pub trait Device: Send + Sync {
    /// Identity, for the Manifest.
    fn describe(&self) -> Value;
    /// Master-clock ticks per second (integer; refused otherwise).
    fn tick_rate(&self) -> u64;
    /// The device's current time in ticks.
    fn time_now(&self) -> Result<i64, String>;
    /// Selects the reference clock and time sources (`internal`, `external`, `gpsdo`).
    fn set_sources(&self, clock: Option<&str>, time: Option<&str>) -> Result<(), String>;
    /// Sets device time to zero: now, or at the next PPS edge (blocks up to ~2 s).
    fn set_time_zero(&self, at_unknown_pps: bool) -> Result<(), String>;
    /// Channels the device offers in one direction.
    fn channels(&self, dir: Dir) -> usize;
    /// Applies settings to one channel, as a timed command when `at` is given.
    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, String>;
    /// Creates the receive streamer for channels `0..n`.
    fn rx_open(&self, n: usize) -> Result<(), String>;
    /// Starts continuous receive at a device tick.
    fn rx_start(&self, at: i64) -> Result<(), String>;
    /// Receives up to `n` samples per channel.
    fn rx_recv(&self, n: usize, timeout_s: f64) -> RxRecv;
    /// Stops continuous receive.
    fn rx_stop(&self) -> Result<(), String>;
    /// Creates the transmit streamer for channels `0..n`.
    fn tx_open(&self, n: usize) -> Result<(), String>;
    /// Sends one buffer per channel; returns samples accepted per channel.
    fn tx_send(&self, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout_s: f64)
    -> Result<usize, String>;
    /// The next asynchronous transmit report, if one arrives within the timeout.
    fn tx_async(&self, timeout_s: f64) -> Option<TxEvent>;
    /// Frees both streamers. Called only after every thread using them has stopped.
    fn close_streams(&self);
}

// ---------------------------------------------------------------- UHD

fn last_error() -> String {
    let mut buf = [0 as c_char; 1024];
    unsafe {
        uhd_get_last_error(buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

fn check(what: &str, code: uhd_error) -> Result<(), String> {
    if code == UHD_ERROR_NONE { Ok(()) } else { Err(format!("{what}: UHD error {code}: {}", last_error())) }
}

fn cstring(s: &str) -> CString {
    CString::new(s).expect("no interior NUL")
}

/// Lists devices matching `args` (the C API's `uhd_usrp_find`).
pub fn find(args: &str) -> Result<Vec<String>, String> {
    let mut v: uhd_string_vector_handle = std::ptr::null_mut();
    let args = cstring(args);
    unsafe {
        check("uhd_string_vector_make", uhd_string_vector_make(&mut v))?;
        let r = check("uhd_usrp_find", uhd_usrp_find(args.as_ptr(), &mut v));
        let mut out = Vec::new();
        if r.is_ok() {
            let mut n = 0usize;
            uhd_string_vector_size(v, &mut n);
            for i in 0..n {
                let mut buf = [0 as c_char; 1024];
                uhd_string_vector_at(v, i, buf.as_mut_ptr(), buf.len());
                out.push(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned());
            }
        }
        uhd_string_vector_free(&mut v);
        r.map(|()| out)
    }
}

struct RxStream {
    h: uhd_rx_streamer_handle,
    md: uhd_rx_metadata_handle,
    channels: usize,
}

struct TxStream {
    h: uhd_tx_streamer_handle,
    async_md: uhd_async_metadata_handle,
    channels: usize,
}

/// A USRP opened through `uhd_usrp_make`.
pub struct UhdDevice {
    usrp: uhd_usrp_handle,
    args: String,
    rate: u64,
    rx: Mutex<Option<RxStream>>,
    tx: Mutex<Option<TxStream>>,
}

// ponytail: the raw handles are shared across the Provider's threads. UHD's C API is
// safe to call concurrently on one usrp handle; each streamer is used by one thread
// (recv by the RX thread, send by the TX thread, recv_async_msg by the async thread),
// and `close_streams` runs only after those threads are joined.
unsafe impl Send for UhdDevice {}
unsafe impl Sync for UhdDevice {}

impl UhdDevice {
    /// Opens the device named by UHD device `args` (for example `addr=192.168.40.2`).
    pub fn open(args: &str) -> Result<UhdDevice, String> {
        let mut usrp: uhd_usrp_handle = std::ptr::null_mut();
        let c_args = cstring(args);
        unsafe { check("uhd_usrp_make", uhd_usrp_make(&mut usrp, c_args.as_ptr()))? };
        let mut mcr = 0.0;
        unsafe { check("get_master_clock_rate", uhd_usrp_get_master_clock_rate(usrp, 0, &mut mcr))? };
        if mcr.fract() != 0.0 || mcr <= 0.0 {
            return Err(format!("master clock rate {mcr} is not a positive integer; the root needs integer ticks"));
        }
        Ok(UhdDevice { usrp, args: args.to_owned(), rate: mcr as u64, rx: Mutex::new(None), tx: Mutex::new(None) })
    }

    fn to_spec(&self, tick: i64) -> (i64, f64) {
        let r = self.rate as i64;
        (tick.div_euclid(r), tick.rem_euclid(r) as f64 / r as f64)
    }

    fn from_spec(&self, full: i64, frac: f64) -> i64 {
        full * self.rate as i64 + (frac * self.rate as f64).round() as i64
    }
}

impl Drop for UhdDevice {
    fn drop(&mut self) {
        self.close_streams();
        unsafe { uhd_usrp_free(&mut self.usrp) };
    }
}

impl Device for UhdDevice {
    fn describe(&self) -> Value {
        let mut buf = vec![0 as c_char; 16384];
        let pp = unsafe {
            uhd_usrp_get_pp_string(self.usrp, buf.as_mut_ptr(), buf.len());
            CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
        };
        json!({
            "args": self.args,
            "master_clock_rate": self.rate,
            "rx_channels": self.channels(Dir::Rx),
            "tx_channels": self.channels(Dir::Tx),
            "pp_string": pp,
        })
    }

    fn tick_rate(&self) -> u64 {
        self.rate
    }

    fn time_now(&self) -> Result<i64, String> {
        let (mut full, mut frac) = (0i64, 0f64);
        unsafe { check("get_time_now", uhd_usrp_get_time_now(self.usrp, 0, &mut full, &mut frac))? };
        Ok(self.from_spec(full, frac))
    }

    fn set_sources(&self, clock: Option<&str>, time: Option<&str>) -> Result<(), String> {
        // v3 order: clock source, then time source (v3/cpp/uhd_usrp/multiusrp.cpp:332-347).
        if let Some(c) = clock {
            let c = cstring(c);
            unsafe { check("set_clock_source", uhd_usrp_set_clock_source(self.usrp, c.as_ptr(), 0))? };
        }
        if let Some(t) = time {
            let t = cstring(t);
            unsafe { check("set_time_source", uhd_usrp_set_time_source(self.usrp, t.as_ptr(), 0))? };
        }
        Ok(())
    }

    fn set_time_zero(&self, at_unknown_pps: bool) -> Result<(), String> {
        unsafe {
            if at_unknown_pps {
                check("set_time_unknown_pps", uhd_usrp_set_time_unknown_pps(self.usrp, 0, 0.0))
            } else {
                check("set_time_now", uhd_usrp_set_time_now(self.usrp, 0, 0.0, 0))
            }
        }
    }

    fn channels(&self, dir: Dir) -> usize {
        let mut n = 0usize;
        unsafe {
            match dir {
                Dir::Rx => uhd_usrp_get_rx_num_channels(self.usrp, &mut n),
                Dir::Tx => uhd_usrp_get_tx_num_channels(self.usrp, &mut n),
            };
        }
        n
    }

    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, String> {
        let empty = cstring("");
        unsafe {
            if let Some(t) = at {
                let (full, frac) = self.to_spec(t);
                check("set_command_time", uhd_usrp_set_command_time(self.usrp, full, frac, 0))?;
            }
            let result = (|| -> Result<(), String> {
                if let Some(rate) = s.rate {
                    match dir {
                        Dir::Rx => check("set_rx_rate", uhd_usrp_set_rx_rate(self.usrp, rate, chan))?,
                        Dir::Tx => check("set_tx_rate", uhd_usrp_set_tx_rate(self.usrp, rate, chan))?,
                    }
                }
                if let Some(freq) = s.freq {
                    let mut req = uhd_tune_request_t {
                        target_freq: freq,
                        rf_freq_policy: UHD_TUNE_REQUEST_POLICY_AUTO,
                        rf_freq: 0.0,
                        dsp_freq_policy: UHD_TUNE_REQUEST_POLICY_AUTO,
                        dsp_freq: 0.0,
                        args: empty.as_ptr() as *mut c_char,
                    };
                    let mut res = uhd_tune_result_t::default();
                    match dir {
                        Dir::Rx => check("set_rx_freq", uhd_usrp_set_rx_freq(self.usrp, &mut req, chan, &mut res))?,
                        Dir::Tx => check("set_tx_freq", uhd_usrp_set_tx_freq(self.usrp, &mut req, chan, &mut res))?,
                    }
                }
                if let Some(gain) = s.gain {
                    match dir {
                        Dir::Rx => check("set_rx_gain", uhd_usrp_set_rx_gain(self.usrp, gain, chan, empty.as_ptr()))?,
                        Dir::Tx => check("set_tx_gain", uhd_usrp_set_tx_gain(self.usrp, gain, chan, empty.as_ptr()))?,
                    }
                }
                if let Some(ant) = &s.antenna {
                    let ant = cstring(ant);
                    match dir {
                        Dir::Rx => check("set_rx_antenna", uhd_usrp_set_rx_antenna(self.usrp, ant.as_ptr(), chan))?,
                        Dir::Tx => check("set_tx_antenna", uhd_usrp_set_tx_antenna(self.usrp, ant.as_ptr(), chan))?,
                    }
                }
                Ok(())
            })();
            if at.is_some() {
                uhd_usrp_clear_command_time(self.usrp, 0);
            }
            result?;
            let (mut rate, mut freq, mut gain) = (0.0, 0.0, 0.0);
            match dir {
                Dir::Rx => {
                    uhd_usrp_get_rx_rate(self.usrp, chan, &mut rate);
                    uhd_usrp_get_rx_freq(self.usrp, chan, &mut freq);
                    uhd_usrp_get_rx_gain(self.usrp, chan, empty.as_ptr(), &mut gain);
                }
                Dir::Tx => {
                    uhd_usrp_get_tx_rate(self.usrp, chan, &mut rate);
                    uhd_usrp_get_tx_freq(self.usrp, chan, &mut freq);
                    uhd_usrp_get_tx_gain(self.usrp, chan, empty.as_ptr(), &mut gain);
                }
            }
            Ok(Applied { rate, freq, gain })
        }
    }

    fn rx_open(&self, n: usize) -> Result<(), String> {
        let mut chans: Vec<usize> = (0..n).collect();
        let (fc32, sc16, empty) = (cstring("fc32"), cstring("sc16"), cstring(""));
        let mut args = uhd_stream_args_t {
            cpu_format: fc32.as_ptr() as *mut c_char,
            otw_format: sc16.as_ptr() as *mut c_char,
            args: empty.as_ptr() as *mut c_char,
            channel_list: chans.as_mut_ptr(),
            n_channels: n as c_int,
        };
        let mut h: uhd_rx_streamer_handle = std::ptr::null_mut();
        let mut md: uhd_rx_metadata_handle = std::ptr::null_mut();
        unsafe {
            check("rx_streamer_make", uhd_rx_streamer_make(&mut h))?;
            check("get_rx_stream", uhd_usrp_get_rx_stream(self.usrp, &mut args, h))?;
            check("rx_metadata_make", uhd_rx_metadata_make(&mut md))?;
        }
        *self.rx.lock().unwrap() = Some(RxStream { h, md, channels: n });
        Ok(())
    }

    fn rx_start(&self, at: i64) -> Result<(), String> {
        let h = self.rx.lock().unwrap().as_ref().ok_or("rx not open")?.h;
        let (full, frac) = self.to_spec(at);
        let cmd = uhd_stream_cmd_t {
            stream_mode: UHD_STREAM_MODE_START_CONTINUOUS,
            num_samps: 0,
            stream_now: false,
            time_spec_full_secs: full,
            time_spec_frac_secs: frac,
        };
        unsafe { check("issue_stream_cmd(start)", uhd_rx_streamer_issue_stream_cmd(h, &cmd)) }
    }

    fn rx_recv(&self, n: usize, timeout_s: f64) -> RxRecv {
        let (h, mut md, channels) = match self.rx.lock().unwrap().as_ref() {
            Some(s) => (s.h, s.md, s.channels),
            None => return RxRecv::Error("rx not open".into()),
        };
        let mut bufs: Vec<Vec<Iq>> = vec![vec![[0.0; 2]; n]; channels];
        let mut ptrs: Vec<*mut c_void> = bufs.iter_mut().map(|b| b.as_mut_ptr() as *mut c_void).collect();
        let mut got = 0usize;
        let code = unsafe { uhd_rx_streamer_recv(h, ptrs.as_mut_ptr(), n, &mut md, timeout_s, false, &mut got) };
        if code != UHD_ERROR_NONE {
            return RxRecv::Error(format!("recv: UHD error {code}: {}", last_error()));
        }
        let mut err: c_int = 0;
        unsafe { uhd_rx_metadata_error_code(md, &mut err) };
        match err {
            UHD_RX_METADATA_ERROR_CODE_NONE if got > 0 => {
                let (mut has, mut full, mut frac) = (false, 0i64, 0f64);
                unsafe {
                    uhd_rx_metadata_has_time_spec(md, &mut has);
                    uhd_rx_metadata_time_spec(md, &mut full, &mut frac);
                }
                if !has {
                    return RxRecv::Error("recv returned samples without a time_spec".into());
                }
                for b in &mut bufs {
                    b.truncate(got);
                }
                RxRecv::Samples { first_tick: self.from_spec(full, frac), samples: bufs }
            }
            UHD_RX_METADATA_ERROR_CODE_NONE | UHD_RX_METADATA_ERROR_CODE_TIMEOUT => RxRecv::Timeout,
            UHD_RX_METADATA_ERROR_CODE_OVERFLOW => {
                let mut oos = false;
                unsafe { uhd_rx_metadata_out_of_sequence(md, &mut oos) };
                RxRecv::Overflow { out_of_sequence: oos }
            }
            other => {
                let mut buf = [0 as c_char; 256];
                unsafe { uhd_rx_metadata_strerror(md, buf.as_mut_ptr(), buf.len()) };
                let s = unsafe { CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned() };
                RxRecv::Error(format!("rx error code {other:#x}: {s}"))
            }
        }
    }

    fn rx_stop(&self) -> Result<(), String> {
        let h = self.rx.lock().unwrap().as_ref().ok_or("rx not open")?.h;
        let cmd = uhd_stream_cmd_t {
            stream_mode: UHD_STREAM_MODE_STOP_CONTINUOUS,
            num_samps: 0,
            stream_now: true,
            time_spec_full_secs: 0,
            time_spec_frac_secs: 0.0,
        };
        unsafe { check("issue_stream_cmd(stop)", uhd_rx_streamer_issue_stream_cmd(h, &cmd)) }
    }

    fn tx_open(&self, n: usize) -> Result<(), String> {
        let mut chans: Vec<usize> = (0..n).collect();
        let (fc32, sc16, empty) = (cstring("fc32"), cstring("sc16"), cstring(""));
        let mut args = uhd_stream_args_t {
            cpu_format: fc32.as_ptr() as *mut c_char,
            otw_format: sc16.as_ptr() as *mut c_char,
            args: empty.as_ptr() as *mut c_char,
            channel_list: chans.as_mut_ptr(),
            n_channels: n as c_int,
        };
        let mut h: uhd_tx_streamer_handle = std::ptr::null_mut();
        let mut async_md: uhd_async_metadata_handle = std::ptr::null_mut();
        unsafe {
            check("tx_streamer_make", uhd_tx_streamer_make(&mut h))?;
            check("get_tx_stream", uhd_usrp_get_tx_stream(self.usrp, &mut args, h))?;
            check("async_metadata_make", uhd_async_metadata_make(&mut async_md))?;
        }
        *self.tx.lock().unwrap() = Some(TxStream { h, async_md, channels: n });
        Ok(())
    }

    fn tx_send(&self, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout_s: f64) -> Result<usize, String> {
        let (h, channels) = match self.tx.lock().unwrap().as_ref() {
            Some(s) => (s.h, s.channels),
            None => return Err("tx not open".into()),
        };
        if samples.len() != channels {
            return Err(format!("{} buffers for {channels} channels", samples.len()));
        }
        let n = samples.first().map_or(0, |s| s.len());
        let (full, frac) = at.map_or((0, 0.0), |t| self.to_spec(t));
        let mut md: uhd_tx_metadata_handle = std::ptr::null_mut();
        let mut ptrs: Vec<*const c_void> = samples.iter().map(|s| s.as_ptr() as *const c_void).collect();
        let mut sent = 0usize;
        unsafe {
            check("tx_metadata_make", uhd_tx_metadata_make(&mut md, at.is_some(), full, frac, sob, eob))?;
            let r = check("send", uhd_tx_streamer_send(h, ptrs.as_mut_ptr(), n, &mut md, timeout_s, &mut sent));
            uhd_tx_metadata_free(&mut md);
            r?;
        }
        Ok(sent)
    }

    fn tx_async(&self, timeout_s: f64) -> Option<TxEvent> {
        let (h, mut md) = {
            let g = self.tx.lock().unwrap();
            let s = g.as_ref()?;
            (s.h, s.async_md)
        };
        let mut valid = false;
        unsafe {
            if uhd_tx_streamer_recv_async_msg(h, &mut md, timeout_s, &mut valid) != UHD_ERROR_NONE || !valid {
                return None;
            }
            let (mut code, mut chan, mut has, mut full, mut frac) = (0 as c_int, 0usize, false, 0i64, 0f64);
            uhd_async_metadata_event_code(md, &mut code);
            uhd_async_metadata_channel(md, &mut chan);
            uhd_async_metadata_has_time_spec(md, &mut has);
            uhd_async_metadata_time_spec(md, &mut full, &mut frac);
            let code = match code {
                UHD_ASYNC_METADATA_EVENT_CODE_BURST_ACK => "burst_ack",
                UHD_ASYNC_METADATA_EVENT_CODE_UNDERFLOW => "underflow",
                UHD_ASYNC_METADATA_EVENT_CODE_SEQ_ERROR => "seq_error",
                UHD_ASYNC_METADATA_EVENT_CODE_TIME_ERROR => "time_error",
                UHD_ASYNC_METADATA_EVENT_CODE_UNDERFLOW_IN_PACKET => "underflow_in_packet",
                UHD_ASYNC_METADATA_EVENT_CODE_SEQ_ERROR_IN_BURST => "seq_error_in_burst",
                _ => "other",
            };
            Some(TxEvent { code, tick: has.then(|| self.from_spec(full, frac)), channel: chan })
        }
    }

    fn close_streams(&self) {
        unsafe {
            if let Some(mut s) = self.rx.lock().unwrap().take() {
                uhd_rx_metadata_free(&mut s.md);
                uhd_rx_streamer_free(&mut s.h);
            }
            if let Some(mut s) = self.tx.lock().unwrap().take() {
                uhd_async_metadata_free(&mut s.async_md);
                uhd_tx_streamer_free(&mut s.h);
            }
        }
    }
}

// ---------------------------------------------------------------- fake

/// A wall-clock device for running the Kernel path without hardware. Receive is a
/// loopback of whatever was transmitted, and a ramp `(tick / ratio mod 65536) / 65536`
/// where nothing was; a receive loop that falls more than `OVERRUN` behind gets an
/// overflow, like a USRP whose host stopped reading.
pub struct FakeDevice {
    epoch: Instant,
    state: Mutex<Fake>,
}

const FAKE_RATE: u64 = 200_000_000;
const OVERRUN: Duration = Duration::from_millis(100);

#[derive(Default)]
struct Fake {
    offset: i64,
    rx_rate: f64,
    tx_rate: f64,
    rx_channels: usize,
    tx_channels: usize,
    rx_next: Option<i64>,
    rx_late: bool,
    tx_cursor: Option<i64>,
    tx: BTreeMap<i64, Vec<Iq>>,
    events: VecDeque<TxEvent>,
}

impl Default for FakeDevice {
    fn default() -> Self {
        FakeDevice::new()
    }
}

impl FakeDevice {
    /// A fake with 2 RX and 2 TX channels at a 200 MHz master clock.
    pub fn new() -> FakeDevice {
        FakeDevice { epoch: Instant::now(), state: Mutex::new(Fake { rx_rate: 1e6, tx_rate: 1e6, ..Fake::default() }) }
    }

    fn now_ticks(&self, offset: i64) -> i64 {
        offset + (self.epoch.elapsed().as_nanos() as i128 * FAKE_RATE as i128 / 1_000_000_000) as i64
    }

    fn sleep_until(&self, tick: i64) {
        let now = self.now_ticks(self.state.lock().unwrap().offset);
        if tick > now {
            std::thread::sleep(Duration::from_nanos(((tick - now) as u128 * 1_000_000_000 / FAKE_RATE as u128) as u64));
        }
    }
}

impl Device for FakeDevice {
    fn describe(&self) -> Value {
        json!({ "args": "fake", "master_clock_rate": FAKE_RATE, "fake": true })
    }

    fn tick_rate(&self) -> u64 {
        FAKE_RATE
    }

    fn time_now(&self) -> Result<i64, String> {
        Ok(self.now_ticks(self.state.lock().unwrap().offset))
    }

    fn set_sources(&self, _clock: Option<&str>, _time: Option<&str>) -> Result<(), String> {
        Ok(())
    }

    fn set_time_zero(&self, _at_unknown_pps: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        s.offset = -self.now_ticks(0);
        Ok(())
    }

    fn channels(&self, _dir: Dir) -> usize {
        2
    }

    fn apply(&self, dir: Dir, _chan: usize, st: &Settings, _at: Option<i64>) -> Result<Applied, String> {
        let mut s = self.state.lock().unwrap();
        if let Some(rate) = st.rate {
            let n = (FAKE_RATE as f64 / rate).round().max(1.0);
            match dir {
                Dir::Rx => s.rx_rate = FAKE_RATE as f64 / n,
                Dir::Tx => s.tx_rate = FAKE_RATE as f64 / n,
            }
        }
        let rate = if dir == Dir::Rx { s.rx_rate } else { s.tx_rate };
        Ok(Applied {
            rate,
            freq: st.freq.unwrap_or(0.0),
            gain: st.gain.map_or(0.0, |g| (g * 2.0).round().clamp(0.0, 63.0) / 2.0),
        })
    }

    fn rx_open(&self, n: usize) -> Result<(), String> {
        self.state.lock().unwrap().rx_channels = n;
        Ok(())
    }

    fn rx_start(&self, at: i64) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        s.rx_late = at < self.now_ticks(s.offset);
        s.rx_next = Some(at);
        Ok(())
    }

    fn rx_recv(&self, n: usize, timeout_s: f64) -> RxRecv {
        let (next, ratio, offset) = {
            let s = self.state.lock().unwrap();
            if s.rx_late {
                drop(s);
                self.state.lock().unwrap().rx_late = false;
                return RxRecv::Error("rx error code 0x2: late command".into());
            }
            match s.rx_next {
                Some(next) => (next, (FAKE_RATE as f64 / s.rx_rate).round() as i64, s.offset),
                None => {
                    drop(s);
                    std::thread::sleep(Duration::from_secs_f64(timeout_s));
                    return RxRecv::Timeout;
                }
            }
        };
        let end = next + n as i64 * ratio;
        let now = self.now_ticks(offset);
        if now - end > (OVERRUN.as_nanos() as i64) * FAKE_RATE as i64 / 1_000_000_000 {
            // Skip to the sample grid just after now, as a restarted stream would.
            let mut s = self.state.lock().unwrap();
            s.rx_next = Some(next + (now - next).div_euclid(ratio) * ratio + ratio);
            return RxRecv::Overflow { out_of_sequence: false };
        }
        if end > now + (timeout_s * FAKE_RATE as f64) as i64 {
            std::thread::sleep(Duration::from_secs_f64(timeout_s));
            return RxRecv::Timeout;
        }
        self.sleep_until(end);
        let mut s = self.state.lock().unwrap();
        let tx_ratio = (FAKE_RATE as f64 / s.tx_rate).round() as i64;
        let mut samples = vec![Vec::with_capacity(n); s.rx_channels];
        for i in 0..n as i64 {
            let t = next + i * ratio;
            let looped = s.tx.range(..=t).next_back().and_then(|(start, v)| {
                let k = t - start;
                (k % tx_ratio == 0).then(|| v.get((k / tx_ratio) as usize).copied()).flatten()
            });
            let v = looped.unwrap_or([((t / ratio) % 65_536) as f32 / 65_536.0, 0.0]);
            for ch in samples.iter_mut() {
                ch.push(v);
            }
        }
        s.rx_next = Some(end);
        let horizon = end - 2 * FAKE_RATE as i64;
        s.tx.retain(|start, v| start + v.len() as i64 * tx_ratio > horizon);
        RxRecv::Samples { first_tick: next, samples }
    }

    fn rx_stop(&self) -> Result<(), String> {
        self.state.lock().unwrap().rx_next = None;
        Ok(())
    }

    fn tx_open(&self, n: usize) -> Result<(), String> {
        self.state.lock().unwrap().tx_channels = n;
        Ok(())
    }

    fn tx_send(&self, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, _timeout_s: f64) -> Result<usize, String> {
        let n = samples.first().map_or(0, |s| s.len());
        let (start, ratio) = {
            let mut s = self.state.lock().unwrap();
            let now = self.now_ticks(s.offset);
            let ratio = (FAKE_RATE as f64 / s.tx_rate).round() as i64;
            if sob {
                if let Some(t) = at.filter(|t| *t < now) {
                    s.events.push_back(TxEvent { code: "time_error", tick: Some(t), channel: 0 });
                    s.tx_cursor = None;
                    return Ok(n);
                }
                s.tx_cursor = Some(at.unwrap_or(now));
            }
            let Some(start) = s.tx_cursor else { return Ok(n) };
            if n > 0 {
                s.tx.insert(start, samples[0].to_vec());
            }
            s.tx_cursor = Some(start + n as i64 * ratio);
            if eob {
                s.events.push_back(TxEvent { code: "burst_ack", tick: Some(start + n as i64 * ratio), channel: 0 });
                s.tx_cursor = None;
            }
            (start, ratio)
        };
        // Flow control: accept at most ~50 ms ahead of the air, as a USRP's buffer does.
        self.sleep_until(start + n as i64 * ratio - FAKE_RATE as i64 / 20);
        Ok(n)
    }

    fn tx_async(&self, timeout_s: f64) -> Option<TxEvent> {
        if let Some(e) = self.state.lock().unwrap().events.pop_front() {
            return Some(e);
        }
        std::thread::sleep(Duration::from_secs_f64(timeout_s));
        None
    }

    fn close_streams(&self) {}
}
