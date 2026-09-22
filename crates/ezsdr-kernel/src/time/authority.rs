//! The Time Authority: who answers "now" and "wait until" (`01-time-model.md` TM-16, TM-17).

use super::{ClockRegistry, TimeError, TimePoint};
use crate::id::ClockDomainId;
use crate::module_api::Pacing;

/// A pending scheduled callback, as returned by
/// [`TimeAuthority::schedule`] and consumed by [`TimeAuthority::cancel`] (TM-16c).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ScheduleHandle {
    /// The root domain the callback is queued against.
    pub root: ClockDomainId,
    /// The callback's fire time, in root ticks.
    pub ticks: i64,
    /// Insertion ordinal, which breaks ties at one instant (TM-16c).
    pub seq: u64,
}

/// A Run's single source of "now" and "wait until".
///
/// One Authority per Run. It **declares** the set of domains it governs, which must
/// include one primary `Root`, every `Derived` domain of that root, and
/// `host.monotonic`; a Simulation Authority declares every root it simulates.
/// Governing a domain and being exactly related to it are different properties:
/// `NotGoverned` means no timekeeper in this Run advances that domain, not that the
/// domain is unrelated to the primary root.
///
/// The trait is synchronous: an async one would pull a runtime into the Kernel and
/// force every Provider to be async (decision T8).
///
/// Rule: TM-16a, TM-16b, TM-16c, TM-16d.
pub trait TimeAuthority: Send + Sync {
    /// The primary `Root` this Authority advances (TM-16a).
    fn primary_root(&self) -> ClockDomainId;

    /// How this Authority paces its primary root; MA-41 cross-checks it against the
    /// derived ExecutionClass (TM-16a1, MA-29).
    fn pacing(&self) -> Pacing;

    /// True when some timekeeper in this Run advances `domain` (TM-16b).
    fn governs(&self, domain: ClockDomainId) -> bool;

    /// The current instant in `domain`. For a `Derived` domain this is the floor
    /// conversion of its root's current time (TM-16c, decision T12).
    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError>;

    /// Blocks until `now` reaches `t`. Fails with `NotGoverned` when `t` is in a
    /// domain no timekeeper advances. It blocks, so it belongs to thread-driven
    /// components and must not be called by a step-driven one (TM-16d).
    fn wait_until(&self, t: TimePoint) -> Result<(), TimeError>;

    /// Queues `f` to run at `t`. Fails with `InPast` when `t` precedes `now`.
    /// Callbacks fire in ascending time, ties in insertion order, and observe
    /// `now()` equal to their fire time while running (TM-16c).
    fn schedule(
        &self,
        t: TimePoint,
        f: Box<dyn FnOnce(TimePoint) + Send>,
    ) -> Result<ScheduleHandle, TimeError>;

    /// Removes a pending callback, reporting whether it was still pending (TM-16c).
    fn cancel(&self, h: ScheduleHandle) -> bool;
}

/// Resolves a `TimePoint` in any domain to `(root, root ticks)` (TM-16c).
#[cfg_attr(not(feature = "testing"), allow(dead_code))]
pub(crate) fn to_root(
    registry: &ClockRegistry,
    t: TimePoint,
) -> Result<(ClockDomainId, i64), TimeError> {
    let root = registry.get(t.domain)?.root_id();
    if root == t.domain {
        return Ok((root, t.ticks));
    }
    // `try_exact`, not a floor: TM-16c requires a callback **at** `t`, and a callback
    // fired at the floor observes `now()` earlier than `t` — and two distinct derived
    // instants collapse onto one root tick. A reading may round (`now` still floors);
    // a firing may not. A caller that wants one converts and rounds itself, rather
    // than having the Kernel pick a rounding on its behalf (§65 #39).
    Ok((root, registry.conversion(t.domain, root)?.try_exact(t)?.ticks))
}

#[cfg(feature = "testing")]
pub use manual::ManualTimeAuthority;

/// `ManualTimeAuthority` is normative (TM-17a) and Phase 2's Simulation Engine
/// builds on it, which a sibling crate's `tests/support/` cannot provide, so it
/// ships in `src/` behind the non-default `testing` feature (OV-20).
#[cfg(feature = "testing")]
mod manual {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Condvar, Mutex};

    use super::*;
    use crate::time::Duration;

    type Callback = Box<dyn FnOnce(TimePoint) + Send>;

    #[derive(Default)]
    struct RootState {
        now: i64,
        pending: BTreeMap<(i64, u64), Callback>,
    }

    struct State {
        roots: BTreeMap<ClockDomainId, RootState>,
        next_seq: u64,
    }

    /// A Time Authority whose clocks advance only on [`Self::advance_to`]: the seed of
    /// the Simulation Engine's `step(until)` semantics (audit F18). The Engine itself
    /// is Phase 2.
    ///
    /// Rule: TM-17a, TM-17b.
    pub struct ManualTimeAuthority {
        registry: Arc<ClockRegistry>,
        primary: ClockDomainId,
        pacing: Pacing,
        state: Mutex<State>,
        woken: Condvar,
        host_base: std::time::Instant,
        callback_cap: usize,
    }

    impl ManualTimeAuthority {
        /// Default cap on callbacks firing at one instant. A model that reschedules
        /// itself at its own fire time otherwise makes `advance_to` spin forever,
        /// which hangs the test suite instead of failing it (TM-17b).
        pub const DEFAULT_CALLBACK_CAP: usize = 1_000;

        /// Builds an Authority over `primary` plus `extra_roots`. `host.monotonic` is
        /// always governed (TM-16a); it is **driven** as virtual time only when
        /// `pacing` is [`Pacing::FreeRunning`], which MA-41 makes exactly the
        /// Simulation class (TM-16a1).
        ///
        /// Rule: TM-16a, TM-16a1, TM-17a.
        pub fn new(
            registry: Arc<ClockRegistry>,
            primary: ClockDomainId,
            extra_roots: &[ClockDomainId],
            pacing: Pacing,
        ) -> Result<ManualTimeAuthority, TimeError> {
            let mut roots = BTreeMap::new();
            for id in std::iter::once(&primary).chain(extra_roots) {
                let domain = registry.get(*id)?;
                if domain.root_id() != *id {
                    return Err(TimeError::Unrelated { a: *id, b: domain.root_id() });
                }
                roots.insert(*id, RootState::default());
            }
            // TM-16a: `host.monotonic` is always governed, so `governs`, `now`,
            // `wait_until` and `schedule` must all agree about it. TM-16a1 decides
            // only who *drives* it: this Authority outside the Simulation class
            // reads the real clock instead of advancing it (see `root_now`).
            roots.entry(ClockDomainId::HOST_MONOTONIC).or_default();
            Ok(ManualTimeAuthority {
                registry,
                primary,
                pacing,
                state: Mutex::new(State { roots, next_seq: 0 }),
                woken: Condvar::new(),
                host_base: std::time::Instant::now(),
                callback_cap: Self::DEFAULT_CALLBACK_CAP,
            })
        }

        /// Replaces [`Self::DEFAULT_CALLBACK_CAP`] for this Authority (TM-17b).
        pub fn with_callback_cap(mut self, cap: usize) -> ManualTimeAuthority {
            self.callback_cap = cap;
            self
        }

        /// The registry this Authority resolves domains against (TM-16c).
        pub fn registry(&self) -> &Arc<ClockRegistry> {
            &self.registry
        }

        /// Advances to `t`, firing every due callback — including callbacks scheduled
        /// by callbacks during the same advance — in TM-16c order, then setting `now`
        /// and waking every `wait_until` waiter.
        ///
        /// Fails with `InPast` when `t` precedes `now`, and with `LimitExceeded` after
        /// [`Self::DEFAULT_CALLBACK_CAP`] callbacks fire at one instant (TM-17b).
        /// Returns how many callbacks fired.
        ///
        /// Rule: TM-17a, TM-17b.
        pub fn advance_to(&self, t: TimePoint) -> Result<usize, TimeError> {
            let (root, target) = to_root(&self.registry, t)?;
            let mut fired = self.advance_root(root, target)?;
            // TM-16a1: in the Simulation class the Authority drives host.monotonic
            // itself, keeping it in step with the primary root.
            if self.pacing == Pacing::FreeRunning
                && root == self.primary
                && root != ClockDomainId::HOST_MONOTONIC
            {
                let host = self
                    .registry
                    .rescale(Duration::new(root, target), ClockDomainId::HOST_MONOTONIC)?
                    .floor();
                // Clamped: `advance_to` on `host.monotonic` itself may already have
                // moved it past this, and syncing backwards would fail with `InPast`
                // naming a domain the caller never asked about.
                let at_least = self.root_now(ClockDomainId::HOST_MONOTONIC)?;
                fired += self
                    .advance_root(ClockDomainId::HOST_MONOTONIC, host.ticks.max(at_least))?;
            }
            Ok(fired)
        }

        fn advance_root(&self, root: ClockDomainId, target: i64) -> Result<usize, TimeError> {
            {
                // Through `root_now`, so that the past check and `now()` agree about
                // `host.monotonic` outside the Simulation class (TM-16c).
                let now = self.root_now(root)?;
                if target < now {
                    return Err(TimeError::InPast {
                        now: TimePoint::new(root, now),
                        requested: TimePoint::new(root, target),
                    });
                }
            }
            let mut at_instant = 0usize;
            let mut fired = 0usize;
            let mut last = None;
            loop {
                // Pop under the lock and run outside it: a callback may call now(),
                // schedule() or cancel() (TM-17a).
                let next = {
                    let mut state = self.lock();
                    let rs = state.roots.get_mut(&root).expect("checked above");
                    match rs.pending.keys().next().copied() {
                        Some(key) if key.0 <= target => {
                            let cb = rs.pending.remove(&key).expect("key came from the map");
                            rs.now = key.0;
                            Some((key.0, cb))
                        }
                        _ => {
                            // TM-17a: a callback may call `advance_to` re-entrantly,
                            // so this must never move `now` backwards.
                            rs.now = rs.now.max(target);
                            None
                        }
                    }
                };
                let Some((ticks, cb)) = next else { break };
                if last == Some(ticks) {
                    at_instant += 1;
                } else {
                    at_instant = 1;
                    last = Some(ticks);
                }
                if at_instant > self.callback_cap {
                    return Err(TimeError::LimitExceeded);
                }
                fired += 1;
                cb(TimePoint::new(root, ticks));
                self.woken.notify_all();
            }
            self.woken.notify_all();
            Ok(fired)
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(|e| e.into_inner())
        }

        fn root_now(&self, root: ClockDomainId) -> Result<i64, TimeError> {
            let queued =
                self.lock().roots.get(&root).map(|rs| rs.now).ok_or(TimeError::NotGoverned { id: root })?;
            if root == ClockDomainId::HOST_MONOTONIC && self.pacing != Pacing::FreeRunning {
                // TM-16a1: outside the Simulation class host.monotonic is the real
                // host clock, which this Authority reads rather than drives. It is
                // still governed (TM-16a), so a caller may schedule on it; the
                // clock never runs backwards below an instant already reached.
                //
                // ponytail: this double fires a host callback only when something
                // drives the Authority. A real hardware Authority fires them from
                // its own timer thread; Phase 1 has no such thread and no test
                // needs one.
                let real = i64::try_from(self.host_base.elapsed().as_nanos())
                    .map_err(|_| TimeError::Overflow)?;
                return Ok(real.max(queued));
            }
            Ok(queued)
        }
    }

    impl TimeAuthority for ManualTimeAuthority {
        fn primary_root(&self) -> ClockDomainId {
            self.primary
        }

        fn pacing(&self) -> Pacing {
            self.pacing
        }

        fn governs(&self, domain: ClockDomainId) -> bool {
            if domain == ClockDomainId::HOST_MONOTONIC {
                return true; // TM-16a: always governed.
            }
            match self.registry.get(domain) {
                Ok(d) => self.lock().roots.contains_key(&d.root_id()),
                Err(_) => false,
            }
        }

        fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError> {
            if !self.governs(domain) {
                return Err(TimeError::NotGoverned { id: domain });
            }
            let root = self.registry.get(domain)?.root_id();
            let ticks = self.root_now(root)?;
            if root == domain {
                return Ok(TimePoint::new(domain, ticks));
            }
            // TM-16c, decision T12: a derived domain's now is the floor conversion.
            Ok(self.registry.convert(TimePoint::new(root, ticks), domain)?.floor())
        }

        fn wait_until(&self, t: TimePoint) -> Result<(), TimeError> {
            if !self.governs(t.domain) {
                return Err(TimeError::NotGoverned { id: t.domain });
            }
            let (root, target) = to_root(&self.registry, t)?;
            if root == ClockDomainId::HOST_MONOTONIC && self.pacing != Pacing::FreeRunning {
                while self.root_now(root)? < target {
                    std::thread::yield_now();
                }
                return Ok(());
            }
            let mut state = self.lock();
            loop {
                let now = state.roots.get(&root).ok_or(TimeError::NotGoverned { id: root })?.now;
                if now >= target {
                    return Ok(());
                }
                state = self.woken.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        }

        fn schedule(
            &self,
            t: TimePoint,
            f: Box<dyn FnOnce(TimePoint) + Send>,
        ) -> Result<ScheduleHandle, TimeError> {
            if !self.governs(t.domain) {
                return Err(TimeError::NotGoverned { id: t.domain });
            }
            let (root, ticks) = to_root(&self.registry, t)?;
            let now = self.root_now(root)?;
            if ticks < now {
                return Err(TimeError::InPast {
                    now: TimePoint::new(root, now),
                    requested: TimePoint::new(root, ticks),
                });
            }
            let mut state = self.lock();
            let seq = state.next_seq;
            let rs = state.roots.get_mut(&root).ok_or(TimeError::NotGoverned { id: root })?;
            rs.pending.insert((ticks, seq), f);
            state.next_seq += 1;
            Ok(ScheduleHandle { root, ticks, seq })
        }

        fn cancel(&self, h: ScheduleHandle) -> bool {
            let mut state = self.lock();
            state
                .roots
                .get_mut(&h.root)
                .is_some_and(|rs| rs.pending.remove(&(h.ticks, h.seq)).is_some())
        }
    }
}
