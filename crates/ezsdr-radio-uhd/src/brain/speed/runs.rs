//! The E2b runs: one condition for one owner (`run_once`), the rate trial, and the ignored measurement tests. Each ignored test prints
//! one JSON line per result, prefixed `E2B_`; run them one at a time in a release build (`--ignored --test-threads=1`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering::Relaxed};
use std::time::Duration;

use ezsdr_kernel::stream::DataLink;
use serde_json::{Value as Json, json};

use super::EXPIRY_NS;
use super::{
    FIRST_NS, INGRESS_LATE_NS, INGRESS_NS, Kind, LEAD_NS, LOOKAHEAD, Offset, REASONS, Tap, TapStats, Timing, Upd, WINDOW_NS, account, dist, newrt, p99_ns, reason_index, set_instrument, table, today,
};
use crate::brain::core::NotReserved;
use crate::device::{Device, Dir, FakeDevice};

pub(super) struct Cond {
    pub(super) kind: Kind,
    pub(super) count: usize,
    pub(super) rate_hz: f64,
    pub(super) block_len: usize,
}

#[derive(Default)]
pub(super) struct Opts {
    /// The decorator's receive counters and the new runtime's retry counters.
    pub(super) counters: bool,
    /// The cell guards time their hold and wait (diagnostic runs only).
    pub(super) diag: bool,
    /// A device loss at this device time (ms since its time was set).
    pub(super) fault_ms: Option<u64>,
    /// The new runtime stamps each update's path (release sent, brain dequeue, push to `S`, `S` dequeue): the latency diagnostic.
    pub(super) stages: bool,
    /// The bound diagnostic (new runtime): per-second samples of the backlog, the in-flight occupancy, the retirements and the bound rejections,
    /// and a per-update table (apply entry and return, application tick, retirement).
    pub(super) sample: bool,
    /// The device capacity K and the chain bound D of the new runtime for this run (0: the design's). A diagnostic run's parameters only.
    pub(super) k: usize,
    pub(super) d: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Owner {
    Today,
    New,
}

impl Owner {
    fn name(self) -> &'static str {
        match self {
            Owner::Today => "today",
            Owner::New => "new",
        }
    }
}

/// An owner's running stream, as the harness sees it.
pub(super) trait Rig {
    fn name(&self) -> &'static str;
    fn fake(&self) -> &Arc<FakeDevice>;
    fn timing(&self) -> &Arc<Timing>;
    fn link(&self) -> &Arc<Tap>;
    fn offset(&self) -> Offset;
    /// One update enters the owner (it holds it until its release).
    fn ingress(&self, u: &Upd);
    /// Read at the end of the observation, before the owner is stopped.
    fn extra(&self) -> Json;
    /// The new runtime's measurement hooks (today's owner has none).
    fn bench(&self) -> Option<&Arc<super::Bench>> {
        None
    }
    fn finish(self: Box<Self>) -> Json;
}

fn start(owner: Owner, cond: &Cond, opts: &Opts) -> Box<dyn Rig> {
    match owner {
        Owner::Today => today::start(cond, opts),
        Owner::New => newrt::start(cond, opts),
    }
}

// ------------------------------------------------------------------------------------------------ the sink reader

/// The consumer of the receive link, the same for both owners: it drains the bounded link, counts the blocks and runs the gap test: the first
/// sample of each block against the end of the one before, the first block's against the stream's start (index 0).
struct Reader {
    stop: Arc<AtomicBool>,
    blocks: Arc<AtomicU64>,
    /// The leading gap and the gaps between the blocks read, in samples.
    missing: Arc<AtomicU64>,
    samples: Arc<AtomicU64>,
    /// The end index of the last block read.
    end_index: Arc<AtomicI64>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Reader {
    fn spawn(link: Arc<dyn DataLink>) -> Reader {
        let (stop, blocks, missing, samples, end_index) =
            (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicI64::new(0)));
        let (s, b, m, n, e) = (stop.clone(), blocks.clone(), missing.clone(), samples.clone(), end_index.clone());
        let handle = std::thread::Builder::new()
            .name("reader".to_owned())
            .spawn(move || {
                let mut expected = 0i64;
                loop {
                    let mut got = 0;
                    while let Some((block, _carry)) = link.receive() {
                        got += 1;
                        let tp = block.first_sample_time();
                        let k = tp.ticks_in(tp.domain()).unwrap_or(0); // own domain: the gap test reads each block in its SampleClock
                        if k > expected {
                            m.fetch_add((k - expected) as u64, Relaxed);
                        }
                        expected = expected.max(k + i64::from(block.header().len));
                        e.store(expected, Relaxed);
                        b.fetch_add(1, Relaxed);
                        n.fetch_add(u64::from(block.header().len), Relaxed);
                    }
                    // After the stop flag, the drain goes on until a pass finds the link empty.
                    if s.load(Relaxed) && got == 0 {
                        return;
                    }
                    if got == 0 {
                        std::thread::sleep(Duration::from_micros(100));
                    }
                }
            })
            .expect("a thread");
        Reader { stop, blocks, missing, samples, end_index, handle: Some(handle) }
    }

    /// Stops the reader after it drained the link; false if its thread panicked.
    fn end(&mut self) -> bool {
        self.stop.store(true, Relaxed);
        self.handle.take().is_none_or(|handle| handle.join().is_ok())
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct Snap {
    t_ns: i64,
    produced: u64,
    samples: u64,
    overflows: u64,
    other: u64,
    read: u64,
    read_samples: u64,
    missing: u64,
    drops: u64,
}

fn snap(timing: &Timing, tap: &Tap, reader: &Reader) -> Snap {
    let c = &timing.counts;
    Snap {
        t_ns: timing.ns(),
        produced: c.blocks.load(Relaxed),
        samples: c.samples.load(Relaxed),
        overflows: c.overflows.load(Relaxed),
        other: c.other.load(Relaxed),
        read: reader.blocks.load(Relaxed),
        read_samples: reader.samples.load(Relaxed),
        missing: tap.stats().missing,
        drops: tap.drops(),
    }
}

fn load_avg() -> String {
    std::fs::read_to_string("/proc/loadavg").map(|s| s.split(' ').take(3).collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

/// One reading of every thread of this process: `(name, tid) -> (voluntary, involuntary context switches, utime + stime clock ticks)`, from
/// `/proc/self/task/*` (the throughput diagnostic's per-thread scheduler view; `perf` is not available in the lab container).
type ThreadStats = std::collections::BTreeMap<(String, u32), (u64, u64, u64)>;

fn thread_stats() -> ThreadStats {
    let mut out = ThreadStats::new();
    let Ok(dir) = std::fs::read_dir("/proc/self/task") else { return out };
    for entry in dir.flatten() {
        let Ok(tid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let status = std::fs::read_to_string(entry.path().join("status")).unwrap_or_default();
        let field = |key: &str| status.lines().find_map(|l| l.strip_prefix(key)).and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(0);
        let name = status.lines().find_map(|l| l.strip_prefix("Name:")).unwrap_or("").trim().to_owned();
        // utime and stime are fields 14 and 15 of `stat`, after the parenthesised name.
        let stat = std::fs::read_to_string(entry.path().join("stat")).unwrap_or_default();
        let cpu = stat.rsplit_once(')').map(|(_, rest)| rest.split_whitespace().skip(11).take(2).filter_map(|v| v.parse::<u64>().ok()).sum()).unwrap_or(0);
        out.insert((name, tid), (field("voluntary_ctxt_switches:"), field("nonvoluntary_ctxt_switches:"), cpu));
    }
    out
}

/// Per thread, the per-second deltas of the readings (a thread that appears or ends inside the run shows only the seconds it was seen in).
fn thread_json(readings: &[ThreadStats]) -> Json {
    let mut out = serde_json::Map::new();
    for key in readings.last().into_iter().flat_map(|r| r.keys()) {
        let series: Vec<(u64, u64, u64)> = readings
            .windows(2)
            .map(|w| {
                let (a, b) = (w[0].get(key).copied().unwrap_or_default(), w[1].get(key).copied().unwrap_or_default());
                (b.0.saturating_sub(a.0), b.1.saturating_sub(a.1), b.2.saturating_sub(a.2))
            })
            .collect();
        out.insert(
            format!("{}#{}", key.0, key.1),
            json!({ "voluntary_per_s": series.iter().map(|x| x.0).collect::<Vec<_>>(), "involuntary_per_s": series.iter().map(|x| x.1).collect::<Vec<_>>(),
                    "cpu_ticks_per_s": series.iter().map(|x| x.2).collect::<Vec<_>>() }),
        );
    }
    Json::Object(out)
}

/// The bound diagnostic's sampler: one thread that reads, every 10 ms, the fake's backlog (device time minus the end of the last sample the
/// receive returned) and the new runtime's in-flight occupancy, and closes a record each second with the second's end and extremes and the
/// second's retirements, bound rejections and blocks. All readings are of one thread at one instant, so they are synchronized with each other.
struct Sampler {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<Json>>>,
}

impl Sampler {
    fn spawn(timing: Arc<Timing>, fake: Arc<FakeDevice>, bench: Arc<super::Bench>) -> Sampler {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let mcr = fake.master_clock_rate() as f64;
        let handle = std::thread::Builder::new()
            .name("sampler".to_owned())
            .spawn(move || {
                let begin = timing.ns();
                let mut out = Vec::new();
                let (mut backlog_max, mut occupancy_max) = (f64::MIN, 0usize);
                let (mut retired0, mut rejected0, mut blocks0) = (bench.retired.load(Relaxed), bench.bound_rejections.load(Relaxed), timing.counts.blocks.load(Relaxed));
                for tick in 1u64.. {
                    let target = begin + tick as i64 * 10_000_000;
                    while timing.ns() < target {
                        if s.load(Relaxed) {
                            return out;
                        }
                        std::thread::sleep(Duration::from_micros(500));
                    }
                    let backlog = (fake.time_now().unwrap_or(0) - timing.counts.last_end.load(Relaxed)) as f64 * 1000.0 / mcr;
                    let occupancy = bench.occupancy.load(Relaxed);
                    backlog_max = backlog_max.max(backlog);
                    occupancy_max = occupancy_max.max(occupancy);
                    if tick % 100 == 0 {
                        let (retired, rejected, blocks) = (bench.retired.load(Relaxed), bench.bound_rejections.load(Relaxed), timing.counts.blocks.load(Relaxed));
                        out.push(json!({ "second": tick / 100, "backlog_ms_end": (backlog * 1000.0).round() / 1000.0, "backlog_ms_max": (backlog_max * 1000.0).round() / 1000.0,
                                         "in_flight_end": occupancy, "in_flight_max": occupancy_max, "retired": retired - retired0, "bound_rejections": rejected - rejected0, "blocks": blocks - blocks0 }));
                        (backlog_max, occupancy_max, retired0, rejected0, blocks0) = (f64::MIN, 0, retired, rejected, blocks);
                    }
                }
                out
            })
            .expect("a thread");
        Sampler { stop, handle: Some(handle) }
    }

    fn end(&mut self) -> Vec<Json> {
        self.stop.store(true, Relaxed);
        self.handle.take().and_then(|handle| handle.join().ok()).unwrap_or_default()
    }
}

/// Backlog excursions against bound hits, over the seconds sampled: a second is an excursion when its backlog maximum exceeds `excursion_ms` and
/// a bound-hit second when the bound rejected an attempt in it.
fn coincidence(samples: &[Json], excursion_ms: f64) -> Json {
    let (mut both, mut backlog_only, mut hit_only, mut neither) = (0, 0, 0, 0);
    for second in samples {
        match (second["backlog_ms_max"].as_f64().is_some_and(|b| b > excursion_ms), second["bound_rejections"].as_u64().is_some_and(|n| n > 0)) {
            (true, true) => both += 1,
            (true, false) => backlog_only += 1,
            (false, true) => hit_only += 1,
            (false, false) => neither += 1,
        }
    }
    json!({ "excursion_threshold_ms": excursion_ms, "seconds": samples.len(), "excursion_and_bound_hit": both, "excursion_only": backlog_only, "bound_hit_only": hit_only, "neither": neither })
}

fn wait_until_ns(timing: &Timing, target: i64) {
    loop {
        let d = target - timing.ns();
        if d <= 0 {
            return;
        }
        if d > 2_000_000 {
            std::thread::sleep(Duration::from_nanos(d as u64 - 1_000_000));
        } else if d > 100_000 {
            std::thread::sleep(Duration::from_nanos(d as u64 / 2));
        } else {
            std::hint::spin_loop();
        }
    }
}

// ------------------------------------------------------------------------------------------------ one condition, one owner

pub(super) struct Out {
    pub(super) json: Json,
    /// Acceptance delays (ns) of the updates that were accepted.
    pub(super) acceptance: Vec<i64>,
    /// Updates not applied at their instant, or whose ingress was later than `e - 10 ms`, or that never returned before `e + 10 ms`.
    pub(super) failures: usize,
    pub(super) updates: usize,
    /// Overflow events, missing samples, publication drops, over the run from the owner's start to the common endpoint.
    pub(super) loss: [u64; 3],
    /// Why the run is not clean (every update applied at its instant, no unexpected apply, no extra DSP entry, no booking-floor movement, the
    /// loss reconciled). A verdict passes only over clean runs.
    pub(super) unclean: Vec<String>,
}

/// An update's ingress record: when it was asked to enter and when it did.
#[derive(Clone, Copy, Debug)]
struct Ingress {
    requested: i64,
    at: i64,
    /// The booking floor would have moved it (`timeline::timed_instant`, predicted from the table; an observed movement shows as an apply at an
    /// instant no update has).
    predicted_moved: bool,
}

/// The ingress rule (the same for both owners): update `i` enters at the later of `e_i - 50 ms` and the moment fewer than `LOOKAHEAD` earlier
/// updates are outstanding. An update is outstanding until its owner's `apply` returns or, if it never does, until `e + 10 ms`; an apply that returns
/// after `e + 10 ms` is a failure, decided from its recorded return (`run_once`), not here.
fn ingress_all(rig: &dyn Rig, timing: &Timing, off: &Offset, tab: &[Upd]) -> Vec<Ingress> {
    let (lead, window) = (off.ticks(LEAD_NS), off.ticks(WINDOW_NS));
    let mut out: Vec<Ingress> = Vec::with_capacity(tab.len());
    let mut outstanding: Vec<usize> = Vec::new();
    let mut booked: Vec<i64> = Vec::new();
    for u in tab {
        let requested = u.e_ns - INGRESS_NS;
        wait_until_ns(timing, requested);
        let at = loop {
            let now = timing.ns();
            outstanding.retain(|&j| !(timing.has_returned(tab[j].e) || now >= tab[j].e_ns + EXPIRY_NS));
            if outstanding.len() < LOOKAHEAD {
                break now;
            }
            std::thread::sleep(Duration::from_micros(20));
        };
        let now_tick = off.tick_floor(at);
        booked.retain(|e| *e > now_tick);
        let (effective, late) = ezsdr_radio::timeline::timed_instant(Some(u.e), now_tick, lead, window, booked.iter().copied());
        booked.push(effective);
        out.push(Ingress { requested, at, predicted_moved: effective != u.e || late });
        outstanding.push(u.i);
        rig.ingress(u);
    }
    out
}

/// A value was held at the in-flight bound when one of its rejections was a bound rejection; a refusal of the cell (a lost cell, a cut) is not.
fn is_bound_hit(by_reason: &[u64]) -> bool {
    [NotReserved::DeviceBound, NotReserved::ChainBound].into_iter().any(|reason| by_reason.get(reason_index(reason)).is_some_and(|n| *n > 0))
}

/// Whether the update's `apply` came back after the update's own deadline (`e + 10 ms`): the update fails, whatever the DSP log shows.
fn returned_after_expiry(row: Option<&super::Row>, u: &Upd) -> bool {
    row.is_some_and(|r| r.ret_ns > u.e_ns + EXPIRY_NS)
}

/// What an owner's stop and join say about the run: empty when it stopped orderly (every thread joined, no panic, the executors' exits as an
/// orderly shutdown leaves them, no failure flag).
fn finish_problems(finish: &Json) -> Vec<String> {
    let mut out = Vec::new();
    if finish["joined"] != json!(true) {
        out.push(format!("the owner did not stop and join: {}", finish["error"].as_str().or(finish["stop"].as_str()).unwrap_or("")));
    }
    if finish["orderly_exit"] == json!(false) {
        out.push("the executors did not exit orderly".to_owned());
    }
    if finish["timer_ok"] == json!(false) {
        out.push("the release timer thread panicked".to_owned());
    }
    if finish["panics"].as_array().is_some_and(|p| !p.is_empty()) {
        out.push(format!("panics: {}", finish["panics"]));
    }
    if finish["failure_flag"] == json!(true) {
        out.push("the failure flag was set".to_owned());
    }
    if finish["stop"].as_str().is_some_and(|stop| stop != "ok") {
        out.push(format!("the provider's stop: {}", finish["stop"]));
    }
    out
}

/// The latency stages of the new runtime per update, from the stamps, the release stamps and the decorator's rows (nanoseconds between two stages).
fn stage_table(stamps: &[Json], tab: &[Upd], rows: &[Option<super::Row>]) -> Json {
    let mut stages: [Vec<i64>; 7] = Default::default();
    for stamp in stamps {
        let (Some(id), Some(sent), Some(brain), Some(pushed), Some(s_take)) = (stamp[0].as_u64(), stamp[1].as_i64(), stamp[2].as_i64(), stamp[3].as_i64(), stamp[4].as_i64()) else { continue };
        let Some((u, Some(row))) = tab.get(id as usize - 1).zip(rows.get(id as usize - 1).copied()) else { continue };
        let release = u.e_ns - WINDOW_NS;
        for (stage, v) in stages.iter_mut().zip([sent - release, brain - sent, pushed - brain, s_take - pushed, row.entry_ns - s_take, row.ret_ns - row.entry_ns, row.ret_ns - release]) {
            stage.push(v);
        }
    }
    let names = [
        "release_sent_after_deadline",
        "timer_send_to_brain_dequeue",
        "brain_dequeue_to_push_to_S",
        "push_to_S_dequeue",
        "S_dequeue_to_apply_entry",
        "apply_entry_to_return",
        "total_release_to_apply_return",
    ];
    json!({ "updates": stages[0].len(), "stages": names.iter().zip(&stages).map(|(n, v)| json!({ *n: dist(v) })).collect::<Vec<_>>() })
}

pub(super) fn run_once(owner: Owner, cond: &Cond, opts: &Opts) -> Out {
    run_with(start(owner, cond, opts), owner, cond, opts)
}

/// One condition on a started owner (`rig`): ingress of the table, the observation, the owner's stop, then the logs and counters read.
fn run_with(rig: Box<dyn Rig>, owner: Owner, cond: &Cond, opts: &Opts) -> Out {
    set_instrument(opts.diag);
    let load0 = load_avg();
    let uptime0 = uptime_line();
    let (timing, fake, off, tap) = (rig.timing().clone(), rig.fake().clone(), rig.offset(), rig.link().clone());
    let mut reader = Reader::spawn(tap.clone());
    let base_ns = timing.ns() + FIRST_NS;
    let tab = table(cond.kind, cond.count, &off, base_ns);
    let s0 = snap(&timing, &tap, &reader);
    let mut sampler = rig.bench().filter(|_| opts.sample).map(|bench| Sampler::spawn(timing.clone(), fake.clone(), bench.clone()));
    let ingress = ingress_all(&*rig, &timing, &off, &tab);
    wait_until_ns(&timing, tab.last().map_or(base_ns, |u| u.e_ns) + 100_000_000);
    let t_end_ns = timing.ns();
    let samples = sampler.as_mut().map(Sampler::end);
    let extra = rig.extra();
    let name = rig.name();
    // The common endpoint: the owner stops (its threads join), the reader drains the link, and only then are the apply and DSP logs and the
    // counters read: a call made while stopping is accounted too.
    let finish = rig.finish();
    let reader_ok = reader.end();
    let s1 = snap(&timing, &tap, &reader);
    let tap_stats = tap.stats();
    let rows = timing.rows();
    let dsp = fake.dsp_log(Dir::Rx, 0);
    let load1 = load_avg();

    let acc = account(&tab, &rows, &dsp);
    let (mut acceptance, mut entry) = (Vec::new(), Vec::new());
    let mut why: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut first_failures = Vec::new();
    let mut failed = vec![false; tab.len()];
    let (mut late_ingress, mut returned_late) = (0, 0);
    for ((u, (row, failure)), ing) in tab.iter().zip(&acc.per).zip(&ingress) {
        let release = u.e_ns - WINDOW_NS;
        if let Some(row) = row.filter(|r| r.ok) {
            acceptance.push(row.ret_ns - release);
            entry.push(row.entry_ns - release);
        }
        let ingress_late = ing.at > u.e_ns - INGRESS_LATE_NS;
        // Each recorded return against its own deadline (also the ones that came back after the ingress rule had freed their slot).
        let late_return = returned_after_expiry(row.as_ref(), u);
        late_ingress += usize::from(ingress_late);
        returned_late += usize::from(late_return);
        if failure.is_some() || ingress_late || late_return {
            failed[u.i] = true;
            let cause = match failure {
                Some(failure) => format!("{failure:?}").split('(').next().unwrap_or("").to_owned(),
                None if ingress_late => "IngressLate".to_owned(),
                None => "ReturnedAfterExpiry".to_owned(),
            };
            *why.entry(cause).or_default() += 1;
            if first_failures.len() < 10 {
                first_failures.push(json!({ "i": u.i, "e": u.e, "why": format!("{failure:?}"), "ingress_late": ingress_late, "returned_after_expiry": late_return }));
            }
        }
    }
    let failures = failed.iter().filter(|f| **f).count();
    // The booking floor's observed movement. The new runtime tags each apply with its request's id (`S` sets it before the call), so a booking and
    // its effective instant come from the owner itself; today's owner exposes no booking identity without a production change (the seq of
    // `Control::book_timed` stays inside the owner), so there a moved update is only seen as an apply at an instant no update has (stray), and
    // the pairing of a request with an apply by its instant is INFERRED.
    let by_id: std::collections::BTreeMap<u64, super::Row> = rows.iter().filter(|r| r.id != 0).map(|r| (r.id, *r)).collect();
    let owner_booked = |u: &Upd| by_id.get(&(u.i as u64 + 1)).map(|r| r.e);
    let moved_by_owner: Vec<Json> = tab.iter().filter_map(|u| owner_booked(u).filter(|e| *e != u.e).map(|e| json!({ "i": u.i, "requested_e": u.e, "effective_e": e }))).collect();
    let untagged_rows = rows.iter().filter(|r| r.id == 0).count();
    let in_table: std::collections::BTreeSet<i64> = tab.iter().map(|u| u.e).collect();
    let mut strays: Vec<i64> = rows.iter().map(|r| r.e).filter(|e| !in_table.contains(e)).collect();
    strays.sort_unstable();
    let lateness: Vec<i64> = ingress.iter().map(|r| r.at - r.requested).collect();
    let predicted_moved = ingress.iter().filter(|r| r.predicted_moved).count();

    let ep = &finish["endpoint"];
    let n = |v: &Json| v.as_u64().unwrap_or(0);
    let protocol = Protocol {
        blocks: n(&ep["published_blocks"]),
        samples: n(&ep["published_samples"]),
        // Today's provider counts discarded blocks only (and its stop trims the last block at its cut without counting it); the new runtime counts
        // discarded samples by reason. A sample count of today's discards would be derived, so it is not compared on.
        discarded_blocks: ["before_origin", "off_lattice", "overlapping"].iter().map(|k| n(&ep["protocol_discarded_blocks"][*k])).sum(),
        discarded_samples: (owner == Owner::New).then(|| n(&ep["protocol_discarded_samples"]["before_origin"]) + n(&ep["protocol_discarded_samples"]["after_cut"])),
    };
    let read = ReaderStats { blocks: s1.read, samples: s1.read_samples, missing: reader.missing.load(Relaxed), end: reader.end_index.load(Relaxed) };
    let reconcile = reconcile_json(owner, s1.produced, s1.samples, &protocol, tap_stats, &read, s1.drops);
    // The loss numbers over the whole run, to the common endpoint: overflow events; the samples missing at the reader (its leading and in-between
    // gaps and the trailing gap up to the published stream's end, each counted once); the blocks published and never read (dropped by the link: a
    // separate number, whose samples are inside the reader's gaps).
    let loss = [s1.overflows, reader_missing_total(&read, tap_stats.end), s1.drops];
    let mut unclean = Vec::new();
    if failures > 0 {
        unclean.push(format!("{failures} updates not applied at their instant, not ingressed in time or returned after e + 10 ms"));
    }
    if owner == Owner::New && untagged_rows > 0 {
        unclean.push(format!("{untagged_rows} timed applies carry no owner booking identity"));
    }
    if acc.unexpected_rows > 0 {
        unclean.push(format!("{} unexpected timed applies", acc.unexpected_rows));
    }
    if acc.extra_dsp > 0 {
        unclean.push(format!("{} extra timed DSP entries", acc.extra_dsp));
    }
    if predicted_moved > 0 {
        unclean.push(format!("booking floor movement (predicted for {predicted_moved} updates)"));
    }
    if reconcile["balanced"] != json!(true) {
        unclean.push("the loss accounting does not reconcile".to_owned());
    }
    unclean.extend(finish_problems(&finish));
    if !reader_ok {
        unclean.push("the sink reader's thread panicked".to_owned());
    }
    // Rejected reservation attempts (the new runtime), per value with their reasons: only a rejection at the in-flight bound is a bound hit.
    let rejected: Vec<Json> = extra["reservations"]["per_value"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| {
                    let (id, first, reserved, attempts) = (value[0].as_u64()?, value[1].as_i64()?, value[2].as_i64(), value[3].as_u64()?);
                    let by_reason: Vec<u64> = value[4].as_array()?.iter().filter_map(Json::as_u64).collect();
                    let u = tab.get(id as usize - 1)?;
                    Some(json!({ "i": u.i, "attempts": attempts, "bound_hit": is_bound_hit(&by_reason),
                        "rejections": REASONS.iter().zip(&by_reason).filter(|(_, n)| **n > 0).map(|(r, n)| (r.to_string(), json!(n))).collect::<serde_json::Map<_, _>>(),
                        "held_us": reserved.map(|r| (r - first) as f64 / 1e3), "reserved_after_instant_us": reserved.map(|r| (r - u.e_ns) as f64 / 1e3),
                        "applied_at_instant": !failed[u.i], "why_not_applied": acc.per[u.i].1.as_ref().map(|w| format!("{w:?}")) }))
                })
                .collect()
        })
        .unwrap_or_default();
    let bound_hit_values = rejected.iter().filter(|r| r["bound_hit"] == json!(true)).count();
    let per_update_rows: Vec<Option<super::Row>> = acc.per.iter().map(|(row, _)| *row).collect();
    let stages = extra["stage_stamps"].as_array().map(|stamps| stage_table(stamps, &tab, &per_update_rows));
    let bound_diag = samples.map(|samples| bound_diag(&tab, &rows, &dsp, &off, &extra, &samples, cond, opts.k));
    let json = json!({
        "owner": name, "kind": cond.kind.name(), "rate_hz": cond.rate_hz, "block_len": cond.block_len, "updates": tab.len(), "counters": opts.counters, "diag": opts.diag,
        "observation_s_to_last_update": (t_end_ns - s0.t_ns) as f64 / 1e9, "observation_s_to_endpoint": (s1.t_ns - s0.t_ns) as f64 / 1e9,
        "acceptance": dist(&acceptance), "entry": dist(&entry),
        "applied_at_instant": tab.len() - acc.failures(), "failures": failures, "failure_causes": why, "first_failures": first_failures,
        "unexpected_timed_applies": acc.unexpected_rows, "extra_dsp_entries": acc.extra_dsp, "extra_dsp_entries_first": acc.extra_dsp_entries, "dropped_rows": timing.dropped_rows.load(Relaxed),
        "ingress": { "rule": "max(e - 50 ms, fewer than 8 earlier updates outstanding); outstanding until apply returns or e + 10 ms",
                     "lateness_vs_e_minus_50ms_us": dist(&lateness), "ingress_later_than_e_minus_10ms": late_ingress, "returned_after_e_plus_10ms": returned_late,
                     "booking_floor": { "predicted_moved": predicted_moved, "observed_stray_apply_instants": strays.len(), "first_stray_instants": strays.iter().take(10).collect::<Vec<_>>() },
                     "booking_identity": if owner == Owner::New { json!({ "source": "owner: the command id `S` tags on the apply", "identified_applies": by_id.len(), "untagged_applies": untagged_rows,
                                                                          "moved_by_owner": moved_by_owner.len(), "first_moved_by_owner": moved_by_owner.iter().take(10).collect::<Vec<_>>() }) }
                                         else { json!({ "source": "none: today's owner exposes no booking identity without a production change; requests and applies are paired by instant (INFERRED)",
                                                        "identified_applies": 0, "untagged_applies": untagged_rows }) },
                     "lookahead": LOOKAHEAD },
        "bound_hits": { "values_with_a_bound_hit": bound_hit_values, "values_with_a_rejection": rejected.len(), "per_update": rejected },
        "loss": { "overflow_events": loss[0], "missing_samples": loss[1], "publication_drops": loss[2], "other_receive_results": s1.other,
                  "missing_samples_published_stream_gaps": tap_stats.missing,
                  "window": "from the owner's start to the common endpoint (stopped, drained); missing = the sink reader's gaps (leading, between, trailing), each once" },
        "reconcile": reconcile,
        "clean": unclean.is_empty(), "unclean": unclean,
        "latency_stages": stages, "bound_diag": bound_diag,
        "clock_offset_unc_ns": off.unc_ns, "load_before": load0, "load_after": load1, "uptime_before": uptime0, "owner_extra": extra, "finish": finish,
    });
    if std::env::var_os("E2B_UPDATES").is_some() {
        let per: Vec<Json> = tab
            .iter()
            .zip(&acc.per)
            .zip(&ingress)
            .map(|((u, (row, why)), ing)| {
                let release = u.e_ns - WINDOW_NS;
                json!([
                    u.i,
                    u.e,
                    ing.requested,
                    ing.at,
                    owner_booked(u),
                    row.map(|r| r.freq),
                    row.map(|r| r.entry_ns - release),
                    row.map(|r| r.ret_ns - release),
                    why.as_ref().map(|w| format!("{w:?}"))
                ])
            })
            .collect();
        println!(
            "E2B_UPDATES {}",
            json!({ "owner": name, "kind": cond.kind.name(), "rate_hz": cond.rate_hz, "columns": ["i", "requested_e", "ingress_requested_ns", "ingress_ns", "owner_booking_effective_e", "applied_freq", "entry_delay_ns", "acceptance_delay_ns", "why"], "updates": per })
        );
    }
    set_instrument(false);
    Out { json, acceptance, failures, updates: tab.len(), loss, unclean }
}

/// The bound diagnostic of a run: the per-second samples, the backlog excursions against the bound hits, and per update the apply entry and return
/// (relative to the release instant `e - 3 ms`), the application tick (the tick at which the fake's DSP took the value) and the retirement time
/// (relative to the update's instant). An update's apply is the one the owner tagged with its id; its application tick is that of the DSP log
/// entry at the same position among the successful timed applies (the DSP FIFO takes the commands in the order the single applier `S` gave them;
/// a frequency that differs from the apply's counts as a pairing mismatch). A retirement is stamped at the end of the brain operation after
/// which the value is no longer in flight.
#[allow(clippy::too_many_arguments)]
fn bound_diag(tab: &[Upd], rows: &[super::Row], dsp: &[(i64, f64, bool)], off: &Offset, extra: &Json, samples: &[Json], cond: &Cond, bound: usize) -> Json {
    let retired: std::collections::BTreeMap<u64, i64> = extra["retire_stamps"].as_array().map(|a| a.iter().filter_map(|v| Some((v[0].as_u64()?, v[1].as_i64()?))).collect()).unwrap_or_default();
    let applied: Vec<(i64, f64)> = dsp.iter().filter(|d| d.2).map(|d| (d.0, d.1)).collect();
    let ok_rows: Vec<&super::Row> = rows.iter().filter(|r| r.ok).collect();
    let mut tick_of: std::collections::BTreeMap<u64, i64> = std::collections::BTreeMap::new();
    let mut mismatches = 0;
    for (row, &(tick, freq)) in ok_rows.iter().zip(&applied) {
        if row.freq.to_bits() == freq.to_bits() {
            tick_of.insert(row.id, tick);
        } else {
            mismatches += 1;
        }
    }
    let by_id: std::collections::BTreeMap<u64, &super::Row> = rows.iter().filter(|r| r.id != 0).map(|r| (r.id, r)).collect();
    let us_per_tick = 1e6 / off.mcr as f64;
    let (mut app_late, mut retire_after, mut table) = (Vec::new(), Vec::new(), Vec::new());
    for u in tab {
        let id = u.i as u64 + 1;
        let release = u.e_ns - WINDOW_NS;
        let row = by_id.get(&id);
        let tick = tick_of.get(&id).copied();
        let retired_ns = retired.get(&id).map(|r| r - u.e_ns);
        if let Some(tick) = tick {
            app_late.push(((tick - u.e) as f64 * us_per_tick * 1000.0) as i64);
        }
        if let Some(r) = retired_ns {
            retire_after.push(r);
        }
        table.push(json!([u.i, row.map(|r| r.entry_ns - release), row.map(|r| r.ret_ns - release), tick.map(|t| t - u.e), retired_ns]));
    }
    println!(
        "E2B_BOUND_UPDATES {}",
        json!({ "kind": cond.kind.name(), "bound": bound, "columns": ["i", "apply_entry_after_release_ns", "apply_return_after_release_ns", "application_tick_minus_e_ticks", "retired_after_e_ns"], "updates": table })
    );
    json!({ "bound": if bound == 0 { json!("design (4)") } else { json!(bound) }, "samples": samples, "coincidence_1ms": coincidence(samples, 1.0), "coincidence_5ms": coincidence(samples, 5.0),
            "application_lateness": dist(&app_late), "retired_after_e": dist(&retire_after), "application_pairing_mismatches": mismatches, "applications_paired": tick_of.len(),
            "note": "samples: per second, end and maximum of 10 ms readings of the fake's backlog and the chain's in-flight occupancy, and the second's retirements and bound rejections" })
}

/// What the protocol itself counted at the endpoint.
struct Protocol {
    blocks: u64,
    samples: u64,
    discarded_blocks: u64,
    /// The new runtime's count of the samples it discarded; today's provider counts blocks only, so there it is `None` (derived, never compared).
    discarded_samples: Option<u64>,
}

/// What the sink reader saw.
struct ReaderStats {
    blocks: u64,
    samples: u64,
    /// The leading gap and the gaps between the blocks read.
    missing: u64,
    end: i64,
}

/// The samples after the last block the reader read, up to the published stream's end (a block dropped at the tail of the stream).
fn trailing_gap(tap_end: i64, read_end: i64) -> u64 {
    (tap_end - read_end).max(0) as u64
}

/// The samples missing at the sink reader: its leading and in-between gaps and the trailing gap, each counted once.
fn reader_missing_total(read: &ReaderStats, tap_end: i64) -> u64 {
    read.missing + trailing_gap(tap_end, read.end)
}

/// The endpoint's reconciliation, in four identities:
///   * the published stream (the tap): samples published + samples missing from its timestamps = its end index (no gap counted twice or not at all);
///   * the protocol's own counters equal the tap's;
///   * the reader: samples read + its gaps (leading, between, and trailing up to the tap's end) = the tap's end index;
///   * blocks published = blocks read + blocks the link dropped (the link is drained).
///
/// The device side: for the new runtime, samples the device produced = samples published + samples it discarded (counted by reason). Today's
/// provider does not count discarded samples: that side is reported as derived and not compared.
fn reconcile_json(owner: Owner, device_blocks: u64, device_samples: u64, protocol: &Protocol, tap: TapStats, read: &ReaderStats, drops: u64) -> Json {
    let tap_span = tap.samples as i64 + tap.missing as i64 == tap.end;
    let counters = protocol.blocks == tap.blocks && protocol.samples == tap.samples;
    let trailing = trailing_gap(tap.end, read.end);
    let reader_span = read.samples as i64 + read.missing as i64 + trailing as i64 == tap.end;
    let blocks = tap.blocks == read.blocks + drops;
    let device_side = protocol.discarded_samples.map(|discarded| device_samples == protocol.samples + discarded);
    json!({ "device_blocks": device_blocks, "device_samples": device_samples,
            "protocol_published_blocks": protocol.blocks, "protocol_published_samples": protocol.samples, "protocol_discarded_blocks": protocol.discarded_blocks, "protocol_discarded_samples": protocol.discarded_samples,
            "device_side": if owner == Owner::New { json!("exact: produced = published + discarded") } else { json!("derived (today's provider counts no discarded samples): not compared") },
            "device_samples_not_published": device_samples as i64 - protocol.samples as i64,
            "tap": { "blocks": tap.blocks, "samples": tap.samples, "missing_samples": tap.missing, "end_index": tap.end },
            "reader": { "blocks": read.blocks, "samples": read.samples, "gaps_leading_and_between": read.missing, "gap_trailing": trailing, "end_index": read.end },
            "link_drops": drops, "queued_at_end": tap.blocks as i64 - read.blocks as i64 - drops as i64,
            "checks": { "published_span": tap_span, "protocol_counters_equal_tap": counters, "reader_span": reader_span, "blocks_published_eq_read_plus_dropped": blocks, "device_side": device_side },
            "balanced": tap_span && counters && reader_span && blocks && device_side.unwrap_or(true) })
}

fn uptime_line() -> String {
    std::fs::read_to_string("/proc/uptime").map(|s| s.trim().to_owned()).unwrap_or_default()
}

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn host_json() -> Json {
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    json!({ "hostname": std::fs::read_to_string("/etc/hostname").unwrap_or_default().trim(), "cores": cores, "loadavg": load_avg(), "uptime_s": std::fs::read_to_string("/proc/uptime").unwrap_or_default().split(' ').next().unwrap_or("").to_owned() })
}

fn emit(tag: &str, value: Json) {
    println!("{tag} {value}");
}

// ------------------------------------------------------------------------------------------------ the rate trial

/// Today's owner at `rate_hz` x `block_len` with no updates, for `secs`: the achieved block rate (skipping 5 s of warm-up), the overflow and
/// gap counts, and the fake's backlog sampled every second (device time minus the end of the last sample returned).
fn trial_run(owner: Owner, rate_hz: f64, block_len: usize, secs: usize, preload: usize) -> Json {
    let cond = Cond { kind: Kind::Spaced, count: 0, rate_hz, block_len };
    let rig = match owner {
        Owner::Today => today::start(&cond, &Opts { counters: true, diag: false, fault_ms: None, stages: false, ..Opts::default() }),
        Owner::New => newrt::start(&cond, &Opts { counters: true, diag: false, fault_ms: None, stages: false, ..Opts::default() }),
    };
    let (timing, fake, tap) = (rig.timing().clone(), rig.fake().clone(), rig.link().clone());
    let mut reader = Reader::spawn(tap.clone());
    let preloaded = preload_dsp_history(&fake, preload);
    let mcr = 200_000_000f64;
    let actual = mcr / (mcr / rate_hz).round();
    let target = actual / block_len as f64;
    let mut windows: Vec<(f64, u64, f64)> = Vec::new();
    let mut last = snap(&timing, &tap, &reader);
    let begin = last;
    let mut threads = vec![thread_stats()];
    for s in 1..=secs {
        wait_until_ns(&timing, begin.t_ns + s as i64 * 1_000_000_000);
        let now = snap(&timing, &tap, &reader);
        let backlog_ms = (fake.time_now().unwrap_or(0) - timing.counts.last_end.load(Relaxed)) as f64 * 1000.0 / mcr;
        windows.push(((now.produced - last.produced) as f64 / ((now.t_ns - last.t_ns) as f64 / 1e9), now.overflows - last.overflows, backlog_ms));
        last = now;
        threads.push(thread_stats());
    }
    let end = last;
    let warm = if secs > 10 { 5 } else { 1 };
    let steady = &windows[warm.min(windows.len().saturating_sub(1))..];
    let achieved = steady.iter().map(|w| w.0).sum::<f64>() / steady.len().max(1) as f64;
    let max_of = |w: &[(f64, u64, f64)]| w.iter().map(|x| x.2).fold(f64::MIN, f64::max);
    let (early, late) = (&windows[warm..(warm + 10).min(windows.len())], &windows[windows.len().saturating_sub(10)..]);
    let (early_max, late_max) = (max_of(early), max_of(late));
    let (overflows, missing) = (end.overflows - begin.overflows, end.missing - begin.missing);
    let within = ((achieved - target) / target).abs() <= 0.01;
    let pass = within && overflows == 0 && missing == 0 && late_max <= early_max + 2.0;
    let rate_applied = rig.extra()["rate_applied"].clone();
    let finish = rig.finish();
    reader.end();
    json!({
        "preloaded_timed_dsp_entries": preloaded, "dsp_log_entries_at_end": fake.dsp_log(Dir::Rx, 0).len(),
        "rate_hz": rate_hz, "rate_after_decimation_hz": actual, "rate_applied_by_provider": rate_applied, "block_len": block_len, "target_blocks_per_s": target, "achieved_blocks_per_s_steady": achieved,
        "within_1_percent": within, "overflow_events": overflows, "missing_samples": missing, "publication_drops": end.drops - begin.drops, "backlog_ms_early_max": early_max, "backlog_ms_late_max": late_max,
        "backlog_does_not_grow": late_max <= early_max + 2.0, "pass": pass, "seconds": secs, "per_second_blocks": windows.iter().map(|w| w.0.round()).collect::<Vec<_>>(),
        "per_second_backlog_ms": windows.iter().map(|w| (w.2 * 100.0).round() / 100.0).collect::<Vec<_>>(),
        "threads_per_second": thread_json(&threads), "perf": "not available in the lab container (no perf binary, perf_event_paranoid = 4)", "finish": finish, "load": load_avg(),
    })
}

/// The scan-cost diagnostic: `h` timed frequency values applied straight to the fake (not through an owner) with instants far in the past, so the
/// chain's DSP history holds `h` entries before the observation (the fake scans that history per block, `device.rs` `freq_constant`). A FIFO that
/// is full is waited for (the fake's own back-pressure); returns how long it took and how many waits there were.
fn preload_dsp_history(fake: &FakeDevice, h: usize) -> Json {
    let begun = std::time::Instant::now();
    let mut waits = 0u64;
    for i in 0..h {
        let settings = crate::device::Settings { freq: Some(super::FREQS[i % 2]), ..Default::default() };
        while fake.apply(Dir::Rx, 0, &settings, Some(0)).is_err() {
            waits += 1;
            assert!(waits < 10_000_000, "the DSP FIFO never drained");
            std::thread::sleep(Duration::from_micros(20));
        }
    }
    json!({ "requested": h, "waits": waits, "took_ms": begun.elapsed().as_secs_f64() * 1e3 })
}

/// The candidates, fixed before any run (brief amendment 4): 200 MS/s x 2,000, then 100,000 blocks/s at 1,000 / 500 / 200 samples per block;
/// and, only if none passes, the fallback ladder at 200 samples per block (50,000, 20,000 and 10,000 blocks/s).
const CANDIDATES: [(f64, usize); 4] = [(200e6, 2000), (100e6, 1000), (50e6, 500), (20e6, 200)];
const LADDER: [(f64, usize); 3] = [(10e6, 200), (4e6, 200), (2e6, 200)];

#[test]
#[ignore = "E2b measurement: run on a lab host in a release build"]
fn e2b_trial() {
    let secs = env_usize("E2B_TRIAL_SECS", 50);
    emit("E2B_HOST", host_json());
    let only: Option<usize> = std::env::var("E2B_TRIAL_ONLY").ok().and_then(|v| v.parse().ok());
    for (n, (rate, block)) in CANDIDATES.iter().chain(LADDER.iter()).enumerate() {
        if only.is_some_and(|only| only != n) || (only.is_none() && n >= CANDIDATES.len() && std::env::var_os("E2B_LADDER").is_none()) {
            continue;
        }
        // `E2B_TRIAL_OWNER` = `today` (default) or `new`: the trial of the owner named, at the same condition.
        let owner = if std::env::var("E2B_TRIAL_OWNER").is_ok_and(|v| v == "new") { Owner::New } else { Owner::Today };
        let result = trial_run(owner, *rate, *block, secs, env_usize("E2B_TRIAL_PRELOAD", 0));
        emit("E2B_TRIAL", json!({ "owner": owner.name(), "candidate": n, "result": result }));
        if only.is_none() && result["pass"] == json!(true) {
            break;
        }
    }
}

// ------------------------------------------------------------------------------------------------ the verdict runs

/// The latency and loss checks over a set of runs of each owner (brief: p99(new) - p99(today) < 100 us, p99(new) <= 300 us, loss(new) <=
/// loss(today) in each of the three numbers). Used pooled and for one repetition.
fn latency_loss_checks(today: &[&Out], new: &[&Out]) -> [(&'static str, bool); 3] {
    let pool = |outs: &[&Out]| outs.iter().flat_map(|o| o.acceptance.iter().copied()).collect::<Vec<_>>();
    let sums = |outs: &[&Out]| (0..3).map(|i| outs.iter().map(|o| o.loss[i]).sum::<u64>()).collect::<Vec<_>>();
    let (pt, pn) = (p99_ns(&pool(today)), p99_ns(&pool(new)));
    let (lt, ln) = (sums(today), sums(new));
    [
        ("p99_new_minus_today_under_100us", matches!((pn, pt), (Some(n), Some(t)) if n - t < 100_000)),
        ("p99_new_at_most_300us", pn.is_some_and(|n| n <= 300_000)),
        ("loss_new_at_most_today", ln.iter().zip(&lt).all(|(n, t)| n <= t)),
    ]
}

/// A run is clean when it had updates, every one applied at its instant, and nothing in its record is unclean (an unexpected apply, an extra DSP
/// entry, floor movement, loss that does not reconcile, an owner that did not stop and join orderly).
fn is_clean(out: &Out) -> bool {
    out.updates > 0 && out.failures == 0 && out.unclean.is_empty()
}

/// The condition's verdict, fail closed: PASS only if every update of both owners was applied at its instant with no unexpected apply, no extra
/// DSP entry, no booking-floor movement and the loss reconciled (every run clean), and the latency and loss checks hold pooled and in each
/// repetition (the k-th run of today's owner against the k-th of the new one). The per-repetition verdicts are in the output.
fn verdict(kind: &str, today: &[Out], new: &[Out]) -> Json {
    let (all_today, all_new): (Vec<&Out>, Vec<&Out>) = (today.iter().collect(), new.iter().collect());
    let pooled = latency_loss_checks(&all_today, &all_new);
    let per_rep: Vec<Json> = today
        .iter()
        .zip(new)
        .enumerate()
        .map(|(rep, (t, n))| {
            let checks = latency_loss_checks(&[t], &[n]);
            let (clean_today, clean_new) = (is_clean(t), is_clean(n));
            json!({ "rep": rep, "p99_today_us": p99_ns(&t.acceptance).map(|v| v as f64 / 1e3), "p99_new_us": p99_ns(&n.acceptance).map(|v| v as f64 / 1e3), "loss_today": t.loss, "loss_new": n.loss,
                    "clean_today": clean_today, "clean_new": clean_new, "unclean_today": t.unclean, "unclean_new": n.unclean,
                    "checks": checks.iter().map(|(k, v)| json!({ *k: v })).collect::<Vec<_>>(), "pass": clean_today && clean_new && checks.iter().all(|c| c.1) })
        })
        .collect();
    let rep_pass = today.len() == new.len() && !today.is_empty() && per_rep.iter().all(|r| r["pass"] == json!(true));
    let clean = |outs: &[Out]| !outs.is_empty() && outs.iter().all(is_clean);
    let unclean = |outs: &[Out]| outs.iter().flat_map(|o| o.unclean.iter().cloned()).collect::<Vec<_>>();
    let failures = |outs: &[Out]| outs.iter().map(|o| o.failures).sum::<usize>();
    let sums = |outs: &[Out]| (0..3).map(|i| outs.iter().map(|o| o.loss[i]).sum::<u64>()).collect::<Vec<_>>();
    let pool_p99 = |outs: &[&Out]| p99_ns(&outs.iter().flat_map(|o| o.acceptance.iter().copied()).collect::<Vec<_>>()).map(|v| v as f64 / 1e3);
    let checks: Vec<(&str, bool)> =
        pooled.into_iter().chain([("every_update_of_both_owners_applied_at_e_and_runs_clean", clean(today) && clean(new)), ("every_repetition_clean_and_latency_and_loss_hold", rep_pass)]).collect();
    json!({ "kind": kind, "pooled_p99_today_us": pool_p99(&all_today), "pooled_p99_new_us": pool_p99(&all_new), "loss_sums_today": sums(today), "loss_sums_new": sums(new),
            "loss_order": ["overflow_events", "missing_samples", "publication_drops"], "failures_today": failures(today), "failures_new": failures(new),
            "unclean_today": unclean(today), "unclean_new": unclean(new), "per_repetition": per_rep,
            "checks": checks.iter().map(|(k, v)| json!({ *k: v })).collect::<Vec<_>>(), "pass": checks.iter().all(|c| c.1) })
}

#[test]
#[ignore = "E2B: run on a lab host in a release build"]
fn e2b_verdict() {
    let rate = env_f64("E2B_RATE_HZ", 1e6);
    let block_len = env_usize("E2B_BLOCK_LEN", 2000);
    let count = env_usize("E2B_COUNT", 10_000);
    let kinds = std::env::var("E2B_KINDS").unwrap_or_else(|_| "spaced,bursty".to_owned());
    emit("E2B_HOST", host_json());
    // ABBA, then BAAB (A = today, B = new).
    use Owner::{New as B, Today as A};
    for kind in kinds.split(',').map(|k| if k == "bursty" { Kind::Bursty } else { Kind::Spaced }) {
        let (mut today, mut new) = (Vec::new(), Vec::new());
        for (rep, owner) in [A, B, B, A, B, A, A, B].into_iter().enumerate() {
            let out = run_once(owner, &Cond { kind, count, rate_hz: rate, block_len }, &Opts { counters: true, diag: false, fault_ms: None, stages: false, ..Opts::default() });
            let mut line = out.json.clone();
            line["rep"] = json!(rep);
            line["order"] = json!("ABBA,BAAB (A=today, B=new)");
            emit("E2B_RUN", line);
            if owner == A { today.push(out) } else { new.push(out) }
        }
        emit("E2B_VERDICT", verdict(kind.name(), &today, &new));
    }
}

/// Counters on and off, and a diagnostic run with the cell hold and wait timing on, per owner (the overhead of the instrumentation), at
/// 2,000 spaced updates.
#[test]
#[ignore = "E2B: run on a lab host in a release build"]
fn e2b_diag() {
    let rate = env_f64("E2B_RATE_HZ", 1e6);
    let block_len = env_usize("E2B_BLOCK_LEN", 2000);
    emit("E2B_HOST", host_json());
    for owner in [Owner::Today, Owner::New, Owner::New, Owner::Today] {
        for (counters, diag) in [(false, false), (true, false), (true, true)] {
            let out = run_once(owner, &Cond { kind: Kind::Spaced, count: 2000, rate_hz: rate, block_len }, &Opts { counters, diag, fault_ms: None, stages: false, ..Opts::default() });
            let mut line = out.json.clone();
            line["variant"] = json!(match (counters, diag) {
                (false, _) => "counters_off",
                (true, false) => "counters_on",
                _ => "diagnostic",
            });
            emit("E2B_DIAG", line);
        }
    }
}

/// One bursty diagnostic run of the new runtime with a stamp per update at each stage of its path (the release sent by the timer, the brain's dequeue,
/// the command pushed to `S`, `S`'s dequeue, the `apply` entry and return): where the delay of an update is spent. The stamps are one store each.
#[test]
#[ignore = "E2B: run on a lab host in a release build"]
fn e2b_latency() {
    emit("E2B_HOST", host_json());
    let count = env_usize("E2B_COUNT", 10_000);
    let cond = Cond { kind: Kind::Bursty, count, rate_hz: env_f64("E2B_RATE_HZ", 1e6), block_len: env_usize("E2B_BLOCK_LEN", 2000) };
    let out = run_once(Owner::New, &cond, &Opts { counters: true, diag: false, fault_ms: None, stages: true, ..Opts::default() });
    emit("E2B_LATENCY", out.json);
}

/// The paired bound diagnostic (new runtime only, never a verdict): the same condition at the in-flight bound of the design (4) and at the device's
/// capacity (16, the fake's command queue: `FakeConfig::command_queue`, `device.rs`; the DSP FIFO holds 32, so the queue binds first), spaced and
/// then bursty, each pair in ABBA order. Per-second samples and a per-update table come out with each run.
#[test]
#[ignore = "E2B: run on a lab host in a release build"]
fn e2b_bound() {
    emit("E2B_HOST", host_json());
    let (rate, block_len, count) = (env_f64("E2B_RATE_HZ", 50e6), env_usize("E2B_BLOCK_LEN", 500), env_usize("E2B_COUNT", 10_000));
    let capacity = env_usize("E2B_CAPACITY", 16);
    for kind in [Kind::Spaced, Kind::Bursty] {
        for bound in [4, capacity, capacity, 4] {
            let out = run_once(Owner::New, &Cond { kind, count, rate_hz: rate, block_len }, &Opts { counters: true, sample: true, k: bound, ..Opts::default() });
            emit("E2B_BOUND", out.json);
        }
    }
}

/// One device loss in the middle of the table per owner: what happens to the remaining updates and to the threads' exits.
#[test]
#[ignore = "E2B: run on a lab host in a release build"]
fn e2b_fault() {
    emit("E2B_HOST", host_json());
    for (owner, fault_ms) in [(Owner::Today, 4_000), (Owner::New, 1_500)] {
        let out =
            run_once(owner, &Cond { kind: Kind::Spaced, count: 600, rate_hz: 1e6, block_len: 2000 }, &Opts { counters: true, diag: false, fault_ms: Some(fault_ms), stages: false, ..Opts::default() });
        let mut line = out.json.clone();
        line["fault"] = json!({ "device_loss_at_ms_since_time_set": fault_ms, "owner": owner.name() });
        emit("E2B_FAULT", line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ezsdr_kernel::contract::DataContractId;
    use ezsdr_kernel::id::ClockDomainId;
    use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, ChannelMask, DataLinkDecl, Direction, SampleBlock};
    use ezsdr_kernel::time::TimePoint;

    fn block(first: i64, len: u32) -> Arc<SampleBlock> {
        let header = BlockHeader {
            first_sample_time: TimePoint::new(ClockDomainId::HOST_MONOTONIC, first),
            len,
            channels: 1,
            direction: Direction::Rx,
            valid: ChannelMask::full(1),
            flags: BlockFlags::NONE,
            lost: None,
            contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        };
        Arc::new(SampleBlock::new_host(header, ezsdr_hostmem::HOST_MEMORY, vec![0u8; len as usize * 8].into(), 8).unwrap())
    }

    #[test]
    fn the_reader_counts_blocks_and_the_samples_missing_between_them() {
        use ezsdr_kernel::module_api::Link;
        let decl = DataLinkDecl {
            id: ezsdr_kernel::id::DataLinkId::local(0),
            from: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("a").unwrap(), port: ezsdr_kernel::spec::Ident::parse("p").unwrap() },
            to: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("b").unwrap(), port: ezsdr_kernel::spec::Ident::parse("q").unwrap() },
            contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
            policy: BackPressure::DropOldest,
            capacity: 2,
        };
        let link = ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap();
        let mut reader = Reader::spawn(link.clone());
        link.publish(block(0, 10));
        link.publish(block(10, 10));
        std::thread::sleep(Duration::from_millis(30));
        link.publish(block(25, 10));
        std::thread::sleep(Duration::from_millis(30));
        reader.end();
        assert_eq!((reader.blocks.load(Relaxed), reader.missing.load(Relaxed), link.drops(), reader.end_index.load(Relaxed)), (3, 5, 0, 35));
    }

    /// The first block's index against the stream's start is a leading gap.
    #[test]
    fn the_reader_counts_the_leading_gap() {
        use ezsdr_kernel::module_api::Link;
        let decl = DataLinkDecl {
            id: ezsdr_kernel::id::DataLinkId::local(0),
            from: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("a").unwrap(), port: ezsdr_kernel::spec::Ident::parse("p").unwrap() },
            to: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("b").unwrap(), port: ezsdr_kernel::spec::Ident::parse("q").unwrap() },
            contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
            policy: BackPressure::DropOldest,
            capacity: 4,
        };
        let link = Tap::new(ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap());
        let mut reader = Reader::spawn(link.clone());
        link.publish(block(7, 10));
        link.publish(block(17, 10));
        std::thread::sleep(Duration::from_millis(30));
        assert!(reader.end());
        assert_eq!((reader.missing.load(Relaxed), link.stats()), (7, TapStats { blocks: 2, samples: 20, missing: 7, end: 27 }));
    }

    fn out(acceptance_us: &[i64], failures: usize, loss: [u64; 3]) -> Out {
        let unclean = if failures > 0 { vec!["updates not applied".to_owned()] } else { Vec::new() };
        Out { json: Json::Null, acceptance: acceptance_us.iter().map(|us| us * 1000).collect(), failures, updates: acceptance_us.len(), loss, unclean }
    }

    fn checks(v: &Json) -> Vec<bool> {
        v["checks"].as_array().unwrap().iter().map(|c| c.as_object().unwrap().values().next().unwrap().as_bool().unwrap()).collect()
    }

    // The checks, in order: p99 difference, p99 bound, loss, clean runs of both owners, every repetition.
    #[test]
    fn the_verdict_compares_p99_and_each_loss_number_and_accounting() {
        let today = [out(&[100, 150, 200], 0, [1, 2, 3])];
        // Equal loss and a p99 within bounds: every check holds.
        let v = verdict("spaced", &today, &[out(&[100, 150, 250], 0, [1, 2, 3])]);
        assert_eq!((checks(&v), v["pass"].clone()), (vec![true; 5], json!(true)));
        // One more of any loss number fails the loss check (and, with one repetition, that repetition).
        for i in 0..3 {
            let mut loss = [1, 2, 3];
            loss[i] += 1;
            assert_eq!(checks(&verdict("spaced", &today, &[out(&[100, 150, 250], 0, loss)])), [true, true, false, true, false], "loss number {i}");
        }
        // The p99 may not exceed today's by 100 us (strictly under), nor 300 us (at most).
        assert_eq!(checks(&verdict("spaced", &today, &[out(&[100, 150, 300], 0, [1, 2, 3])])), [false, true, true, true, false], "300 - 200 = 100 us is not under 100 us");
        assert_eq!(checks(&verdict("spaced", &[out(&[100, 150, 250], 0, [1, 2, 3])], &[out(&[100, 150, 301], 0, [1, 2, 3])])), [true, false, true, true, false]);
        // An unapplied update of the new owner, or of today's, fails the clean-runs check and its repetition (the repetition's pass includes cleanliness).
        assert_eq!(checks(&verdict("spaced", &today, &[out(&[100, 150, 250], 1, [1, 2, 3])])), [true, true, true, false, false]);
        assert_eq!(checks(&verdict("spaced", &[out(&[100, 150, 200], 1, [1, 2, 3])], &[out(&[100, 150, 250], 0, [1, 2, 3])])), [true, true, true, false, false]);
        // A run with an unexpected apply, an extra DSP entry, floor movement, an unreconciled loss or an owner that did not stop orderly is unclean
        // whatever its numbers.
        let mut dirty = out(&[100, 150, 250], 0, [1, 2, 3]);
        dirty.unclean.push("1 unexpected timed applies".to_owned());
        let v = verdict("spaced", &today, &[dirty]);
        assert_eq!((checks(&v), v["per_repetition"][0]["clean_new"].clone()), (vec![true, true, true, false, false], json!(false)));
    }

    /// Cleanliness is per repetition too: an unclean repetition fails its pass although its latency and loss are fine and the others are clean.
    #[test]
    fn an_unclean_repetition_fails_its_own_pass() {
        let mut dirty = out(&[100, 150, 250], 0, [1, 2, 3]);
        dirty.unclean.push("the owner did not stop and join".to_owned());
        let v = verdict("spaced", &[out(&[100, 150, 200], 0, [1, 2, 3]), out(&[100, 150, 200], 0, [1, 2, 3])], &[out(&[100, 150, 250], 0, [1, 2, 3]), dirty]);
        assert_eq!((v["per_repetition"][0]["pass"].clone(), v["per_repetition"][1]["pass"].clone(), v["pass"].clone()), (json!(true), json!(false), json!(false)));
    }

    /// Pooled numbers can hide a repetition that fails: new loses more in repetition 1 than today did there, and fewer in repetition 0.
    #[test]
    fn the_verdict_holds_every_repetition_not_only_the_pool() {
        let today = [out(&[100, 150, 200], 0, [1, 0, 0]), out(&[100, 150, 200], 0, [1, 0, 0])];
        let new = [out(&[100, 150, 250], 0, [0, 0, 0]), out(&[100, 150, 250], 0, [2, 0, 0])];
        let v = verdict("spaced", &today, &new);
        assert_eq!(checks(&v), [true, true, true, true, false]);
        assert_eq!(v["per_repetition"][0]["pass"], json!(true));
        assert_eq!(v["per_repetition"][1]["pass"], json!(false));
        assert_eq!(v["pass"], json!(false));
    }

    fn tapst(blocks: u64, samples: u64, missing: u64, end: i64) -> TapStats {
        TapStats { blocks, samples, missing, end }
    }

    fn read(blocks: u64, samples: u64, missing: u64, end: i64) -> ReaderStats {
        ReaderStats { blocks, samples, missing, end }
    }

    fn proto(blocks: u64, samples: u64, discarded_samples: Option<u64>) -> Protocol {
        Protocol { blocks, samples, discarded_blocks: 0, discarded_samples }
    }

    fn checks_of(v: &Json) -> Vec<Json> {
        let c = &v["checks"];
        ["published_span", "protocol_counters_equal_tap", "reader_span", "blocks_published_eq_read_plus_dropped", "device_side"].iter().map(|k| c[*k].clone()).collect()
    }

    /// The endpoint balances through four identities. 8 blocks of 10 samples were published from index 0 with a gap of 5 before the fifth (end 85); the reader
    /// read 7 of them (one dropped by the link, its 10 samples a gap in the middle).
    #[test]
    fn the_endpoint_reconciles_the_published_stream_the_counters_and_the_reader() {
        let tap = tapst(8, 80, 5, 85);
        // The reader missed the block at 30..40 (a gap of 10 in the middle) and the 5 missing before block 5: 80 - 10 read, 15 missing, end 85.
        let ok = reconcile_json(Owner::New, 10, 100, &proto(8, 80, Some(20)), tap, &read(7, 70, 15, 85), 1);
        assert_eq!((ok["balanced"].clone(), checks_of(&ok)), (json!(true), vec![json!(true); 5]));
        // The dropped block was the last one: the reader's end is short, and that trailing gap is counted once (read 70 + 5 leading/between + 10 trailing).
        let tail = reconcile_json(Owner::New, 10, 100, &proto(8, 80, Some(20)), tap, &read(7, 70, 5, 75), 1);
        assert_eq!(tail["balanced"], json!(true), "{tail}");
        assert_eq!(tail["reader"]["gap_trailing"], json!(10));
        // A leading gap is a gap: the first block at index 7 of a stream that starts at 0 is 7 missing samples, in the tap and in the reader.
        let lead = reconcile_json(Owner::Today, 3, 30, &proto(3, 30, None), tapst(3, 30, 7, 37), &read(3, 30, 7, 37), 0);
        assert_eq!(lead["balanced"], json!(true), "{lead}");
        // Counting a gap twice (or not at all) breaks the span identity of the tap or of the reader.
        assert_eq!(checks_of(&reconcile_json(Owner::New, 10, 100, &proto(8, 80, Some(20)), tapst(8, 80, 0, 85), &read(7, 70, 15, 85), 1))[0], json!(false));
        assert_eq!(checks_of(&reconcile_json(Owner::New, 10, 100, &proto(8, 80, Some(20)), tap, &read(7, 70, 5, 85), 1))[2], json!(false), "a gap the reader did not count");
        // The protocol's counters must equal the tap's; a block neither read nor dropped is not balanced.
        assert_eq!(checks_of(&reconcile_json(Owner::New, 10, 100, &proto(9, 80, Some(20)), tap, &read(7, 70, 15, 85), 1))[1], json!(false));
        assert_eq!(checks_of(&reconcile_json(Owner::New, 10, 100, &proto(8, 80, Some(20)), tap, &read(6, 60, 25, 85), 1))[3], json!(false));
        // New: produced = published + discarded, exactly; today: derived, never compared.
        assert_eq!(checks_of(&reconcile_json(Owner::New, 10, 101, &proto(8, 80, Some(20)), tap, &read(7, 70, 15, 85), 1))[4], json!(false));
        assert_eq!(checks_of(&reconcile_json(Owner::Today, 10, 101, &proto(8, 80, None), tap, &read(7, 70, 15, 85), 1))[4], Json::Null);
    }

    #[test]
    fn a_failed_stop_or_a_panic_makes_the_run_unclean() {
        assert!(finish_problems(&json!({ "joined": true, "orderly_exit": true, "timer_ok": true, "panics": [], "failure_flag": false })).is_empty());
        assert!(finish_problems(&json!({ "joined": true, "stop": "ok" })).is_empty());
        for bad in [
            json!({ "joined": false, "error": "a thread did not finish" }),
            json!({ "joined": true, "orderly_exit": false }),
            json!({ "joined": true, "timer_ok": false }),
            json!({ "joined": true, "panics": ["Brain: boom"] }),
            json!({ "joined": true, "failure_flag": true }),
            json!({ "joined": true, "stop": "error: stop failed" }),
        ] {
            assert_eq!(finish_problems(&bad).len(), 1, "{bad}");
        }
    }

    /// An apply that returned after its own deadline is a failure (also when it is the last outstanding one); one at the deadline is not.
    #[test]
    fn a_return_after_its_own_deadline_is_expired() {
        let u = Upd { i: 3, e: 100, e_ns: 50_000_000, freq: 1.0 };
        let row = |ret_ns| super::super::Row { e: 100, entry_ns: 0, ret_ns, ok: true, lost: false, freq: 1.0, id: 0 };
        assert!(!returned_after_expiry(Some(&row(u.e_ns + EXPIRY_NS)), &u), "exactly at e + 10 ms is in time");
        assert!(returned_after_expiry(Some(&row(u.e_ns + EXPIRY_NS + 1)), &u));
        assert!(!returned_after_expiry(None, &u), "no return is a NoApply failure, not an expiry");
    }

    /// Only a rejection at the in-flight bound is a bound hit: a lost cell's refusal (phase) is not.
    #[test]
    fn only_a_rejection_at_the_bound_is_a_bound_hit() {
        assert!(is_bound_hit(&[2, 1, 0, 0, 0, 0, 0]));
        assert!(!is_bound_hit(&[0, 0, 357, 0, 0, 0, 0, 0]), "357 rejections of a lost cell are no bound hit");
        assert!(!is_bound_hit(&[0; 8]));
    }

    /// Each stage of the new runtime's path is the difference of two stamps.
    #[test]
    fn the_stage_table_reads_each_stage_between_two_stamps() {
        let tab = vec![Upd { i: 0, e: 100, e_ns: 10_000_000, freq: 1.0 }];
        let release = 10_000_000 - WINDOW_NS;
        let row = super::super::Row { e: 100, entry_ns: release + 400, ret_ns: release + 900, ok: true, lost: false, freq: 1.0, id: 0 };
        let stamps = vec![json!([1, release + 50, release + 150, release + 200, release + 300])];
        let t = stage_table(&stamps, &tab, &[Some(row)]);
        let get = |name: &str| t["stages"].as_array().unwrap().iter().find_map(|s| s.get(name)).unwrap()["p50_us"].as_f64().unwrap();
        assert_eq!(
            [
                "release_sent_after_deadline",
                "timer_send_to_brain_dequeue",
                "brain_dequeue_to_push_to_S",
                "push_to_S_dequeue",
                "S_dequeue_to_apply_entry",
                "apply_entry_to_return",
                "total_release_to_apply_return"
            ]
            .map(get),
            [0.05, 0.1, 0.05, 0.1, 0.1, 0.5, 0.9]
        );
    }

    /// A link that refuses every block from index `lose_from` on, as a lossy link at the tail of a stream does: the publishing end (the tap) has seen the
    /// block, the reader never will.
    struct TailLoss {
        inner: Arc<dyn DataLink>,
        lose_from: i64,
        dropped: AtomicU64,
    }

    impl DataLink for TailLoss {
        fn publish(&self, b: ezsdr_kernel::stream::BlockRef) -> ezsdr_kernel::stream::PublishOutcome {
            let t = b.first_sample_time();
            let k = t.ticks_in(t.domain()).unwrap_or(0); // own domain: a test link counts in the block's SampleClock
            if k >= self.lose_from {
                self.dropped.fetch_add(1, Relaxed);
                return ezsdr_kernel::stream::PublishOutcome::DroppedNewest;
            }
            self.inner.publish(b)
        }
        fn receive(&self) -> Option<(ezsdr_kernel::stream::BlockRef, ezsdr_kernel::stream::DropCarry)> {
            self.inner.receive()
        }
        fn drops(&self) -> u64 {
            self.dropped.load(Relaxed) + self.inner.drops()
        }
        fn take_drop_carry(&self) -> ezsdr_kernel::stream::DropCarry {
            self.inner.take_drop_carry()
        }
        fn policy(&self) -> ezsdr_kernel::stream::BackPressure {
            self.inner.policy()
        }
    }

    /// An owner whose stop is delayed and does work while it stops: a timed apply at an instant no update has, and the publication of two blocks of
    /// which the link refuses the second.
    struct Stopping {
        fake: Arc<FakeDevice>,
        timing: Arc<Timing>,
        link: Arc<Tap>,
        off: Offset,
        /// Whether each update is booked at an instant 3,000 ticks later than requested, with the owner's identity of the request on the apply.
        books_moved: bool,
    }

    impl Rig for Stopping {
        fn name(&self) -> &'static str {
            "stopping"
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
        fn ingress(&self, u: &Upd) {
            if self.books_moved {
                self.timing.tag(u.i as u64 + 1);
                self.timing.apply(Dir::Rx, 0, &crate::device::Settings { freq: Some(u.freq), ..Default::default() }, Some(u.e + 3_000)).expect("the fake takes a timed apply");
            }
        }
        fn extra(&self) -> Json {
            Json::Null
        }
        fn finish(self: Box<Self>) -> Json {
            std::thread::sleep(Duration::from_millis(40));
            let at = self.fake.time_now().unwrap() + 10_000_000;
            self.timing.apply(Dir::Rx, 0, &crate::device::Settings { freq: Some(1.5e9), ..Default::default() }, Some(at)).expect("the fake takes a timed apply");
            self.link.publish(block(0, 10));
            self.link.publish(block(10, 10));
            json!({ "joined": true })
        }
    }

    fn run_stopping() -> Out {
        run_stub(Owner::Today, false)
    }

    fn run_stub(owner: Owner, books_moved: bool) -> Out {
        use ezsdr_kernel::module_api::Link;
        let fake = Arc::new(FakeDevice::new(crate::device::FakeConfig::default()));
        fake.set_time_zero(false).unwrap();
        let off = Offset::establish(&fake, 200_000_000);
        let decl = DataLinkDecl {
            id: ezsdr_kernel::id::DataLinkId::local(0),
            from: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("a").unwrap(), port: ezsdr_kernel::spec::Ident::parse("p").unwrap() },
            to: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("b").unwrap(), port: ezsdr_kernel::spec::Ident::parse("q").unwrap() },
            contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
            policy: BackPressure::DropOldest,
            capacity: 8,
        };
        let inner = ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap();
        let link = Tap::new(Arc::new(TailLoss { inner, lose_from: 10, dropped: AtomicU64::new(0) }));
        fake.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), ..Default::default() }, None).unwrap();
        let timing = Timing::new(fake.clone(), 16, true);
        let rig = Box::new(Stopping { fake, timing, link, off, books_moved });
        run_with(rig, owner, &Cond { kind: Kind::Spaced, count: 1, rate_hz: 1e6, block_len: 2000 }, &Opts::default())
    }

    /// The logs are read after the owner stopped: a call the owner makes while it is stopping is accounted (an apply at an instant no update has is
    /// an unexpected one), not lost to a snapshot taken before the stop.
    #[test]
    fn a_call_made_while_the_owner_stops_is_accounted() {
        let out = run_stopping();
        assert_eq!(out.json["unexpected_timed_applies"], json!(1), "{}", out.json["unclean"]);
    }

    /// The loss's missing samples are the reader's: the second block was published (the tap saw it) and the link dropped it, so the reader misses its 10
    /// samples as a trailing gap that the published stream's own gaps (none) do not show; the drop is its own number.
    #[test]
    fn the_loss_counts_the_samples_the_reader_missed_at_the_tail() {
        let out = run_stopping();
        assert_eq!(out.loss, [0, 10, 1], "overflows, reader-missing samples, publication drops: {}", out.json["loss"]);
        assert_eq!(out.json["loss"]["missing_samples_published_stream_gaps"], json!(0));
    }

    /// The booking's identity and effective instant come from the owner when it tags the apply with its request (the new runtime); an owner that tags
    /// nothing (today's) is reported as pairing by instant, which is inferred, and an apply of the new runtime with no tag makes the run unclean.
    #[test]
    fn a_moved_booking_is_attributed_by_the_owners_own_identity_and_an_untagging_owner_is_labelled_inferred() {
        let out = run_stub(Owner::New, true);
        let id = &out.json["ingress"]["booking_identity"];
        assert_eq!((id["identified_applies"].clone(), id["moved_by_owner"].clone(), id["first_moved_by_owner"][0]["i"].clone()), (json!(1), json!(1), json!(0)), "{id}");
        let (requested, effective) = (id["first_moved_by_owner"][0]["requested_e"].as_i64().unwrap(), id["first_moved_by_owner"][0]["effective_e"].as_i64().unwrap());
        assert_eq!(effective - requested, 3_000, "the effective instant is the owner's, not an instant paired with the request");
        assert_eq!(id["untagged_applies"], json!(1), "the stop's own apply carries no tag");
        assert!(out.unclean.iter().any(|why| why.contains("no owner booking identity")), "{:?}", out.unclean);
        let today = run_stub(Owner::Today, true);
        let source = today.json["ingress"]["booking_identity"]["source"].as_str().unwrap().to_owned();
        assert!(source.starts_with("none") && source.contains("INFERRED"), "{source}");
    }

    /// A second is a backlog excursion above the threshold and a bound-hit second when the bound rejected an attempt in it.
    #[test]
    fn the_coincidence_counts_excursions_against_bound_hits_per_second() {
        let sec = |backlog: f64, hits: u64| json!({ "backlog_ms_max": backlog, "bound_rejections": hits });
        let c = coincidence(&[sec(30.0, 4), sec(30.0, 0), sec(0.05, 2), sec(0.05, 0), sec(30.0, 1)], 1.0);
        assert_eq!(
            (c["excursion_and_bound_hit"].clone(), c["excursion_only"].clone(), c["bound_hit_only"].clone(), c["neither"].clone(), c["seconds"].clone()),
            (json!(2), json!(1), json!(1), json!(1), json!(5))
        );
    }

    /// The per-update table pairs each update with its own apply (by the owner's id), the DSP entry at the same position among the successful applies, and
    /// the retirement stamp of its id; a DSP entry of another frequency at that position is a pairing mismatch, not an application.
    #[test]
    fn the_bound_diagnostic_pairs_an_update_with_its_apply_its_application_and_its_retirement() {
        let off = Offset::establish(
            &{
                let fake = FakeDevice::new(crate::device::FakeConfig::default());
                fake.set_time_zero(false).unwrap();
                fake
            },
            200_000_000,
        );
        let tab = vec![Upd { i: 0, e: 1_000_000, e_ns: 10_000_000, freq: 1.0 }, Upd { i: 1, e: 2_000_000, e_ns: 20_000_000, freq: 2.0 }];
        let rows = vec![
            super::super::Row { e: 1_000_000, entry_ns: 7_100_000, ret_ns: 7_200_000, ok: true, lost: false, freq: 1.0, id: 1 },
            super::super::Row { e: 2_000_000, entry_ns: 17_100_000, ret_ns: 17_300_000, ok: true, lost: false, freq: 2.0, id: 2 },
        ];
        let extra = json!({ "retire_stamps": [[1, 10_500_000], [2, 21_000_000]] });
        let cond = Cond { kind: Kind::Spaced, count: 2, rate_hz: 1e6, block_len: 2000 };
        let dsp = vec![(5, 0.0, false), (1_000_000, 1.0, true), (2_000_040, 2.0, true)];
        let d = bound_diag(&tab, &rows, &dsp, &off, &extra, &[], &cond, 4);
        assert_eq!((d["applications_paired"].clone(), d["application_pairing_mismatches"].clone()), (json!(2), json!(0)));
        assert_eq!(d["application_lateness"]["max_us"].as_f64(), Some(0.2), "40 ticks of 5 ns after the second value's instant");
        assert_eq!((d["retired_after_e"]["min_us"].as_f64(), d["retired_after_e"]["max_us"].as_f64()), (Some(500.0), Some(1000.0)));
        let wrong = vec![(1_000_000, 1.0, true), (2_000_040, 9.0, true)];
        let d = bound_diag(&tab, &rows, &wrong, &off, &extra, &[], &cond, 4);
        assert_eq!((d["applications_paired"].clone(), d["application_pairing_mismatches"].clone()), (json!(1), json!(1)), "a frequency that differs from the apply's is no application");
    }

    /// The scan-cost diagnostic's preload gives the chain's DSP history the entries asked for, whatever the FIFO's back-pressure.
    #[test]
    fn the_preload_fills_the_dsp_history_through_the_fifos_back_pressure() {
        let fake = FakeDevice::new(crate::device::FakeConfig::default());
        fake.set_time_zero(false).unwrap();
        fake.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), ..Default::default() }, None).unwrap();
        fake.rx_open(0, &[0]).unwrap();
        fake.rx_start(0, fake.time_now().unwrap() + 2_000_000).unwrap();
        // Samples pass the chain only as a receive reads them (a receive executor's part).
        let (fake, stop) = (Arc::new(fake), Arc::new(AtomicBool::new(false)));
        let reader = {
            let (fake, stop) = (fake.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Relaxed) {
                    fake.rx_recv(0, 1000, Duration::from_millis(50), false);
                }
            })
        };
        let load = preload_dsp_history(&fake, 200);
        assert_eq!(load["requested"], json!(200));
        let started = std::time::Instant::now();
        while fake.dsp_log(Dir::Rx, 0).iter().filter(|d| d.2).count() < 200 {
            assert!(started.elapsed() < Duration::from_secs(10), "the history never reached 200 entries");
            std::thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Relaxed);
        reader.join().unwrap();
    }

    #[test]
    fn the_readers_missing_samples_are_its_gaps_and_its_trailing_gap_once_each() {
        assert_eq!(reader_missing_total(&read(3, 30, 0, 30), 40), 10, "a block dropped at the tail");
        assert_eq!(reader_missing_total(&read(3, 25, 5, 30), 40), 15, "a gap between blocks and the tail");
        assert_eq!(reader_missing_total(&read(3, 30, 7, 37), 37), 7, "no tail: the leading gap alone");
        assert_eq!(trailing_gap(30, 40), 0, "a reader past the tap's end has no trailing gap");
    }

    #[test]
    #[ignore = "E2B smoke of the whole path: seconds of Provider start-up, and a debug build is too slow for the new runtime; run in release"]
    fn a_short_table_runs_on_both_owners_and_the_run_reports_its_shape() {
        // A short smoke of the whole path (a few updates, 1 MS/s): the numbers are not asserted, only that every update was accounted and the
        // run's record is complete.
        for owner in [Owner::Today, Owner::New] {
            let out = run_once(owner, &Cond { kind: Kind::Spaced, count: 12, rate_hz: 1e6, block_len: 2000 }, &Opts { counters: true, diag: false, fault_ms: None, stages: false, ..Opts::default() });
            assert_eq!((out.updates, out.failures), (12, 0), "{owner:?}: {}", out.json);
            assert_eq!(out.json["finish"]["joined"], json!(true), "{owner:?}: {}", out.json);
            assert_eq!(out.json["ingress"]["booking_floor"]["observed_stray_apply_instants"], json!(0), "{owner:?}");
            assert_eq!(out.json["clean"], json!(true), "{owner:?}: {}", out.json);
        }
    }
}
