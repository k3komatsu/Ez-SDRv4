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
}

impl Refusal {
    /// The reason, without a rule prefix: the rule and the cause (RM-21).
    pub fn reason(&self) -> String {
        match self {
            Refusal::Instant => "RM-21: a staged value takes effect at the next start, never at an instant".to_owned(),
            Refusal::NotStaged => "RM-21: only a receive channel's frequency, gain or antenna is staged".to_owned(),
            Refusal::NoStream(channel) => format!("RM-21: receive channel {channel} is in no stream, so no start would use the value"),
            Refusal::Streaming { channel, stream } => format!("RM-21: receive channel {channel}'s stream {stream} is streaming; send Stop first, then the value, then start_rx"),
            Refusal::StartWhileStreaming(stream) => format!("RM-21: stream {stream} is streaming (a booked start counts); send Stop first, then start_rx"),
        }
    }
}

/// The receive channel values of record: the Run's configuration, changed by the values a stopped
/// stream accepted (RM-21). A `start_rx` takes a copy of it at its booking.
#[derive(Clone, Debug)]
pub struct Staging {
    values: BTreeMap<Key, Value>,
}

impl Staging {
    /// The values at the Run's start.
    pub fn new(configuration: BTreeMap<Key, Value>) -> Staging {
        Staging { values: configuration }
    }

    /// RM-21: stages `value` for `key` when it is a receive channel's frequency, gain or antenna,
    /// carries no instant, the channel is in a stream and that stream is not streaming
    /// (`streaming(k)` is the Provider's answer for stream `k`, asked for the channel's own stream
    /// only). A later value for the same key replaces an earlier one.
    pub fn set(&mut self, key: &Key, value: Value, has_at: bool, streaming: impl Fn(u32) -> bool) -> Result<(), Refusal> {
        let Some(Concrete::Channel { rx: true, number, leaf: "frequency_hz" | "gain_db" | "antenna" }) = concrete(key) else {
            return Err(Refusal::NotStaged);
        };
        if has_at {
            return Err(Refusal::Instant);
        }
        let stream_key = Key::parse(&crate::keys::channel("rx", number, "stream")).expect("a declared radio key");
        let stream = match self.values.get(&stream_key).and_then(Value::as_scalar) {
            Some(ezsdr_kernel::spec::Scalar::Int(stream)) if *stream >= 0 => *stream as u32,
            _ => return Err(Refusal::NoStream(number)),
        };
        if streaming(stream) {
            return Err(Refusal::Streaming { channel: number, stream });
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
}
