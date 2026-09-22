//! `04-run-and-session.md` RS-32: emitting on the hot path allocates nothing.
//!
//! In its own test binary, because the counting allocator is global and any other
//! test allocating inside the measurement window would inflate the count.

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ezsdr_kernel::event::{
    EventCollector, EventKind, EventRecord, EventSink, HOT_PAYLOAD_BYTES, Severity,
};
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::policy::{EventKindRegistry, Reaction};
use ezsdr_kernel::run::RunError;
use ezsdr_kernel::time::TimePoint;
use support::rid;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn rs_32_emit_allocates_nothing() {
    let kinds = EventKindRegistry::with_kernel_kinds();
    let policy = kinds.compile(&BTreeMap::new()).expect("compiles");
    let source = rid("radio");
    let kind = EventKind::parse(EventKind::LINK_BACKPRESSURE).expect("parses");
    // A ring deep enough that the drop path is not the only one exercised.
    let c = EventCollector::new(
        &[(source.clone(), kind.clone())],
        &policy.table.keys().cloned().collect::<Vec<_>>(),
        1_024,
        &policy,
    );
    let handle = c.resolve(&source, &kind);
    let payload = [7u8; 16];
    let at = TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0);

    ALLOCS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    for _ in 0..100_000 {
        c.emit(handle, at, Severity::Warning, &payload).expect("emits");
    }
    COUNTING.store(false, Ordering::Relaxed);
    assert_eq!(ALLOCS.load(Ordering::Relaxed), 0, "the hot path allocates nothing");

    // The record's size equals the documented constant.
    assert_eq!(HOT_PAYLOAD_BYTES, 32);
    assert!(
        size_of::<EventRecord>() <= 64,
        "an EventRecord is fixed-size and small, not {} bytes",
        size_of::<EventRecord>()
    );
    // A larger payload is produced on the control path only.
    assert_eq!(
        c.emit(handle, at, Severity::Warning, &[0u8; HOT_PAYLOAD_BYTES + 1]),
        Err(RunError::PayloadTooLarge)
    );
    assert_eq!(policy.reaction_for(&kind), Reaction::MarkArtifact);
}
