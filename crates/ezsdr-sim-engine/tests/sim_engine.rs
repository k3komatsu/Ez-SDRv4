use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::id::{ClockDomainId, ModuleId};
use ezsdr_kernel::module_api::{
    Authority, Factories, ModuleRegistry, Pacing, ProfileRef, Role, Version,
};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::spec::{Ident, Namespace, Value};
use ezsdr_kernel::time::{
    ClockDomain, ClockDomainKind, ClockRegistry, EpochRef, Rational, TimeAuthority, TimeError,
    TimePoint,
};
use ezsdr_sim::{SimRng, VIRTUAL_EPOCH, VIRTUAL_TICK_RATE_HZ};
use ezsdr_sim_engine::{descriptor, SimEngine};

fn engine() -> (SimEngine, Arc<ClockRegistry>) {
    let clocks = Arc::new(ClockRegistry::new());
    let engine = SimEngine::new(clocks.clone()).unwrap();
    (engine, clocks)
}

fn module_ref() -> ezsdr_kernel::module_api::ModuleRef {
    ezsdr_kernel::module_api::ModuleRef {
        id: ModuleId::parse("ezsdr.sim-engine").unwrap(),
        version: Version::new(1, 0, 0),
    }
}

fn binding() -> Binding {
    Binding {
        module: module_ref(),
        selector: BTreeMap::new(),
        profile: None,
        feed: None,
    }
}

#[test]
fn se_09_exhausted_clock_ids_refuse_a_new_engine() {
    let clocks = Arc::new(ClockRegistry::new());
    clocks.register(ClockDomain::root(ClockDomainId::local(u32::MAX - 1),
        Rational::ONE, EpochRef::Arbitrary { set_by: "test".to_owned() })).unwrap();
    assert!(matches!(SimEngine::new(clocks.clone()), Err(TimeError::LimitExceeded)));
    assert_eq!(clocks.allocate_id(), Err(TimeError::LimitExceeded));
}

#[test]
fn se_08_descriptor_registers() {
    let mut modules = ModuleRegistry::new();
    let mut checks = ezsdr_kernel::binding::AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_sim::register(&mut modules, &mut checks, &mut kinds).unwrap();
    modules
        .register(
            descriptor(),
            Factories {
                authority: true,
                ..Factories::default()
            },
        )
        .unwrap();

    let registered = modules.modules().next().unwrap();
    assert_eq!(registered.id.as_str(), "ezsdr.sim-engine");
    assert_eq!(registered.version, Version::new(1, 0, 0));
    assert_eq!(registered.roles, vec![Role::Authority]);
    assert_eq!(registered.vocabularies[0].id, Namespace::parse("sim").unwrap());
    assert!(registered.vocabularies[0].req.matches(Version::new(1, 0, 0)));
}

#[test]
fn se_09_the_virtual_root_is_registered() {
    let (engine, clocks) = engine();
    let root = clocks.get(engine.root()).unwrap();
    assert!(matches!(
        root.kind,
        ClockDomainKind::Root { tick_rate, epoch: EpochRef::Arbitrary { ref set_by } }
            if tick_rate == Rational::new(VIRTUAL_TICK_RATE_HZ, 1).unwrap()
                && set_by == VIRTUAL_EPOCH
    ));
    let declared = Authority::descriptor(&engine);
    assert_eq!(declared.module, module_ref());
    assert_eq!(declared.governs, [engine.root(), ClockDomainId::HOST_MONOTONIC]);
    assert_eq!(declared.pacing, Pacing::FreeRunning);
    let time = Authority::time(&engine);
    assert_eq!(time.primary_root(), engine.root());
    assert!(time.governs(engine.root()));
    assert!(time.governs(ClockDomainId::HOST_MONOTONIC));

    let from_binding = SimEngine::from_binding(&binding(), Arc::new(ClockRegistry::new())).unwrap();
    assert_eq!(from_binding.root(), ClockDomainId::local(2));

    let mut selector = binding();
    selector.selector.insert(Ident::parse("extra").unwrap(), Value::Bool(true));
    assert!(SimEngine::from_binding(&selector, Arc::new(ClockRegistry::new()))
        .err()
        .unwrap()
        .message
        .starts_with("SE-9: "));

    let mut profile = binding();
    profile.profile = Some(ProfileRef {
        name: "ideal".to_owned(),
        version: Version::new(1, 0, 0),
    });
    assert!(SimEngine::from_binding(&profile, Arc::new(ClockRegistry::new()))
        .err()
        .unwrap()
        .message
        .starts_with("SE-9: "));

    let mut wrong_module = binding();
    wrong_module.module.id = ModuleId::parse("ezsdr.other").unwrap();
    assert!(SimEngine::from_binding(&wrong_module, Arc::new(ClockRegistry::new()))
        .err()
        .unwrap()
        .message
        .starts_with("SE-9: "));
}

#[test]
fn se_10_time_authority_contract() {
    let (engine, clocks) = engine();
    let root = engine.root();
    let time = Authority::time(&engine);
    let derived = clocks.allocate_id().unwrap();
    clocks
        .register(ClockDomain::derived(
            derived,
            root,
            Rational::new(3, 1).unwrap(),
            1,
        ))
        .unwrap();
    let inexact = clocks.allocate_id().unwrap();
    clocks
        .register(ClockDomain::derived(
            inexact,
            root,
            Rational::new(2, 3).unwrap(),
            1,
        ))
        .unwrap();
    let unrelated = clocks.allocate_id().unwrap();
    clocks
        .register(ClockDomain::root(
            unrelated,
            Rational::ONE,
            EpochRef::Arbitrary {
                set_by: "test".to_owned(),
            },
        ))
        .unwrap();

    time.schedule(TimePoint::new(root, 7), Box::new(|_| {})).unwrap();
    assert_eq!(engine.next_wakeup(), Some(TimePoint::new(root, 7)));
    assert_eq!(time.now(root).unwrap(), TimePoint::new(root, 7));
    assert_eq!(
        time.now(ClockDomainId::HOST_MONOTONIC).unwrap(),
        TimePoint::new(ClockDomainId::HOST_MONOTONIC, 7)
    );
    assert_eq!(time.now(derived).unwrap(), TimePoint::new(derived, 2));
    assert_eq!(
        time.now(unrelated),
        Err(TimeError::NotGoverned { id: unrelated })
    );
    assert_eq!(
        time.schedule(TimePoint::new(root, 5), Box::new(|_| {})),
        Err(TimeError::InPast {
            now: TimePoint::new(root, 7),
            requested: TimePoint::new(root, 5),
        })
    );
    assert!(matches!(
        time.schedule(TimePoint::new(inexact, 1), Box::new(|_| {})),
        Err(TimeError::Inexact { floor }) if floor == TimePoint::new(root, 1)
    ));
    assert_eq!(
        time.schedule(TimePoint::new(unrelated, 8), Box::new(|_| {})),
        Err(TimeError::NotGoverned { id: unrelated })
    );
    time.wait_until(TimePoint::new(root, 7)).unwrap();
    assert_eq!(
        time.wait_until(TimePoint::new(unrelated, 7)),
        Err(TimeError::NotGoverned { id: unrelated })
    );

    let handle = time.schedule(TimePoint::new(root, 8), Box::new(|_| {})).unwrap();
    assert!(time.cancel(handle));
    assert!(!time.cancel(handle));
    assert_eq!(engine.next_wakeup(), None);
}

#[test]
fn se_10_cancel_does_not_remove_another_roots_callback() {
    let clocks = Arc::new(ClockRegistry::new());
    let a = SimEngine::new(clocks.clone()).unwrap();
    let b = SimEngine::new(clocks).unwrap();
    let fired = Arc::new(AtomicUsize::new(0));
    let flag = fired.clone();
    let foreign = a.time().schedule(TimePoint::new(a.root(), 10), Box::new(|_| {})).unwrap();
    let own = b.time().schedule(TimePoint::new(b.root(), 10),
        Box::new(move |_| { flag.fetch_add(1, Ordering::SeqCst); })).unwrap();
    assert_ne!(foreign.root, own.root);
    assert_eq!((foreign.ticks, foreign.seq), (own.ticks, own.seq));
    assert!(!b.time().cancel(foreign));
    assert_eq!(b.next_wakeup(), Some(TimePoint::new(b.root(), 10)));
    assert_eq!(fired.load(Ordering::SeqCst), 1);
    assert!(a.time().cancel(foreign), "the foreign callback was never removed");
    assert!(!b.time().cancel(own), "the own callback already fired");
}

#[test]
fn se_10_host_monotonic_advances_in_lockstep() {
    let (engine, _) = engine();
    let time = Authority::time(&engine);
    let root = engine.root();
    time.schedule(TimePoint::new(root, 1_000_000), Box::new(|_| {}))
        .unwrap();
    engine.next_wakeup();
    assert_eq!(
        time.now(ClockDomainId::HOST_MONOTONIC).unwrap().ticks_in(ClockDomainId::HOST_MONOTONIC).unwrap(),
        1_000_000
    );
}

#[test]
fn se_11_next_wakeup_order_ties_and_cap() {
    let (order_engine, _) = engine();
    let root = order_engine.root();
    let time = Authority::time(&order_engine);
    let log = Arc::new(Mutex::new(Vec::<(i64, &'static str)>::new()));

    let late_log = log.clone();
    time.schedule(
        TimePoint::new(root, 30),
        Box::new(move |at| late_log.lock().unwrap_or_else(|e| e.into_inner()).push((at.ticks_in(at.domain()).unwrap(), "late"))),
    )
    .unwrap();
    let first_log = log.clone();
    let nested_time = time.clone();
    let nested_log = log.clone();
    time.schedule(
        TimePoint::new(root, 10),
        Box::new(move |at| {
            first_log.lock().unwrap_or_else(|e| e.into_inner()).push((at.ticks_in(at.domain()).unwrap(), "first"));
            nested_time
                .schedule(
                    at,
                    Box::new(move |nested_at| {
                        nested_log.lock().unwrap_or_else(|e| e.into_inner()).push((nested_at.ticks_in(nested_at.domain()).unwrap(), "nested"));
                    }),
                )
                .unwrap();
        }),
    )
    .unwrap();
    let second_log = log.clone();
    time.schedule(
        TimePoint::new(root, 10),
        Box::new(move |at| second_log.lock().unwrap_or_else(|e| e.into_inner()).push((at.ticks_in(at.domain()).unwrap(), "second"))),
    )
    .unwrap();

    assert_eq!(order_engine.next_wakeup(), Some(TimePoint::new(root, 10)));
    assert_eq!(
        *log.lock().unwrap_or_else(|e| e.into_inner()),
        [(10, "first"), (10, "second"), (10, "nested")]
    );
    assert_eq!(order_engine.next_wakeup(), Some(TimePoint::new(root, 30)));
    assert_eq!(order_engine.next_wakeup(), None);

    fn self_rescheduling(
        time: Arc<dyn TimeAuthority>,
        calls: Arc<AtomicUsize>,
    ) -> Box<dyn FnOnce(TimePoint) + Send> {
        Box::new(move |at| {
            calls.fetch_add(1, Ordering::Relaxed);
            time.schedule(at, self_rescheduling(time.clone(), calls.clone()))
                .unwrap();
        })
    }

    let (engine, _) = engine();
    let root = engine.root();
    let time = Authority::time(&engine);
    let calls = Arc::new(AtomicUsize::new(0));
    time.schedule(
        TimePoint::new(root, 10),
        self_rescheduling(time.clone(), calls.clone()),
    )
    .unwrap();
    assert_eq!(engine.next_wakeup(), Some(TimePoint::new(root, 10)));
    assert_eq!(calls.load(Ordering::Relaxed), 1000);
    assert_eq!(engine.next_wakeup(), Some(TimePoint::new(root, 10)));
    assert_eq!(calls.load(Ordering::Relaxed), 2000);
}

#[test]
fn se_11_empty_is_none_and_moves_nothing() {
    let (engine, _) = engine();
    assert_eq!(engine.next_wakeup(), None);
    assert_eq!(
        Authority::time(&engine).now(engine.root()).unwrap(),
        TimePoint::new(engine.root(), 0)
    );
}

#[test]
fn se_13_two_engines_fed_the_same_schedule_fire_identically() {
    fn firing_log() -> Vec<(i64, usize)> {
        let (engine, _) = engine();
        let root = engine.root();
        let time = Authority::time(&engine);
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut rng = SimRng::new(1, "t");
        for sequence in 0..1_000 {
            let tick = rng.below(1_000) as i64;
            let log = log.clone();
            time.schedule(
                TimePoint::new(root, tick),
                Box::new(move |at| {
                    log.lock().unwrap_or_else(|e| e.into_inner()).push((at.ticks_in(at.domain()).unwrap(), sequence));
                }),
            )
            .unwrap();
        }
        while engine.next_wakeup().is_some() {}
        Arc::try_unwrap(log)
            .expect("all callback clones have run")
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
    }

    assert_eq!(firing_log(), firing_log());
}
