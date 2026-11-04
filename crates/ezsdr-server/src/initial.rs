//! The server's half of `connect`'s initial streams (EA-10, RS-12): expands what a client
//! wrote into the radio-independent `InitialRequests` the Kernel's Session entry takes.
//! It runs after the Modules are built and before the Kernel is entered, because only
//! then are a layout's channel and stream counts known, so a request for a channel the
//! layout lacks is refused here and no Run exists. A channel name is resolved here too, from
//! the radios' selectors (RM-28): a stream of names goes to the radio that owns them.

use std::collections::{BTreeMap, BTreeSet};

use ezsdr_kernel::coordinator::Assembly;
use ezsdr_kernel::session::InitialRequests;
use ezsdr_kernel::spec::{Finite, Ident, Key, Scalar};
use ezsdr_radio::channels::{canonical_number, name_error};
use ezsdr_radio::{Concrete, keys};

use crate::protocol::{ChannelRef, DirectionNames, ResourceInit, StreamInit};

/// The requests for every named resource and the numbers given to stream names. `tables` are
/// the radios' channel names (`catalogue::channel_tables`).
pub(crate) fn expand(initial: &BTreeMap<String, ResourceInit>, assembly: &Assembly, tables: &BTreeMap<String, DirectionNames>) -> Result<(InitialRequests, BTreeMap<String, DirectionNames>), String> {
    let mut requests = InitialRequests::new();
    let mut names = BTreeMap::new();
    let mut taken_names = BTreeSet::new();
    let empty = DirectionNames::default();
    for (resource, init) in &route(initial, tables)? {
        let table = tables.get(resource).unwrap_or(&empty);
        let ident = Ident::parse(resource).map_err(|e| format!("EA-10: {resource}: {e}"))?;
        let provider = assembly.providers.get(&ident).ok_or_else(|| format!("EA-10: initial streams name {resource}, which is no radio of this profile"))?;
        let mut keys_of = BTreeMap::new();
        let mut given = DirectionNames::default();
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
            let by_name = if direction == "rx" { &table.rx } else { &table.tx };
            for (stream, number) in streams.iter().zip(&numbers) {
                let channels = channels_of(stream, count, &mut in_stream, by_name)?;
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
                    let mut valued = BTreeSet::new();
                    for (channel, value) in values {
                        let number = member(channel, &channels, by_name, &mut valued, leaf)?;
                        keys_of.insert(key(&keys::channel(direction, number, leaf))?, number_value(*value, leaf)?);
                    }
                }
                let mut valued = BTreeSet::new();
                for (channel, value) in &stream.antenna {
                    let number = member(channel, &channels, by_name, &mut valued, "antenna")?;
                    keys_of.insert(key(&keys::channel(direction, number, "antenna"))?, Scalar::Str(value.clone()));
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
                if name_error(name).is_some() {
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
/// direction (`seen`); a name is its number in `by_name`, where `route` has already found it (EA-10a).
fn channels_of(stream: &StreamInit, count: u32, seen: &mut BTreeSet<u32>, by_name: &BTreeMap<String, u32>) -> Result<Vec<u32>, String> {
    let mut channels = Vec::new();
    for channel in &stream.channels {
        let number = match channel {
            ChannelRef::Number(number) => *number,
            ChannelRef::Name(name) => by_name[name],
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

/// The channel a value's key names, which must be one of its stream's and valued once: a key is
/// a number when it is a canonical ASCII decimal and a name otherwise (EA-10a).
fn member(text: &str, channels: &[u32], by_name: &BTreeMap<String, u32>, valued: &mut BTreeSet<u32>, what: &str) -> Result<u32, String> {
    let number = match canonical_number(text) {
        Some(number) => number,
        None => by_name[text],
    };
    if !channels.contains(&number) {
        return Err(format!("EA-10: c{number} has a value but is not a channel of its stream"));
    }
    if !valued.insert(number) {
        return Err(format!("EA-10: {what} has two values for c{number}"));
    }
    Ok(number)
}

/// The radio that owns channel name `name`, and the direction it is a channel of.
fn owner_of<'a>(name: &str, tables: &'a BTreeMap<String, DirectionNames>) -> Result<(&'a str, &'static str), String> {
    if name_error(name).is_some() {
        return Err(format!("EA-10: {name:?} is not a channel name"));
    }
    for (radio, table) in tables {
        if table.rx.contains_key(name) {
            return Ok((radio, "rx"));
        }
        if table.tx.contains_key(name) {
            return Ok((radio, "tx"));
        }
    }
    Err(format!("EA-10: no channel is named {name:?}"))
}

/// Puts each stream under the radio it belongs to (EA-10a): the owner of the channel names it
/// mentions, whatever resource it was written under; a stream with no name stays where it
/// was written. Names of two radios, or of the other direction, or beside a number of a
/// radio that does not own them, are refused.
fn route(initial: &BTreeMap<String, ResourceInit>, tables: &BTreeMap<String, DirectionNames>) -> Result<BTreeMap<String, ResourceInit>, String> {
    let mut routed: BTreeMap<String, ResourceInit> = BTreeMap::new();
    for (resource, init) in initial {
        if init.rx.is_empty() && init.tx.is_empty() {
            routed.entry(resource.clone()).or_default();
        }
        for (direction, streams) in [("rx", &init.rx), ("tx", &init.tx)] {
            for stream in streams {
                let keys: Vec<&str> = stream.frequency.keys().chain(stream.gain.keys()).chain(stream.antenna.keys()).map(String::as_str).collect();
                let mut mentioned: Vec<&str> = stream.channels.iter().filter_map(|c| if let ChannelRef::Name(name) = c { Some(name.as_str()) } else { None }).collect();
                mentioned.extend(keys.iter().copied().filter(|key| canonical_number(key).is_none()));
                let numbered = stream.channels.iter().any(|c| matches!(c, ChannelRef::Number(_))) || keys.iter().any(|key| canonical_number(key).is_some());
                let mut owner: Option<&str> = None;
                for name in mentioned {
                    let (radio, named_direction) = owner_of(name, tables)?;
                    if named_direction != direction {
                        let (is, wanted) = if named_direction == "tx" { ("a transmit", "receive") } else { ("a receive", "transmit") };
                        return Err(format!("EA-10: {name:?} is {is} channel, not a {wanted} channel"));
                    }
                    match owner {
                        Some(other) if other != radio => return Err(format!("EA-10: a stream mixes the channels of radios {other} and {radio}")),
                        _ => owner = Some(radio),
                    }
                }
                let home = match owner {
                    Some(radio) if numbered && radio != resource => return Err(format!("EA-10: a stream mixes channel numbers of {resource} with channel names of radio {radio}")),
                    Some(radio) => radio,
                    None => resource,
                };
                let target = routed.entry(home.to_owned()).or_default();
                (if direction == "rx" { &mut target.rx } else { &mut target.tx }).push(stream.clone());
            }
        }
    }
    Ok(routed)
}

fn number_value(value: f64, what: &str) -> Result<Scalar, String> {
    Finite::new(value).map(Scalar::Num).ok_or_else(|| format!("EA-10: {what} is not a finite number"))
}

fn key(text: &str) -> Result<Key, String> {
    Key::parse(text).map_err(|e| format!("EA-10: {e}"))
}
