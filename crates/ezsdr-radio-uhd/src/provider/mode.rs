//! Helpers for the UHD Module's layout description (RM-27…RM-32).

use std::collections::BTreeMap;

use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::spec::{Key, Scalar, Value};
use ezsdr_radio::channels::Description;

use super::core::Core;
use crate::device::{Dir, Settings};

pub(crate) trait DescriptionExt {
    fn addressed(&self, core: &Core, node: Option<&ResourceId>) -> ezsdr_radio::Addressed;
    fn channel_count(&self, dir: Dir) -> usize;
    fn stream_count(&self, dir: Dir) -> usize;
    fn stream_node(&self, id: &ResourceId, dir: Dir, k: usize) -> ResourceId;
    fn rx_port(&self, k: usize) -> String;
    fn stream_rate(&self, configuration: &BTreeMap<Key, Value>, dir: Dir, k: usize) -> f64;
    fn settings(&self, configuration: &BTreeMap<Key, Value>, dir: Dir, k: usize, chan: usize) -> Settings;
    fn applied_key(&self, dir: Dir, chan: usize, k: usize, leaf: &str) -> String;
}

impl DescriptionExt for Description {
    fn addressed(&self, core: &Core, node: Option<&ResourceId>) -> ezsdr_radio::Addressed {
        ezsdr_radio::addressed(&core.id, node, |direction| self.stream_count(Dir::from(direction)))
    }

    fn channel_count(&self, dir: Dir) -> usize {
        match dir {
            Dir::Rx => self.rx.len(),
            Dir::Tx => self.tx.len(),
        }
    }

    fn stream_count(&self, dir: Dir) -> usize {
        self.channel_count(dir)
    }

    fn stream_node(&self, id: &ResourceId, dir: Dir, k: usize) -> ResourceId {
        id.child(&format!("{}/s{k}", dir.name())).expect("a valid stream id")
    }

    fn rx_port(&self, k: usize) -> String {
        format!("rx{k}")
    }

    fn stream_rate(&self, configuration: &BTreeMap<Key, Value>, dir: Dir, k: usize) -> f64 {
        configuration
            .get(&Key::parse(&ezsdr_radio::keys::stream(dir.name(), k as u32)).expect("a declared radio key"))
            .and_then(Value::as_scalar)
            .and_then(scalar_number)
            .unwrap_or(0.0)
    }

    fn settings(&self, configuration: &BTreeMap<Key, Value>, dir: Dir, k: usize, chan: usize) -> Settings {
        Settings {
            rate: Some(self.stream_rate(configuration, dir, k)),
            freq: number(configuration, &ezsdr_radio::keys::channel(dir.name(), chan as u32, "frequency_hz")),
            gain: number(configuration, &ezsdr_radio::keys::channel(dir.name(), chan as u32, "gain_db")),
            antenna: text(configuration, &ezsdr_radio::keys::channel(dir.name(), chan as u32, "antenna")),
        }
    }

    fn applied_key(&self, dir: Dir, chan: usize, k: usize, leaf: &str) -> String {
        if leaf == ezsdr_radio::keys::STREAM_LEAF {
            ezsdr_radio::keys::stream(dir.name(), k as u32)
        } else {
            ezsdr_radio::keys::channel(dir.name(), chan as u32, leaf)
        }
    }
}

pub(crate) fn channel_list(channels: ezsdr_radio::timeline::Channels) -> Vec<usize> {
    channels.iter().map(|chan| chan as usize).collect()
}

pub(crate) fn scalar_number(scalar: &Scalar) -> Option<f64> {
    match scalar {
        Scalar::Num(v) => Some(v.get()),
        Scalar::Int(v) => Some(*v as f64),
        _ => None,
    }
}

fn number(configuration: &BTreeMap<Key, Value>, name: &str) -> Option<f64> {
    configuration
        .get(&Key::parse(name).expect("a declared radio key"))
        .and_then(Value::as_scalar)
        .and_then(scalar_number)
}

fn text(configuration: &BTreeMap<Key, Value>, name: &str) -> Option<String> {
    configuration
        .get(&Key::parse(name).expect("a declared radio key"))
        .and_then(Value::as_scalar)
        .and_then(|scalar| match scalar {
            Scalar::Str(value) => Some(value.clone()),
            _ => None,
        })
}
