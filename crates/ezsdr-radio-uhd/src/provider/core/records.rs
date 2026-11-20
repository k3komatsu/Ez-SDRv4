//! What the threads record, and the bound on it (UR-30): the first N rows of each kind of `bursts` (intermediate parts only), `async`, `rejected` and
//! `timing`, the rest counted. `applied` and `switches` are complete.

use std::collections::BTreeMap;
use std::ops::Deref;

use serde_json::{Value as Json, json};

/// Rows of each kind a Run keeps by default (UR-5's `record_rows_per_kind`).
pub(crate) const DEFAULT_RECORD_ROWS: usize = 1_000;
/// Kinds a section tracks by name; the rest share `other` (a kind's budget is its own, `other`'s included).
const KIND_CAP: usize = 64;
const OTHER: &str = "other";

/// Timing rows only an Action writes: their outcome exists nowhere else, so a Run of any length keeps them all.
const TIMING_EXEMPT: [&str; 5] = ["tx_stop_applied", "rx_segment", "first_rx_block", "rx_stop_untimed", "rx_stop_late"];

/// The budget a row is counted against: `Exempt` rows are never bounded and use no kind.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RowKey {
    Exempt,
    Kind(String),
}

impl RowKey {
    pub fn kind(kind: impl Into<String>) -> RowKey {
        RowKey::Kind(kind.into())
    }
}

/// A push-only list of rows that keeps the first `limit` of each kind and counts the rest. It derefs to the kept rows, so a reader sees one array.
#[derive(Clone, Debug)]
pub(crate) struct Bounded {
    rows: Vec<Json>,
    limit: usize,
    kept: BTreeMap<String, usize>,
    omitted: BTreeMap<String, u64>,
}

impl Bounded {
    pub fn new(limit: usize) -> Bounded {
        Bounded { rows: Vec::new(), limit, kept: BTreeMap::new(), omitted: BTreeMap::new() }
    }

    /// Counts the row against its kind, then keeps it while the kind has budget. A kind that arrives after [`KIND_CAP`] others goes to `other`.
    pub fn push(&mut self, key: RowKey, row: Json) {
        let RowKey::Kind(kind) = key else {
            self.rows.push(row);
            return;
        };
        let named = self.kept.keys().filter(|name| name.as_str() != OTHER).count();
        let bucket = if self.kept.contains_key(&kind) || named < KIND_CAP { kind } else { OTHER.to_owned() };
        let kept = self.kept.entry(bucket.clone()).or_default();
        if *kept < self.limit {
            *kept += 1;
            self.rows.push(row);
        } else {
            *self.omitted.entry(bucket).or_default() += 1;
        }
    }

    pub fn omitted(&self) -> &BTreeMap<String, u64> {
        &self.omitted
    }
}

impl Deref for Bounded {
    type Target = [Json];
    fn deref(&self) -> &[Json] {
        &self.rows
    }
}

/// The kind of a `timing` row: its `what`; the names in [`TIMING_EXEMPT`] are exempt.
pub(crate) fn timing_key(row: &Json) -> RowKey {
    match row["what"].as_str() {
        Some(what) if TIMING_EXEMPT.contains(&what) => RowKey::Exempt,
        Some(what) => RowKey::kind(what),
        None => RowKey::kind(OTHER),
    }
}

/// `UR-22` of `UR-22: the rest`, the rule id a reason starts with, if it does.
fn rule_id(reason: &str) -> Option<&str> {
    let (id, _) = reason.split_once(':')?;
    let (letters, digits) = id.split_once('-')?;
    let numbers = digits.trim_end_matches(|c: char| c.is_ascii_lowercase());
    (!letters.is_empty() && letters.bytes().all(|b| b.is_ascii_uppercase()) && !numbers.is_empty() && numbers.bytes().all(|b| b.is_ascii_digit()) && digits.len() - numbers.len() <= 1).then_some(id)
}

/// The kind of a `rejected` row: its `call`; else its `kind` and the rule id its `reason` starts with (a reason carries device text, so never the whole of it);
/// else the name of the field that identifies it.
pub(crate) fn rejected_key(row: &Json) -> RowKey {
    if let Some(call) = row["call"].as_str() {
        return RowKey::kind(call);
    }
    if let Some(kind) = row["kind"].as_str() {
        return match row["reason"].as_str().and_then(rule_id) {
            Some(id) => RowKey::kind(format!("{kind}:{id}")),
            None => RowKey::kind(kind),
        };
    }
    ["thread", "leaked", "event_not_emitted", "burst_tracker", "clock_not_ended", "clock_not_registered"]
        .into_iter()
        .find(|field| row.get(field).is_some())
        .map_or_else(|| RowKey::kind(OTHER), RowKey::kind)
}

/// The kind of an `async` row: its `code`, a flush's acknowledgement apart (`BurstAck:flush`, so a flood of ordinary ones cannot hide the recovery's evidence).
/// `booked_ack`: the row acknowledges the end of a booked burst, which is never bounded.
pub(crate) fn async_key(row: &Json, booked_ack: bool) -> RowKey {
    match row["code"].as_str() {
        Some("BurstAck") if row["tx_flush"] == true => RowKey::kind("BurstAck:flush"),
        Some("BurstAck") if booked_ack => RowKey::Exempt,
        Some(code) => RowKey::kind(code),
        None => RowKey::kind(OTHER),
    }
}

/// The key of a burst record: a repeat's intermediate part (a record a later resume closed) is bounded, as `resumed_part`; any other is kept.
pub(crate) fn burst_key(intermediate_part: bool) -> RowKey {
    if intermediate_part { RowKey::kind("resumed_part") } else { RowKey::Exempt }
}

/// Everything the threads record; written to the sections at `cleanup` (UR-30).
pub(crate) struct Records {
    pub applied: Vec<Json>,
    pub bursts: Bounded,
    pub device_async: Bounded,
    pub rejected: Bounded,
    pub switches: Vec<Json>,
    pub timing: Bounded,
    pub stats: BTreeMap<&'static str, i64>,
    rows_per_kind: usize,
}

impl Default for Records {
    fn default() -> Self {
        Records::with_limit(DEFAULT_RECORD_ROWS)
    }
}

impl Records {
    pub fn with_limit(rows_per_kind: usize) -> Records {
        let stats = [
            "rx_blocks",
            "rx_samples",
            "rx_overflows",
            "rx_errors",
            "rx_off_lattice",
            "rx_overlapping",
            "rx_before_origin",
            ezsdr_radio::rx_link::DISCARDED_STAT,
            "link_drops_seen",
            "tx_bursts",
            "tx_samples",
            "tx_errors",
            "tx_resumes",
            "tx_gap_samples",
        ]
        .into_iter()
        .map(|name| (name, 0))
        .collect();
        Records {
            applied: Vec::new(),
            bursts: Bounded::new(rows_per_kind),
            device_async: Bounded::new(rows_per_kind),
            rejected: Bounded::new(rows_per_kind),
            switches: Vec::new(),
            timing: Bounded::new(rows_per_kind),
            stats,
            rows_per_kind,
        }
    }

    /// Every section the Module writes from these records, with `record_bound`, from one borrow: the rows and the count of what was left out agree.
    pub fn written(&self) -> Vec<(&'static str, Json)> {
        let mut omitted = serde_json::Map::new();
        for (section, rows) in [("bursts", &self.bursts), ("async", &self.device_async), ("rejected", &self.rejected), ("timing", &self.timing)] {
            if !rows.omitted().is_empty() {
                omitted.insert(section.to_owned(), json!(rows.omitted()));
            }
        }
        vec![
            ("applied", Json::Array(self.applied.clone())),
            ("bursts", Json::Array(self.bursts.to_vec())),
            ("async", Json::Array(self.device_async.to_vec())),
            ("rejected", Json::Array(self.rejected.to_vec())),
            ("switches", Json::Array(self.switches.clone())),
            ("timing", Json::Array(self.timing.to_vec())),
            ("stats", json!(self.stats)),
            ("record_bound", json!({ "rows_per_kind": self.rows_per_kind, "omitted": omitted })),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize) -> impl Iterator<Item = Json> {
        (0..n).map(|i| json!({ "i": i }))
    }

    fn kind(name: &str) -> RowKey {
        RowKey::kind(name)
    }

    fn omitted(bounded: &Bounded) -> BTreeMap<&str, u64> {
        bounded.omitted().iter().map(|(kind, n)| (kind.as_str(), *n)).collect()
    }

    // B1
    #[test]
    fn bound_a_kind_keeps_its_first_n_rows_and_counts_the_rest() {
        let mut bounded = Bounded::new(DEFAULT_RECORD_ROWS);
        for row in rows(1_001) {
            bounded.push(kind("k"), row);
        }
        assert_eq!(bounded.len(), 1_000);
        assert!(bounded.iter().enumerate().all(|(i, row)| row["i"] == i), "the first 1000, in order");
        assert_eq!(omitted(&bounded), BTreeMap::from([("k", 1)]));
        let mut exact = Bounded::new(DEFAULT_RECORD_ROWS);
        for row in rows(1_000) {
            exact.push(kind("k"), row);
        }
        assert_eq!((exact.len(), exact.omitted().len()), (1_000, 0), "a kind at its limit leaves nothing out");
    }

    // B2
    #[test]
    fn bound_two_kinds_keep_their_own_budgets() {
        let mut bounded = Bounded::new(1_000);
        for (i, row) in rows(1_510).enumerate() {
            bounded.push(kind(if i % 151 == 0 { "b" } else { "a" }), row);
        }
        assert_eq!(bounded.len(), 1_000 + 10, "a keeps 1000, b keeps its 10");
        assert_eq!(omitted(&bounded), BTreeMap::from([("a", 500)]));
    }

    // B3
    #[test]
    fn bound_a_one_off_after_a_flood_is_kept() {
        let mut bounded = Bounded::new(1_000);
        for row in rows(1_500) {
            bounded.push(kind("a"), row);
        }
        bounded.push(kind("b"), json!({ "one_off": true }));
        assert!(bounded.last().is_some_and(|row| row["one_off"] == true), "the one-off kind is kept");
        assert_eq!(omitted(&bounded), BTreeMap::from([("a", 500)]));
    }

    // B4
    #[test]
    fn bound_an_exempt_key_is_never_bounded() {
        let mut bounded = Bounded::new(2);
        for row in rows(10) {
            bounded.push(RowKey::Exempt, row);
        }
        assert_eq!((bounded.len(), bounded.omitted().len()), (10, 0));
        // After the 64 kinds and `other` exist, an exempt row is still kept and takes no kind slot.
        for i in 0..70 {
            bounded.push(kind(&format!("k{i}")), json!({ "k": i }));
        }
        bounded.push(RowKey::Exempt, json!({ "exempt": "late" }));
        assert!(bounded.last().is_some_and(|row| row["exempt"] == "late"));
        let other = bounded.iter().filter(|row| row["k"].as_i64().is_some_and(|k| k >= 64)).count();
        assert_eq!(other, 2, "kinds 64..70 share `other`, whose own budget is 2");
    }

    // B5
    #[test]
    fn bound_the_65th_kind_goes_to_other() {
        let mut bounded = Bounded::new(1);
        for i in 0..64 {
            bounded.push(kind(&format!("k{i}")), json!({ "k": i }));
        }
        assert_eq!(bounded.len(), 64);
        bounded.push(kind("k64"), json!({ "k": 64 }));
        bounded.push(kind("k65"), json!({ "k": 65 }));
        assert_eq!(bounded.len(), 65, "k64 is kept under `other`");
        assert_eq!(omitted(&bounded), BTreeMap::from([("other", 1)]), "k65 shares the one row of `other`");
        bounded.push(kind("k0"), json!({ "k": "again" }));
        assert_eq!(omitted(&bounded), BTreeMap::from([("k0", 1), ("other", 1)]), "a kind keyed before the cap keeps its own budget");
    }

    // B6
    #[test]
    fn bound_counts_before_it_keeps() {
        let (limit, kinds, pushes) = (7_usize, 100_usize, 5_000_usize);
        let mut bounded = Bounded::new(limit);
        let mut seed = 12_345_u64;
        let mut named: Vec<usize> = Vec::new();
        let mut pushed: BTreeMap<String, usize> = BTreeMap::new();
        let mut kept_rows: BTreeMap<String, usize> = BTreeMap::new();
        for _ in 0..pushes {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let k = ((seed >> 33) as usize) % kinds;
            // The independent first-64 mapping: the first 64 distinct kinds in push order are their own buckets, later kinds land in `other`.
            if !named.contains(&k) && named.len() < KIND_CAP {
                named.push(k);
            }
            let bucket = if named.contains(&k) { format!("k{k}") } else { OTHER.to_owned() };
            *pushed.entry(bucket.clone()).or_default() += 1;
            let before = bounded.len();
            bounded.push(kind(&format!("k{k}")), json!({ "k": k }));
            if bounded.len() > before {
                *kept_rows.entry(bucket).or_default() += 1;
            }
        }
        for (bucket, pushes) in &pushed {
            let kept = kept_rows.get(bucket).copied().unwrap_or(0);
            let left_out = bounded.omitted().get(bucket).copied().unwrap_or(0) as usize;
            assert_eq!(kept + left_out, *pushes, "{bucket}: kept + omitted == pushed");
        }
        assert_eq!(kept_rows.values().sum::<usize>(), bounded.len());
        assert_eq!(bounded.len() + bounded.omitted().values().sum::<u64>() as usize, pushes);
    }

    // B7
    #[test]
    fn bound_derefs_to_the_rows_in_write_order() {
        let mut bounded = Bounded::new(2);
        for (i, name) in ["a", "b", "a", "c", "b", "a"].into_iter().enumerate() {
            bounded.push(kind(name), json!({ "i": i, "kind": name }));
        }
        let order: Vec<i64> = bounded.iter().map(|row| row["i"].as_i64().unwrap()).collect();
        assert_eq!(order, [0, 1, 2, 3, 4], "push order, not kind order");
        assert_eq!(bounded[3]["kind"], "c");
        assert_eq!(bounded.to_vec().len(), 5);
        assert_eq!(bounded.last().map(|row| row["i"].clone()), Some(json!(4)));
    }

    // B8
    #[test]
    fn bound_a_limit_of_one() {
        let mut bounded = Bounded::new(1);
        for name in ["a", "a", "b", "b", "b"] {
            bounded.push(kind(name), json!({ "kind": name }));
        }
        assert_eq!(bounded.iter().map(|row| row["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(omitted(&bounded), BTreeMap::from([("a", 1), ("b", 2)]));
    }

    // B9
    #[test]
    fn records_written_builds_every_section_and_record_bound_from_one_borrow() {
        let mut records = Records::with_limit(3);
        for i in 0..5 {
            records.timing.push(kind("tx_resume"), json!({ "what": "tx_resume", "i": i }));
        }
        for i in 0..4 {
            records.device_async.push(kind("Underflow"), json!({ "code": "Underflow", "i": i }));
            records.bursts.push(kind("resumed_part"), json!({ "i": i }));
        }
        for i in 0..5 {
            records.rejected.push(kind("tx_burst:UR-22"), json!({ "i": i }));
        }
        for i in 0..10 {
            records.applied.push(json!({ "i": i }));
            records.switches.push(json!({ "i": i }));
        }
        let written: BTreeMap<_, _> = records.written().into_iter().collect();
        assert_eq!(written.keys().copied().collect::<Vec<_>>(), ["applied", "async", "bursts", "record_bound", "rejected", "stats", "switches", "timing"]);
        for section in ["async", "bursts", "rejected", "timing"] {
            assert_eq!(written[section].as_array().unwrap().len(), 3, "{section} is its first 3 rows");
        }
        assert_eq!((written["applied"].as_array().unwrap().len(), written["switches"].as_array().unwrap().len()), (10, 10), "applied and switches are complete");
        assert_eq!(
            written["record_bound"],
            json!({ "rows_per_kind": 3, "omitted": { "timing": { "tx_resume": 2 }, "async": { "Underflow": 1 }, "rejected": { "tx_burst:UR-22": 2 }, "bursts": { "resumed_part": 1 } } })
        );
        let clean: BTreeMap<_, _> = Records::with_limit(7).written().into_iter().collect();
        assert_eq!(clean["record_bound"], json!({ "rows_per_kind": 7, "omitted": {} }), "a clean Run still writes record_bound");
    }

    // K1
    #[test]
    fn key_async_by_code_with_booked_acks_exempt() {
        let row = |code: &str| json!({ "code": code, "tick": 1, "channel": 0, "stream": 0 });
        assert_eq!(async_key(&row("Underflow"), false), kind("Underflow"));
        assert_eq!(async_key(&row("TimeError"), false), kind("TimeError"));
        assert_eq!(async_key(&row("SeqError"), false), kind("SeqError"));
        assert_eq!(async_key(&row("Other(7)"), false), kind("Other(7)"));
        assert_eq!(async_key(&row("BurstAck"), true), RowKey::Exempt, "a booked burst's ack");
        assert_eq!(async_key(&row("BurstAck"), false), kind("BurstAck"), "a resume's close or an unmatched ack");
        let flush = json!({ "code": "BurstAck", "tick": 1, "channel": 0, "stream": 0, "tx_flush": true });
        assert_eq!(async_key(&flush, false), kind("BurstAck:flush"));
        assert_eq!(async_key(&flush, true), kind("BurstAck:flush"), "a flush ack matches no entry, whatever the caller thinks");
        let flagged = json!({ "code": "Underflow", "tx_flush": true });
        assert_eq!(async_key(&flagged, false), kind("Underflow"), "only a flush's BurstAck is keyed apart");
    }

    // K2
    #[test]
    fn key_timing_by_what_and_the_five_exempt_names() {
        let row = |what: &str| json!({ "what": what });
        assert_eq!(timing_key(&row("tx_resume")), kind("tx_resume"));
        for name in ["tx_stop_applied", "rx_segment", "first_rx_block", "rx_stop_untimed", "rx_stop_late"] {
            assert_eq!(timing_key(&row(name)), RowKey::Exempt, "{name}");
        }
        for name in ["rx_error", "ref_locked_failed", "time_read_failed", "rx_restart", "rx_block_refused", "device_lost", "plan"] {
            assert_eq!(timing_key(&row(name)), kind(name), "a new kind is bounded unless exempted on purpose: {name}");
        }
        assert_eq!(timing_key(&json!({ "at": 1 })), kind("other"));
    }

    // K3
    #[test]
    fn key_rejected_by_call_then_kind_and_rule_then_field() {
        assert_eq!(rejected_key(&json!({ "call": "tx_flush", "attempt": 2 })), kind("tx_flush"));
        for call in ["tx_async", "tx_close", "set_thread_priority"] {
            assert_eq!(rejected_key(&json!({ "call": call })), kind(call));
        }
        assert_eq!(rejected_key(&json!({ "kind": "tx_burst", "reason": "UR-22: a send; burst not resumed" })), kind("tx_burst:UR-22"));
        assert_eq!(rejected_key(&json!({ "kind": "stop", "reason": "UR-26: the Stop target" })), kind("stop:UR-26"));
        assert_ne!(rejected_key(&json!({ "kind": "tx_burst", "reason": "UR-21: x" })), rejected_key(&json!({ "kind": "tx_burst", "reason": "UR-22: x" })));
        assert_eq!(rejected_key(&json!({ "kind": "tx_burst", "reason": "EOB close accepted 1 of 2 samples" })), kind("tx_burst"));
        assert_eq!(rejected_key(&json!({ "kind": "tx_burst", "reason": "device said UR-22 inside" })), kind("tx_burst"), "the id counts only at the start");
        let keys: std::collections::BTreeSet<RowKey> = (0..200).map(|n| rejected_key(&json!({ "kind": "stop", "reason": format!("UR-26: the device said {n}") }))).collect();
        assert_eq!(keys.len(), 1, "device text makes no new kind");
        for field in ["thread", "leaked", "event_not_emitted", "burst_tracker", "clock_not_ended", "clock_not_registered"] {
            assert_eq!(rejected_key(&json!({ field: "x", "error": "y" })), kind(field));
        }
        assert_eq!(rejected_key(&json!({ "something": 1 })), kind("other"));
    }

    // K5
    #[test]
    fn cardinality_a_flood_of_new_kinds_cannot_grow_a_section_without_bound() {
        let mut timing = Bounded::new(5);
        for n in 0..10_000 {
            let row = json!({ "what": format!("w{n}") });
            timing.push(timing_key(&row), row);
        }
        assert!(timing.len() <= 65 * 5 && timing.omitted().len() <= 65, "{} rows, {} kinds omitted", timing.len(), timing.omitted().len());
        let mut asyncs = Bounded::new(5);
        for n in 0..1_000 {
            let row = json!({ "code": format!("Other({n})") });
            asyncs.push(async_key(&row, false), row);
        }
        assert!(asyncs.len() <= 65 * 5 && asyncs.omitted().len() <= 65);
    }
}
