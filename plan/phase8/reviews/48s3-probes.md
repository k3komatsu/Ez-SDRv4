# Design note 48 S3 — probe sources kept with the records

Throwaway-copy probes behind the #79 divergences recorded in issue #79 (not tests: they observe Mock/Module disagreements that note 47 stage 5 removes).

## Implementer's early-booking and inside-window parity probes (diff against parity.rs at f1d502e)

```diff
diff -ru '--exclude=target' '--exclude=.git' a/crates/ezsdr-acceptance/tests/parity.rs p1/crates/ezsdr-acceptance/tests/parity.rs
--- a/crates/ezsdr-acceptance/tests/parity.rs	2026-11-06 06:48:47.790638034 +0900
+++ p1/crates/ezsdr-acceptance/tests/parity.rs	2026-11-06 12:38:15.143446681 +0900
@@ -981,7 +981,8 @@
 impl CancellationPair {
     fn new(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)], ports: &[&str], fake: FakeConfig, environment: BTreeMap<Namespace, serde_json::Value>) -> CancellationPair {
         let (waveform, mut inputs) = cancellation_waveform();
-        let (single, extra) = waveform_of(&[[0.25f32, 0.0f32]]);
+        let probe_len: usize = std::env::var("PROBE_LEN").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
+        let (single, extra) = waveform_of(&vec![[0.25f32, 0.0f32]; probe_len]);
         inputs.extend(extra);
         let t0 = ms(2_001);
         let mut mock = MockRig::with_run(selector.clone(), constraints, ports, inputs.clone(), environment);
@@ -1590,3 +1591,32 @@
         assert_eq!(tx_send_calls(&rig.uhd.device, 0).iter().filter(|call| call.contains("sob=true")).count(), 1, "{name}: nothing sent at `s + 1`: {:?}", rig.uhd.device.calls());
     }
 }
+
+#[test]
+fn probe_p1_adjacent_inside_window() {
+    let n = std::env::var("PROBE_LEN").ok().and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
+    for (order, off) in [("AB", n), ("AB", n + 1), ("BA", n), ("BA", n + 1)] {
+        let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
+        let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
+        let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
+        let single = pair.single.clone();
+        submit(&mut pair, 401, 91, |pair| {
+            let a = short_booking(pair, &single, 0, 100, 0);
+            let b = short_booking(pair, &single, 0, 100, off);
+            if order == "AB" { vec![a, b] } else { vec![b, a] }
+        });
+        pair.uhd.wait_until(pair.uhd_t0 + ms(140));
+        pair.mock.step(pair.t0 + ms(140));
+        let mock = pair.mock.finish_in_place();
+        let mock_events = pair.mock.events();
+        let uhd = pair.uhd.finish_in_place();
+        let uhd_events = pair.uhd.events();
+        let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
+        let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
+        let m_s = booked_ticks(&pair.mock.clocks, 0, pair.t0, 100, 0);
+        let u_s = booked_ticks(&pair.uhd.clocks, 0, pair.uhd_t0, 100, 0);
+        let played = |rows: &[serde_json::Value]| rows.iter().map(|r| r["target"]["ticks"].as_i64().unwrap() - if r["target"]["ticks"].as_i64().unwrap() >= 0 { 0 } else { 0 }).collect::<Vec<_>>();
+        eprintln!("PROBEP1 n={n} order={order} off={off} MOCK s={m_s} late={:?} played={:?}", late_drops(&mock_rejected, &mock_events), played(&manifest_rows(&mock, "ezsdr.radio.mock.bursts")));
+        eprintln!("PROBEP1 n={n} order={order} off={off} UHD  s={u_s} late={:?} played={:?} sends={:?}", late_drops(&uhd_rejected, &uhd_events), played(&manifest_rows(&uhd, "ezsdr.radio.uhd.bursts")), tx_send_calls(&pair.uhd.device, 0));
+    }
+}
diff -ru '--exclude=target' '--exclude=.git' a/crates/ezsdr-mock-radio/src/lib.rs p1/crates/ezsdr-mock-radio/src/lib.rs
--- a/crates/ezsdr-mock-radio/src/lib.rs	2026-11-06 06:48:47.789655945 +0900
+++ p1/crates/ezsdr-mock-radio/src/lib.rs	2026-11-06 12:38:15.142446658 +0900
@@ -292,7 +292,7 @@
 
 /// MR-17: a finite burst shorter than `DEVICE_MIN_FIRST_SEND` keeps the frontier at its padded device end plus one sample.
 fn short_burst_floor(start: i64, len: i64, repeat: bool) -> Option<i64> {
-    (!repeat && len < DEVICE_MIN_FIRST_SEND).then(|| start.saturating_add(DEVICE_MIN_FIRST_SEND + 1))
+    (!repeat && len <= 32).then(|| start.saturating_add(len.max(DEVICE_MIN_FIRST_SEND) + 1))
 }
 
 #[derive(Clone, Copy)]
@@ -1364,7 +1364,9 @@
             let end = (!repeat).then_some(start.saturating_add(len));
             let candidate = handed_end_before(order, before, start, end, window_end).filter(|_| start.saturating_add(first) <= window_end);
             payload = merge_front(payload, candidate);
-            handed = merge_front(handed, candidate);
+            if repeat || len > 32 {
+                handed = merge_front(handed, candidate);
+            }
             if candidate.is_some() {
                 device_floor = merge_front(device_floor, short_burst_floor(start, len, repeat));
             }
@@ -1374,7 +1376,7 @@
                 include(open.order, open.start, open.len, open.repeat, 1);
             }
             for held in self.tx[k].held.values() {
-                include(held.order, held.start, held.len, held.repeat, if held.repeat { DEVICE_MIN_FIRST_SEND } else { held.len.min(DEVICE_MIN_FIRST_SEND) });
+                include(held.order, held.start, held.len, held.repeat, if held.repeat { 32 } else { held.len.min(32) });
             }
         }
         let (front, floor) = if self.profile.timing().tx_in_flight_ns == 0 {
diff -ru '--exclude=target' '--exclude=.git' a/crates/ezsdr-radio-uhd/src/provider/tx.rs p1/crates/ezsdr-radio-uhd/src/provider/tx.rs
--- a/crates/ezsdr-radio-uhd/src/provider/tx.rs	2026-11-06 05:49:48.768572925 +0900
+++ p1/crates/ezsdr-radio-uhd/src/provider/tx.rs	2026-11-06 12:38:15.142446658 +0900
@@ -537,14 +537,14 @@
         let tiled = open.held.repeat && open.held.samples[0].len() > len as usize;
         let mut wanted = if tiled { chunk } else { chunk.min(len - offset) };
         if device_sob {
-            wanted = wanted.max(if tiled { min_first } else { min_first.min(len - offset) });
+            wanted = wanted.max(if tiled { 32 } else { 32.min(len - offset) });
         }
         let cut = [held_cut, clock_cut].into_iter().flatten().min();
         if let Some(cut) = cut {
             wanted = wanted.min(cut - open.next);
         }
         let avail = window_cut - open.next;
-        if avail < if device_sob { wanted.min(min_first) } else { wanted } {
+        if avail < if device_sob { wanted.min(32) } else { wanted } {
             return false;
         }
         let count = wanted.min(avail);
@@ -567,9 +567,9 @@
         // of a burst that continues the device burst there (Review N, B3; Review O, O-B1).
         // A one-sample burst's first buffer is not held: its start goes with it.
         let open_ended = clock.instant(open.next + count) - self.core.now() > self.core.ticks(DEVICE_LEAD_NS);
-        let hold = eob && !ends_at_held && open_ended && !(device_sob && count <= min_first);
+        let hold = eob && !ends_at_held && open_ended && !(device_sob && count <= 32);
         // A short first send is closed here even where a booking follows at its end: that booking meets the padded front (UR-22).
-        let device_eob = eob && (!ends_at_held || device_sob && count < min_first) && !hold;
+        let device_eob = eob && (!ends_at_held || device_sob && count <= 32) && !hold;
         let sending = (if hold { count - 1 } else { count }) as usize;
         // The held tail is at `start - 1`; the continuation cursor already follows it.
         if continuing {
```

## Re-check 2: continuation receipt order (fix round 2's flag; removed in fix round 3)

```rust
fn rk_repeat(pair: &CancellationPair, waveform: &ezsdr_kernel::manifest::ArtifactRef, relative_ms: i64, samples: i64) -> (Action, Action) {
    let (mut m, mut u) = short_booking(pair, waveform, 0, relative_ms, samples);
    for a in [&mut m, &mut u] {
        if let Action::TxBurst { repeat, .. } = a {
            *repeat = true;
        }
    }
    (m, u)
}

fn rk_run(name: &str, first: &dyn Fn(&CancellationPair) -> Vec<(Action, Action)>, second: Option<&dyn Fn(&CancellationPair) -> Vec<(Action, Action)>>, b_at: i64, b_ms: i64) {
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    submit(&mut pair, 201, 50, |p| first(p));
    let mut token = 202;
    if let Some(second) = second {
        submit(&mut pair, token, 60, |p| second(p));
        token += 1;
    }
    if b_ms == 92 {
        submit(&mut pair, token, 91, |_| vec![stop_both("tx/s0")]);
        token += 1;
    }
    let single = pair.single.clone();
    submit(&mut pair, token, b_ms, |p| vec![short_booking(p, &single, 0, 100, b_at)]);
    pair.uhd.wait_until(pair.uhd_t0 + ms(140));
    pair.mock.step(pair.t0 + ms(140));
    let mock = pair.mock.finish_in_place();
    let me = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let ue = pair.uhd.events();
    let md = late_drops(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected"), &me);
    let ud = late_drops(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected"), &ue);
    eprintln!("RK targets mock={:?} uhd={:?} | B mock={} uhd={} | C20 mock={} uhd={}", md.1, ud.1, booked_ticks(&pair.mock.clocks, 0, pair.t0, 100, b_at), booked_ticks(&pair.uhd.clocks, 0, pair.uhd_t0, 100, b_at), booked_ticks(&pair.mock.clocks, 0, pair.t0, 100, 20), booked_ticks(&pair.uhd.clocks, 0, pair.uhd_t0, 100, 20));
    eprintln!(
        "RK {name} B@{b_at}: mock drops={} bursts={} | uhd drops={} bursts={} | uhd sends={:?}",
        md.0,
        manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(),
        ud.0,
        manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(),
        tx_send_calls(&pair.uhd.device, 0)
    );
}

#[test]
fn rk_probes() {
    for b in [51, 52] {
        rk_run("receipt A then L, Stop at 91, B at 92", &|p| vec![short_booking(p, p.sized(10), 0, 100, 40), short_booking(p, p.sized(40), 0, 100, 0)], None, b, 92);
    }
}
```

## Re-check 3: early booking at a closed short burst's end, and a preemption cut leaving 4–32

```rust
fn probe_early_booking_after_short_and_preempt() {
    let mut out = Vec::new();
    // (kind, samples, offset): kind 0 = A short closed then B at A.end+offset, both received at 50 ms (outside the window); kind 1 = long 40 (single 40 waveform) then B at start+samples, both at 50 ms.
    for kind in 0..2 {
        for samples in [4usize, 10, 32] {
            for offset in 0..=2i64 {
                let mut pair = CancellationPair::new(
                    selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
                    &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
                    &["rx0"],
                    FakeConfig::default(),
                    BTreeMap::new(),
                );
                let first = if kind == 0 { pair.sized(samples).clone() } else { pair.sized(40).clone() };
                let single = pair.single.clone();
                let at = samples as i64 + offset;
                submit(&mut pair, 201, 50, |pair| vec![short_booking(pair, &first, 0, 100, 0), short_booking(pair, &single, 0, 100, at)]);
                pair.uhd.wait_until(pair.uhd_t0 + ms(140));
                pair.mock.step(pair.t0 + ms(140));
                let mock = pair.mock.finish_in_place();
                let mock_events = pair.mock.events();
                let uhd = pair.uhd.finish_in_place();
                let uhd_events = pair.uhd.events();
                let m = late_drops(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected"), &mock_events).0;
                let u = late_drops(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected"), &uhd_events).0;
                let mb = manifest_rows(&mock, "ezsdr.radio.mock.bursts").len();
                let ub = manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len();
                out.push(format!("kind {kind} samples {samples} offset {offset}: mock late {m} bursts {mb} | uhd late {u} bursts {ub} sends {:?}", tx_send_calls(&pair.uhd.device, 0)));
            }
        }
    }
    panic!("{}", out.join("\n"));
}
```
