#![allow(dead_code)]

use std::collections::BTreeMap;

use ezsdr_acceptance::rig::{self, TempDir};
use ezsdr_kernel::coordinator::{self, RunHandle, RunHandleError};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::run::Lease;
use serde_json::Value;

pub const T0: i64 = 2_000_000_000;

pub fn spec_run(
    _temp: &TempDir,
    spec: &Value,
    profile: &Value,
    inputs: BTreeMap<ContentHash, Vec<u8>>,
) -> RunHandle {
    coordinator::start_spec_run(spec, profile, rig::assemble(profile, inputs)).expect("Spec Run starts")
}

pub fn session_run(_temp: &TempDir, profile: &Value) -> RunHandle {
    coordinator::connect(profile, rig::assemble(profile, BTreeMap::new()), Lease::attached()).expect("Session connects")
}

pub fn root(run: &RunHandle) -> ClockDomainId { run.now().domain }

pub fn finish_at(mut run: RunHandle, root: ClockDomainId, tick: i64) -> Manifest {
    match run.run_until_end(ezsdr_kernel::time::TimePoint::new(root, tick)) {
        Ok(()) | Err(RunHandleError::Ended { .. }) => {},
        Err(error) => panic!("advance Spec Run: {error}"),
    }
    run.finish()
}

pub fn end_session(mut run: RunHandle, root: ClockDomainId, tick: i64) -> Manifest {
    run.advance_to(ezsdr_kernel::time::TimePoint::new(root, tick)).expect("advance Session");
    run.finish()
}

pub fn section<'a>(manifest: &'a Manifest, name: &str) -> &'a Value {
    &manifest.sections[&ezsdr_kernel::spec::Namespace::parse(name).unwrap()]
}

pub fn artifact<'a>(manifest: &'a Manifest, id: &str) -> &'a ezsdr_kernel::manifest::ArtifactRef {
    manifest.artifacts.iter().find(|artifact| artifact.id.as_str() == id).expect("artifact exists")
}

pub fn modules(manifest: &Manifest) -> Vec<&str> {
    manifest.modules.iter().map(|entry| entry.module.id.as_str()).collect()
}
