//! The `x3xx` and `ideal` profiles the Mock presents (MR-3, design 33 D5), each in its
//! two key sets: the current description (RM-26) and the layout builder's qualification
//! of mixed boards (RM-27…RM-32). The `x3xx` values are `ezsdr_radio::x3xx`'s; only the
//! Mock's own values live here.

use std::collections::BTreeMap;

use ezsdr_kernel::id::{MemoryDomainId, ResourceId};
use ezsdr_kernel::module_api::{
    CoerceReport, CoercionFidelity, EnvelopeFidelity, Fidelity, ModuleError, ProfileRef,
    Requested, Resource, RfFidelity, TransportFidelity, Version,
};
use ezsdr_kernel::spec::{Key, Value};
use ezsdr_kernel::stream::Direction;
use ezsdr_radio::channels;
use ezsdr_radio::device::{DeviceDescription, Grid};
use ezsdr_radio::x3xx::{self, ProviderTerms, Selection};
use ezsdr_radio::{PerformanceEnvelope, RadioEnvelope, TimingEnvelope};

/// The samples in one receive block, both profiles and both key sets (MR-3).
pub(crate) const BLOCK_LEN: u32 = 2_000;

/// Which profile the Mock presents (MR-3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileKind {
    /// The measured-envelope target for an X3xx (MR-3, design 33 D5).
    X3xx,
    /// A wide-range, no-latency test profile (MR-3).
    Ideal,
}

/// The description one profile presents (RM-26, RM-29): its current keys, or the layout
/// builder's channel and stream keys. A Provider instance uses one of the two, never
/// both (RM-27).
#[derive(Clone, PartialEq, Debug)]
enum Mode {
    /// RM-26's `DeviceDescription`: one stream per direction, direction-prefixed keys.
    Current(Box<DeviceDescription>),
    /// The layout builder's description (RM-27…RM-32): one channel and stream key per
    /// logical channel of the binding's layout.
    Layout(Box<channels::Description>),
}

/// One profile (MR-3), in one of its two key sets (RM-27).
#[derive(Clone, PartialEq, Debug)]
pub struct Profile {
    /// The selected profile (MR-3).
    pub kind: ProfileKind,
    /// The layout the selector declared, for `x3xx` (design 33 D5).
    pub selection: Option<Selection>,
    /// The description this profile presents, built once when the binding is read
    /// (RM-26, design 33 D5); every query reads it rather than rebuilding it.
    mode: Mode,
    /// RM-5's or RM-30's defaults, the `effective` an unconstrained key takes (MR-5).
    defaults: BTreeMap<Key, Value>,
}

impl Profile {
    /// The profile of a binding: its description built once, from the layout on `x3xx`
    /// (RM-26, design 33 D5). `layout` is the builder the selector named, when it named
    /// one: the description is then RM-29's, of numbered channel and stream keys.
    pub(crate) fn new(kind: ProfileKind, selection: Option<Selection>, layout: Option<channels::Layout>) -> Profile {
        let mode = match layout {
            Some(layout) => Mode::Layout(Box::new(channels::Description::new(&layout, BLOCK_LEN, &Profile::terms()))),
            None => Mode::Current(Box::new(match kind {
                ProfileKind::X3xx => x3xx::description(
                    selection.as_ref().expect("an x3xx layout"),
                    BLOCK_LEN,
                    &Profile::terms(),
                ),
                ProfileKind::Ideal => DeviceDescription {
                    profile: ProfileRef { name: "ideal".to_owned(), version: Version::new(2, 0, 0) },
                    rates: Grid::Integer { lo: 1, hi: 1_000_000_000 },
                    whole_hertz_rates: true,
                    frequency: Grid::Step { lo: 0.0, hi: 1_000_000_000_000.0, step: 0.0 },
                    gain: Grid::Step { lo: -200.0, hi: 200.0, step: 0.0 },
                    rx_max_channels: 64,
                    tx_max_channels: 64,
                    rx_antennas: vec!["RX2".to_owned(), "TX/RX".to_owned()],
                    tx_antennas: vec!["TX/RX".to_owned()],
                    coherent: true,
                    full_duplex: true,
                    hardware_time: true,
                    phase_behavior_on_retune: "deterministic".to_owned(),
                    repeat_max_samples: u64::from(u32::MAX),
                    repeat_align_samples: 1,
                    block_len: BLOCK_LEN,
                    tx_path_delay_samples: 0,
                    rx_path_delay_samples: 0,
                    timing: TimingEnvelope {
                        min_timed_command_lead_ns: 0,
                        startup_latency_ns: 0,
                        command_queue_depth: i64::from(u32::MAX),
                        overflow_restart_gap_ns: 0,
                        rx_start_lead_ns: 0,
                        tx_start_lead_ns: 0,
                    },
                    performance: PerformanceEnvelope {
                        rx_bytes_per_s: 1_i64 << 62,
                        tx_bytes_per_s: 1_i64 << 62,
                        wire_bytes_per_sample: 8,
                    },
                    defaults: DeviceDescription::x310_defaults(),
                },
            })),
        };
        let defaults = match &mode {
            Mode::Current(description) => description.defaults.clone(),
            Mode::Layout(description) => description.defaults(),
        };
        Profile { kind, selection, mode, defaults }
    }

    /// The values only the Mock contributes to the `x3xx` description (design 33 D5): its
    /// 2 ms lead (INFERRED, measured by Phase 8) and its Replay-like repeat constraints.
    pub(crate) fn terms() -> ProviderTerms {
        ProviderTerms {
            min_timed_command_lead_ns: 2_000_000,
            repeat_max_samples: 268_435_456,
            repeat_align_samples: 2,
        }
    }

    /// Whether this profile presents the layout builder's keys (RM-27).
    pub(crate) fn is_layout(&self) -> bool {
        matches!(self.mode, Mode::Layout(_))
    }

    /// The layout builder's description (RM-29), when this profile presents its keys.
    pub(crate) fn channels(&self) -> Option<&channels::Description> {
        match &self.mode {
            Mode::Layout(description) => Some(description),
            Mode::Current(_) => None,
        }
    }

    /// The current keys' description (RM-26). Only the paths a layout instance never takes
    /// read it: a layout instance builds no SimulationChannel model (MR-41), and every
    /// caller is behind `channel.is_some()`.
    pub(crate) fn description(&self) -> &DeviceDescription {
        match &self.mode {
            Mode::Current(description) => description,
            Mode::Layout(_) => panic!("MR-27: a layout instance has no current description"),
        }
    }

    /// RM-5's or RM-30's defaults of this profile's key set (MR-5).
    pub(crate) fn defaults(&self) -> &BTreeMap<Key, Value> {
        &self.defaults
    }

    /// RM-2's or RM-29's tree of this profile (MR-4).
    pub(crate) fn tree(&self, device: &ResourceId, memory_domain: MemoryDomainId) -> Resource {
        match &self.mode {
            Mode::Current(description) => description.tree(device, memory_domain),
            Mode::Layout(description) => description.tree(device, memory_domain),
        }
    }

    /// RM-5's, RM-7's and RM-8's coercion of this profile's key set (MR-6, RM-30).
    pub(crate) fn coerce(&self, device: &ResourceId, request: &Requested) -> Result<CoerceReport, ModuleError> {
        match &self.mode {
            Mode::Current(description) => description.coerce(device, request),
            Mode::Layout(description) => description.coerce(device, request),
        }
    }

    /// The number of the instance's receive streams: one per logical receive channel of a
    /// layout description, and one under the current keys (RM-29, MR-10).
    pub(crate) fn rx_streams(&self) -> usize {
        match &self.mode {
            Mode::Current(_) => 1,
            Mode::Layout(description) => description.rx.len(),
        }
    }

    /// The number of the instance's transmit streams (RM-29).
    pub(crate) fn tx_streams(&self) -> usize {
        match &self.mode {
            Mode::Current(_) => 1,
            Mode::Layout(description) => description.tx.len(),
        }
    }

    /// A stream's node path below the device: `rx` and `tx` under the current keys
    /// (MR-10), `rx/s<k>` and `tx/s<k>` under a layout description (RM-29).
    pub(crate) fn stream_path(&self, direction: Direction, k: usize) -> String {
        let side = if direction == Direction::Rx { "rx" } else { "tx" };
        if self.is_layout() {
            format!("{side}/s{k}")
        } else {
            side.to_owned()
        }
    }

    /// A receive stream's port name: `rx` under the current keys (MR-7), `rx<k>` under a
    /// layout description (RM-29).
    pub(crate) fn rx_port(&self, k: usize) -> String {
        if self.is_layout() {
            format!("rx{k}")
        } else {
            "rx".to_owned()
        }
    }

    pub(crate) fn profile_ref(&self) -> ProfileRef {
        match &self.mode {
            Mode::Current(description) => description.profile.clone(),
            Mode::Layout(description) => description.profile.clone(),
        }
    }

    /// The decimation of `rate` on `x3xx` (MR-7); `ideal` has an arbitrary integer rate.
    pub(crate) fn decimation(&self, rate: f64) -> Option<u64> {
        match self.kind {
            ProfileKind::X3xx => x3xx::decimation(x3xx::MASTER_CLOCK_HZ, rate),
            ProfileKind::Ideal => None,
        }
    }

    pub(crate) fn repeat_max_samples(&self) -> u64 {
        match &self.mode {
            Mode::Current(description) => description.repeat_max_samples,
            Mode::Layout(description) => description.repeat_max_samples,
        }
    }

    pub(crate) fn repeat_align_samples(&self) -> u64 {
        match &self.mode {
            Mode::Current(description) => description.repeat_align_samples,
            Mode::Layout(description) => description.repeat_align_samples,
        }
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

    /// Whether the initial untimed tune leaves each channel's LO at a random phase, from
    /// the description's `radio.phase_behavior_on_retune` (MR-34). A layout description
    /// states the phase behaviour per channel (RM-29); its SimulationChannel model is not
    /// built at this stage, so nothing draws from this.
    pub(crate) fn random_phase_on_untimed_tune(&self) -> bool {
        match &self.mode {
            Mode::Current(description) => description.phase_behavior_on_retune != "deterministic",
            Mode::Layout(_) => false,
        }
    }

    /// Whether a timed tune replaces each channel's phase with its timed-tune constant
    /// (MR-34): false for the `random` phase value, whose every tune redraws the phase
    /// instead (design 33 D13). As [`Profile::random_phase_on_untimed_tune`].
    pub(crate) fn timed_tune_fixes_phase(&self) -> bool {
        match &self.mode {
            Mode::Current(description) => description.phase_behavior_on_retune != "random",
            Mode::Layout(_) => false,
        }
    }

    pub(crate) fn timing(&self) -> TimingEnvelope {
        match &self.mode {
            Mode::Current(description) => description.timing,
            Mode::Layout(description) => description.timing,
        }
    }

    pub(crate) fn performance(&self) -> PerformanceEnvelope {
        match &self.mode {
            Mode::Current(description) => description.performance,
            Mode::Layout(description) => description.performance,
        }
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
