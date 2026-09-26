//! Ez-SDR v4 host memory domain and buffer pool (plan/phase2/10-host-data-path.md).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

use ezsdr_kernel::id::MemoryDomainId;
use ezsdr_kernel::stream::SampleBlock;

/// The host-accessible memory domain used by Phase 2 Modules (HD-1).
pub const HOST_MEMORY: MemoryDomainId = MemoryDomainId::local(0);

/// Bytes in one `ezsdr.stream.cf32` complex sample (HD-3).
pub const CF32_BYTES: usize = 8;

/// Reuses host byte buffers after every block reference has been released (HD-2).
pub struct HostPool {
    slot_bytes: usize,
    slots: Vec<Arc<[u8]>>,
}

impl HostPool {
    /// Creates a pool whose reusable slots hold `slot_bytes` bytes (HD-2).
    pub fn new(slot_bytes: usize) -> HostPool {
        HostPool {
            slot_bytes,
            slots: Vec::new(),
        }
    }

    /// Fills and returns a shared buffer, reusing only an unreferenced slot (HD-2).
    pub fn fill(&mut self, len: usize, write: impl FnOnce(&mut [u8])) -> Arc<[u8]> {
        if len > self.slot_bytes {
            let mut bytes = vec![0; len];
            write(&mut bytes);
            return bytes.into();
        }

        for slot in &mut self.slots {
            if let Some(bytes) = Arc::get_mut(slot) {
                write(&mut bytes[..len]);
                return Arc::clone(slot);
            }
        }

        let mut bytes = vec![0; self.slot_bytes];
        write(&mut bytes[..len]);
        let slot: Arc<[u8]> = bytes.into();
        self.slots.push(Arc::clone(&slot));
        slot
    }

    /// Returns the number of reusable slots allocated so far (HD-2).
    pub fn slots(&self) -> usize {
        self.slots.len()
    }
}

/// Writes one planar `cf32` sample in little-endian real/imaginary order (HD-3).
pub fn write_cf32(
    buf: &mut [u8],
    len: usize,
    channel: usize,
    index: usize,
    re: f32,
    im: f32,
) {
    let offset = (channel * len + index) * CF32_BYTES;
    buf[offset..offset + 4].copy_from_slice(&re.to_le_bytes());
    buf[offset + 4..offset + CF32_BYTES].copy_from_slice(&im.to_le_bytes());
}

/// Reads one planar `cf32` sample in little-endian real/imaginary order (HD-3).
pub fn read_cf32(buf: &[u8], len: usize, channel: usize, index: usize) -> (f32, f32) {
    let offset = (channel * len + index) * CF32_BYTES;
    let re = f32::from_le_bytes(buf[offset..offset + 4].try_into().expect("four real bytes"));
    let im = f32::from_le_bytes(
        buf[offset + 4..offset + CF32_BYTES]
            .try_into()
            .expect("four imaginary bytes"),
    );
    (re, im)
}

/// Copies a sample range from planar host bytes into channel-interleaved order (HD-3).
pub fn interleave(
    block: &SampleBlock,
    bytes_per_sample: usize,
    from: usize,
    to: usize,
) -> Vec<u8> {
    let header = block.header();
    let Some(bytes) = block.host_bytes() else {
        return Vec::new();
    };
    let len = header.len as usize;
    if from > to || to > len {
        return Vec::new();
    }

    let mut interleaved = Vec::new();
    for sample in from..to {
        for channel in 0..header.channels as usize {
            // `channels` is a `u16` and `len` a `u32`, so for any bytes-per-sample a
            // DataContract names the offset fits a 64-bit `usize` and cannot overflow.
            // A buffer short for *this* contract — SC-10a checks the producer's own
            // `bytes_per_sample`, which is not stored on the block — fails the slice
            // read, and the caller turns the empty result into a clean refusal.
            let offset = (channel * len + sample) * bytes_per_sample;
            let Some(sample_bytes) = bytes.get(offset..offset + bytes_per_sample) else {
                return Vec::new();
            };
            interleaved.extend_from_slice(sample_bytes);
        }
    }
    interleaved
}
