//! The two resource and timing envelopes the Mock exposes (MR-3, MR-4).

use ezsdr_kernel::module_api::{
    CoercionFidelity, EnvelopeFidelity, Fidelity, ProfileRef, RfFidelity, TransportFidelity,
    Version,
};
use ezsdr_radio::device::{DeviceDescription, Grid};
use ezsdr_radio::{PerformanceEnvelope, RadioEnvelope, TimingEnvelope};

/// Which Phase 2 radio envelope the Mock presents (MR-3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileKind {
    /// The measured-envelope target for an X310 (MR-3).
    X310Like,
    /// A wide-range, no-latency test profile (MR-3).
    Ideal,
}

/// One profile and its emulated motherboard count (MR-3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Profile {
    /// The selected profile (MR-3).
    pub kind: ProfileKind,
    /// Number of emulated motherboards, from one through four (MR-2).
    pub n: u32,
}

impl Profile {
    pub(crate) fn profile_ref(self) -> ProfileRef {
        ProfileRef {
            name: match self.kind {
                ProfileKind::X310Like => "x310-like",
                ProfileKind::Ideal => "ideal",
            }
            .to_owned(),
            version: Version::new(1, 2, 0),
        }
    }

    pub(crate) fn decimation(self, rate: f64) -> Option<u64> {
        if self.kind != ProfileKind::X310Like || !rate.is_finite() || rate <= 0.0 {
            return None;
        }
        let n = (200_000_000.0 / rate).round();
        if !(1.0..=512.0).contains(&n) || 200_000_000.0 / n != rate {
            return None;
        }
        Some(n as u64)
    }

    pub(crate) fn max_channels(self) -> i64 {
        match self.kind {
            ProfileKind::X310Like => i64::from(2 * self.n),
            ProfileKind::Ideal => 64,
        }
    }

    pub(crate) fn repeat_max_samples(self) -> u64 {
        match self.kind {
            ProfileKind::X310Like => 268_435_456,
            ProfileKind::Ideal => u64::from(u32::MAX),
        }
    }

    pub(crate) fn repeat_align_samples(self) -> u64 {
        match self.kind {
            ProfileKind::X310Like => 2,
            ProfileKind::Ideal => 1,
        }
    }

    pub(crate) fn block_len(self) -> u32 {
        2_000
    }

    /// The transmit path delay in transmit samples: srsRAN's X300 time advance for
    /// `x310-like` (INFERRED), none for `ideal` (MR-3, RM-23).
    pub(crate) fn tx_path_delay_samples(self) -> i64 {
        match self.kind {
            ProfileKind::X310Like => 45,
            ProfileKind::Ideal => 0,
        }
    }

    /// The receive path delay in receive samples: 0, because `x310-like` carries the
    /// whole loopback delay on its transmit side (MR-3, RM-23).
    pub(crate) fn rx_path_delay_samples(self) -> i64 {
        0
    }

    /// Whether an untimed tune leaves each channel's LO at a random phase (MR-34).
    pub(crate) fn random_phase_on_untimed_tune(self) -> bool {
        self.kind == ProfileKind::X310Like
    }

    pub(crate) fn timing(self) -> TimingEnvelope {
        match self.kind {
            ProfileKind::X310Like => TimingEnvelope {
                min_timed_command_lead_ns: 2_000_000,
                startup_latency_ns: 2_000_000_000,
                stop_tail_ns: 1_000_000,
                command_queue_depth: 16,
                overflow_restart_gap_ns: 50_000_000,
                // INFERRED, as spec 18 §3's UHD Module waits them; Phase 8 measures both
                // (spec 20, VF-6).
                restart_lead_ns: 50_000_000,
                start_lead_ns: 50_000_000,
            },
            ProfileKind::Ideal => TimingEnvelope {
                min_timed_command_lead_ns: 0,
                startup_latency_ns: 0,
                stop_tail_ns: 0,
                command_queue_depth: i64::from(u32::MAX),
                overflow_restart_gap_ns: 0,
                restart_lead_ns: 0,
                start_lead_ns: 0,
            },
        }
    }

    pub(crate) fn performance(self) -> PerformanceEnvelope {
        match self.kind {
            ProfileKind::X310Like => PerformanceEnvelope {
                rx_bytes_per_s: 1_000_000_000 * i64::from(self.n),
                tx_bytes_per_s: 1_000_000_000 * i64::from(self.n),
                wire_bytes_per_sample: 4,
            },
            ProfileKind::Ideal => PerformanceEnvelope {
                rx_bytes_per_s: 1_i64 << 62,
                tx_bytes_per_s: 1_i64 << 62,
                wire_bytes_per_sample: 8,
            },
        }
    }

    pub(crate) fn fidelity(self) -> Fidelity {
        match self.kind {
            ProfileKind::X310Like => Fidelity {
                timing: EnvelopeFidelity::Envelope,
                continuity: EnvelopeFidelity::Envelope,
                coercion: CoercionFidelity::Grid,
                rf: RfFidelity::None,
                transport: TransportFidelity::None,
            },
            ProfileKind::Ideal => Fidelity::NONE,
        }
    }

    pub(crate) fn envelope(self) -> RadioEnvelope {
        RadioEnvelope {
            profile: self.profile_ref(),
            timing: self.timing(),
            performance: self.performance(),
        }
    }

    /// MR-3's values as RM-26's description, whose `tree` and `coerce` are MR-4's and
    /// MR-6's (Phase 7, VE-1, VE-4).
    pub(crate) fn description(self) -> DeviceDescription {
        let (rates, whole_hertz_rates, frequency, gain, phase) = match self.kind {
            ProfileKind::X310Like => (
                Grid::Values((1..=512).rev().map(|n| 200_000_000.0 / f64::from(n)).collect()),
                false,
                Grid::Step { lo: 10_000_000.0, hi: 6_000_000_000.0, step: 1.0 },
                Grid::Step { lo: 0.0, hi: 31.5, step: 0.5 },
                "random_unless_timed_tune",
            ),
            ProfileKind::Ideal => (
                Grid::Integer { lo: 1, hi: 1_000_000_000 },
                true,
                Grid::Step { lo: 0.0, hi: 1_000_000_000_000.0, step: 0.0 },
                Grid::Step { lo: -200.0, hi: 200.0, step: 0.0 },
                "deterministic",
            ),
        };
        DeviceDescription {
            profile: self.profile_ref(),
            rates,
            whole_hertz_rates,
            frequency,
            gain,
            max_channels: self.max_channels(),
            rx_antennas: vec!["RX2".to_owned(), "TX/RX".to_owned()],
            tx_antennas: vec!["TX/RX".to_owned()],
            coherent: true,
            full_duplex: true,
            hardware_time: true,
            phase_behavior_on_retune: phase.to_owned(),
            repeat_max_samples: self.repeat_max_samples(),
            repeat_align_samples: self.repeat_align_samples(),
            block_len: self.block_len(),
            tx_path_delay_samples: self.tx_path_delay_samples(),
            rx_path_delay_samples: self.rx_path_delay_samples(),
            timing: self.timing(),
            performance: self.performance(),
            defaults: DeviceDescription::x310_defaults(),
        }
    }
}
