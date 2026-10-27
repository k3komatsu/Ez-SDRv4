use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::module_api::Fidelity;
use ezsdr_radio_uhd::{Applied, Device, DeviceError, Dir, HostClock, Iq, RxRecv, Settings, TxReport};
use serde_json::{Value as Json, json};

struct ReceiveGate {
    paused: bool,
    active: usize,
}

/// Test-only Device forwarder for recording Session RX ordering on the host clock.
pub struct ProbeDevice {
    inner: Arc<dyn Device>,
    origin: Instant,
    rows: Mutex<Vec<Json>>,
    tx_eob_count: AtomicUsize,
    gate: Mutex<ReceiveGate>,
    ready: Condvar,
}

struct ResumeReaderOnDrop<'a>(&'a ProbeDevice);

impl Drop for ResumeReaderOnDrop<'_> {
    fn drop(&mut self) { self.0.resume_reader(); }
}

pub fn successful_tx_eob(row: &Json) -> bool {
    if !(row["call"] == "tx_send" && row["eob"].as_bool() == Some(true)) { return false; }
    let Some(accepted) = row["result"]["accepted_samples"].as_u64() else { return false; };
    row["sample_counts"].as_array().is_some_and(|counts| {
        !counts.is_empty() && counts.iter().all(|count| count.as_u64() == Some(accepted))
    })
}

pub fn receive_sample_follows_start(start: &Json, sample: &Json) -> bool {
    start["call"] == "rx_start" && start["result"].is_null()
        && sample["call"] == "rx_recv" && sample["outcome"]["kind"] == "samples"
        && start["host_after_ns"].as_u64().zip(sample["host_before_ns"].as_u64()).is_some_and(|(finished, begun)| begun >= finished)
        && start["at"].as_i64().zip(sample["outcome"]["first_tick"].as_i64()).is_some_and(|(at, first)| first >= at)
}

impl ProbeDevice {
    pub fn new(inner: Arc<dyn Device>) -> ProbeDevice {
        ProbeDevice {
            inner,
            origin: Instant::now(),
            rows: Mutex::new(Vec::new()),
            tx_eob_count: AtomicUsize::new(0),
            gate: Mutex::new(ReceiveGate { paused: false, active: 0 }),
            ready: Condvar::new(),
        }
    }

    pub fn host_ns(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    pub fn rows(&self) -> Vec<Json> {
        self.rows.lock().unwrap().clone()
    }

    pub fn count(&self, call: &str) -> usize {
        self.rows.lock().unwrap().iter().filter(|row| row["call"] == call).count()
    }

    pub fn tx_eob_count(&self) -> usize {
        self.tx_eob_count.load(Ordering::Acquire)
    }

    pub fn inner(&self) -> &dyn Device {
        &*self.inner
    }

    pub fn pause_reader(&self) {
        let mut gate = self.gate.lock().unwrap();
        gate.paused = true;
        while gate.active != 0 { gate = self.ready.wait(gate).unwrap(); }
    }

    pub fn resume_reader(&self) {
        self.gate.lock().unwrap().paused = false;
        self.ready.notify_all();
    }

    /// Pauses the Provider's reader, drains the device streamer, then resumes it.
    pub fn drain_without_reader(&self, stream: usize, request: usize, timeout: Wall, limit: Wall) -> Vec<Json> {
        self.pause_reader();
        let _resume = ResumeReaderOnDrop(self);

        let begun = Instant::now();
        let mut idle = 0;
        let mut drained = Vec::new();
        while begun.elapsed() < limit && idle < 2 {
            let before = self.host_ns();
            let packet_samples = self.inner.rx_packet_samples(stream);
            let result = self.inner.rx_recv(stream, request, timeout);
            let after = self.host_ns();
            let outcome = rx_outcome(&result);
            if outcome["kind"] == "timeout" { idle += 1; } else { idle = 0; }
            let row = json!({
                "call": "device_drain", "stream": stream, "request_samples": request,
                "packet_samples": packet_samples, "timeout_ns": timeout.as_nanos(),
                "host_before_ns": before, "host_after_ns": after,
                "outcome": outcome,
            });
            self.rows.lock().unwrap().push(row.clone());
            drained.push(row);
        }
        drained
    }

    fn record(&self, row: Json) {
        let eob = row["call"] == "tx_send" && row["eob"].as_bool().unwrap_or(false);
        self.rows.lock().unwrap().push(row);
        if eob { self.tx_eob_count.fetch_add(1, Ordering::Release); }
    }
}

fn rx_outcome(result: &RxRecv) -> Json {
    match result {
        RxRecv::Samples { first_tick, samples } => json!({
            "kind": "samples", "first_tick": first_tick,
            "samples": samples.first().map_or(0, Vec::len),
        }),
        RxRecv::Overflow { out_of_sequence } => json!({ "kind": "overflow", "out_of_sequence": out_of_sequence }),
        RxRecv::LateCommand => json!({ "kind": "late_command" }),
        RxRecv::Alignment => json!({ "kind": "alignment" }),
        RxRecv::BadPacket => json!({ "kind": "bad_packet" }),
        RxRecv::Timeout => json!({ "kind": "timeout" }),
        RxRecv::Failed(error) => json!({ "kind": "failed", "lost": error.lost, "message": error.message }),
    }
}

impl Device for ProbeDevice {
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> Fidelity { self.inner.fidelity() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> { self.inner.front_end(dir, chan) }
    fn serial(&self) -> Result<String, DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> { self.inner.set_subdev_spec(dir, markup) }
    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> { self.inner.rates(dir, chan) }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> { self.inner.set_sources(clock, time) }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> { self.inner.set_time_zero(at_next_pps) }
    fn time_now(&self) -> Result<i64, DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> { self.inner.ref_locked() }
    fn apply(&self, dir: Dir, chan: usize, settings: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> { self.inner.apply(dir, chan, settings, at) }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.rx_open(stream, channels) }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> {
        let before = self.host_ns();
        let result = self.inner.rx_start(stream, at);
        let after = self.host_ns();
        self.record(json!({ "call": "rx_start", "stream": stream, "at": at, "host_before_ns": before, "host_after_ns": after, "result": result.as_ref().err().map(|error| error.message.clone()) }));
        result
    }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> {
        let before = self.host_ns();
        let result = self.inner.rx_stop(stream, at);
        let after = self.host_ns();
        self.record(json!({ "call": "rx_stop", "stream": stream, "at": at, "host_before_ns": before, "host_after_ns": after, "result": result.as_ref().err().map(|error| error.message.clone()) }));
        result
    }
    fn rx_recv(&self, stream: usize, n: usize, timeout: Wall) -> RxRecv {
        {
            let mut gate = self.gate.lock().unwrap();
            while gate.paused { gate = self.ready.wait(gate).unwrap(); }
            gate.active += 1;
        }
        let before = self.host_ns();
        let packet_samples = self.inner.rx_packet_samples(stream);
        let result = self.inner.rx_recv(stream, n, timeout);
        let after = self.host_ns();
        self.record(json!({
            "call": "rx_recv", "stream": stream, "request_samples": n, "timeout_ns": timeout.as_nanos(),
            "packet_samples": packet_samples, "host_before_ns": before, "host_after_ns": after,
            "outcome": rx_outcome(&result),
        }));
        let mut gate = self.gate.lock().unwrap();
        gate.active -= 1;
        self.ready.notify_all();
        result
    }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.tx_open(stream, channels) }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, DeviceError> {
        let before = self.host_ns();
        let result = self.inner.tx_send(stream, samples, at, sob, eob, timeout);
        let after = self.host_ns();
        let result_row = match &result {
            Ok(accepted_samples) => json!({ "accepted_samples": accepted_samples }),
            Err(error) => json!({ "error": error.message }),
        };
        self.record(json!({
            "call": "tx_send", "stream": stream, "host_before_ns": before, "host_after_ns": after,
            "sample_counts": samples.iter().map(|channel| channel.len()).collect::<Vec<_>>(),
            "at": at, "sob": sob, "eob": eob, "timeout_ns": timeout.as_nanos(), "result": result_row,
        }));
        result
    }
    fn tx_async(&self, stream: usize, timeout: Wall) -> Result<Option<TxReport>, DeviceError> { self.inner.tx_async(stream, timeout) }
    fn rx_close(&self, stream: usize) { self.inner.rx_close(stream) }
    fn tx_close(&self, stream: usize) { self.inner.tx_close(stream) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn host(&self) -> HostClock { self.inner.host() }
    fn mark_lost(&self) { self.inner.mark_lost() }
}

#[cfg(all(test, not(feature = "uhd")))]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn successful_tx_eob_requires_an_accepted_eob() {
        assert!(successful_tx_eob(&json!({"call":"tx_send","eob":true,"sample_counts":[1],"result":{"accepted_samples":1}})));
        assert!(!successful_tx_eob(&json!({"call":"tx_send","eob":true,"sample_counts":[1],"result":{"accepted_samples":0}})));
        assert!(!successful_tx_eob(&json!({"call":"tx_send","eob":true,"sample_counts":[100],"result":{"accepted_samples":99}})));
        assert!(!successful_tx_eob(&json!({"call":"tx_send","eob":false,"result":{"accepted_samples":100}})));
        assert!(!successful_tx_eob(&json!({"call":"tx_send","eob":true,"result":{"error":"device error"}})));
        assert!(!successful_tx_eob(&json!({"call":"rx_start","eob":true,"result":{"accepted_samples":0}})));
    }

    #[test]
    fn tx_eob_count_tracks_recorded_eob_sends() {
        let inner = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig::default()));
        let probe = ProbeDevice::new(inner);
        probe.record(json!({"call":"tx_send","eob":false}));
        probe.record(json!({"call":"rx_stop","eob":true}));
        assert_eq!(probe.tx_eob_count(), 0);
        probe.record(json!({"call":"tx_send","eob":true}));
        assert_eq!(probe.tx_eob_count(), 1);
        assert_eq!(probe.rows().len(), 3);
    }

    #[test]
    fn receive_sample_follows_a_successful_device_start() {
        let start = json!({"call":"rx_start","at":100,"host_after_ns":10,"result":null});
        let sample = json!({"call":"rx_recv","host_before_ns":10,"outcome":{"kind":"samples","first_tick":100}});
        assert!(receive_sample_follows_start(&start, &sample));

        let mut stale_host_sample = sample.clone();
        stale_host_sample["host_before_ns"] = json!(9);
        assert!(!receive_sample_follows_start(&start, &stale_host_sample));
        let mut stale_tick_sample = sample.clone();
        stale_tick_sample["outcome"]["first_tick"] = json!(99);
        assert!(!receive_sample_follows_start(&start, &stale_tick_sample));
        let mut failed_start = start.clone();
        failed_start["result"] = json!("start failed");
        assert!(!receive_sample_follows_start(&failed_start, &sample));
    }

    #[test]
    fn drain_without_reader_resumes_after_rx_panic() {
        let inner = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig {
            clock: ezsdr_radio_uhd::ClockMode::Manual,
            faults: vec![ezsdr_radio_uhd::FakeFault::RxPanic(Wall::ZERO)],
            ..ezsdr_radio_uhd::FakeConfig::default()
        }));
        inner.rx_open(0, &[0]).unwrap();
        let at = inner.time_now().unwrap() + inner.master_clock_rate() as i64;
        inner.rx_start(0, at).unwrap();
        let probe = ProbeDevice::new(inner);
        let panic = catch_unwind(AssertUnwindSafe(|| probe.drain_without_reader(0, 2_000, Wall::from_millis(1), Wall::from_millis(10))));
        assert!(panic.is_err());
        assert!(!probe.gate.lock().unwrap().paused);
    }
}
