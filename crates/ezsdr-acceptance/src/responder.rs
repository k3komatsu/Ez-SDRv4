//! The Mini Reactive Radio's Reactor (Vision §67 Phase 5, §58 #9): a component that
//! hears a PING on its receive port and answers with a timed PONG.
//!
//! It is application logic, so it speaks only the Kernel contract and the native
//! Executor's component ABI, and names no simulated device (§58 #10). It detects a PING as the
//! first sample whose power exceeds a threshold — the instant a packet detector would
//! report, `k` samples into a block whose first sample has a device time (Vision §23) —
//! and hands its Executor a `TxBurst` of its PONG waveform at that instant plus its
//! turnaround, in the receive stream's SampleClock, which the Executor submits (NX-6).
//! Admission converts the target onto the transmit clock (SC-23a), and the radio decides
//! the burst's lead (MA-14, SC-27).
//!
//! What it does not do, being a Mini radio's detector: it reads channel 0 only and ignores
//! per-channel validity (SC-14), and it counts its rearm in samples delivered, so a gap
//! (`GAP_BEFORE`, SC-13) shortens the quiet it waits for by the samples the gap lost.

use std::sync::Arc;

use ezsdr_exec_native::{Component, ComponentContext, Implementation};
use ezsdr_kernel::event::Action;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{Endpoint, ModuleError, StepOutcome};
use ezsdr_kernel::spec::{Ident, Namespace, Value};
use ezsdr_kernel::stream::{DataLink, LatePolicy};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockRegistry, Duration, TimePoint};

/// The implementation id a Spec's `impl.id` names.
pub const IMPL_ID: &str = "ezsdr.acceptance.ping_responder";

/// The implementation hash a Spec's `impl.hash` names.
pub fn impl_hash() -> ContentHash {
    ContentHash::of_bytes(b"ezsdr.acceptance.ping_responder 1.0.0")
}

/// The compiled-in implementation, for [`ezsdr_exec_native::NativeExecutor::new`].
pub fn implementation() -> Implementation {
    Implementation { id: IMPL_ID.to_owned(), hash: impl_hash(), make: || Box::new(PingResponder::default()) }
}

/// The parameter keys, under the native Executor's `ext.` prefix (MA-34).
pub mod params {
    /// Power above which a sample is a PING, `re² + im²` (num).
    pub const THRESHOLD: &str = "ext.ezsdr.exec.native.ping.threshold";
    /// From the PING's first sample to the PONG's target, in nanoseconds (int).
    pub const TURNAROUND_NS: &str = "ext.ezsdr.exec.native.ping.turnaround_ns";
    /// Quiet samples after which the next PING is answered (int).
    pub const REARM_SAMPLES: &str = "ext.ezsdr.exec.native.ping.rearm_samples";
    /// The transmit stream, Spec-relative (str).
    pub const TARGET: &str = "ext.ezsdr.exec.native.ping.target";
    /// The PONG waveform's content hash, one of the Spec's inputs (str).
    pub const WAVEFORM: &str = "ext.ezsdr.exec.native.ping.waveform";
    /// `send_asap_and_flag` or `drop_and_flag` (str).
    pub const LATE_POLICY: &str = "ext.ezsdr.exec.native.ping.late_policy";
}

#[derive(Default)]
struct PingResponder {
    link: Option<Arc<dyn DataLink>>,
    clocks: Option<Arc<ClockRegistry>>,
    threshold: f64,
    turnaround_ns: i64,
    rearm: u64,
    target: Option<ResourceId>,
    waveform: Option<ArtifactRef>,
    late_policy: Option<LatePolicy>,
    /// Quiet samples seen since the last loud one; the responder answers only once this
    /// reaches `rearm`, so one PING is answered once.
    quiet: u64,
}

fn param<'a>(ctx: &'a ComponentContext, key: &str) -> Result<&'a Value, ModuleError> {
    ctx.descriptor
        .params
        .iter()
        .find(|p| p.key.as_str() == key)
        .map(|p| &p.default)
        .ok_or_else(|| ModuleError::rejected(format!("responder: parameter {key} is missing")))
}

fn wrong(key: &str) -> ModuleError {
    ModuleError::rejected(format!("responder: parameter {key} has the wrong kind"))
}

impl Component for PingResponder {
    fn prepare(&mut self, ctx: ComponentContext) -> Result<(), ModuleError> {
        self.threshold = match param(&ctx, params::THRESHOLD)? {
            Value::Num(v) => *v,
            Value::Int(v) => *v as f64,
            _ => return Err(wrong(params::THRESHOLD)),
        };
        let Value::Int(turnaround) = param(&ctx, params::TURNAROUND_NS)? else { return Err(wrong(params::TURNAROUND_NS)) };
        let Value::Int(rearm) = param(&ctx, params::REARM_SAMPLES)? else { return Err(wrong(params::REARM_SAMPLES)) };
        let Value::Str(target) = param(&ctx, params::TARGET)? else { return Err(wrong(params::TARGET)) };
        let Value::Str(hash) = param(&ctx, params::WAVEFORM)? else { return Err(wrong(params::WAVEFORM)) };
        let Value::Str(policy) = param(&ctx, params::LATE_POLICY)? else { return Err(wrong(params::LATE_POLICY)) };
        if *turnaround < 0 || *rearm < 1 {
            return Err(ModuleError::rejected("responder: turnaround must be ≥ 0 and rearm ≥ 1"));
        }
        self.turnaround_ns = *turnaround;
        self.rearm = *rearm as u64;
        self.quiet = self.rearm;
        self.target = Some(ResourceId::parse(target).map_err(|_| wrong(params::TARGET))?);
        self.late_policy = Some(match policy.as_str() {
            "send_asap_and_flag" => LatePolicy::SendAsapAndFlag,
            "drop_and_flag" => LatePolicy::DropAndFlag,
            _ => return Err(wrong(params::LATE_POLICY)),
        });
        let hash: ContentHash = serde_json::from_value(serde_json::Value::String(hash.clone())).map_err(|_| wrong(params::WAVEFORM))?;
        let bytes = ctx.inputs.get(&hash).ok_or_else(|| ModuleError::rejected(format!("responder: the PONG waveform {hash} is not an input of this Run")))?;
        self.waveform = Some(ArtifactRef {
            id: Ident::parse("pong").expect("a valid artifact id"),
            kind: Namespace::parse("ezsdr.input").expect("a valid artifact kind"),
            uri: format!("mem:{hash}"),
            hash,
            size_bytes: bytes.len() as u64,
            partial: false,
            marks: Vec::new(),
            continuity: Vec::new(),
        });
        for attached in &ctx.links {
            match (&attached.endpoint, attached.port.as_str()) {
                (Endpoint::StreamIn(link), "rx") => self.link = Some(link.clone()),
                _ => return Err(ModuleError::rejected(format!("responder: unexpected link end on port {}", attached.port))),
            }
        }
        if self.link.is_none() {
            return Err(ModuleError::rejected("responder: port rx is not linked"));
        }
        self.clocks = Some(ctx.clocks.clone());
        Ok(())
    }

    fn step(&mut self, _until: TimePoint, out: &mut Vec<Action>) -> Result<StepOutcome, ModuleError> {
        let Some(link) = self.link.clone() else { return Ok(StepOutcome { progressed: false }) };
        let mut progressed = false;
        while let Some(block) = link.receive() {
            progressed = true;
            let header = block.header();
            let bytes = block.host_bytes().ok_or_else(|| ModuleError::rejected("responder: a block is not in host memory"))?;
            let first = header.first_sample_time;
            for k in 0..header.len as usize {
                // Channel 0 of SC-4's planar cf32 layout.
                let at = k * 8;
                let re = f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
                let im = f32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("four bytes"));
                let loud = f64::from(re).powi(2) + f64::from(im).powi(2) > self.threshold;
                if !loud {
                    self.quiet = self.quiet.saturating_add(1);
                    continue;
                }
                if self.quiet >= self.rearm {
                    out.push(self.answer(TimePoint::new(first.domain, first.ticks + k as i64))?);
                }
                self.quiet = 0;
            }
        }
        Ok(StepOutcome { progressed })
    }

    fn cleanup(&mut self) {
        self.link = None;
        self.clocks = None;
    }
}

impl PingResponder {
    /// The PONG at `heard` plus the turnaround, both in the receive SampleClock.
    fn answer(&self, heard: TimePoint) -> Result<Action, ModuleError> {
        let clocks = self.clocks.as_ref().expect("prepared");
        let turnaround = clocks
            .rescale(Duration::new(ClockDomainId::HOST_MONOTONIC, self.turnaround_ns), heard.domain)
            .map_err(|error| ModuleError::rejected(format!("responder: {error}")))?;
        let ticks = match turnaround {
            ezsdr_kernel::time::Rescaled::Exact { duration } => duration.ticks,
            ezsdr_kernel::time::Rescaled::Inexact { floor, .. } => floor.ticks + 1,
        };
        Ok(Action::TxBurst {
            target: self.target.clone().expect("prepared"),
            waveform: self.waveform.clone().expect("prepared"),
            repeat: false,
            at: AbsoluteDeadline::new(TimePoint::new(heard.domain, heard.ticks + ticks)),
            requested_at: None,
            late_policy: self.late_policy.expect("prepared"),
            metadata: Default::default(),
        })
    }
}
