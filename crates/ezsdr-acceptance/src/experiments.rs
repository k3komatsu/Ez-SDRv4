//! Portable experiment documents expressed only through the Kernel contract.
#![allow(dead_code)]

use std::collections::BTreeMap;

use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::spec::{Ident, Namespace};
use serde_json::{Value, json};

/// The layout configuration key for a logical channel.
pub fn channel_key(direction: &str, channel: u32, leaf: &str) -> String {
    format!("radio.{direction}.c{channel}.{leaf}")
}

/// The layout configuration key for a stream's sample rate.
pub fn stream_key(direction: &str, stream: u32) -> String {
    format!("radio.{direction}.s{stream}.sample_rate_hz")
}

/// A recorder feed attached to receive stream `stream`.
pub fn rx_feed(component: &str, stream: usize) -> Value {
    json!({
        "port": { "component": component, "port": format!("rx{stream}") },
        "policy": "drop_oldest",
        "capacity": 64
    })
}

fn direction_requires(direction: &str, channels: i64, rate_hz: f64, frequency_hz: f64) -> serde_json::Map<String, Value> {
    let mut requires = serde_json::Map::new();
    requires.insert(stream_key(direction, 0), json!({ "kind": "eq", "value": rate_hz }));
    requires.insert(channel_key(direction, 0, "frequency_hz"), json!({ "kind": "eq", "value": frequency_hz }));
    if channels == 0 {
        requires.insert(channel_key(direction, 0, "stream"), json!({ "kind": "eq", "value": -1 }));
    } else {
        for channel in 0..channels {
            requires.insert(channel_key(direction, channel as u32, "stream"), json!({ "kind": "eq", "value": 0 }));
        }
    }
    requires
}

/// A receive experiment with a recorder connected to the receive port.
pub fn receive(channels: i64, rate_hz: f64, frequency_hz: f64, capture: Option<i64>) -> Value {
    let mut params = BTreeMap::new();
    if let Some(samples) = capture {
        params.insert("sink.capture_samples", json!(samples));
    }
    let mut output = json!({
        "id": "rec",
        "kind": "sink.capture",
        "feed": rx_feed("radio", 0),
        "params": {}
    });
    for (key, value) in params {
        output["params"][key] = value;
    }
    json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 2 }, { "id": "sink", "major": 1 }] },
        "resources": {
            "radio": {
                "kind": "radio.device",
                "requires": direction_requires("rx", channels, rate_hz, frequency_hz)
            }
        },
        "outputs": [output],
        "policies": {},
        "extensions": {}
    })
}

/// A receive experiment with a scheduled transmit burst.
pub fn transmit(rate_hz: f64, waveform: &ArtifactRef, repeat: bool, late_policy: &str, offset_ticks: i64, capture: Option<i64>) -> Value {
    let mut spec = receive(1, rate_hz, 1.0e9, capture);
    spec["resources"]["radio"]["requires"].as_object_mut().unwrap().extend(direction_requires("tx", 1, rate_hz, 1.0e9));
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": offset_ticks },
        "action": {
            "kind": "tx_burst",
            "target": { "kind": "resource", "resource": "radio", "path": "tx/s0" },
            "waveform": waveform,
            "repeat": repeat,
            "late_policy": late_policy,
            "metadata": {}
        }
    }]);
    spec
}

/// Adds a capture request at an instant measured on the named resource's clock.
pub fn with_timed_capture(spec: Value, samples: i64, offset_ticks: i64) -> Value {
    let mut spec = spec;
    spec["schedule"].as_array_mut().expect("experiment schedule is an array").push(json!({
        "at": { "clock": "radio", "offset_ticks": offset_ticks },
        "action": {
            "kind": "update_parameter",
            "target": { "kind": "output", "output": "rec" },
            "key": "sink.capture_samples",
            "value": samples
        }
    }));
    spec
}

/// Builds zero-valued complex samples and their content-addressed input reference.
pub fn waveform(samples: usize) -> (Vec<u8>, ArtifactRef) {
    waveform_of(&vec![(0.0, 0.0); samples])
}

/// Builds one channel of little-endian `cf32` samples and their content-addressed input
/// reference (RM-13).
pub fn waveform_of(samples: &[(f32, f32)]) -> (Vec<u8>, ArtifactRef) {
    let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| re.to_le_bytes().into_iter().chain(im.to_le_bytes())).collect();
    let hash = ContentHash::of_bytes(&bytes);
    let artifact = ArtifactRef {
        id: Ident::parse("waveform").expect("valid artifact id"),
        kind: Namespace::parse("ezsdr.input").expect("valid artifact kind"),
        uri: format!("mem:{hash}"),
        hash,
        size_bytes: bytes.len() as u64,
        partial: false,
        marks: Vec::new(),
        continuity: Vec::new(),
    };
    (bytes, artifact)
}

/// A transmitter resource `tx` sending `waveform` once at `offset_ticks` on its own clock,
/// and a receiver resource `rx` recorded by the output `rec`; nothing says how the two are
/// coupled, which is the environment's (Vision §8, §58 #8).
pub fn link(tx: &str, rx: &str, rate_hz: f64, waveform: &ArtifactRef, offset_ticks: i64, capture: i64) -> Value {
    let mut resources = serde_json::Map::new();
    resources.insert(
        tx.to_owned(),
        json!({
            "kind": "radio.device",
            "requires": direction_requires("tx", 1, rate_hz, 1.0e9)
        }),
    );
    resources.insert(
        rx.to_owned(),
        json!({
            "kind": "radio.device",
            "requires": direction_requires("rx", 1, rate_hz, 1.0e9)
        }),
    );
    let target = json!({ "kind": "resource", "resource": tx, "path": "tx/s0" });
    json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 2 }, { "id": "sink", "major": 1 }] },
        "resources": resources,
        "outputs": [{
            "id": "rec",
            "kind": "sink.capture",
            "feed": rx_feed(rx, 0),
            "params": { "sink.capture_samples": capture }
        }],
        "schedule": [{
            "at": { "clock": tx, "offset_ticks": offset_ticks },
            "action": {
                "kind": "tx_burst",
                "target": target,
                "waveform": waveform,
                "repeat": false,
                "late_policy": "send_asap_and_flag",
                "metadata": {}
            }
        }],
        "policies": {},
        "extensions": {}
    })
}

/// The Mini Reactive Radio (Vision §67 Phase 5, §58 #9): `pinger` sends `ping` once at
/// `offset_ticks` on its own clock; the component `responder`, fed by `responder_radio`'s
/// receive port, answers each PING it hears with `pong` on `responder_radio`'s transmit
/// stream, `turnaround_ns` after the PING's first sample; the output `rec` records
/// `pinger`'s receive port. `implementation` is the responder's `(impl.id, impl.hash)`.
/// How the two radios hear each other is the environment's (Vision §8).
#[allow(clippy::too_many_arguments)]
pub fn ping_pong(
    pinger: &str,
    responder_radio: &str,
    rate_hz: f64,
    ping: &ArtifactRef,
    pong: &ArtifactRef,
    offset_ticks: i64,
    turnaround_ns: i64,
    late_policy: &str,
    capture: i64,
    implementation: (&str, &ContentHash),
) -> Value {
    let radio = || {
        let mut requires = direction_requires("rx", 1, rate_hz, 1.0e9);
        requires.extend(direction_requires("tx", 1, rate_hz, 1.0e9));
        json!({ "kind": "radio.device", "requires": requires })
    };
    let mut resources = serde_json::Map::new();
    resources.insert(pinger.to_owned(), radio());
    resources.insert(responder_radio.to_owned(), radio());
    let param = |key: &str, default: Value| json!({ "key": key, "schema": {}, "update_class": "cold", "default": default });
    let ping_target = json!({ "kind": "resource", "resource": pinger, "path": "tx/s0" });
    json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 2 }, { "id": "sink", "major": 1 }] },
        "resources": resources,
        "inputs": [pong],
        "graph": {
            "components": {
                "responder": {
                    "id": "responder",
                    "kind": "reactor",
                    "ports": [{ "name": "rx", "direction": "in", "contract": "ezsdr.stream.cf32" }],
                    "params": [
                        param("ext.ezsdr.exec.native.ping.threshold", json!(0.02)),
                        param("ext.ezsdr.exec.native.ping.turnaround_ns", json!(turnaround_ns)),
                        param("ext.ezsdr.exec.native.ping.rearm_samples", json!(100)),
                        param("ext.ezsdr.exec.native.ping.target", json!(format!("{responder_radio}/tx/s0"))),
                        param("ext.ezsdr.exec.native.ping.waveform", json!(pong.hash)),
                        param("ext.ezsdr.exec.native.ping.late_policy", json!(late_policy))
                    ],
                    "timing": {},
                    "requires": { "executor_kind": "ezsdr.exec.native" },
                    "impl": { "kind": "ezsdr.impl.native", "id": implementation.0, "hash": implementation.1 }
                }
            },
            "links": [{
                "from": { "component": responder_radio, "port": "rx0" },
                "to": { "component": "responder", "port": "rx" },
                "policy": "drop_oldest",
                "capacity": 64
            }]
        },
        "outputs": [{
            "id": "rec",
            "kind": "sink.capture",
            "feed": rx_feed(pinger, 0),
            "params": { "sink.capture_samples": capture }
        }],
        "schedule": [{
            "at": { "clock": pinger, "offset_ticks": offset_ticks },
            "action": {
                "kind": "tx_burst",
                "target": ping_target,
                "waveform": ping,
                "repeat": false,
                "late_policy": "send_asap_and_flag",
                "metadata": {}
            }
        }],
        "policies": {},
        "extensions": {}
    })
}
