//! Shared helpers for MockRadio's layout fixtures.
#![allow(dead_code)]

use std::collections::BTreeMap;

use ezsdr_kernel::spec::{Ident, Key, Scalar, Value};
use serde_json::{json, Value as JsonValue};

/// Builds an `x3xx` layout selector using the numbered channel-and-stream key set.
pub fn x3xx_selector<R: AsRef<str>, T: AsRef<str>>(rx: &[R], tx: &[T]) -> BTreeMap<Ident, Value> {
    let list = |elements: &[R]| Value::List(elements.iter().map(|element| Scalar::from(element.as_ref())).collect());
    let tx_list = Value::List(tx.iter().map(|element| Scalar::from(element.as_ref())).collect());
    let mut selector = layout_selector("mock", list(rx), tx_list);
    selector.remove(&Ident::parse("id").unwrap());
    selector
}

/// Builds an `x3xx` selector with caller-supplied layout values.
pub fn layout_selector(id: &str, rx: Value, tx: Value) -> BTreeMap<Ident, Value> {
    BTreeMap::from([
        (Ident::parse("id").unwrap(), Value::from(id)),
        (Ident::parse("rx").unwrap(), rx),
        (Ident::parse("tx").unwrap(), tx),
    ])
}

/// Adds a Mock device id to the shared `x3xx` layout selector.
pub fn mock_x3xx_selector<R: AsRef<str>, T: AsRef<str>>(id: &str, rx: &[R], tx: &[T]) -> BTreeMap<Ident, Value> {
    let mut selector = x3xx_selector(rx, tx);
    selector.insert(Ident::parse("id").unwrap(), Value::from(id));
    selector
}

/// Builds an `ideal` layout selector with its channel counts and device id.
pub fn ideal_selector(id: &str, rx: i64, tx: i64) -> BTreeMap<Ident, Value> {
    BTreeMap::from([
        (Ident::parse("id").unwrap(), Value::from(id)),
        (Ident::parse("rx").unwrap(), Value::from(rx)),
        (Ident::parse("tx").unwrap(), Value::from(tx)),
    ])
}

/// Adds the layout selector and default channel counts to an acceptance binding selector.
pub fn binding_selector(profile: &str, mut selector: JsonValue) -> JsonValue {
    let object = selector.as_object_mut().expect("a binding selector is an object");
    if profile == "x3xx" {
        object.entry("rx").or_insert(json!(["0:A:ubx:0", "0:B:ubx:0"]));
        object.entry("tx").or_insert(json!(["0:A:ubx:0", "0:B:ubx:0"]));
    } else {
        object.entry("rx").or_insert(json!(2));
        object.entry("tx").or_insert(json!(2));
    }
    selector
}

/// Looks up an effective layout key in a prepared configuration.
pub fn effective<'a>(configuration: &'a BTreeMap<Key, Value>, name: &str) -> &'a Value {
    &configuration[&Key::parse(name).unwrap()]
}
