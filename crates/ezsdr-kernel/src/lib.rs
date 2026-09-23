//! Ez-SDR v4 Kernel semantic model.
//!
//! Phase 1 of Vision §67: the decisions every Module signature depends on, with no
//! MockRadio, no Simulation Engine and no hardware. The crate is the Core tier of
//! Vision §5 and nothing on the Vision §6 list appears in it.
//!
//! # Rule index
//!
//! Every public item's doc comment cites the rule it implements; `kernel_surface`
//! fails the build when one does not (OV-23).
//!
//! | Prefix | Spec |
//! |---|---|
//! | `OV-n` | `plan/phase1/00-overview.md` — governance, crate layout, schemas, hashing |
//! | `TM-n` | `01-time-model.md` — [`time`] |
//! | `SC-n` | `02-stream-contract.md` — [`stream`], [`contract`] |
//! | `SB-n` | `03-spec-and-binding.md` — [`spec`], [`binding`], [`plan`] |
//! | `RS-n` | `04-run-and-session.md` — [`event`], [`policy`], [`run`], [`session`], [`manifest`] |
//! | `MA-n` | `05-module-api.md` — [`module_api`] |

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// BindingProfile, the generic matcher and the admission-check hook (SB-6…SB-36).
pub mod binding;
/// The DataContract registry and Port (SC-1…SC-5).
pub mod contract;
/// Events, counters and the Kernel Action set (RS-26…RS-36, RS-48…RS-52).
pub mod event;
/// Canonical JSON and content hashes (OV-14…OV-17).
pub mod hash;
/// Node-qualified identifiers (X7; TM-11, SC-6, SB-3, MA-38, RS-1).
pub mod id;
/// The Manifest envelope and `ArtifactRef` (RS-38…RS-47).
pub mod manifest;
/// The three axes, the five role traits and the registry (MA-1…MA-46).
pub mod module_api;
/// The ExecutionPlan, the compile pipeline and Island admission (SB-37…SB-46, MA-22, MA-39).
pub mod plan;
/// The event-kind registry and the Policy table (RS-26…RS-29).
pub mod policy;
/// The Run state machine, cleanup and the Lease (RS-1…RS-11, RS-21…RS-25).
pub mod run;
/// JSON Schema generation for every document type (OV-10…OV-13).
pub mod schema;
/// Sessions, the action log and `admit()` (RS-12…RS-20).
pub mod session;
/// Identifiers, values, constraints and the ExperimentSpec envelope (SB-1…SB-20).
pub mod spec;
/// The Stream Contract: blocks, links, bursts and continuity (SC-1…SC-32).
pub mod stream;
/// The Kernel time model: ticks, domains, deadlines and the Authority (TM-1…TM-21).
pub mod time;
