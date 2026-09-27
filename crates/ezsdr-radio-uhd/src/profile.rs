//! The profile `x310-ubx` 0.1.0 (UR-9) and the Module's own constants (§3).

use ezsdr_kernel::module_api::{ProfileRef, Version};
use ezsdr_radio::device::{DeviceDescription, Grid};
use ezsdr_radio::{PerformanceEnvelope, TimingEnvelope};

/// The X310's master clock the profile needs (UR-5).
pub const MASTER_CLOCK_HZ: u64 = 200_000_000;
/// The lead the device needs from receipt (§3, INFERRED; B8).
pub const DEVICE_LEAD_NS: i64 = 2_000_000;
/// The delivery allowance: the poll period plus booking the Actions ahead (UR-14).
pub const DELIVERY_ALLOWANCE_NS: i64 = 3_000_000;
/// The least spacing of a stream stop and its restart (UR-25, INFERRED; B8).
pub const RESTART_LEAD_NS: i64 = 50_000_000;
/// The most samples handed to the device ahead of its time (UR-23, INFERRED; B8).
pub const IN_FLIGHT_WINDOW_NS: i64 = 10_000_000;
/// How far ahead a held timed command is released to the device (UR-24).
pub const RELEASE_WINDOW_NS: i64 = 3_000_000;
/// The default samples per receive block (UR-5, UR-9).
pub const DEFAULT_BLOCK_LEN: u32 = 2_000;

/// `x310-ubx` 0.1.0.
pub fn profile_ref() -> ProfileRef {
    ProfileRef {
        name: "x310-ubx".to_owned(),
        version: Version::new(0, 1, 0),
    }
}

/// UR-9's values as RM-26's description; `block_len` is the selector's.
pub fn description(block_len: u32) -> DeviceDescription {
    DeviceDescription {
        profile: profile_ref(),
        rates: Grid::Values((1..=512).rev().map(|n| MASTER_CLOCK_HZ as f64 / f64::from(n)).collect()),
        whole_hertz_rates: false,
        frequency: Grid::Step { lo: 10_000_000.0, hi: 6_000_000_000.0, step: 1.0 },
        gain: Grid::Step { lo: 0.0, hi: 31.5, step: 0.5 },
        max_channels: 2,
        rx_antennas: vec!["RX2".to_owned(), "TX/RX".to_owned()],
        tx_antennas: vec!["TX/RX".to_owned()],
        coherent: true,
        full_duplex: true,
        hardware_time: true,
        phase_behavior_on_retune: "random_unless_timed_tune".to_owned(),
        // Streamed from host memory (UR-22): neither Replay's size nor its alignment
        // applies; the limit is `BurstOpen.waveform_len`'s u32.
        repeat_max_samples: u64::from(u32::MAX),
        repeat_align_samples: 1,
        block_len,
        tx_path_delay_samples: 45,
        rx_path_delay_samples: 0,
        timing: TimingEnvelope {
            min_timed_command_lead_ns: DEVICE_LEAD_NS + DELIVERY_ALLOWANCE_NS,
            startup_latency_ns: 2_000_000_000,
            stop_tail_ns: 1_000_000,
            command_queue_depth: 16,
            overflow_restart_gap_ns: 50_000_000,
        },
        performance: PerformanceEnvelope {
            rx_bytes_per_s: 1_000_000_000,
            tx_bytes_per_s: 1_000_000_000,
            wire_bytes_per_sample: 4,
        },
        defaults: DeviceDescription::x310_defaults(),
    }
}
