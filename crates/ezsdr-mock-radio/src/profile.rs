//! The `x3xx` and `ideal` layout profiles the Mock presents (MR-3, design 33 D5).

use std::collections::BTreeMap;

use ezsdr_kernel::id::{MemoryDomainId, ResourceId};
use ezsdr_kernel::module_api::{
    CoerceReport, CoercionFidelity, EnvelopeFidelity, Fidelity, ModuleError, ProfileRef,
    Requested, Resource, RfFidelity, TransportFidelity,
};
use ezsdr_kernel::spec::{Key, Value};
use ezsdr_kernel::stream::Direction;
use ezsdr_radio::channels;
use ezsdr_radio::x3xx::{self, ProviderTerms};
use ezsdr_radio::{PerformanceEnvelope, RadioEnvelope, TimingEnvelope};

/// The samples in one receive block (MR-3).
pub const BLOCK_LEN: u32 = 2_000;

/// Which profile the Mock presents (MR-3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileKind {
    /// The measured-envelope target for an X3xx (MR-3, design 33 D5).
    X3xx,
    /// A wide-range, no-latency test profile (MR-3).
    Ideal,
}

/// One profile (MR-3), described by its layout's numbered channel and stream keys.
#[derive(Clone, PartialEq, Debug)]
pub struct Profile {
    /// The selected profile (MR-3).
    pub kind: ProfileKind,
    /// The layout description, built once when the binding is read (RM-29).
    description: channels::Description,
    /// RM-30's defaults, the `effective` an unconstrained key takes (MR-5).
    defaults: BTreeMap<Key, Value>,
}

impl Profile {
    /// The profile of a binding, with the description built once from its layout (RM-29).
    pub(crate) fn new(kind: ProfileKind, layout: channels::Layout) -> Profile {
        let description = channels::Description::new(&layout, BLOCK_LEN, &Profile::terms());
        let defaults = description.defaults();
        Profile { kind, description, defaults }
    }

    /// The values only the Mock contributes to the `x3xx` description (design 33 D5): its
    /// 2 ms lead (INFERRED, measured by Phase 8) and its Replay-like repeat constraints.
    pub fn terms() -> ProviderTerms {
        ProviderTerms {
            min_timed_command_lead_ns: 2_000_000,
            repeat_max_samples: 268_435_456,
            repeat_align_samples: 2,
        }
    }

    pub(crate) fn channels(&self) -> &channels::Description {
        &self.description
    }

    /// The number of logical channels in this profile's direction (MR-3, RM-29).
    pub(crate) fn channel_count(&self, direction: Direction) -> usize {
        match direction {
            Direction::Rx => self.description.rx.len(),
            Direction::Tx => self.description.tx.len(),
        }
    }

    /// A channel's LO behaviour and fixed path delay (RM-29, RM-23).
    pub(crate) fn channel_model_terms(&self, direction: Direction, channel: usize) -> (&str, i64) {
        let channel = match direction {
            Direction::Rx => &self.description.rx[channel],
            Direction::Tx => &self.description.tx[channel],
        };
        (&channel.phase_behavior_on_retune, channel.path_delay_samples)
    }

    /// RM-30's defaults of this profile's key set (MR-5).
    pub(crate) fn defaults(&self) -> &BTreeMap<Key, Value> {
        &self.defaults
    }

    /// RM-29's tree of this profile (MR-4).
    pub(crate) fn tree(&self, device: &ResourceId, memory_domain: MemoryDomainId) -> Resource {
        self.description.tree(device, memory_domain)
    }

    /// RM-5's, RM-7's and RM-8's coercion of this profile's key set (MR-6, RM-30).
    pub(crate) fn coerce(&self, device: &ResourceId, request: &Requested) -> Result<CoerceReport, ModuleError> {
        self.description.coerce(device, request)
    }

    /// The number of the instance's receive streams (RM-29, MR-10).
    pub(crate) fn rx_streams(&self) -> usize {
        self.description.rx.len()
    }

    /// The number of the instance's transmit streams (RM-29).
    pub(crate) fn tx_streams(&self) -> usize {
        self.description.tx.len()
    }

    /// A stream's node path below the device (RM-29).
    pub(crate) fn stream_path(&self, direction: Direction, k: usize) -> String {
        let side = if direction == Direction::Rx { "rx" } else { "tx" };
        format!("{side}/s{k}")
    }

    /// A receive stream's port name (RM-29).
    pub(crate) fn rx_port(&self, k: usize) -> String {
        format!("rx{k}")
    }

    pub(crate) fn profile_ref(&self) -> ProfileRef {
        self.description.profile.clone()
    }

    /// The decimation of `rate` on `x3xx` (MR-7); `ideal` has an arbitrary integer rate.
    pub(crate) fn decimation(&self, rate: f64) -> Option<u64> {
        match self.kind {
            ProfileKind::X3xx => x3xx::decimation(x3xx::MASTER_CLOCK_HZ, rate),
            ProfileKind::Ideal => None,
        }
    }

    pub(crate) fn repeat_max_samples(&self) -> u64 {
        self.description.repeat_max_samples
    }

    pub(crate) fn repeat_align_samples(&self) -> u64 {
        self.description.repeat_align_samples
    }

    pub(crate) fn block_len(&self) -> u32 {
        BLOCK_LEN
    }

    /// One receive call, in samples, for RM-25's ready instant after a cut: on `x3xx` the
    /// longer of `block_len` and the X300's packet of 1 996 samples (UHD's
    /// `max_num_samps` at MTU 9000), as the UHD Module's; none on `ideal`, ready at receipt
    /// (MR-3, MR-18; spec 22, VH-4).
    pub(crate) fn rx_call_samples(&self) -> i64 {
        match self.kind {
            ProfileKind::X3xx => i64::from(self.block_len()).max(1_996),
            ProfileKind::Ideal => 0,
        }
    }

    /// The delivery allowance after that call, in nanoseconds (MR-3).
    pub(crate) fn delivery_allowance_ns(&self) -> i64 {
        match self.kind {
            ProfileKind::X3xx => 3_000_000,
            ProfileKind::Ideal => 0,
        }
    }

    pub(crate) fn timing(&self) -> TimingEnvelope {
        self.description.timing
    }

    pub(crate) fn performance(&self) -> PerformanceEnvelope {
        self.description.performance
    }

    pub(crate) fn fidelity(&self) -> Fidelity {
        match self.kind {
            ProfileKind::X3xx => Fidelity {
                timing: EnvelopeFidelity::Envelope,
                continuity: EnvelopeFidelity::Envelope,
                coercion: CoercionFidelity::Grid,
                rf: RfFidelity::None,
                transport: TransportFidelity::None,
            },
            ProfileKind::Ideal => Fidelity::NONE,
        }
    }

    pub(crate) fn envelope(&self) -> RadioEnvelope {
        RadioEnvelope {
            profile: self.profile_ref(),
            timing: self.timing(),
            performance: self.performance(),
        }
    }
}
