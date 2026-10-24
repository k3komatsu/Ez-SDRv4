//! `DeviceAuthority`: the device-paced Time Authority whose primary root is the
//! USRP's master clock (UR-6, UR-7, UR-8; TM-16a1). `now` extrapolates from an anchor
//! that the thread `uhd-clock` re-reads every 100 ms, and every nap of `next_wakeup`
//! and `wait_until` re-reads as well.
//!
//! The host's time is read from the device's host clock ([`HostClock`]): the real clock,
//! or the manual one a `FakeDevice` may be built with. The anchor holds the device read in
//! the host clock's time and in real time; `now(host.monotonic)` is real, and a
//! `host.monotonic` wait or schedule is judged on real host time, never converted to root
//! ticks (spec 18 UR-7, plan 30 §4.6).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{Authority, AuthorityDescriptor, Pacing};
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, ClockRelation, EpochRef, Rational, ScheduleHandle, TimeAuthority,
    TimeError, TimePoint, Validity,
};

use crate::device::Device;
use crate::host::HostClock;

type Callback = Box<dyn FnOnce(TimePoint) + Send>;
/// `uhd-clock`'s stop flag.
type ClockStop = Arc<Mutex<bool>>;

/// The longest single wait of `next_wakeup` and `wait_until` (UR-7).
const NAP: Duration = Duration::from_millis(20);
/// How often `uhd-clock` re-reads the device (UR-7).
const REANCHOR: Duration = Duration::from_millis(100);
/// A read bracketed more widely than this is discarded (UR-7).
const WIDEST_BRACKET: Duration = Duration::from_millis(1);
/// TM-17b's cap on callbacks at one instant.
const CALLBACK_CAP: usize = 1_000;
/// How `DeviceAuthority::new`'s error begins when the X300's reference PLL did not lock
/// within UHD's 30 s (UR-7; design-notes §18): a builder may open the device again on it.
pub const REFERENCE_DID_NOT_LOCK: &str = "UR-7: the reference clock did not lock";

/// How long `drop` waits for `uhd-clock` to end; a read blocked inside UHD is left
/// detached after it (D-1).
const CLOCK_JOIN: Duration = Duration::from_secs(1);
/// How long `new` waits on a manual clock for `uhd-clock`'s first read, the first anchor
/// (UR-7); a device that has not answered by then refuses the Authority.
const FIRST_ANCHOR: Duration = Duration::from_secs(5);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone, Copy)]
struct Anchor {
    /// The host clock's time at the read's midpoint. `extrapolate` and UR-7's 1 ms bracket
    /// rule run on this (plan 30 §4.6).
    host: Duration,
    /// The real instant at the read's midpoint (§4.6). Unused today: the real clock is only
    /// `now(host.monotonic)`, which is `built.elapsed()`, and the relations measure it.
    #[allow(dead_code)]
    real: Instant,
    device: i64,
}

/// One timeline's scheduled instants: KG-9's `last_fired` and the pending callbacks, in
/// `(instant, sequence)` order (TM-16c).
#[derive(Default)]
struct Queue {
    last_fired: Option<i64>,
    pending: BTreeMap<(i64, u64), Callback>,
}

/// The two timelines: the device root and its derived domains share one (`root`), and
/// `host.monotonic` the other (plan 30 §4.6): their instants are not comparable.
#[derive(Default)]
struct Schedules {
    root: Queue,
    host: Queue,
    next_seq: u64,
}

/// The Authority's clock; the coordinator holds it as its `TimeAuthority`.
pub(crate) struct DeviceTime {
    device: Arc<dyn Device>,
    host: HostClock,
    clocks: Arc<ClockRegistry>,
    root: ClockDomainId,
    mcr: u64,
    /// Real instants for `now(host.monotonic)` (UR-7).
    built: Instant,
    anchor: Mutex<Anchor>,
    last: AtomicI64,
    schedule: Mutex<Schedules>,
    failed_reads: AtomicU64,
    discarded_reads: AtomicU64,
}

impl DeviceTime {
    /// One device read bracketed by host-clock reads (UR-3's brackets, UR-7's width rule).
    fn read(&self) -> Option<(Anchor, Duration)> {
        let before = self.host.now();
        let real_before = Instant::now();
        let device = self.device.time_now();
        let after = self.host.now();
        let real_after = Instant::now();
        match device {
            Ok(device) => {
                let width = after.saturating_sub(before);
                let real = real_before + (real_after - real_before) / 2;
                Some((Anchor { host: before + width / 2, real, device }, width))
            }
            Err(_) => {
                self.failed_reads.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Re-reads the device and sets the anchor when UR-7's rules let it; returns whether it did
    /// (on a manual clock: whether the read was exact).
    fn reanchor(&self) -> bool {
        if let Some((anchor, width)) = self.read() {
            if width > WIDEST_BRACKET {
                self.discarded_reads.fetch_add(1, Ordering::Relaxed);
            } else if !self.host.is_manual() || width.is_zero() {
                *lock(&self.anchor) = anchor;
                return true;
            }
            // On a manual clock a read during which the clock moved does not pin the host
            // instant of the device's value (it is anywhere in the bracket, ±width/2), and the
            // device's time there is a pure function of that clock, so the previous anchor is
            // kept: only a read with no movement names the instant. `uhd-clock`'s reads are
            // always of that kind — a participant is busy while it reads, so the clock is
            // stopped — while a nap's read of `next_wakeup` or `wait_until` usually is not, and
            // its midpoint estimate would make the extrapolation lead by up to width/2.
        }
        false
    }

    fn extrapolate(&self) -> i64 {
        let anchor = *lock(&self.anchor);
        let elapsed = self.host.now().saturating_sub(anchor.host).as_nanos() as i128;
        let t = anchor.device + (elapsed * self.mcr as i128 / 1_000_000_000) as i64;
        // Monotonic across a re-anchor that lands behind (UR-7).
        self.last.fetch_max(t, Ordering::AcqRel).max(t)
    }

    /// The real host's monotonic time since the Authority was built (UR-7).
    fn host_ns(&self) -> i64 {
        self.built.elapsed().as_nanos() as i64
    }

    /// `t` as a root tick, exactly (TM-16c: a firing may not round). `host.monotonic` is
    /// never converted here: its waits and schedules are judged on real host time.
    fn root_tick(&self, t: TimePoint) -> Result<i64, TimeError> {
        if let Ok(ticks) = t.ticks_in(self.root) {
            return Ok(ticks);
        }
        if !self.governs(t.domain()) {
            return Err(TimeError::NotGoverned { id: t.domain() });
        }
        self.clocks.conversion(t.domain(), self.root)?.try_exact(t)?.ticks_in(self.root)
    }

    /// `ticks` of the root as the host clock's time, rounded up so that a wait reaches them.
    fn host_time(&self, ticks: i64) -> Duration {
        let ns = (ticks.max(0) as u128 * 1_000_000_000).div_ceil(self.mcr as u128);
        Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
    }

    /// The next nap's deadlines (UR-7, plan 30 §4.6): in the host clock, the root entry's
    /// remaining ticks, at most [`NAP`] so that the anchor is re-read; in real time, the
    /// `host.monotonic` entry's instant. A stopped manual clock reaches no manual deadline,
    /// so the nap blocks instead of turning a frozen remainder into a real wait.
    fn nap(&self, root_next: Option<i64>, host_next: Option<i64>) -> (Option<Duration>, Option<Instant>) {
        let wait = root_next.map_or(NAP, |tick| self.host_time(tick - self.extrapolate()).min(NAP));
        let real = host_next.map(|ns| {
            let remaining = Duration::from_nanos((ns - self.host_ns()).max(0) as u64);
            Instant::now() + remaining
        });
        (Some(self.host.now() + wait), real)
    }

    /// Fires the callbacks due at root `tick`; `None` when the instant `next_wakeup` read is no
    /// longer the queue's first (a concurrent `cancel` or a nearer `schedule` won the lock
    /// first), so the caller reads the queue again instead of firing a stale instant.
    fn fire_root(&self, tick: i64) -> Option<TimePoint> {
        let mut s = lock(&self.schedule);
        if !s.root.pending.first_key_value().is_some_and(|(&(t, _), _)| t == tick) {
            return None;
        }
        s.root.last_fired = Some(tick);
        // MA-29 as KG-3 amends it: callbacks run without the lock `schedule` takes.
        for _ in 0..CALLBACK_CAP {
            let due = s.root.pending.first_key_value().is_some_and(|(&(t, _), _)| t == tick);
            if !due {
                break;
            }
            let (_, callback) = s.root.pending.pop_first().expect("checked above");
            drop(s);
            callback(TimePoint::new(self.root, tick));
            s = lock(&self.schedule);
        }
        Some(TimePoint::new(self.root, tick))
    }

    /// `fire_root` for the `host.monotonic` timeline.
    fn fire_host(&self, ns: i64) -> Option<TimePoint> {
        let mut s = lock(&self.schedule);
        if !s.host.pending.first_key_value().is_some_and(|(&(t, _), _)| t == ns) {
            return None;
        }
        s.host.last_fired = Some(ns);
        for _ in 0..CALLBACK_CAP {
            let due = s.host.pending.first_key_value().is_some_and(|(&(t, _), _)| t == ns);
            if !due {
                break;
            }
            let (_, callback) = s.host.pending.pop_first().expect("checked above");
            drop(s);
            callback(TimePoint::new(ClockDomainId::HOST_MONOTONIC, ns));
            s = lock(&self.schedule);
        }
        Some(TimePoint::new(ClockDomainId::HOST_MONOTONIC, ns))
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
        // A `host.monotonic` instant is judged on real host time, never converted to the
        // root, so it is reached even while the host clock is stopped (plan 30 §4.6).
        if t.domain() == ClockDomainId::HOST_MONOTONIC {
            let target = t.ticks_in(ClockDomainId::HOST_MONOTONIC)?;
            while self.host_ns() < target {
                let wait = Duration::from_nanos((target - self.host_ns()).max(0) as u64);
                // Real time on purpose, not a clock wait: only Kernel non-participants call
                // this (M6), and making it a clock wait would stop the device's clock for a
                // Provider thread that called it (plan 30 §4.6, item 13).
                std::thread::sleep(wait.min(NAP));
                self.reanchor();
            }
            return Ok(());
        }
        let target = self.root_tick(t)?;
        loop {
            let now = self.extrapolate();
            if now >= target {
                return Ok(());
            }
            // A clock sleep: it blocks while the manual clock is stopped.
            self.host.sleep(self.host_time(target - now).min(NAP));
            self.reanchor();
        }
    }

    /// TM-16c as KG-9 amends it: only an instant before the last one fired is refused. The
    /// root and its derived domains share one queue and `last_fired`, `host.monotonic` the
    /// other (plan 30 §4.6).
    fn schedule(&self, t: TimePoint, f: Callback) -> Result<ScheduleHandle, TimeError> {
        if t.domain() == ClockDomainId::HOST_MONOTONIC {
            let ns = t.ticks_in(ClockDomainId::HOST_MONOTONIC)?;
            let mut s = lock(&self.schedule);
            if let Some(fired) = s.host.last_fired.filter(|fired| ns < *fired) {
                return Err(TimeError::InPast { now: TimePoint::new(ClockDomainId::HOST_MONOTONIC, fired), requested: t });
            }
            let seq = s.next_seq;
            s.next_seq += 1;
            s.host.pending.insert((ns, seq), f);
            drop(s);
            self.host.notify_all();
            return Ok(ScheduleHandle { root: ClockDomainId::HOST_MONOTONIC, ticks: ns, seq });
        }
        let ticks = self.root_tick(t)?;
        let mut s = lock(&self.schedule);
        if let Some(fired) = s.root.last_fired.filter(|fired| ticks < *fired) {
            return Err(TimeError::InPast { now: TimePoint::new(self.root, fired), requested: t });
        }
        let seq = s.next_seq;
        s.next_seq += 1;
        s.root.pending.insert((ticks, seq), f);
        drop(s);
        self.host.notify_all();
        Ok(ScheduleHandle { root: self.root, ticks, seq })
    }

    /// MA-29 as KG-3 amends it: a cancel wakes a waiting `next_wakeup`.
    fn cancel(&self, h: ScheduleHandle) -> bool {
        let removed = if h.root == self.root {
            lock(&self.schedule).root.pending.remove(&(h.ticks, h.seq)).is_some()
        } else if h.root == ClockDomainId::HOST_MONOTONIC {
            lock(&self.schedule).host.pending.remove(&(h.ticks, h.seq)).is_some()
        } else {
            false
        };
        self.host.notify_all();
        removed
    }
}

/// The Authority role of `ezsdr.radio.uhd` (UR-6, UR-7).
pub struct DeviceAuthority {
    time: Arc<DeviceTime>,
    descriptor: AuthorityDescriptor,
    relations: Vec<ClockRelation>,
    /// `uhd-clock`'s stop and the thread.
    clock: Option<(ClockStop, JoinHandle<()>)>,
}

/// UR-6, UR-7: the epoch of the root a device's time sets, its `set_by` naming how: its PPS
/// edge when both sources are not internal, so that every whole second of the root is a PPS
/// edge; otherwise arbitrary, set at a PPS edge when the time source is not internal and at
/// once when it is.
pub(crate) fn epoch(clock_source: &str, time_source: &str, args: &str) -> EpochRef {
    if time_source == "internal" {
        return EpochRef::Arbitrary { set_by: format!("ezsdr.radio.uhd.set_time_now:{args}") };
    }
    let set_by = format!("ezsdr.radio.uhd.set_time_unknown_pps:{args}");
    if clock_source == "internal" { EpochRef::Arbitrary { set_by } } else { EpochRef::Pps { set_by } }
}

impl DeviceAuthority {
    /// Sets the reference sources (the clock source first), zeroes the device's time
    /// (at the next PPS when `time_source` is not `internal`), registers the master
    /// clock as a Root and measures its relations (UR-7, UR-8). On a manual host clock it
    /// returns only once `uhd-clock`'s first read, the first anchor, was exact (UR-7).
    pub fn new(
        device: Arc<dyn Device>,
        clocks: Arc<ClockRegistry>,
        clock_source: &str,
        time_source: &str,
        args: &str,
    ) -> Result<DeviceAuthority, String> {
        device.set_sources(clock_source, time_source).map_err(|e| {
            // The X300's reference PLL not locking within UHD's 30 s, named for the client;
            // a new open usually locks (bench-results.md parts 3–9; design-notes §18).
            if e.message.contains("Reference Clock PLL failed to lock") {
                format!("{REFERENCE_DID_NOT_LOCK} to its {clock_source} source within UHD's 30 s; connecting again usually succeeds ({e})")
            } else {
                format!("UR-7: {e}")
            }
        })?;
        device.set_time_zero(time_source != "internal").map_err(|e| format!("UR-7: {e}"))?;
        let mcr = device.master_clock_rate();
        let rate = Rational::new(mcr, 1).map_err(|e| format!("UR-7: {e}"))?;
        let root = clocks.allocate_id().map_err(|e| format!("UR-7: {e}"))?;
        clocks
            .register(ClockDomain::root(root, rate, epoch(clock_source, time_source, args)))
            .map_err(|e| format!("UR-7: {e}"))?;
        let host = device.host();
        let built = Instant::now();
        let utc = || {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as i64)
        };
        // UR-8: one device read bracketed by the monotonic clock and UTC, and by the host
        // clock (plan 30 §4.6: the anchor takes the host clock's reading).
        let (mono_before, utc_before) = (Instant::now(), utc());
        let host_before = host.now();
        let device_tick = device.time_now().map_err(|e| format!("UR-8: {e}"))?;
        let host_after = host.now();
        let (utc_after, mono_after) = (utc(), Instant::now());
        let ns = |i: Instant| i.duration_since(built).as_nanos() as i64;
        let (mb, ma) = (ns(mono_before), ns(mono_after));
        let measured = TimePoint::new(root, device_tick);
        // UR-8 as plan 30 §4.6 amends it: with the manual clock the device time stands still
        // while the host and UTC go on, so the relation holds only at the instant measured.
        let validity = if host.is_manual() { Validity { from: measured, to: Some(measured) } } else { Validity { from: measured, to: None } };
        let relation = |target, offset: i64, uncertainty: i64, drift_uncertainty: f64, method: &str, validity: Validity| {
            ClockRelation::new(
                root,
                target,
                measured,
                TimePoint::new(target, offset),
                0.0,
                drift_uncertainty,
                ezsdr_kernel::time::Duration::new(target, uncertainty.max(0)),
                method.to_owned(),
                validity,
            )
            .map_err(|e| format!("UR-8: {e}"))
        };
        let relations = vec![
            relation(
                ClockDomainId::HOST_MONOTONIC,
                mb + (ma - mb) / 2,
                ma - mb,
                1e-4,
                "ezsdr.radio.uhd.host_bracket",
                validity,
            )?,
            relation(
                ClockDomainId::UTC,
                utc_before + (utc_after - utc_before) / 2,
                utc_after - utc_before,
                1e-5,
                "ezsdr.radio.uhd.host_utc_bracket",
                validity,
            )?,
        ];
        // On a manual clock only provisional: this read ran on a running clock, so its midpoint
        // can be off by half its bracket, and uhd-clock's first read replaces it before `new`
        // returns (UR-7).
        let anchor = Anchor {
            host: host_before + (host_after - host_before) / 2,
            real: mono_before + (mono_after - mono_before) / 2,
            device: device_tick,
        };
        let time = Arc::new(DeviceTime {
            device,
            host,
            clocks,
            root,
            mcr,
            built,
            anchor: Mutex::new(anchor),
            last: AtomicI64::new(i64::MIN),
            schedule: Mutex::new(Schedules::default()),
            failed_reads: AtomicU64::new(0),
            discarded_reads: AtomicU64::new(0),
        });
        let stop: ClockStop = Arc::new(Mutex::new(false));
        let (clock, flag) = (Arc::downgrade(&time), stop.clone());
        let clock_host = time.host.clone();
        let thread_clock = clock_host.clone();
        // UR-7 on a manual clock: the first anchor is uhd-clock's first read, made before its
        // first wait while it is busy, so the clock stands still and the read is exact; `new`
        // waits for it. On the real clock `new`'s own read stays the first anchor.
        let manual = clock_host.is_manual();
        let (sender, first_read) = std::sync::mpsc::channel();
        let mut first = manual.then_some(sender);
        #[cfg(test)]
        let mut pause = hooks::take();
        let thread = clock_host
            .spawn("uhd-clock", move || loop {
                // Woken at once by `drop`, and the flag checked before the device is read,
                // so that no read follows the Authority's end (D-1).
                if *lock(&flag) {
                    return;
                }
                if let Some(report) = first.take() {
                    let _ = report.send(clock.upgrade().is_some_and(|time| time.reanchor()));
                }
                // `drop` sets the flag before it notifies: reading the generation before this
                // second check means a notify between them cannot be missed, so drop ends the
                // nap at once even while the clock is stopped (UR-7's D-1).
                let seen = thread_clock.generation();
                if *lock(&flag) {
                    return;
                }
                thread_clock.wait(Some(thread_clock.now() + REANCHOR), None, seen);
                if *lock(&flag) {
                    return;
                }
                #[cfg(test)]
                if let Some((reached, release)) = pause.take() {
                    let _ = reached.send(());
                    let _ = release.recv();
                }
                match clock.upgrade() {
                    Some(time) => time.reanchor(),
                    None => return,
                };
            })
            .map_err(|e| format!("UR-7: uhd-clock: {e}"))?;
        let descriptor = AuthorityDescriptor {
            module: crate::module_ref(),
            governs: vec![root, ClockDomainId::HOST_MONOTONIC],
            pacing: Pacing::Device,
        };
        let authority = DeviceAuthority { time, descriptor, relations, clock: Some((stop, thread)) };
        if manual {
            // A first read that is not exact, or does not come, refuses the Authority, whose
            // drop then stops uhd-clock.
            match first_read.recv_timeout(FIRST_ANCHOR) {
                Ok(true) => {}
                Ok(false) => return Err("UR-7: the device's first time read was not exact on the manual host clock: it failed, or the clock moved while it ran".to_owned()),
                Err(e) => return Err(format!("UR-7: the device's first time read did not come within {FIRST_ANCHOR:?} ({e})")),
            }
        }
        Ok(authority)
    }

    /// The device's Root.
    pub fn root(&self) -> ClockDomainId {
        self.time.root
    }

    /// Device time reads that failed and left the anchor as it was (UR-7).
    pub fn failed_reads(&self) -> u64 {
        self.time.failed_reads.load(Ordering::Relaxed)
    }

    /// Device time reads bracketed more widely than 1 ms, discarded (UR-7).
    pub fn discarded_reads(&self) -> u64 {
        self.time.discarded_reads.load(Ordering::Relaxed)
    }
}

impl Drop for DeviceAuthority {
    /// Stops `uhd-clock` and waits for it, so that no device call outlives the Authority
    /// and the last device handle is not dropped on that thread while the process exits,
    /// racing UHD's static registry of open devices (`usrp_c.cpp:80–84`; design-notes §15,
    /// D-1). A read blocked inside UHD past `CLOCK_JOIN` leaves the thread detached.
    fn drop(&mut self) {
        let Some((stop, thread)) = self.clock.take() else { return };
        *lock(&stop) = true;
        self.time.host.notify_all();
        let deadline = Instant::now() + CLOCK_JOIN;
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        // Past the deadline the handle is dropped, detaching the thread: a read blocked
        // that long is a device gone, which the Provider reports (UR-29).
        if thread.is_finished() {
            let _ = thread.join();
        }
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
        loop {
            // Read before the queue: a `schedule` or `cancel` after the reading below bumps it,
            // so the wait returns at once and the queue is read again; no wake is lost.
            let seen = self.time.host.generation();
            let (root_next, host_next) = {
                let s = lock(&self.time.schedule);
                (
                    s.root.pending.first_key_value().map(|(&(t, _), _)| t),
                    s.host.pending.first_key_value().map(|(&(t, _), _)| t),
                )
            };
            if let Some(tick) = root_next.filter(|tick| *tick <= self.time.extrapolate()) {
                if let Some(fired) = self.time.fire_root(tick) {
                    return Some(fired);
                }
                continue; // the entry went away under us: read the queue again
            }
            let now_host = self.time.host_ns();
            if let Some(ns) = host_next.filter(|ns| *ns <= now_host) {
                if let Some(fired) = self.time.fire_host(ns) {
                    return Some(fired);
                }
                continue;
            }
            if root_next.is_none() && host_next.is_none() {
                return None;
            }
            // One wait, ended by the root entry's deadline in the host clock, the HOST entry's in
            // real time or a notify, whichever comes first (plan 30 §4.6).
            let (manual, real) = self.time.nap(root_next, host_next);
            self.time.host.wait(manual, real, seen);
            self.time.reanchor();
        }
    }

    fn relations(&self) -> Vec<ClockRelation> {
        self.relations.clone()
    }
}

/// Test hooks, per thread as `host.rs`'s are: a test arms the `uhd-clock` of the Authority it is
/// about to build, since the thread does not exist before `new`.
#[cfg(test)]
mod hooks {
    use std::cell::Cell;
    use std::sync::mpsc::{Receiver, Sender};

    pub(super) type Pause = (Sender<()>, Receiver<()>);

    thread_local! {
        static BEFORE_FIRST_REANCHOR: Cell<Option<Pause>> = const { Cell::new(None) };
    }

    /// The next `DeviceAuthority::new` on this thread makes its `uhd-clock`, once its first nap
    /// is over and before that nap's re-anchor, send on `reached` and block until `release`
    /// receives (or its sender is dropped): busy all the while, so a manual clock stands still at
    /// the nap's deadline.
    pub(super) fn pause_before_first_reanchor(reached: Sender<()>, release: Receiver<()>) {
        BEFORE_FIRST_REANCHOR.set(Some((reached, release)));
    }

    pub(super) fn take() -> Option<Pause> {
        BEFORE_FIRST_REANCHOR.take()
    }
}

#[cfg(test)]
mod tests {
    //! The Authority's own behaviour on the manual clock (plan 30 §7): `schedule`, `cancel`,
    //! `drop` and KC-46a's wake path, and the HOST_MONOTONIC timeline — a `wait_until` and a
    //! `schedule` judged on real host time, reached even while the device's clock is stopped.
    //! Every test that could freeze or deadlock has a wall-clock bound (`bounded`, `until`), and
    //! none lets a short wall interval decide a property: where a property needs the device's
    //! clock to stand still, a participant holds it until the test releases it, after its checks.

    use super::*;
    use crate::device::{FakeConfig, FakeDevice, FakeFault};
    use crate::host::{ClockMode, passes};
    use ezsdr_kernel::id::ResourceId;
    use std::sync::mpsc::{channel, Receiver, Sender};

    /// Root ticks of `n` milliseconds on the fake's 200 MHz master clock.
    fn ms(n: i64) -> i64 {
        n * 200_000
    }

    fn host() -> ClockDomainId {
        ClockDomainId::HOST_MONOTONIC
    }

    /// An Authority on a fake device whose clock is `mode`, with its registry and root.
    fn authority(mode: ClockMode, time_source: &str) -> (DeviceAuthority, Arc<FakeDevice>, Arc<ClockRegistry>, ClockDomainId) {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: mode, ..FakeConfig::default() }));
        let clocks = Arc::new(ClockRegistry::new());
        let authority = DeviceAuthority::new(device.clone(), clocks.clone(), "internal", time_source, "test").unwrap();
        let root = authority.root();
        (authority, device, clocks, root)
    }

    /// The clock's reading in `domain`.
    fn ticks(time: &dyn TimeAuthority, domain: ClockDomainId) -> i64 {
        time.now(domain).unwrap().ticks_in(domain).unwrap()
    }

    /// A wall-clock bound on a channel a freezing or deadlocking clock would never feed: the
    /// test fails here instead of hanging the suite (plan 30 §7's guard).
    fn bounded<T>(rx: &Receiver<T>, what: &str) -> T {
        rx.recv_timeout(Duration::from_secs(3)).unwrap_or_else(|_| panic!("{what} within 3 s"))
    }

    /// The same bound for an inline call a freezing clock would never return from.
    fn within<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = channel();
        let _ = std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(3)).unwrap_or_else(|_| panic!("{what} within 3 s"))
    }

    /// Polls `done` until it holds, failing after the same bound.
    fn until(what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !done() {
            assert!(Instant::now() < deadline, "{what} within 3 s");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Holds a participant busy for `d` of real time — a busy `std::thread::sleep`, not a clock
    /// wait — so the manual clock stops for as long as it runs (M3).
    fn hold(device: &FakeDevice, d: Duration) -> JoinHandle<()> {
        device.host().spawn("busy", move || std::thread::sleep(d)).expect("a participant thread")
    }

    /// Holds a participant busy — blocked on a channel, not a clock wait — until the returned
    /// sender sends or is dropped (a failing test's unwind included), so the manual clock stands
    /// still until the test releases it (M3).
    fn hold_until_released(device: &FakeDevice) -> (Sender<()>, JoinHandle<()>) {
        let (release, gate) = channel::<()>();
        let holder = device.host().spawn("busy", move || {
            let _ = gate.recv();
        }).expect("a participant thread");
        (release, holder)
    }

    /// Spawns `next_wakeup` on `authority` and returns once it naps on the held clock (its wait
    /// registered beside uhd-clock's), with the channel its result comes on.
    fn napping(authority: &Arc<DeviceAuthority>, device: &FakeDevice) -> (Receiver<Option<TimePoint>>, JoinHandle<()>) {
        until("uhd-clock waits on the held clock", || device.host().blocked() == 1);
        let (tx, rx) = channel();
        let waiter = { let a = authority.clone(); std::thread::spawn(move || { let _ = tx.send(a.next_wakeup()); }) };
        until("the waiter naps on the held clock", || device.host().blocked() >= 2);
        (rx, waiter)
    }

    #[test]
    fn ur_07_a_manual_schedule_wakes_a_waiting_next_wakeup() {
        // MA-29 as KG-3 amends it: a `schedule` wakes a waiting `next_wakeup`. The waiter naps on
        // a callback far ahead while a participant holds the clock — released only after the
        // checks — so its nap cannot end on its own: only the wake returns the near instant.
        let (authority, device, _clocks, root) = authority(ClockMode::Manual, "internal");
        let authority = Arc::new(authority);
        let time = authority.time();
        let far = time.schedule(TimePoint::new(root, ticks(&*time, root) + ms(10_000)), Box::new(|_| {})).unwrap();
        let (release, holder) = hold_until_released(&device);
        let (rx, waiter) = napping(&authority, &device);
        let now = time.now(root).unwrap();
        time.schedule(now, Box::new(|_| {})).unwrap();
        assert_eq!(bounded(&rx, "a schedule at now woke a waiting next_wakeup on the held clock"), Some(now));
        let _ = release.send(());
        holder.join().unwrap();
        assert!(time.cancel(far));
        let _ = waiter.join();
    }

    #[test]
    fn ur_07_a_manual_cancel_wakes_a_waiting_next_wakeup() {
        // A `cancel` wakes a waiting `next_wakeup`, on a held clock as above: the waiter naps on
        // its only callback, far ahead, and only the wake can return the emptied schedule's None.
        let (authority, device, _clocks, root) = authority(ClockMode::Manual, "internal");
        let authority = Arc::new(authority);
        let time = authority.time();
        let handle = time.schedule(TimePoint::new(root, ticks(&*time, root) + ms(10_000)), Box::new(|_| {})).unwrap();
        let (release, holder) = hold_until_released(&device);
        let (rx, waiter) = napping(&authority, &device);
        assert!(time.cancel(handle));
        assert_eq!(bounded(&rx, "a cancel woke a waiting next_wakeup on the held clock"), None, "the emptied schedule returns None");
        let _ = release.send(());
        holder.join().unwrap();
        let _ = waiter.join();
    }

    #[test]
    fn ur_07_dropping_the_authority_ends_uhd_clock_while_the_manual_clock_is_stopped() {
        // D-1 on the manual clock: `drop` wakes uhd-clock from its clock wait even while a
        // participant holds the clock stopped, so the Authority never waits for the clock. The
        // holder starts before uhd-clock and is released only once uhd-clock has ended — its
        // `Weak` on the Authority's clock gone — so the wait the drop finds it in cannot end by its
        // deadline meanwhile: a drop that does not wake it (Review R's R-1) leaves it waiting, and
        // detached once `drop` gives up after CLOCK_JOIN. This is the drop's promptness pin, and no
        // wall interval decides it (plan 30 D2, R20).
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let (release, holder) = hold_until_released(&device);
        let authority = DeviceAuthority::new(device.clone(), Arc::new(ClockRegistry::new()), "internal", "internal", "test").unwrap();
        let time = authority.time.clone();
        until("uhd-clock entered its wait", || device.host().blocked() > 0);
        drop(authority);
        until("uhd-clock ended while the clock was held", || Arc::weak_count(&time) == 0);
        let _ = release.send(());
        holder.join().unwrap();
    }

    #[test]
    fn ur_07_a_host_monotonic_wait_returns_while_a_participant_is_busy() {
        // A `host.monotonic` wait is judged on real host time (plan 30 §4.6): it ends while a
        // participant holds the device's clock — released only after the checks — at or after its
        // instant, the root standing still. No wall interval decides it (R21).
        let (authority, device, _clocks, root) = authority(ClockMode::Manual, "internal");
        let time = authority.time();
        let (release, holder) = hold_until_released(&device);
        // The device's own count on the held clock. The wait re-anchors on that clock, exactly,
        // so after it the root must read this count, not more. (Not a root reading taken now: an
        // extrapolation from the first anchor floors the elapsed time on its own and can be a
        // tick below the count, which an exact re-anchor adds back.)
        let held = device.time_now().unwrap();
        let target = ticks(&*time, host()) + 100_000_000; // 100 ms of HOST nanoseconds
        {
            let t = time.clone();
            within("the HOST wait on the held clock", move || t.wait_until(TimePoint::new(host(), target))).unwrap();
        }
        assert!(ticks(&*time, host()) >= target, "the HOST wait returned before its instant");
        assert_eq!(ticks(&*time, root), held, "the device's time advanced while a participant was busy");
        let _ = release.send(());
        holder.join().unwrap();
    }

    #[test]
    fn ur_07_the_first_anchor_is_exact_on_the_manual_clock() {
        // UR-7's manual-clock rule with no exception: `new`'s own read here takes 10 ms of the
        // running clock, so its midpoint would lead the device by 5 ms; the first anchor is
        // uhd-clock's first read instead, made while it is busy and the clock stands still. It is
        // seen with uhd-clock held after its first nap, before that nap's re-anchor could repair a
        // wrong first anchor (R19): held, uhd-clock is busy, so the clock stands at the nap's
        // deadline, exactly 100 ms after the first read, where `now` is the device's count to the
        // tick. The pause is armed before `new`, so no delay of this thread opens a window.
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        *device.slow_next_read.lock().unwrap() = Some(Duration::from_millis(10));
        let (reached_tx, reached) = channel();
        let (release, release_rx) = channel();
        hooks::pause_before_first_reanchor(reached_tx, release_rx);
        let authority = DeviceAuthority::new(device.clone(), Arc::new(ClockRegistry::new()), "internal", "internal", "test").unwrap();
        bounded(&reached, "uhd-clock held before its first re-anchor");
        let now = ticks(&*authority.time(), authority.root());
        let count = device.time_now().unwrap();
        let _ = release.send(());
        until("uhd-clock went on after its release", || device.host().blocked() > 0);
        assert_eq!(now, count, "now against the device's count: the first anchor is {} ticks off", now - count);
    }

    #[test]
    fn ur_07_a_first_read_that_is_not_exact_refuses_the_authority() {
        // A device whose time reads all take 5 ms of the clock gives no exact first anchor on the
        // manual clock, so `new` refuses it instead of keeping its own read's estimate (UR-7).
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, faults: vec![FakeFault::SlowTimeRead(Duration::ZERO)], ..FakeConfig::default() }));
        let refused = within("the refused new", move || DeviceAuthority::new(device, Arc::new(ClockRegistry::new()), "internal", "internal", "test").err());
        let error = refused.expect("the Authority was refused");
        assert!(error.starts_with("UR-7: the device's first time read"), "{error}");
    }

    #[test]
    fn ur_07_a_read_bracketed_wider_than_1_ms_is_discarded() {
        // UR-7: a device read that took 5 ms is not an anchor (Review L, L05). The reads turn slow
        // at 50 ms of device time; the Authority is built while a participant holds the clock at
        // its zero, so its first read is quick and exact whatever the host's load, and uhd-clock's
        // re-anchors at 100, 205 and 310 ms of the clock are slow. (Moved from tests/fake.rs, whose
        // fault slowed the first read too: that now refuses the Authority, as the test above pins.)
        let slow = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, faults: vec![FakeFault::SlowTimeRead(Duration::from_millis(50))], ..FakeConfig::default() }));
        let (release, holder) = hold_until_released(&slow);
        let slow_authority = DeviceAuthority::new(slow.clone(), Arc::new(ClockRegistry::new()), "internal", "internal", "test").unwrap();
        let _ = release.send(());
        holder.join().unwrap();
        within("350 ms of the slow device's clock", move || slow.host().sleep(Duration::from_millis(350)));
        assert!(slow_authority.discarded_reads() >= 2, "{}", slow_authority.discarded_reads());
        let (quick, quick_device, _clocks, _root) = authority(ClockMode::Manual, "internal");
        within("350 ms of the quick device's clock", move || quick_device.host().sleep(Duration::from_millis(350)));
        assert_eq!(quick.discarded_reads(), 0);
    }

    #[test]
    fn ur_07_uhd_clock_re_anchors_follow_a_drifting_device() {
        // UR-7 (plan 30 §4.5, row U35): `uhd-clock` re-reads the device every 100 ms, so a
        // device drifting 100 ppm against the host clock is followed to within one period's
        // drift — 10 µs, 2 000 root ticks — however long it has run; an Authority whose
        // `uhd-clock` never got past its first nap keeps its first anchor and is 50 µs
        // (10 000 ticks) behind after 500 ms. The clock is held by a participant for both
        // reads, so both are of one instant and a delayed test thread moves neither.
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, drift_ppm: 100.0, ..FakeConfig::default() }));
        let authority = DeviceAuthority::new(device.clone(), Arc::new(ClockRegistry::new()), "internal", "internal", "test").unwrap();
        let time = authority.time();
        {
            let d = device.clone();
            within("500 ms of the drifting device's clock", move || d.host().sleep(Duration::from_millis(500)));
        }
        let (release, holder) = hold_until_released(&device);
        let count = device.time_now().unwrap();
        let now = ticks(&*time, authority.root());
        assert_eq!(device.time_now().unwrap(), count, "the device's clock moved between its read and the Authority's");
        let _ = release.send(());
        holder.join().unwrap();
        assert!((now - count).abs() <= 2_500, "{now} ticks against the device's {count}: not the drift of one re-anchor period (100 ppm over 100 ms is 2 000)");
    }

    #[test]
    fn ur_07_a_host_deadline_reaches_a_waiting_next_wakeup_while_a_participant_is_busy() {
        // A HOST deadline to come, added while the waiter naps on a root entry only, is reached
        // on real time while a participant holds the device's clock — released only after the
        // checks — so it fires at or after its instant, the root standing still (plan 30 §4.6,
        // U02). No wall interval decides it (R21).
        let (authority, device, _clocks, root) = authority(ClockMode::Manual, "internal");
        let authority = Arc::new(authority);
        let time = authority.time();
        let far = time.schedule(TimePoint::new(root, ticks(&*time, root) + ms(10_000)), Box::new(|_| {})).unwrap();
        let (release, holder) = hold_until_released(&device);
        let held = device.time_now().unwrap();
        let (rx, waiter) = napping(&authority, &device);
        let at = TimePoint::new(host(), ticks(&*time, host()) + 80_000_000);
        time.schedule(at, Box::new(|_| {})).unwrap();
        assert_eq!(bounded(&rx, "a future HOST deadline reached a waiting next_wakeup on the held clock"), Some(at));
        assert!(ticks(&*time, host()) >= at.ticks_in(host()).unwrap(), "the HOST instant fired before its real time");
        assert_eq!(ticks(&*time, root), held, "the device's time advanced while a participant was busy");
        let _ = release.send(());
        holder.join().unwrap();
        assert!(time.cancel(far));
        let _ = waiter.join();
    }

    #[test]
    fn cpu_a_stopped_clock_with_a_near_root_entry_does_not_poll() {
        // A root entry 1 ms ahead while a participant holds the clock stopped for 400 ms: the
        // waiting `next_wakeup` blocks, then wakes once when the clock runs on and once at the
        // entry. A root remainder read on the stopped clock and waited for in real time polls
        // instead, hundreds of times. The measure is the waiter's own passes through its
        // clock waits (`host::passes`): std has no CPU clock, and process CPU time would count
        // the other tests of this binary.
        let (authority, device, _clocks, root) = authority(ClockMode::Manual, "internal");
        let authority = Arc::new(authority);
        let time = authority.time();
        let busy = hold(&device, Duration::from_millis(400));
        let at = TimePoint::new(root, ticks(&*time, root) + ms(1));
        time.schedule(at, Box::new(|_| {})).unwrap();
        let (tx, rx) = channel();
        let waiter = { let a = authority.clone(); std::thread::spawn(move || { let woke = a.next_wakeup(); let _ = tx.send((woke, passes())); }) };
        let (woke, passes) = bounded(&rx, "the root entry after the busy interval");
        assert_eq!(woke, Some(at));
        assert!(passes <= 8, "a next_wakeup on a stopped clock passed {passes} times through its clock waits");
        let _ = waiter.join();
        busy.join().unwrap();
    }

    #[test]
    fn ur_07_the_root_fires_first_when_both_are_due() {
        // One check with both timelines due: the root first (plan 30 §4.6). The HOST callback
        // is registered first, so insertion order alone would not give the root first.
        let (authority, _device, _clocks, root) = authority(ClockMode::Manual, "internal");
        let time = authority.time();
        let host_at = TimePoint::new(host(), ticks(&*time, host()));
        let root_at = TimePoint::new(root, ticks(&*time, root));
        time.schedule(host_at, Box::new(|_| {})).unwrap();
        time.schedule(root_at, Box::new(|_| {})).unwrap();
        let authority = Arc::new(authority);
        let first = { let a = authority.clone(); within("the first due instant", move || a.next_wakeup()) };
        assert_eq!(first, Some(root_at), "the root timeline fires first");
        let second = { let a = authority.clone(); within("the second due instant", move || a.next_wakeup()) };
        assert_eq!(second, Some(host_at));
    }

    #[test]
    fn ur_07_in_past_is_judged_per_root_timeline() {
        // KG-9 as plan 30 §4.6 amends it: the device root and a domain derived from it share
        // one timeline and `last_fired`; `host.monotonic` is another, so it is never refused
        // for an instant before a root one that fired.
        let (authority, _device, clocks, root) = authority(ClockMode::Manual, "internal");
        let time = authority.time();
        std::thread::sleep(Duration::from_millis(10));
        let fired = TimePoint::new(root, ticks(&*time, root));
        time.schedule(fired, Box::new(|_| {})).unwrap();
        let authority = Arc::new(authority);
        let woke = { let a = authority.clone(); within("the root instant", move || a.next_wakeup()) };
        assert_eq!(woke, Some(fired));
        // A derived domain shares the root's axis: sample 0 is root tick 0, before `fired`.
        let handle = clocks.declare_sample_clock(ResourceId::parse("usrp/rx").unwrap(), root, Rational::new(200, 1).unwrap()).unwrap();
        let derived = clocks.register_sample_clock(&handle, 0).unwrap();
        assert!(matches!(
            time.schedule(TimePoint::new(derived, 0), Box::new(|_| {})),
            Err(TimeError::InPast { .. })
        ), "a derived domain shares the root's timeline");
        // The HOST timeline is separate: an instant before the fired root one is accepted.
        time.schedule(TimePoint::new(host(), 0), Box::new(|_| {})).expect("host.monotonic is another timeline");
    }

    #[test]
    fn ur_07_now_does_not_follow_an_anchor_that_lands_behind_it() {
        // UR-7's monotonic guard (row U06), on the Authority's clock directly. The Authority is
        // dropped first, which ends uhd-clock (UR-7's D-1) — confirmed by the clock's weak count:
        // the thread's `Weak` is gone with it — so no re-anchor can overwrite the anchor set
        // below; a busy participant then holds the manual clock still, so no time passes
        // between that anchor and the extrapolation checked.
        let (authority, device, _clocks, _root) = authority(ClockMode::Manual, "internal");
        let time = authority.time.clone();
        let returned = time.extrapolate();
        drop(authority);
        assert_eq!(Arc::weak_count(&time), 0, "uhd-clock still runs after the Authority's drop");
        let (release, holder) = hold_until_released(&device);
        *lock(&time.anchor) = Anchor { host: time.host.now(), real: Instant::now(), device: returned - ms(1_000) };
        assert!(time.extrapolate() >= returned, "now went back with an anchor behind it");
        let _ = release.send(());
        holder.join().unwrap();
    }

    #[test]
    fn ur_07_the_host_queue_keeps_its_order_across_a_reanchor() {
        // Two HOST instants registered out of order, with `uhd-clock`'s re-anchors running
        // between the naps (plan 30 §7): the earlier fires first and each returns its own
        // HOST instant, that timeline being independent of the device's anchor.
        let (authority, device, _clocks, _root) = authority(ClockMode::Manual, "internal");
        let time = authority.time();
        let later = TimePoint::new(host(), ticks(&*time, host()) + 200_000_000);
        let earlier = TimePoint::new(host(), ticks(&*time, host()) + 60_000_000);
        time.schedule(later, Box::new(|_| {})).unwrap();
        time.schedule(earlier, Box::new(|_| {})).unwrap();
        let authority = Arc::new(authority);
        let first = { let a = authority.clone(); within("the earlier HOST instant", move || a.next_wakeup()) };
        assert_eq!(first, Some(earlier), "the earlier HOST instant fires first");
        // A re-anchor of uhd-clock happens while the second waits.
        device.host().sleep(Duration::from_millis(140));
        let second = { let a = authority.clone(); within("the later HOST instant", move || a.next_wakeup()) };
        assert_eq!(second, Some(later));
    }

    #[test]
    fn ur_07_the_host_timeline_is_judged_on_real_time_on_the_real_clock() {
        // The same rules in the other mode (plan 30 §7): the real clock's HOST wait and
        // schedule take real host time and return the HOST instant, and the root fires first.
        let (authority, _device, _clocks, root) = authority(ClockMode::Real, "internal");
        let time = authority.time();
        let begun = Instant::now();
        {
            let t = time.clone();
            within("the real clock's HOST wait", move || t.wait_until(TimePoint::new(host(), ticks(&*t, host()) + 40_000_000))).unwrap();
        }
        assert!(begun.elapsed() >= Duration::from_millis(30), "the HOST wait returned early: {:?}", begun.elapsed());
        let at = TimePoint::new(host(), ticks(&*time, host()) + 40_000_000);
        let fired = Arc::new(Mutex::new(Vec::new()));
        let log = fired.clone();
        time.schedule(at, Box::new(move |t| log.lock().unwrap().push(t))).unwrap();
        let authority = Arc::new(authority);
        let begun = Instant::now();
        let woke = { let a = authority.clone(); within("the HOST schedule", move || a.next_wakeup()) };
        assert_eq!(woke, Some(at), "the HOST schedule fires and returns its instant");
        assert!(begun.elapsed() >= Duration::from_millis(30), "the HOST schedule fired early: {:?}", begun.elapsed());
        assert_eq!(*fired.lock().unwrap(), vec![at]);
        let root_at = TimePoint::new(root, ticks(&*time, root));
        let host_at = TimePoint::new(host(), ticks(&*time, host()));
        time.schedule(root_at, Box::new(|_| {})).unwrap();
        time.schedule(host_at, Box::new(|_| {})).unwrap();
        let first = { let a = authority.clone(); within("the root instant", move || a.next_wakeup()) };
        assert_eq!(first, Some(root_at), "the root timeline fires first");
        let second = { let a = authority.clone(); within("the HOST instant", move || a.next_wakeup()) };
        assert_eq!(second, Some(host_at));
    }

    #[test]
    fn ur_08_the_manual_clock_s_relations_hold_only_at_the_instant_measured() {
        for (mode, limited) in [(ClockMode::Manual, true), (ClockMode::Real, false)] {
            let (authority, _device, _clocks, root) = authority(mode, "internal");
            let relations = authority.relations();
            assert_eq!(relations.len(), 2, "{mode:?}");
            for relation in &relations {
                assert_eq!(relation.source(), root, "{mode:?}");
                let valid = relation.valid();
                assert_eq!(valid.from, relation.measured_at(), "{mode:?}");
                if limited {
                    assert_eq!(valid.to, Some(valid.from), "{mode:?}: the manual clock's relation holds at the instant measured only");
                } else {
                    assert_eq!(valid.to, None, "{mode:?}: the real clock's relation is unbounded");
                }
            }
        }
    }

    #[test]
    fn ur_08_the_host_domain_follows_real_time_across_a_busy_interval() {
        // `host.monotonic` is real time (TM-16a1) while a participant holds the device's clock,
        // released only after the checks: the HOST domain moves on through a 150 ms real sleep
        // (a lower bound no host load can break), the root stands still.
        let (authority, device, _clocks, root) = authority(ClockMode::Manual, "internal");
        let time = authority.time();
        let (release, holder) = hold_until_released(&device);
        let host_before = ticks(&*time, host());
        let root_before = ticks(&*time, root);
        std::thread::sleep(Duration::from_millis(150));
        let host_after = ticks(&*time, host());
        let root_after = ticks(&*time, root);
        assert!(host_after - host_before >= 100_000_000, "the HOST domain did not follow real time: {} ns", host_after - host_before);
        assert_eq!(root_after, root_before, "the device's time advanced while a participant was busy");
        let _ = release.send(());
        holder.join().unwrap();
    }
}
