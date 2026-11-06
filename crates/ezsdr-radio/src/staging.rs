//! RM-21, RM-27 computed once (note 47, stage 3): which receive value a Provider accepts, and
//! the configuration a `start_rx` takes. Pure, like [`crate::timeline`]: MockRadio and the UHD
//! Module each hold one and only wire it (event emission, snapshot storage), so the rule is
//! written once and the differential harness (VH) proves the wiring.
//!
//! "Streaming" is not kept here. It is the stream's line after replay ([`crate::timeline::Line::streaming`]),
//! which each Provider passes in as a closure, so the fact has one representation.

use std::collections::BTreeMap;

use ezsdr_kernel::spec::{Key, Value};

use crate::{Concrete, concrete};

/// Why a receive value is not staged (RM-21). Each cause has its own text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The update carries an instant: a staged value takes effect at the next start, never at an instant.
    Instant,
    /// The key is not a receive channel's frequency, gain or antenna.
    NotStaged,
    /// The channel is in no stream: nothing would ever use the value.
    NoStream(u32),
    /// The channel's own stream is streaming.
    Streaming {
        /// The channel's number.
        channel: u32,
        /// Its stream.
        stream: u32,
    },
    /// A `start_rx` for a stream that is streaming: a booked start not yet begun counts, and it is never revised to an earlier origin.
    StartWhileStreaming(u32),
    /// A transmit stream that its bookings or a repeat keep busy (see [`Busy`]).
    TxBusy {
        /// Until when the stream is busy.
        until: Until,
    },
}

impl Refusal {
    /// The reason, without a rule prefix: the rule and the cause (RM-21).
    pub fn reason(&self) -> String {
        match self {
            Refusal::Instant => "RM-21: a staged value takes effect at the next start, never at an instant".to_owned(),
            Refusal::NotStaged => "RM-21: only a channel's frequency, gain or antenna is staged".to_owned(),
            Refusal::NoStream(channel) => format!("RM-21: receive channel {channel} is in no stream, so no start would use the value"),
            Refusal::Streaming { channel, stream } => format!("RM-21: receive channel {channel}'s stream {stream} is streaming; send Stop first, then the value, then start_rx"),
            Refusal::StartWhileStreaming(stream) => format!("RM-21: stream {stream} is streaming (a booked start counts); send Stop first, then start_rx"),
            Refusal::TxBusy { until: Until::At(end) } => format!("RM-21: the transmit stream is busy until tick {end}; a value is taken once its last booked burst has ended"),
            Refusal::TxBusy { until: Until::Open } => "RM-21: the transmit stream is repeating; send Stop, then the value once the in-flight window has passed".to_owned(),
        }
    }
}

/// When a transmit stream stays busy (RM-21): at an exclusive end in root ticks, or with no end
/// until a `Stop` cuts a repeat.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Until {
    /// The exclusive end, in root ticks.
    At(i64),
    /// A repeat that no `Stop` has cut yet.
    Open,
}

/// The transmit value rule of one stream (RM-21, RM-16): the bookings it has accepted and the
/// Stops that cut them, in root ticks. Written once here; each Provider keeps one per transmit
/// stream and calls it at its booking, at its `Stop` (and at a loss or `Provider::stop`), and at a
/// value's receipt, and nothing else. Nothing removes a booking: a burst refused later stays
/// counted to its booked end.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Busy {
    until: Option<Until>,
}

impl Busy {
    /// A booking accepted, after any booking-time move: its exclusive end in root ticks, or `None`
    /// for a repeat. The later end counts, and an open repeat stays open.
    pub fn book(&mut self, end: Option<i64>) {
        self.until = match (self.until, end) {
            (Some(Until::Open), _) | (_, None) => Some(Until::Open),
            (Some(Until::At(old)), Some(end)) => Some(Until::At(old.max(end))),
            (None, Some(end)) => Some(Until::At(end)),
        };
    }

    /// A `Stop`, a loss or `Provider::stop` taking effect at `at`, with the in-flight window
    /// `in_flight` (RM-16): the stream is busy until `at + in_flight` at the latest, and no
    /// later than a booked end already there. An idle stream stays idle.
    pub fn stop(&mut self, at: i64, in_flight: i64) {
        let bound = at.saturating_add(in_flight);
        self.until = match self.until {
            None => None,
            Some(Until::Open) => Some(Until::At(bound)),
            Some(Until::At(end)) => Some(Until::At(end.min(bound))),
        };
    }

    /// RM-21: a value received at `receipt` is staged only when the stream is not busy, that is
    /// when its last booked end is at or before the receipt.
    pub fn stageable(&self, receipt: i64) -> Result<(), Refusal> {
        match self.until {
            None => Ok(()),
            Some(Until::At(end)) if end <= receipt => Ok(()),
            Some(until) => Err(Refusal::TxBusy { until }),
        }
    }
}

/// The values of record: the Run's configuration, changed by the values a stopped receive stream
/// or an idle transmit stream accepted (RM-21). A `start_rx` or a burst takes a copy of them at
/// its booking.
#[derive(Clone, Debug)]
pub struct Staging {
    values: BTreeMap<Key, Value>,
    /// Per transmit stream, the number of values it has accepted (RM-21): a burst's snapshot
    /// carries it, and the device's configuration is told by it (UR-36).
    tx_generation: BTreeMap<u32, u64>,
}

impl Staging {
    /// The values at the Run's start.
    pub fn new(configuration: BTreeMap<Key, Value>) -> Staging {
        Staging { values: configuration, tx_generation: BTreeMap::new() }
    }

    /// RM-21: stages `value` for `key` when it is a receive channel's frequency, gain or antenna,
    /// or a transmit channel's frequency, gain or antenna, carries no instant, and the channel is
    /// in a stream that takes it: a receive stream that is not streaming (`streaming(k)`, asked
    /// for the channel's own stream only), or a transmit stream whose [`Busy`] allows it at
    /// `receipt`. A later value for the same key replaces an earlier one; an accepted transmit
    /// value bumps its stream's generation.
    pub fn set(&mut self, key: &Key, value: Value, has_at: bool, receipt: i64, streaming: impl Fn(u32) -> bool, busy: impl Fn(u32) -> Busy) -> Result<(), Refusal> {
        let Some(Concrete::Channel { rx, number, leaf: "frequency_hz" | "gain_db" | "antenna" }) = concrete(key) else {
            return Err(Refusal::NotStaged);
        };
        if has_at {
            return Err(Refusal::Instant);
        }
        let stream_key = Key::parse(&crate::keys::channel(if rx { "rx" } else { "tx" }, number, "stream")).expect("a declared radio key");
        let stream = match self.values.get(&stream_key).and_then(Value::as_scalar) {
            Some(ezsdr_kernel::spec::Scalar::Int(stream)) if *stream >= 0 => *stream as u32,
            _ => return Err(Refusal::NoStream(number)),
        };
        if rx && streaming(stream) {
            return Err(Refusal::Streaming { channel: number, stream });
        }
        if !rx {
            busy(stream).stageable(receipt)?;
            *self.tx_generation.entry(stream).or_insert(0) += 1;
        }
        self.values.insert(key.clone(), value);
        Ok(())
    }

    /// RM-21: a `start_rx` of `stream` booked now. Refused when the stream is streaming (its
    /// line, replayed, ends on: a booked start not yet begun counts), else the configuration its
    /// segment begins with.
    pub fn start(&self, stream: u32, streaming: bool) -> Result<BTreeMap<Key, Value>, Refusal> {
        if streaming { Err(Refusal::StartWhileStreaming(stream)) } else { Ok(self.snapshot()) }
    }

    /// The configuration a `start_rx` booked now begins with (RM-21); a copy, so a later value
    /// does not change it.
    pub fn snapshot(&self) -> BTreeMap<Key, Value> {
        self.values.clone()
    }

    /// The configuration a transmit burst of `stream` booked now begins with, and the stream's
    /// generation at that time (RM-21, UR-36): a copy, so a later value does not change it.
    pub fn tx_snapshot(&self, stream: u32) -> (u64, BTreeMap<Key, Value>) {
        (self.tx_generation.get(&stream).copied().unwrap_or(0), self.snapshot())
    }
}
