//! ContinuityMap to SigMF metadata (HD-15, SC-32).

use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::stream::ContinuityMap;
use ezsdr_kernel::time::Rational;

use super::{CF32_CONTRACT, SC16_CONTRACT, SIGMF_VERSION};

/// The SigMF metadata of a capture with one ContinuityMap: each run of delivered
/// samples between stream gaps is a capture segment, and the gaps, per-channel
/// validity and whether the capture is partial go in the `ezsdr` extension (HD-15,
/// SC-32).
///
/// A file index counts delivered samples from `map.first`: gaps are not in the file,
/// so a tick's index is its offset less the extent of every gap before it. A map
/// whose indices would be negative or out of range — which no `ContinuityBuilder`
/// produces, but a hand-built map can — is refused rather than described wrongly.
pub fn sigmf_meta(
    map: &ContinuityMap,
    rate: Rational,
    contract: &DataContractId,
    partial: bool,
) -> Result<serde_json::Value, String> {
    let datatype = match contract.as_str() {
        CF32_CONTRACT => "cf32_le",
        SC16_CONTRACT => "ci16_le",
        other => return Err(format!("HD-15: contract {other} has no SigMF datatype")),
    };
    // Every instant of the map is in the map's own SampleClock (SC-12).
    let tick = |t: ezsdr_kernel::time::TimePoint| {
        t.ticks_in(map.domain).map_err(|e| format!("HD-15: {e}"))
    };
    let first = i128::from(tick(map.first)?);
    let gap_end = |gap: &ezsdr_kernel::stream::Gap| -> Result<i128, String> {
        Ok(i128::from(tick(gap.start)?) + i128::from(gap.len))
    };
    let file_index = |tick: i64| -> Result<u64, String> {
        let tick = i128::from(tick);
        let skipped: i128 = map
            .gaps
            .iter()
            .filter(|gap| gap_end(gap).is_ok_and(|end| end <= tick))
            .map(|gap| i128::from(gap.len))
            .sum();
        u64::try_from(tick - first - skipped)
            .ok()
            .filter(|index| *index <= i64::MAX as u64)
            .ok_or_else(|| format!("HD-15: tick {tick} has no file index in this map"))
    };
    let segment = |tick: i64| -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "core:sample_start": file_index(tick)?, "core:global_index": tick }))
    };

    let mut captures = Vec::new();
    let mut run_start = first;
    for gap in &map.gaps {
        if i128::from(tick(gap.start)?) > run_start {
            captures.push(segment(run_start as i64)?);
        }
        run_start = gap_end(gap)?;
    }
    if i128::from(tick(map.end)?) > run_start {
        captures.push(segment(run_start as i64)?);
    }
    let gaps = map
        .gaps
        .iter()
        .map(|gap| {
            let after = i64::try_from(gap_end(gap)?).map_err(|_| {
                format!("HD-15: a gap of {} samples leaves the tick range", gap.len)
            })?;
            Ok(serde_json::json!({
                "sample_start": file_index(after)?,
                "global_index": tick(gap.start)?,
                "len": gap.len,
                "lost": gap.lost,
                "cause": gap.cause,
                "link_dropped": gap.link_dropped,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let valid = map
        .valid
        .iter()
        .map(|segments| {
            segments
                .iter()
                .map(|s| Ok(serde_json::json!({ "sample_start": file_index(tick(s.start)?)?, "sample_count": s.len })))
                .collect::<Result<Vec<_>, String>>()
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut channel_gaps = map
        .channel_gaps
        .iter()
        .map(|g| Ok((file_index(tick(g.start)?)?, g)))
        .collect::<Result<Vec<_>, String>>()?;
    // The builder emits a channel's break when it closes, so a short break on a high
    // channel can precede a long one below it; SigMF requires start order.
    channel_gaps.sort_by_key(|(index, g)| (*index, g.channel));
    let annotations: Vec<_> = channel_gaps
        .into_iter()
        .map(|(index, g)| {
            serde_json::json!({
                "core:sample_start": index,
                "core:sample_count": g.len,
                "core:label": "invalid channel",
                "ezsdr:channel": g.channel,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "global": {
            "core:datatype": datatype,
            "core:version": SIGMF_VERSION,
            "core:num_channels": map.channels,
            "core:sample_rate": rate.num() as f64 / rate.den() as f64,
            "core:recorder": "ezsdr.sink.capture 1.2.0",
            "core:extensions": [{ "name": "ezsdr", "version": "1.0.0", "optional": true }],
            "ezsdr:sample_rate": { "num": rate.num(), "den": rate.den() },
            "ezsdr:partial": partial,
            "ezsdr:gaps": gaps,
            "ezsdr:valid": valid,
        },
        "captures": captures,
        "annotations": annotations,
    }))
}
