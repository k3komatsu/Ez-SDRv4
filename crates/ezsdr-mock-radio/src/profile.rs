//! The `x3xx` and `ideal` profiles the Mock presents (MR-3, design 33 D5). The `x3xx`
//! values are `ezsdr_radio::x3xx`'s; only the Mock's own values live here.

use ezsdr_kernel::module_api::{
    CoercionFidelity, EnvelopeFidelity, Fidelity, ProfileRef, RfFidelity, TransportFidelity,
    Version,
};
use ezsdr_radio::device::{DeviceDescription, Grid};
use ezsdr_radio::x3xx::{self, ProviderTerms, Selection};
use ezsdr_radio::{PerformanceEnvelope, TimingEnvelope};

/// The samples in one receive block, both profiles (MR-3).
pub(crate) const BLOCK_LEN: u32 = 2_000;

/// Which profile the Mock presents (MR-3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileKind {
    /// The measured-envelope target for an X3xx (MR-3, design 33 D5).
    X3xx,
    /// A wide-range, no-latency test profile (MR-3).
    Ideal,
}

/// One profile (MR-3).
#[derive(Clone, PartialEq, Debug)]
pub struct Profile {
    /// The selected profile (MR-3).
    pub kind: ProfileKind,
    /// The layout the selector declared, for `x3xx`; none for `ideal` (design 33 D5).
    pub selection: Option<Selection>,
    /// The description this profile presents, built once when the binding is read (RM-26,
    /// design 33 D5); every query reads it rather than rebuilding it.
    description: DeviceDescription,
}

impl Profile {
    /// The profile of a binding: its description built once, from the layout on `x3xx`
    /// (RM-26, design 33 D5).
    pub(crate) fn new(kind: ProfileKind, selection: Option<Selection>) -> Profile {
        let description = match kind {
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
        };
        Profile { kind, selection, description }
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

    /// Whether the initial untimed tune leaves each channel's LO at a random phase, from
    /// the profile's `radio.phase_behavior_on_retune` (MR-34).
    pub(crate) fn random_phase_on_untimed_tune(&self) -> bool {
        self.description.phase_behavior_on_retune != "deterministic"
    }

    /// Whether a timed tune replaces each channel's phase with its timed-tune constant
    /// (MR-34): false for the `random` phase value, whose every tune redraws the phase
    /// instead (design 33 D13).
    pub(crate) fn timed_tune_fixes_phase(&self) -> bool {
        self.description.phase_behavior_on_retune != "random"
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

    pub(crate) fn envelope(&self) -> ezsdr_radio::RadioEnvelope {
        ezsdr_radio::RadioEnvelope {
            profile: self.profile_ref(),
            timing: self.timing(),
            performance: self.performance(),
        }
    }

    /// MR-3's values as RM-26's description, whose `tree` and `coerce` are MR-4's and
    /// MR-6's. The `x3xx` values are `ezsdr_radio::x3xx`'s, one place for both Providers
    /// (design 33 D5).
    pub(crate) fn description(&self) -> &DeviceDescription {
        &self.description
    }
}
