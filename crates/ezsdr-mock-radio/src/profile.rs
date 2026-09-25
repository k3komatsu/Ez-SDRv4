//! The two resource and timing envelopes the Mock exposes (MR-3, MR-4).

use std::collections::BTreeMap;

use ezsdr_kernel::contract::{DataContractId, Port, PortDirection};
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::module_api::{
    CoercionFidelity, EnvelopeFidelity, Fidelity, ProfileRef, Resource, RfFidelity,
    TransportFidelity, Version,
};
use ezsdr_kernel::spec::{CapabilityValue, Ident, Key, Namespace, Value};
use ezsdr_radio::{PerformanceEnvelope, RadioEnvelope, TimingEnvelope};

use crate::coerce::Grid;

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
            version: Version::new(1, 1, 0),
        }
    }

    pub(crate) fn rate_grid(self) -> Grid {
        match self.kind {
            ProfileKind::X310Like => Grid::Values(
                (1..=512).rev().map(|n| 200_000_000.0 / f64::from(n)).collect(),
            ),
            ProfileKind::Ideal => Grid::Integer {
                lo: 1,
                hi: 1_000_000_000,
            },
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

    pub(crate) fn freq_grid(self) -> Grid {
        match self.kind {
            ProfileKind::X310Like => Grid::Step {
                lo: 10_000_000.0,
                hi: 6_000_000_000.0,
                step: 1.0,
            },
            ProfileKind::Ideal => Grid::Step {
                lo: 0.0,
                hi: 1_000_000_000_000.0,
                step: 0.0,
            },
        }
    }

    pub(crate) fn gain_grid(self) -> Grid {
        match self.kind {
            ProfileKind::X310Like => Grid::Step {
                lo: 0.0,
                hi: 31.5,
                step: 0.5,
            },
            ProfileKind::Ideal => Grid::Step {
                lo: -200.0,
                hi: 200.0,
                step: 0.0,
            },
        }
    }

    pub(crate) fn rx_antennas(self) -> &'static [&'static str] {
        &["RX2", "TX/RX"]
    }

    pub(crate) fn tx_antennas(self) -> &'static [&'static str] {
        &["TX/RX"]
    }

    pub(crate) fn phase_behavior(self) -> &'static str {
        match self.kind {
            ProfileKind::X310Like => "random_unless_timed_tune",
            ProfileKind::Ideal => "deterministic",
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
            },
            ProfileKind::Ideal => TimingEnvelope {
                min_timed_command_lead_ns: 0,
                startup_latency_ns: 0,
                stop_tail_ns: 0,
                command_queue_depth: i64::from(u32::MAX),
                overflow_restart_gap_ns: 0,
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

    pub(crate) fn tree(self, id: &ResourceId) -> Resource {
        use ezsdr_radio::keys::*;

        let mut capabilities = BTreeMap::new();
        let range = |min, max| CapabilityValue::Range { min, max };
        let one = |value| CapabilityValue::One { value };
        for direction in ["rx", "tx"] {
            let channel = key(&format!("radio.{direction}.channels"));
            capabilities.insert(
                channel,
                range(Value::Int(0), Value::Int(self.max_channels())),
            );
            let rates = key(&format!("radio.{direction}.sample_rate_hz"));
            let rate_cap = match self.kind {
                ProfileKind::X310Like => CapabilityValue::AnyOf {
                    values: (1..=512)
                        .map(|n| Value::Num(200_000_000.0 / f64::from(n)))
                        .collect(),
                },
                ProfileKind::Ideal => range(Value::Num(1.0), Value::Num(1_000_000_000.0)),
            };
            capabilities.insert(rates, rate_cap);
            let frequency = key(&format!("radio.{direction}.frequency_hz"));
            let frequency_grid = self.freq_grid();
            capabilities.insert(
                frequency,
                range(Value::Num(frequency_grid.min()), Value::Num(frequency_grid.max())),
            );
            let gain = key(&format!("radio.{direction}.gain_db"));
            let gain_grid = self.gain_grid();
            capabilities.insert(
                gain,
                range(Value::Num(gain_grid.min()), Value::Num(gain_grid.max())),
            );
            let frequency_step = key(&format!("radio.{direction}.frequency_step_hz"));
            capabilities.insert(frequency_step, one(Value::Num(frequency_grid.step())));
            let gain_step = key(&format!("radio.{direction}.gain_step_db"));
            capabilities.insert(gain_step, one(Value::Num(gain_grid.step())));
        }
        capabilities.insert(
            key(RX_ANTENNA),
            CapabilityValue::AnyOf {
                values: self.rx_antennas().iter().map(|s| Value::Str((*s).to_owned())).collect(),
            },
        );
        capabilities.insert(
            key(TX_ANTENNA),
            CapabilityValue::AnyOf {
                values: self.tx_antennas().iter().map(|s| Value::Str((*s).to_owned())).collect(),
            },
        );
        for (name, value) in [
            (RX_COHERENT, Value::Bool(true)),
            (FULL_DUPLEX, Value::Bool(true)),
            (HARDWARE_TIME, Value::Bool(true)),
            (PHASE_BEHAVIOR_ON_RETUNE, Value::Str(self.phase_behavior().to_owned())),
            (
                TX_REPEAT_MAX_SAMPLES,
                Value::Int(i64::try_from(self.repeat_max_samples()).expect("profile limit fits i64")),
            ),
            (TX_REPEAT_ALIGN_SAMPLES, Value::Int(self.repeat_align_samples() as i64)),
            (RX_BLOCK_LEN, Value::Int(i64::from(self.block_len()))),
            (
                MIN_TIMED_COMMAND_LEAD_NS,
                Value::Int(self.timing().min_timed_command_lead_ns),
            ),
            (STARTUP_LATENCY_NS, Value::Int(self.timing().startup_latency_ns)),
            (STOP_TAIL_NS, Value::Int(self.timing().stop_tail_ns)),
            (
                COMMAND_QUEUE_DEPTH,
                Value::Int(self.timing().command_queue_depth),
            ),
            (
                OVERFLOW_RESTART_GAP_NS,
                Value::Int(self.timing().overflow_restart_gap_ns),
            ),
            (RX_BYTES_PER_S, Value::Int(self.performance().rx_bytes_per_s)),
            (TX_BYTES_PER_S, Value::Int(self.performance().tx_bytes_per_s)),
            (
                WIRE_BYTES_PER_SAMPLE,
                Value::Int(self.performance().wire_bytes_per_sample),
            ),
            (TX_PATH_DELAY_SAMPLES, Value::Int(self.tx_path_delay_samples())),
            (RX_PATH_DELAY_SAMPLES, Value::Int(self.rx_path_delay_samples())),
        ] {
            capabilities.insert(key(name), one(value));
        }
        let rx_id = id.child("rx").expect("valid RX resource id");
        let tx_id = id.child("tx").expect("valid TX resource id");
        Resource {
            id: id.clone(),
            kind: Namespace::parse(DEVICE_KIND).expect("radio device kind"),
            capabilities,
            children: vec![
                Resource {
                    id: rx_id,
                    kind: Namespace::parse(RX_STREAM_KIND).expect("RX stream kind"),
                    capabilities: BTreeMap::new(),
                    children: Vec::new(),
                    ports: Vec::new(),
                    shareable: false,
                },
                Resource {
                    id: tx_id,
                    kind: Namespace::parse(TX_STREAM_KIND).expect("TX stream kind"),
                    capabilities: BTreeMap::new(),
                    children: Vec::new(),
                    ports: Vec::new(),
                    shareable: false,
                },
            ],
            ports: vec![Port {
                name: Ident::parse("rx").expect("rx port name"),
                direction: PortDirection::Out,
                contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32"),
            }],
            shareable: false,
        }
    }
}

fn key(name: &str) -> Key {
    Key::parse(name).expect("declared radio key")
}

const DEVICE_KIND: &str = ezsdr_radio::DEVICE_KIND;
const RX_STREAM_KIND: &str = ezsdr_radio::RX_STREAM_KIND;
const TX_STREAM_KIND: &str = ezsdr_radio::TX_STREAM_KIND;

impl Grid {
    pub(crate) fn step(&self) -> f64 {
        match self {
            Grid::Step { step, .. } => *step,
            Grid::Values(_) | Grid::Integer { .. } => 0.0,
        }
    }
}
