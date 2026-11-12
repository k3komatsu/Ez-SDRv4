//! What the two driving rigs share (note 51 stage 1c): `tests/fake.rs`'s `Direct` and `crates/ezsdr-acceptance`'s `UhdRig` (which includes this file with `#[path]`, as it does `infra.rs`).
#![allow(dead_code)]

use ezsdr_kernel::time::TimeAuthority;
use ezsdr_radio_uhd::{DeviceAuthority, HostClock};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// The infra marker: `common` re-exports the function, and the including test file imports it beside its own `mod infra`.
use super::infra;

/// A rig's Action queue: each entry is stamped with the device clock at its push, and `take` withholds it until the clock is strictly later. A rig thread that is a participant of the clock
/// stands it still while it works, so the stamp is an exact instant; an entry is then received at the first poll strictly after the instant it was pushed, whichever order the pusher and the
/// poll woke in at an instant they share (the rig's waits end on one, M4 gives no order there). Without a clock (the Mock's rig) every entry is ready.
pub struct Stamped<T> {
    clock: Option<HostClock>,
    entries: Mutex<VecDeque<(Option<Duration>, T)>>,
}

impl<T> Stamped<T> {
    pub fn new(clock: Option<HostClock>) -> Stamped<T> {
        Stamped { clock, entries: Mutex::new(VecDeque::new()) }
    }

    pub fn push(&self, entry: T) {
        let stamp = self.clock.as_ref().map(HostClock::now);
        self.entries.lock().unwrap().push_back((stamp, entry));
    }

    /// Every entry of `entries` at one instant, under one lock: nothing takes a part of them.
    pub fn push_all(&self, entries: impl IntoIterator<Item = T>) {
        let stamp = self.clock.as_ref().map(HostClock::now);
        self.entries.lock().unwrap().extend(entries.into_iter().map(|entry| (stamp, entry)));
    }

    /// The oldest entry, if the clock has moved since its push.
    pub fn take(&self) -> Option<T> {
        let now = self.clock.as_ref().map(HostClock::now);
        let mut entries = self.entries.lock().unwrap();
        match entries.front() {
            Some((Some(stamp), _)) if now.is_some_and(|now| *stamp >= now) => None,
            _ => entries.pop_front().map(|(_, entry)| entry),
        }
    }

    /// The entries not taken yet.
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

/// The Authority and, in the same field so that no pause between two field drops can reorder them, the probe a test may install: the probe reads what the Authority's drop left behind.
/// A rig declares it as its last field, behind its `_driver` (the first), so that the driver drops before the Authority: `DeviceAuthority::drop` waits in real time for `uhd-clock`, and a
/// thread still registered (busy) would hold the clock that `uhd-clock` may be asleep on (note 51 §4.2).
pub struct AuthorityGuard {
    authority: Option<DeviceAuthority>,
    probe: Option<DropProbe>,
}

impl AuthorityGuard {
    pub fn new(authority: DeviceAuthority) -> AuthorityGuard {
        AuthorityGuard { authority: Some(authority), probe: None }
    }

    /// What the rig's drop left, read by the probe: set it, read the device clock, drop the rig (or let it unwind), then `assert_joined`.
    pub fn install_probe(&mut self, time: &Arc<dyn TimeAuthority>, host: HostClock) -> Arc<Mutex<Option<(usize, Duration)>>> {
        let seen = Arc::new(Mutex::new(None));
        self.probe = Some(DropProbe { time: Arc::downgrade(time), host, seen: seen.clone() });
        seen
    }
}

impl Drop for AuthorityGuard {
    fn drop(&mut self) {
        drop(self.authority.take());
        if let Some(probe) = self.probe.take() {
            probe.record();
        }
    }
}

/// What the probe saw once the Authority was gone: how many strong references to its time were left (`uhd-clock`'s upgraded `Arc`, if the Authority gave up on the thread), and the device clock then.
struct DropProbe {
    time: Weak<dyn TimeAuthority>,
    host: HostClock,
    seen: Arc<Mutex<Option<(usize, Duration)>>>,
}

impl DropProbe {
    fn record(self) {
        *self.seen.lock().unwrap() = Some((self.time.strong_count(), self.host.now()));
    }
}

/// The device clock does not move while the test thread works: the wall sleep is the stimulus, the clock's reading the observation.
pub fn assert_stands_still(host: &HostClock, what: &str) {
    let at = host.now();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(host.now(), at, "{what}: the device clock moved while the test thread worked");
}

/// When the unreachable fault of the drop tests starts, in device time: so late that the constructor, which runs before the thread drives the clock, cannot reach it unless the host
/// stalled for that long (the rig then writes the marker).
pub const FAULT_AT: Duration = Duration::from_secs(2);

/// Drives `rig`, built on a device with `FakeFault::Unreachable(FAULT_AT)`, until its uhd-clock sleeps inside its read: from `FAULT_AT` `time_now` is a 100 ms clock sleep, and uhd-clock
/// holds an upgraded `Arc` of the Authority's time (`refs` counts it) for the read. The test thread is a busy driver, so the clock stands still and the state holds until the next wait.
/// `idle`, the count with no extra `Arc`, is taken before the fault, from the smaller of two readings one clock millisecond apart: uhd-clock holds the `Arc` across a read only while busy
/// at an instant, and the clock does not advance while it is. The state is told from a transient (uhd-clock busy at its deadline) by two readings above `idle` one clock millisecond apart.
pub fn asleep_inside_its_read<R>(mut rig: R, host: &HostClock, refs: impl Fn(&R) -> usize, mut settle: impl FnMut(&mut R, Duration)) -> R {
    let before_the_fault = |when: &str| {
        if host.now() >= FAULT_AT {
            infra(&format!("the rig's device time reached the fault ({FAULT_AT:?}) {when}: the host stalled"));
            panic!("the device time reached the fault {when}");
        }
    };
    before_the_fault("before its first reading");
    let first = refs(&rig);
    settle(&mut rig, Duration::from_millis(1));
    before_the_fault("before its second reading");
    let idle = first.min(refs(&rig));
    settle(&mut rig, FAULT_AT - host.now());
    let mut above = 0;
    for _ in 0..600 {
        settle(&mut rig, Duration::from_millis(1));
        above = if refs(&rig) > idle { above + 1 } else { 0 };
        if above == 2 {
            return rig;
        }
    }
    panic!("uhd-clock never slept inside its read of the unreachable device")
}

/// The ordering fact: with the driver dropped first, the clock runs while the Authority drops, uhd-clock's read ends and nothing is left. If the Authority dropped while the driver
/// still held the clock, the read could not end and the clock stood still across the drop (`before` is the clock just before it). The one other way to be left with a reference is a
/// host that did not run uhd-clock for longer than a read lasts in device time while the clock ran: that is the marker, not a verdict.
pub fn assert_joined(seen: &Mutex<Option<(usize, Duration)>>, before: Duration, what: &str) {
    let (left, after) = seen.lock().unwrap().expect("the probe did not run");
    if left == 0 {
        return;
    }
    let advanced = after.saturating_sub(before);
    // One reference left while the clock moved is uhd-clock starved past the join window (a host stall); a driver that held the clock leaves the clock where it was (N51B-03, N51C-02), and a
    // probe that reads before the Authority drops sees two references (N51B-09, N51C-10).
    if left == 1 && advanced > Duration::ZERO {
        infra(&format!("{what}: uhd-clock was not joined although the device clock advanced {advanced:?} across the drop"));
    }
    panic!("{what}: {left} reference(s) to the Authority's time were left and the device clock advanced {advanced:?} across the drop: the Authority dropped while the driver still held the clock");
}

#[test]
fn rig_an_entry_is_taken_only_at_a_later_clock_instant_than_its_push() {
    let clock = HostClock::manual();
    let _driver = clock.drive();
    let queue = Stamped::new(Some(clock.clone()));
    queue.push(1);
    queue.push(2);
    assert_eq!((queue.take(), queue.len()), (None, 2), "the driver is busy, so the clock stands at the instant of both pushes");
    clock.sleep(Duration::from_micros(1));
    queue.push(3);
    assert_eq!((queue.take(), queue.take(), queue.take()), (Some(1), Some(2), None), "the two earlier entries in order; the one pushed at the new instant waits for a later one");
    clock.sleep(Duration::from_micros(1));
    assert_eq!(queue.take(), Some(3));
}

#[test]
fn rig_an_entry_with_no_clock_is_ready_at_once() {
    let queue = Stamped::new(None);
    queue.push("a");
    assert_eq!(queue.take(), Some("a"));
}
