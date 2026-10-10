//! Events, counters and the Kernel Action set —
//! `04-run-and-session.md` RS-26…RS-36, RS-48…RS-52 (Vision §29, §5, §19).

use std::collections::{btree_map::Entry, BTreeMap};
use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::id::ResourceId;
use crate::manifest::ArtifactRef;
use crate::module_api::UpdateClass;
use crate::policy::{Policy, Reaction};
use crate::run::{RunError, StopCause};
use crate::spec::{Ident, Key, Value};
use crate::stream::LatePolicy;
use crate::time::{AbsoluteDeadline, TimePoint};

/// How bad an event is; RS-29 derives a default reaction from it (Vision §29).
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Diagnostic detail.
    Debug,
    /// Normal progress.
    Info,
    /// Something is off but the Run continues.
    Warning,
    /// Something failed.
    Error,
    /// The Run cannot continue.
    Fatal,
}

/// What kind of event this is, matching `^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$`:
/// every kind is namespaced. The Kernel owns the five of RS-28, under `ezsdr.`, and
/// every other kind belongs to the Vocabulary or Module that emits it.
///
/// Rule: SB-1, RS-27.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct EventKind(String);

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::id::parsed(d, "an EventKind", EventKind::parse)
    }
}

impl EventKind {
    /// The Kernel's own drain reports dropped bodies with this kind (RS-35).
    pub const EVENTS_DROPPED: &'static str = "ezsdr.EVENTS_DROPPED";
    /// A DataLink policy refused or discarded a block (SC-20a).
    pub const LINK_BACKPRESSURE: &'static str = "ezsdr.LINK_BACKPRESSURE";
    /// A `RelativeBudget` the Kernel defines was exceeded (TM-15).
    pub const PROCESSOR_DEADLINE_MISS: &'static str = "ezsdr.PROCESSOR_DEADLINE_MISS";
    /// Vision §35 states in as many words that this is a Kernel policy (RS-27).
    pub const DEVICE_LOST: &'static str = "ezsdr.DEVICE_LOST";
    /// The stepping loop failed to quiesce (MA-30).
    pub const STEP_LIVELOCK: &'static str = "ezsdr.STEP_LIVELOCK";
    /// RS-33's fallback row reports every unforeseen kind under this one.
    pub const UNFORESEEN: &'static str = "ezsdr.unforeseen";

    /// Parses a kind: at least two `.`-separated segments, each
    /// `[A-Za-z][A-Za-z0-9_]*`, such as the Kernel's `ezsdr.EVENTS_DROPPED` and a
    /// Vocabulary's `test.custom` (RS-27).
    pub fn parse(s: &str) -> Result<EventKind, RunError> {
        let ok = s.contains('.')
            && s.split('.').all(|seg| {
                let mut chars = seg.chars();
                matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
                    && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        if ok {
            Ok(EventKind(s.to_owned()))
        } else {
            Err(RunError::UnknownEventKind { kind: s.to_owned() })
        }
    }

    /// The kind as written (RS-27).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The five kinds the Kernel registers, because it emits them itself or owns
    /// the policy for them. Every other kind belongs to its emitter's namespace: a
    /// Kernel holding a dozen radio kinds would make the thirteenth a Kernel change
    /// (RS-27).
    pub fn kernel_kinds() -> Vec<EventKind> {
        [
            EventKind::EVENTS_DROPPED,
            EventKind::LINK_BACKPRESSURE,
            EventKind::PROCESSOR_DEADLINE_MISS,
            EventKind::DEVICE_LOST,
            EventKind::STEP_LIVELOCK,
        ]
        .into_iter()
        .map(|s| EventKind(s.to_owned()))
        .collect()
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The control-path form of an event: a source, a `TimePoint` in a named domain, a
/// severity, a kind and a schema-versioned payload (Vision §29).
///
/// Rule: RS-31.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// Which resource it came from (RS-31).
    pub source: ResourceId,
    /// When, in a named domain (TM-1).
    pub time: TimePoint,
    /// How bad (RS-29).
    pub severity: Severity,
    /// What kind (RS-27).
    pub kind: EventKind,
    /// The body; its schema belongs to the kind's owner (RS-31).
    pub payload: serde_json::Value,
}

/// Largest inline payload a hot-path record carries. A larger payload is produced
/// on the control path only (RS-32).
pub const HOT_PAYLOAD_BYTES: usize = 32;

/// A fixed-size hot-path record: pre-resolved source and kind indices rather than
/// strings, a `TimePoint`, a severity, and an inline payload.
///
/// In-process by X8, so raising the inline limit is a recompile, not a schema
/// change (decision R8).
///
/// Rule: RS-32.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EventRecord {
    /// Index into the counter table, pre-resolved at `prepare` (RS-33).
    pub row: u32,
    /// Index into the kind table, pre-resolved at `prepare` (RS-33).
    pub kind: u32,
    /// When (TM-1).
    pub time: TimePoint,
    /// How bad (RS-29).
    pub severity: Severity,
    /// How many of `payload` are in use (RS-32).
    pub len: u8,
    /// The inline body (RS-32).
    pub payload: [u8; HOT_PAYLOAD_BYTES],
}

/// One row of the never-dropping counter table (RS-33).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CounterRow {
    /// Which resource, or the fallback row's placeholder (RS-33).
    pub source: ResourceId,
    /// Which kind (RS-33).
    pub kind: EventKind,
    /// How many were emitted, whether or not the body survived (RS-33).
    pub count: u64,
}

/// A pre-resolved `(source, kind)` pair, so that the hot path resolves no strings
/// and allocates nothing (RS-32, RS-33).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EventHandle {
    /// Counter-table row.
    pub row: u32,
    /// Kind-table index.
    pub kind: u32,
}

/// Where a Module emits events. A Plugin host later maps it to an event stream
/// without a Kernel change (MA-6, MA-46).
pub trait EventSink: Send + Sync {
    /// Pre-resolves a `(source, kind)` pair. Control path only (RS-33).
    fn resolve(&self, source: &ResourceId, kind: &EventKind) -> EventHandle;
    /// Emits on the hot path: counts, then queues the body if the ring has room.
    /// Allocates nothing. A payload above [`HOT_PAYLOAD_BYTES`] is
    /// `PayloadTooLarge` (RS-32, RS-33, RS-34).
    fn emit(
        &self,
        handle: EventHandle,
        time: TimePoint,
        severity: Severity,
        payload: &[u8],
    ) -> Result<(), RunError>;
    /// Emits a full-bodied event on the control path (RS-32).
    fn emit_control(&self, event: Event) -> Result<(), RunError>;
}

/// A preallocated ring of hot-path records.
///
/// ponytail: a mutex around a fixed slice, not lock-free (decision R8's ceiling is
/// the inline size, not the lock). Locking allocates nothing, which is what RS-32
/// requires; a lock-free ring is a Phase 2 concern if contention shows up.
struct Ring {
    slots: Box<[EventRecord]>,
    head: usize,
    len: usize,
}

/// The event collector: the counter table, the bounded ring, the per-kind drop
/// counts and the escalation flags.
///
/// Rule: RS-32…RS-36.
pub struct EventCollector {
    sources: Vec<ResourceId>,
    source_index: BTreeMap<ResourceId, usize>,
    kinds: Vec<EventKind>,
    kind_index: BTreeMap<EventKind, usize>,
    /// `(row, kind)` → counter index; the last row is the fallback (RS-33).
    index: BTreeMap<(usize, usize), usize>,
    pairs: Vec<(usize, usize)>,
    counts: Vec<AtomicU64>,
    dropped: Vec<AtomicU64>,
    escalate: Vec<AtomicU8>,
    reactions: Vec<Option<Reaction>>,
    ring: Mutex<Ring>,
    control: Mutex<Vec<Event>>,
}

impl EventCollector {
    /// Sizes the counter table at `prepare` from the plan's `(source, kind)` pairs,
    /// plus one fallback row for pairs that were not foreseen, and preallocates a
    /// ring of `ring_depth` bodies.
    ///
    /// Rule: RS-33, RS-34.
    pub fn new(
        pairs: &[(ResourceId, EventKind)],
        kinds: &[EventKind],
        ring_depth: usize,
        policy: &Policy,
    ) -> EventCollector {
        let mut sources: Vec<ResourceId> = Vec::new();
        let mut source_index = BTreeMap::new();
        let mut kind_list: Vec<EventKind> = kinds.to_vec();
        let mut kind_index = BTreeMap::new();
        for (i, kind) in kind_list.iter().enumerate() {
            kind_index.entry(kind.clone()).or_insert(i);
        }
        for (s, k) in pairs {
            if let Entry::Vacant(entry) = source_index.entry(s.clone()) {
                entry.insert(sources.len());
                sources.push(s.clone());
            }
            if let Entry::Vacant(entry) = kind_index.entry(k.clone()) {
                entry.insert(kind_list.len());
                kind_list.push(k.clone());
            }
        }
        // RS-33: one fallback row, not one per source, which would be unbounded
        // when a Module mislabels its source. RS-35's meta-event always exists, so
        // its source, kind and row always exist too: without the row the drain's own
        // `EVENTS_DROPPED` bodies would land on the fallback row, which reports a
        // different kind, and RS-35's invariant would read as violated for exactly
        // the kind that exists to keep it true (RS-33, RS-38).
        let mut source_of = |source: &str| {
            let source = ResourceId::parse(source).expect("a valid literal path");
            match source_index.entry(source.clone()) {
                Entry::Vacant(entry) => {
                    sources.push(source);
                    *entry.insert(sources.len() - 1)
                }
                Entry::Occupied(entry) => *entry.get(),
            }
        };
        let fallback_source = source_of("unforeseen");
        let kernel_source = source_of(crate::coordinator::KERNEL_SOURCE);
        let dropped_kind = EventKind(EventKind::EVENTS_DROPPED.to_owned());
        let dropped_row_kind = *kind_index.entry(dropped_kind.clone()).or_insert_with(|| {
            kind_list.push(dropped_kind);
            kind_list.len() - 1
        });

        let mut index = BTreeMap::new();
        let mut resolved = Vec::new();
        for (s, k) in pairs {
            let pair = (source_index[s], kind_index[k]);
            if let Entry::Vacant(entry) = index.entry(pair) {
                entry.insert(resolved.len());
                resolved.push(pair);
            }
        }
        index.entry((kernel_source, dropped_row_kind)).or_insert_with(|| {
            resolved.push((kernel_source, dropped_row_kind));
            resolved.len() - 1
        });
        let fallback_kind = kind_list.len();
        let unforeseen_kind = EventKind(EventKind::UNFORESEEN.to_owned());
        kind_index
            .entry(unforeseen_kind.clone())
            .or_insert(fallback_kind);
        kind_list.push(unforeseen_kind);
        index.insert((fallback_source, fallback_kind), resolved.len());
        resolved.push((fallback_source, fallback_kind));

        // RS-29: `None` marks a kind with no entry in the Run's Policy, whose
        // reaction comes from the **event's** severity at emit time — an
        // unregistered kind has no declared severity for the Kernel to look up.
        let reactions: Vec<Option<Reaction>> =
            kind_list.iter().map(|k| policy.table.get(k).copied()).collect();
        let counts = resolved.iter().map(|_| AtomicU64::new(0)).collect();
        let dropped = kind_list.iter().map(|_| AtomicU64::new(0)).collect();
        // 0 = nothing, 1 = stop, 2 = abort (RS-36).
        let escalate: Vec<AtomicU8> = kind_list.iter().map(|_| AtomicU8::new(0)).collect();
        let empty = EventRecord {
            row: 0,
            kind: 0,
            time: TimePoint::new(crate::id::ClockDomainId::HOST_MONOTONIC, 0),
            severity: Severity::Debug,
            len: 0,
            payload: [0; HOT_PAYLOAD_BYTES],
        };
        let collector = EventCollector {
            sources,
            source_index,
            kinds: kind_list,
            kind_index,
            index,
            pairs: resolved,
            counts,
            dropped,
            escalate,
            reactions,
            ring: Mutex::new(Ring {
                slots: vec![empty; ring_depth.max(1)].into_boxed_slice(),
                head: 0,
                len: 0,
            }),
            control: Mutex::new(Vec::new()),
        };
        // RS-32: on some platforms `Mutex` boxes its OS primitive at the first
        // lock, so the first `emit` would allocate once. Warm it here, at `prepare`,
        // where RS-33 already sizes the table.
        // ponytail: a spin or lock-free ring would remove the lock entirely; the
        // measured cost is one allocation per Run, so it is not worth the code.
        drop(collector.ring.lock());
        drop(collector.control.lock());
        collector
    }

    /// The whole counter table, including rows whose count is zero (RS-33, RS-38).
    pub fn counters(&self) -> Vec<CounterRow> {
        self.pairs
            .iter()
            .enumerate()
            .map(|(i, (s, k))| CounterRow {
                source: self.sources[*s].clone(),
                kind: self.kinds[*k].clone(),
                count: self.counts[i].load(Ordering::Relaxed),
            })
            .collect()
    }

    /// The strongest pending reaction when the hot path dropped a stopping body.
    /// An `abort` takes precedence over `stop`, regardless of kind-table order.
    /// Without this a `DEVICE_LOST` arriving during an event storm would be
    /// discarded and the Run would continue on a device that is gone (RS-36). A
    /// queued body raises no flag: it is reacted to when it is drained (KC-31).
    pub fn escalation(&self) -> Option<(EventKind, Reaction)> {
        self.escalate.iter().enumerate().filter_map(|(i, f)| match f.load(Ordering::Relaxed) {
                1 => Some((self.kinds[i].clone(), Reaction::Stop)),
                2 => Some((self.kinds[i].clone(), Reaction::Abort)),
                _ => None,
            }).max_by_key(|(_, reaction)| *reaction == Reaction::Abort)
    }

    /// The reaction to one emission: the kind's Policy entry when it has one, and
    /// otherwise RS-29's severity-derived default. An unregistered kind carries no
    /// declared severity, so only the event itself can supply it (RS-26, RS-29).
    fn reaction(&self, kind: usize, severity: Severity) -> Reaction {
        self.reactions
            .get(kind)
            .copied()
            .flatten()
            .unwrap_or_else(|| Policy::by_severity(severity))
    }

    /// Raises the escalation flag when the reaction is `stop` or `abort`, for a body the
    /// full ring drops (RS-36).
    fn escalate(&self, kind: usize, severity: Severity) {
        let code = match self.reaction(kind, severity) {
            Reaction::Stop => 1,
            Reaction::Abort => 2,
            _ => return,
        };
        if let Some(flag) = self.escalate.get(kind) {
            flag.fetch_max(code, Ordering::Relaxed);
        }
    }

    /// Drains the ring at `at`. Returns the delivered bodies followed by one
    /// `EVENTS_DROPPED` per kind that dropped since the last drain, carrying the
    /// delta rather than a running total, stamped `at` with source `kernel`.
    ///
    /// Those are produced on the control path and never enter the ring: an
    /// `EVENTS_DROPPED` that could itself be dropped would break, under load and
    /// non-deterministically, the very invariant it exists to preserve.
    ///
    /// Rule: RS-34, RS-35.
    pub fn drain(&self, at: TimePoint) -> Vec<Event> {
        let records: Vec<EventRecord> = {
            let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
            let n = ring.len;
            let out = (0..n)
                .map(|i| ring.slots[(ring.head + i) % ring.slots.len()])
                .collect::<Vec<_>>();
            ring.head = 0;
            ring.len = 0;
            out
        };
        let mut out: Vec<Event> = records
            .into_iter()
            .map(|r| Event {
                source: self.sources[self.pairs[r.row as usize].0].clone(),
                time: r.time,
                severity: r.severity,
                kind: self.kinds[r.kind as usize].clone(),
                payload: serde_json::Value::Array(
                    r.payload[..r.len as usize]
                        .iter()
                        .map(|b| serde_json::Value::from(*b))
                        .collect(),
                ),
            })
            .collect();
        out.extend(
            self.control
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain(..),
        );
        let dropped_kind = EventKind(EventKind::EVENTS_DROPPED.to_owned());
        for (i, d) in self.dropped.iter().enumerate() {
            let count = d.swap(0, Ordering::Relaxed);
            if count > 0 {
                // RS-35's invariant is stated "for every kind", so the meta-event is
                // counted like any other body it is delivered beside (RS-33, RS-38).
                let source = ResourceId::parse(crate::coordinator::KERNEL_SOURCE)
                    .expect("a valid literal path");
                let handle = self.resolve(&source, &dropped_kind);
                self.counts[handle.row as usize].fetch_add(1, Ordering::Relaxed);
                out.push(Event {
                    source,
                    time: at,
                    severity: Severity::Warning,
                    kind: dropped_kind.clone(),
                    payload: serde_json::json!({
                        "kind": self.kinds[i].as_str(),
                        "count": count,
                    }),
                });
            }
        }
        out
    }

    /// Both fields of a handle, checked where one enters. `EventHandle`'s fields are
    /// public, so a Module can fabricate one, and RS-32's infallible hot path is a
    /// promise about the **Kernel's** handles. Checking only `row` moved the panic
    /// out of the offending Module's call and into the coordinator's `drain`, which
    /// indexes `kinds` with the stored `kind` — away from the code that caused it
    /// (RS-32, MA-9).
    fn check_handle(&self, handle: EventHandle) -> Result<&AtomicU64, RunError> {
        if self.kinds.len() <= handle.kind as usize {
            return Err(RunError::BadEventHandle { row: handle.row });
        }
        self.counts
            .get(handle.row as usize)
            .ok_or(RunError::BadEventHandle { row: handle.row })
    }
}

impl EventSink for EventCollector {
    fn resolve(&self, source: &ResourceId, kind: &EventKind) -> EventHandle {
        let si = self.source_index.get(source).copied();
        let ki = self.kind_index.get(kind).copied();
        let fallback_row = (self.pairs.len() - 1) as u32;
        match (si, ki) {
            (Some(s), Some(k)) => match self.index.get(&(s, k)) {
                Some(row) => EventHandle { row: *row as u32, kind: k as u32,
                },
                // RS-33: an unforeseen pair merges into the fallback row, which is
                // reported as such.
                None => EventHandle { row: fallback_row, kind: k as u32,
                },
            },
            // An unforeseen *source* must not cost the kind: RS-36's abort on
            // `DEVICE_LOST` and RS-31's delivered kind both hang off it, and a
            // second or hot-plugged device is an ordinary reason for a source the
            // plan did not foresee.
            (None, Some(k)) => EventHandle { row: fallback_row, kind: k as u32,
            },
            _ => EventHandle { row: fallback_row, kind: (self.kinds.len() - 1) as u32,
            },
        }
    }

    fn emit(
        &self,
        handle: EventHandle,
        time: TimePoint,
        severity: Severity,
        payload: &[u8],
    ) -> Result<(), RunError> {
        if payload.len() > HOT_PAYLOAD_BYTES {
            return Err(RunError::PayloadTooLarge);
        }
        // RS-33: counted before the body is queued, so a count is never lost even
        // when the body is.
        // `EventHandle`'s fields are public, so a Module can fabricate one. An
        // out-of-range row must return an error, not panic the Kernel: RS-32's hot
        // path is infallible for the *Kernel's* handles only (MA-9).
        let row = self.check_handle(handle)?;
        row.fetch_add(1, Ordering::Relaxed);
        let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        if ring.len == ring.slots.len() {
            drop(ring);
            if let Some(d) = self.dropped.get(handle.kind as usize) {
                d.fetch_add(1, Ordering::Relaxed);
            }
            // RS-36: a stopping body that is dropped still ends the Run, through the
            // escalation flag. A body that is queued raises none: it is reacted to when
            // it is delivered, so the cause is the first stopping event in `delivered`
            // (KC-31), not a flag raised before its body arrives (design-notes §21).
            self.escalate(handle.kind as usize, severity);
            return Ok(());
        }
        let cap = ring.slots.len();
        let at = (ring.head + ring.len) % cap;
        let slot = &mut ring.slots[at];
        slot.row = handle.row;
        slot.kind = handle.kind;
        slot.time = time;
        slot.severity = severity;
        slot.len = payload.len() as u8;
        slot.payload[..payload.len()].copy_from_slice(payload);
        ring.len += 1;
        Ok(())
    }


    fn emit_control(&self, event: Event) -> Result<(), RunError> {
        let handle = self.resolve(&event.source, &event.kind);
        let row = self.check_handle(handle)?;
        row.fetch_add(1, Ordering::Relaxed);
        // The control path never drops a body, so it raises no escalation flag (RS-36).
        self.control.lock().unwrap_or_else(|e| e.into_inner()).push(event);
        Ok(())
    }
}

// ---------------------------------------------------------------- the Kernel Action set

/// Identifies one dispatched Action inside a Run (RS-15).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct ActionId(pub u64);

/// The closed Kernel Action set: what a Reactor emits into the real-time path.
/// Adding a member is a Kernel major.
///
/// Every timed Action names its instant with an `AbsoluteDeadline`, which Vision
/// §19 defines for exactly this and which TM-15 made a distinct type so that
/// envelope checks have one thing to compare.
///
/// Rule: RS-48, RS-49.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// One transmit burst. Carries no channel list: mapping a burst's channels onto
    /// a stream's channels is the Radio Model's, and deciding it here would put a
    /// radio field in a frozen Kernel document. It travels in `metadata` until
    /// Phase 2 settles it (RS-49).
    TxBurst {
        /// The transmit stream (RS-49).
        target: ResourceId,
        /// The samples, by reference; never the bytes (RS-44).
        waveform: ArtifactRef,
        /// Whether the waveform repeats (SC-26).
        repeat: bool,
        /// The instant, in the transmit stream's SampleClock (RS-51, SC-23a).
        at: AbsoluteDeadline,
        /// What was asked for, when SC-23a advanced it onto the sample grid (RS-51).
        requested_at: Option<AbsoluteDeadline>,
        /// What to do if the lead is short (SC-27).
        late_policy: LatePolicy,
        /// Namespaced content the Kernel does not interpret (RS-49).
        #[serde(default)]
        metadata: BTreeMap<Key, Value>,
    },
    /// Wake at an instant (RS-49).
    SetTimer {
        /// Whose timer (RS-49).
        target: ResourceId,
        /// When (RS-49).
        at: AbsoluteDeadline,
        /// The caller's token, returned with the wake-up (RS-49).
        token: u64,
    },
    /// Change a declared parameter under its declared update class (RS-52, Vision §27).
    UpdateParameter {
        /// Whose parameter (RS-49).
        target: ResourceId,
        /// Which parameter (SB-2).
        key: Key,
        /// Its new value (SB-4).
        value: Value,
        /// The parameter's declared update class; an undeclared one was rejected at
        /// admission and never arrives (RS-52, RS-17).
        class: UpdateClass,
        /// The instant at or after which the update takes effect under its class.
        /// Absent means the first instant the class permits, which is what a bare
        /// `sdr.rx.gain = 20` asks for (Vision §3). Optional for every class,
        /// including `hardware_timed`: a class cannot make the field mandatory,
        /// because the `set_parameter` Session action carries no time to put in it
        /// (RS-49, RS-19, RS-14).
        at: Option<AbsoluteDeadline>,
    },
    /// A Vocabulary-defined verb sent to any target: a radio stream, a peripheral
    /// (RS-49).
    Command {
        /// Whose verb (RS-49).
        target: ResourceId,
        /// The verb, defined by its Vocabulary (RS-13a).
        verb: Ident,
        /// Its parameters (SB-4).
        #[serde(default)]
        params: BTreeMap<Key, Value>,
        /// When, if it is timed (RS-49).
        at: Option<AbsoluteDeadline>,
    },
    /// Produce an event (RS-31).
    Emit {
        /// Whose event (RS-49).
        target: ResourceId,
        /// The event itself (RS-31).
        event: Event,
    },
    /// With a target it stops that resource; without one it stops the Run. Vision
    /// §3 uses the word for both, so the ambiguity is resolved in the type (RS-50).
    Stop {
        /// The resource, or the Run when absent (RS-50).
        target: Option<ResourceId>,
    },
    /// End the Run immediately (RS-49).
    Abort {
        /// Why (RS-3).
        cause: StopCause,
    },
}

impl Action {
    /// The number of members, which RS-48 fixes. A test asserts it so that an
    /// eighth cannot be added without the rule being revisited (RS-48).
    pub const MEMBERS: usize = 7;

    /// The resource an Action addresses; `Abort` addresses the Run (RS-49).
    pub fn target(&self) -> Option<&ResourceId> {
        match self {
            Action::TxBurst { target, .. }
            | Action::SetTimer { target, .. }
            | Action::UpdateParameter { target, .. }
            | Action::Command { target, .. }
            | Action::Emit { target, .. } => Some(target),
            Action::Stop { target } => target.as_ref(),
            Action::Abort { .. } => None,
        }
    }
}

/// An Action as a Spec schedules it: the Action without its time field.
///
/// A Spec cannot name a `ClockDomainId`, because domains are allocated at `prepare`
/// (SB-16, TM-13a), and every timed Action names one through its
/// `AbsoluteDeadline`. `arm` resolves the entry's `SpecTime` and substitutes it.
/// Without the split, `ExperimentSpec.schedule` could hold no timed Action at all,
/// which is the one thing it exists for.
///
/// Rule: RS-49a.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionTemplate {
    /// The `tx_burst` Action without `at` (RS-49a).
    TxBurst {
        /// The transmit stream.
        target: ResourceId,
        /// The samples, by reference.
        waveform: ArtifactRef,
        /// Whether the waveform repeats.
        repeat: bool,
        /// What to do if the lead is short.
        late_policy: LatePolicy,
        /// Namespaced content.
        #[serde(default)]
        metadata: BTreeMap<Key, Value>,
    },
    /// The `set_timer` Action without `at` (RS-49a).
    SetTimer {
        /// Whose timer.
        target: ResourceId,
        /// The caller's token.
        token: u64,
    },
    /// The `update_parameter` Action without `at`; `arm` substitutes the resolved
    /// deadline, so a scheduled parameter change takes effect at the instant the
    /// Spec named (RS-49a).
    UpdateParameter {
        /// Whose parameter.
        target: ResourceId,
        /// Which parameter.
        key: Key,
        /// Its new value.
        value: Value,
        /// The declared update class.
        class: UpdateClass,
    },
    /// The `command` Action without `at` (RS-49a).
    Command {
        /// Whose verb.
        target: ResourceId,
        /// The verb.
        verb: Ident,
        /// Its parameters.
        #[serde(default)]
        params: BTreeMap<Key, Value>,
    },
    /// The `stop` Action, which carries no time (RS-49a).
    Stop {
        /// The resource, or the Run when absent.
        target: Option<ResourceId>,
    },
}

impl ActionTemplate {
    /// Substitutes the deadline `arm` resolved, producing the Action (RS-49a, SB-43).
    pub fn resolve(self, at: AbsoluteDeadline) -> Action {
        match self {
            ActionTemplate::TxBurst { target, waveform, repeat, late_policy, metadata,
            } => Action::TxBurst {
                    target,
                    waveform,
                    repeat,
                    at,
                    requested_at: None,
                    late_policy,
                    metadata,
                },
            ActionTemplate::SetTimer { target, token } => Action::SetTimer { target, at, token },
            ActionTemplate::UpdateParameter { target, key, value, class,
            } => Action::UpdateParameter { target, key, value, class, at: Some(at),
            },
            ActionTemplate::Command { target, verb, params,
            } => Action::Command { target, verb, params, at: Some(at),
            },
            ActionTemplate::Stop { target } => Action::Stop { target },
        }
    }

    /// The resource a scheduled Action addresses, which SB-16 requires to name a
    /// resource or an output the Spec itself declares (RS-49a, SB-16).
    pub fn target(&self) -> Option<&ResourceId> {
        match self {
            ActionTemplate::TxBurst { target, .. }
            | ActionTemplate::SetTimer { target, .. }
            | ActionTemplate::UpdateParameter { target, .. }
            | ActionTemplate::Command { target, .. } => Some(target),
            ActionTemplate::Stop { target } => target.as_ref(),
        }
    }

    /// True when `resolve` needs a deadline; the others ignore it (RS-49a).
    pub fn is_timed(&self) -> bool {
        matches!(
            self,
            ActionTemplate::TxBurst { .. }
                | ActionTemplate::SetTimer { .. }
                | ActionTemplate::UpdateParameter { .. }
                | ActionTemplate::Command { .. }
        )
    }
}
