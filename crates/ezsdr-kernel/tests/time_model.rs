//! Phase 1 tests for `01-time-model.md`. Each name begins with the rule it proves (OV-19).

use std::cmp::Ordering;
use std::sync::{Arc, Mutex};

use ezsdr_kernel::id::{ClockDomainId, NodeId, ResourceId};
use ezsdr_kernel::module_api::Pacing;
use ezsdr_kernel::time::{
    AbsoluteDeadline, ClockDomain, ClockRegistry, ClockRelation, Converted, Duration, EpochRef,
    ManualTimeAuthority, Rational, RelativeBudget, Rescaled, TimeAuthority, TimeError, TimePoint,
    Validity,
};

const GHZ: u64 = 1_000_000_000;
const MCLK: u64 = 200_000_000;

fn arbitrary(set_by: &str) -> EpochRef {
    EpochRef::Arbitrary { set_by: set_by.to_owned() }
}

fn rat(num: u64, den: u64) -> Rational {
    Rational::new(num, den).expect("valid rational")
}

/// A registry with a 200 MHz device root registered; returns it and the root id.
fn registry_with_device_root() -> (Arc<ClockRegistry>, ClockDomainId) {
    let reg = Arc::new(ClockRegistry::new());
    let root = reg.allocate_id();
    reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("uhd.set_time_unknown_pps")))
        .expect("root registers");
    (reg, root)
}

/// Registers a derived domain with the given root ratio and origin.
fn derived(reg: &ClockRegistry, root: ClockDomainId, n: u64, d: u64, origin: i64) -> ClockDomainId {
    let id = reg.allocate_id();
    reg.register(ClockDomain::derived(id, root, rat(n, d), origin)).expect("derived registers");
    id
}

fn stream(path: &str) -> ResourceId {
    ResourceId::parse(path).expect("valid resource path")
}

// ---------------------------------------------------------------- representation

#[test]
fn tm_02_rational_normalises_by_gcd() {
    assert_eq!(rat(MCLK, 10), rat(20_000_000, 1));
    assert_eq!(rat(6, 4), rat(3, 2));
    assert_eq!(rat(6, 4).num(), 3);
    assert_eq!(rat(6, 4).den(), 2);
}

#[test]
fn tm_02_rational_rejects_zero() {
    assert_eq!(Rational::new(0, 5), Err(TimeError::InvalidRational));
    assert_eq!(Rational::new(5, 0), Err(TimeError::InvalidRational));
}

#[test]
fn tm_02_rational_mul_div_use_wide_intermediates() {
    let huge = rat(u64::MAX, 1);
    assert_eq!(huge.checked_mul(rat(1, u64::MAX)), Ok(Rational::ONE));
    let p = 1u64 << 40;
    assert_eq!(rat(p, 3).checked_mul(rat(3, p)), Ok(Rational::ONE));
    assert_eq!(rat(p, 3).checked_div(rat(p, 3)), Ok(Rational::ONE));
    // A reduced result that no longer fits in 64 bits is an error, not a wrap.
    assert_eq!(rat(u64::MAX, 1).checked_mul(rat(u64::MAX, 1)), Err(TimeError::Overflow));
}

#[test]
fn tm_02_rational_cmp_cross_multiplies() {
    assert!(rat(1, 3) < rat(2, 5));
    assert!(rat(u64::MAX, 1) > rat(u64::MAX - 1, 1));
    assert!(rat(1, u64::MAX) < rat(1, u64::MAX - 1));
}

// ---------------------------------------------------------------- exact conversion

#[test]
fn tm_04_derived_to_root_exact() {
    let (reg, root) = registry_with_device_root();
    let a = derived(&reg, root, 10, 1, 1_000_000_003);
    let got = reg.convert(TimePoint::new(a, 7), root).expect("same root converts");
    assert_eq!(got, Converted::Exact { point: TimePoint::new(root, 1_000_000_073) });
}

#[test]
fn tm_04_sibling_20_25_msps_exact() {
    let (reg, root) = registry_with_device_root();
    let a = derived(&reg, root, 10, 1, 0); // 20 Msps
    let b = derived(&reg, root, 8, 1, 0); // 25 Msps
    assert_eq!(
        reg.convert(TimePoint::new(a, 4), b),
        Ok(Converted::Exact { point: TimePoint::new(b, 5) })
    );
    assert_eq!(
        reg.convert(TimePoint::new(a, 1_000_000_000_000), b),
        Ok(Converted::Exact { point: TimePoint::new(b, 1_250_000_000_000) })
    );
}

#[test]
fn tm_04_sibling_20_25_msps_inexact() {
    let (reg, root) = registry_with_device_root();
    let a = derived(&reg, root, 10, 1, 0);
    let b = derived(&reg, root, 8, 1, 0);
    let conv = reg.conversion(a, b).expect("same root");
    assert_eq!(
        conv.apply(TimePoint::new(a, 7)),
        Ok(Converted::Inexact { floor: TimePoint::new(b, 8), remainder: rat(3, 4) })
    );
    assert_eq!(
        conv.try_exact(TimePoint::new(a, 7)),
        Err(TimeError::Inexact { floor: TimePoint::new(b, 8) })
    );
}

#[test]
fn tm_04_disjoint_grids_never_exact() {
    let (reg, root) = registry_with_device_root();
    let a = derived(&reg, root, 10, 1, 1_000_000_003);
    let b = derived(&reg, root, 8, 1, 1_000_000_000);
    let conv = reg.conversion(a, b).expect("same root");
    for t in 0..1000 {
        let got = conv.apply(TimePoint::new(a, t)).expect("in range");
        assert!(matches!(got, Converted::Inexact { .. }), "t={t} landed on a shared instant");
    }
}

#[test]
fn tm_08_conversion_overflow_is_error() {
    let (reg, root) = registry_with_device_root();
    let big = 1u64 << 31;
    let slow = derived(&reg, root, big, 1, 0); // one tick = 2^31 root ticks
    let fast = derived(&reg, root, 1, big, 0); // 2^31 ticks per root tick
    let t = TimePoint::new(slow, 1i64 << 62);
    assert_eq!(reg.convert(t, root), Err(TimeError::Overflow));
    assert_eq!(reg.convert(t, fast), Err(TimeError::Overflow));
    // The shrinking direction stays in range, which is what makes the two above a
    // magnitude check rather than a blanket refusal.
    assert!(reg.convert(TimePoint::new(root, 1i64 << 62), slow).is_ok());
}

#[test]
fn tm_03_registration_limits() {
    let (reg, root) = registry_with_device_root();
    let over = (1u64 << 31) + 1;
    let id = reg.allocate_id();
    assert_eq!(
        reg.register(ClockDomain::derived(id, root, rat(over, 1), 0)),
        Err(TimeError::LimitExceeded)
    );

    let mid = derived(&reg, root, 10, 1, 0);
    let id = reg.allocate_id();
    assert!(matches!(
        reg.register(ClockDomain::derived(id, mid, rat(2, 1), 0)),
        Err(TimeError::Unrelated { .. })
    ));

    let ghost = ClockDomainId::local(9_999);
    let id = reg.allocate_id();
    assert_eq!(
        reg.register(ClockDomain::derived(id, ghost, rat(2, 1), 0)),
        Err(TimeError::UnknownDomain { id: ghost })
    );
}

#[test]
fn tm_03_root_tick_rate_capped() {
    let reg = ClockRegistry::new();
    let id = reg.allocate_id();
    let p = 1u64 << 40;
    assert_eq!(
        reg.register(ClockDomain::root(id, rat(p, p - 1), arbitrary("test"))),
        Err(TimeError::LimitExceeded)
    );
}

#[test]
fn tm_05_unrelated_roots_need_relation() {
    let (reg, root) = registry_with_device_root();
    assert!(matches!(
        reg.conversion(root, ClockDomainId::UTC),
        Err(TimeError::Unrelated { .. })
    ));
}

// ---------------------------------------------------------------- arithmetic

#[test]
fn tm_06_cross_domain_cmp_is_error() {
    let (reg, root) = registry_with_device_root();
    let a = derived(&reg, root, 10, 1, 0);
    let pa = TimePoint::new(a, 5);
    let pr = TimePoint::new(root, 5);
    assert!(matches!(pa.try_cmp(pr), Err(TimeError::DomainMismatch { .. })));
    assert!(matches!(pa.checked_sub(pr), Err(TimeError::DomainMismatch { .. })));
    assert!(matches!(pa.ticks_in(root), Err(TimeError::DomainMismatch { .. })));
    assert_eq!(pa.try_cmp(TimePoint::new(a, 6)), Ok(Ordering::Less));
    assert_eq!(pa.checked_sub(TimePoint::new(a, 2)), Ok(Duration::new(a, 3)));
    assert_eq!(pa.ticks_in(a), Ok(5));
}

#[test]
fn tm_07_duration_add_checks_domain() {
    let (reg, root) = registry_with_device_root();
    let a = derived(&reg, root, 10, 1, 0);
    let pa = TimePoint::new(a, 100);
    assert!(matches!(
        pa.checked_add(Duration::new(root, 10)),
        Err(TimeError::DomainMismatch { .. })
    ));
    assert_eq!(pa.checked_add(Duration::new(a, 10)), Ok(TimePoint::new(a, 110)));
}

#[test]
fn tm_08_tick_add_overflow_is_error() {
    let d = ClockDomainId::HOST_MONOTONIC;
    assert_eq!(
        TimePoint::new(d, i64::MAX).checked_add(Duration::new(d, 1)),
        Err(TimeError::Overflow)
    );
    assert_eq!(
        TimePoint::new(d, i64::MIN).checked_sub(TimePoint::new(d, 1)),
        Err(TimeError::Overflow)
    );
}

#[test]
fn tm_09_duration_nominal_rescale() {
    let (reg, root) = registry_with_device_root();
    let msps20 = derived(&reg, root, 10, 1, 0);
    assert_eq!(
        reg.rescale(Duration::new(msps20, 1000), ClockDomainId::HOST_MONOTONIC),
        Ok(Rescaled::Exact { duration: Duration::new(ClockDomainId::HOST_MONOTONIC, 50_000) })
    );

    let hz3 = reg.allocate_id();
    reg.register(ClockDomain::root(hz3, rat(3, 1), arbitrary("test"))).expect("root");
    assert_eq!(
        reg.rescale(Duration::new(hz3, 1), ClockDomainId::HOST_MONOTONIC),
        Ok(Rescaled::Inexact {
            floor: Duration::new(ClockDomainId::HOST_MONOTONIC, 333_333_333),
            remainder: rat(1, 3),
        })
    );
}

// ---------------------------------------------------------------- relations

fn device_to_utc(dev: ClockDomainId, drift: f64, drift_uncertainty: f64) -> ClockRelation {
    ClockRelation {
        source: dev,
        target: ClockDomainId::UTC,
        measured_at: TimePoint::new(dev, 200_000_000),
        offset: TimePoint::new(ClockDomainId::UTC, 1_700_000_000_000_000_000),
        drift,
        drift_uncertainty,
        uncertainty: Duration::new(ClockDomainId::UTC, 50),
        method: "test.poll".to_owned(),
        valid: Validity { from: TimePoint::new(dev, 0), to: Some(TimePoint::new(dev, 1 << 40)) },
    }
}

#[test]
fn tm_14_relation_converts_with_uncertainty() {
    let (reg, root) = registry_with_device_root();
    let rel = device_to_utc(root, 1e-6, 1e-8);
    let got = rel.convert(&reg, TimePoint::new(root, 400_000_000)).expect("inside validity");
    assert_eq!(got.nominal, TimePoint::new(ClockDomainId::UTC, 1_700_000_001_000_001_000));
    assert_eq!(got.uncertainty, Duration::new(ClockDomainId::UTC, 61));
}

#[test]
fn tm_14_relation_outside_validity() {
    let (reg, root) = registry_with_device_root();
    let rel = device_to_utc(root, 1e-6, 1e-8);
    let beyond = TimePoint::new(root, (1i64 << 40) + 1);
    assert_eq!(rel.convert(&reg, beyond), Err(TimeError::OutsideValidity { at: beyond }));
    let before = TimePoint::new(root, -1);
    assert_eq!(rel.convert(&reg, before), Err(TimeError::OutsideValidity { at: before }));
}

#[test]
fn tm_14_uncertainty_grows_with_elapsed_time() {
    let (reg, root) = registry_with_device_root();
    let rel = device_to_utc(root, 0.0, 1e-8);
    // 600 s after measured_at, at 200 MHz.
    let t = TimePoint::new(root, 200_000_000 + 600 * 200_000_000);
    let got = rel.convert(&reg, t).expect("inside validity");
    // 600e9 ns * 1e-8 = 6000 ns, plus the stored 50 ns and the rounding tick.
    assert_eq!(got.uncertainty, Duration::new(ClockDomainId::UTC, 50 + 6_000 + 1));
    assert!(got.uncertainty.ticks > 5_000, "the bound must grow, not stay near 51 ns");
}

#[test]
fn tm_14_zero_drift_uncertainty_is_constant() {
    let (reg, root) = registry_with_device_root();
    let rel = device_to_utc(root, 1e-6, 0.0);
    let near = rel.convert(&reg, TimePoint::new(root, 400_000_000)).expect("valid");
    let far = rel
        .convert(&reg, TimePoint::new(root, 200_000_000 + 600 * 200_000_000))
        .expect("valid");
    assert_eq!(near.uncertainty, Duration::new(ClockDomainId::UTC, 51));
    assert_eq!(far.uncertainty, near.uncertainty);
}

// ---------------------------------------------------------------- deadlines

#[test]
fn tm_15_budget_deadline_from_arrival() {
    let budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 500_000))
        .expect("host.monotonic budget");
    let arrival = TimePoint::new(ClockDomainId::HOST_MONOTONIC, 1_000);
    assert_eq!(
        budget.deadline_from(arrival),
        Ok(AbsoluteDeadline::new(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 501_000)))
    );
    let (reg, root) = registry_with_device_root();
    let _ = &reg;
    assert!(matches!(
        budget.deadline_from(TimePoint::new(root, 1_000)),
        Err(TimeError::DomainMismatch { .. })
    ));
}

#[test]
fn tm_15_budget_rejects_non_host_domain() {
    let (reg, root) = registry_with_device_root();
    let sample_clock = derived(&reg, root, 10, 1, 0);
    assert!(matches!(
        RelativeBudget::new(Duration::new(sample_clock, 1_000)),
        Err(TimeError::DomainMismatch { .. })
    ));
    assert!(matches!(
        RelativeBudget::new(Duration::new(ClockDomainId::UTC, 1_000)),
        Err(TimeError::DomainMismatch { .. })
    ));
}

// ---------------------------------------------------------------- registry and SampleClocks

#[test]
fn tm_08_relation_rounding_does_not_saturate_at_two_to_the_63() {
    // `i64::MAX as f64` rounds *up* to 2^63, so a guard written against it lets
    // 2^63 through and the cast then saturates to `i64::MAX` — a silently wrong
    // instant where TM-2 and TM-8 require an error.
    let (reg, root) = registry_with_device_root();
    let mut rel = device_to_utc(root, 9_223_372_036_854_775_808.0, 0.0);
    rel.offset = TimePoint::new(ClockDomainId::UTC, -100);
    rel.measured_at = TimePoint::new(root, 0);
    rel.uncertainty = Duration::new(ClockDomainId::UTC, 0);
    assert_eq!(rel.convert(&reg, TimePoint::new(root, 1)), Err(TimeError::Overflow));

    // The uncertainty term has the same boundary.
    let mut rel = device_to_utc(root, 0.0, 9_223_372_036_854_775_808.0);
    rel.measured_at = TimePoint::new(root, 0);
    assert_eq!(rel.convert(&reg, TimePoint::new(root, 1)), Err(TimeError::Overflow));
}

#[test]
fn tm_11_registration_at_the_id_ceiling_does_not_wrap() {
    // TM-11: ids are strictly increasing and never reused. A plain `+ 1` on the top
    // id panics in debug and wraps to the reserved id 0 in release, so the registry
    // refuses it outright rather than leaving itself unable to allocate.
    let reg = ClockRegistry::new();
    let top = ClockDomainId::local(u32::MAX);
    assert_eq!(
        reg.register(ClockDomain::root(top, rat(3, 1), arbitrary("test"))),
        Err(TimeError::LimitExceeded)
    );
    let a = reg.allocate_id();
    let b = reg.allocate_id();
    assert!(a.local < b.local, "still strictly increasing");
    assert_ne!(a, ClockDomainId::UTC);
    assert_ne!(a, ClockDomainId::HOST_MONOTONIC);
}

#[test]
fn tm_16a_host_monotonic_accepts_a_callback_in_every_class() {
    // TM-16a makes `host.monotonic` governed in every class, so `governs`, `now`,
    // `wait_until` and `schedule` must agree about it. TM-16a1 decides only who
    // drives it.
    let host = ClockDomainId::HOST_MONOTONIC;
    for pacing in [Pacing::FreeRunning, Pacing::WallPaced, Pacing::Device] {
        let (reg, root) = registry_with_device_root();
        let auth = ManualTimeAuthority::new(reg, root, &[], pacing).expect("authority");
        assert!(auth.governs(host), "{pacing:?}");
        let now = auth.now(host).expect("always governed");
        assert!(
            auth.schedule(TimePoint::new(host, now.ticks + 1_000_000_000), Box::new(|_| ())).is_ok(),
            "{pacing:?}: governs() says yes, so schedule must not say NotGoverned"
        );
    }
}

#[test]
fn tm_16c_schedule_and_now_agree_on_host_monotonic() {
    // TM-16c: "`schedule(t, f)` requires `t` in a governed domain at or after
    // `now`". Outside the Simulation class `now(host)` is the real clock, so the
    // past check has to read the same value.
    let host = ClockDomainId::HOST_MONOTONIC;
    let (reg, root) = registry_with_device_root();
    let auth = ManualTimeAuthority::new(reg, root, &[], Pacing::Device).expect("authority");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let now = auth.now(host).expect("always governed").ticks;
    assert!(now > 0, "the real clock has moved");
    assert!(matches!(
        auth.schedule(TimePoint::new(host, 0), Box::new(|_| ())),
        Err(TimeError::InPast { .. })
    ));
    assert!(auth.schedule(TimePoint::new(host, now + 1_000_000_000), Box::new(|_| ())).is_ok());
}

#[test]
fn tm_11_reserved_domains_present() {
    let reg = ClockRegistry::new();
    let utc = reg.get(ClockDomainId::UTC).expect("utc is reserved");
    let host = reg.get(ClockDomainId::HOST_MONOTONIC).expect("host.monotonic is reserved");
    assert_eq!(reg.nominal_rate(utc.id), Ok(rat(GHZ, 1)));
    assert_eq!(reg.nominal_rate(host.id), Ok(rat(GHZ, 1)));
    assert_eq!(utc.root_id(), utc.id);
    assert!(matches!(
        reg.get(ClockDomainId::UTC).map(|d| d.kind),
        Ok(ezsdr_kernel::time::ClockDomainKind::Root { epoch: EpochRef::Utc1970, .. })
    ));
}

#[test]
fn tm_11_registry_allocates_monotonic_unique() {
    let reg = ClockRegistry::new();
    let ids: Vec<_> = (0..3).map(|_| reg.allocate_id()).collect();
    assert!(ids.windows(2).all(|w| w[0].local < w[1].local));
    assert!(ids.iter().all(|id| id.node == NodeId::LOCAL));
    assert!(ids[0].local >= ClockDomainId::FIRST_ALLOCATABLE);
}

#[test]
fn tm_12_duplicate_registration_is_error() {
    let (reg, root) = registry_with_device_root();
    assert_eq!(
        reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("again"))),
        Err(TimeError::DuplicateDomain { id: root })
    );
}

#[test]
fn tm_13a_sample_clock_declared_before_its_origin_exists() {
    let (reg, root) = registry_with_device_root();
    let handle = reg
        .declare_sample_clock(stream("dev0/rx/0"), root, rat(10, 1))
        .expect("declared at prepare");
    // TM-13a: the id and the ratio exist now; the origin does not.
    assert!(!reg.is_registered(handle.id));
    assert_eq!(handle.root_ticks_per_tick, rat(10, 1));
    reg.register_sample_clock(&handle, 1_000_000_003).expect("registered before the first block");
    assert_eq!(reg.nominal_rate(handle.id), Ok(rat(20_000_000, 1)));
}

#[test]
fn tm_13b_first_block_is_tick_zero() {
    let (reg, root) = registry_with_device_root();
    let handle =
        reg.declare_sample_clock(stream("dev0/rx/0"), root, rat(10, 1)).expect("declared");
    let first_sample_root_tick = 1_000_000_003;
    let domain = reg.register_sample_clock(&handle, first_sample_root_tick).expect("registered");
    assert_eq!(
        reg.convert(TimePoint::new(root, first_sample_root_tick), domain),
        Ok(Converted::Exact { point: TimePoint::new(domain, 0) })
    );
    // TM-12: a domain that was never registered cannot be named by anything leaving
    // its producer. The Kernel enforces that at the registry, which is the only
    // seam it has: an unregistered domain has no rate and no conversion, so no
    // consumer can place a block that names one. (The refusal of the block itself
    // is a producer obligation — see finding D16.)
    let ghost = ClockDomainId::local(9_999);
    assert!(!reg.is_registered(ghost));
    assert_eq!(reg.get(ghost), Err(TimeError::UnknownDomain { id: ghost }));
    assert_eq!(reg.nominal_rate(ghost), Err(TimeError::UnknownDomain { id: ghost }));
    assert_eq!(
        reg.conversion(ghost, root),
        Err(TimeError::UnknownDomain { id: ghost }),
        "nothing can convert a block time in an unregistered domain"
    );
    assert!(matches!(
        reg.convert(TimePoint::new(ghost, 0), domain),
        Err(TimeError::UnknownDomain { .. })
    ));
}

#[test]
fn tm_13c_sample_clock_new_id_on_rate_change() {
    let (reg, root) = registry_with_device_root();
    let s = stream("dev0/rx/0");
    let a = reg.declare_sample_clock(s.clone(), root, rat(10, 1)).expect("declared");
    reg.register_sample_clock(&a, 0).expect("registered");

    // A cold rate change: end the current clock and allocate a new id.
    reg.end(a.id, TimePoint::new(root, 500)).expect("ends once");
    let b = reg.declare_sample_clock(s, root, rat(8, 1)).expect("declared");
    reg.register_sample_clock(&b, 500).expect("registered");

    assert_ne!(a.id, b.id);
    assert_eq!(reg.get(a.id).expect("a").ended_at, Some(TimePoint::new(root, 500)));
    assert_eq!(reg.get(b.id).expect("b").ended_at, None);
    assert_eq!(reg.end(a.id, TimePoint::new(root, 600)), Err(TimeError::Stopped));

    let records = reg.sample_clock_records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].domain, a.id);
    assert_eq!(records[0].ended_at, Some(TimePoint::new(root, 500)));
    assert_eq!(records[0].origin, TimePoint::new(root, 0));
    assert_eq!(records[1].domain, b.id);
    assert_eq!(records[1].origin, TimePoint::new(root, 500));
    assert_eq!(records[1].nominal_rate, rat(25_000_000, 1));
}

#[test]
fn tm_13e_tx_origin_fixed_at_arm() {
    let (reg, root) = registry_with_device_root();
    let handle =
        reg.declare_sample_clock(stream("dev0/tx/0"), root, rat(10, 1)).expect("declared at prepare");
    // TM-13e: prepare precedes arm, so no origin exists yet; a plan-time admission
    // check compares leads only, which needs the nominal rate and not the origin.
    assert!(!reg.is_registered(handle.id));
    assert_eq!(handle.root_ticks_per_tick, rat(10, 1));

    let arm_anchor = 4_000_000_000;
    let domain = reg.register_sample_clock(&handle, arm_anchor).expect("registered at arm");
    // The grid exists at the first admission point after arm: a burst target in root
    // ticks converts onto it (SC-23a).
    assert_eq!(
        reg.convert(TimePoint::new(root, arm_anchor + 25), domain),
        Ok(Converted::Inexact { floor: TimePoint::new(domain, 2), remainder: rat(1, 2) })
    );
    assert_eq!(reg.sample_clock_records()[0].origin, TimePoint::new(root, arm_anchor));
}

// ---------------------------------------------------------------- duration comparison

#[test]
fn tm_21_duration_cmp_same_domain() {
    let (reg, root) = registry_with_device_root();
    let sc = derived(&reg, root, 10, 1, 0);
    assert_eq!(reg.compare_durations(Duration::new(sc, 1_000), Duration::new(sc, 2_000)), Ok(Ordering::Less));
    assert_eq!(Duration::new(sc, 1_000).try_cmp(Duration::new(sc, 2_000)), Ok(Ordering::Less));
}

#[test]
fn tm_21_duration_cmp_across_domains_is_exact() {
    let (reg, root) = registry_with_device_root();
    let msps20 = derived(&reg, root, 10, 1, 0);
    let one_ms = Duration::new(ClockDomainId::HOST_MONOTONIC, 1_000_000);
    let thirty_k = Duration::new(msps20, 30_000); // 1.5 ms
    assert_eq!(reg.compare_durations(one_ms, thirty_k), Ok(Ordering::Less));
    assert_eq!(reg.compare_durations(thirty_k, one_ms), Ok(Ordering::Greater));
}

#[test]
fn tm_21_duration_cmp_survives_awkward_rates() {
    let reg = ClockRegistry::new();
    let hz3 = reg.allocate_id();
    reg.register(ClockDomain::root(hz3, rat(3, 1), arbitrary("test"))).expect("root");
    let lte = reg.allocate_id();
    reg.register(ClockDomain::root(lte, rat(30_720_000, 1), arbitrary("test"))).expect("root");

    let one_ms = Duration::new(ClockDomainId::HOST_MONOTONIC, 1_000_000);
    // One tick at 3 Hz is 333 ms: a rescale-and-round implementation reports this
    // 333 times too strict.
    assert_eq!(reg.compare_durations(one_ms, Duration::new(hz3, 1)), Ok(Ordering::Less));
    // One tick at 30.72 Msps is 3125/96 ns, which no rescale represents exactly.
    assert_eq!(reg.compare_durations(one_ms, Duration::new(lte, 1)), Ok(Ordering::Greater));
    assert_eq!(reg.compare_durations(Duration::new(lte, 30_720), one_ms), Ok(Ordering::Equal));
}

#[test]
fn tm_21_duration_cmp_overflow_is_error() {
    let reg = ClockRegistry::new();
    let cap = 1u64 << 31;
    let fast_root = reg.allocate_id();
    reg.register(ClockDomain::root(fast_root, rat(cap, 1), arbitrary("test"))).expect("root");
    let slow_root = reg.allocate_id();
    reg.register(ClockDomain::root(slow_root, rat(1, cap), arbitrary("test"))).expect("root");
    let fast = derived(&reg, fast_root, 1, cap, 0); // nominal 2^62 / 1
    let slow = derived(&reg, slow_root, cap, 1, 0); // nominal 1 / 2^62
    assert_eq!(
        reg.compare_durations(Duration::new(fast, i64::MAX), Duration::new(slow, i64::MAX)),
        Err(TimeError::Overflow)
    );
}

// ---------------------------------------------------------------- the Authority

fn sim_authority() -> (Arc<ClockRegistry>, ClockDomainId, Arc<ManualTimeAuthority>) {
    let (reg, root) = registry_with_device_root();
    let auth = ManualTimeAuthority::new(reg.clone(), root, &[], Pacing::FreeRunning)
        .expect("authority over a registered root");
    (reg, root, Arc::new(auth))
}

#[test]
fn tm_16_authority_order_and_now() {
    let (_reg, root, auth) = sim_authority();
    let log: Arc<Mutex<Vec<(i64, i64)>>> = Arc::new(Mutex::new(Vec::new()));
    for at in [30i64, 10, 10] {
        let log = log.clone();
        let auth2 = auth.clone();
        auth.schedule(
            TimePoint::new(root, at),
            Box::new(move |fired| {
                let now = auth2.now(root).expect("governed").ticks;
                log.lock().expect("lock").push((fired.ticks, now));
            }),
        )
        .expect("scheduled at or after now");
    }
    assert_eq!(auth.advance_to(TimePoint::new(root, 30)), Ok(3));
    // Ascending time, ties in insertion order; now() inside each equals its fire time.
    assert_eq!(*log.lock().expect("lock"), vec![(10, 10), (10, 10), (30, 30)]);
    assert_eq!(auth.now(root), Ok(TimePoint::new(root, 30)));
}

#[test]
fn tm_17_authority_nested_schedule() {
    let (_reg, root, auth) = sim_authority();
    let log: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let (log, inner, weak) = (log.clone(), auth.clone(), auth.clone());
        auth.schedule(
            TimePoint::new(root, 10),
            Box::new(move |t| {
                log.lock().expect("lock").push(t.ticks);
                let log = log.clone();
                inner
                    .schedule(
                        TimePoint::new(root, 20),
                        Box::new(move |t| log.lock().expect("lock").push(t.ticks)),
                    )
                    .expect("scheduled from inside a callback");
                drop(weak);
            }),
        )
        .expect("scheduled");
    }
    {
        let log = log.clone();
        auth.schedule(
            TimePoint::new(root, 30),
            Box::new(move |t| log.lock().expect("lock").push(t.ticks)),
        )
        .expect("scheduled");
    }
    assert_eq!(auth.advance_to(TimePoint::new(root, 30)), Ok(3));
    assert_eq!(*log.lock().expect("lock"), vec![10, 20, 30]);
}

#[test]
fn tm_17_authority_cancel_and_in_past() {
    let (_reg, root, auth) = sim_authority();
    let h = auth
        .schedule(TimePoint::new(root, 10), Box::new(|_| unreachable!("cancelled")))
        .expect("scheduled");
    assert!(auth.cancel(h));
    assert!(!auth.cancel(h));
    assert_eq!(auth.advance_to(TimePoint::new(root, 20)), Ok(0));

    assert!(matches!(
        auth.schedule(TimePoint::new(root, 5), Box::new(|_| ())),
        Err(TimeError::InPast { .. })
    ));
    assert!(matches!(auth.advance_to(TimePoint::new(root, 5)), Err(TimeError::InPast { .. })));
}

#[test]
fn tm_17b_advance_to_zero_delay_cap() {
    let (reg, root) = registry_with_device_root();
    let auth = Arc::new(
        ManualTimeAuthority::new(reg, root, &[], Pacing::FreeRunning)
            .expect("authority")
            .with_callback_cap(8),
    );

    fn respawn(auth: Arc<ManualTimeAuthority>, root: ClockDomainId, at: i64) {
        let again = auth.clone();
        let _ = auth.schedule(
            TimePoint::new(root, at),
            Box::new(move |t| respawn(again, root, t.ticks)),
        );
    }
    respawn(auth.clone(), root, 10);
    assert_eq!(auth.advance_to(TimePoint::new(root, 20)), Err(TimeError::LimitExceeded));
}

#[test]
fn tm_16_authority_now_in_derived_floors() {
    let (reg, root, auth) = sim_authority();
    let sc = derived(&reg, root, 10, 1, 3);
    auth.advance_to(TimePoint::new(root, 1_000_000_007)).expect("advances");
    assert_eq!(auth.now(sc), Ok(TimePoint::new(sc, 100_000_000)));
}

#[test]
fn tm_16_authority_wait_until_wakes() {
    let (_reg, root, auth) = sim_authority();
    let waiter = {
        let auth = auth.clone();
        std::thread::spawn(move || auth.wait_until(TimePoint::new(root, 50)))
    };
    // Advance in two steps so the waiter has to be woken by the second.
    auth.advance_to(TimePoint::new(root, 20)).expect("advances");
    auth.advance_to(TimePoint::new(root, 50)).expect("advances");
    assert_eq!(waiter.join().expect("thread"), Ok(()));
}

#[test]
fn tm_16d_wait_until_non_governed() {
    let (reg, root, auth) = sim_authority();
    let _ = root;
    let other = reg.allocate_id();
    reg.register(ClockDomain::root(other, rat(MCLK, 1), arbitrary("other"))).expect("root");
    assert_eq!(
        auth.wait_until(TimePoint::new(other, 5)),
        Err(TimeError::NotGoverned { id: other })
    );
}

#[test]
fn tm_16a_host_monotonic_always_governed() {
    let (reg, root) = registry_with_device_root();
    let device = ManualTimeAuthority::new(reg.clone(), root, &[], Pacing::Device).expect("auth");
    let virt_root = reg.allocate_id();
    reg.register(ClockDomain::root(virt_root, rat(MCLK, 1), arbitrary("sim.run_start")))
        .expect("root");
    let sim =
        ManualTimeAuthority::new(reg, virt_root, &[], Pacing::FreeRunning).expect("auth");
    assert!(device.governs(ClockDomainId::HOST_MONOTONIC));
    assert!(sim.governs(ClockDomainId::HOST_MONOTONIC));
    assert!(device.now(ClockDomainId::HOST_MONOTONIC).is_ok());
    assert!(sim.now(ClockDomainId::HOST_MONOTONIC).is_ok());
}

#[test]
fn tm_16a1_host_monotonic_driven_only_in_simulation() {
    let (reg, root) = registry_with_device_root();
    let sim = ManualTimeAuthority::new(reg.clone(), root, &[], Pacing::FreeRunning).expect("auth");
    sim.advance_to(TimePoint::new(root, MCLK as i64)).expect("one simulated second");
    assert_eq!(
        sim.now(ClockDomainId::HOST_MONOTONIC),
        Ok(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 1_000_000_000))
    );

    let (reg2, root2) = registry_with_device_root();
    let _ = &reg2;
    let rt = ManualTimeAuthority::new(reg2, root2, &[], Pacing::WallPaced).expect("auth");
    rt.advance_to(TimePoint::new(root2, MCLK as i64)).expect("one simulated second");
    let host = rt.now(ClockDomainId::HOST_MONOTONIC).expect("always governed").ticks;
    assert!(host < 1_000_000_000, "wall-paced host.monotonic tracks the real clock, got {host}");
    let _ = &reg;
}

#[test]
fn tm_16b_engine_governs_a_drifting_virtual_device() {
    let reg = Arc::new(ClockRegistry::new());
    let primary = reg.allocate_id();
    reg.register(ClockDomain::root(primary, rat(MCLK, 1), arbitrary("sim.run_start")))
        .expect("root");
    let second = reg.allocate_id();
    // A second virtual device, drifting: a different rate and no exact relation.
    reg.register(ClockDomain::root(second, rat(199_999_997, 1), arbitrary("sim.run_start")))
        .expect("root");
    let auth = ManualTimeAuthority::new(reg.clone(), primary, &[second], Pacing::FreeRunning)
        .expect("authority declares both roots");

    assert!(auth.governs(second));
    assert_eq!(auth.now(second), Ok(TimePoint::new(second, 0)));

    let order = |auth: &ManualTimeAuthority| {
        let log: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
        for at in [7i64, 3, 5] {
            let log = log.clone();
            auth.schedule(
                TimePoint::new(second, at),
                Box::new(move |t| log.lock().expect("lock").push(t.ticks)),
            )
            .expect("schedules on the second root");
        }
        auth.advance_to(TimePoint::new(second, 10)).expect("advances the second root");
        log.lock().expect("lock").clone()
    };
    let first_run = order(&auth);
    let auth2 = ManualTimeAuthority::new(reg, primary, &[second], Pacing::FreeRunning)
        .expect("authority");
    assert_eq!(first_run, vec![3, 5, 7]);
    assert_eq!(first_run, order(&auth2), "the event order reproduces");
}

#[test]
fn tm_16b_ungoverned_root_is_not_answered() {
    let (reg, root, auth) = sim_authority();
    let foreign = reg.allocate_id();
    reg.register(ClockDomain::root(foreign, rat(MCLK, 1), arbitrary("hackrf-like")))
        .expect("root");
    assert!(!auth.governs(foreign));
    assert_eq!(auth.now(foreign), Err(TimeError::NotGoverned { id: foreign }));

    // Governance settles who advances a clock, not what arithmetic is exact: the
    // relation still converts (TM-16b1).
    let rel = ClockRelation {
        source: foreign,
        target: root,
        measured_at: TimePoint::new(foreign, 0),
        offset: TimePoint::new(root, 1_000),
        drift: 0.0,
        drift_uncertainty: 0.0,
        uncertainty: Duration::new(root, 2),
        method: "test.poll".to_owned(),
        valid: Validity { from: TimePoint::new(foreign, 0), to: None },
    };
    let got = rel.convert(&reg, TimePoint::new(foreign, 400)).expect("converts");
    assert_eq!(got.nominal, TimePoint::new(root, 1_400));
}
