//! The `x3xx` profile of the UHD Module (UR-9) and the Module's own constants (§3). The
//! values are `ezsdr_radio::x3xx`'s, one place for both Providers (design 33 D5).

use ezsdr_kernel::module_api::ProfileRef;
use ezsdr_radio::device::DeviceDescription;
use ezsdr_radio::x3xx::{self, ProviderTerms, Selection};

/// The X310's master clock the profile needs (UR-5).
pub const MASTER_CLOCK_HZ: u64 = x3xx::MASTER_CLOCK_HZ;
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

/// The values only the UHD Module contributes to the `x3xx` description (UR-9, design 33
/// D5): its 5 ms command lead (the device lead plus the delivery allowance) and its
/// streamed-from-host repeat, to which neither Replay's size nor its alignment applies.
pub fn terms() -> ProviderTerms {
    ProviderTerms {
        min_timed_command_lead_ns: DEVICE_LEAD_NS + DELIVERY_ALLOWANCE_NS,
        repeat_max_samples: u64::from(u32::MAX),
        repeat_align_samples: 1,
    }
}

/// The profile this Module's binding must name (UR-9, design 33 D5).
pub fn profile_ref() -> ProfileRef {
    x3xx::profile_ref()
}

/// UR-9's values, from the binding's layout, as RM-26's description; `block_len` is the
/// selector's (UR-5).
pub fn description(selection: &Selection, block_len: u32) -> DeviceDescription {
    x3xx::description(selection, block_len, &terms())
}

/// The `x3xx` layout the Module's rigs use: a UBX in slot A and slot B, both directions
/// (tests).
#[cfg(test)]
pub(crate) fn ubx() -> Selection {
    Selection::of(x3xx::Board::Ubx, &[x3xx::Slot::A, x3xx::Slot::B], &[x3xx::Slot::A, x3xx::Slot::B])
}

/// [`ubx`]'s description with `block_len` (tests).
#[cfg(test)]
pub(crate) fn ubx_description(block_len: u32) -> DeviceDescription {
    description(&ubx(), block_len)
}
