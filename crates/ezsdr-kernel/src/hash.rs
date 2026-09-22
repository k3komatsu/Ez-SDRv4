//! Content hashing — `00-overview.md` §7 (rules `OV-14`…`OV-17`).
//!
//! A content hash is `sha256:` followed by 64 lowercase hex digits, taken over the
//! document's canonical form: **RFC 8785 (JCS) with an integer profile**. A value
//! serialised from an integer type is written as an exact decimal whatever its
//! magnitude, while a value serialised from a floating-point type takes RFC 8785's
//! ECMAScript `Number::toString` form.
//!
//! The deviation is deliberate: strict JCS routes every number through an IEEE-754
//! double, so two Manifests whose epoch-to-UTC offsets differ by 1 ns in
//! 1.7 × 10^18 would share a hash. It is why a stock JCS crate **cannot** be
//! dropped in later (OV-15a).

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Why canonicalisation was refused (OV-15).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HashError {
    /// A number was NaN or infinite. Non-finite numbers are rejected rather than
    /// written as `null` (OV-15).
    NonFiniteNumber,
    /// An object key was not ASCII. All Ez-SDR keys are ASCII by the key-namespace
    /// rule, where byte order and UTF-16 order coincide, and the canonicaliser
    /// asserts that rather than implementing UTF-16 collation (OV-15).
    NonAsciiKey {
        /// The offending key.
        key: String,
    },
    /// The value could not be turned into JSON at all.
    NotSerialisable {
        /// What `serde_json` said.
        message: String,
    },
    /// The string was not `sha256:` plus 64 lowercase hex digits (OV-14).
    MalformedHash,
}

impl fmt::Display for HashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HashError::NonFiniteNumber => f.write_str("a non-finite number cannot be canonicalised"),
            HashError::NonAsciiKey { key } => write!(f, "object key {key:?} is not ASCII"),
            HashError::NotSerialisable { message } => write!(f, "not serialisable: {message}"),
            HashError::MalformedHash => f.write_str("a content hash is `sha256:` plus 64 lowercase hex digits"),
        }
    }
}

impl std::error::Error for HashError {}

/// `sha256:<64 lowercase hex>`. The algorithm is named in the value so a later
/// algorithm is an additive change (OV-14).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct ContentHash(String);

/// The marker [`serialize_finite_f64`] puts in its error so that [`ContentHash::of`]
/// can report [`HashError::NonFiniteNumber`] rather than a generic failure (OV-15).
const NON_FINITE_MARKER: &str = "ezsdr:non-finite";

/// Serialises a float, refusing a non-finite one.
///
/// The guard has to live here rather than in the canonicaliser: `serde_json` maps a
/// non-finite `f64` to `null` while building the `Value`, so by the time
/// [`canonical_json`] sees it the information is gone and a NaN drift would hash
/// identically to a drift that was never measured — in the one Manifest field
/// OV-15a exists to protect. Every Kernel-owned float document field uses it
/// through `#[serde(serialize_with = ...)]`.
///
/// Rule: OV-15.
pub fn serialize_finite_f64<S: serde::Serializer>(x: &f64, s: S) -> Result<S::Ok, S::Error> {
    if !x.is_finite() {
        return Err(serde::ser::Error::custom(format!(
            "{NON_FINITE_MARKER}: OV-15 rejects a non-finite number rather than writing `null`"
        )));
    }
    s.serialize_f64(*x)
}

impl ContentHash {
    /// The hash of a document's canonical form (OV-15, OV-16).
    pub fn of<T: Serialize>(value: &T) -> Result<ContentHash, HashError> {
        let v = serde_json::to_value(value).map_err(|e| {
            let message = e.to_string();
            if message.contains(NON_FINITE_MARKER) {
                HashError::NonFiniteNumber
            } else {
                HashError::NotSerialisable { message }
            }
        })?;
        ContentHash::of_value(&v)
    }

    /// The hash of an already-built JSON value's canonical form (OV-15).
    pub fn of_value(value: &serde_json::Value) -> Result<ContentHash, HashError> {
        Ok(ContentHash::of_bytes(canonical_json(value)?.as_bytes()))
    }

    /// The hash of raw bytes, which is what an artifact's hash is (OV-17).
    pub fn of_bytes(bytes: &[u8]) -> ContentHash {
        let digest = Sha256::digest(bytes);
        let mut s = String::with_capacity(7 + 64);
        s.push_str("sha256:");
        for b in digest {
            s.push(char::from_digit((b >> 4) as u32, 16).expect("nibble"));
            s.push(char::from_digit((b & 0xf) as u32, 16).expect("nibble"));
        }
        ContentHash(s)
    }

    /// Parses a hash read back from a document (OV-14).
    pub fn parse(s: &str) -> Result<ContentHash, HashError> {
        let hex = s.strip_prefix("sha256:").ok_or(HashError::MalformedHash)?;
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(HashError::MalformedHash);
        }
        Ok(ContentHash(s.to_owned()))
    }

    /// The hash as written (OV-14).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The canonical form: UTF-8 JSON text, object members sorted by key, no
/// insignificant whitespace, RFC 8785 string escaping, and the integer profile.
///
/// Equal inputs produce equal output across processes, platforms and map insertion
/// orders; two documents that differ only in key order or whitespace have one form.
///
/// Rule: OV-15, OV-15a, OV-16.
pub fn canonical_json(value: &serde_json::Value) -> Result<String, HashError> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

fn write_value(v: &serde_json::Value, out: &mut String) -> Result<(), HashError> {
    match v {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => out.push_str(&write_number(n)?),
        serde_json::Value::String(s) => out.push_str(&escape_string(s)),
        serde_json::Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            // OV-15: sorted here rather than relied on from `serde_json::Map`, because
            // any crate in the workspace enabling `preserve_order` would switch that
            // map to insertion order.
            let mut keys: Vec<&String> = map.keys().collect();
            if let Some(bad) = keys.iter().find(|k| !k.is_ascii()) {
                return Err(HashError::NonAsciiKey { key: (*bad).clone() });
            }
            keys.sort_unstable();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&escape_string(k));
                out.push(':');
                write_value(&map[*k], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// RFC 8785 string escaping, which is exactly what `serde_json` emits for a string
/// (OV-15). Reusing it keeps one implementation of the escape table.
fn escape_string(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
}

/// The integer profile of OV-15: an integer is an exact decimal with no exponent,
/// whatever its magnitude; anything else takes the ECMAScript `Number::toString`
/// form RFC 8785 requires.
fn write_number(n: &serde_json::Number) -> Result<String, HashError> {
    if let Some(i) = n.as_i64() {
        return Ok(i.to_string());
    }
    if let Some(u) = n.as_u64() {
        return Ok(u.to_string());
    }
    let f = n.as_f64().ok_or(HashError::NonFiniteNumber)?;
    ecmascript_number(f)
}

/// Whether an integer and a float are **one value**: whether they canonicalise to
/// one string, which under OV-15a is the same question as whether they hash the same.
///
/// SB-6 and SC-2 both state the criterion as "share one canonical form", with the
/// parenthetical that this holds while `|v| ≤ 2^53`. The parenthetical is not an iff,
/// and implementing it instead of the criterion was wrong in both directions:
/// `Int(10^16)` and `Num(1e16)` write the same `10000000000000000` and were held
/// unequal, while `Int(2^60)` and `Num(2^60 as f64)` write `1152921504606846976` and
/// `1152921504606847000` — two hashes — and compared equal under the exact ordering,
/// so an `Eq` constraint matched a capability the content hash calls a different
/// value. Asking the canonicaliser is exact in both directions by construction.
///
/// Rule: SB-6, SC-2, OV-15, OV-15a.
pub(crate) fn same_canonical_number(a: i64, b: f64) -> bool {
    ecmascript_number(b).is_ok_and(|written| written == a.to_string())
}

/// ECMAScript `Number::toString` as RFC 8785 §3.2.2.3 specifies it: plain decimal
/// for `1e-7 ≤ |x| < 1e21`, exponential otherwise, `-0` written as `0`.
fn ecmascript_number(x: f64) -> Result<String, HashError> {
    if !x.is_finite() {
        return Err(HashError::NonFiniteNumber);
    }
    if x == 0.0 {
        return Ok("0".to_owned()); // covers -0.0 (OV-15)
    }
    let sign = if x < 0.0 { "-" } else { "" };
    // Rust's `{:e}` is the shortest round-trip form, as `Number::toString` requires.
    let sci = format!("{:e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').expect("LowerExp always emits an exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let k = digits.len() as i32;
    // `value = digits × 10^(n − k)`, so n is one past the last integral digit.
    let n = exp.parse::<i32>().expect("LowerExp emits a decimal exponent") + 1;

    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign_e = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{sign_e}{}", e.abs())
        } else {
            format!("{}.{}e{sign_e}{}", &digits[..1], &digits[1..], e.abs())
        }
    };
    Ok(format!("{sign}{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::excessive_precision)] // the RFC 8785 vector is written as published
    fn ecmascript_layout_matches_rfc_8785_vectors() {
        for (input, want) in [
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (333333333.33333329, "333333333.3333333"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (2400000000.0, "2400000000"),
            (-0.0, "0"),
            (-1.5, "-1.5"),
        ] {
            assert_eq!(ecmascript_number(input).as_deref(), Ok(want), "for {input}");
        }
        assert_eq!(ecmascript_number(f64::NAN), Err(HashError::NonFiniteNumber));
        assert_eq!(ecmascript_number(f64::INFINITY), Err(HashError::NonFiniteNumber));
    }
}
