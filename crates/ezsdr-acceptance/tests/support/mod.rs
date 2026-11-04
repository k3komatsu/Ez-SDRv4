#![allow(dead_code)]

use std::collections::BTreeMap;

use ezsdr_acceptance::rig::{self, TempDir};
use ezsdr_kernel::coordinator::{self, Assembly, RunHandle, RunHandleError};
use ezsdr_kernel::event::{EventSource, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::run::Lease;
use ezsdr_kernel::session::InitialRequests;
use ezsdr_kernel::spec::{Key, Scalar};
use serde_json::Value;

pub const T0: i64 = 2_000_000_000;

pub fn spec_run(_temp: &TempDir, spec: &Value, profile: &Value, inputs: BTreeMap<ContentHash, Vec<u8>>) -> RunHandle {
    coordinator::start_spec_run(spec, profile, rig::assemble(profile, inputs)).expect("Spec Run starts")
}

/// RM-30: no channel is in a stream unless a Session asks. The request that puts receive channel
/// c0 of every radio of `assembly` in stream 0, which most runs receive on.
pub fn rx0(assembly: &Assembly) -> InitialRequests {
    let key = Key::parse("radio.rx.c0.stream").unwrap();
    assembly
        .providers
        .iter()
        .filter(|(_, provider)| provider.instance().tree.capabilities.contains_key(&key))
        .map(|(name, _)| (name.clone(), BTreeMap::from([(key.clone(), Scalar::Int(0))])))
        .collect()
}

/// As [`rx0`], and transmit channel c0 of every radio in stream 0 as well: the Session of a
/// loopback, which receives what it transmits.
pub fn rx0_tx0(assembly: &Assembly) -> InitialRequests {
    let tx = Key::parse("radio.tx.c0.stream").unwrap();
    let mut initial = rx0(assembly);
    for (name, provider) in &assembly.providers {
        if provider.instance().tree.capabilities.contains_key(&tx) {
            initial.entry(name.clone()).or_default().insert(tx.clone(), Scalar::Int(0));
        }
    }
    initial
}

/// A Session that receives on c0 and transmits on c0, both asked for at `connect`.
pub fn loopback_run(temp: &TempDir, profile: &Value) -> RunHandle {
    let assembly = rig::assemble(profile, BTreeMap::new());
    let initial = rx0_tx0(&assembly);
    session_run_with(temp, profile, assembly, &initial)
}

pub fn session_run(temp: &TempDir, profile: &Value) -> RunHandle {
    let assembly = rig::assemble(profile, BTreeMap::new());
    let initial = rx0(&assembly);
    session_run_with(temp, profile, assembly, &initial)
}

/// A Session that asks for `initial` at `connect` (RS-12).
pub fn session_run_with(_temp: &TempDir, profile: &Value, assembly: Assembly, initial: &InitialRequests) -> RunHandle {
    coordinator::connect(profile, initial, assembly, Lease::attached()).expect("Session connects")
}

pub fn root(run: &RunHandle) -> ClockDomainId {
    run.now().domain()
}

pub fn finish_at(mut run: RunHandle, root: ClockDomainId, tick: i64) -> Manifest {
    match run.run_until_end(ezsdr_kernel::time::TimePoint::new(root, tick)) {
        Ok(()) | Err(RunHandleError::Ended { .. }) => {}
        Err(error) => panic!("advance Spec Run: {error}"),
    }
    run.finish()
}

pub fn end_session(mut run: RunHandle, root: ClockDomainId, tick: i64) -> Manifest {
    run.advance_to(ezsdr_kernel::time::TimePoint::new(root, tick)).expect("advance Session");
    run.finish()
}

/// The section `name` the instance whose node is `node` wrote (RS-39).
pub fn section<'a>(manifest: &'a Manifest, node: &str, name: &str) -> &'a Value {
    manifest.section(&source(node), name).unwrap_or_else(|| panic!("no section {name} from {node}"))
}

/// A Provider node's event source (KC-8).
pub fn source(node: &str) -> EventSource {
    EventSource::Node { node: ezsdr_kernel::id::ResourceId::parse(node).unwrap() }
}

/// A resource target, `<resource>[/<path>]` (KC-23).
pub fn resource(path: &str) -> Target {
    let (resource, path) = path.split_once('/').unwrap_or((path, ""));
    Target::Resource { resource: ezsdr_kernel::spec::Ident::parse(resource).unwrap(), path: path.to_owned() }
}

/// An output target (KC-23).
pub fn output(name: &str) -> Target {
    Target::Output { output: ezsdr_kernel::spec::Ident::parse(name).unwrap() }
}

/// The `n`-th artifact the Sink of `output` returned (RS-38).
pub fn artifact<'a>(manifest: &'a Manifest, output: &str, n: usize) -> &'a ezsdr_kernel::manifest::ArtifactRef {
    manifest.artifacts.iter().find(|(o, _)| o.as_str() == output).and_then(|(_, list)| list.get(n)).expect("artifact exists")
}

pub fn modules(manifest: &Manifest) -> Vec<&str> {
    manifest.modules.iter().map(|entry| entry.module.id.as_str()).collect()
}

/// The `InitialRequests` of one radio, `resource`, from `(key, value)` pairs: the form
/// `connect` puts in the implicit Spec's `requires` (RS-12).
pub fn asking(resource: &str, requests: &[(&str, Scalar)]) -> InitialRequests {
    [(ezsdr_kernel::spec::Ident::parse(resource).unwrap(), requests.iter().map(|(name, value)| (Key::parse(name).unwrap(), value.clone())).collect())].into_iter().collect()
}
