//! The server's half of `connect`'s initial streams (EA-10, RS-12): expands what a client
//! wrote into the radio-independent `InitialRequests` the Kernel's Session entry takes.
//! It runs after the Modules are built and before the Kernel is entered, because only
//! then are a layout's channel and stream counts known, so a request for a channel the
//! layout lacks is refused here and no Run exists.

use std::collections::{BTreeMap, BTreeSet};

use ezsdr_kernel::coordinator::Assembly;
use ezsdr_kernel::session::InitialRequests;
use ezsdr_kernel::spec::{Finite, Ident, Key, Scalar};
use ezsdr_radio::{Concrete, keys};

use crate::protocol::{ChannelRef, ResourceInit, StreamInit, StreamNames};

/// The requests for every named resource and the numbers given to stream names.
pub(crate) fn expand(initial: &BTreeMap<String, ResourceInit>, assembly: &Assembly) -> Result<(InitialRequests, BTreeMap<String, StreamNames>), String> {
    let mut requests = InitialRequests::new();
    let mut names = BTreeMap::new();
    let mut taken_names = BTreeSet::new();
    for (resource, init) in initial {
        let ident = Ident::parse(resource).map_err(|e| format!("EA-10: {resource}: {e}"))?;
        let provider = assembly.providers.get(&ident).ok_or_else(|| format!("EA-10: initial streams name {resource}, which is no radio of this profile"))?;
        let mut keys_of = BTreeMap::new();
        let mut given = StreamNames::default();
        for (direction, streams) in [("rx", &init.rx), ("tx", &init.tx)] {
            let count = provider
                .instance()
                .tree
                .capabilities
                .keys()
                .filter(|key| matches!(ezsdr_radio::concrete(key), Some(Concrete::Channel { rx, leaf: "stream", .. }) if rx == (direction == "rx")))
                .count() as u32;
            let numbers = number_streams(direction, streams, count, &mut taken_names)?;
            let mut in_stream = BTreeSet::new();
            for (stream, number) in streams.iter().zip(&numbers) {
                let channels = channels_of(stream, count, &mut in_stream)?;
                if let Some(name) = &stream.name {
                    (if direction == "rx" { &mut given.rx } else { &mut given.tx }).insert(name.clone(), *number);
                }
                for channel in &channels {
                    keys_of.insert(key(&keys::channel(direction, *channel, "stream"))?, Scalar::Int(i64::from(*number)));
                }
                if let Some(rate) = stream.sample_rate {
                    keys_of.insert(key(&keys::stream(direction, *number))?, number_value(rate, "sample_rate")?);
                }
                for (leaf, values) in [("frequency_hz", &stream.frequency), ("gain_db", &stream.gain)] {
                    for (channel, value) in values {
                        keys_of.insert(key(&keys::channel(direction, member(channel, &channels)?, leaf))?, number_value(*value, leaf)?);
                    }
                }
                for (channel, value) in &stream.antenna {
                    keys_of.insert(key(&keys::channel(direction, member(channel, &channels)?, "antenna"))?, Scalar::Str(value.clone()));
                }
            }
        }
        requests.insert(ident, keys_of);
        names.insert(resource.clone(), given);
    }
    Ok((requests, names))
}

/// The number of each stream of one direction, in request order: its own `number`, or for
/// a name the smallest number no stream of the direction claims (EA-10).
fn number_streams(direction: &str, streams: &[StreamInit], count: u32, taken_names: &mut BTreeSet<String>) -> Result<Vec<u32>, String> {
    let mut claimed = BTreeSet::new();
    for stream in streams {
        match (&stream.number, &stream.name) {
            (Some(number), None) => {
                if !claimed.insert(*number) {
                    return Err(format!("EA-10: {direction} stream s{number} is given twice"));
                }
            }
            (None, Some(name)) => {
                if name.is_empty() || name.bytes().all(|b| b.is_ascii_digit()) || name.contains([':', '#']) || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
                    return Err(format!("EA-10: {name:?} is not a stream name"));
                }
                if !taken_names.insert(name.clone()) {
                    return Err(format!("EA-10: stream name {name} is used twice"));
                }
            }
            _ => return Err(format!("EA-10: a {direction} stream gives exactly one of number and name")),
        }
    }
    let mut next = 0;
    let mut numbers = Vec::new();
    for stream in streams {
        let number = match stream.number {
            Some(number) => number,
            None => {
                while claimed.contains(&next) {
                    next += 1;
                }
                claimed.insert(next);
                next
            }
        };
        if number >= count {
            return Err(format!("RM-4: no such stream s{number}"));
        }
        numbers.push(number);
    }
    Ok(numbers)
}

/// A stream's channels, each a number the layout has and in no other stream of the
/// direction (`seen`); a name is a later stage's (EA-10).
fn channels_of(stream: &StreamInit, count: u32, seen: &mut BTreeSet<u32>) -> Result<Vec<u32>, String> {
    let mut channels = Vec::new();
    for channel in &stream.channels {
        let number = match channel {
            ChannelRef::Number(number) => *number,
            ChannelRef::Name(name) => return Err(format!("EA-10: channel name {name:?}: channel names need stage 1b")),
        };
        if number >= count {
            return Err(format!("RM-4: no such channel c{number}"));
        }
        if !seen.insert(number) {
            return Err(format!("EA-10: channel c{number} is given twice"));
        }
        channels.push(number);
    }
    Ok(channels)
}

/// The channel a value's key names, which must be one of its stream's (EA-10).
fn member(text: &str, channels: &[u32]) -> Result<u32, String> {
    let number: u32 = text.parse().map_err(|_| format!("EA-10: value key {text:?}: channel names need stage 1b"))?;
    if text != number.to_string() {
        return Err(format!("EA-10: value key {text:?} is not a channel number as written (a decimal without sign or leading zero)"));
    }
    if channels.contains(&number) { Ok(number) } else { Err(format!("EA-10: c{number} has a value but is not a channel of its stream")) }
}

fn number_value(value: f64, what: &str) -> Result<Scalar, String> {
    Finite::new(value).map(Scalar::Num).ok_or_else(|| format!("EA-10: {what} is not a finite number"))
}

fn key(text: &str) -> Result<Key, String> {
    Key::parse(text).map_err(|e| format!("EA-10: {e}"))
}
