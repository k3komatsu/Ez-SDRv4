//! The two key sets a `UhdRadio` instance presents (UR-5, design 33 D5): the current
//! description (RM-26), or the layout builder's numbered channel and stream keys
//! (RM-27…RM-32) when the selector says `keys: "channel"` (RM-38). One `Mode`, as
//! MockRadio's `Profile` has, so that no judgement about a key set is written twice.

use std::collections::BTreeMap;

use ezsdr_kernel::id::{MemoryDomainId, ResourceId};
use ezsdr_kernel::module_api::{CoerceReport, ModuleError, ProfileRef, Requested, Resource};
use ezsdr_kernel::spec::{Key, Scalar, Value};
use ezsdr_radio::channels;
use ezsdr_radio::device::{DeviceDescription, Grid};
use ezsdr_radio::keys;
use ezsdr_radio::timeline::Channels;
use ezsdr_radio::{PerformanceEnvelope, RadioEnvelope, TimingEnvelope};

use super::core::Core;
use crate::device::{Dir, Settings};

/// The description one instance presents (RM-26, RM-29): its current keys, or the layout
/// builder's channel and stream keys. A Provider instance uses one of the two, never
/// both (RM-27).
#[derive(Clone)]
pub(crate) enum Mode {
    /// RM-26's `DeviceDescription`: one stream per direction, direction-prefixed keys.
    Current(Box<DeviceDescription>),
    /// The layout builder's description (RM-27…RM-32): one channel and stream key per
    /// logical channel of the binding's layout.
    Layout(Box<channels::Description>),
}

impl Mode {
    /// RM-12, RM-29: what the resolved node of an Action names for this instance, through
    /// the shared `ezsdr_radio::addressed` (the one place both Providers read a
    /// destination): the instance's own node, one of its streams, or another node.
    pub fn addressed(&self, core: &Core, node: Option<&ResourceId>) -> ezsdr_radio::Addressed {
        ezsdr_radio::addressed(&core.id, node, self.is_layout(), Some(&core.rx_id), Some(&core.tx_id), |direction| {
            self.stream_count(direction.into())
        })
    }

    /// Whether this instance presents the layout builder's keys (RM-27).
    pub fn is_layout(&self) -> bool {
        matches!(self, Mode::Layout(_))
    }

    /// RM-5's or RM-30's defaults, the `effective` an unconstrained key takes.
    pub fn defaults(&self) -> BTreeMap<Key, Value> {
        match self {
            Mode::Current(description) => description.defaults.clone(),
            Mode::Layout(description) => description.defaults(),
        }
    }

    /// RM-2's or RM-29's tree.
    pub fn tree(&self, device: &ResourceId, memory_domain: MemoryDomainId) -> Resource {
        match self {
            Mode::Current(description) => description.tree(device, memory_domain),
            Mode::Layout(description) => description.tree(device, memory_domain),
        }
    }

    /// RM-5's, RM-7's, RM-8's and RM-30's coercion of this key set (UR-11); pure (MA-11).
    pub fn coerce(&self, device: &ResourceId, request: &Requested) -> Result<CoerceReport, ModuleError> {
        match self {
            Mode::Current(description) => description.coerce(device, request),
            Mode::Layout(description) => description.coerce(device, request),
        }
    }

    /// RM-20: the profile this describes.
    pub fn profile_ref(&self) -> ProfileRef {
        match self {
            Mode::Current(description) => description.profile.clone(),
            Mode::Layout(description) => description.profile.clone(),
        }
    }

    /// RM-6's TimingEnvelope.
    pub fn timing(&self) -> TimingEnvelope {
        match self {
            Mode::Current(description) => description.timing,
            Mode::Layout(description) => description.timing,
        }
    }

    /// RM-7's PerformanceEnvelope.
    pub fn performance(&self) -> PerformanceEnvelope {
        match self {
            Mode::Current(description) => description.performance,
            Mode::Layout(description) => description.performance,
        }
    }

    /// RM-20's envelope section.
    pub fn envelope(&self) -> RadioEnvelope {
        RadioEnvelope {
            profile: self.profile_ref(),
            timing: self.timing(),
            performance: self.performance(),
        }
    }

    /// `radio.rx.block_len` (RM-4).
    pub fn block_len(&self) -> u32 {
        match self {
            Mode::Current(description) => description.block_len,
            Mode::Layout(description) => description.block_len,
        }
    }

    /// `radio.tx.repeat_max_samples` (RM-4).
    pub fn repeat_max_samples(&self) -> u64 {
        match self {
            Mode::Current(description) => description.repeat_max_samples,
            Mode::Layout(description) => description.repeat_max_samples,
        }
    }

    /// Both directions' rate grid, for UR-13's set comparison.
    pub fn rates(&self) -> &Grid {
        match self {
            Mode::Current(description) => &description.rates,
            Mode::Layout(description) => &description.rates,
        }
    }

    /// Whether `key` is one of this key set's configuration keys (RM-5, RM-30).
    pub fn is_configuration(&self, key: &Key) -> bool {
        match self {
            Mode::Current(_) => keys::CONFIGURATION.contains(&key.as_str()),
            Mode::Layout(_) => channels::is_configuration(key),
        }
    }

    /// RM-27's update class of the configuration key `key` of this key set; `None` when
    /// the key is none of this set's or its declaration states no class (UR-24, UR-26).
    pub fn update_class(&self, key: &Key) -> Option<ezsdr_kernel::module_api::UpdateClass> {
        match self {
            Mode::Current(_) => {
                if matches!(key.as_str(), keys::RX_FREQUENCY_HZ | keys::TX_FREQUENCY_HZ | keys::RX_GAIN_DB | keys::TX_GAIN_DB) {
                    Some(ezsdr_kernel::module_api::UpdateClass::HardwareTimed)
                } else if matches!(key.as_str(), keys::RX_CHANNELS | keys::TX_CHANNELS | keys::RX_SAMPLE_RATE_HZ | keys::TX_SAMPLE_RATE_HZ) {
                    Some(ezsdr_kernel::module_api::UpdateClass::Cold)
                } else {
                    None
                }
            }
            Mode::Layout(_) => channels::update_class(key),
        }
    }

    /// How many logical channels a direction of this description has (RM-28, RM-29).
    pub fn channel_count(&self, dir: Dir) -> usize {
        match self {
            Mode::Current(description) => match dir {
                Dir::Rx => description.rx_max_channels.max(0) as usize,
                Dir::Tx => description.tx_max_channels.max(0) as usize,
            },
            Mode::Layout(description) => match dir {
                Dir::Rx => description.rx.len(),
                Dir::Tx => description.tx.len(),
            },
        }
    }

    /// How many streams a direction of this description has (RM-29): as many as its
    /// channels in the layout, one under the current keys (MR-10's shape).
    pub fn stream_count(&self, dir: Dir) -> usize {
        match self {
            Mode::Current(_) => 1,
            Mode::Layout(_) => self.channel_count(dir),
        }
    }

    /// Stream `k` of `dir`'s node below the instance (RM-29): `<id>/rx` and `<id>/tx`
    /// under the current keys, `<id>/rx/s<k>` and `<id>/tx/s<k>` under the layout.
    pub fn stream_node(&self, id: &ResourceId, dir: Dir, k: usize) -> ResourceId {
        match self {
            Mode::Current(_) => id.child(dir.name()).expect("a valid stream id"),
            Mode::Layout(_) => id.child(&format!("{}/s{k}", dir.name())).expect("a valid stream id"),
        }
    }

    /// RM-29, UR-12: stream `k` of the receive side's port name — `rx` under the current
    /// keys, `rx<k>` under the layout, one port per channel.
    pub fn rx_port(&self, k: usize) -> String {
        match self {
            Mode::Current(_) => "rx".to_owned(),
            Mode::Layout(_) => format!("rx{k}"),
        }
    }

    /// RM-3, RM-30: the set of channels stream `k` of `dir` holds in `configuration` —
    /// the direction's count under the current keys, the channels whose `stream` is `k`
    /// under the layout.
    pub fn stream_channels(&self, configuration: &BTreeMap<Key, Value>, dir: Dir, k: usize) -> Channels {
        match self {
            Mode::Current(_) => Channels::first(Core::channels(configuration, dir) as u32),
            Mode::Layout(description) => description.stream_channels(configuration, dir == Dir::Rx, k as u32),
        }
    }

    /// RM-30: the stream logical channel `number` of `dir` is in, or −1 in none.
    pub fn stream_of(&self, configuration: &BTreeMap<Key, Value>, dir: Dir, number: usize) -> i64 {
        match self {
            Mode::Current(_) => -1,
            Mode::Layout(description) => description.stream_of(configuration, dir == Dir::Rx, number as u32),
        }
    }

    /// RM-3, RM-30: the sample rate of stream `k` of `dir` in `configuration` — 0 when
    /// the key has no value, as `Core::settings` reads a missing rate.
    pub fn stream_rate(&self, configuration: &BTreeMap<Key, Value>, dir: Dir, k: usize) -> f64 {
        let name = match self {
            Mode::Current(_) => format!("radio.{}.sample_rate_hz", dir.name()),
            Mode::Layout(_) => keys::stream(dir.name(), k as u32),
        };
        configuration
            .get(&Key::parse(&name).expect("a declared radio key"))
            .and_then(Value::as_scalar)
            .and_then(scalar_number)
            .unwrap_or(0.0)
    }

    /// One logical channel's full settings in `configuration` (UR-12): the direction's
    /// values under the current keys, the channel's own numbered and stream `k`'s rate
    /// keys under the layout.
    pub fn settings(&self, configuration: &BTreeMap<Key, Value>, dir: Dir, k: usize, chan: usize) -> Settings {
        match self {
            Mode::Current(_) => Core::settings(configuration, dir),
            Mode::Layout(_) => Settings {
                rate: Some(self.stream_rate(configuration, dir, k)),
                freq: number(configuration, &keys::channel(dir.name(), chan as u32, "frequency_hz")),
                gain: number(configuration, &keys::channel(dir.name(), chan as u32, "gain_db")),
                antenna: text(configuration, &keys::channel(dir.name(), chan as u32, "antenna")),
            },
        }
    }

    /// UR-30: the key an applied row of one channel's setting is written under — the
    /// direction key under the current keys; under the layout the concrete channel key
    /// (with `channel` beside it), or, for the rate, stream `k`'s own key: a rate is the
    /// stream's, not a channel's (RM-27).
    pub fn applied_key(&self, dir: Dir, chan: usize, k: usize, leaf: &str) -> String {
        match self {
            Mode::Current(_) => format!("radio.{}.{leaf}", dir.name()),
            Mode::Layout(_) if leaf == keys::STREAM_LEAF => keys::stream(dir.name(), k as u32),
            Mode::Layout(_) => keys::channel(dir.name(), chan as u32, leaf),
        }
    }
}

/// The device channel list of a segment's set (RM-29): the channels in the set, smallest
/// first — the order a streamer's channels come in, and the order a block's channels have.
pub(crate) fn channel_list(channels: ezsdr_radio::timeline::Channels) -> Vec<usize> {
    channels.iter().map(|chan| chan as usize).collect()
}

/// The integer value of the numbered key `name` in `configuration`.
fn number(configuration: &BTreeMap<Key, Value>, name: &str) -> Option<f64> {
    configuration
        .get(&Key::parse(name).expect("a declared radio key"))
        .and_then(Value::as_scalar)
        .and_then(scalar_number)
}

/// The numeric value of a scalar, whole or fractional.
pub(crate) fn scalar_number(scalar: &Scalar) -> Option<f64> {
    match scalar {
        Scalar::Num(v) => Some(v.get()),
        Scalar::Int(v) => Some(*v as f64),
        _ => None,
    }
}

/// The direction a key addresses under the current keys (RM-12): `radio.rx.` is receive,
/// every other key transmit.
pub(crate) fn current_key_direction(key: &Key) -> Dir {
    if key.as_str().starts_with("radio.rx.") { Dir::Rx } else { Dir::Tx }
}

/// The string value of the numbered key `name` in `configuration`.
fn text(configuration: &BTreeMap<Key, Value>, name: &str) -> Option<String> {
    configuration
        .get(&Key::parse(name).expect("a declared radio key"))
        .and_then(Value::as_scalar)
        .and_then(|scalar| match scalar {
            Scalar::Str(value) => Some(value.clone()),
            _ => None,
        })
}
