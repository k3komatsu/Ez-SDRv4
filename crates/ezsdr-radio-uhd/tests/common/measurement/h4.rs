//! H4 transmit headroom and load. These are observations, not wake-up latency or RF execution.
//! The Session asks for receive and transmit c0 at 10 MS/s at connect and repeats a burst with no
//! Run-time update (note 47 stage 2); the dispatch proxy of timed gain updates is gone.

use super::super as common;
use super::*;
use common::probe_device::ProbeDevice;

pub const H4_WINDOW_MS: i64 = 2_000;
const H4_RATE_HZ: u64 = 10_000_000;
const CLOCK_RATE_TOLERANCE_PPM: i128 = 1_000;

/// Receive and transmit c0 in stream 0, both at 10 MS/s, asked at connect (RM-30).
fn h4_initial(_profile: &Json) -> ezsdr_kernel::session::InitialRequests {
    let rate = ezsdr_kernel::spec::Scalar::try_from(H4_RATE_HZ as f64).expect("a finite rate");
    let requests = ["radio.rx.c0.stream", "radio.tx.c0.stream"].map(|key| (Key::parse(key).unwrap(), ezsdr_kernel::spec::Scalar::from(0)));
    let rates = ["radio.rx.s0.sample_rate_hz", "radio.tx.s0.sample_rate_hz"].map(|key| (Key::parse(key).unwrap(), rate.clone()));
    BTreeMap::from([(ezsdr_kernel::spec::Ident::parse("radio").unwrap(), requests.into_iter().chain(rates).collect())])
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
    let device_span = after.device_tick.checked_sub(before.device_tick).filter(|span| *span > 0).ok_or_else(|| "device-time anchors are missing, reversed or discontinuous".to_owned())?;
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
    let bracket = left_weight * i128::from(before.host_after_ns - before.host_before_ns) + right_weight * i128::from(after.host_after_ns - after.host_before_ns);
    let radius = (bracket + 2 * i128::from(device_span) - 1) / (2 * i128::from(device_span)) + 1;
    let lower = i64::try_from(target - radius).map_err(|_| "mapped host time underflow".to_owned())?;
    let upper = i64::try_from(target + radius).map_err(|_| "mapped host time overflow".to_owned())?;
    let midpoint_ns = i64::try_from(target).map_err(|_| "mapped host midpoint overflow".to_owned())?;
    let before_age_ns = u64::try_from((target - h0).abs()).map_err(|_| "anchor age overflow".to_owned())?;
    let after_age_ns = u64::try_from((h1 - target).abs()).map_err(|_| "anchor age overflow".to_owned())?;
    let bracket_width_ns = u64::try_from(i128::from(upper) - i128::from(lower)).map_err(|_| "mapped bracket width overflow".to_owned())?;
    Ok(HostInterval {
        lower_ns: lower,
        upper_ns: upper,
        midpoint_ns,
        bracket_width_ns,
        anchor_span_ticks: device_span,
        anchor_span_host_ns: i64::try_from(host_span).map_err(|_| "host anchor span overflow".to_owned())?,
        before_age_ns,
        after_age_ns,
    })
}

fn clock_rate_matches(anchors: [ClockAnchor; 2], master_clock_hz: u64) -> bool {
    if master_clock_hz == 0 {
        return false;
    }
    let midpoint = |anchor: ClockAnchor| i128::from(anchor.host_before_ns) + (i128::from(anchor.host_after_ns) - i128::from(anchor.host_before_ns)) / 2;
    let host_span = midpoint(anchors[1]) - midpoint(anchors[0]);
    let device_span = i128::from(anchors[1].device_tick) - i128::from(anchors[0].device_tick);
    if host_span <= 0 || device_span <= 0 {
        return false;
    }
    let Some(expected) = host_span.checked_mul(i128::from(master_clock_hz)) else { return false };
    let Some(actual) = device_span.checked_mul(1_000_000_000) else { return false };
    let Some(difference) = actual.checked_sub(expected).and_then(i128::checked_abs) else { return false };
    let Some(tolerance) = expected.checked_mul(CLOCK_RATE_TOLERANCE_PPM).map(|value| value / 1_000_000) else { return false };
    let bracket_ns = (i128::from(anchors[0].host_after_ns - anchors[0].host_before_ns) + i128::from(anchors[1].host_after_ns - anchors[1].host_before_ns)) / 2;
    let Some(bracket_tolerance) = bracket_ns.checked_mul(i128::from(master_clock_hz)) else { return false };
    difference <= tolerance + bracket_tolerance
}

fn host_to_device_tick(anchors: [ClockAnchor; 2], host_ns: u64, master_clock_hz: u64) -> Option<i64> {
    let midpoint = |anchor: ClockAnchor| i128::from(anchor.host_before_ns) + (i128::from(anchor.host_after_ns) - i128::from(anchor.host_before_ns)) / 2;
    let h0 = midpoint(anchors[0]);
    let h1 = midpoint(anchors[1]);
    let d0 = i128::from(anchors[0].device_tick);
    let d1 = i128::from(anchors[1].device_tick);
    let span = h1 - h0;
    if anchors.iter().any(|anchor| anchor.host_before_ns > anchor.host_after_ns)
        || span <= 0
        || d1 <= d0
        || i128::from(host_ns) < h0
        || i128::from(host_ns) > h1
        || !clock_rate_matches(anchors, master_clock_hz)
    {
        return None;
    }
    i64::try_from(d0 + (i128::from(host_ns) - h0) * (d1 - d0) / span).ok()
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
        let Some(host_before) = host_before else {
            segment = None;
            continue;
        };
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
        let Some(accepted) = row["result"]["accepted_samples"].as_u64() else {
            segment = None;
            continue;
        };
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
            let Some(total) = cumulative.get_mut(index) else {
                cumulative_overflow = true;
                break;
            };
            let Some(next) = total.checked_add(*count) else {
                cumulative_overflow = true;
                break;
            };
            *total = next;
        }
        if cumulative_overflow {
            segment = None;
            continue;
        }
        if row["eob"].as_bool() == Some(true) {
            segment = None;
        }
    }
    results
}

fn distribution(values: &[i64]) -> Json {
    if values.is_empty() {
        return json!({ "count": 0, "min_ns": null, "median_ns": null, "p99_ns": null, "max_ns": null });
    }
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
    if fields.len() < 4 {
        return None;
    }
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
            if let Some(row) = parse_thread_status(tid, &stat, &status) {
                rows.push(row);
            }
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

/// Runs the H4 probe: a repeat for `window_ms` (`H4_WINDOW_MS` on hardware, short in the fake rehearsal), the transmit headroom of each send and the CPU and thread load meanwhile.
pub fn measure_h4_load(probe: Arc<ProbeDevice>, window_ms: i64) -> Result<MeasurementGroup, String> {
    let trials = observation_trials(1)?;
    let device: Arc<dyn Device> = probe.clone();
    let cpu_before = proc_cpu_stat();
    let master = device.master_clock_rate();
    let mut group = MeasurementGroup::default();
    group.conditions.push(Condition {
        id: "h4-rx-tx-repeat".to_owned(),
        streams: vec![stream_condition(Direction::Rx, 0, vec![0], H4_RATE_HZ as f64), stream_condition(Direction::Tx, 0, vec![0], H4_RATE_HZ as f64)],
        concurrent: vec![stream_ref(Direction::Rx, 0), stream_ref(Direction::Tx, 0)],
        reopened: false,
    });
    let (mut run, _dir) = candidate_session_asking(device.clone(), default_timing(), h4_initial)?;
    await_tx(&mut run);
    let (waveform, _) = common::transmit_waveform(1_000);
    let start_anchor = probe.clock_anchor().ok().map(|(device_tick, host_before_ns, host_after_ns)| ClockAnchor { device_tick, host_before_ns, host_after_ns });
    start_repeat(&mut run, &waveform)?;
    common::wait(&mut run, common::ms(window_ms));
    let manifest = run.finish();
    let end_anchor = probe.clock_anchor().ok().map(|(device_tick, host_before_ns, host_after_ns)| ClockAnchor { device_tick, host_before_ns, host_after_ns });
    let probe_rows = probe.rows();
    let probe_rows_dropped = probe.dropped_rows();
    let tx_send_rows: Vec<Json> = probe_rows.iter().filter(|row| row["call"] == "tx_send").cloned().collect();
    let tx_rows: Vec<Json> = tx_send_rows.iter().filter(|row| row["tx_flush"] != true).cloned().collect();
    let tx_underflows: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == "radio.TX_UNDERFLOW").collect();
    let mut underflow_ticks = Vec::new();
    let mut underflow_unconvertible = Vec::new();
    for event in &tx_underflows {
        match super::point_root_tick(event.time, &manifest.clocks.sample_clocks) {
            Ok(tick) => underflow_ticks.push(tick),
            Err(error) => underflow_unconvertible.push(error),
        }
    }
    let headroom_rows = match (start_anchor, end_anchor) {
        (Some(start), Some(end)) if underflow_unconvertible.is_empty() => tx_headroom(&tx_rows, [start, end], master, H4_RATE_HZ, &underflow_ticks),
        _ => Vec::new(),
    };
    let headroom_lower: Vec<i64> = headroom_rows.iter().filter_map(|row| row["headroom_ns"]["lower"].as_i64()).collect();
    let headroom_upper: Vec<i64> = headroom_rows.iter().filter_map(|row| row["headroom_ns"]["upper"].as_i64()).collect();
    let partial = start_anchor.is_none() || end_anchor.is_none() || headroom_rows.is_empty() || !underflow_ticks.is_empty() || !underflow_unconvertible.is_empty() || probe_rows_dropped > 0;
    let result = json!({
        "status": if partial { "partial" } else { "complete" },
        "master_clock_hz": master, "sample_rate_hz": H4_RATE_HZ, "window_ms": window_ms,
        "tx_send_count": tx_send_rows.len(), "tx_payload_send_count": tx_rows.len(),
        "tx_headroom": headroom_rows,
        "probe_rows_retained_count": probe_rows.len(), "probe_rows_dropped_count": probe_rows_dropped,
        "tx_headroom_lower_ns": distribution(&headroom_lower), "tx_headroom_upper_ns": distribution(&headroom_upper),
        "tx_headroom_proxy": "nominal next-sample submission margin; not RF execution time",
        "tx_underflow_events": tx_underflows.len(), "tx_underflow_converted_events": underflow_ticks.len(),
        "tx_underflow_unconvertible_events": underflow_unconvertible.len(), "tx_underflow_unconvertible_reasons": underflow_unconvertible,
        "thread_policy_affinity": proc_thread_snapshot(),
        "cpu_stat_during_probe": cpu_delta(cpu_before, proc_cpu_stat()),
        "clock_note": "host/device affine mapping is INFERRED; anchors must agree with the master clock within 1,000 ppm plus measured bracket uncertainty; bracket widths are not a long-horizon error bound",
    });
    group.observations = vec![observation("hw_h4_load", "tests/hardware.rs::hw_h4_load", Some("h4-rx-tx-repeat".to_owned()), trials, result)?];
    Ok(group)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchors() -> [ClockAnchor; 2] {
        [ClockAnchor { device_tick: 0, host_before_ns: 90, host_after_ns: 110 }, ClockAnchor { device_tick: 200, host_before_ns: 1_090, host_after_ns: 1_110 }]
    }

    #[test]
    fn h4_mapping_gives_an_ordered_interval_and_refuses_bad_anchors() {
        let mapped = map_device_tick(anchors(), 100, 200_000_000).unwrap();
        assert!(mapped.lower_ns <= mapped.midpoint_ns && mapped.midpoint_ns <= mapped.upper_ns, "{mapped:?}");
        assert!(mapped.bracket_width_ns > 0);
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
        let fields = (0..38)
            .map(|index| {
                if index == 14 {
                    "-5"
                } else if index == 37 {
                    "2"
                } else {
                    "0"
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let stat = format!("123 (uhd-tx) R {fields}");
        let parsed = parse_thread_status(123, &stat, "Name:\tuhd-tx\nCpus_allowed_list:\t1-3\n").unwrap();
        assert_eq!((parsed["priority"].as_i64(), parsed["policy"].as_u64(), parsed["cpus_allowed_list"].as_str()), (Some(-5), Some(2), Some("1-3")));
    }
}
