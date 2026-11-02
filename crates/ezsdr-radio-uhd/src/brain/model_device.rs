//! `ModelDevice` and `CheckingDevice` (stage E2a; test plan §6): the device model of `model.rs` behind a mutex, with the
//! native control lock `L`, behind the `Device` trait; and the decorator that panics on a device call made while a cell
//! guard is held (the I10 oracle). Virtual time advances only by [`ModelDevice::advance`]; where the model says "not
//! yet", the caller waits through the scheduler. Receive and DDC parts only: every other method returns a fixed value or
//! an error (listed in the plan).

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ezsdr_kernel::module_api::Fidelity;

use super::cell::Outcome;
use super::force::{Actor, Forced, Point, Sched, actor};
use super::model::{Apply, Device as Model};
use super::rules::DevCall;
use super::shared::held_cells;
use crate::device::{Applied, Device, DeviceError, Dir, Iq, RxRecv, Settings};

/// The marker a partial result carries in `DeviceError::message` (test convention: the trait has no partial outcome).
pub(crate) const PARTIAL: &str = "model: partial";
pub(crate) const REFUSED: &str = "model: refused";

/// `L`'s history: who acquired and released it, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LEvent {
    Acquire(Actor),
    Release(Actor),
}

pub(crate) struct MState {
    pub(crate) dev: Model,
    pub(crate) l: Option<Actor>,
    pub(crate) calls: Vec<DevCall>,
    pub(crate) llog: Vec<LEvent>,
}

/// A diagnostic hook run inside the device's effect when a timed write is accepted: `(value id, effective instant)`.
pub(crate) type AcceptHook = Arc<dyn Fn(u64, i64) + Send + Sync>;

pub(crate) struct ModelDevice {
    pub(crate) accept_hook: Mutex<Option<AcceptHook>>,
    sched: Arc<Sched>,
    st: Arc<Mutex<MState>>,
    /// The samples one recv returns.
    p: usize,
}

fn lost() -> DeviceError {
    DeviceError { lost: true, message: "model: the device is lost".to_string() }
}

impl ModelDevice {
    pub(crate) fn new(sched: Arc<Sched>, dev: Model, p: usize) -> Arc<ModelDevice> {
        Arc::new(ModelDevice { accept_hook: Mutex::new(None), sched, st: Arc::new(Mutex::new(MState { dev, l: None, calls: Vec::new(), llog: Vec::new() })), p })
    }

    pub(crate) fn sched(&self) -> &Arc<Sched> {
        &self.sched
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, MState> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Virtual time, read without `L` and without an event (the executors' `now` source).
    pub(crate) fn now(&self) -> i64 {
        self.state().dev.t
    }

    /// Advances virtual time by `ticks` samples.
    pub(crate) fn advance(&self, ticks: usize) {
        {
            let mut st = self.state();
            for _ in 0..ticks {
                st.dev.tick();
            }
        }
        self.sched.notify();
    }

    pub(crate) fn set_lost(&self) {
        self.state().dev.lost = true;
        self.sched.notify();
    }

    pub(crate) fn snapshot(&self) -> (Model, Vec<DevCall>, Vec<LEvent>) {
        let st = self.state();
        (st.dev.clone(), st.calls.clone(), st.llog.clone())
    }

    /// The native lock around one call: wait for it, take it at the permit, optionally wait (holding it) for `ready`, run
    /// the effect at the call point, release at the unlock permit.
    fn locked<T>(&self, ready: Option<Arc<dyn Fn() -> bool + Send + Sync>>, effect: impl FnOnce(&mut MState, Forced) -> T) -> T {
        let a = actor();
        let st = self.st.clone();
        let free = Arc::new({
            let st = st.clone();
            move || st.lock().unwrap_or_else(|e| e.into_inner()).l.is_none()
        });
        self.sched.acquire(a, Point::Lock, free, {
            let st = st.clone();
            move || {
                let mut g = st.lock().unwrap_or_else(|e| e.into_inner());
                g.l = Some(a);
                g.llog.push(LEvent::Acquire(a));
            }
        });
        if let Some(ready) = ready {
            self.sched.wait_until(a, ready);
        }
        let forced = self.sched.hook(a, Point::Call);
        let result = effect(&mut self.state(), forced);
        self.sched.notify();
        self.sched.hook(a, Point::Unlock);
        {
            let mut g = self.state();
            assert_eq!(g.l, Some(a), "{a:?} releases a lock it does not hold");
            g.l = None;
            g.llog.push(LEvent::Release(a));
        }
        self.sched.notify();
        result
    }

    fn applied() -> Applied {
        Applied { rate: 0.0, freq: 0.0, gain: 0.0, antenna: None }
    }
}

impl Device for ModelDevice {
    fn describe(&self) -> serde_json::Value {
        serde_json::json!({ "model": true })
    }
    fn fidelity(&self) -> Fidelity {
        Fidelity::NONE
    }
    fn master_clock_rate(&self) -> u64 {
        1000
    }
    fn channels(&self, _dir: Dir) -> usize {
        2
    }
    fn front_end(&self, _dir: Dir, chan: usize) -> Result<String, DeviceError> {
        Ok(format!("MODEL {chan}"))
    }
    /// The identity read, which takes no lock and is no event: it errs when the device is lost (the executors' lost probe).
    fn serial(&self) -> Result<String, DeviceError> {
        if self.state().dev.lost { Err(lost()) } else { Ok("MODEL".to_string()) }
    }
    fn set_subdev_spec(&self, _dir: Dir, _markup: &str) -> Result<(), DeviceError> {
        Ok(())
    }
    fn rates(&self, _dir: Dir, _chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> {
        Ok(vec![(1.0, 1.0, 0.0)])
    }
    fn set_sources(&self, _clock: &str, _time: &str) -> Result<(), DeviceError> {
        Ok(())
    }
    fn set_time_zero(&self, _at_next_pps: bool) -> Result<(), DeviceError> {
        Ok(())
    }
    /// A time read under `L` (not logged as a call); a lost device errs at the call point.
    fn time_now(&self) -> Result<i64, DeviceError> {
        self.locked(None, |g, forced| {
            if g.dev.lost {
                Err(lost())
            } else if forced == Forced::Fault {
                Err(DeviceError::failed("model: fault"))
            } else {
                Ok(g.dev.t)
            }
        })
    }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> {
        Ok(None)
    }
    fn apply(&self, _dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        let st = self.st.clone();
        let ready: Option<Arc<dyn Fn() -> bool + Send + Sync>> = at.map(|_| {
            Arc::new(move || {
                let g = st.lock().unwrap_or_else(|e| e.into_inner());
                g.dev.lost || g.dev.room(chan) != Some(Apply::Blocked)
            }) as Arc<dyn Fn() -> bool + Send + Sync>
        });
        let freq = s.freq;
        let hook = self.accept_hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
        self.locked(ready, |g, forced| {
            g.calls.push(DevCall::Apply { channel: chan, freq: freq.map(|f| f as i64), at });
            if g.dev.lost {
                assert!(matches!(forced, Forced::None | Forced::Outcome(Outcome::Lost)), "forced {forced:?} on a lost device");
                return Err(lost());
            }
            let Some(at) = at else {
                assert!(matches!(forced, Forced::None | Forced::Outcome(Outcome::Ok)), "forced {forced:?} on an untimed apply");
                return Ok(Self::applied());
            };
            let id = freq.expect("a timed apply carries the value id") as u64;
            match g.dev.apply_timed(chan, id, at) {
                Apply::Accepted { effective } => {
                    if let Some(hook) = &hook {
                        hook(id, effective);
                    }
                    match forced {
                        Forced::Outcome(Outcome::Partial) => Err(DeviceError::failed(PARTIAL)),
                        Forced::None | Forced::Outcome(Outcome::Ok) => Ok(Self::applied()),
                        other => panic!("forced {other:?} on an accepted write: the trace does not describe this run"),
                    }
                }
                Apply::Refused => {
                    assert!(matches!(forced, Forced::None | Forced::Outcome(Outcome::Refused)), "forced {forced:?} on a refused write");
                    Err(DeviceError::failed(REFUSED))
                }
                Apply::Blocked => panic!("a blocked write was not waited for"),
            }
        })
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        let channels = channels.to_vec();
        self.locked(None, |g, forced| {
            if g.dev.lost {
                return Err(lost());
            }
            if forced == Forced::Fault {
                return Err(DeviceError::failed("model: fault"));
            }
            g.calls.push(DevCall::Open { stream, channels: channels.clone() });
            g.dev.open(stream, channels);
            Ok(())
        })
    }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> {
        self.locked(None, |g, forced| {
            if g.dev.lost {
                return Err(lost());
            }
            if forced == Forced::Fault {
                return Err(DeviceError::failed("model: fault"));
            }
            g.calls.push(DevCall::Start { stream, at });
            g.dev.start(stream, at);
            Ok(())
        })
    }
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> {
        self.locked(None, |g, forced| {
            if g.dev.lost {
                return Err(lost());
            }
            if forced == Forced::Fault {
                return Err(DeviceError::failed("model: fault"));
            }
            g.calls.push(DevCall::StartNow { stream });
            g.dev.start_now(stream);
            Ok(())
        })
    }
    fn rx_packet_samples(&self, stream: usize) -> usize {
        if self.state().dev.streamers.contains_key(&stream) { self.p } else { 0 }
    }
    fn rx_stop(&self, stream: usize, _at: Option<i64>) -> Result<(), DeviceError> {
        self.locked(None, |g, forced| {
            if g.dev.lost {
                return Err(lost());
            }
            if forced == Forced::Fault {
                return Err(DeviceError::failed("model: fault"));
            }
            g.calls.push(DevCall::Stop { stream });
            g.dev.stop(stream);
            Ok(())
        })
    }
    /// A recv: enter (the stopped tail's promotion), wait until a packet, the tail, a missed start, the timeout or a loss,
    /// park at `REnd`, then take. The timeout is `timeout` in milliseconds read as ticks.
    fn rx_recv(&self, stream: usize, n: usize, timeout: Duration, _one_packet: bool) -> RxRecv {
        let a = actor();
        let deadline = {
            let mut g = self.state();
            g.calls.push(DevCall::Recv { stream });
            g.dev.enter_recv(stream);
            g.dev.t + timeout.as_millis() as i64
        };
        let st = self.st.clone();
        self.sched.wait_until(
            a,
            Arc::new(move || {
                let g = st.lock().unwrap_or_else(|e| e.into_inner());
                g.dev.lost || g.dev.packet_ready(stream, n) || g.dev.t >= deadline
            }),
        );
        self.sched.hook(a, Point::REnd);
        let mut g = self.state();
        if g.dev.lost {
            return RxRecv::Failed(lost());
        }
        if g.dev.take_late(stream) {
            return RxRecv::LateCommand;
        }
        let ticks = g.dev.recv(stream, n, deadline);
        let Some(&first_tick) = ticks.first() else { return RxRecv::Timeout };
        assert!(ticks.windows(2).all(|w| w[1] == w[0] + 1), "a recv returns contiguous ticks: {ticks:?}");
        let channels = g.dev.streamers.get(&stream).map_or(1, |s| s.channels.len());
        RxRecv::Samples { first_tick, samples: vec![vec![[0.0, 0.0] as Iq; ticks.len()]; channels] }
    }
    fn tx_open(&self, _stream: usize, _channels: &[usize]) -> Result<(), DeviceError> {
        Err(DeviceError::failed("model: transmit is not modelled"))
    }
    fn tx_send(&self, _stream: usize, _samples: &[&[Iq]], _at: Option<i64>, _sob: bool, _eob: bool, _timeout: Duration) -> Result<usize, DeviceError> {
        Err(DeviceError::failed("model: transmit is not modelled"))
    }
    fn tx_async(&self, _stream: usize, _timeout: Duration) -> Result<Option<crate::device::TxReport>, DeviceError> {
        Err(DeviceError::failed("model: transmit is not modelled"))
    }
    /// A close under `L`. A close on a lost device, or a forced fault, leaves the streamer open (the trait's close is
    /// infallible; the executor reads the outcome from `rx_packet_samples`).
    fn rx_close(&self, stream: usize) {
        self.locked(None, |g, forced| {
            if !g.dev.lost && forced != Forced::Fault {
                g.calls.push(DevCall::Close { stream });
                g.dev.close(stream);
            }
        });
    }
    fn tx_close(&self, _stream: usize) {}
    fn close_streams(&self) {
        panic!("the executors never close every stream");
    }
    fn mark_lost(&self) {}
    fn raise_thread_priority(&self) -> Result<(), DeviceError> {
        Ok(())
    }
}

/// The I10 oracle: a decorator that panics if a device method is called while this thread holds a cell guard, and logs
/// each call with the failure flag's value at the call (S5 reads the order).
pub(crate) struct CheckingDevice {
    pub(crate) inner: Arc<dyn Device>,
    pub(crate) failure: Arc<std::sync::atomic::AtomicBool>,
    /// The call history; none in a measurement run (it would grow with every block).
    pub(crate) log: Option<Mutex<Vec<(&'static str, bool)>>>,
}

impl CheckingDevice {
    pub(crate) fn new(inner: Arc<dyn Device>, failure: Arc<std::sync::atomic::AtomicBool>) -> CheckingDevice {
        CheckingDevice { inner, failure, log: Some(Mutex::new(Vec::new())) }
    }
    /// The same assertion with no history.
    pub(crate) fn unlogged(inner: Arc<dyn Device>, failure: Arc<std::sync::atomic::AtomicBool>) -> CheckingDevice {
        CheckingDevice { inner, failure, log: None }
    }
    fn check(&self, name: &'static str) {
        assert_eq!(held_cells(), 0, "I10: device call `{name}` made while a cell guard is held");
        if let Some(log) = &self.log {
            log.lock().unwrap_or_else(|e| e.into_inner()).push((name, self.failure.load(std::sync::atomic::Ordering::SeqCst)));
        }
    }
    pub(crate) fn calls(&self) -> Vec<(&'static str, bool)> {
        self.log.as_ref().map_or_else(Vec::new, |log| log.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

macro_rules! checked {
    ($self:ident, $name:literal, $call:expr) => {{
        $self.check($name);
        $call
    }};
}

impl Device for CheckingDevice {
    fn describe(&self) -> serde_json::Value {
        checked!(self, "describe", self.inner.describe())
    }
    fn fidelity(&self) -> Fidelity {
        checked!(self, "fidelity", self.inner.fidelity())
    }
    fn master_clock_rate(&self) -> u64 {
        checked!(self, "master_clock_rate", self.inner.master_clock_rate())
    }
    fn channels(&self, dir: Dir) -> usize {
        checked!(self, "channels", self.inner.channels(dir))
    }
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> {
        checked!(self, "front_end", self.inner.front_end(dir, chan))
    }
    fn serial(&self) -> Result<String, DeviceError> {
        checked!(self, "serial", self.inner.serial())
    }
    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> {
        checked!(self, "set_subdev_spec", self.inner.set_subdev_spec(dir, markup))
    }
    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> {
        checked!(self, "rates", self.inner.rates(dir, chan))
    }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> {
        checked!(self, "set_sources", self.inner.set_sources(clock, time))
    }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> {
        checked!(self, "set_time_zero", self.inner.set_time_zero(at_next_pps))
    }
    fn time_now(&self) -> Result<i64, DeviceError> {
        checked!(self, "time_now", self.inner.time_now())
    }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> {
        checked!(self, "ref_locked", self.inner.ref_locked())
    }
    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        checked!(self, "apply", self.inner.apply(dir, chan, s, at))
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        checked!(self, "rx_open", self.inner.rx_open(stream, channels))
    }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> {
        checked!(self, "rx_start", self.inner.rx_start(stream, at))
    }
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> {
        checked!(self, "rx_start_now", self.inner.rx_start_now(stream))
    }
    fn rx_packet_samples(&self, stream: usize) -> usize {
        checked!(self, "rx_packet_samples", self.inner.rx_packet_samples(stream))
    }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> {
        checked!(self, "rx_stop", self.inner.rx_stop(stream, at))
    }
    fn rx_recv(&self, stream: usize, n: usize, timeout: Duration, one_packet: bool) -> RxRecv {
        checked!(self, "rx_recv", self.inner.rx_recv(stream, n, timeout, one_packet))
    }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        checked!(self, "tx_open", self.inner.tx_open(stream, channels))
    }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Duration) -> Result<usize, DeviceError> {
        checked!(self, "tx_send", self.inner.tx_send(stream, samples, at, sob, eob, timeout))
    }
    fn tx_async(&self, stream: usize, timeout: Duration) -> Result<Option<crate::device::TxReport>, DeviceError> {
        checked!(self, "tx_async", self.inner.tx_async(stream, timeout))
    }
    fn rx_close(&self, stream: usize) {
        checked!(self, "rx_close", self.inner.rx_close(stream))
    }
    fn tx_close(&self, stream: usize) {
        checked!(self, "tx_close", self.inner.tx_close(stream))
    }
    fn close_streams(&self) {
        checked!(self, "close_streams", self.inner.close_streams())
    }
    fn mark_lost(&self) {
        checked!(self, "mark_lost", self.inner.mark_lost())
    }
    fn raise_thread_priority(&self) -> Result<(), DeviceError> {
        checked!(self, "raise_thread_priority", self.inner.raise_thread_priority())
    }
}
