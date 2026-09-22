//! Clock domains and the registry that allocates and relates them
//! (`01-time-model.md` TM-3, TM-9…TM-13, TM-21).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::{Duration, ExactConversion, Rational, Rescaled, TimeError, TimePoint};
use crate::id::{ClockDomainId, ResourceId};

/// Largest numerator or denominator a `Root` tick rate or a `root_ticks_per_tick`
/// may carry. Without it a rate of 2^40 / (2^40 − 1) is legal under TM-2 and
/// TM-21's cross-product reaches 2^190 (TM-3).
pub const RATIO_TERM_CAP: u64 = 1 << 31;

/// What a domain's tick zero is anchored to (TM-3).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EpochRef {
    /// Tick zero is 1970-01-01T00:00:00Z.
    Utc1970,
    /// Tick zero is whatever set it; the string names the mechanism, for example
    /// `uhd.set_time_unknown_pps` or `sim.run_start` (Vision §15, §50; OV-23a: a
    /// named example, not a Kernel dependency).
    Arbitrary {
        /// Namespaced name of whatever established the epoch.
        set_by: String,
    },
}

/// A domain is either a timekeeper of its own or an exact division of one (TM-3).
///
/// A `Derived` domain names a `Root` directly; there are no chains (decision T2).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClockDomainKind {
    /// A timekeeper: a tick rate and what tick zero means.
    Root {
        /// Ticks per second; neither term may exceed [`RATIO_TERM_CAP`] (TM-3).
        tick_rate: Rational,
        /// What tick zero is anchored to.
        epoch: EpochRef,
    },
    /// An exact division of a root, such as a SampleClock (TM-10, TM-13a).
    Derived {
        /// The `Root` this divides; refused if unknown or itself `Derived` (TM-3).
        root: ClockDomainId,
        /// Root ticks per one tick here; neither term may exceed [`RATIO_TERM_CAP`] (TM-3).
        root_ticks_per_tick: Rational,
        /// This domain's tick zero, expressed in root ticks (decision T2).
        origin: i64,
    },
}

/// A registered clock domain. Immutable once registered, except that `ended_at`
/// may be set once (TM-12).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClockDomain {
    /// Node-qualified identity (TM-11).
    pub id: ClockDomainId,
    /// Root or Derived, with its rate or ratio (TM-3).
    pub kind: ClockDomainKind,
    /// When the domain stopped advancing, in root ticks; set once, by a rate change (TM-13c).
    pub ended_at: Option<TimePoint>,
}

impl ClockDomain {
    /// A root timekeeper (TM-3).
    pub fn root(id: ClockDomainId, tick_rate: Rational, epoch: EpochRef) -> ClockDomain {
        ClockDomain { id, kind: ClockDomainKind::Root { tick_rate, epoch }, ended_at: None }
    }

    /// An exact division of a root (TM-3).
    pub fn derived(
        id: ClockDomainId,
        root: ClockDomainId,
        root_ticks_per_tick: Rational,
        origin: i64,
    ) -> ClockDomain {
        ClockDomain {
            id,
            kind: ClockDomainKind::Derived { root, root_ticks_per_tick, origin },
            ended_at: None,
        }
    }

    /// The root of this domain; a `Root` is its own root (TM-4).
    pub fn root_id(&self) -> ClockDomainId {
        match &self.kind {
            ClockDomainKind::Root { .. } => self.id,
            ClockDomainKind::Derived { root, .. } => *root,
        }
    }

    /// `(n, d, origin)`: the root ratio and origin. A `Root` is `(1, 1, 0)` (TM-4).
    fn terms(&self) -> (u64, u64, i64) {
        match &self.kind {
            ClockDomainKind::Root { .. } => (1, 1, 0),
            ClockDomainKind::Derived { root_ticks_per_tick, origin, .. } => {
                (root_ticks_per_tick.num(), root_ticks_per_tick.den(), *origin)
            }
        }
    }
}

/// One SampleClock as the Manifest records it, one per clock, in order, per stream (TM-13d).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SampleClockRecord {
    /// The stream that owns the clock (TM-13a).
    pub stream: ResourceId,
    /// The SampleClock domain's id (TM-13a).
    pub domain: ClockDomainId,
    /// The root the clock divides (TM-3).
    pub root: ClockDomainId,
    /// Root ticks per sample (TM-13a).
    pub root_ticks_per_tick: Rational,
    /// The root tick of sample zero (TM-13b, TM-13e).
    pub origin: TimePoint,
    /// The root tick at which the clock stopped, on a rate change (TM-13c).
    pub ended_at: Option<TimePoint>,
    /// Samples per second, derived by TM-10.
    pub nominal_rate: Rational,
}

/// A SampleClock whose id and ratio are fixed but whose origin is not yet known.
///
/// TM-13a allocates both at `prepare`, from the effective post-coercion rate;
/// TM-13b and TM-13e fix the origin later, at the first sample or at `arm`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SampleClockHandle {
    /// The allocated domain id (TM-13a).
    pub id: ClockDomainId,
    /// The root this clock will divide (TM-13a).
    pub root: ClockDomainId,
    /// Root ticks per sample (TM-13a).
    pub root_ticks_per_tick: Rational,
    /// The stream that owns the clock (TM-13a).
    pub stream: ResourceId,
}

struct Inner {
    domains: BTreeMap<ClockDomainId, ClockDomain>,
    records: Vec<SampleClockRecord>,
    next_local: u32,
}

/// The node's registry of clock domains: it allocates ids, enforces TM-3's caps,
/// and is the only place an exact conversion can be built (TM-11, TM-12).
pub struct ClockRegistry {
    inner: RwLock<Inner>,
}

impl Default for ClockRegistry {
    fn default() -> Self {
        ClockRegistry::new()
    }
}

impl ClockRegistry {
    /// A registry holding only the reserved domains: `utc` and `host.monotonic`,
    /// both `Root` at 1 GHz, with epochs `Utc1970` and `Arbitrary` (TM-11).
    pub fn new() -> ClockRegistry {
        let ghz = Rational::new(1_000_000_000, 1).expect("1 GHz is a valid rational");
        let mut domains = BTreeMap::new();
        domains.insert(
            ClockDomainId::UTC,
            ClockDomain::root(ClockDomainId::UTC, ghz, EpochRef::Utc1970),
        );
        domains.insert(
            ClockDomainId::HOST_MONOTONIC,
            ClockDomain::root(
                ClockDomainId::HOST_MONOTONIC,
                ghz,
                EpochRef::Arbitrary { set_by: "host.monotonic".to_owned() },
            ),
        );
        ClockRegistry {
            inner: RwLock::new(Inner {
                domains,
                records: Vec::new(),
                next_local: ClockDomainId::FIRST_ALLOCATABLE,
            }),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Allocates a fresh, strictly increasing, never-reused local id (TM-11).
    ///
    /// ponytail: saturating rather than fallible. Reaching `u32::MAX` needs 2^32
    /// allocations in one Run — each one a SampleClock, so each one a rate change —
    /// and `register` already refuses the only reachable way to jump there
    /// (TM-11, TM-2).
    pub fn allocate_id(&self) -> ClockDomainId {
        let mut inner = self.write();
        let local = inner.next_local;
        inner.next_local = inner.next_local.saturating_add(1);
        ClockDomainId::local(local)
    }

    /// Registers a domain. Fails with `DuplicateDomain` on a known id, `UnknownDomain`
    /// when a `Derived` domain's root is not registered, `Unrelated` when that root is
    /// itself `Derived`, and `LimitExceeded` when a ratio or rate term exceeds
    /// [`RATIO_TERM_CAP`] (TM-3, TM-12).
    pub fn register(&self, domain: ClockDomain) -> Result<(), TimeError> {
        if !domain.id.node.is_local() {
            return Err(TimeError::UnknownDomain { id: domain.id });
        }
        if domain.id.local == u32::MAX {
            // TM-11: ids are allocated by the registry, strictly increasing and
            // never reused. Accepting the top id would leave the registry unable to
            // allocate another without reusing one.
            return Err(TimeError::LimitExceeded);
        }
        let mut inner = self.write();
        if inner.domains.contains_key(&domain.id) {
            return Err(TimeError::DuplicateDomain { id: domain.id });
        }
        match &domain.kind {
            ClockDomainKind::Root { tick_rate, .. } => {
                if tick_rate.exceeds(RATIO_TERM_CAP) {
                    return Err(TimeError::LimitExceeded);
                }
            }
            ClockDomainKind::Derived { root, root_ticks_per_tick, .. } => {
                if root_ticks_per_tick.exceeds(RATIO_TERM_CAP) {
                    return Err(TimeError::LimitExceeded);
                }
                match inner.domains.get(root) {
                    None => return Err(TimeError::UnknownDomain { id: *root }),
                    Some(r) if !matches!(r.kind, ClockDomainKind::Root { .. }) => {
                        // TM-3: a Derived domain names a Root directly; no chains.
                        return Err(TimeError::Unrelated { a: domain.id, b: *root });
                    }
                    Some(_) => {}
                }
            }
        }
        if inner.next_local <= domain.id.local {
            // TM-11: ids are strictly increasing and never reused. A plain `+ 1`
            // panics in debug and wraps to the reserved id 0 in release when a
            // caller registers `u32::MAX`, which TM-2 forbids.
            inner.next_local = domain.id.local.saturating_add(1);
        }
        inner.domains.insert(domain.id, domain);
        Ok(())
    }

    /// The registered domain, or `UnknownDomain` (TM-12).
    pub fn get(&self, id: ClockDomainId) -> Result<ClockDomain, TimeError> {
        self.read().domains.get(&id).cloned().ok_or(TimeError::UnknownDomain { id })
    }

    /// True when the id is registered (TM-12).
    pub fn is_registered(&self, id: ClockDomainId) -> bool {
        self.read().domains.contains_key(&id)
    }

    /// Marks a domain as no longer advancing, in root ticks. Setting it twice fails
    /// with `Stopped`; a domain is otherwise immutable (TM-12, TM-13c).
    pub fn end(&self, id: ClockDomainId, at: TimePoint) -> Result<(), TimeError> {
        let mut inner = self.write();
        let root = inner.domains.get(&id).ok_or(TimeError::UnknownDomain { id })?.root_id();
        if at.domain != root {
            return Err(TimeError::DomainMismatch { expected: root, found: at.domain });
        }
        let domain = inner.domains.get_mut(&id).ok_or(TimeError::UnknownDomain { id })?;
        if domain.ended_at.is_some() {
            return Err(TimeError::Stopped);
        }
        domain.ended_at = Some(at);
        for record in inner.records.iter_mut().filter(|r| r.domain == id) {
            record.ended_at = Some(at);
        }
        Ok(())
    }

    /// Allocates a SampleClock's id and ratio from the effective post-coercion rate.
    /// The origin is not known yet: TM-13b fixes it at the first sample, TM-13e at `arm`.
    ///
    /// Rule: TM-13a.
    pub fn declare_sample_clock(
        &self,
        stream: ResourceId,
        root: ClockDomainId,
        root_ticks_per_tick: Rational,
    ) -> Result<SampleClockHandle, TimeError> {
        if root_ticks_per_tick.exceeds(RATIO_TERM_CAP) {
            return Err(TimeError::LimitExceeded);
        }
        match self.get(root)?.kind {
            ClockDomainKind::Root { .. } => {}
            ClockDomainKind::Derived { .. } => {
                return Err(TimeError::Unrelated { a: root, b: root });
            }
        }
        Ok(SampleClockHandle { id: self.allocate_id(), root, root_ticks_per_tick, stream })
    }

    /// Registers a declared SampleClock once its origin is known, and records it for
    /// the Manifest. `origin` is a root tick (TM-13b, TM-13d, TM-13e).
    pub fn register_sample_clock(
        &self,
        handle: &SampleClockHandle,
        origin: i64,
    ) -> Result<ClockDomainId, TimeError> {
        self.register(ClockDomain::derived(
            handle.id,
            handle.root,
            handle.root_ticks_per_tick,
            origin,
        ))?;
        let nominal_rate = self.nominal_rate(handle.id)?;
        self.write().records.push(SampleClockRecord {
            stream: handle.stream.clone(),
            domain: handle.id,
            root: handle.root,
            root_ticks_per_tick: handle.root_ticks_per_tick,
            origin: TimePoint::new(handle.root, origin),
            ended_at: None,
            nominal_rate,
        });
        Ok(handle.id)
    }

    /// Every SampleClock the registry has seen, in registration order (TM-13d).
    pub fn sample_clock_records(&self) -> Vec<SampleClockRecord> {
        self.read().records.clone()
    }

    /// Ticks per second. A `Derived` domain's rate is its root's divided by
    /// `root_ticks_per_tick` (TM-10).
    pub fn nominal_rate(&self, id: ClockDomainId) -> Result<Rational, TimeError> {
        let inner = self.read();
        let domain = inner.domains.get(&id).ok_or(TimeError::UnknownDomain { id })?;
        match &domain.kind {
            ClockDomainKind::Root { tick_rate, .. } => Ok(*tick_rate),
            ClockDomainKind::Derived { root, root_ticks_per_tick, .. } => {
                let root = inner.domains.get(root).ok_or(TimeError::UnknownDomain { id: *root })?;
                let ClockDomainKind::Root { tick_rate, .. } = root.kind else {
                    return Err(TimeError::Unrelated { a: id, b: root.id });
                };
                tick_rate.checked_div(*root_ticks_per_tick)
            }
        }
    }

    /// Builds the conversion between two exactly related domains, that is, two
    /// domains with the same root. Different roots fail with `Unrelated`: the only
    /// path between them is a [`ClockRelation`](super::ClockRelation) (TM-4, TM-5).
    pub fn conversion(
        &self,
        from: ClockDomainId,
        to: ClockDomainId,
    ) -> Result<ExactConversion, TimeError> {
        let inner = self.read();
        let a = inner.domains.get(&from).ok_or(TimeError::UnknownDomain { id: from })?;
        let b = inner.domains.get(&to).ok_or(TimeError::UnknownDomain { id: to })?;
        if a.root_id() != b.root_id() {
            return Err(TimeError::Unrelated { a: from, b: to });
        }
        ExactConversion::build(from, to, a.terms(), b.terms())
    }

    /// Converts an instant between exactly related domains (TM-4).
    pub fn convert(&self, t: TimePoint, to: ClockDomainId) -> Result<super::Converted, TimeError> {
        self.conversion(t.domain, to)?.apply(t)
    }

    /// Rescales a duration by the ratio of two nominal tick rates. Drift is ignored
    /// and no uncertainty is produced: this is for budgets, admission arithmetic and
    /// display, and it must not be used to place a `TimePoint` in another domain,
    /// which is TM-4 and TM-5.
    ///
    /// Rule: TM-9.
    pub fn rescale(&self, d: Duration, to: ClockDomainId) -> Result<Rescaled, TimeError> {
        if d.domain == to {
            return Ok(Rescaled::Exact { duration: d });
        }
        let from_rate = self.nominal_rate(d.domain)?;
        let to_rate = self.nominal_rate(to)?;
        // ticks_to = ticks · (num_to/den_to) / (num_from/den_from)
        let numerator = (d.ticks as i128)
            .checked_mul(to_rate.num() as i128)
            .and_then(|v| v.checked_mul(from_rate.den() as i128))
            .ok_or(TimeError::Overflow)?;
        let denominator = (to_rate.den() as i128)
            .checked_mul(from_rate.num() as i128)
            .ok_or(TimeError::Overflow)?;
        let floor_ticks =
            i64::try_from(numerator.div_euclid(denominator)).map_err(|_| TimeError::Overflow)?;
        let rem = numerator.rem_euclid(denominator);
        let floor = Duration::new(to, floor_ticks);
        if rem == 0 {
            Ok(Rescaled::Exact { duration: floor })
        } else {
            Ok(Rescaled::Inexact {
                floor,
                remainder: super::rational_from_u128(rem as u128, denominator as u128)?,
            })
        }
    }

    /// Orders two durations exactly, across domains if need be, by cross-multiplying
    /// against their nominal tick rates: `a` is at least `b` exactly when
    /// `a.ticks · num_b · den_a ≥ b.ticks · num_a · den_b`. Nothing is rescaled and
    /// nothing is rounded, so no direction has to be chosen and no resolution is
    /// lost. Products are checked; `Overflow` rather than a wrap.
    ///
    /// Rule: TM-21.
    pub fn compare_durations(&self, a: Duration, b: Duration) -> Result<Ordering, TimeError> {
        if a.domain == b.domain {
            return Ok(a.ticks.cmp(&b.ticks));
        }
        let ra = self.nominal_rate(a.domain)?;
        let rb = self.nominal_rate(b.domain)?;
        let lhs = (a.ticks as i128)
            .checked_mul(rb.num() as i128)
            .and_then(|v| v.checked_mul(ra.den() as i128))
            .ok_or(TimeError::Overflow)?;
        let rhs = (b.ticks as i128)
            .checked_mul(ra.num() as i128)
            .and_then(|v| v.checked_mul(rb.den() as i128))
            .ok_or(TimeError::Overflow)?;
        Ok(lhs.cmp(&rhs))
    }
}
