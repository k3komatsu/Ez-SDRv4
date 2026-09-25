//! Smoke test for Phase 2 coordinator doubles (spec 06 §3).

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use ezsdr_kernel::event::EventCollector;
use ezsdr_kernel::id::RunId;
use ezsdr_kernel::module_api::{
    Authority, ExecutionClass, Pacing, PrepareContext, Provider, StopMode,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::Policy;
use ezsdr_kernel::time::{ClockRegistry, Duration, RelativeBudget, TimePoint};
use support::{
    Probe, QueueReceiver, SimAuthority, SteppedProvider, TestProvider, TestSubmitter, id, mref,
};

#[test]
fn ma_44_the_run_doubles_record_their_calls() {
    let probe = Probe::new();
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, root) =
        SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    let time = authority.time();
    let events = Arc::new(EventCollector::new(&[], &[], 4096, &Policy::default()));
    let queue = Arc::new(QueueReceiver::new());
    let submitter = Arc::new(TestSubmitter::new());
    let host_budget = RelativeBudget::new(Duration::new(
        ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC,
        1_000_000_000,
    ))
    .expect("a host-monotonic test budget");
    let context = PrepareContext {
        run: RunId::from_string("test-run".to_owned()),
        class: ExecutionClass::Simulation,
        time,
        clocks,
        events,
        actions: queue,
        actions_out: submitter,
        environment: Arc::new(BTreeMap::new()),
        links: Vec::new(),
        components: BTreeMap::new(),
        host_budget,
    };
    let fragment = Fragment {
        id: id("radio"),
        instance: mref("ezsdr.test.provider"),
        role: ezsdr_kernel::module_api::Role::Provider,
        content: serde_json::json!({}),
        after: Vec::new(),
    };
    let t0 = TimePoint::new(root, 0);
    let mut provider = SteppedProvider::new("p", TestProvider::new("radio", 2), &probe);
    provider.prepare(&fragment, context).expect("prepares");
    provider.arm().expect("arms");
    provider.start(Some(t0)).expect("starts");
    provider.step(t0).expect("steps");
    provider.stop(StopMode::Orderly).expect("stops");
    provider.cleanup();

    assert_eq!(
        probe.lines(),
        [
            "p:prepare:radio",
            "p:arm",
            "p:start:0",
            "p:step:0",
            "p:now:0",
            "p:stop:Orderly",
            "p:cleanup",
        ]
    );
}
