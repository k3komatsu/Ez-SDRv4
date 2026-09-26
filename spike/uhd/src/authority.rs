//! A device-paced Time Authority (`Pacing::Device`, TM-16a1): the Run's primary root
//! is the USRP's own timekeeper, and `next_wakeup` blocks on the wall clock until the
//! device reaches the earliest scheduled instant.
//!
//! Reading the device on every `now()` would cost a control-port round trip per call,
//! and the coordinator calls `now()` often. So `now()` extrapolates from an anchor
//! (host instant, device tick) with the host monotonic clock, and every
//! `next_wakeup` re-anchors from the device: the error is the host/device frequency
//! offset over one wakeup interval (ppm × ms), never accumulated.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{Authority, AuthorityDescriptor, Pacing};
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, Converted, EpochRef, Rational, ScheduleHandle, TimeAuthority, TimeError,
    TimePoint,
};

use crate::device::Device;

type Callback = Box<dyn FnOnce(TimePoint) + Send>;

/// Longest single sleep inside `next_wakeup`, so a newly scheduled earlier instant
/// or a device re-anchor is seen promptly.
const MAX_NAP: Duration = Duration::from_millis(20);
/// Callbacks fired per `next_wakeup` call, as the Simulation Engine's SE-11 cap.
const CALLBACK_CAP: usize = 1000;

struct State {
    fired: i64,
    pending: BTreeMap<(i64, u64), Callback>,
    next_seq: u64,
}

struct Anchor {
    host: Instant,
    device: i64,
}

pub(crate) struct DeviceTime {
    device: Arc<dyn Device>,
    clocks: Arc<ClockRegistry>,
    root: ClockDomainId,
    rate: u64,
    created: Instant,
    anchor: Mutex<Anchor>,
    last: AtomicI64,
    state: Mutex<State>,
    woken: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl DeviceTime {
    fn extrapolate(&self) -> i64 {
        let a = lock(&self.anchor);
        let dt = a.host.elapsed().as_nanos() as i128;
        let t = a.device + (dt * self.rate as i128 / 1_000_000_000) as i64;
        drop(a);
        // Monotonic even across a re-anchor that lands slightly behind.
        let prev = self.last.fetch_max(t, Ordering::AcqRel);
        prev.max(t)
    }

    fn reanchor(&self) {
        let host = Instant::now();
        if let Ok(device) = self.device.time_now() {
            *lock(&self.anchor) = Anchor { host, device };
        }
    }

    /// Root tick of `point`, rounding a derived instant up when asked.
    fn root_tick(&self, point: TimePoint, round_up: bool) -> Result<i64, TimeError> {
        if point.domain == self.root {
            return Ok(point.ticks);
        }
        if point.domain == ClockDomainId::HOST_MONOTONIC {
            let a = lock(&self.anchor);
            let host_anchor = a.host.duration_since(self.created).as_nanos() as i128;
            let dt = point.ticks as i128 - host_anchor;
            return Ok(a.device + (dt * self.rate as i128 / 1_000_000_000) as i64);
        }
        if !self.governs(point.domain) {
            return Err(TimeError::NotGoverned { id: point.domain });
        }
        match self.clocks.convert(point, self.root)? {
            Converted::Exact { point } => Ok(point.ticks),
            Converted::Inexact { floor, .. } if round_up => floor.ticks.checked_add(1).ok_or(TimeError::Overflow),
            Converted::Inexact { floor, .. } => Err(TimeError::Inexact { floor }),
        }
    }
}

impl TimeAuthority for DeviceTime {
    fn primary_root(&self) -> ClockDomainId {
        self.root
    }

    fn pacing(&self) -> Pacing {
        Pacing::Device
    }

    fn governs(&self, domain: ClockDomainId) -> bool {
        domain == self.root
            || domain == ClockDomainId::HOST_MONOTONIC
            || self.clocks.get(domain).is_ok_and(|c| c.root_id() == self.root)
    }

    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError> {
        if domain == ClockDomainId::HOST_MONOTONIC {
            return Ok(TimePoint::new(domain, self.created.elapsed().as_nanos() as i64));
        }
        if !self.governs(domain) {
            return Err(TimeError::NotGoverned { id: domain });
        }
        let ticks = self.extrapolate();
        if domain == self.root {
            return Ok(TimePoint::new(domain, ticks));
        }
        Ok(self.clocks.convert(TimePoint::new(self.root, ticks), domain)?.floor())
    }

    fn wait_until(&self, point: TimePoint) -> Result<(), TimeError> {
        let target = self.root_tick(point, true)?;
        loop {
            let now = self.extrapolate();
            if now >= target {
                return Ok(());
            }
            let ns = ((target - now) as u128 * 1_000_000_000 / self.rate as u128) as u64;
            std::thread::sleep(Duration::from_nanos(ns).min(MAX_NAP));
            self.reanchor();
        }
    }

    fn schedule(&self, point: TimePoint, callback: Callback) -> Result<ScheduleHandle, TimeError> {
        let tick = self.root_tick(point, false)?;
        let mut s = lock(&self.state);
        if tick < s.fired {
            return Err(TimeError::InPast { now: TimePoint::new(self.root, s.fired), requested: point });
        }
        let seq = s.next_seq;
        s.next_seq += 1;
        s.pending.insert((tick, seq), callback);
        drop(s);
        self.woken.notify_all();
        Ok(ScheduleHandle { root: self.root, ticks: tick, seq })
    }

    fn cancel(&self, h: ScheduleHandle) -> bool {
        lock(&self.state).pending.remove(&(h.ticks, h.seq)).is_some()
    }
}

/// The device-paced Authority of Module `ezsdr.radio.uhd` (its second role).
pub struct DeviceAuthority {
    time: Arc<DeviceTime>,
    descriptor: AuthorityDescriptor,
}

impl DeviceAuthority {
    /// Selects the reference sources, sets device time to zero (at the next PPS when
    /// `time_source` is external), and registers the device's master clock as the
    /// Run's primary root.
    pub fn new(
        device: Arc<dyn Device>,
        clocks: Arc<ClockRegistry>,
        clock_source: Option<&str>,
        time_source: Option<&str>,
    ) -> Result<DeviceAuthority, String> {
        device.set_sources(clock_source, time_source)?;
        let pps = time_source.is_some_and(|t| t != "internal");
        device.set_time_zero(pps)?;
        let rate = device.tick_rate();
        let root = clocks.allocate_id();
        let set_by = if pps { "uhd.set_time_unknown_pps" } else { "uhd.set_time_now" };
        clocks
            .register(ClockDomain::root(
                root,
                Rational::new(rate, 1).map_err(|e| e.to_string())?,
                EpochRef::Arbitrary { set_by: set_by.to_owned() },
            ))
            .map_err(|e| e.to_string())?;
        let created = Instant::now();
        let time = Arc::new(DeviceTime {
            anchor: Mutex::new(Anchor { host: Instant::now(), device: device.time_now()? }),
            device,
            clocks,
            root,
            rate,
            created,
            last: AtomicI64::new(i64::MIN),
            state: Mutex::new(State { fired: i64::MIN, pending: BTreeMap::new(), next_seq: 0 }),
            woken: Condvar::new(),
        });
        let descriptor = AuthorityDescriptor {
            module: crate::module_ref(),
            governs: vec![root, ClockDomainId::HOST_MONOTONIC],
            pacing: Pacing::Device,
        };
        Ok(DeviceAuthority { time, descriptor })
    }

    /// The device root domain.
    pub fn root(&self) -> ClockDomainId {
        self.time.root
    }
}

impl Authority for DeviceAuthority {
    fn descriptor(&self) -> &AuthorityDescriptor {
        &self.descriptor
    }

    fn time(&self) -> Arc<dyn TimeAuthority> {
        self.time.clone()
    }

    fn next_wakeup(&self) -> Option<TimePoint> {
        let t = &self.time;
        let tick = loop {
            t.reanchor();
            let s = lock(&t.state);
            let (&(tick, _), _) = s.pending.iter().next()?;
            let now = t.extrapolate();
            if tick <= now {
                break tick;
            }
            let ns = ((tick - now) as u128 * 1_000_000_000 / t.rate as u128) as u64;
            // Woken early by `schedule` when something earlier arrives.
            let _ = t.woken.wait_timeout(s, Duration::from_nanos(ns).min(MAX_NAP));
        };
        lock(&t.state).fired = tick;
        for _ in 0..CALLBACK_CAP {
            let cb = {
                let mut s = lock(&t.state);
                match s.pending.keys().next().copied() {
                    Some(k) if k.0 == tick => s.pending.remove(&k),
                    _ => None,
                }
            };
            let Some(cb) = cb else { break };
            cb(TimePoint::new(t.root, tick));
        }
        Some(TimePoint::new(t.root, tick))
    }
}
