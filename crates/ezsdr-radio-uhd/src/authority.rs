//! `DeviceAuthority`: the device-paced Time Authority whose primary root is the
//! USRP's master clock (UR-6, UR-7, UR-8; TM-16a1). `now` extrapolates from an anchor
//! that the thread `uhd-clock` re-reads every 100 ms, and every nap of `next_wakeup`
//! and `wait_until` re-reads as well.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{Authority, AuthorityDescriptor, Pacing};
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, ClockRelation, EpochRef, Rational, ScheduleHandle, TimeAuthority,
    TimeError, TimePoint, Validity,
};

use crate::device::Device;

type Callback = Box<dyn FnOnce(TimePoint) + Send>;

/// The longest single wait of `next_wakeup` and `wait_until` (UR-7).
const NAP: Duration = Duration::from_millis(20);
/// How often `uhd-clock` re-reads the device (UR-7).
const REANCHOR: Duration = Duration::from_millis(100);
/// A read bracketed more widely than this is discarded (UR-7).
const WIDEST_BRACKET: Duration = Duration::from_millis(1);
/// TM-17b's cap on callbacks at one instant.
const CALLBACK_CAP: usize = 1_000;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone, Copy)]
struct Anchor {
    host: Instant,
    device: i64,
}

struct Schedule {
    last_fired: Option<i64>,
    pending: BTreeMap<(i64, u64), Callback>,
    next_seq: u64,
}

/// The Authority's clock; the coordinator holds it as its `TimeAuthority`.
pub(crate) struct DeviceTime {
    device: Arc<dyn Device>,
    clocks: Arc<ClockRegistry>,
    root: ClockDomainId,
    mcr: u64,
    built: Instant,
    anchor: Mutex<Anchor>,
    last: AtomicI64,
    schedule: Mutex<Schedule>,
    changed: Condvar,
    failed_reads: AtomicU64,
}

impl DeviceTime {
    /// One device read bracketed by host reads (UR-3's brackets, UR-7's width rule).
    fn read(&self) -> Option<(Anchor, Duration)> {
        let before = Instant::now();
        let device = self.device.time_now();
        let after = Instant::now();
        match device {
            Ok(device) => {
                let width = after - before;
                Some((Anchor { host: before + width / 2, device }, width))
            }
            Err(_) => {
                self.failed_reads.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    fn reanchor(&self) {
        if let Some((anchor, width)) = self.read() {
            if width <= WIDEST_BRACKET {
                *lock(&self.anchor) = anchor;
            }
        }
    }

    fn extrapolate(&self) -> i64 {
        let anchor = *lock(&self.anchor);
        let elapsed = Instant::now().saturating_duration_since(anchor.host).as_nanos() as i128;
        let t = anchor.device + (elapsed * self.mcr as i128 / 1_000_000_000) as i64;
        // Monotonic across a re-anchor that lands behind (UR-7).
        self.last.fetch_max(t, Ordering::AcqRel).max(t)
    }

    fn host_ns(&self) -> i64 {
        self.built.elapsed().as_nanos() as i64
    }

    /// `t` as a root tick, exactly (TM-16c: a firing may not round).
    fn root_tick(&self, t: TimePoint) -> Result<i64, TimeError> {
        if t.domain == self.root {
            return Ok(t.ticks);
        }
        if t.domain == ClockDomainId::HOST_MONOTONIC {
            let anchor = *lock(&self.anchor);
            let anchor_ns = anchor.host.saturating_duration_since(self.built).as_nanos() as i128;
            let delta = t.ticks as i128 - anchor_ns;
            return Ok(anchor.device + (delta * self.mcr as i128 / 1_000_000_000) as i64);
        }
        if !self.governs(t.domain) {
            return Err(TimeError::NotGoverned { id: t.domain });
        }
        Ok(self.clocks.conversion(t.domain, self.root)?.try_exact(t)?.ticks)
    }

    /// Waits up to one nap for `target`, woken by `schedule` and `cancel`.
    fn nap<'a>(&self, guard: MutexGuard<'a, Schedule>, target: i64) -> MutexGuard<'a, Schedule> {
        let now = self.extrapolate();
        let wait = Duration::from_nanos(((target - now).max(0) as i128 * 1_000_000_000 / self.mcr as i128) as u64);
        let (guard, _) = self
            .changed
            .wait_timeout(guard, wait.min(NAP))
            .unwrap_or_else(|e| e.into_inner());
        guard
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
            || self.clocks.get(domain).is_ok_and(|d| d.root_id() == self.root)
    }

    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError> {
        if domain == ClockDomainId::HOST_MONOTONIC {
            return Ok(TimePoint::new(domain, self.host_ns()));
        }
        if !self.governs(domain) {
            return Err(TimeError::NotGoverned { id: domain });
        }
        let root = TimePoint::new(self.root, self.extrapolate());
        if domain == self.root {
            return Ok(root);
        }
        Ok(self.clocks.convert(root, domain)?.floor())
    }

    fn wait_until(&self, t: TimePoint) -> Result<(), TimeError> {
        let target = self.root_tick(t)?;
        while self.extrapolate() < target {
            let now = self.extrapolate();
            let wait = Duration::from_nanos(((target - now).max(0) as i128 * 1_000_000_000 / self.mcr as i128) as u64);
            std::thread::sleep(wait.min(NAP));
            self.reanchor();
        }
        Ok(())
    }

    /// TM-16c as KG-9 amends it: only an instant before the last one fired is refused.
    fn schedule(&self, t: TimePoint, f: Callback) -> Result<ScheduleHandle, TimeError> {
        let ticks = self.root_tick(t)?;
        let mut schedule = lock(&self.schedule);
        if let Some(fired) = schedule.last_fired.filter(|fired| ticks < *fired) {
            return Err(TimeError::InPast { now: TimePoint::new(self.root, fired), requested: t });
        }
        let seq = schedule.next_seq;
        schedule.next_seq += 1;
        schedule.pending.insert((ticks, seq), f);
        drop(schedule);
        self.changed.notify_all();
        Ok(ScheduleHandle { root: self.root, ticks, seq })
    }

    /// MA-29 as KG-3 amends it: a cancel wakes a waiting `next_wakeup`.
    fn cancel(&self, h: ScheduleHandle) -> bool {
        let removed = lock(&self.schedule).pending.remove(&(h.ticks, h.seq)).is_some();
        self.changed.notify_all();
        removed
    }
}

/// The Authority role of `ezsdr.radio.uhd` (UR-6, UR-7).
pub struct DeviceAuthority {
    time: Arc<DeviceTime>,
    descriptor: AuthorityDescriptor,
    relations: Vec<ClockRelation>,
    stop: Arc<AtomicBool>,
}

impl DeviceAuthority {
    /// Sets the reference sources (the clock source first), zeroes the device's time
    /// (at the next PPS when `time_source` is not `internal`), registers the master
    /// clock as a Root and measures its relations (UR-7, UR-8).
    pub fn new(
        device: Arc<dyn Device>,
        clocks: Arc<ClockRegistry>,
        clock_source: &str,
        time_source: &str,
        args: &str,
    ) -> Result<DeviceAuthority, String> {
        device.set_sources(clock_source, time_source).map_err(|e| format!("UR-7: {e}"))?;
        let pps = time_source != "internal";
        device.set_time_zero(pps).map_err(|e| format!("UR-7: {e}"))?;
        let mcr = device.master_clock_rate();
        let rate = Rational::new(mcr, 1).map_err(|e| format!("UR-7: {e}"))?;
        let root = clocks.allocate_id();
        let set_by = if pps { "set_time_unknown_pps" } else { "set_time_now" };
        clocks
            .register(ClockDomain::root(
                root,
                rate,
                EpochRef::Arbitrary { set_by: format!("ezsdr.radio.uhd.{set_by}:{args}") },
            ))
            .map_err(|e| format!("UR-7: {e}"))?;
        let built = Instant::now();
        let utc = || {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as i64)
        };
        // UR-8: one device read bracketed by the monotonic clock and UTC.
        let (mono_before, utc_before) = (Instant::now(), utc());
        let device_tick = device.time_now().map_err(|e| format!("UR-8: {e}"))?;
        let (utc_after, mono_after) = (utc(), Instant::now());
        let ns = |i: Instant| i.duration_since(built).as_nanos() as i64;
        let (mb, ma) = (ns(mono_before), ns(mono_after));
        let measured = TimePoint::new(root, device_tick);
        let relation = |target, offset: i64, uncertainty: i64, drift_uncertainty: f64, method: &str| ClockRelation {
            source: root,
            target,
            measured_at: measured,
            offset: TimePoint::new(target, offset),
            drift: 0.0,
            drift_uncertainty,
            uncertainty: ezsdr_kernel::time::Duration::new(target, uncertainty.max(0)),
            method: method.to_owned(),
            valid: Validity { from: measured, to: None },
        };
        let relations = vec![
            relation(ClockDomainId::HOST_MONOTONIC, mb + (ma - mb) / 2, ma - mb, 1e-4, "ezsdr.radio.uhd.host_bracket"),
            relation(
                ClockDomainId::UTC,
                utc_before + (utc_after - utc_before) / 2,
                utc_after - utc_before,
                1e-5,
                "ezsdr.radio.uhd.host_utc_bracket",
            ),
        ];
        let time = Arc::new(DeviceTime {
            device,
            clocks,
            root,
            mcr,
            built,
            anchor: Mutex::new(Anchor { host: mono_before + (mono_after - mono_before) / 2, device: device_tick }),
            last: AtomicI64::new(i64::MIN),
            schedule: Mutex::new(Schedule { last_fired: None, pending: BTreeMap::new(), next_seq: 0 }),
            changed: Condvar::new(),
            failed_reads: AtomicU64::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (clock, flag) = (Arc::downgrade(&time), stop.clone());
        std::thread::Builder::new()
            .name("uhd-clock".to_owned())
            .spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    std::thread::sleep(REANCHOR);
                    match clock.upgrade() {
                        Some(time) => time.reanchor(),
                        None => return,
                    }
                }
            })
            .map_err(|e| format!("UR-7: uhd-clock: {e}"))?;
        let descriptor = AuthorityDescriptor {
            module: crate::module_ref(),
            governs: vec![root, ClockDomainId::HOST_MONOTONIC],
            pacing: Pacing::Device,
        };
        Ok(DeviceAuthority { time, descriptor, relations, stop })
    }

    /// The device's Root.
    pub fn root(&self) -> ClockDomainId {
        self.time.root
    }

    /// Device time reads that failed and left the anchor as it was (UR-7).
    pub fn failed_reads(&self) -> u64 {
        self.time.failed_reads.load(Ordering::Relaxed)
    }
}

impl Drop for DeviceAuthority {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
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
        let time = &self.time;
        let mut schedule = lock(&time.schedule);
        let tick = loop {
            let (&(tick, _), _) = schedule.pending.first_key_value()?;
            if time.extrapolate() >= tick {
                break tick;
            }
            schedule = time.nap(schedule, tick);
            drop(schedule);
            time.reanchor();
            schedule = lock(&time.schedule);
        };
        schedule.last_fired = Some(tick);
        // MA-29 as KG-3 amends it: callbacks run without the lock `schedule` takes.
        for _ in 0..CALLBACK_CAP {
            let due = schedule.pending.first_key_value().is_some_and(|(&(t, _), _)| t == tick);
            if !due {
                break;
            }
            let (_, callback) = schedule.pending.pop_first().expect("checked above");
            drop(schedule);
            callback(TimePoint::new(time.root, tick));
            schedule = lock(&time.schedule);
        }
        Some(TimePoint::new(time.root, tick))
    }

    fn relations(&self) -> Vec<ClockRelation> {
        self.relations.clone()
    }
}
