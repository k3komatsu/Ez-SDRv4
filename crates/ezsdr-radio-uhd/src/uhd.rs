//! `UhdDevice`: [`Device`] on UHD's own C API (UR-3), the crate's only `unsafe` code
//! (GZ-3). Every C entry point is `UHD_SAFE_C`-wrapped by UHD, so a C++ exception
//! arrives here as a `uhd_error` and its text from `uhd_get_last_error`.
//!
//! Threads: every control call is made under one mutex (the command-time bracket is
//! per-motherboard state), and each streamer has one owning thread at a time, which
//! holds it through an `Arc` so that `close_streams` frees only one nobody holds.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ezsdr_kernel::module_api::{
    CoercionFidelity, EnvelopeFidelity, Fidelity, RfFidelity, TransportFidelity,
};

use crate::device::{
    Applied, Device, DeviceError, Dir, Iq, RxRecv, Settings, TxCode, TxReport, from_time_spec,
    to_time_spec,
};

#[allow(non_camel_case_types, dead_code)]
mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    pub type uhd_error = c_int;
    pub const UHD_ERROR_NONE: uhd_error = 0;
    pub const UHD_ERROR_USB: uhd_error = 21;
    pub const UHD_ERROR_IO: uhd_error = 30;
    pub const UHD_ERROR_OS: uhd_error = 31;
    pub const UHD_ERROR_RUNTIME: uhd_error = 44;

    pub type uhd_usrp_handle = *mut c_void;
    pub type uhd_rx_streamer_handle = *mut c_void;
    pub type uhd_tx_streamer_handle = *mut c_void;
    pub type uhd_rx_metadata_handle = *mut c_void;
    pub type uhd_tx_metadata_handle = *mut c_void;
    pub type uhd_async_metadata_handle = *mut c_void;
    pub type uhd_string_vector_handle = *mut c_void;
    pub type uhd_sensor_value_handle = *mut c_void;
    pub type uhd_subdev_spec_handle = *mut c_void;
    pub type uhd_meta_range_handle = *mut c_void;

    /// `uhd/usrp/usrp.h`: 40 bytes on LP64.
    #[repr(C)]
    pub struct uhd_stream_args_t {
        pub cpu_format: *mut c_char,
        pub otw_format: *mut c_char,
        pub args: *mut c_char,
        pub channel_list: *mut usize,
        pub n_channels: c_int,
    }

    /// `uhd/types/ranges.h`: 24 bytes.
    #[repr(C)]
    #[derive(Default)]
    pub struct uhd_range_t {
        pub start: f64,
        pub stop: f64,
        pub step: f64,
    }

    /// `uhd/types/usrp_info.h`: 64 bytes on LP64.
    #[repr(C)]
    pub struct uhd_usrp_rx_info_t {
        pub mboard_id: *mut c_char,
        pub mboard_name: *mut c_char,
        pub mboard_serial: *mut c_char,
        pub rx_id: *mut c_char,
        pub rx_subdev_name: *mut c_char,
        pub rx_subdev_spec: *mut c_char,
        pub rx_serial: *mut c_char,
        pub rx_antenna: *mut c_char,
    }

    pub const UHD_STREAM_MODE_START_CONTINUOUS: c_int = 97;
    pub const UHD_STREAM_MODE_STOP_CONTINUOUS: c_int = 111;

    /// `uhd/usrp/usrp.h`: 40 bytes on LP64.
    #[repr(C)]
    pub struct uhd_stream_cmd_t {
        pub stream_mode: c_int,
        pub num_samps: usize,
        pub stream_now: bool,
        pub time_spec_full_secs: i64,
        pub time_spec_frac_secs: f64,
    }

    pub const UHD_TUNE_REQUEST_POLICY_NONE: c_int = 78;
    pub const UHD_TUNE_REQUEST_POLICY_AUTO: c_int = 65;
    pub const UHD_TUNE_REQUEST_POLICY_MANUAL: c_int = 77;

    /// `uhd/types/tune_request.h`: 48 bytes on LP64.
    #[repr(C)]
    pub struct uhd_tune_request_t {
        pub target_freq: f64,
        pub rf_freq_policy: c_int,
        pub rf_freq: f64,
        pub dsp_freq_policy: c_int,
        pub dsp_freq: f64,
        pub args: *mut c_char,
    }

    /// `uhd/types/tune_result.h`: 40 bytes.
    #[repr(C)]
    #[derive(Default)]
    pub struct uhd_tune_result_t {
        pub clipped_rf_freq: f64,
        pub target_rf_freq: f64,
        pub actual_rf_freq: f64,
        pub target_dsp_freq: f64,
        pub actual_dsp_freq: f64,
    }

    // `uhd/types/metadata.h:80-92`.
    pub const UHD_RX_METADATA_ERROR_CODE_NONE: c_int = 0x0;
    pub const UHD_RX_METADATA_ERROR_CODE_TIMEOUT: c_int = 0x1;
    pub const UHD_RX_METADATA_ERROR_CODE_LATE_COMMAND: c_int = 0x2;
    pub const UHD_RX_METADATA_ERROR_CODE_BROKEN_CHAIN: c_int = 0x4;
    pub const UHD_RX_METADATA_ERROR_CODE_OVERFLOW: c_int = 0x8;
    pub const UHD_RX_METADATA_ERROR_CODE_ALIGNMENT: c_int = 0xC;
    pub const UHD_RX_METADATA_ERROR_CODE_BAD_PACKET: c_int = 0xF;
    // `uhd/types/metadata.h:232-244`.
    pub const UHD_ASYNC_METADATA_EVENT_CODE_BURST_ACK: c_int = 0x1;
    pub const UHD_ASYNC_METADATA_EVENT_CODE_UNDERFLOW: c_int = 0x2;
    pub const UHD_ASYNC_METADATA_EVENT_CODE_SEQ_ERROR: c_int = 0x4;
    pub const UHD_ASYNC_METADATA_EVENT_CODE_TIME_ERROR: c_int = 0x8;
    pub const UHD_ASYNC_METADATA_EVENT_CODE_UNDERFLOW_IN_PACKET: c_int = 0x10;
    pub const UHD_ASYNC_METADATA_EVENT_CODE_SEQ_ERROR_IN_BURST: c_int = 0x20;

    unsafe extern "C" {
        pub fn uhd_get_last_error(error_out: *mut c_char, strbuffer_len: usize) -> uhd_error;
        pub fn uhd_get_version_string(version_out: *mut c_char, buffer_len: usize) -> uhd_error;
        pub fn uhd_set_thread_priority(priority: f32, realtime: bool) -> uhd_error;

        pub fn uhd_string_vector_make(h: *mut uhd_string_vector_handle) -> uhd_error;
        pub fn uhd_string_vector_free(h: *mut uhd_string_vector_handle) -> uhd_error;
        pub fn uhd_string_vector_size(h: uhd_string_vector_handle, size_out: *mut usize) -> uhd_error;
        pub fn uhd_string_vector_at(h: uhd_string_vector_handle, index: usize, value_out: *mut c_char, strbuffer_len: usize) -> uhd_error;

        pub fn uhd_usrp_find(args: *const c_char, strings_out: *mut uhd_string_vector_handle) -> uhd_error;
        pub fn uhd_usrp_make(h: *mut uhd_usrp_handle, args: *const c_char) -> uhd_error;
        pub fn uhd_usrp_free(h: *mut uhd_usrp_handle) -> uhd_error;
        pub fn uhd_usrp_last_error(h: uhd_usrp_handle, error_out: *mut c_char, strbuffer_len: usize) -> uhd_error;
        pub fn uhd_usrp_get_pp_string(h: uhd_usrp_handle, pp_string_out: *mut c_char, strbuffer_len: usize) -> uhd_error;
        pub fn uhd_usrp_get_master_clock_rate(h: uhd_usrp_handle, mboard: usize, clock_rate_out: *mut f64) -> uhd_error;
        pub fn uhd_usrp_get_time_now(h: uhd_usrp_handle, mboard: usize, full_secs_out: *mut i64, frac_secs_out: *mut f64) -> uhd_error;
        pub fn uhd_usrp_set_time_now(h: uhd_usrp_handle, full_secs: i64, frac_secs: f64, mboard: usize) -> uhd_error;
        pub fn uhd_usrp_set_time_unknown_pps(h: uhd_usrp_handle, full_secs: i64, frac_secs: f64) -> uhd_error;
        pub fn uhd_usrp_set_command_time(h: uhd_usrp_handle, full_secs: i64, frac_secs: f64, mboard: usize) -> uhd_error;
        pub fn uhd_usrp_clear_command_time(h: uhd_usrp_handle, mboard: usize) -> uhd_error;
        pub fn uhd_usrp_set_clock_source(h: uhd_usrp_handle, clock_source: *const c_char, mboard: usize) -> uhd_error;
        pub fn uhd_usrp_set_time_source(h: uhd_usrp_handle, time_source: *const c_char, mboard: usize) -> uhd_error;
        pub fn uhd_usrp_get_mboard_sensor(h: uhd_usrp_handle, name: *const c_char, mboard: usize, sensor_value_out: *mut uhd_sensor_value_handle) -> uhd_error;
        pub fn uhd_usrp_get_mboard_sensor_names(h: uhd_usrp_handle, mboard: usize, mboard_sensor_names_out: *mut uhd_string_vector_handle) -> uhd_error;

        pub fn uhd_sensor_value_make(h: *mut uhd_sensor_value_handle) -> uhd_error;
        pub fn uhd_sensor_value_free(h: *mut uhd_sensor_value_handle) -> uhd_error;
        pub fn uhd_sensor_value_to_bool(h: uhd_sensor_value_handle, value_out: *mut bool) -> uhd_error;

        pub fn uhd_subdev_spec_make(h: *mut uhd_subdev_spec_handle, markup: *const c_char) -> uhd_error;
        pub fn uhd_subdev_spec_free(h: *mut uhd_subdev_spec_handle) -> uhd_error;

        pub fn uhd_meta_range_make(h: *mut uhd_meta_range_handle) -> uhd_error;
        pub fn uhd_meta_range_free(h: *mut uhd_meta_range_handle) -> uhd_error;
        pub fn uhd_meta_range_size(h: uhd_meta_range_handle, size_out: *mut usize) -> uhd_error;
        pub fn uhd_meta_range_at(h: uhd_meta_range_handle, num: usize, range_out: *mut uhd_range_t) -> uhd_error;

        pub fn uhd_usrp_get_rx_info(h: uhd_usrp_handle, chan: usize, info_out: *mut uhd_usrp_rx_info_t) -> uhd_error;
        pub fn uhd_usrp_rx_info_free(info: *mut uhd_usrp_rx_info_t) -> uhd_error;
        pub fn uhd_usrp_set_rx_subdev_spec(h: uhd_usrp_handle, subdev_spec: uhd_subdev_spec_handle, mboard: usize) -> uhd_error;
        pub fn uhd_usrp_set_tx_subdev_spec(h: uhd_usrp_handle, subdev_spec: uhd_subdev_spec_handle, mboard: usize) -> uhd_error;

        pub fn uhd_usrp_get_rx_num_channels(h: uhd_usrp_handle, num_channels_out: *mut usize) -> uhd_error;
        pub fn uhd_usrp_get_rx_subdev_name(h: uhd_usrp_handle, chan: usize, rx_subdev_name_out: *mut c_char, strbuffer_len: usize) -> uhd_error;
        pub fn uhd_usrp_get_rx_rates(h: uhd_usrp_handle, chan: usize, rates_out: uhd_meta_range_handle) -> uhd_error;
        pub fn uhd_usrp_set_rx_rate(h: uhd_usrp_handle, rate: f64, chan: usize) -> uhd_error;
        pub fn uhd_usrp_get_rx_rate(h: uhd_usrp_handle, chan: usize, rate_out: *mut f64) -> uhd_error;
        pub fn uhd_usrp_set_rx_freq(h: uhd_usrp_handle, tune_request: *mut uhd_tune_request_t, chan: usize, tune_result: *mut uhd_tune_result_t) -> uhd_error;
        pub fn uhd_usrp_get_rx_freq(h: uhd_usrp_handle, chan: usize, freq_out: *mut f64) -> uhd_error;
        pub fn uhd_usrp_set_rx_gain(h: uhd_usrp_handle, gain: f64, chan: usize, gain_name: *const c_char) -> uhd_error;
        pub fn uhd_usrp_get_rx_gain(h: uhd_usrp_handle, chan: usize, gain_name: *const c_char, gain_out: *mut f64) -> uhd_error;
        pub fn uhd_usrp_set_rx_antenna(h: uhd_usrp_handle, ant: *const c_char, chan: usize) -> uhd_error;
        pub fn uhd_usrp_get_rx_antenna(h: uhd_usrp_handle, chan: usize, ant_out: *mut c_char, strbuffer_len: usize) -> uhd_error;

        pub fn uhd_usrp_get_tx_num_channels(h: uhd_usrp_handle, num_channels_out: *mut usize) -> uhd_error;
        pub fn uhd_usrp_get_tx_subdev_name(h: uhd_usrp_handle, chan: usize, tx_subdev_name_out: *mut c_char, strbuffer_len: usize) -> uhd_error;
        pub fn uhd_usrp_get_tx_rates(h: uhd_usrp_handle, chan: usize, rates_out: uhd_meta_range_handle) -> uhd_error;
        pub fn uhd_usrp_set_tx_rate(h: uhd_usrp_handle, rate: f64, chan: usize) -> uhd_error;
        pub fn uhd_usrp_get_tx_rate(h: uhd_usrp_handle, chan: usize, rate_out: *mut f64) -> uhd_error;
        pub fn uhd_usrp_set_tx_freq(h: uhd_usrp_handle, tune_request: *mut uhd_tune_request_t, chan: usize, tune_result: *mut uhd_tune_result_t) -> uhd_error;
        pub fn uhd_usrp_get_tx_freq(h: uhd_usrp_handle, chan: usize, freq_out: *mut f64) -> uhd_error;
        pub fn uhd_usrp_set_tx_gain(h: uhd_usrp_handle, gain: f64, chan: usize, gain_name: *const c_char) -> uhd_error;
        pub fn uhd_usrp_get_tx_gain(h: uhd_usrp_handle, chan: usize, gain_name: *const c_char, gain_out: *mut f64) -> uhd_error;
        pub fn uhd_usrp_set_tx_antenna(h: uhd_usrp_handle, ant: *const c_char, chan: usize) -> uhd_error;
        pub fn uhd_usrp_get_tx_antenna(h: uhd_usrp_handle, chan: usize, ant_out: *mut c_char, strbuffer_len: usize) -> uhd_error;

        pub fn uhd_rx_streamer_make(h: *mut uhd_rx_streamer_handle) -> uhd_error;
        pub fn uhd_rx_streamer_free(h: *mut uhd_rx_streamer_handle) -> uhd_error;
        pub fn uhd_usrp_get_rx_stream(h: uhd_usrp_handle, stream_args: *mut uhd_stream_args_t, h_out: uhd_rx_streamer_handle) -> uhd_error;
        pub fn uhd_rx_streamer_max_num_samps(h: uhd_rx_streamer_handle, max_num_samps_out: *mut usize) -> uhd_error;
        pub fn uhd_rx_streamer_recv(h: uhd_rx_streamer_handle, buffs: *mut *mut c_void, samps_per_buff: usize, md: *mut uhd_rx_metadata_handle, timeout: f64, one_packet: bool, items_recvd: *mut usize) -> uhd_error;
        pub fn uhd_rx_streamer_issue_stream_cmd(h: uhd_rx_streamer_handle, stream_cmd: *const uhd_stream_cmd_t) -> uhd_error;
        pub fn uhd_rx_streamer_last_error(h: uhd_rx_streamer_handle, error_out: *mut c_char, strbuffer_len: usize) -> uhd_error;

        pub fn uhd_rx_metadata_make(handle: *mut uhd_rx_metadata_handle) -> uhd_error;
        pub fn uhd_rx_metadata_free(handle: *mut uhd_rx_metadata_handle) -> uhd_error;
        pub fn uhd_rx_metadata_has_time_spec(h: uhd_rx_metadata_handle, result_out: *mut bool) -> uhd_error;
        pub fn uhd_rx_metadata_time_spec(h: uhd_rx_metadata_handle, full_secs_out: *mut i64, frac_secs_out: *mut f64) -> uhd_error;
        pub fn uhd_rx_metadata_error_code(h: uhd_rx_metadata_handle, error_code_out: *mut c_int) -> uhd_error;
        pub fn uhd_rx_metadata_out_of_sequence(h: uhd_rx_metadata_handle, result_out: *mut bool) -> uhd_error;
        pub fn uhd_rx_metadata_strerror(h: uhd_rx_metadata_handle, strerror_out: *mut c_char, strbuffer_len: usize) -> uhd_error;

        pub fn uhd_tx_streamer_make(h: *mut uhd_tx_streamer_handle) -> uhd_error;
        pub fn uhd_tx_streamer_free(h: *mut uhd_tx_streamer_handle) -> uhd_error;
        pub fn uhd_usrp_get_tx_stream(h: uhd_usrp_handle, stream_args: *mut uhd_stream_args_t, h_out: uhd_tx_streamer_handle) -> uhd_error;
        pub fn uhd_tx_streamer_max_num_samps(h: uhd_tx_streamer_handle, max_num_samps_out: *mut usize) -> uhd_error;
        pub fn uhd_tx_streamer_send(h: uhd_tx_streamer_handle, buffs: *mut *const c_void, samps_per_buff: usize, md: *mut uhd_tx_metadata_handle, timeout: f64, items_sent: *mut usize) -> uhd_error;
        pub fn uhd_tx_streamer_recv_async_msg(h: uhd_tx_streamer_handle, md: *mut uhd_async_metadata_handle, timeout: f64, valid: *mut bool) -> uhd_error;
        pub fn uhd_tx_streamer_last_error(h: uhd_tx_streamer_handle, error_out: *mut c_char, strbuffer_len: usize) -> uhd_error;

        pub fn uhd_tx_metadata_make(handle: *mut uhd_tx_metadata_handle, has_time_spec: bool, full_secs: i64, frac_secs: f64, start_of_burst: bool, end_of_burst: bool) -> uhd_error;
        pub fn uhd_tx_metadata_free(handle: *mut uhd_tx_metadata_handle) -> uhd_error;

        pub fn uhd_async_metadata_make(handle: *mut uhd_async_metadata_handle) -> uhd_error;
        pub fn uhd_async_metadata_free(handle: *mut uhd_async_metadata_handle) -> uhd_error;
        pub fn uhd_async_metadata_channel(h: uhd_async_metadata_handle, channel_out: *mut usize) -> uhd_error;
        pub fn uhd_async_metadata_has_time_spec(h: uhd_async_metadata_handle, result_out: *mut bool) -> uhd_error;
        pub fn uhd_async_metadata_time_spec(h: uhd_async_metadata_handle, full_secs_out: *mut i64, frac_secs_out: *mut f64) -> uhd_error;
        pub fn uhd_async_metadata_event_code(h: uhd_async_metadata_handle, event_code_out: *mut c_int) -> uhd_error;
    }
}

use ffi::*;

/// The structs' sizes as this build lays them out (UR-3's `size_of` check).
pub fn struct_sizes() -> [(&'static str, usize); 6] {
    [
        ("uhd_stream_args_t", std::mem::size_of::<uhd_stream_args_t>()),
        ("uhd_stream_cmd_t", std::mem::size_of::<uhd_stream_cmd_t>()),
        ("uhd_tune_request_t", std::mem::size_of::<uhd_tune_request_t>()),
        ("uhd_tune_result_t", std::mem::size_of::<uhd_tune_result_t>()),
        ("uhd_range_t", std::mem::size_of::<uhd_range_t>()),
        ("uhd_usrp_rx_info_t", std::mem::size_of::<uhd_usrp_rx_info_t>()),
    ]
}

/// An error text read into a buffer by one of UHD's `*_last_error` calls.
fn text(read: impl FnOnce(*mut c_char, usize) -> uhd_error) -> String {
    let mut buffer = [0 as c_char; 1024];
    read(buffer.as_mut_ptr(), buffer.len() - 1);
    // SAFETY: UHD's wrappers `strncpy` at most the length passed, one short of the
    // zeroed buffer, so its last byte stays a NUL (Review of 09f11b4, P2-4).
    unsafe { CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned() }
}

/// A `uhd_error` as a [`DeviceError`] whose text `message` reads; `streaming` says
/// whether a `RUNTIME` error counts as a lost device (UR-29, INFERRED).
fn check_with(call: &str, code: uhd_error, streaming: bool, message: impl FnOnce() -> String) -> Result<(), DeviceError> {
    if code == UHD_ERROR_NONE {
        return Ok(());
    }
    let lost = matches!(code, UHD_ERROR_IO | UHD_ERROR_USB | UHD_ERROR_OS) || (streaming && code == UHD_ERROR_RUNTIME);
    Err(DeviceError { lost, message: format!("{call}: UHD error {code}: {}", message()) })
}

/// A call with no handle of its own: UHD's process-global text, which another
/// thread's call may already have overwritten (Review L, P1-2), so only for calls
/// made before the Module's threads run or on objects no other thread touches.
fn check(call: &str, code: uhd_error, streaming: bool) -> Result<(), DeviceError> {
    // SAFETY: the buffer is writable for the length passed.
    check_with(call, code, streaming, || text(|b, n| unsafe { uhd_get_last_error(b, n) }))
}

/// A call on a receive streamer, with that streamer's own error text.
fn check_rx(h: uhd_rx_streamer_handle, call: &str, code: uhd_error) -> Result<(), DeviceError> {
    // SAFETY: `h` is a live streamer; the buffer is writable for the length passed.
    check_with(call, code, true, || text(|b, n| unsafe { uhd_rx_streamer_last_error(h, b, n) }))
}

/// A call on a transmit streamer, with that streamer's own error text.
fn check_tx(h: uhd_tx_streamer_handle, call: &str, code: uhd_error) -> Result<(), DeviceError> {
    // SAFETY: as for `check_rx`.
    check_with(call, code, true, || text(|b, n| unsafe { uhd_tx_streamer_last_error(h, b, n) }))
}

fn cstring(s: &str) -> Result<CString, DeviceError> {
    CString::new(s).map_err(|_| DeviceError::failed(format!("UR-3: {s:?} holds a NUL")))
}

fn strings(h: uhd_string_vector_handle) -> Vec<String> {
    let mut n = 0usize;
    let mut out = Vec::new();
    // SAFETY: `h` is a live string vector; each read writes into a sized buffer.
    unsafe {
        uhd_string_vector_size(h, &mut n);
        for i in 0..n {
            let mut buffer = [0 as c_char; 1024];
            uhd_string_vector_at(h, i, buffer.as_mut_ptr(), buffer.len() - 1);
            out.push(CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned());
        }
    }
    out
}

/// Lists the devices UHD finds for `args` (`uhd_usrp_find`).
pub fn find(args: &str) -> Result<Vec<String>, DeviceError> {
    let args = cstring(args)?;
    let mut vector: uhd_string_vector_handle = std::ptr::null_mut();
    // SAFETY: the vector is made, filled and freed here; `args` outlives the call.
    unsafe {
        check("uhd_string_vector_make", uhd_string_vector_make(&mut vector), false)?;
        let found = check("uhd_usrp_find", uhd_usrp_find(args.as_ptr(), &mut vector), false).map(|()| strings(vector));
        uhd_string_vector_free(&mut vector);
        found
    }
}

struct RxStream {
    h: uhd_rx_streamer_handle,
    call: Mutex<()>,
    md: uhd_rx_metadata_handle,
    /// The device's lost mark: a lost device's streamer is not freed (F4).
    lost: Arc<AtomicBool>,
    channels: usize,
    /// Samples per packet (`uhd_rx_streamer_max_num_samps`).
    spp: usize,
}

struct TxStream {
    h: uhd_tx_streamer_handle,
    call: Mutex<()>,
    md: uhd_async_metadata_handle,
    /// As for `RxStream`.
    lost: Arc<AtomicBool>,
    channels: usize,
}

impl Drop for RxStream {
    fn drop(&mut self) {
        // A lost device's handles are leaked: UHD's teardown writes to the device and,
        // over a dead link, throws out of a destructor (design-notes §17, F4).
        if self.lost.load(Ordering::Acquire) {
            return;
        }
        // SAFETY: the last `Arc` holder frees the handles it made, once.
        unsafe {
            uhd_rx_metadata_free(&mut self.md);
            uhd_rx_streamer_free(&mut self.h);
        }
    }
}

impl Drop for TxStream {
    fn drop(&mut self) {
        if self.lost.load(Ordering::Acquire) {
            return;
        }
        // SAFETY: as for `RxStream`.
        unsafe {
            uhd_async_metadata_free(&mut self.md);
            uhd_tx_streamer_free(&mut self.h);
        }
    }
}

impl RxStream {
    /// Makes a receive streamer and its metadata, `attach` binding it to a device;
    /// the value owns both handles from the first, so an early return frees them
    /// once, and the caller moves it into its `Arc` (Review L, P0-1).
    fn make(channels: usize, lost: Arc<AtomicBool>, attach: impl FnOnce(uhd_rx_streamer_handle) -> Result<usize, DeviceError>) -> Result<RxStream, DeviceError> {
        let mut h: uhd_rx_streamer_handle = std::ptr::null_mut();
        // SAFETY: `h` is written by the call.
        unsafe { check("uhd_rx_streamer_make", uhd_rx_streamer_make(&mut h), true)? };
        let mut stream = RxStream { h, call: Mutex::new(()), md: std::ptr::null_mut(), lost, channels, spp: 0 };
        stream.spp = attach(stream.h)?;
        // SAFETY: the metadata handle is written into the value that frees it.
        unsafe { check("uhd_rx_metadata_make", uhd_rx_metadata_make(&mut stream.md), true)? };
        Ok(stream)
    }
}

impl TxStream {
    /// As [`RxStream::make`], for transmit.
    fn make(channels: usize, lost: Arc<AtomicBool>, attach: impl FnOnce(uhd_tx_streamer_handle) -> Result<(), DeviceError>) -> Result<TxStream, DeviceError> {
        let mut h: uhd_tx_streamer_handle = std::ptr::null_mut();
        // SAFETY: `h` is written by the call.
        unsafe { check("uhd_tx_streamer_make", uhd_tx_streamer_make(&mut h), true)? };
        let mut stream = TxStream { h, call: Mutex::new(()), md: std::ptr::null_mut(), lost, channels };
        attach(stream.h)?;
        // SAFETY: as above.
        unsafe { check("uhd_async_metadata_make", uhd_async_metadata_make(&mut stream.md), true)? };
        Ok(stream)
    }
}

/// Makes and drops one unattached streamer per direction, through the same path as
/// `rx_open` and `tx_open`, so that each handle's make/free path runs: UHD's
/// `uhd_{rx,tx}_streamer_free` stores NULL in the slot it is given (`usrp_c.cpp:97-101`,
/// `:153-157`), while freeing a copy of a handle twice aborts (Review L, P0-1).
pub fn streamer_lifecycle() -> Result<(), DeviceError> {
    let lost = Arc::new(AtomicBool::new(false));
    let rx = Arc::new(RxStream::make(1, lost.clone(), |_| Ok(0))?);
    let tx = Arc::new(TxStream::make(1, lost, |_| Ok(()))?);
    drop((rx.clone(), tx.clone()));
    drop((rx, tx));
    Ok(())
}

/// The subdev spec `set_subdev_spec` sets, made from `markup` (UR-3). UHD 4.10's
/// `uhd_subdev_spec_make` allocates the handle before it parses the markup
/// (`subdev_spec_c.cpp:31-36`), so a refused markup leaves an allocated handle: that handle
/// is freed here and only the error is returned. The caller frees the handle on `Ok`.
fn made_subdev_spec(markup: &CStr) -> Result<uhd_subdev_spec_handle, DeviceError> {
    let mut spec: uhd_subdev_spec_handle = std::ptr::null_mut();
    // SAFETY: `spec` is written by the call, and a handle the failed make allocated is
    // freed once here.
    let made = unsafe { check("uhd_subdev_spec_make", uhd_subdev_spec_make(&mut spec, markup.as_ptr()), false) };
    if made.is_err() && !spec.is_null() {
        // SAFETY: the handle is freed once, and the make's own error is the one returned.
        unsafe { uhd_subdev_spec_free(&mut spec) };
    }
    made.map(|()| spec)
}

/// The subdev spec's own lifetime, without a device: a refused markup returns UHD's error
/// with the handle it allocated before parsing already freed (`made_subdev_spec`), and an
/// accepted one is freed after it is made. UHD's `uhd_subdev_spec_free` stores NULL in the
/// handle's slot (`subdev_spec_c.cpp:41`), so a second free through that slot deletes
/// nothing; the count each path frees is pinned by the replaced calls in `ffi_tests`. Called
/// by `tests/uhd_api.rs` (feature `uhd`).
pub fn subdev_spec_lifecycle(markup: &str) -> Result<(), DeviceError> {
    let markup = cstring(markup)?;
    let mut spec = made_subdev_spec(&markup)?;
    // SAFETY: the handle `made_subdev_spec` handed over is freed exactly once.
    unsafe { check("uhd_subdev_spec_free", uhd_subdev_spec_free(&mut spec), false) }
}

// SAFETY (UR-3): every published streamer's native calls, metadata and error-text
// access are serialized by its call mutex; the Arc keeps its handles alive.
// Private construction and final drop have exclusive access to those handles.
unsafe impl Send for RxStream {}
unsafe impl Sync for RxStream {}
unsafe impl Send for TxStream {}
unsafe impl Sync for TxStream {}

/// A USRP opened with `uhd_usrp_make` (UR-3, UR-4).
pub struct UhdDevice {
    usrp: uhd_usrp_handle,
    args: String,
    mcr: u64,
    control: Mutex<()>,
    /// The open receive and transmit streamers, by stream number (design 33 D5: several
    /// per direction).
    rx: Mutex<BTreeMap<usize, Arc<RxStream>>>,
    tx: Mutex<BTreeMap<usize, Arc<TxStream>>>,
    /// Set by `mark_lost`: the device and its streamers are then never freed (F4).
    lost: Arc<AtomicBool>,
}

// SAFETY (UR-3, INFERRED from UHD's documented thread safety of multi_usrp control
// calls): every control call is made under `control`; streamer access is locked as above.
unsafe impl Send for UhdDevice {}
unsafe impl Sync for UhdDevice {}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl UhdDevice {
    /// A call on the device handle, with the handle's own error text; made under the
    /// control mutex, so the text is this call's (Review L, P1-2).
    fn check_usrp(&self, call: &str, code: uhd_error, streaming: bool) -> Result<(), DeviceError> {
        let usrp = self.usrp;
        // SAFETY: the device handle lives as long as `self`; the buffer is writable.
        check_with(call, code, streaming, || text(|b, n| unsafe { uhd_usrp_last_error(usrp, b, n) }))
    }

    /// Opens the device `args` names; refuses one whose master clock is not a positive
    /// whole number of hertz (UR-3).
    pub fn open(args: &str) -> Result<UhdDevice, String> {
        reclaim(args);
        let c_args = cstring(args).map_err(|e| e.message)?;
        let mut usrp: uhd_usrp_handle = std::ptr::null_mut();
        // SAFETY: `usrp` is written by the call; `c_args` outlives it.
        unsafe { check("uhd_usrp_make", uhd_usrp_make(&mut usrp, c_args.as_ptr()), false) }.map_err(|e| e.message)?;
        let mut mcr = 0.0;
        // SAFETY: `usrp` was made above.
        let rate = unsafe { check("uhd_usrp_get_master_clock_rate", uhd_usrp_get_master_clock_rate(usrp, 0, &mut mcr), false) };
        if rate.is_err() || mcr <= 0.0 || mcr.fract() != 0.0 {
            // SAFETY: freed once, here.
            unsafe { uhd_usrp_free(&mut usrp) };
            return Err(format!("UR-3: the master clock rate {mcr} is not a positive whole number of hertz"));
        }
        Ok(UhdDevice {
            usrp,
            args: args.to_owned(),
            mcr: mcr as u64,
            control: Mutex::new(()),
            rx: Mutex::new(BTreeMap::new()),
            tx: Mutex::new(BTreeMap::new()),
            lost: Arc::new(AtomicBool::new(false)),
        })
        .inspect(|_| lock(&LIVE).push(args.to_owned()))
    }

    fn stream_args(channels: &mut [usize], fc32: &CString, sc16: &CString, empty: &CString) -> uhd_stream_args_t {
        uhd_stream_args_t {
            cpu_format: fc32.as_ptr() as *mut c_char,
            otw_format: sc16.as_ptr() as *mut c_char,
            args: empty.as_ptr() as *mut c_char,
            channel_list: channels.as_mut_ptr(),
            n_channels: channels.len() as c_int,
        }
    }

    /// Design note 39's bench probes only: sets the DDC's (receive) or DUC's (transmit) shift
    /// on `chan` to `dsp_hz`, leaving the LO alone (`rf_freq_policy` NONE), at `at` or now;
    /// returns the shift UHD reports.
    pub fn tune_dsp(&self, dir: Dir, chan: usize, dsp_hz: f64, at: Option<i64>) -> Result<f64, DeviceError> {
        let empty = cstring("")?;
        let _control = lock(&self.control);
        // SAFETY: every pointer passed outlives its call; the command time is set and
        // cleared under the control mutex, as `apply` does.
        unsafe {
            if let Some(at) = at {
                let (full, frac) = to_time_spec(at, self.mcr);
                self.check_usrp("uhd_usrp_set_command_time", uhd_usrp_set_command_time(self.usrp, full, frac, 0), false)?;
            }
            let mut request = uhd_tune_request_t {
                target_freq: 0.0,
                rf_freq_policy: UHD_TUNE_REQUEST_POLICY_NONE,
                rf_freq: 0.0,
                dsp_freq_policy: UHD_TUNE_REQUEST_POLICY_MANUAL,
                dsp_freq: dsp_hz,
                args: empty.as_ptr() as *mut c_char,
            };
            let mut result = uhd_tune_result_t::default();
            let tuned = match dir {
                Dir::Rx => self.check_usrp("uhd_usrp_set_rx_freq", uhd_usrp_set_rx_freq(self.usrp, &mut request, chan, &mut result), false),
                Dir::Tx => self.check_usrp("uhd_usrp_set_tx_freq", uhd_usrp_set_tx_freq(self.usrp, &mut request, chan, &mut result), false),
            };
            let cleared = if at.is_some() {
                self.check_usrp("uhd_usrp_clear_command_time", uhd_usrp_clear_command_time(self.usrp, 0), false)
            } else {
                Ok(())
            };
            tuned?;
            cleared?;
            Ok(result.actual_dsp_freq)
        }
    }

    /// Design note 42's bench probes only: tunes `chan` to `target_hz` as `apply` does — AUTO
    /// for the LO and for the DSP, at `at` or now —, and returns what UHD reports it chose,
    /// `(actual_rf_freq, actual_dsp_freq)`, so a probe knows whether the DSP value changed.
    pub fn tune_auto(&self, dir: Dir, chan: usize, target_hz: f64, at: Option<i64>) -> Result<(f64, f64), DeviceError> {
        let empty = cstring("")?;
        let _control = lock(&self.control);
        // SAFETY: every pointer passed outlives its call; the command time is set and
        // cleared under the control mutex, as `apply` does.
        unsafe {
            if let Some(at) = at {
                let (full, frac) = to_time_spec(at, self.mcr);
                self.check_usrp("uhd_usrp_set_command_time", uhd_usrp_set_command_time(self.usrp, full, frac, 0), false)?;
            }
            let mut request = uhd_tune_request_t {
                target_freq: target_hz,
                rf_freq_policy: UHD_TUNE_REQUEST_POLICY_AUTO,
                rf_freq: 0.0,
                dsp_freq_policy: UHD_TUNE_REQUEST_POLICY_AUTO,
                dsp_freq: 0.0,
                args: empty.as_ptr() as *mut c_char,
            };
            let mut result = uhd_tune_result_t::default();
            let tuned = match dir {
                Dir::Rx => self.check_usrp("uhd_usrp_set_rx_freq", uhd_usrp_set_rx_freq(self.usrp, &mut request, chan, &mut result), false),
                Dir::Tx => self.check_usrp("uhd_usrp_set_tx_freq", uhd_usrp_set_tx_freq(self.usrp, &mut request, chan, &mut result), false),
            };
            let cleared = if at.is_some() {
                self.check_usrp("uhd_usrp_clear_command_time", uhd_usrp_clear_command_time(self.usrp, 0), false)
            } else {
                Ok(())
            };
            tuned?;
            cleared?;
            Ok((result.actual_rf_freq, result.actual_dsp_freq))
        }
    }

    fn stream_cmd(&self, stream: usize, start: bool, at: Option<i64>) -> Result<(), DeviceError> {
        let Some(stream) = lock(&self.rx).get(&stream).cloned() else {
            return Err(DeviceError::failed("UR-3: the receive streamer is not open"));
        };
        let (full, frac) = at.map_or((0, 0.0), |t| to_time_spec(t, self.mcr));
        let cmd = uhd_stream_cmd_t {
            stream_mode: if start { UHD_STREAM_MODE_START_CONTINUOUS } else { UHD_STREAM_MODE_STOP_CONTINUOUS },
            num_samps: 0,
            stream_now: at.is_none(),
            time_spec_full_secs: full,
            time_spec_frac_secs: frac,
        };
        let _control = lock(&self.control);
        let _call = lock(&stream.call);
        // SAFETY: the streamer is held by the Arc and its call lock.
        unsafe { check_rx(stream.h, "uhd_rx_streamer_issue_stream_cmd", uhd_rx_streamer_issue_stream_cmd(stream.h, &cmd)) }
    }
}

/// A device kept rather than freed (F4), with its streamers, until a later open of the
/// same `args` finds it answering again (Review S, NB-S2).
struct Kept {
    args: String,
    usrp: uhd_usrp_handle,
    lost: Arc<AtomicBool>,
    streams: (BTreeMap<usize, Arc<RxStream>>, BTreeMap<usize, Arc<TxStream>>),
}

// SAFETY: a kept handle is touched only under `KEPT`'s lock, by one thread at a time.
unsafe impl Send for Kept {}

static KEPT: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// The live `UhdDevice`s per `args`: a kept device is freed only when none of its `args`
/// lives, since its teardown writes the X300's registers and releases its claim (Review T,
/// NB-T4).
static LIVE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// How many devices this process keeps rather than freed (F4; Review T, TG-T2).
pub fn kept_count() -> usize {
    lock(&KEPT).len()
}

/// Whether the device behind `usrp` answers a time read (the C API catches the
/// exception a dead link throws).
fn answers(usrp: uhd_usrp_handle) -> bool {
    let (mut full, mut frac) = (0i64, 0f64);
    // SAFETY: `usrp` is a live handle; both outputs are written by the call.
    unsafe { uhd_usrp_get_time_now(usrp, 0, &mut full, &mut frac) == UHD_ERROR_NONE }
}

/// Frees every kept device of `args` that answers again: its teardown writes to the
/// device, which a live link takes (NB-S2). Called before a new open of `args`, so that
/// its writes go out on its own transport.
fn reclaim(args: &str) {
    if lock(&LIVE).iter().any(|live| live == args) {
        return;
    }
    // Taken out of the list and read without its lock: on a dead link each read waits out
    // UHD's timeouts, which would hold up every drop that keeps a device (Review T, NB-T3).
    let mine: Vec<Kept> = {
        let mut kept = lock(&KEPT);
        let (mine, others) = std::mem::take(&mut *kept).into_iter().partition(|k| k.args == args);
        *kept = others;
        mine
    };
    for mut kept in mine {
        if !answers(kept.usrp) {
            lock(&KEPT).push(kept);
            continue;
        }
        kept.lost.store(false, Ordering::Release);
        kept.streams = (BTreeMap::new(), BTreeMap::new());
        // SAFETY: the kept handle is freed once, here, its streamers gone first.
        unsafe { uhd_usrp_free(&mut kept.usrp) };
    }
}

impl Drop for UhdDevice {
    fn drop(&mut self) {
        // A lost device is not freed: `uhd_usrp_free` tears the RFNoC graph down, whose
        // X300 radio `deinit()` writes the device's registers and, over a dead link, throws
        // out of `~rfnoc_graph_impl`, ending the process (`x300_radio_control.cpp:1890–1911`;
        // design-notes §17, F4). Nor is one that no longer answers, though not marked: a
        // dead link the Provider has not found yet (Review S, S-B1). It is kept until a
        // later open of the same `args` finds it answering (`reclaim`); UHD's static
        // registry holds it till the process exits otherwise.
        if !self.lost.load(Ordering::Acquire) && !answers(self.usrp) {
            self.lost.store(true, Ordering::Release);
        }
        {
            let mut live = lock(&LIVE);
            if let Some(i) = live.iter().position(|a| *a == self.args) {
                live.swap_remove(i);
            }
        }
        if self.lost.load(Ordering::Acquire) {
            let streams = (std::mem::take(&mut *lock(&self.rx)), std::mem::take(&mut *lock(&self.tx)));
            lock(&KEPT).push(Kept { args: self.args.clone(), usrp: self.usrp, lost: self.lost.clone(), streams });
            return;
        }
        self.close_streams();
        // SAFETY: the device outlives every streamer made from it, then is freed once.
        unsafe { uhd_usrp_free(&mut self.usrp) };
    }
}

impl Device for UhdDevice {
    fn describe(&self) -> serde_json::Value {
        let mut pp = vec![0 as c_char; 16_384];
        let mut version = [0 as c_char; 256];
        // SAFETY: both buffers are writable for their lengths; as in `text`, UHD is
        // given one byte less, so each ends in a NUL.
        let (pp, version) = unsafe {
            let _control = lock(&self.control);
            uhd_usrp_get_pp_string(self.usrp, pp.as_mut_ptr(), pp.len() - 1);
            uhd_get_version_string(version.as_mut_ptr(), version.len() - 1);
            (
                CStr::from_ptr(pp.as_ptr()).to_string_lossy().into_owned(),
                CStr::from_ptr(version.as_ptr()).to_string_lossy().into_owned(),
            )
        };
        serde_json::json!({
            "args": self.args,
            "master_clock_rate": self.mcr,
            "rx_channels": self.channels(Dir::Rx),
            "tx_channels": self.channels(Dir::Tx),
            "pp_string": pp,
            "uhd_version": version,
        })
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity {
            timing: EnvelopeFidelity::Real,
            continuity: EnvelopeFidelity::Real,
            coercion: CoercionFidelity::Real,
            rf: RfFidelity::Real,
            transport: TransportFidelity::Real,
        }
    }

    fn master_clock_rate(&self) -> u64 {
        self.mcr
    }

    fn channels(&self, dir: Dir) -> usize {
        let mut n = 0usize;
        let _control = lock(&self.control);
        // SAFETY: `n` is written by the call.
        unsafe {
            match dir {
                Dir::Rx => uhd_usrp_get_rx_num_channels(self.usrp, &mut n),
                Dir::Tx => uhd_usrp_get_tx_num_channels(self.usrp, &mut n),
            };
        }
        n
    }

    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> {
        let _control = lock(&self.control);
        let mut code = UHD_ERROR_NONE;
        let name = text(|b, n| {
            // SAFETY: `text`'s buffer is writable for the length passed.
            code = unsafe {
                match dir {
                    Dir::Rx => uhd_usrp_get_rx_subdev_name(self.usrp, chan, b, n),
                    Dir::Tx => uhd_usrp_get_tx_subdev_name(self.usrp, chan, b, n),
                }
            };
            code
        });
        let call = if dir == Dir::Rx { "uhd_usrp_get_rx_subdev_name" } else { "uhd_usrp_get_tx_subdev_name" };
        self.check_usrp(call, code, false)?;
        Ok(name)
    }

    fn serial(&self) -> Result<String, DeviceError> {
        let _control = lock(&self.control);
        // SAFETY: the info struct is written by the call and freed here.
        unsafe {
            let mut info = uhd_usrp_rx_info_t {
                mboard_id: std::ptr::null_mut(), mboard_name: std::ptr::null_mut(), mboard_serial: std::ptr::null_mut(),
                rx_id: std::ptr::null_mut(), rx_subdev_name: std::ptr::null_mut(), rx_subdev_spec: std::ptr::null_mut(),
                rx_serial: std::ptr::null_mut(), rx_antenna: std::ptr::null_mut(),
            };
            let read = self.check_usrp("uhd_usrp_get_rx_info", uhd_usrp_get_rx_info(self.usrp, 0, &mut info), false);
            // SAFETY: `mboard_serial` is filled in by the call, one NUL-terminated string; the
            // struct's fields are freed here whether it succeeded or not (UR-3).
            let serial = (!info.mboard_serial.is_null()).then(|| CStr::from_ptr(info.mboard_serial).to_string_lossy().into_owned());
            uhd_usrp_rx_info_free(&mut info);
            read?;
            serial.ok_or_else(|| DeviceError::failed("uhd_usrp_get_rx_info: no motherboard serial"))
        }
    }

    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> {
        let markup = cstring(markup)?;
        let _control = lock(&self.control);
        // SAFETY: the spec is made from `markup` by `made_subdev_spec`, set on mboard 0 and
        // freed here.
        unsafe {
            let mut spec = made_subdev_spec(&markup)?;
            let call = if dir == Dir::Rx { "uhd_usrp_set_rx_subdev_spec" } else { "uhd_usrp_set_tx_subdev_spec" };
            let set = if dir == Dir::Rx {
                self.check_usrp(call, uhd_usrp_set_rx_subdev_spec(self.usrp, spec, 0), false)
            } else {
                self.check_usrp(call, uhd_usrp_set_tx_subdev_spec(self.usrp, spec, 0), false)
            };
            uhd_subdev_spec_free(&mut spec);
            set?;
        }
        // UR-33: UHD's `set_*_subdev_spec` rebuilds the direction's channel map, so that
        // direction's open streamers are disconnected with it (a thread still inside a call
        // keeps its own `Arc`, UR-16). Nothing here sets a spec with a streamer open.
        match dir {
            Dir::Rx => lock(&self.rx).clear(),
            Dir::Tx => lock(&self.tx).clear(),
        }
        Ok(())
    }

    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> {
        let _control = lock(&self.control);
        // SAFETY: the meta range is made, read and freed here; each read writes into a local.
        unsafe {
            let mut range: uhd_meta_range_handle = std::ptr::null_mut();
            check("uhd_meta_range_make", uhd_meta_range_make(&mut range), false)?;
            let read = if dir == Dir::Rx {
                self.check_usrp("uhd_usrp_get_rx_rates", uhd_usrp_get_rx_rates(self.usrp, chan, range), false)
            } else {
                self.check_usrp("uhd_usrp_get_tx_rates", uhd_usrp_get_tx_rates(self.usrp, chan, range), false)
            };
            let mut out = Vec::new();
            let read = read.and_then(|()| {
                let mut size = 0usize;
                check("uhd_meta_range_size", uhd_meta_range_size(range, &mut size), false)?;
                for index in 0..size {
                    let mut one = uhd_range_t::default();
                    check("uhd_meta_range_at", uhd_meta_range_at(range, index, &mut one), false)?;
                    out.push((one.start, one.stop, one.step));
                }
                Ok(())
            });
            uhd_meta_range_free(&mut range);
            read.map(|()| out)
        }
    }

    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> {
        let (clock_c, time_c) = (cstring(clock)?, cstring(time)?);
        let _control = lock(&self.control);
        // SAFETY: the strings outlive the calls. The clock source first (UR-7).
        unsafe {
            self.check_usrp("uhd_usrp_set_clock_source", uhd_usrp_set_clock_source(self.usrp, clock_c.as_ptr(), 0), false)?;
            self.check_usrp("uhd_usrp_set_time_source", uhd_usrp_set_time_source(self.usrp, time_c.as_ptr(), 0), false)
        }
    }

    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> {
        let _control = lock(&self.control);
        // SAFETY: plain calls on the device handle.
        unsafe {
            if at_next_pps {
                self.check_usrp("uhd_usrp_set_time_unknown_pps", uhd_usrp_set_time_unknown_pps(self.usrp, 0, 0.0), false)
            } else {
                self.check_usrp("uhd_usrp_set_time_now", uhd_usrp_set_time_now(self.usrp, 0, 0.0, 0), false)
            }
        }
    }

    fn time_now(&self) -> Result<i64, DeviceError> {
        let (mut full, mut frac) = (0i64, 0f64);
        let _control = lock(&self.control);
        // SAFETY: both outputs are written by the call.
        unsafe { self.check_usrp("uhd_usrp_get_time_now", uhd_usrp_get_time_now(self.usrp, 0, &mut full, &mut frac), false)? };
        from_time_spec(full, frac, self.mcr)
    }

    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> {
        let _control = lock(&self.control);
        let mut names: uhd_string_vector_handle = std::ptr::null_mut();
        // SAFETY: the vector and the sensor value are made and freed here.
        unsafe {
            check("uhd_string_vector_make", uhd_string_vector_make(&mut names), false)?;
            let listed = self.check_usrp("uhd_usrp_get_mboard_sensor_names", uhd_usrp_get_mboard_sensor_names(self.usrp, 0, &mut names), false)
                .map(|()| strings(names));
            uhd_string_vector_free(&mut names);
            if !listed?.iter().any(|n| n == "ref_locked") {
                return Ok(None);
            }
            let name = cstring("ref_locked")?;
            let mut value: uhd_sensor_value_handle = std::ptr::null_mut();
            check("uhd_sensor_value_make", uhd_sensor_value_make(&mut value), false)?;
            let mut locked = false;
            let read = self.check_usrp("uhd_usrp_get_mboard_sensor", uhd_usrp_get_mboard_sensor(self.usrp, name.as_ptr(), 0, &mut value), false)
                .and_then(|()| check("uhd_sensor_value_to_bool", uhd_sensor_value_to_bool(value, &mut locked), false));
            uhd_sensor_value_free(&mut value);
            read.map(|()| Some(locked))
        }
    }

    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        let empty = cstring("")?;
        let antenna = s.antenna.as_deref().map(cstring).transpose()?;
        let _control = lock(&self.control);
        // SAFETY: every pointer passed outlives its call; the command time is set and
        // cleared under the control mutex, as it is per-motherboard state (UR-3).
        unsafe {
            if let Some(at) = at {
                let (full, frac) = to_time_spec(at, self.mcr);
                self.check_usrp("uhd_usrp_set_command_time", uhd_usrp_set_command_time(self.usrp, full, frac, 0), false)?;
            }
            let applied = (|| {
                if let Some(rate) = s.rate {
                    match dir {
                        Dir::Rx => self.check_usrp("uhd_usrp_set_rx_rate", uhd_usrp_set_rx_rate(self.usrp, rate, chan), false)?,
                        Dir::Tx => self.check_usrp("uhd_usrp_set_tx_rate", uhd_usrp_set_tx_rate(self.usrp, rate, chan), false)?,
                    }
                }
                if let Some(freq) = s.freq {
                    let mut request = uhd_tune_request_t {
                        target_freq: freq,
                        rf_freq_policy: UHD_TUNE_REQUEST_POLICY_AUTO,
                        rf_freq: 0.0,
                        dsp_freq_policy: UHD_TUNE_REQUEST_POLICY_AUTO,
                        dsp_freq: 0.0,
                        args: empty.as_ptr() as *mut c_char,
                    };
                    let mut result = uhd_tune_result_t::default();
                    match dir {
                        Dir::Rx => self.check_usrp("uhd_usrp_set_rx_freq", uhd_usrp_set_rx_freq(self.usrp, &mut request, chan, &mut result), false)?,
                        Dir::Tx => self.check_usrp("uhd_usrp_set_tx_freq", uhd_usrp_set_tx_freq(self.usrp, &mut request, chan, &mut result), false)?,
                    }
                }
                if let Some(gain) = s.gain {
                    match dir {
                        Dir::Rx => self.check_usrp("uhd_usrp_set_rx_gain", uhd_usrp_set_rx_gain(self.usrp, gain, chan, empty.as_ptr()), false)?,
                        Dir::Tx => self.check_usrp("uhd_usrp_set_tx_gain", uhd_usrp_set_tx_gain(self.usrp, gain, chan, empty.as_ptr()), false)?,
                    }
                }
                if let Some(antenna) = &antenna {
                    match dir {
                        Dir::Rx => self.check_usrp("uhd_usrp_set_rx_antenna", uhd_usrp_set_rx_antenna(self.usrp, antenna.as_ptr(), chan), false)?,
                        Dir::Tx => self.check_usrp("uhd_usrp_set_tx_antenna", uhd_usrp_set_tx_antenna(self.usrp, antenna.as_ptr(), chan), false)?,
                    }
                }
                Ok::<(), DeviceError>(())
            })();
            // A command time left set would make every later untimed call timed.
            let cleared = if at.is_some() {
                self.check_usrp("uhd_usrp_clear_command_time", uhd_usrp_clear_command_time(self.usrp, 0), false)
            } else {
                Ok(())
            };
            applied?;
            cleared?;
            let (mut rate, mut freq, mut gain) = (0.0, 0.0, 0.0);
            match dir {
                Dir::Rx => {
                    self.check_usrp("uhd_usrp_get_rx_rate", uhd_usrp_get_rx_rate(self.usrp, chan, &mut rate), false)?;
                    self.check_usrp("uhd_usrp_get_rx_freq", uhd_usrp_get_rx_freq(self.usrp, chan, &mut freq), false)?;
                    self.check_usrp("uhd_usrp_get_rx_gain", uhd_usrp_get_rx_gain(self.usrp, chan, empty.as_ptr(), &mut gain), false)?;
                }
                Dir::Tx => {
                    self.check_usrp("uhd_usrp_get_tx_rate", uhd_usrp_get_tx_rate(self.usrp, chan, &mut rate), false)?;
                    self.check_usrp("uhd_usrp_get_tx_freq", uhd_usrp_get_tx_freq(self.usrp, chan, &mut freq), false)?;
                    self.check_usrp("uhd_usrp_get_tx_gain", uhd_usrp_get_tx_gain(self.usrp, chan, empty.as_ptr(), &mut gain), false)?;
                }
            }
            // The antenna read back after an untimed change that named one (UR-2, design 33
            // D16): UHD's getter reads the property the setter wrote.
            let antenna = match (s.antenna.as_ref(), at) {
                (Some(_), None) => {
                    let mut code = UHD_ERROR_NONE;
                    let read = text(|b, n| {
                        // SAFETY: `text`'s buffer is writable for the length passed.
                        code = match dir {
                            Dir::Rx => uhd_usrp_get_rx_antenna(self.usrp, chan, b, n),
                            Dir::Tx => uhd_usrp_get_tx_antenna(self.usrp, chan, b, n),
                        };
                        code
                    });
                    let call = if dir == Dir::Rx { "uhd_usrp_get_rx_antenna" } else { "uhd_usrp_get_tx_antenna" };
                    self.check_usrp(call, code, false)?;
                    Some(read)
                }
                _ => None,
            };
            Ok(Applied { rate, freq, gain, antenna })
        }
    }

    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        let (fc32, sc16, empty) = (cstring("fc32")?, cstring("sc16")?, cstring("")?);
        // UHD reads an empty channel list as channel 0 (`sanitize_stream_args`), so the
        // streamer's channel count is what the device streams (UR-33).
        let mut list: Vec<usize> = if channels.is_empty() { vec![0] } else { channels.to_vec() };
        let mut args = Self::stream_args(&mut list, &fc32, &sc16, &empty);
        let _control = lock(&self.control);
        // UR-25: this stream's old streamer goes before the new one is made (a thread still
        // inside a call on it keeps it alive through its own Arc, UR-16).
        drop(lock(&self.rx).remove(&stream));
        let made = RxStream::make(list.len(), self.lost.clone(), |h| {
            let mut samples = 0usize;
            // SAFETY: the device handle lives as long as `self`; `args` outlives the call.
            unsafe {
                check_rx(h, "uhd_usrp_get_rx_stream", uhd_usrp_get_rx_stream(self.usrp, &mut args, h))?;
                check_rx(h, "uhd_rx_streamer_max_num_samps", uhd_rx_streamer_max_num_samps(h, &mut samples))?;
            }
            Ok(samples)
        })?;
        lock(&self.rx).insert(stream, Arc::new(made));
        Ok(())
    }

    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> {
        self.stream_cmd(stream, true, Some(at))
    }

    fn rx_packet_samples(&self, stream: usize) -> usize {
        lock(&self.rx).get(&stream).map_or(0, |stream| stream.spp)
    }

    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> {
        self.stream_cmd(stream, false, at)
    }

    fn rx_recv(&self, stream: usize, n: usize, timeout: Duration) -> RxRecv {
        let Some(stream) = lock(&self.rx).get(&stream).cloned() else {
            return RxRecv::Failed(DeviceError::failed("UR-3: the receive streamer is not open"));
        };
        let _call = lock(&stream.call);
        // ponytail: a fresh buffer per call, because the trait returns owned samples;
        // UR-32's reuse is Phase 8's copy-regression benchmark to settle.
        let mut buffers: Vec<Vec<Iq>> = vec![vec![[0.0; 2]; n]; stream.channels];
        let mut pointers: Vec<*mut c_void> = buffers.iter_mut().map(|b| b.as_mut_ptr() as *mut c_void).collect();
        let mut md = stream.md;
        let mut received = 0usize;
        // SAFETY: each buffer holds `n` samples per channel; the Arc keeps the streamer.
        let code = unsafe { uhd_rx_streamer_recv(stream.h, pointers.as_mut_ptr(), n, &mut md, timeout.as_secs_f64(), false, &mut received) };
        if let Err(error) = check_rx(stream.h, "uhd_rx_streamer_recv", code) {
            return RxRecv::Failed(error);
        }
        let mut error_code: c_int = 0;
        // SAFETY: the metadata belongs to this streamer.
        unsafe { uhd_rx_metadata_error_code(md, &mut error_code) };
        match error_code {
            UHD_RX_METADATA_ERROR_CODE_NONE if received > 0 => {
                let (mut has, mut full, mut frac) = (false, 0i64, 0f64);
                // SAFETY: as above.
                unsafe {
                    uhd_rx_metadata_has_time_spec(md, &mut has);
                    uhd_rx_metadata_time_spec(md, &mut full, &mut frac);
                }
                if !has {
                    return RxRecv::Failed(DeviceError::failed("UR-3: samples arrived without a time spec"));
                }
                for buffer in &mut buffers {
                    buffer.truncate(received);
                }
                match from_time_spec(full, frac, self.mcr) {
                    Ok(first_tick) => RxRecv::Samples { first_tick, samples: buffers },
                    Err(error) => RxRecv::Failed(error),
                }
            }
            UHD_RX_METADATA_ERROR_CODE_NONE | UHD_RX_METADATA_ERROR_CODE_TIMEOUT => RxRecv::Timeout,
            UHD_RX_METADATA_ERROR_CODE_OVERFLOW => {
                let mut out_of_sequence = false;
                // SAFETY: as above.
                unsafe { uhd_rx_metadata_out_of_sequence(md, &mut out_of_sequence) };
                RxRecv::Overflow { out_of_sequence }
            }
            UHD_RX_METADATA_ERROR_CODE_LATE_COMMAND => RxRecv::LateCommand,
            UHD_RX_METADATA_ERROR_CODE_ALIGNMENT => RxRecv::Alignment,
            UHD_RX_METADATA_ERROR_CODE_BAD_PACKET => RxRecv::BadPacket,
            other => {
                let mut text = [0 as c_char; 256];
                // SAFETY: the buffer is writable for its length, less its final NUL.
                let text = unsafe {
                    uhd_rx_metadata_strerror(md, text.as_mut_ptr(), text.len() - 1);
                    CStr::from_ptr(text.as_ptr()).to_string_lossy().into_owned()
                };
                // Not a lost device: UR-29's list names none of these codes.
                RxRecv::Failed(DeviceError { lost: false, message: format!("uhd_rx_streamer_recv: metadata error {other:#x}: {text}") })
            }
        }
    }

    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        let (fc32, sc16, empty) = (cstring("fc32")?, cstring("sc16")?, cstring("")?);
        // As in `rx_open`, an empty list is channel 0 (UR-33).
        let mut list: Vec<usize> = if channels.is_empty() { vec![0] } else { channels.to_vec() };
        let mut args = Self::stream_args(&mut list, &fc32, &sc16, &empty);
        let _control = lock(&self.control);
        drop(lock(&self.tx).remove(&stream));
        let made = TxStream::make(list.len(), self.lost.clone(), |h| {
            let mut samples = 0usize;
            // SAFETY: as for `rx_open`.
            unsafe {
                check_tx(h, "uhd_usrp_get_tx_stream", uhd_usrp_get_tx_stream(self.usrp, &mut args, h))?;
                check_tx(h, "uhd_tx_streamer_max_num_samps", uhd_tx_streamer_max_num_samps(h, &mut samples))
            }
        })?;
        lock(&self.tx).insert(stream, Arc::new(made));
        Ok(())
    }

    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Duration) -> Result<usize, DeviceError> {
        let Some(streamer) = lock(&self.tx).get(&stream).cloned() else {
            return Err(DeviceError::failed("UR-3: the transmit streamer is not open"));
        };
        if samples.len() != streamer.channels {
            return Err(DeviceError::failed(format!("UR-3: {} buffers for {} channels", samples.len(), streamer.channels)));
        }
        let n = samples.first().map_or(0, |s| s.len());
        if samples.iter().any(|samples| samples.len() < n) {
            return Err(DeviceError::failed(format!("UR-3: a transmit channel buffer is shorter than {n} samples")));
        }
        let _call = lock(&streamer.call);
        let (full, frac) = at.map_or((0, 0.0), |t| to_time_spec(t, self.mcr));
        let mut pointers: Vec<*const c_void> = samples.iter().map(|s| s.as_ptr() as *const c_void).collect();
        let mut md: uhd_tx_metadata_handle = std::ptr::null_mut();
        let mut sent = 0usize;
        // SAFETY: the metadata is made and freed here; each buffer holds `n` samples.
        unsafe {
            check("uhd_tx_metadata_make", uhd_tx_metadata_make(&mut md, at.is_some(), full, frac, sob, eob), true)?;
            let result = check_tx(streamer.h, "uhd_tx_streamer_send", uhd_tx_streamer_send(streamer.h, pointers.as_mut_ptr(), n, &mut md, timeout.as_secs_f64(), &mut sent));
            uhd_tx_metadata_free(&mut md);
            result?;
        }
        Ok(sent)
    }

    fn tx_async(&self, stream: usize, timeout: Duration) -> Result<Option<TxReport>, DeviceError> {
        let Some(streamer) = lock(&self.tx).get(&stream).cloned() else { return Ok(None) };
        let _call = lock(&streamer.call);
        let mut md = streamer.md;
        let mut valid = false;
        // SAFETY: the metadata belongs to this streamer, which the Arc keeps.
        unsafe {
            let code = uhd_tx_streamer_recv_async_msg(streamer.h, &mut md, timeout.as_secs_f64(), &mut valid);
            check_tx(streamer.h, "uhd_tx_streamer_recv_async_msg", code)?;
            if !valid {
                return Ok(None);
            }
            let (mut event, mut channel, mut has, mut full, mut frac) = (0 as c_int, 0usize, false, 0i64, 0f64);
            uhd_async_metadata_event_code(md, &mut event);
            uhd_async_metadata_channel(md, &mut channel);
            uhd_async_metadata_has_time_spec(md, &mut has);
            uhd_async_metadata_time_spec(md, &mut full, &mut frac);
            let code = match event {
                UHD_ASYNC_METADATA_EVENT_CODE_BURST_ACK => TxCode::BurstAck,
                UHD_ASYNC_METADATA_EVENT_CODE_UNDERFLOW => TxCode::Underflow,
                UHD_ASYNC_METADATA_EVENT_CODE_UNDERFLOW_IN_PACKET => TxCode::UnderflowInPacket,
                UHD_ASYNC_METADATA_EVENT_CODE_SEQ_ERROR => TxCode::SeqError,
                UHD_ASYNC_METADATA_EVENT_CODE_SEQ_ERROR_IN_BURST => TxCode::SeqErrorInBurst,
                UHD_ASYNC_METADATA_EVENT_CODE_TIME_ERROR => TxCode::TimeError,
                other => TxCode::Other(other),
            };
            let tick = if has { from_time_spec(full, frac, self.mcr).ok() } else { None };
            Ok(Some(TxReport { code, tick, channel }))
        }
    }

    fn mark_lost(&self) {
        self.lost.store(true, Ordering::Release);
    }

    fn raise_thread_priority(&self) -> Result<(), DeviceError> {
        // SAFETY: no handle; acts on the calling thread. UHD's number is the verdict; its text is the process's last error.
        // A refusal (EPERM is `UHD_ERROR_OS`, which `check` takes for a loss) is never the device's loss.
        check("uhd_set_thread_priority", unsafe { uhd_set_thread_priority(0.5, true) }, false).map_err(|error| DeviceError { lost: false, ..error })
    }

    fn rx_close(&self, stream: usize) {
        let _control = lock(&self.control);
        #[cfg(test)]
        debug_assert!(self.control.try_lock().is_err(), "rx_close must serialize with rx_open");
        drop(lock(&self.rx).remove(&stream));
    }

    fn tx_close(&self, stream: usize) {
        let _control = lock(&self.control);
        #[cfg(test)]
        debug_assert!(self.control.try_lock().is_err(), "tx_close must serialize with tx_open");
        drop(lock(&self.tx).remove(&stream));
    }

    fn close_streams(&self) {
        // UR-2, UR-16: every streamer of both directions; dropping the device's reference
        // frees each only once no thread still holds its Arc (and never a lost device's, F4).
        lock(&self.rx).clear();
        lock(&self.tx).clear();
    }
}

// Native-boundary regressions without a USRP. Only this unit-test executable
// substitutes streamer calls; integration tests still link real libuhd entry points.
#[cfg(test)]
mod ffi_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Barrier, Condvar};

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static SEND_CALLS: AtomicUsize = AtomicUsize::new(0);
    static PROBE: AtomicBool = AtomicBool::new(false);
    static ASYNC_ERROR: AtomicBool = AtomicBool::new(false);
    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    static MAX_ACTIVE: AtomicUsize = AtomicUsize::new(0);
    static ENTERED: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
    static RX_STREAM_FREES: AtomicUsize = AtomicUsize::new(0);
    static OPEN_STREAM_FAILS: AtomicBool = AtomicBool::new(false);
    static PRIORITY_CALLS: AtomicUsize = AtomicUsize::new(0);
    static PRIORITY_CODE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    static PRIORITY_ARGS: Mutex<Option<(f32, bool)>> = Mutex::new(None);

    fn native_entry() {
        if !PROBE.load(Ordering::SeqCst) { return; }
        let active = ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
        MAX_ACTIVE.fetch_max(active, Ordering::SeqCst);
        let mut entered = lock(&ENTERED.0);
        *entered += 1;
        ENTERED.1.notify_all();
        // A missing call lock allows both entries to rendezvous. With the lock,
        // the first waits out the rendezvous, returns, then permits the second.
        let _ = ENTERED.1.wait_timeout_while(entered, Duration::from_millis(500), |n| *n < 2)
            .unwrap_or_else(|e| e.into_inner());
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }

    fn pair(device: &Arc<UhdDevice>, a: fn(&UhdDevice), b: fn(&UhdDevice)) {
        ACTIVE.store(0, Ordering::SeqCst);
        MAX_ACTIVE.store(0, Ordering::SeqCst);
        *lock(&ENTERED.0) = 0;
        PROBE.store(true, Ordering::SeqCst);
        let start = Barrier::new(3);
        std::thread::scope(|scope| {
            for call in [a, b] {
                let start = &start;
                scope.spawn(move || { start.wait(); call(device); });
            }
            start.wait();
        });
        PROBE.store(false, Ordering::SeqCst);
        assert_eq!(*lock(&ENTERED.0), 2, "both calls entered the native boundary");
        assert_eq!(MAX_ACTIVE.load(Ordering::SeqCst), 1, "safe shared methods must serialize same-streamer native calls");
    }

    fn send(device: &UhdDevice) {
        device.tx_send(0, &[&[[0.0, 0.0]]], None, false, false, Duration::from_secs(1)).unwrap();
    }

    #[unsafe(no_mangle)]
    extern "C" fn uhd_tx_streamer_send(
        _h: uhd_tx_streamer_handle, _buffs: *mut *const c_void,
        n: usize, _md: *mut uhd_tx_metadata_handle, _timeout: f64, sent: *mut usize,
    ) -> c_int {
        native_entry();
        SEND_CALLS.fetch_add(1, Ordering::SeqCst);
        // SAFETY: tx_send supplies a live local output slot. Sample pointers and
        // dummy streamer handles are never read by this test replacement.
        unsafe { *sent = n; }
        UHD_ERROR_NONE
    }

    fn dummy(channels: usize) -> Arc<UhdDevice> {
        let lost = Arc::new(AtomicBool::new(true));
        let mut tx = BTreeMap::new();
        tx.insert(0, Arc::new(TxStream {
            h: std::ptr::null_mut(), call: Mutex::new(()), md: std::ptr::null_mut(), channels, lost: lost.clone(),
        }));
        Arc::new(UhdDevice {
            usrp: std::ptr::null_mut(), args: "ffi-test-no-device".to_owned(), mcr: 200_000_000,
            control: Mutex::new(()), rx: Mutex::new(BTreeMap::new()), tx: Mutex::new(tx), lost,
        })
    }

    #[test]
    fn uhd_tx_send_refuses_short_channels_before_c() {
        let _test = lock(&TEST_LOCK);
        let device = dummy(3);
        let long = vec![[0.0, 0.0]; 100];
        let short = [[0.0, 0.0]; 1];
        let longer = vec![[0.0, 0.0]; 101];
        for samples in [vec![&long[..], &short[..], &long[..]],
                        vec![&long[..], &long[..], &short[..]],
                        vec![&long[..], &[][..], &long[..]]] {
            SEND_CALLS.store(0, Ordering::SeqCst);
            assert!(device.tx_send(0, &samples, None, false, false, Duration::from_millis(1)).is_err());
            assert_eq!(SEND_CALLS.load(Ordering::SeqCst), 0, "short buffers must never reach C");
        }
        SEND_CALLS.store(0, Ordering::SeqCst);
        assert_eq!(device.tx_send(0, &[&long, &long, &longer], None, true, true, Duration::from_millis(1)).unwrap(), 100);
        assert_eq!(SEND_CALLS.load(Ordering::SeqCst), 1, "equal or longer buffers remain accepted");
        assert_eq!(device.tx_send(0, &[&[], &[], &[]], None, false, true, Duration::from_millis(1)).unwrap(), 0);
        assert!(device.tx_send(0, &[&long, &long], None, false, false, Duration::from_millis(1)).is_err());
    }
    #[test]
    fn uhd_safe_shared_device_serializes_send() {
        let _test = lock(&TEST_LOCK);
        pair(&dummy(1), send, send);
    }

    #[test]
    fn uhd_streams_are_looked_up_by_number() {
        // UR-2, design 33 D5: the device holds one streamer per stream number; a call on a
        // number with no streamer fails as UHD's does, and each streamer keeps its own
        // channel count and packet size (the fake's double is `src/device.rs`).
        let _test = lock(&TEST_LOCK);
        let device = dummy(1);
        let wave = [[0.0f32, 0.0]; 4];
        assert!(device.tx_send(1, &[&wave], None, false, false, Duration::from_millis(1)).is_err(), "no transmit streamer 1");
        assert!(device.tx_async(1, Duration::ZERO).unwrap().is_none());
        lock(&device.tx).insert(1, Arc::new(TxStream {
            h: std::ptr::null_mut(), call: Mutex::new(()), md: std::ptr::null_mut(), channels: 2, lost: device.lost.clone(),
        }));
        assert!(device.tx_send(0, &[&wave, &wave], None, false, false, Duration::from_millis(1)).is_err(), "stream 0 has one channel");
        assert!(device.tx_send(1, &[&wave, &wave], None, false, false, Duration::from_millis(1)).is_ok(), "stream 1 has two");
        let mut md = std::ptr::null_mut();
        // SAFETY: made by real libuhd for rx_recv's metadata getters; the receive
        // replacement never dereferences the dummy streamer or sample pointers.
        unsafe { check("uhd_rx_metadata_make", uhd_rx_metadata_make(&mut md), true).unwrap(); }
        lock(&device.rx).insert(1, Arc::new(RxStream {
            h: std::ptr::null_mut(), call: Mutex::new(()), md, channels: 2, spp: 7, lost: device.lost.clone(),
        }));
        assert_eq!(device.rx_packet_samples(1), 7);
        assert_eq!(device.rx_packet_samples(0), 0, "stream 0 has no receive streamer");
        assert!(matches!(device.rx_recv(0, 1, Duration::from_millis(1)), RxRecv::Failed(_)));
        assert!(matches!(device.rx_recv(1, 1, Duration::from_millis(1)), RxRecv::Timeout));
        // SAFETY: nothing else holds this metadata; the dummy streamer's Drop is skipped.
        unsafe { uhd_rx_metadata_free(&mut md); }
    }

    #[unsafe(no_mangle)]
    extern "C" fn uhd_rx_streamer_recv(
        _h: uhd_rx_streamer_handle, _buffs: *mut *mut c_void, _n: usize,
        _md: *mut uhd_rx_metadata_handle, _timeout: f64, _one_packet: bool, received: *mut usize,
    ) -> c_int {
        native_entry();
        // SAFETY: rx_recv supplies a live output slot. No sample pointers read.
        unsafe { *received = 0; }
        UHD_ERROR_NONE
    }

    #[unsafe(no_mangle)]
    extern "C" fn uhd_rx_streamer_issue_stream_cmd(
        _h: uhd_rx_streamer_handle, _cmd: *const uhd_stream_cmd_t,
    ) -> c_int {
        native_entry();
        UHD_ERROR_NONE
    }

    #[unsafe(no_mangle)]
    extern "C" fn uhd_set_thread_priority(priority: f32, realtime: bool) -> uhd_error {
        PRIORITY_CALLS.fetch_add(1, Ordering::SeqCst);
        *lock(&PRIORITY_ARGS) = Some((priority, realtime));
        PRIORITY_CODE.load(Ordering::SeqCst)
    }

    #[test]
    fn ur_23_f1_the_priority_call_is_uhd_s_realtime_half() {
        let _test = lock(&TEST_LOCK);
        PRIORITY_CALLS.store(0, Ordering::SeqCst);
        PRIORITY_CODE.store(UHD_ERROR_NONE, Ordering::SeqCst);
        dummy(1).raise_thread_priority().unwrap();
        assert_eq!(PRIORITY_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(*lock(&PRIORITY_ARGS), Some((0.5f32, true)));
    }

    #[test]
    fn ur_23_f2_every_refusal_is_an_error_naming_the_call_and_the_code() {
        let _test = lock(&TEST_LOCK);
        for code in [UHD_ERROR_OS, 20, UHD_ERROR_RUNTIME] {
            PRIORITY_CODE.store(code, Ordering::SeqCst);
            let error = dummy(1).raise_thread_priority().unwrap_err();
            assert!(error.message.contains("uhd_set_thread_priority"), "{}", error.message);
            assert!(error.message.contains(&format!("UHD error {code}")), "{}", error.message);
            assert!(!error.lost, "a refused priority is not a lost device: {}", error.message);
        }
        PRIORITY_CODE.store(UHD_ERROR_NONE, Ordering::SeqCst);
    }

    #[unsafe(no_mangle)]
    extern "C" fn uhd_tx_streamer_recv_async_msg(
        _h: uhd_tx_streamer_handle, _md: *mut uhd_async_metadata_handle,
        _timeout: f64, valid: *mut bool,
    ) -> c_int {
        native_entry();
        // SAFETY: tx_async supplies a live output slot, and reads no metadata on false.
        unsafe { *valid = false; }
        if ASYNC_ERROR.load(Ordering::SeqCst) { UHD_ERROR_IO } else { UHD_ERROR_NONE }
    }

    #[unsafe(no_mangle)]
    extern "C" fn uhd_tx_streamer_last_error(_h: uhd_tx_streamer_handle, buf: *mut c_char, len: usize) -> c_int {
        let text = b"ffi test: async message failed\0";
        // SAFETY: `text` reads the error into a buffer writable for `len` bytes.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr() as *const c_char, buf, text.len().min(len)); }
        UHD_ERROR_NONE
    }

    #[test]
    fn uhd_tx_async_reports_a_failed_call_as_an_error() {
        // UR-3: a failed recv_async_msg is a DeviceError with its text, not "no report".
        let _test = lock(&TEST_LOCK);
        let device = dummy(1);
        assert!(device.tx_async(0, Duration::ZERO).unwrap().is_none());
        ASYNC_ERROR.store(true, Ordering::SeqCst);
        let error = device.tx_async(0, Duration::ZERO);
        ASYNC_ERROR.store(false, Ordering::SeqCst);
        let error = error.unwrap_err();
        assert!(error.lost, "UHD_ERROR_IO is the device's loss");
        assert!(error.message.starts_with("uhd_tx_streamer_recv_async_msg: UHD error"), "{}", error.message);
        assert!(error.message.ends_with("ffi test: async message failed"), "{}", error.message);
    }

    fn receive(device: &UhdDevice) {
        assert!(matches!(device.rx_recv(0, 1, Duration::from_secs(1)), RxRecv::Timeout));
    }
    fn start(device: &UhdDevice) { device.rx_start(0, 0).unwrap(); }
    fn report(device: &UhdDevice) { assert!(device.tx_async(0, Duration::from_secs(1)).unwrap().is_none()); }

    fn rx_open(device: &UhdDevice) { device.rx_open(0, &[0]).unwrap(); }
    fn rx_open_then_close(device: &UhdDevice) { device.rx_open(0, &[0]).unwrap(); device.rx_close(0); }

    fn rx_dummy(lost: bool) -> Arc<UhdDevice> {
        Arc::new(UhdDevice {
            usrp: std::ptr::null_mut(), args: "ffi-rx-test-no-device".to_owned(), mcr: 200_000_000,
            control: Mutex::new(()), rx: Mutex::new(BTreeMap::new()), tx: Mutex::new(BTreeMap::new()),
            lost: Arc::new(AtomicBool::new(lost)),
        })
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn uhd_rx_streamer_make(h: *mut uhd_rx_streamer_handle) -> c_int {
        // SAFETY: the caller supplies a live output slot; the paired free stub owns this box.
        unsafe { *h = Box::into_raw(Box::new(0u8)) as uhd_rx_streamer_handle; }
        UHD_ERROR_NONE
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn uhd_rx_streamer_free(h: *mut uhd_rx_streamer_handle) -> c_int {
        RX_STREAM_FREES.fetch_add(1, Ordering::SeqCst);
        // SAFETY: the test open stub allocated this handle once; the caller frees it once.
        unsafe { if !(*h).is_null() { drop(Box::from_raw(*h as *mut u8)); *h = std::ptr::null_mut(); } }
        UHD_ERROR_NONE
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn uhd_usrp_get_rx_stream(_usrp: uhd_usrp_handle, _args: *mut uhd_stream_args_t, _h: uhd_rx_streamer_handle) -> c_int {
        native_entry();
        if OPEN_STREAM_FAILS.load(Ordering::SeqCst) { UHD_ERROR_RUNTIME } else { UHD_ERROR_NONE }
    }

    /// Every test that opens a transmit streamer wants it to fail, so that no real graph is needed.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn uhd_usrp_get_tx_stream(_usrp: uhd_usrp_handle, _args: *mut uhd_stream_args_t, _h: uhd_tx_streamer_handle) -> c_int {
        UHD_ERROR_RUNTIME
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn uhd_rx_streamer_last_error(_h: uhd_rx_streamer_handle, buf: *mut c_char, len: usize) -> c_int {
        let text = b"ffi test: the rx streamer's text\0";
        // SAFETY: `buf` is writable for `len` bytes, as `text` passes it.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr() as *const c_char, buf, text.len().min(len)); }
        UHD_ERROR_NONE
    }

    #[test]
    fn uhd_a_failed_streamer_open_reads_its_text_from_the_streamer() {
        // UHD saves the error of `uhd_usrp_get_rx_stream` and `uhd_usrp_get_tx_stream` on the streamer's handle
        // (`UHD_SAFE_C_SAVE_ERROR(h_s, ...)`, usrp_c.cpp, v4.10.0.0), so the device handle's text is another call's.
        let _test = lock(&TEST_LOCK);
        OPEN_STREAM_FAILS.store(true, Ordering::SeqCst);
        let rx = rx_dummy(true).rx_open(0, &[0]);
        OPEN_STREAM_FAILS.store(false, Ordering::SeqCst);
        let tx = dummy(1).tx_open(0, &[0]);
        for (call, texts, error) in [("uhd_usrp_get_rx_stream", "the rx streamer's text", rx), ("uhd_usrp_get_tx_stream", "async message failed", tx)] {
            let error = error.unwrap_err();
            assert!(error.message.starts_with(&format!("{call}: UHD error 44: ")) && error.message.ends_with(texts), "{}", error.message);
            assert!(!error.message.contains("the usrp set failed"), "the device handle's text was read: {}", error.message);
        }
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn uhd_rx_streamer_max_num_samps(_h: uhd_rx_streamer_handle, n: *mut usize) -> c_int {
        // SAFETY: the caller supplies a live output slot.
        unsafe { *n = 64; }
        UHD_ERROR_NONE
    }

    #[test]
    fn uhd_close_drops_its_map_reference_but_an_in_flight_arc_keeps_the_streamer() {
        let _test = lock(&TEST_LOCK);
        RX_STREAM_FREES.store(0, Ordering::SeqCst);
        let device = rx_dummy(false);
        device.rx_open(0, &[0]).unwrap();
        let held = lock(&device.rx).get(&0).unwrap().clone();
        assert_eq!(Arc::strong_count(&held), 2, "the map and the call hold the streamer");
        device.rx_close(0);
        assert!(lock(&device.rx).get(&0).is_none(), "close removed the map entry");
        assert_eq!(Arc::strong_count(&held), 1, "the in-flight call still owns the streamer");
        assert_eq!(RX_STREAM_FREES.load(Ordering::SeqCst), 0);
        drop(held);
        assert_eq!(RX_STREAM_FREES.load(Ordering::SeqCst), 1);
        std::mem::forget(device);
    }

    #[test]
    fn uhd_stream_close_and_open_compete_under_the_control_mutex() {
        let _test = lock(&TEST_LOCK);
        let device = rx_dummy(true);
        pair(&device, rx_open_then_close, rx_open);
        let streams = lock(&device.rx);
        assert!(streams.len() <= 1, "one stream number has at most one map entry");
        drop(streams);
        device.rx_open(0, &[0]).unwrap();
        assert!(lock(&device.rx).contains_key(&0));
        device.rx_close(0);
        assert!(lock(&device.rx).is_empty());
        std::mem::forget(device);
    }

    #[test]
    fn uhd_safe_shared_device_serializes_receive_and_reports() {
        let _test = lock(&TEST_LOCK);
        let device = dummy(1);
        let mut md = std::ptr::null_mut();
        // SAFETY: made by real libuhd for rx_recv's metadata getters; the receive
        // replacement never dereferences the dummy streamer or sample pointers.
        unsafe { check("uhd_rx_metadata_make", uhd_rx_metadata_make(&mut md), true).unwrap(); }
        lock(&device.rx).insert(0, Arc::new(RxStream {
            h: std::ptr::null_mut(), call: Mutex::new(()), md,
            channels: 1, spp: 1, lost: device.lost.clone(),
        }));
        pair(&device, receive, receive);
        pair(&device, receive, start);
        pair(&device, send, report);
        pair(&device, report, report);
        // SAFETY: every worker has joined; no further receive call uses this metadata.
        unsafe { uhd_rx_metadata_free(&mut md); }
        // Dummy handles are marked lost, so their Drop path never frees metadata again.
    }

    /// The subdev spec calls, replaced for this test binary as the streamer calls above:
    /// UHD's real make allocates the handle before it parses the markup and leaves it
    /// allocated when the parse fails (`subdev_spec_c.cpp:31-36`), and a free is no witness
    /// in this process (`uhd_subdev_spec_free` stores NULL, `:38`). `SUBDEV_MADE` and
    /// `SUBDEV_FREED` count every path's allocation and frees, `SUBDEV_SET_FAILS` makes the
    /// set fail as a device error would, and the substitutions end with this binary. The
    /// replacement refuses a token of three or more `:`-parts, as UHD does
    /// (`subdev_spec.cpp:58-59`), which is the refusal the test's markup meets.
    static SUBDEV_MADE: AtomicUsize = AtomicUsize::new(0);
    static SUBDEV_FREED: AtomicUsize = AtomicUsize::new(0);
    static SUBDEV_SET_FAILS: AtomicBool = AtomicBool::new(false);

    #[unsafe(no_mangle)]
    extern "C" fn uhd_subdev_spec_make(h: *mut uhd_subdev_spec_handle, markup: *const c_char) -> uhd_error {
        // SAFETY: `h` is a live output slot and `markup` the caller's C string, as in UHD.
        let malformed = unsafe { CStr::from_ptr(markup) }
            .to_string_lossy()
            .split(' ')
            .filter(|token| !token.is_empty())
            .any(|token| token.split(':').count() > 2);
        SUBDEV_MADE.fetch_add(1, Ordering::SeqCst);
        // SAFETY: the caller's slot is written once, with the handle its free takes back.
        unsafe { *h = Box::into_raw(Box::new(0usize)) as uhd_subdev_spec_handle };
        if malformed { UHD_ERROR_RUNTIME } else { UHD_ERROR_NONE }
    }

    #[unsafe(no_mangle)]
    extern "C" fn uhd_subdev_spec_free(h: *mut uhd_subdev_spec_handle) -> uhd_error {
        // SAFETY: `h` is the caller's live handle variable, and the handle was made above; as
        // in UHD, the free stores NULL, so a second free through the slot deletes nothing.
        unsafe {
            if !(*h).is_null() {
                SUBDEV_FREED.fetch_add(1, Ordering::SeqCst);
                drop(Box::from_raw(*h as *mut usize));
                *h = std::ptr::null_mut();
            }
        }
        UHD_ERROR_NONE
    }

    /// The replaced setters' answer: an error while `SUBDEV_SET_FAILS` says so, else success
    /// (the dummy device holds no usrp a real setter could call).
    fn subdev_set_fails() -> uhd_error {
        if SUBDEV_SET_FAILS.load(Ordering::SeqCst) { UHD_ERROR_RUNTIME } else { UHD_ERROR_NONE }
    }

    #[unsafe(no_mangle)]
    extern "C" fn uhd_usrp_set_rx_subdev_spec(_h: uhd_usrp_handle, _spec: uhd_subdev_spec_handle, _mboard: usize) -> uhd_error {
        subdev_set_fails()
    }

    #[unsafe(no_mangle)]
    extern "C" fn uhd_usrp_set_tx_subdev_spec(_h: uhd_usrp_handle, _spec: uhd_subdev_spec_handle, _mboard: usize) -> uhd_error {
        subdev_set_fails()
    }

    /// The replaced error-text reader: the dummy device's usrp handle is null, where UHD's
    /// own reader dereferences it, and only the replaced setters above can fail here.
    #[unsafe(no_mangle)]
    extern "C" fn uhd_usrp_last_error(_h: uhd_usrp_handle, buf: *mut c_char, len: usize) -> uhd_error {
        let text = b"ffi test: the usrp set failed\0";
        // SAFETY: `buf` is writable for `len` bytes, as `text` passes it.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr() as *const c_char, buf, text.len().min(len)); }
        UHD_ERROR_NONE
    }

    #[test]
    fn uhd_a_subdev_spec_is_freed_once_on_every_path() {
        // UR-3: `set_subdev_spec` frees its spec handle on every path exactly once — after the
        // set, the handle `uhd_subdev_spec_make` allocated before it refused the markup, and
        // a failed set's —, which the replaced calls count here (the real ones leave no
        // witness in this process). `tests/uhd_api.rs` runs the real make and free.
        let _test = lock(&TEST_LOCK);
        let device = dummy(1);
        let counts = || (SUBDEV_MADE.load(Ordering::SeqCst), SUBDEV_FREED.load(Ordering::SeqCst));
        let reset = || {
            SUBDEV_MADE.store(0, Ordering::SeqCst);
            SUBDEV_FREED.store(0, Ordering::SeqCst);
        };
        reset();
        device.set_subdev_spec(Dir::Rx, "A:0").unwrap();
        assert_eq!(counts(), (1, 1), "an accepted spec is made and freed once after the set");
        reset();
        let error = device.set_subdev_spec(Dir::Rx, "A:0:1").unwrap_err();
        assert!(error.message.starts_with("uhd_subdev_spec_make: UHD error "), "{}", error.message);
        assert_eq!(counts(), (1, 1), "the handle a refused make allocated is freed once");
        reset();
        SUBDEV_SET_FAILS.store(true, Ordering::SeqCst);
        let error = device.set_subdev_spec(Dir::Tx, "A:0").unwrap_err();
        SUBDEV_SET_FAILS.store(false, Ordering::SeqCst);
        assert!(error.message.starts_with("uhd_usrp_set_tx_subdev_spec: UHD error 44: ffi test: the usrp set failed"), "{}", error.message);
        assert_eq!(counts(), (1, 1), "the spec of a failed set is freed once");
    }

}
