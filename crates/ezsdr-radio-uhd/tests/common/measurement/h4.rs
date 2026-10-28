//! H4 scheduler/submission proxies. These are observations, not wake-up latency or RF execution.

use super::*;
use super::super as common;
use common::probe_device::ProbeDevice;
use std::time::{Duration as Wall, Instant};

pub const H4_DISPATCH_UPDATES: usize = 200;
const H4_RATE_HZ: u64 = 10_000_000;
const H4_PERIOD_NS: u64 = 10_000_000;
const H4_FIRST_LEAD_NS: u64 = 50_000_000;
const H4_APPLY_WAIT: Wall = Wall::from_secs(2);
const CLOCK_RATE_TOLERANCE_PPM: i128 = 1_000;

// The Radio vocabulary has no timed gain verb; this test-only verb exercises its existing timed Session update path.
fn h4_gain_vocabulary() -> ezsdr_kernel::module_api::VocabularyDescriptor {
    let id = ezsdr_kernel::spec::Namespace::parse("ezsdr.h4").expect("valid H4 vocabulary");
    ezsdr_kernel::module_api::VocabularyDescriptor {
        id: id.clone(),
        version: ezsdr_kernel::module_api::Version::new(1, 0, 0),
        prefix: id,
        keys: Vec::new(),
        event_kinds: Vec::new(),
        verbs: vec![ezsdr_kernel::module_api::VerbDecl {
            verb: ezsdr_kernel::spec::Ident::parse("set_rx_gain").expect("valid H4 verb"),
            compiles_to: ezsdr_kernel::module_api::CompileRule::UpdateParameter {
                key: ezsdr_kernel::spec::Key::parse("radio.rx.c0.gain_db").expect("declared radio gain key"),
            },
        }],
        checks: Vec::new(),
    }
}

fn h4_gain_action(at: TimePoint, gain: f64) -> ezsdr_kernel::session::SessionAction {
    ezsdr_kernel::session::SessionAction::Vocabulary {
        ns: ezsdr_kernel::spec::Namespace::parse("ezsdr.h4").expect("valid H4 vocabulary"),
        verb: ezsdr_kernel::spec::Ident::parse("set_rx_gain").expect("valid H4 verb"),
        target: common::resource("radio"),
        at: Some(at),
        params: std::collections::BTreeMap::from([(
            ezsdr_kernel::spec::Key::parse("radio.rx.c0.gain_db").expect("declared radio gain key"),
            ezsdr_kernel::spec::Value::num(gain).expect("finite H4 gain"),
        )]),
    }
}

fn h4_candidate_session(device: Arc<dyn Device>, timing: ezsdr_radio::TimingEnvelope) -> Result<(RunHandle, common::TempDir), String> {
    let dir = common::TempDir::new();
    let doc = common::bench_profile(&*device, &dir, Json::Object(Default::default()), common::bench_envelope(&*device), true);
    let mut assembly = common::assembly(&doc, device, Default::default(), |radio| radio.with_measurement_timing(timing));
    assembly.registry.register_vocabulary(h4_gain_vocabulary()).map_err(|error| error.to_string())?;
    let run = ezsdr_kernel::coordinator::connect(&doc, assembly, Lease::attached()).map_err(|error| error.to_string())?;
    Ok((run, dir))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockAnchor {
    pub device_tick: i64,
    pub host_before_ns: u64,
    pub host_after_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostInterval {
    pub lower_ns: i64,
    pub upper_ns: i64,
    pub midpoint_ns: i64,
    pub bracket_width_ns: u64,
    pub anchor_span_ticks: i64,
    pub anchor_span_host_ns: i64,
    pub before_age_ns: u64,
    pub after_age_ns: u64,
}

/// Interpolates only between ordered brackets whose rate agrees with the master clock within 1,000 ppm.
/// Bracket uncertainty is added to that tolerance; neither bounds long-horizon drift.
pub fn map_device_tick(anchors: [ClockAnchor; 2], tick: i64, master_clock_hz: u64) -> Result<HostInterval, String> {
    let [before, after] = anchors;
    if before.host_before_ns > before.host_after_ns || after.host_before_ns > after.host_after_ns {
        return Err("a host-clock bracket is reversed".to_owned());
    }
    let device_span = after.device_tick.checked_sub(before.device_tick).filter(|span| *span > 0)
        .ok_or_else(|| "device-time anchors are missing, reversed or discontinuous".to_owned())?;
    let midpoint = |anchor: ClockAnchor| i128::from(anchor.host_before_ns) + (i128::from(anchor.host_after_ns) - i128::from(anchor.host_before_ns)) / 2;
    let h0 = midpoint(before);
    let h1 = midpoint(after);
    let host_span = h1 - h0;
    if host_span <= 0 || tick < before.device_tick || tick > after.device_tick {
        return Err("the target is outside the ordered host/device anchor interval".to_owned());
    }
    if !clock_rate_matches(anchors, master_clock_hz) {
        return Err(format!("the host/device rate differs by more than {CLOCK_RATE_TOLERANCE_PPM} ppm"));
    }
    let offset = i128::from(tick - before.device_tick);
    let target = h0 + offset * host_span / i128::from(device_span);
    let left_weight = i128::from(after.device_tick - tick);
    let right_weight = i128::from(tick - before.device_tick);
    let bracket = left_weight * i128::from(before.host_after_ns - before.host_before_ns)
        + right_weight * i128::from(after.host_after_ns - after.host_before_ns);
    let radius = (bracket + 2 * i128::from(device_span) - 1) / (2 * i128::from(device_span)) + 1;
    let lower = i64::try_from(target - radius).map_err(|_| "mapped host time underflow".to_owned())?;
    let upper = i64::try_from(target + radius).map_err(|_| "mapped host time overflow".to_owned())?;
    let midpoint_ns = i64::try_from(target).map_err(|_| "mapped host midpoint overflow".to_owned())?;
    let before_age_ns = u64::try_from((target - h0).abs()).map_err(|_| "anchor age overflow".to_owned())?;
    let after_age_ns = u64::try_from((h1 - target).abs()).map_err(|_| "anchor age overflow".to_owned())?;
    let bracket_width_ns = u64::try_from(i128::from(upper) - i128::from(lower)).map_err(|_| "mapped bracket width overflow".to_owned())?;
    Ok(HostInterval {
        lower_ns: lower, upper_ns: upper, midpoint_ns, bracket_width_ns,
        anchor_span_ticks: device_span, anchor_span_host_ns: i64::try_from(host_span).map_err(|_| "host anchor span overflow".to_owned())?,
        before_age_ns, after_age_ns,
    })
}

fn clock_rate_matches(anchors: [ClockAnchor; 2], master_clock_hz: u64) -> bool {
    if master_clock_hz == 0 { return false; }
    let midpoint = |anchor: ClockAnchor| i128::from(anchor.host_before_ns) + (i128::from(anchor.host_after_ns) - i128::from(anchor.host_before_ns)) / 2;
    let host_span = midpoint(anchors[1]) - midpoint(anchors[0]);
    let device_span = i128::from(anchors[1].device_tick) - i128::from(anchors[0].device_tick);
    if host_span <= 0 || device_span <= 0 { return false; }
    let Some(expected) = host_span.checked_mul(i128::from(master_clock_hz)) else { return false };
    let Some(actual) = device_span.checked_mul(1_000_000_000) else { return false };
    let Some(difference) = actual.checked_sub(expected).and_then(i128::checked_abs) else { return false };
    let Some(tolerance) = expected.checked_mul(CLOCK_RATE_TOLERANCE_PPM).map(|value| value / 1_000_000) else { return false };
    let bracket_ns = (i128::from(anchors[0].host_after_ns - anchors[0].host_before_ns)
        + i128::from(anchors[1].host_after_ns - anchors[1].host_before_ns)) / 2;
    let Some(bracket_tolerance) = bracket_ns.checked_mul(i128::from(master_clock_hz)) else { return false };
    difference <= tolerance + bracket_tolerance
}

/// Signed delay from the mapped release-window instant to the Device::apply entry.
pub fn dispatch_delay(entry_host_ns: u64, e: i64, release_window_ticks: i64, anchors: [ClockAnchor; 2], master_clock_hz: u64) -> Result<(i64, i64, HostInterval), String> {
    let release_tick = e.checked_sub(release_window_ticks).ok_or("release-window tick underflow")?;
    let mapped = map_device_tick(anchors, release_tick, master_clock_hz)?;
    let entry = i128::from(entry_host_ns);
    let lower = i64::try_from(entry - i128::from(mapped.upper_ns)).map_err(|_| "dispatch delay underflow".to_owned())?;
    let upper = i64::try_from(entry - i128::from(mapped.lower_ns)).map_err(|_| "dispatch delay overflow".to_owned())?;
    Ok((lower, upper, mapped))
}

fn host_to_device_tick(anchors: [ClockAnchor; 2], host_ns: u64, master_clock_hz: u64) -> Option<i64> {
    let midpoint = |anchor: ClockAnchor| i128::from(anchor.host_before_ns) + (i128::from(anchor.host_after_ns) - i128::from(anchor.host_before_ns)) / 2;
    let h0 = midpoint(anchors[0]);
    let h1 = midpoint(anchors[1]);
    let d0 = i128::from(anchors[0].device_tick);
    let d1 = i128::from(anchors[1].device_tick);
    let span = h1 - h0;
    if anchors.iter().any(|anchor| anchor.host_before_ns > anchor.host_after_ns)
        || span <= 0 || d1 <= d0 || i128::from(host_ns) < h0 || i128::from(host_ns) > h1
        || !clock_rate_matches(anchors, master_clock_hz) { return None; }
    i64::try_from(d0 + (i128::from(host_ns) - h0) * (d1 - d0) / span).ok()
}

fn clock_anchor_json(anchor: Option<ClockAnchor>) -> Json {
    match anchor {
        Some(anchor) => json!({ "device_tick": anchor.device_tick, "host_before_ns": anchor.host_before_ns, "host_after_ns": anchor.host_after_ns }),
        None => json!({ "status": "unavailable" }),
    }
}

/// Headroom is the nominal next-sample submission margin, not RF execution time.
pub fn tx_headroom(rows: &[Json], anchors: [ClockAnchor; 2], master_clock_hz: u64, rate_hz: u64, underflow_ticks: &[i64]) -> Vec<Json> {
    let mut results = Vec::new();
    let mut segment: Option<(u64, i64, Vec<u64>)> = None;
    let mut underflows = underflow_ticks.to_vec();
    underflows.sort_unstable();
    let mut next_underflow = 0;
    for row in rows.iter().filter(|row| row["call"] == "tx_send") {
        let generation = row["generation"].as_u64().unwrap_or_default();
        let host_before = row["host_before_ns"].as_u64();
        let Some(host_before) = host_before else { segment = None; continue };
        let Some(call_tick) = host_to_device_tick(anchors, host_before, master_clock_hz) else {
            segment = None;
            results.push(json!({ "stream": row["stream"], "generation": generation, "host_before_ns": host_before,
                "status": "unmapped_call_time", "reason": "the call time is outside a continuous host/device anchor interval" }));
            continue;
        };
        while underflows.get(next_underflow).is_some_and(|event| *event <= call_tick) {
            segment = None;
            next_underflow += 1;
        }
        let sample_counts: Vec<u64> = row["sample_counts"].as_array().into_iter().flatten().filter_map(Json::as_u64).collect();
        if row["sob"].as_bool() == Some(true) {
            segment = row["at"].as_i64().map(|at| (generation, at, vec![0; sample_counts.len()]));
        }
        if segment.as_ref().is_some_and(|(current_generation, _, _)| *current_generation != generation) {
            segment = None;
            continue;
        }
        let Some((_, target_tick, cumulative)) = segment.as_mut() else { continue };
        if row["result"]["error"].is_string() || sample_counts.is_empty() {
            segment = None;
            continue;
        }
        let Some(accepted) = row["result"]["accepted_samples"].as_u64() else { segment = None; continue };
        if accepted == 0 || sample_counts.contains(&0) {
            results.push(json!({ "stream": row["stream"], "generation": generation, "host_before_ns": host_before,
                "status": "discarded_zero_send", "accepted_samples": accepted, "sample_counts": sample_counts }));
            segment = None;
            continue;
        }
        let partial = sample_counts.iter().any(|count| *count != accepted);
        if partial {
            results.push(json!({ "stream": row["stream"], "generation": generation, "host_before_ns": host_before,
                "status": "discarded_partial_send", "accepted_samples": accepted, "sample_counts": sample_counts }));
            segment = None;
            continue;
        }
        for (channel, cumulative_samples) in cumulative.iter().enumerate() {
            let Some(offset) = u128::from(*cumulative_samples).checked_mul(u128::from(master_clock_hz)) else { continue };
            let Some(ticks) = offset.checked_add(u128::from(rate_hz) - 1).map(|value| value / u128::from(rate_hz)) else { continue };
            let Ok(next_tick) = i64::try_from(i128::from(*target_tick) + i128::try_from(ticks).unwrap_or(i128::MAX)) else { continue };
            match map_device_tick(anchors, next_tick, master_clock_hz) {
                Ok(mapped) => {
                    let lower = mapped.lower_ns - i64::try_from(host_before).unwrap_or(i64::MAX);
                    let upper = mapped.upper_ns - i64::try_from(host_before).unwrap_or(i64::MAX);
                    results.push(json!({ "stream": row["stream"], "channel_index": channel, "generation": generation,
                        "next_sample_tick": next_tick, "host_before_ns": host_before,
                        "headroom_ns": { "lower": lower, "upper": upper },
                        "anchor_span_ticks": mapped.anchor_span_ticks, "anchor_span_host_ns": mapped.anchor_span_host_ns,
                        "anchor_age_before_ns": mapped.before_age_ns, "anchor_age_after_ns": mapped.after_age_ns,
                        "bracket_width_ns": mapped.bracket_width_ns, "proxy": "nominal submission margin; not RF execution time" }));
                }
                Err(reason) => results.push(json!({ "stream": row["stream"], "channel_index": channel, "generation": generation,
                    "host_before_ns": host_before, "status": "unmapped", "reason": reason })),
            }
        }
        let mut cumulative_overflow = false;
        for (index, count) in sample_counts.iter().enumerate() {
            let Some(total) = cumulative.get_mut(index) else { cumulative_overflow = true; break };
            let Some(next) = total.checked_add(*count) else { cumulative_overflow = true; break };
            *total = next;
        }
        if cumulative_overflow { segment = None; continue; }
        if row["eob"].as_bool() == Some(true) { segment = None; }
    }
    results
}

fn distribution(values: &[i64]) -> Json {
    if values.is_empty() { return json!({ "count": 0, "min_ns": null, "median_ns": null, "p99_ns": null, "max_ns": null }); }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let p99 = sorted.len().saturating_mul(99).div_ceil(100).saturating_sub(1);
    json!({ "count": sorted.len(), "min_ns": sorted[0], "median_ns": sorted[sorted.len() / 2], "p99_ns": sorted[p99], "max_ns": sorted[sorted.len() - 1] })
}

fn proc_cpu_stat() -> Option<Json> {
    std::fs::read_to_string("/proc/stat").ok().and_then(|text| parse_proc_cpu_stat(&text))
}

pub fn parse_proc_cpu_stat(text: &str) -> Option<Json> {
    let line = text.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line.split_whitespace().skip(1).map(str::parse).collect::<Result<_, _>>().ok()?;
    if fields.len() < 4 { return None; }
    let total = fields.iter().take(8).try_fold(0u64, |total, value| total.checked_add(*value))?;
    let idle = fields.get(3)?.checked_add(*fields.get(4).unwrap_or(&0))?;
    Some(json!({ "total_ticks": total, "busy_ticks": total.checked_sub(idle)? }))
}

fn proc_thread_snapshot() -> Json {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else { return json!({ "status": "unavailable" }) };
    let mut rows = Vec::new();
    for task in tasks.flatten() {
        let tid = task.file_name().to_string_lossy().parse::<u64>().ok();
        let stat = std::fs::read_to_string(task.path().join("stat")).ok();
        let status = std::fs::read_to_string(task.path().join("status")).ok();
        if let (Some(tid), Some(stat), Some(status)) = (tid, stat, status) {
            if let Some(row) = parse_thread_status(tid, &stat, &status) { rows.push(row); }
        }
    }
    json!({ "threads": rows })
}

pub fn parse_thread_status(tid: u64, stat: &str, status: &str) -> Option<Json> {
    let close = stat.rfind(')')?;
    let comm = stat[..close].split_once('(')?.1;
    let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    let priority = fields.get(15)?.parse::<i64>().ok()?;
    let policy = fields.get(38)?.parse::<u32>().ok()?;
    let cpus = status.lines().find_map(|line| line.strip_prefix("Cpus_allowed_list:").map(str::trim));
    Some(json!({ "tid": tid, "name": comm, "priority": priority, "policy": policy,
        "policy_name": match policy { 0 => "OTHER", 1 => "FIFO", 2 => "RR", 3 => "BATCH", 5 => "IDLE", 6 => "DEADLINE", _ => "OTHER_OR_UNKNOWN" },
        "cpus_allowed_list": cpus }))
}

fn cpu_delta(before: Option<Json>, after: Option<Json>) -> Json {
    let (Some(before), Some(after)) = (before, after) else { return json!({ "status": "unavailable" }) };
    let Some(total) = after["total_ticks"].as_u64().zip(before["total_ticks"].as_u64()).and_then(|(a, b)| a.checked_sub(b)) else {
        return json!({ "status": "discontinuous" });
    };
    let Some(busy) = after["busy_ticks"].as_u64().zip(before["busy_ticks"].as_u64()).and_then(|(a, b)| a.checked_sub(b)) else {
        return json!({ "status": "discontinuous" });
    };
    json!({ "total_ticks": total, "busy_ticks": busy, "busy_percent": if total == 0 { Json::Null } else { json!(100.0 * busy as f64 / total as f64) } })
}

fn matching_timed_apply(probe: &ProbeDevice, e: i64) -> Option<Json> {
    probe.matching_timed_apply(e)
}

fn await_timed_apply(probe: &ProbeDevice, e: i64, manual: bool) -> Option<Json> {
    if manual { return matching_timed_apply(probe, e); }
    let deadline = Instant::now() + H4_APPLY_WAIT;
    loop {
        if let Some(row) = matching_timed_apply(probe, e) { return Some(row); }
        if Instant::now() >= deadline { return None; }
        std::thread::sleep(Wall::from_millis(1));
    }
}

/// Runs the Session-to-Provider H4 probe. `updates` is 200 on hardware and small in the fake rehearsal.
pub fn measure_h4_dispatch(probe: Arc<ProbeDevice>, updates: usize) -> Result<MeasurementGroup, String> {
    if updates == 0 || updates > H4_DISPATCH_UPDATES { return Err(format!("H4 needs 1..={H4_DISPATCH_UPDATES} timed gain updates")); }
    let trials = observation_trials(updates)?;
    let device: Arc<dyn Device> = probe.clone();
    let cpu_before = proc_cpu_stat();
    let timing = default_timing();
    let master = device.master_clock_rate();
    let window_ns = u64::try_from(timing.release_window_ns).map_err(|_| "negative release window")?;
    let window_ticks = ticks_for_ns(window_ns, master)?;
    let step_ticks = ticks_for_ns(H4_PERIOD_NS, master)?;
    let first_lead_ticks = ticks_for_ns(H4_FIRST_LEAD_NS, master)?;
    let mut group = MeasurementGroup::default();
    group.conditions.push(Condition {
        id: "h4-rx-gain-with-tx-repeat".to_owned(),
        streams: vec![stream_condition(Direction::Rx, 0, vec![0], H4_RATE_HZ as f64), stream_condition(Direction::Tx, 0, vec![0], H4_RATE_HZ as f64)],
        concurrent: vec![stream_ref(Direction::Rx, 0), stream_ref(Direction::Tx, 0)], reopened: false,
    });
    let (mut run, _dir) = h4_candidate_session(device.clone(), timing)?;
    common::past_t0(&mut run, common::ms(100));
    for key in ["radio.rx.s0.sample_rate_hz", "radio.tx.s0.sample_rate_hz"] {
        let value = ezsdr_kernel::spec::Value::num(H4_RATE_HZ as f64).ok_or("invalid H4 sample rate")?;
        let entry = run.submit(common::set(key, value), None).map_err(|error| error.to_string())?;
        if !common::admitted(&entry) { return Err(format!("H4 stream rate was not admitted for `{key}`: {:?}", entry.outcome)); }
        common::wait(&mut run, common::ms(20));
    }
    enable_tx(&mut run)?;
    let (waveform, _) = common::transmit_waveform(1_000);
    let start_anchor = probe.clock_anchor().ok().map(|(device_tick, host_before_ns, host_after_ns)| ClockAnchor { device_tick, host_before_ns, host_after_ns });
    start_repeat(&mut run, &waveform)?;
    let first = probe.time_now().map_err(|error| error.message)?;
    let first_e = first.checked_add(window_ticks).and_then(|tick| tick.checked_add(first_lead_ticks)).ok_or("H4 first target overflow")?;
    let manual = device.host().is_manual();
    let mut dispatch_rows = Vec::with_capacity(updates);
    let mut thread_snapshot = None;
    let mut stop_after: Option<String> = None;
    for index in 0..updates {
        let e = first_e.checked_add(i64::try_from(index).map_err(|_| "H4 update index overflow")?.checked_mul(step_ticks).ok_or("H4 update offset overflow")?)
            .ok_or("H4 target overflow")?;
        let gain = if index % 2 == 0 { 1.0 } else { 0.0 };
        if let Some(reason) = &stop_after {
            dispatch_rows.push(json!({ "index": index, "direction": "rx", "channel": 0, "setting": "gain_db", "e": e,
                "requested_gain_db": gain, "status": "not_submitted", "reason": reason }));
            continue;
        }
        let anchor_before = probe.clock_anchor().ok().map(|(device_tick, host_before_ns, host_after_ns)| ClockAnchor { device_tick, host_before_ns, host_after_ns });
        let action = h4_gain_action(TimePoint::new(run.now().domain(), e), gain);
        let entry = match run.submit(action, None) {
            Ok(entry) => entry,
            Err(error) => {
                stop_after = Some(error.to_string());
                dispatch_rows.push(json!({ "index": index, "direction": "rx", "channel": 0, "setting": "gain_db", "e": e,
                    "requested_gain_db": gain, "status": "submit_error", "error": error.to_string(), "apply_called": false }));
                continue;
            }
        };
        if !common::admitted(&entry) {
            stop_after = Some(format!("timed gain update was not admitted: {:?}", entry.outcome));
            dispatch_rows.push(json!({ "index": index, "direction": "rx", "channel": 0, "setting": "gain_db", "e": e,
                "requested_gain_db": gain, "status": "rejected", "apply_called": false, "outcome": format!("{:?}", entry.outcome) }));
            continue;
        }
        if manual {
            let root = run.now().domain();
            advance_root(&mut run, root, e)?;
        }
        let apply = await_timed_apply(&probe, e, manual);
        let after = probe.clock_anchor().ok().map(|(device_tick, host_before_ns, host_after_ns)| ClockAnchor { device_tick, host_before_ns, host_after_ns });
        let session_action = match &entry.action {
            ezsdr_kernel::session::SessionAction::Vocabulary { ns, verb, .. } => format!("{}.{}", ns.as_str(), verb.as_str()),
            _ => "unexpected-session-action".to_owned(),
        };
        let mut row = json!({ "index": index, "direction": "rx", "channel": 0, "setting": "gain_db", "e": e,
            "release_window_ns": window_ns, "requested_gain_db": gain, "accepted": true, "session_action": session_action,
            "apply_called": apply.is_some(), "apply": apply.clone() });
        row["clock_anchors"] = json!({ "before": clock_anchor_json(anchor_before), "after": clock_anchor_json(after) });
        if thread_snapshot.is_none() && apply.is_some() { thread_snapshot = Some(proc_thread_snapshot()); }
        if let (Some(anchor_before), Some(anchor_after), Some(apply_row)) = (anchor_before, after, apply.as_ref()) {
            let anchors = [anchor_before, anchor_after];
            match dispatch_delay(apply_row["host_entry_ns"].as_u64().unwrap_or_default(), e, window_ticks, anchors, master) {
                Ok((lower, upper, mapped)) => {
                    row["dispatch_delay_ns"] = json!({ "lower": lower, "upper": upper });
                    row["clock_mapping"] = json!({ "status": "mapped", "release_host_interval_ns": [mapped.lower_ns, mapped.upper_ns],
                        "anchor_span_ticks": mapped.anchor_span_ticks, "anchor_span_host_ns": mapped.anchor_span_host_ns,
                        "anchor_before_age_ns": mapped.before_age_ns, "anchor_after_age_ns": mapped.after_age_ns,
                        "bracket_width_ns": mapped.bracket_width_ns, "rate_assumption": "host and device clocks are treated as affine between these anchors (INFERRED)" });
                }
                Err(reason) => row["clock_mapping"] = json!({ "status": "unavailable", "reason": reason }),
            }
        } else {
            row["clock_mapping"] = json!({ "status": "unavailable", "reason": "timed apply or bracketed time_now anchor was missing" });
        }
        if apply.is_none() {
            stop_after = Some("timed apply was not observed; later updates were not submitted".to_owned());
        } else if apply.as_ref().is_some_and(|row| row["result"]["error"].is_string()) {
            stop_after = Some("timed apply failed; later updates were not submitted".to_owned());
        }
        dispatch_rows.push(row);
    }
    let root = run.now().domain();
    let manifest = run.finish();
    let headroom_after = probe.clock_anchor().ok().map(|(device_tick, host_before_ns, host_after_ns)| ClockAnchor { device_tick, host_before_ns, host_after_ns });
    let probe_rows = probe.rows();
    let probe_rows_dropped = probe.dropped_rows();
    let tx_rows: Vec<Json> = probe_rows.iter().filter(|row| row["call"] == "tx_send").cloned().collect();
    let time_now_rows: Vec<Json> = probe_rows.iter().filter(|row| row["call"] == "time_now").cloned().collect();
    let tx_underflows: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == "radio.TX_UNDERFLOW").collect();
    let mut underflow_ticks = Vec::new();
    let mut underflow_unconvertible = Vec::new();
    for event in &tx_underflows {
        match super::point_root_tick(event.time, &manifest.clocks.sample_clocks) {
            Ok(tick) => underflow_ticks.push(tick),
            Err(error) => underflow_unconvertible.push(error),
        }
    }
    let tx_clock = manifest.clocks.sample_clocks.iter().rev().find(|clock| clock.stream.to_string().ends_with("usrp/tx/s0"));
    let tx_sample_clock_underflows = tx_underflows.iter().filter(|event| manifest.clocks.sample_clocks.iter()
        .any(|clock| clock.stream.to_string().ends_with("usrp/tx/s0") && clock.domain == event.time.domain())).count();
    let mut tx_start_tick = tx_clock.and_then(|clock| (clock.root == root).then(|| clock.origin.ticks_in(root).ok()).flatten());
    let tx_end_tick = tx_clock.and_then(|clock| (clock.root == root).then(|| clock.ended_at.and_then(|end| end.ticks_in(root).ok())).flatten());
    let run_end_tick = manifest.termination.at.and_then(|end| end.ticks_in(root).ok());
    let mut tx_end_ticks = underflow_ticks.clone();
    tx_end_ticks.extend(tx_end_tick);
    tx_end_ticks.extend(run_end_tick);
    // The repeat's normal stop is an end-of-burst send with no samples (the device may count it as one): not a failed payload send.
    let incomplete_tx_sends = tx_rows.iter().filter(|row| !(row["eob"] == true && !row["result"]["error"].is_string() && row["sample_counts"].as_array().is_some_and(|counts| counts.iter().all(|count| count.as_u64() == Some(0))))).filter(|row| {
        let accepted = row["result"]["accepted_samples"].as_u64();
        let counts = row["sample_counts"].as_array();
        row["result"]["error"].is_string() || accepted.is_none_or(|accepted| accepted == 0
            || counts.is_none_or(|counts| counts.is_empty() || counts.iter().any(|count| count.as_u64() != Some(accepted))))
    }).count();
    let tx_start_send_ns = tx_rows.iter().find(|row| row["sob"] == true && row["result"]["accepted_samples"].as_u64().is_some_and(|accepted| {
        accepted > 0 && row["sample_counts"].as_array().is_some_and(|counts| !counts.is_empty() && counts.iter().all(|count| count.as_u64() == Some(accepted)))
    })).and_then(|row| row["host_after_ns"].as_u64()).and_then(|ns| i64::try_from(ns).ok());
    let mut tx_interval_ambiguous = !underflow_unconvertible.is_empty() || tx_start_tick.is_none();
    let mut tx_mapping_errors = Vec::new();
    let tx_anchors = start_anchor.zip(headroom_after);
    if let (Some((start, end)), Some(send_ns)) = (tx_anchors, tx_start_send_ns) {
        if let Some(send_tick) = host_to_device_tick([start, end], send_ns as u64, master) {
            tx_start_tick = tx_start_tick.map(|tick| tick.max(send_tick));
        } else {
            tx_interval_ambiguous = true;
            tx_mapping_errors.push("the first complete TX send could not be mapped to root time".to_owned());
        }
    }
    if let Some((start, end)) = tx_anchors {
        for row in tx_rows.iter().filter(|row| {
            row["result"]["error"].is_string() || row["eob"] == true || row["result"]["accepted_samples"].as_u64().is_none_or(|accepted| {
                accepted == 0 || row["sample_counts"].as_array().is_none_or(|counts| counts.is_empty() || counts.iter().any(|count| count.as_u64() != Some(accepted)))
            })
        }) {
            if let Some(ns) = row["host_after_ns"].as_u64() {
                if let Some(tick) = host_to_device_tick([start, end], ns, master) { tx_end_ticks.push(tick); }
                else { tx_interval_ambiguous = true; tx_mapping_errors.push("a TX send end could not be mapped to root time".to_owned()); }
            }
        }
    } else {
        tx_interval_ambiguous = true;
        tx_mapping_errors.push("the enclosing host/device clock anchors were unavailable".to_owned());
    }
    let tx_end_tick = tx_end_ticks.into_iter().min();
    if tx_start_tick.is_none() || tx_end_tick.is_none() { tx_interval_ambiguous = true; }
    if !underflow_unconvertible.is_empty() || !tx_mapping_errors.is_empty() { tx_interval_ambiguous = true; }
    let headroom_rows = match (start_anchor, headroom_after) {
        (Some(start), Some(end)) if underflow_unconvertible.is_empty() => tx_headroom(&tx_rows, [start, end], master, H4_RATE_HZ, &underflow_ticks),
        _ => Vec::new(),
    };
    let mut successful_delays = Vec::new();
    let (mut concurrent_dispatch_count, mut after_tx_end_count, mut overlap_ambiguous_count) = (0, 0, 0);
    let mut before_tx_count = 0;
    let mut successful_apply_count = 0;
    for row in &mut dispatch_rows {
        if row["apply_called"] != true || row["apply"]["result"]["error"].is_string() { continue; }
        successful_apply_count += 1;
        let Some(entry_ns) = row["apply"]["host_entry_ns"].as_u64() else {
            row["tx_overlap"] = json!("ambiguous");
            overlap_ambiguous_count += 1;
            continue;
        };
        let apply_tick = tx_anchors.and_then(|(start, end)| host_to_device_tick([start, end], entry_ns, master));
        let overlap = match (tx_interval_ambiguous, row["dispatch_delay_ns"].is_object(), apply_tick, tx_start_tick, tx_end_tick) {
            (false, true, Some(apply), Some(start), Some(_)) if apply < start => "before_tx",
            (false, true, Some(apply), Some(_), Some(end)) if apply < end => "concurrent",
            (false, true, Some(_), Some(_), Some(_)) => "after_tx_end",
            _ => "ambiguous",
        };
        row["tx_overlap"] = json!(overlap);
        match overlap {
            "concurrent" if row["dispatch_delay_ns"].is_object() => {
                concurrent_dispatch_count += 1;
                successful_delays.push((row["dispatch_delay_ns"]["lower"].as_i64().unwrap_or_default(), row["dispatch_delay_ns"]["upper"].as_i64().unwrap_or_default()));
            }
            "after_tx_end" => after_tx_end_count += 1,
            "ambiguous" | "concurrent" => overlap_ambiguous_count += 1,
            "before_tx" => before_tx_count += 1,
            _ => (),
        }
    }
    let delay_lower: Vec<i64> = successful_delays.iter().map(|(lower, _)| *lower).collect();
    let delay_upper: Vec<i64> = successful_delays.iter().map(|(_, upper)| *upper).collect();
    let headroom_lower: Vec<i64> = headroom_rows.iter().filter_map(|row| row["headroom_ns"]["lower"].as_i64()).collect();
    let headroom_upper: Vec<i64> = headroom_rows.iter().filter_map(|row| row["headroom_ns"]["upper"].as_i64()).collect();
    let cpu_after = proc_cpu_stat();
    let accepted_count = dispatch_rows.iter().filter(|row| row["accepted"] == true).count();
    let rejected_count = dispatch_rows.iter().filter(|row| row["status"] == "rejected").count();
    let submit_error_count = dispatch_rows.iter().filter(|row| row["status"] == "submit_error").count();
    let apply_called_count = dispatch_rows.iter().filter(|row| row["apply_called"] == true).count();
    let apply_not_called_count = dispatch_rows.iter().filter(|row| row["accepted"] == true && row["apply_called"] == false).count();
    let not_submitted_count = dispatch_rows.iter().filter(|row| row["status"] == "not_submitted").count();
    let apply_error_count = dispatch_rows.iter().filter(|row| row["apply"]["result"]["error"].is_string()).count();
    let tx_send_count = tx_rows.len();
    let tx_end_reason = if !underflow_unconvertible.is_empty() { "unconvertible_tx_underflow" }
        else if !underflow_ticks.is_empty() { "tx_underflow" }
        else if incomplete_tx_sends > 0 { "incomplete_tx_send" }
        else { "run_termination" };
    let partial = stop_after.is_some() || tx_end_tick.is_none() || tx_start_tick.is_none() || tx_interval_ambiguous
        || incomplete_tx_sends > 0 || !underflow_ticks.is_empty() || probe_rows_dropped > 0
        || concurrent_dispatch_count != successful_apply_count;
    let mut result = json!({
        "status": if partial { "partial" } else { "complete" },
        "timed_updates": dispatch_rows, "requested_update_count": updates,
        "master_clock_hz": master, "sample_rate_hz": H4_RATE_HZ,
        "release_window_ns": window_ns, "release_window_ticks": window_ticks,
        "accepted_count": accepted_count, "rejected_count": rejected_count,
        "submit_error_count": submit_error_count, "apply_called_count": apply_called_count,
        "apply_not_called_count": apply_not_called_count, "apply_error_count": apply_error_count,
        "not_submitted_count": not_submitted_count, "stop_reason": stop_after,
        "dispatch_delay_lower_ns": distribution(&delay_lower), "dispatch_delay_upper_ns": distribution(&delay_upper),
    });
    let mut tx_result = json!({
        "concurrent_dispatch_update_count": concurrent_dispatch_count,
        "dispatch_updates_after_tx_end_count": after_tx_end_count,
        "dispatch_updates_before_tx_start_count": before_tx_count,
        "dispatch_updates_overlap_ambiguous_count": overlap_ambiguous_count,
        "dispatch_delay_proxy": "uhd-control timed-apply entry minus the host-mapped release-window instant; not scheduler wake-up latency",
        "tx_send_count": tx_send_count, "tx_send_records": tx_rows, "tx_headroom": headroom_rows,
        "probe_rows_retained_count": probe_rows.len(), "probe_rows_dropped_count": probe_rows_dropped,
        "time_now_records": time_now_rows,
        "tx_headroom_lower_ns": distribution(&headroom_lower), "tx_headroom_upper_ns": distribution(&headroom_upper),
        "tx_headroom_proxy": "nominal next-sample submission margin; not RF execution time",
        "tx_underflow_events": tx_underflows.len(), "tx_underflow_converted_events": underflow_ticks.len(),
        "tx_underflow_sample_clock_events": tx_sample_clock_underflows,
        "tx_underflow_unconvertible_events": underflow_unconvertible.len(), "tx_underflow_unconvertible_reasons": underflow_unconvertible,
        "tx_active_root_start_tick": tx_start_tick, "tx_end_root_tick": tx_end_tick,
        "tx_end_reason": tx_end_reason, "tx_end_mapping_errors": tx_mapping_errors,
        "thread_policy_affinity": thread_snapshot.unwrap_or_else(proc_thread_snapshot),
        "cpu_stat_during_probe": cpu_delta(cpu_before, cpu_after),
        "clock_note": "host/device affine mapping is INFERRED; anchors must agree with the master clock within 1,000 ppm plus measured bracket uncertainty; bracket widths are not a long-horizon error bound",
    });
    result.as_object_mut().unwrap().append(tx_result.as_object_mut().unwrap());
    let observations = vec![observation("hw_h4_dispatch", "tests/hardware.rs::hw_h4_dispatch", Some("h4-rx-gain-with-tx-repeat".to_owned()),
        trials, result)?];
    group.observations = observations;
    Ok(group)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchors() -> [ClockAnchor; 2] {
        [ClockAnchor { device_tick: 0, host_before_ns: 90, host_after_ns: 110 }, ClockAnchor { device_tick: 200, host_before_ns: 1_090, host_after_ns: 1_110 }]
    }

    #[test]
    fn h4_dispatch_maps_signed_intervals_and_refuses_bad_anchors() {
        let (lower, upper, mapped) = dispatch_delay(490, 100, 0, anchors(), 200_000_000).unwrap();
        assert!(lower < 0 && upper < 0, "negative dispatch delay is retained: {lower}..{upper}");
        assert!(lower <= upper, "delay interval endpoints stay ordered: {lower}..{upper}");
        assert!(mapped.bracket_width_ns > 0);
        assert!(dispatch_delay(700, 100, 0, anchors(), 200_000_000).unwrap().0 > 0);
        assert!(map_device_tick([anchors()[1], anchors()[0]], 100, 200_000_000).is_err());
        assert!(map_device_tick(anchors(), 201, 200_000_000).is_err());
        let stalled = [anchors()[0], ClockAnchor { device_tick: 0, host_before_ns: 1_090, host_after_ns: 1_110 }];
        assert!(map_device_tick(stalled, 0, 200_000_000).is_err(), "a stopped device clock is not an affine mapping");
    }

    #[test]
    fn h4_mapping_rejects_a_monotonic_forward_clock_jump() {
        let jumped = [anchors()[0], ClockAnchor { device_tick: 1_200, host_before_ns: 1_090, host_after_ns: 1_110 }];
        assert!(map_device_tick(jumped, 500, 200_000_000).unwrap_err().contains("1000 ppm"));
        assert!(host_to_device_tick(jumped, 600, 200_000_000).is_none());
    }

    #[test]
    fn h4_tx_headroom_uses_per_channel_cumulative_accepts_and_breaks_bad_segments() {
        let rows = vec![
            json!({ "call": "tx_send", "stream": 0, "generation": 1, "host_before_ns": 100, "at": 20, "sob": true, "eob": false,
                "sample_counts": [4, 4], "result": { "accepted_samples": 4 } }),
            json!({ "call": "tx_send", "stream": 0, "generation": 1, "host_before_ns": 120, "at": null, "sob": false, "eob": false,
                "sample_counts": [4, 4], "result": { "accepted_samples": 4 } }),
            json!({ "call": "tx_send", "stream": 0, "generation": 1, "host_before_ns": 140, "at": null, "sob": false, "eob": false,
                "sample_counts": [4, 4], "result": { "accepted_samples": 3 } }),
            json!({ "call": "tx_send", "stream": 0, "generation": 2, "host_before_ns": 160, "at": null, "sob": false, "eob": false,
                "sample_counts": [4, 4], "result": { "accepted_samples": 4 } }),
        ];
        let observed = tx_headroom(&rows, anchors(), 200_000_000, 10_000_000, &[]);
        assert_eq!(observed.len(), 5, "two channels for each of the first two complete sends and a partial-send record");
        assert_eq!(observed[0]["next_sample_tick"], 20);
        assert_eq!(observed[2]["next_sample_tick"], 100, "four accepted samples per channel advance by 80 device ticks");
        assert_eq!(observed[4]["status"], "discarded_partial_send");
        let after_underflow = tx_headroom(&rows, anchors(), 200_000_000, 10_000_000, &[3]);
        assert_eq!(after_underflow.len(), 2, "the underflow breaks the segment before its next send");
    }

    #[test]
    fn h4_tx_headroom_restarts_after_an_error_and_eob() {
        let rows = vec![
            json!({ "call": "tx_send", "stream": 0, "generation": 1, "host_before_ns": 100, "at": 20, "sob": true, "eob": false,
                "sample_counts": [4], "result": { "accepted_samples": 4 } }),
            json!({ "call": "tx_send", "stream": 0, "generation": 1, "host_before_ns": 120, "at": null, "sob": false, "eob": false,
                "sample_counts": [4], "result": { "error": "scripted failure", "accepted_samples": 4 } }),
            json!({ "call": "tx_send", "stream": 0, "generation": 1, "host_before_ns": 140, "at": null, "sob": false, "eob": false,
                "sample_counts": [4], "result": { "accepted_samples": 4 } }),
            json!({ "call": "tx_send", "stream": 0, "generation": 2, "host_before_ns": 160, "at": 40, "sob": true, "eob": true,
                "sample_counts": [4], "result": { "accepted_samples": 4 } }),
            json!({ "call": "tx_send", "stream": 0, "generation": 2, "host_before_ns": 180, "at": null, "sob": false, "eob": false,
                "sample_counts": [4], "result": { "accepted_samples": 4 } }),
        ];
        let observed = tx_headroom(&rows, anchors(), 200_000_000, 10_000_000, &[]);
        assert_eq!(observed.len(), 2, "the error and EOB each end the segment until a new SOB");
        assert_eq!(observed[0]["next_sample_tick"], 20);
        assert_eq!(observed[1]["next_sample_tick"], 40);
    }

    #[test]
    fn h4_tx_headroom_discards_a_discontinuous_host_device_mapping() {
        let rows = vec![json!({ "call": "tx_send", "stream": 0, "generation": 1, "host_before_ns": 100, "at": 20, "sob": true, "eob": false,
            "sample_counts": [4], "result": { "accepted_samples": 4 } })];
        let mut broken = anchors();
        broken[1].device_tick = -1;
        let observed = tx_headroom(&rows, broken, 200_000_000, 10_000_000, &[]);
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0]["status"], "unmapped_call_time");
    }

    #[test]
    fn h4_zero_sample_send_breaks_the_headroom_segment() {
        let rows = vec![
            json!({ "call": "tx_send", "stream": 0, "generation": 1, "host_before_ns": 100, "at": 20, "sob": true, "eob": false,
                "sample_counts": [4], "result": { "accepted_samples": 0 } }),
            json!({ "call": "tx_send", "stream": 0, "generation": 1, "host_before_ns": 120, "at": 30, "sob": true, "eob": false,
                "sample_counts": [4], "result": { "accepted_samples": 4 } }),
        ];
        let observed = tx_headroom(&rows, anchors(), 200_000_000, 10_000_000, &[]);
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0]["status"], "discarded_zero_send");
        assert_eq!(observed[1]["next_sample_tick"], 30, "the zero send does not advance the burst cursor");
    }

    #[test]
    fn h4_proc_parsers_retain_cpu_and_thread_policy_fields() {
        assert_eq!(parse_proc_cpu_stat("cpu  100 2 3 40 5 6 7 8 9 10\ncpu0 1 2").unwrap()["total_ticks"], 171);
        let fields = (0..38).map(|index| if index == 14 { "-5" } else if index == 37 { "2" } else { "0" }).collect::<Vec<_>>().join(" ");
        let stat = format!("123 (uhd-tx) R {fields}");
        let parsed = parse_thread_status(123, &stat, "Name:\tuhd-tx\nCpus_allowed_list:\t1-3\n").unwrap();
        assert_eq!((parsed["priority"].as_i64(), parsed["policy"].as_u64(), parsed["cpus_allowed_list"].as_str()), (Some(-5), Some(2), Some("1-3")));
    }
}
