//! The new runtime for E2b: E2a's brain, `S` and `R_0` over the same fake (behind the same timing decorator), a release timer that
//! delivers each value to the brain at `e - window`, and a receive sink that does today's per-block publication work.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use ezsdr_hostmem::{HOST_MEMORY, HostPool, write_cf32};
use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::Link;
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, ChannelMask, DataLink, DataLinkDecl, Direction, SampleBlock};
use ezsdr_kernel::time::TimePoint;
use serde_json::{Value as Json, json};

use super::runs::{Cond, Opts, Rig};
use super::{Bench, CELL_HOLD, CELL_WAIT, FREQS, Offset, R_CELL_HOLD, R_CELL_WAIT, REASONS, ReserveAgg, Tap, Timing, Upd, WINDOW_NS, dist};
use crate::brain::cell::Phase;
use crate::brain::core::{Held, Input};
use crate::brain::fixtures::{cell, seg};
use crate::brain::force::Actor;
use crate::brain::shared::Shared;
use crate::brain::threads::Runtime;
use crate::brain::world::{Params, World};
use crate::device::{Device, Dir, FakeConfig, FakeDevice, FakeFault, RxRecv, Settings};
use crate::provider::core::RxEntry;

/// The release timer: a thread that sleeps until the earliest pending release and is woken when a new one is booked (a condition variable
/// with a timeout at the deadline, not a fixed tick). It never delivers early; its lateness is inside the measured delay.
/// The pending releases by `(deadline ns, value id)`, and whether the timer is told to stop.
type Pending = (BTreeMap<(i64, u64), Held>, bool);

struct RelTimer {
    state: Mutex<Pending>,
    cv: Condvar,
    /// `(id, deadline, host ns just before the send)`: the diagnostic release stamps.
    stamps: Mutex<Vec<(u64, i64, i64)>>,
}

impl RelTimer {
    fn book(&self, deadline_ns: i64, held: Held) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.0.insert((deadline_ns, held.entry.order.1), held);
        self.cv.notify_one();
    }

    fn run(&self, timing: &Timing, sh: &Arc<Shared>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if state.1 {
                return;
            }
            let Some((&(deadline, id), _)) = state.0.first_key_value() else {
                state = self.cv.wait(state).unwrap_or_else(|e| e.into_inner());
                continue;
            };
            let now = timing.ns();
            if now < deadline {
                state = self.cv.wait_timeout(state, Duration::from_nanos((deadline - now) as u64)).unwrap_or_else(|e| e.into_inner()).0;
                continue;
            }
            let held = state.0.remove(&(deadline, id)).expect("the earliest release");
            drop(state);
            self.stamps.lock().unwrap_or_else(|e| e.into_inner()).push((id, deadline, timing.ns()));
            sh.send(Input::Release(held));
            state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        }
    }
}

pub(super) struct New {
    rt: Runtime,
    fake: Arc<FakeDevice>,
    timing: Arc<Timing>,
    link: Arc<Tap>,
    off: Offset,
    timer: Arc<RelTimer>,
    timer_thread: Option<std::thread::JoinHandle<()>>,
    bench: Arc<Bench>,
    counters: bool,
    published_blocks: Arc<AtomicU64>,
    published_samples: Arc<AtomicU64>,
}

pub(super) fn start(cond: &Cond, opts: &Opts) -> Box<dyn Rig> {
    Box::new(start_new(cond, opts))
}

fn start_new(cond: &Cond, opts: &Opts) -> New {
    // The same fake as today's owner (`FakeConfig::default`, the default master clock): a sample is `decimation` device ticks long, and `R` turns a block
    // into ticks at that step.
    let faults = opts.fault_ms.map(|ms| vec![FakeFault::Lost(Duration::from_millis(ms))]).unwrap_or_default();
    let fake = Arc::new(FakeDevice::new(FakeConfig { faults, ..FakeConfig::default() }));
    let mcr = fake.master_clock_rate();
    let sample_ticks = (mcr as f64 / cond.rate_hz).round() as i64;
    let timing = Timing::new(fake.clone(), cond.count + 1024, opts.counters);
    timing.counts.tps.store(sample_ticks, SeqCst);
    fake.set_time_zero(false).unwrap();
    let off = Offset::establish(&fake, mcr);
    fake.apply(Dir::Rx, 0, &Settings { rate: Some(cond.rate_hz), freq: Some(FREQS[0]), ..Settings::default() }, None).unwrap();
    fake.rx_open(0, &[0]).unwrap();
    let origin = fake.time_now().unwrap() + mcr as i64 / 50;
    fake.rx_start(0, origin).unwrap();

    let decl = DataLinkDecl {
        id: ezsdr_kernel::id::DataLinkId::local(0),
        from: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("radio").unwrap(), port: ezsdr_kernel::spec::Ident::parse("rx0").unwrap() },
        to: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("rec").unwrap(), port: ezsdr_kernel::spec::Ident::parse("in").unwrap() },
        contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        policy: BackPressure::DropOldest,
        capacity: super::LINK_CAPACITY,
    };
    let link = Tap::new(ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap());
    // The per-block publication work of today's `uhd-rx` (provider/rx.rs `samples`): the cf32 conversion into a pooled buffer, a `SampleBlock`,
    // an `Arc`, and the publish to the link. Today's owner also takes its locks and bumps two statistics per block (not reproduced here).
    let pool = Mutex::new(HostPool::new(8 * cond.block_len));
    let published_blocks = Arc::new(AtomicU64::new(0));
    let published_samples = Arc::new(AtomicU64::new(0));
    let (sink_link, counted, counted_samples) = (link.clone(), published_blocks.clone(), published_samples.clone());
    // The block header counts samples from the stream's origin, as today's sample clock does (the reader's gap test reads it so).
    let sink = Box::new(move |result: &RxRecv, range: std::ops::Range<usize>| {
        let RxRecv::Samples { first_tick, samples } = result else { return };
        let n = range.len();
        let first_tick = *first_tick + range.start as i64 * sample_ticks;
        let bytes = pool.lock().unwrap_or_else(|e| e.into_inner()).fill(n * 8, |buffer| {
            for (i, sample) in samples[0][range.clone()].iter().enumerate() {
                write_cf32(buffer, n, 0, i, sample[0], sample[1]);
            }
        });
        let header = BlockHeader {
            first_sample_time: TimePoint::new(ClockDomainId::HOST_MONOTONIC, (first_tick - origin) / sample_ticks),
            len: n as u32,
            channels: 1,
            direction: Direction::Rx,
            valid: ChannelMask::full(1),
            flags: BlockFlags::NONE,
            lost: None,
            contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32"),
        };
        if let Ok(block) = SampleBlock::new_host(header, HOST_MEMORY, bytes, 8) {
            counted.fetch_add(1, SeqCst);
            counted_samples.fetch_add(n as u64, SeqCst);
            sink_link.publish(Arc::new(block));
        }
    });
    let bench = Arc::new(Bench {
        sink,
        counters: opts.counters,
        publish_retries: AtomicU64::new(0),
        stale_rounds: AtomicU64::new(0),
        publishes: AtomicU64::new(0),
        take_dropped: AtomicU64::new(0),
        discarded_before_origin: AtomicU64::new(0),
        discarded_after_cut: AtomicU64::new(0),
        reserve: Mutex::new(vec![ReserveAgg::default(); cond.count + 16]),
        reserve_dropped: AtomicU64::new(0),
        stamps: (opts.stages || opts.sample).then(|| (0..cond.count + 16).map(|_| [AtomicI64::new(0), AtomicI64::new(0), AtomicI64::new(0), AtomicI64::new(0)]).collect()),
        bound_rejections: AtomicU64::new(0),
        occupancy: AtomicUsize::new(0),
        retired: AtomicU64::new(0),
        last_in_flight: Mutex::new(Vec::new()),
        clock: timing.clone(),
    });

    let params = Params {
        horizon: 0,
        epilogue: 0,
        recv_timeout: 100,
        queue_cap: 1,
        fifo_cap: 32,
        drain_window: mcr as i64 / 200,
        packet: cond.block_len,
        call_faults: false,
        panic_step: None,
        k: crate::brain::core::DEVICE_BOUND,
        d: crate::brain::core::CHAIN_BOUND,
    };
    let mut world = World::new(vec![cell(Phase::Streaming(seg(origin)), 0, None, 0)], Vec::new(), params);
    crate::brain::cases::running(&mut world, 0, vec![0]);
    if opts.k != 0 {
        world.brain.device_bound = opts.k;
    }
    if opts.d != 0 {
        world.brain.chain_bound = opts.d;
    }
    // The time source is the extrapolation of the host clock that the map already is (a pure read, no device lock), as today's `core.now()`
    // extrapolates the Authority's re-reads; `FakeDevice::time_now` would take the fake's one state mutex four times per block.
    let host = fake.host();
    let now: Box<dyn Fn() -> i64 + Send + Sync> = Box::new(move || off.tick_floor(host.now().as_nanos() as i64));
    let rt = Runtime::start_measured(&world, fake.clone(), timing.clone(), now, bench.clone(), sample_ticks);
    let timer = Arc::new(RelTimer { state: Mutex::new((BTreeMap::new(), false)), cv: Condvar::new(), stamps: Mutex::new(Vec::with_capacity(cond.count + 16)) });
    let timer_thread = {
        let (timer, timing, sh) = (timer.clone(), timing.clone(), rt.sh.clone());
        Some(std::thread::Builder::new().name("reltimer".to_owned()).spawn(move || timer.run(&timing, &sh)).expect("a thread"))
    };
    New { rt, fake, timing, link, off, timer, timer_thread, bench, counters: opts.counters, published_blocks, published_samples }
}

impl Rig for New {
    fn name(&self) -> &'static str {
        "new"
    }
    fn fake(&self) -> &Arc<FakeDevice> {
        &self.fake
    }
    fn timing(&self) -> &Arc<Timing> {
        &self.timing
    }
    fn link(&self) -> &Arc<Tap> {
        &self.link
    }
    fn offset(&self) -> Offset {
        self.off
    }
    fn bench(&self) -> Option<&Arc<Bench>> {
        Some(&self.bench)
    }
    fn ingress(&self, u: &Upd) {
        let entry = RxEntry { order: (u.e, u.i as u64 + 1), channel: 0, value: u.freq, accepted_at: None };
        self.timer.book(u.e_ns - WINDOW_NS, Held { stream: 0, entry, generation: 1 });
    }
    fn extra(&self) -> Json {
        let late: Vec<i64> = self.timer.stamps.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|&(_, deadline, stamp)| stamp - deadline).collect();
        let brain = self.rt.sh.brain();
        let consumed = brain.dispositions.iter().filter(|d| d.1 == crate::brain::core::Disposition::Consumed).count();
        drop(brain);
        let b = &self.bench;
        // Reservation attempts, aggregated per value: a rejected attempt is attributed to its reason (a bound hit is only a rejection at the in-flight
        // bound; a lost cell is `phase`). `[id, first attempt ns, reserved ns or null, attempts, rejections by reason (REASONS order)]`, for the values
        // that had a rejection.
        let table: Vec<ReserveAgg> = b.reserve.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut by_reason = [0u64; REASONS.len()];
        let mut rejected: Vec<Json> = Vec::new();
        let mut attempted = 0;
        for (index, agg) in table.iter().enumerate().filter(|(_, a)| a.attempts > 0) {
            attempted += 1;
            by_reason.iter_mut().zip(agg.by_reason).for_each(|(sum, n)| *sum += u64::from(n));
            if agg.by_reason.iter().any(|n| *n > 0) {
                rejected.push(json!([index + 1, agg.first_ns, (agg.reserved_ns != 0).then_some(agg.reserved_ns), agg.attempts, agg.by_reason]));
            }
        }
        let stage_stamps: Option<Vec<Json>> = b.stamps.as_ref().map(|stamps| {
            let sent: BTreeMap<u64, i64> = self.timer.stamps.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|&(id, _, at)| (id, at)).collect();
            stamps
                .iter()
                .enumerate()
                .filter_map(|(index, slot)| {
                    let id = index as u64 + 1;
                    let [brain, pushed, s_take] = [0, 1, 2].map(|i| slot[i].load(SeqCst));
                    sent.get(&id).filter(|_| brain != 0).map(|&sent| json!([id, sent, brain, pushed, s_take]))
                })
                .collect()
        });
        let retire_stamps: Option<Vec<Json>> = b.stamps.as_ref().map(|stamps| {
            stamps.iter().enumerate().filter_map(|(index, slot)| Some(slot[super::Stage::Retired as usize].load(SeqCst)).filter(|at| *at != 0).map(|at| json!([index as u64 + 1, at]))).collect()
        });
        json!({
            "retire_stamps": retire_stamps,
            "release_lateness": dist(&late),
            "reservations": { "values_attempted": attempted, "rejected_attempts_by_reason": REASONS.iter().zip(by_reason).map(|(r, n)| (r.to_string(), json!(n))).collect::<serde_json::Map<_, _>>(),
                              "values_rejected": rejected.len(), "per_value": rejected, "per_value_columns": ["id", "first_attempt_ns", "reserved_ns", "attempts", "rejections_by_reason"],
                              "reasons": REASONS, "not_recorded_ids": b.reserve_dropped.load(SeqCst) },
            "stage_stamps": stage_stamps, "stage_stamp_columns": ["id", "release_sent_ns", "brain_take_ns", "pushed_to_S_ns", "S_take_ns"],
            "blocks_published_to_sink": self.published_blocks.load(SeqCst),
            "cell_retries": { "counted": self.counters, "publish_retries": b.publish_retries.load(SeqCst), "stale_rounds": b.stale_rounds.load(SeqCst), "publishes": b.publishes.load(SeqCst),
                              "settings_takes_dropped": b.take_dropped.load(SeqCst),
                              "note": "S's take has no retry loop (one take per command); `take_dropped` counts takes that were not calls" },
            "r_cell_hold_per_acquisition": R_CELL_HOLD.json(),
            "r_cell_wait_per_acquisition": R_CELL_WAIT.json(),
            "cell_hold_all_actors": CELL_HOLD.json(),
            "cell_wait_all_actors": CELL_WAIT.json(),
            "diagnostic_note": "r_cell_*: R's own view and write acquisitions (two per block), recorded after the lock is released; cell_*_all_actors: every cell lock (brain, S, R, the harness), acquisition-level",
            "values_consumed": consumed,
        })
    }
    fn finish(mut self: Box<Self>) -> Json {
        let sh = self.rt.sh.clone();
        sh.send(Input::Shutdown);
        let started = std::time::Instant::now();
        let exited = |sh: &Arc<Shared>| {
            let sink = sh.sink();
            sink.exited.contains(&Actor::Settings) && sink.exited.contains(&Actor::Rx(0))
        };
        while !exited(&sh) && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let orderly = exited(&sh);
        {
            let mut state = self.timer.state.lock().unwrap_or_else(|e| e.into_inner());
            state.1 = true;
            self.timer.cv.notify_all();
        }
        let timer_ok = self.timer_thread.take().is_none_or(|handle| handle.join().is_ok());
        let stopped = self.rt.stop(Duration::from_secs(10));
        let (exits, brain_exit, lost_terminals) = {
            let sink = sh.sink();
            let lost = sink.terminals.iter().filter(|t| matches!(t.1, crate::brain::cell::Terminal::Call(crate::brain::cell::Outcome::Lost))).count();
            (format!("{:?}", sink.exited), format!("{:?}", sh.brain().exit), lost)
        };
        let failed = sh.failed();
        let b = &self.bench;
        let endpoint = json!({
            "published_blocks": self.published_blocks.load(SeqCst), "published_samples": self.published_samples.load(SeqCst), "cell_accepted_publications": b.publishes.load(SeqCst),
            "protocol_discarded_samples": { "before_origin": b.discarded_before_origin.load(SeqCst), "after_cut": b.discarded_after_cut.load(SeqCst) },
        });
        match stopped {
            Ok(panics) => {
                json!({ "endpoint": endpoint, "joined": true, "timer_ok": timer_ok, "orderly_exit": orderly, "panics": panics, "exited": exits, "brain_exit": brain_exit, "failure_flag": failed, "lost_terminals": lost_terminals })
            }
            Err(why) => {
                json!({ "endpoint": endpoint, "joined": false, "timer_ok": timer_ok, "orderly_exit": orderly, "error": why, "exited": exits, "brain_exit": brain_exit, "failure_flag": failed })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond() -> Cond {
        Cond { kind: super::super::Kind::Spaced, count: 1, rate_hz: 1e6, block_len: 2000 }
    }

    /// A measured run keeps no per-call history (it would grow with every block) while its device calls keep running through the checking decorator:
    /// blocks are received, and the decorator's log stays empty.
    #[test]
    fn a_measured_run_keeps_no_per_call_log_while_it_receives() {
        let rig = start_new(&cond(), &Opts { counters: true, ..Opts::default() });
        let started = std::time::Instant::now();
        while rig.timing.counts.calls.load(SeqCst) < 20 {
            assert!(started.elapsed() < Duration::from_secs(30), "the run received no blocks");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(rig.rt.checking.calls().is_empty(), "a measured run's checking decorator logs nothing: {:?}", rig.rt.checking.calls().len());
        assert!(Box::new(rig).finish()["joined"] == json!(true));
    }

    /// One update through the measured runtime: `S`'s apply carries the request's id and the instant it was booked at, and the brain's observation
    /// stamps the value's retirement (after the apply returned), with the retirement counted.
    #[test]
    fn a_value_applied_by_the_measured_runtime_carries_its_id_and_is_seen_retiring() {
        let rig = start_new(&cond(), &Opts { counters: true, sample: true, ..Opts::default() });
        let e = rig.off.tick_floor(rig.timing.ns() + 60_000_000);
        let u = Upd { i: 0, e, e_ns: rig.off.ns_of(e), freq: FREQS[1] };
        rig.ingress(&u);
        let started = std::time::Instant::now();
        while rig.bench.retired.load(SeqCst) < 1 {
            assert!(started.elapsed() < Duration::from_secs(30), "the value was never seen retiring");
            std::thread::sleep(Duration::from_millis(2));
        }
        let rows = rig.timing.rows();
        assert_eq!(rows.iter().map(|r| (r.id, r.e, r.ok)).collect::<Vec<_>>(), vec![(1, e, true)], "S tags the apply with the request's id");
        let retired_at = rig.bench.stamps.as_ref().unwrap()[0][super::super::Stage::Retired as usize].load(SeqCst);
        assert!(retired_at >= rows[0].ret_ns, "the value retires after its apply returned: {retired_at} vs {}", rows[0].ret_ns);
        assert!(Box::new(rig).finish()["joined"] == json!(true));
    }

    /// K and D are parameters of the measured runtime alone: the design's 4 and 64 unless the diagnostic names others.
    #[test]
    fn the_bounds_are_the_designs_unless_a_diagnostic_run_names_others() {
        use crate::brain::core::{CHAIN_BOUND, DEVICE_BOUND};
        for (asked, want) in [((0, 0), (DEVICE_BOUND, CHAIN_BOUND)), ((16, 7), (16, 7))] {
            let rig = start_new(&cond(), &Opts { counters: true, k: asked.0, d: asked.1, ..Opts::default() });
            let brain = rig.rt.sh.brain();
            assert_eq!((brain.device_bound, brain.chain_bound), want, "asked {asked:?}");
            drop(brain);
            assert!(Box::new(rig).finish()["joined"] == json!(true));
        }
    }
}
