//! Portable experiment documents expressed only through the Kernel contract.

use std::collections::BTreeMap;

use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::spec::{Ident, Namespace};
use serde_json::{json, Value};

/// A receive experiment with a recorder connected to the receive port.
pub fn receive(channels: i64, rate_hz: f64, frequency_hz: f64, capture: Option<i64>) -> Value {
    let mut params = BTreeMap::new();
    if let Some(samples) = capture {
        params.insert("sink.capture_samples", json!(samples));
    }
    let mut output = json!({
        "id": "rec",
        "kind": "sink.capture",
        "feed": {
            "port": { "component": "radio", "port": "rx" },
            "policy": "drop_oldest",
            "capacity": 64
        },
        "params": {}
    });
    for (key, value) in params {
        output["params"][key] = value;
    }
    json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 1 }, { "id": "sink", "major": 1 }] },
        "resources": {
            "radio": {
                "kind": "radio.device",
                "requires": {
                    "radio.rx.channels": { "kind": "eq", "value": channels },
                    "radio.rx.sample_rate_hz": { "kind": "eq", "value": rate_hz },
                    "radio.rx.frequency_hz": { "kind": "eq", "value": frequency_hz }
                }
            }
        },
        "outputs": [output],
        "policies": {},
        "extensions": {}
    })
}

/// A receive experiment with a scheduled transmit burst.
pub fn transmit(
    rate_hz: f64,
    waveform: &ArtifactRef,
    repeat: bool,
    late_policy: &str,
    offset_ticks: i64,
    capture: Option<i64>,
) -> Value {
    let mut spec = receive(1, rate_hz, 1.0e9, capture);
    spec["resources"]["radio"]["requires"]["radio.tx.channels"] = json!({ "kind": "eq", "value": 1 });
    spec["resources"]["radio"]["requires"]["radio.tx.sample_rate_hz"] = json!({ "kind": "eq", "value": rate_hz });
    spec["resources"]["radio"]["requires"]["radio.tx.frequency_hz"] = json!({ "kind": "eq", "value": 1.0e9 });
    spec["schedule"] = json!([{
        "at": { "clock": "radio", "offset_ticks": offset_ticks },
        "action": {
            "kind": "tx_burst",
            "target": serde_json::to_value(ResourceId::parse("radio/tx").expect("valid resource id")).expect("resource id serializes"),
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
    spec["schedule"]
        .as_array_mut()
        .expect("experiment schedule is an array")
        .push(json!({
            "at": { "clock": "radio", "offset_ticks": offset_ticks },
            "action": {
                "kind": "update_parameter",
                "target": serde_json::to_value(ResourceId::parse("sink/rec").expect("valid resource id")).expect("resource id serializes"),
                "key": "sink.capture_samples",
                "value": samples,
                "class": "block_boundary"
            }
        }));
    spec
}

/// Builds zero-valued complex samples and their content-addressed input reference.
pub fn waveform(samples: usize) -> (Vec<u8>, ArtifactRef) {
    let bytes = vec![0; samples.checked_mul(8).expect("waveform size fits memory")];
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
