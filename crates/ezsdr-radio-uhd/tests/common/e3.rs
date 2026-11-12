//! Note 49 stage E3a's lifted bench bodies (AGENTS §9: a hardware test whose expectations come from a rule has a rehearsal on the
//! FakeDevice). Each body is the part of a `hw_*` test that runs the Session and reads its sends, taking the Session (or the probe) it runs
//! on, so `hardware.rs` runs it on the USRP and `fake.rs` on `FakeDevice`; the verdicts are the pure ones of `iii.rs`. The rehearsals assert
//! what the booked instants fix (the sizes, the flags, the booked `at`), never a wall-clock window (note 51 §3).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use ezsdr_kernel::coordinator::RunHandle;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::time::TimePoint;
use serde_json::Value as Json;

use super::iii;
use super::n47s5::{data_sends, transmit_root};
use super::probe_device::ProbeDevice;
use super::raw::tone_wave;
use super::tone::Verdict;
use super::*;

/// `hw_s1_short_first_sends` for one case: a burst of `samples` samples (1 to 4) at 40 ms, then a 10 000-sample burst 10 000 samples later, in
/// the transmit Session `run`. The verdict is `iii::short_first_send_verdict`.
pub fn short_first_case(mut run: RunHandle, samples: usize) -> Verdict {
    let (short, _) = waveform_of(&[(0.4f32, 0.0f32); 4][..samples]);
    let (long, _) = waveform_of(&tone_wave(1e6, 10_000, 100e3));
    let a = tx_at(&run, ms(40));
    let b = TimePoint::new(a.domain(), a.ticks_in(a.domain()).unwrap() + 10_000);
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&short)).unwrap()));
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(b), &[]), Some(&long)).unwrap()));
    wait(&mut run, ms(200));
    let long_root = transmit_root(&run, &b);
    let manifest = run.finish();
    let acks: Vec<i64> =
        section(&manifest, "async").as_array().into_iter().flatten().filter(|row| row["code"] == "BurstAck" && row["tx_flush"] != true).filter_map(|row| row["tick"].as_i64()).collect();
    let (before, after) = (acks.iter().filter(|tick| **tick < long_root).count(), acks.iter().filter(|tick| **tick >= long_root).count());
    println!(
        "S1 first send of {samples} samples: TIME_ERROR {:?}; TX_UNDERFLOW {}; async {}; bursts {}",
        events_of(&manifest, "radio.TIME_ERROR").iter().map(|e| e.payload.clone()).collect::<Vec<_>>(),
        events_of(&manifest, "radio.TX_UNDERFLOW").len(),
        section(&manifest, "async"),
        section(&manifest, "bursts")
    );
    iii::short_first_send_verdict(samples, iii::run_faults(&manifest), before, after)
}

/// `hw_txp_b0_send_sizes` for one rate: a 31 000-sample repeat in the transmit Session `run` (whose profile asked the rate and gain 0 dB at
/// connect), repeating for 5 s and stopped. The verdict is `iii::repeat_sizes_verdict` with the `chunk` the window allows, and the faults.
pub fn repeat_sizes_case(mut run: RunHandle, probe: &Arc<ProbeDevice>, rate: f64, chunk: usize) -> Verdict {
    past_t0(&mut run, ms(1));
    past_tx_origin(&mut run);
    let wave: Vec<_> = pn(31_000).into_iter().map(|(re, im)| (re * 0.7, im * 0.7)).collect();
    assert!(wave.iter().all(|(re, im)| f64::from(*re).hypot(f64::from(*im)) <= 0.5), "bench RF safety");
    let (bytes, _) = waveform_of(&wave);
    let baseline = probe.count("tx_send");
    assert!(admitted(&run.submit(verb("start_repeat", resource("radio/tx/s0"), None, &[]), Some(&bytes)).unwrap()));
    wait(&mut run, ms(5_000));
    let _ = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None);
    wait(&mut run, ms(200));
    let manifest = run.finish();
    let rows: Vec<Json> = probe.rows().into_iter().filter(|row| row["call"] == "tx_send").skip(baseline).collect();
    let sizes: Vec<usize> = rows.iter().filter(|row| row["tx_flush"] != true && row["eob"] != true).filter_map(|row| row["sample_counts"][0].as_u64()).filter(|n| *n > 0).map(|n| n as usize).collect();
    let closing: Vec<usize> = rows.iter().filter(|row| row["tx_flush"] != true && row["eob"] == true).filter_map(|row| row["sample_counts"][0].as_u64()).map(|n| n as usize).collect();
    println!("B0 sizes rate {rate}: {} payload sends, sizes {sizes:?}; closing sends {closing:?}", sizes.len());
    iii::all_of(vec![iii::repeat_sizes_verdict(&sizes, &closing, chunk, 31_000), iii::faults_verdict(&[("repeat", iii::run_faults(&manifest), 0)])])
}

/// The Spec of `hw_p72_timed_tx_chunks_are_timed_and_flushed`: a 60 000-sample burst at 10 000 ticks (SendAsap), received on one channel with a
/// 90 000-sample capture, at carrier `hz`; and its input.
pub fn timed_chunks_spec(hz: f64) -> (Json, BTreeMap<ContentHash, Vec<u8>>) {
    let (bytes, waveform) = waveform_of(&tone_wave(1e6, 60_000, 100e3));
    let spec = with_burst(with_tx(receive_spec(1, 1e6, hz, Some(90_000)), 1e6), &waveform, false, "send_asap_and_flag", 10_000);
    (spec, BTreeMap::from([(waveform.hash.clone(), bytes)]))
}

/// T3's evidence in a Manifest: how many flushes were acknowledged (the async section), and the flushes refused.
pub fn flush_evidence(manifest: &Manifest) -> (usize, Vec<Json>) {
    let acked = section(manifest, "async").as_array().unwrap().iter().filter(|row| row["tx_flush"] == true && row["code"] == "BurstAck").count();
    let refused = section(manifest, "rejected").as_array().unwrap().iter().filter(|row| row["call"] == "tx_flush").cloned().collect();
    (acked, refused)
}

/// What `hw_p72_…` reads off a finished Run: the verdict (`iii::timed_sends_verdict`) and the closing sends' sample counts per channel.
pub struct TimedChunks {
    pub verdict: Verdict,
    pub closing: Vec<Json>,
}

pub fn timed_chunks_case(probe: &ProbeDevice, manifest: &Manifest) -> TimedChunks {
    let sends = data_sends(probe);
    let payload = sends.iter().filter(|row| row["sample_counts"][0].as_u64().unwrap_or(0) > 0).count();
    let untimed: Vec<&Json> = sends.iter().filter(|row| row["at"].is_null()).collect();
    let closing: Vec<Json> = sends.iter().filter(|row| row["eob"] == true).map(|row| row["sample_counts"].clone()).collect();
    let short: Vec<&Json> = sends.iter().filter(|row| row["eob"] == true && row["sample_counts"].as_array().is_some_and(|counts| counts.iter().any(|n| n.as_u64().unwrap_or(0) < 2))).collect();
    let (acked, refused) = flush_evidence(manifest);
    let flushes = probe.rows().iter().filter(|row| row["call"] == "tx_send" && row["tx_flush"] == true).count();
    println!("#72 1a: {payload} sends with samples; closing sends {closing:?}; flushes sent {flushes}, acknowledged {acked}, refused {refused:?}");
    let faults = iii::faults_verdict(&[("60 ms device burst", iii::run_faults(manifest), 0)]);
    let counts = iii::TimedSends { refused: refused.len(), untimed: untimed.len(), short_closing: short.len(), payload_sends: payload, closing: closing.len(), flushes, acked };
    TimedChunks { verdict: iii::timed_sends_verdict(faults, &counts), closing }
}

/// A trial of `hw_p70_one_sample_burst_then_timed_burst` as far as the fake shows it: `samples` samples at 40 ms, a 10 000-sample burst 5 000
/// samples after it, booked with it. Returns the device sends in order and the root ticks of the two booked instants (the bench's capture and
/// onset read the tone, which the fake does not show).
pub fn one_sample_pair(mut run: RunHandle, probe: &ProbeDevice, samples: usize) -> (Vec<Json>, i64, i64) {
    let (first, _) = waveform_of(&[(0.4f32, 0.0f32); 4][..samples]);
    let (tone, _) = waveform_of(&tone_wave(1e6, 10_000, 100e3));
    let a = tx_at(&run, ms(40));
    let b = TimePoint::new(a.domain(), a.ticks_in(a.domain()).unwrap() + 5_000);
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&first)).unwrap()));
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(b), &[]), Some(&tone)).unwrap()));
    let (a_root, b_root) = (transmit_root(&run, &a), transmit_root(&run, &b));
    wait(&mut run, ms(200));
    let _ = run.finish();
    (data_sends(probe), a_root, b_root)
}
