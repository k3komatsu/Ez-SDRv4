//! Ez-SDR v4 Module ezsdr.sim-engine 1.0.0 (plan/phase2/08-simulation.md).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId};
use ezsdr_kernel::module_api::{
    Authority, AuthorityDescriptor, Deployment, KERNEL_API, ModuleDescriptor, ModuleError,
    ModuleRef, Pacing, Role, Version, VersionReq, VocabularyRequirement,
};
use ezsdr_kernel::spec::Namespace;
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, Converted, EpochRef, Rational, ScheduleHandle, TimeAuthority,
    TimeError, TimePoint,
};

use ezsdr_sim::{VIRTUAL_EPOCH, VIRTUAL_TICK_RATE_HZ};

/// Maximum callbacks fired by one `next_wakeup` call (SE-11).
pub const CALLBACK_CAP: usize = 1000;

type Callback = Box<dyn FnOnce(TimePoint) + Send>;

struct State {
    now: i64,
    pending: BTreeMap<(i64, u64), Callback>,
    next_seq: u64,
}

struct EngineTime {
    clocks: Arc<ClockRegistry>,
    root: ClockDomainId,
    state: Mutex<State>,
    woken: Condvar,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl EngineTime {
    fn root_tick(&self, point: TimePoint, round_up: bool) -> Result<i64, TimeError> {
        if !self.governs(point.domain()) {
            return Err(TimeError::NotGoverned { id: point.domain() });
        }
        // host.monotonic is driven as the root's own ticks (TM-16a1).
        if let Ok(ticks) = point
            .ticks_in(self.root)
            .or_else(|_| point.ticks_in(ClockDomainId::HOST_MONOTONIC))
        {
            return Ok(ticks);
        }
        match self.clocks.convert(point, self.root)? {
            Converted::Exact { point } => point.ticks_in(self.root),
            Converted::Inexact { floor, .. } if round_up => {
                floor.ticks_in(self.root)?.checked_add(1).ok_or(TimeError::Overflow)
            }
            Converted::Inexact { floor, .. } => Err(TimeError::Inexact { floor }),
        }
    }
}

impl TimeAuthority for EngineTime {
    fn primary_root(&self) -> ClockDomainId {
        self.root
    }

    fn pacing(&self) -> Pacing {
        Pacing::FreeRunning
    }

    fn governs(&self, domain: ClockDomainId) -> bool {
        domain == self.root
            || domain == ClockDomainId::HOST_MONOTONIC
            || self
                .clocks
                .get(domain)
                .is_ok_and(|clock| clock.root_id() == self.root)
    }

    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError> {
        if !self.governs(domain) {
            return Err(TimeError::NotGoverned { id: domain });
        }
        let ticks = lock(&self.state).now;
        if domain == self.root || domain == ClockDomainId::HOST_MONOTONIC {
            return Ok(TimePoint::new(domain, ticks));
        }
        Ok(self
            .clocks
            .convert(TimePoint::new(self.root, ticks), domain)?
            .floor())
    }

    fn wait_until(&self, point: TimePoint) -> Result<(), TimeError> {
        let target = self.root_tick(point, true)?;
        let mut state = lock(&self.state);
        while state.now < target {
            state = self.woken.wait(state).unwrap_or_else(|error| error.into_inner());
        }
        Ok(())
    }

    fn schedule(
        &self,
        point: TimePoint,
        callback: Callback,
    ) -> Result<ScheduleHandle, TimeError> {
        let tick = self.root_tick(point, false)?;
        let mut state = lock(&self.state);
        if tick < state.now {
            return Err(TimeError::InPast {
                now: TimePoint::new(self.root, state.now),
                requested: point,
            });
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        state.pending.insert((tick, seq), callback);
        Ok(ScheduleHandle {
            root: self.root,
            ticks: tick,
            seq,
        })
    }

    fn cancel(&self, handle: ScheduleHandle) -> bool {
        if handle.root != self.root {
            return false;
        }
        lock(&self.state)
            .pending
            .remove(&(handle.ticks, handle.seq))
            .is_some()
    }
}

/// Discrete-event Simulation Engine and its declared Authority state (SE-9).
pub struct SimEngine {
    time: Arc<EngineTime>,
    descriptor: AuthorityDescriptor,
}

fn module_ref() -> ModuleRef {
    ModuleRef {
        id: ModuleId::parse("ezsdr.sim-engine").expect("the Simulation Engine id is valid"),
        version: Version::new(1, 0, 0),
    }
}

/// Describes and registers `ezsdr.sim-engine` 1.0.0 (SE-8).
pub fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: module_ref().id,
        version: Version::new(1, 0, 0),
        kernel_api: KERNEL_API,
        roles: vec![Role::Authority],
        vocabularies: vec![VocabularyRequirement {
            id: Namespace::parse("sim").expect("a valid Vocabulary namespace"),
            req: VersionReq(Version::new(1, 0, 0)),
        }],
        deployment: Deployment::InProcess {},
        impl_hash: Some(ContentHash::of_bytes(b"ezsdr.sim-engine 1.0.0")),
    }
}

impl SimEngine {
    /// Creates an Engine and registers its 1 GHz virtual root (SE-9).
    pub fn new(clocks: Arc<ClockRegistry>) -> Result<SimEngine, TimeError> {
        let root = clocks.allocate_id()?;
        let tick_rate = Rational::new(VIRTUAL_TICK_RATE_HZ, 1)?;
        clocks.register(ClockDomain::root(
            root,
            tick_rate,
            EpochRef::Arbitrary {
                set_by: VIRTUAL_EPOCH.to_owned(),
            },
        ))?;
        let descriptor = AuthorityDescriptor {
            module: module_ref(),
            governs: vec![root, ClockDomainId::HOST_MONOTONIC],
            pacing: Pacing::FreeRunning,
        };
        Ok(SimEngine {
            time: Arc::new(EngineTime {
                clocks,
                root,
                state: Mutex::new(State {
                    now: 0,
                    pending: BTreeMap::new(),
                    next_seq: 0,
                }),
                woken: Condvar::new(),
            }),
            descriptor,
        })
    }

    /// Creates an Engine for its exact empty-selector, profile-free binding (SE-9).
    pub fn from_binding(
        binding: &Binding,
        clocks: Arc<ClockRegistry>,
    ) -> Result<SimEngine, ModuleError> {
        if binding.module != module_ref() {
            return Err(ModuleError::rejected("SE-9: binding names another Module"));
        }
        if !binding.selector.is_empty() {
            return Err(ModuleError::rejected("SE-9: selector must be empty"));
        }
        if binding.profile.is_some() {
            return Err(ModuleError::rejected("SE-9: profile must be absent"));
        }
        SimEngine::new(clocks).map_err(|error| ModuleError::rejected(format!("SE-9: {error}")))
    }

    /// The Engine's virtual root domain (SE-9).
    pub fn root(&self) -> ClockDomainId {
        self.time.root
    }
}

impl Authority for SimEngine {
    fn descriptor(&self) -> &AuthorityDescriptor {
        &self.descriptor
    }

    fn time(&self) -> Arc<dyn TimeAuthority> {
        self.time.clone()
    }

    fn next_wakeup(&self) -> Option<TimePoint> {
        let tick = {
            let mut state = lock(&self.time.state);
            let (&(tick, _), _) = state.pending.iter().next()?;
            state.now = tick;
            tick
        };
        for _ in 0..CALLBACK_CAP {
            let callback = {
                let mut state = lock(&self.time.state);
                match state.pending.keys().next().copied() {
                    Some(key) if key.0 == tick => state.pending.remove(&key),
                    _ => None,
                }
            };
            let Some(callback) = callback else { break };
            callback(TimePoint::new(self.time.root, tick));
        }
        self.time.woken.notify_all();
        Some(TimePoint::new(self.time.root, tick))
    }
}
