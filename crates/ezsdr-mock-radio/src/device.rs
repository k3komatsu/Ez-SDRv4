//! Independent transmit framing check used by the Mock (MR-24).

use ezsdr_kernel::stream::{BlockFlags, BlockHeader};

/// A device-side TX framing violation (MR-24).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceTimeError;

/// Minimal model of the device's open-burst state, independent of BurstTracker.
#[derive(Default)]
pub struct DeviceModel {
    open: bool,
}

impl DeviceModel {
    /// Creates a closed device model (MR-24).
    pub fn new() -> DeviceModel {
        DeviceModel::default()
    }

    /// Checks a transmit block and updates the device's framing state (MR-24).
    pub fn on_tx_block(&mut self, header: &BlockHeader) -> Result<(), DeviceTimeError> {
        if header.flags.contains(BlockFlags::START_OF_BURST) {
            if self.open {
                return Err(DeviceTimeError);
            }
            self.open = true;
        }
        if header.flags.contains(BlockFlags::END_OF_BURST) {
            self.open = false;
        }
        Ok(())
    }

    /// Closes the currently open burst, as a zero-length device EOB would (MR-24).
    pub fn close(&mut self) {
        self.open = false;
    }
}
