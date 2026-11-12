//! Fixtures for `mutate.py --e2e` (note 51 §6.4): each is a test that returns at once unless
//! `EZSDR_INFRA_FIXTURE` names it, so the default suite runs them as no-ops. The e2e driver
//! runs one at a time through the same `cargo_test()` a mutation row uses and checks the
//! verdict the runner draws from the output. They are not `#[ignore]`: that is the
//! hardware tests' mark.

use super::{Gated, Release, held_session};
use crate::common::*;
use ezsdr_kernel::spec::Value;
use ezsdr_radio_uhd::{Applied, Device, DeviceError, Dir, FakeConfig, FakeDevice, Iq, RxRecv, Settings, TxReport};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration as Wall;

fn selected(name: &str) -> bool {
    std::env::var("EZSDR_INFRA_FIXTURE").is_ok_and(|selected| selected == name)
}

/// 1: an assertion fails, with no marker.
#[test]
fn infra_fixture_assert_failure() {
    if selected("assert_failure") {
        assert_eq!(1 + 1, 3, "the fixture's own assertion");
    }
}

/// 2: the clock never moves, so the test never ends and the watchdog (`EZSDR_TEST_WATCHDOG_SECS=1`) aborts with its marker.
#[test]
fn infra_fixture_stopped_clock() {
    if selected("stopped_clock") {
        let device = fake(FakeConfig::default());
        device.host().sleep(Wall::from_secs(3600));
    }
}

/// 3: the Session is created on a device whose `prepare` takes longer than KC-12a's 5 s, so `connect` returns a Run that has ended.
#[test]
fn infra_fixture_creation_limit() {
    if selected("creation_limit") {
        let dir = TempDir::new();
        let device = fake_real(FakeConfig { apply_delay: Wall::from_secs(8), ..FakeConfig::default() });
        let _run = session(&profile(&dir, json!({}), json!({}), true), device);
        unreachable!("the Session cannot be created");
    }
}

/// 3b: the same limit on a Spec's Run, which `spec_run` (through `start_spec`) classifies instead of `session_on`.
#[test]
fn infra_fixture_creation_limit_spec() {
    if selected("creation_limit_spec") {
        let dir = TempDir::new();
        let device = fake_real(FakeConfig { apply_delay: Wall::from_secs(8), ..FakeConfig::default() });
        let _run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, Default::default());
        unreachable!("the Spec's Run cannot be created");
    }
}

/// 5: the process aborts with no marker.
#[test]
fn infra_fixture_abort_without_marker() {
    if selected("abort_without_marker") {
        std::process::abort();
    }
}

/// A timing precondition missed on every run.
#[test]
fn infra_fixture_timing_always() {
    if selected("timing_always") {
        timing("the fixture always misses its timing precondition");
        panic!("timing precondition missed");
    }
}

/// A timing precondition missed on the first run only: the state directory the driver passes (`EZSDR_INFRA_FIXTURE_STATE`) remembers it.
#[test]
fn infra_fixture_timing_once() {
    if selected("timing_once") {
        let seen = std::path::Path::new(&std::env::var("EZSDR_INFRA_FIXTURE_STATE").expect("the driver passes a state directory")).join("seen");
        if !seen.exists() {
            std::fs::write(&seen, b"").unwrap();
            timing("the fixture misses its timing precondition once");
            panic!("timing precondition missed");
        }
    }
}

/// A fixture's setup that does not reach the state the fixture needs says so with a marker: the run that follows would otherwise end in
/// the fixture's own sentinel assertion, which reads as a kill.
fn setup<T>(what: &str, step: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)).unwrap_or_else(|payload| {
        infra(&format!("fixture setup: {what} did not complete"));
        std::panic::resume_unwind(payload)
    })
}

/// A device whose `ref_locked` (uhd-control reads it every 500 ms) or `close_streams` (the Provider's cleanup) stalls in real time once armed.
/// The test waits for the stalled call to be entered (`entries`), so the fixture does not depend on when uhd-control's thread runs.
struct Stalls {
    inner: Arc<FakeDevice>,
    ref_locked: AtomicBool,
    close: AtomicBool,
    entered: Mutex<Sender<&'static str>>,
}

impl Stalls {
    fn new(inner: Arc<FakeDevice>) -> (Arc<Stalls>, Receiver<&'static str>) {
        let (entered, entries) = channel();
        (Arc::new(Stalls { inner, ref_locked: AtomicBool::new(false), close: AtomicBool::new(false), entered: Mutex::new(entered) }), entries)
    }

    fn stall(&self, armed: &AtomicBool, call: &'static str, seconds: u64) {
        if armed.load(Ordering::SeqCst) {
            let _ = self.entered.lock().unwrap().send(call);
            std::thread::sleep(Wall::from_secs(seconds));
        }
    }
}

/// Waits for the stalled call's entry; a bound of 5 s (uhd-control reads every 500 ms) that expires is a marked setup failure.
fn await_entry(entries: &Receiver<&'static str>, call: &str) {
    if entries.recv_timeout(Wall::from_secs(5)).map(|entered| entered == call) != Ok(true) {
        infra(&format!("fixture setup: {call} was not entered within 5 s"));
        panic!("fixture setup: {call} was not entered");
    }
}

impl Device for Stalls {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
        self.inner.fidelity()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> {
        self.inner.front_end(dir, chan)
    }
    fn serial(&self) -> Result<String, DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> {
        self.inner.set_subdev_spec(dir, markup)
    }
    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> {
        self.inner.rates(dir, chan)
    }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> {
        self.inner.set_sources(clock, time)
    }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> {
        self.inner.set_time_zero(at_next_pps)
    }
    fn time_now(&self) -> Result<i64, DeviceError> {
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> {
        self.stall(&self.ref_locked, "ref_locked", 8);
        self.inner.ref_locked()
    }
    fn apply(&self, dir: Dir, chan: usize, s: &Settings) -> Result<Applied, DeviceError> {
        self.inner.apply(dir, chan, s)
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.inner.rx_open(stream, channels)
    }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> {
        self.inner.rx_start(stream, at)
    }
    fn rx_packet_samples(&self, stream: usize) -> usize {
        self.inner.rx_packet_samples(stream)
    }
    fn rx_stop(&self, stream: usize) -> Result<(), DeviceError> {
        self.inner.rx_stop(stream)
    }
    fn rx_recv(&self, stream: usize, n: usize, timeout: Wall, one_packet: bool) -> RxRecv {
        self.inner.rx_recv(stream, n, timeout, one_packet)
    }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.inner.tx_open(stream, channels)
    }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, DeviceError> {
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: Wall) -> Result<Option<TxReport>, DeviceError> {
        self.inner.tx_async(stream, timeout)
    }
    fn rx_close(&self, stream: usize) {
        self.inner.rx_close(stream)
    }
    fn tx_close(&self, stream: usize) {
        self.inner.tx_close(stream)
    }
    fn close_streams(&self) {
        self.stall(&self.close, "close_streams", 7);
        self.inner.close_streams()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), DeviceError> {
        self.inner.raise_thread_priority()
    }
    fn host(&self) -> ezsdr_radio_uhd::HostClock {
        self.inner.host()
    }
}

/// 4: uhd-control blocks in its reference check, so a `submit`'s Action is never taken and KC-21a ends the Run inside the call. The marker comes from `submit`;
/// the assertion after it, which a Run that ended inside the call would otherwise fail, is never reached.
#[test]
fn infra_fixture_submit_limit() {
    if selected("submit_limit") {
        let dir = TempDir::new();
        let (stalls, entries) = Stalls::new(fake_real(FakeConfig::default()));
        let mut run = session(&profile(&dir, json!({ "clock_source": "external" }), json!({}), true), stalls.clone());
        stalls.ref_locked.store(true, Ordering::SeqCst);
        await_entry(&entries, "ref_locked");
        let _ = run.submit(set("radio.rx.c0.gain_db", Value::num(10.0).unwrap()), None);
        panic!("FIXTURE-4-AFTER: the assertion after the limit");
    }
}

/// 4b: a Spec whose `stop` at an offset reaches the agenda (only a Spec `stop` with a target does); its dispatch inside `advance_to` finds uhd-control blocked.
#[test]
fn infra_fixture_agenda_limit() {
    if selected("agenda_limit") {
        let dir = TempDir::new();
        let (stalls, entries) = Stalls::new(fake_real(FakeConfig::default()));
        let mut spec = receive_spec(1, 1e6, 1e9, None);
        spec["schedule"] = json!([{ "at": { "clock": "radio", "offset_ticks": 50_000 }, "action": { "kind": "stop", "target": serde_json::to_value(resource("radio")).unwrap() } }]);
        let mut run = spec_run(&spec, &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), stalls.clone(), BTreeMap::new());
        stalls.ref_locked.store(true, Ordering::SeqCst);
        await_entry(&entries, "ref_locked");
        let _ = run.advance_to(after(&run, ms(3_000)));
        panic!("FIXTURE-4B-AFTER: the assertion after the limit");
    }
}

/// 6: the Provider's cleanup takes longer than RS-8a's 5 s inside the test's own `finish()`, so the Manifest has no Provider section: the marker comes from `finish`,
/// before the read of the section that would fail with `no section`.
#[test]
fn infra_fixture_cleanup_limit() {
    if selected("cleanup_limit") {
        let dir = TempDir::new();
        let (stalls, _) = Stalls::new(fake(FakeConfig::default()));
        let mut run = session(&profile(&dir, json!({}), json!({}), true), stalls.clone());
        past_t0(&mut run, ms(5));
        stalls.close.store(true, Ordering::SeqCst);
        let manifest = run.finish();
        let _ = section(&manifest, "FIXTURE-6-AFTER-bursts");
    }
}

/// 7: the Run ends by the Policy's CLOCK_LOST, which the test declares, and the cleanup then takes longer than RS-8a's 5 s: the declaration of an end
/// does not exempt the bound. (A lost device skips `close_streams`, so the end is CLOCK_LOST, not DEVICE_LOST.)
#[test]
fn infra_fixture_declared_end_cleanup_limit() {
    if selected("declared_end_cleanup_limit") {
        let dir = TempDir::new();
        let (stalls, _) = Stalls::new(fake(FakeConfig { faults: vec![ezsdr_radio_uhd::FakeFault::Unlocked(Wall::from_millis(2_300))], ..FakeConfig::default() }));
        let mut run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({ "clock_source": "external" }), json!({}), false), stalls.clone(), BTreeMap::new())
            .expecting(Expect::Policy("radio.CLOCK_LOST"));
        stalls.close.store(true, Ordering::SeqCst);
        let _ = run.run_until_end(after(&run, ms(5_000)));
        let manifest = run.finish();
        let _ = section(&manifest, "FIXTURE-7-AFTER-bursts");
    }
}

/// 8: both Provider threads are held inside a device call, so the Provider's join in the test's `finish()` expires (the J317/J318 mechanism); the marker comes
/// from `finish`, and no declaration of the test exempts it.
#[test]
fn infra_fixture_join_at_finish() {
    if selected("join_at_finish") {
        let device = Arc::new(Gated::new(FakeConfig::default()));
        let _release = Release(device.clone());
        let (run, _dir) = setup("holding the Provider's threads in a device call", || held_session(&device));
        let _ = run.finish();
        unreachable!("the Run cannot be finished inside the Provider's join");
    }
}
