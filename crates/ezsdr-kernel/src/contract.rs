//! DataContract registry and Port — `02-stream-contract.md` SC-1…SC-5 (Vision §21).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::stream::StreamError;

/// A namespaced contract id matching `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$`,
/// for example `ezsdr.stream.cf32`.
///
/// Rule: SB-1, SC-2.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct DataContractId(String);

impl<'de> Deserialize<'de> for DataContractId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::id::parsed(d, "a DataContractId", DataContractId::parse)
    }
}

impl DataContractId {
    /// Parses a namespaced id; at least two dot-separated segments (SC-2).
    pub fn parse(id: &str) -> Result<DataContractId, StreamError> {
        let bad = |reason: &str| StreamError::InvalidBlock { reason: reason.to_owned() };
        let mut segments = 0;
        for seg in id.split('.') {
            segments += 1;
            let mut chars = seg.chars();
            match chars.next() {
                Some('a'..='z') => {}
                _ => return Err(bad("contract segment must start with a lowercase letter")),
            }
            if chars.any(|c| !matches!(c, 'a'..='z' | '0'..='9' | '_')) {
                return Err(bad("contract segment holds a character outside [a-z0-9_]"));
            }
        }
        if segments < 2 {
            return Err(bad("contract id needs at least two dot-separated segments"));
        }
        Ok(DataContractId(id.to_owned()))
    }

    /// The id as written (SC-2).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DataContractId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An attribute value in a `DataContract`. The Kernel stores these and never
/// interprets them, which is what keeps it from becoming a type system (SC-2).
///
/// Carries no tag (OV-13's carve-out): an attribute value is the scalar an author
/// wrote, not a two-field object wrapping it.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Scalar {
    /// A signed 64-bit integer, written as an exact decimal by the canonicaliser (OV-15).
    Int(i64),
    /// A 64-bit float, written per RFC 8785's number rule; a non-finite one is
    /// refused rather than written as `null` (OV-15).
    Float(#[serde(serialize_with = "crate::hash::serialize_finite_f64")] f64),
    /// A string.
    Str(String),
    /// A boolean.
    Bool(bool),
}

/// Numeric equality crosses `Int` and `Float`, because `1` and `1.0` are one value
/// under SB-6 and share one canonical form and one hash under OV-15a (finding D24).
/// Without it an attribute an author wrote as `1` would not match the same contract
/// re-registered from a Rust literal `1.0`, and SC-4's "identical re-registration is
/// a no-op" would report a conflict.
///
/// Rule: SC-2, SB-6, OV-15a.
impl PartialEq for Scalar {
    fn eq(&self, other: &Scalar) -> bool {
        match (self, other) {
            (Scalar::Int(a), Scalar::Int(b)) => a == b,
            (Scalar::Float(a), Scalar::Float(b)) => a == b,
            // Compared exactly, never through `as f64`: above 2^53 that cast made
            // two values with different canonical forms and different hashes
            // compare equal, so `ContractRegistry::register` took a genuinely
            // different definition for an idempotent re-registration and discarded
            // it without a diagnostic (SC-2). It also made equality non-transitive.
            // Equal iff the two share one canonical form under OV-15. Below 2^53
            // every integer is uniquely representable as an `f64`, so no shorter
            // decimal round-trips to it and the integer profile's exact decimal and
            // `ecmascript_number`'s shortest round-trip agree. At or above it they
            // diverge: `i64::MIN` and -2^63 are the same number and `try_from`
            // succeeds, but the canonicaliser writes `-9223372036854775808` and
            // `-9223372036854776000`, so accepting them as equal let SC-2 take a
            // genuinely different definition for a re-registration again.
            (Scalar::Int(a), Scalar::Float(b)) | (Scalar::Float(b), Scalar::Int(a)) => {
                crate::spec::cmp_int_num(*a, *b) == Some(std::cmp::Ordering::Equal)
            }
            (Scalar::Str(a), Scalar::Str(b)) => a == b,
            (Scalar::Bool(a), Scalar::Bool(b)) => a == b,
            _ => false,
        }
    }
}

/// A registered data contract: an id, a fixed attribute map, and the set of
/// producer contracts it accepts without conversion.
///
/// Rule: SC-2, SC-4.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DataContract {
    /// The namespaced id; identity is the id, not the shape (decision S7).
    pub id: DataContractId,
    /// Attributes fixed at registration, for example `bytes_per_sample` (SC-4).
    pub attributes: BTreeMap<String, Scalar>,
    /// Producer contracts this one accepts unconverted; empty at v4.0 (SC-3, SC-8).
    pub compatible_from: BTreeSet<DataContractId>,
}

impl DataContract {
    /// The `bytes_per_sample` attribute, which SC-10a's buffer check needs (SC-4).
    pub fn bytes_per_sample(&self) -> Option<u32> {
        match self.attributes.get("bytes_per_sample") {
            Some(Scalar::Int(n)) => u32::try_from(*n).ok(),
            _ => None,
        }
    }
}

/// Which way samples flow through a `Port` (SC-1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum PortDirection {
    /// The component consumes blocks here.
    In,
    /// The component produces blocks here.
    Out,
}

/// A name, a direction and a contract id, and nothing more. Per-port parameters
/// the Kernel does not interpret — a tensor shape, a maximum PDU length — live in
/// the `ComponentDescriptor`'s parameters (MA-36), not here.
///
/// Rule: SC-1.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Port {
    /// Port name, unique within its component (MA-37); an `Ident` (SB-1, SC-1).
    pub name: crate::spec::Ident,
    /// Whether the component consumes or produces here (SC-1).
    pub direction: PortDirection,
    /// The contract carried (SC-1).
    pub contract: DataContractId,
}

/// Names one port of one component instance (SC-1).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PortRef {
    /// The component, Spec resource or output the port belongs to (SB-15, SB-22a).
    pub component: crate::spec::Ident,
    /// The port name on that component (SC-1).
    pub port: crate::spec::Ident,
}

/// The registry of data contracts. Registering an identical definition twice is a
/// no-op; registering a different definition under an existing id fails.
///
/// Rule: SC-2, SC-3.
#[derive(Default)]
pub struct ContractRegistry {
    contracts: RwLock<BTreeMap<DataContractId, DataContract>>,
}

impl ContractRegistry {
    /// An empty registry (SC-2).
    pub fn new() -> ContractRegistry {
        ContractRegistry::default()
    }

    /// A registry holding the contracts SC-4 registers at v4.0, which serve as the
    /// Phase 1 test fixtures (SC-4).
    pub fn with_standard_contracts() -> ContractRegistry {
        let reg = ContractRegistry::new();
        for c in standard_contracts() {
            reg.register(c).expect("the SC-4 fixtures do not conflict");
        }
        reg
    }

    /// Registers a contract. An identical re-registration is a no-op; a different
    /// definition under an existing id fails (SC-2).
    pub fn register(&self, contract: DataContract) -> Result<(), StreamError> {
        let mut map = self.contracts.write().unwrap_or_else(|e| e.into_inner());
        match map.get(&contract.id) {
            Some(existing) if *existing == contract => Ok(()),
            Some(_) => Err(StreamError::InvalidBlock {
                reason: format!("contract {} is already registered with a different definition", contract.id),
            }),
            None => {
                map.insert(contract.id.clone(), contract);
                Ok(())
            }
        }
    }

    /// Every registered id, which MA-37's structural check compares a component's
    /// port contracts against.
    ///
    /// Rule: SC-2, MA-37.
    pub fn ids(&self) -> Vec<DataContractId> {
        self.contracts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// The registered contract, if any (SC-2).
    pub fn get(&self, id: &DataContractId) -> Option<DataContract> {
        self.contracts.read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    /// Whether a link from producer contract `from` to consumer contract `to` is
    /// admissible: `from == to`, or `from` is in `to`'s `compatible_from`. The check
    /// is directional, and it is the only contract check the Kernel performs.
    ///
    /// Rule: SC-3.
    pub fn check_link(
        &self,
        from: &DataContractId,
        to: &DataContractId,
    ) -> Result<(), StreamError> {
        if from == to {
            return Ok(());
        }
        let consumer = self.get(to).ok_or_else(|| StreamError::InvalidBlock {
            reason: format!("contract {to} is not registered"),
        })?;
        if consumer.compatible_from.contains(from) {
            Ok(())
        } else {
            Err(StreamError::Incompatible { from: from.clone(), to: to.clone() })
        }
    }
}

/// The contracts registered at v4.0. Their definitions belong to the contracts
/// Vocabulary; they are listed here because Phase 1 needs them as fixtures.
/// `layout: "planar"` means channel `c` occupies the byte range
/// `[c · len · bytes_per_sample, (c+1) · len · bytes_per_sample)` of the buffer.
///
/// Rule: SC-4.
pub fn standard_contracts() -> Vec<DataContract> {
    let attrs = |pairs: &[(&str, Scalar)]| -> BTreeMap<String, Scalar> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
    };
    let id = |s: &str| DataContractId::parse(s).expect("SC-4 ids are well formed");
    vec![
        DataContract {
            id: id("ezsdr.stream.cf32"),
            attributes: attrs(&[
                ("bytes_per_sample", Scalar::Int(8)),
                ("full_scale", Scalar::Float(1.0)),
                ("layout", Scalar::Str("planar".to_owned())),
            ]),
            compatible_from: BTreeSet::new(),
        },
        DataContract {
            id: id("ezsdr.stream.sc16"),
            attributes: attrs(&[
                ("bytes_per_sample", Scalar::Int(4)),
                ("full_scale", Scalar::Float(32767.0)),
                ("layout", Scalar::Str("planar".to_owned())),
            ]),
            compatible_from: BTreeSet::new(),
        },
        DataContract {
            id: id("ezsdr.control"),
            attributes: BTreeMap::new(),
            compatible_from: BTreeSet::new(),
        },
    ]
}
