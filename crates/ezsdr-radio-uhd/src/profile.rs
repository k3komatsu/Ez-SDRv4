//! The profiles `x310-ubx`, `x310-obx` and `x310-cbx` 0.2.0 (UR-9) and the Module's own
//! constants (§3).

use ezsdr_kernel::module_api::{ProfileRef, Version};
use ezsdr_kernel::spec::{Key, Value};
use ezsdr_radio::device::{DeviceDescription, Grid};
use ezsdr_radio::{PerformanceEnvelope, TimingEnvelope, keys};

/// The X310's master clock the profile needs (UR-5).
pub const MASTER_CLOCK_HZ: u64 = 200_000_000;
/// The lead the device needs from receipt (§3, INFERRED; B8).
pub const DEVICE_LEAD_NS: i64 = 2_000_000;
/// The delivery allowance: the poll period plus booking the Actions ahead (UR-14).
pub const DELIVERY_ALLOWANCE_NS: i64 = 3_000_000;
/// The most samples handed to the device ahead of its time (UR-23, INFERRED; B8).
pub const IN_FLIGHT_WINDOW_NS: i64 = 10_000_000;
/// How far ahead a held timed command is released to the device (UR-24).
pub const RELEASE_WINDOW_NS: i64 = 3_000_000;
/// The default samples per receive block (UR-5, UR-9).
pub const DEFAULT_BLOCK_LEN: u32 = 2_000;
/// `x310-cbx`'s default frequency, both directions (UR-9).
pub const CBX_DEFAULT_FREQUENCY_HZ: f64 = 2_450_000_000.0;

/// A profile of this Module: one X310 and the front ends it carries (UR-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// `x310-ubx` 0.2.0: two UBX.
    X310Ubx,
    /// `x310-obx` 0.2.0: one OBX, in slot A (the bench's).
    X310Obx,
    /// `x310-cbx` 0.2.0: one CBX, in slot A.
    X310Cbx,
}

impl Profile {
    /// The profile a binding names, if it is one of this Module's.
    pub fn from_ref(profile: &ProfileRef) -> Option<Profile> {
        [Profile::X310Ubx, Profile::X310Obx, Profile::X310Cbx].into_iter().find(|p| p.profile_ref() == *profile)
    }

    /// The profile's name.
    pub fn name(self) -> &'static str {
        match self {
            Profile::X310Ubx => "x310-ubx",
            Profile::X310Obx => "x310-obx",
            Profile::X310Cbx => "x310-cbx",
        }
    }

    /// The name at version 0.2.0.
    pub fn profile_ref(self) -> ProfileRef {
        ProfileRef {
            name: self.name().to_owned(),
            version: Version::new(0, 2, 0),
        }
    }

    /// How UHD's names of the profile's front ends begin (`UBX RX`, `OBX TX`, `CBX-120 TX`;
    /// UR-5).
    pub fn front_end(self) -> &'static str {
        match self {
            Profile::X310Ubx => "UBX",
            Profile::X310Obx => "OBX",
            Profile::X310Cbx => "CBX",
        }
    }

    /// UR-9's values as RM-26's description; `block_len` is the selector's.
    pub fn description(self, block_len: u32) -> DeviceDescription {
        match self {
            Profile::X310Ubx => x310(self, 10_000_000.0, 6_000_000_000.0, 2, block_len),
            // UHD 4.10's `obx_freq_range(10e6, 8.4e9)` (`db_obx.hpp`, VERIFIED).
            Profile::X310Obx => x310(self, 10_000_000.0, 8_400_000_000.0, 1, block_len),
            // UHD 4.10 clips a CBX tune to 1.2…6 GHz (`db_sbx_common.hpp`, VERIFIED), and
            // RM-5's 1 GHz is outside it.
            // ponytail: CBX has no timed LO phase sync (UHD's `_sync_phase` is UBX's and
            // OBX's only), so `random_unless_timed_tune` overstates it; `radio` has no value
            // for "random" yet, a Vocabulary addition left to Phase 8 (design-notes §9).
            Profile::X310Cbx => {
                let mut description = x310(self, 1_200_000_000.0, 6_000_000_000.0, 1, block_len);
                for key in [keys::RX_FREQUENCY_HZ, keys::TX_FREQUENCY_HZ] {
                    description.defaults.insert(Key::parse(key).expect("a radio key"), Value::Num(CBX_DEFAULT_FREQUENCY_HZ));
                }
                description
            }
        }
    }
}

/// The values the profiles share; they differ in the frequency range, the channel count
/// and `x310-cbx`'s default frequency.
fn x310(profile: Profile, lowest_hz: f64, highest_hz: f64, max_channels: i64, block_len: u32) -> DeviceDescription {
    DeviceDescription {
        profile: profile.profile_ref(),
        rates: Grid::Values((1..=512).rev().map(|n| MASTER_CLOCK_HZ as f64 / f64::from(n)).collect()),
        whole_hertz_rates: false,
        frequency: Grid::Step { lo: lowest_hz, hi: highest_hz, step: 1.0 },
        gain: Grid::Step { lo: 0.0, hi: 31.5, step: 0.5 },
        max_channels,
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
            // §3's restart lead and start lead, INFERRED: the bench's raw restarts began on
            // the requested tick at leads down to 1 ms in 19 of 21 rows (B8), so 50 ms is a
            // margin whose values Phase 8 measures (spec 20, VF-6).
            restart_lead_ns: 50_000_000,
            start_lead_ns: 50_000_000,
        },
        performance: PerformanceEnvelope {
            rx_bytes_per_s: 1_000_000_000,
            tx_bytes_per_s: 1_000_000_000,
            wire_bytes_per_sample: 4,
        },
        defaults: DeviceDescription::x310_defaults(),
    }
}
