//! The slice of the UHD C API (`uhd.h`, UHD 4.x) this spike calls. Hand-written, not
//! bindgen: about forty functions, and every struct is checked against the 4.10 headers.
#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_int, c_void};

pub type uhd_error = c_int;
pub const UHD_ERROR_NONE: uhd_error = 0;

pub type uhd_usrp_handle = *mut c_void;
pub type uhd_rx_streamer_handle = *mut c_void;
pub type uhd_tx_streamer_handle = *mut c_void;
pub type uhd_rx_metadata_handle = *mut c_void;
pub type uhd_tx_metadata_handle = *mut c_void;
pub type uhd_async_metadata_handle = *mut c_void;
pub type uhd_string_vector_handle = *mut c_void;

#[repr(C)]
pub struct uhd_stream_args_t {
    pub cpu_format: *mut c_char,
    pub otw_format: *mut c_char,
    pub args: *mut c_char,
    pub channel_list: *mut usize,
    pub n_channels: c_int,
}

pub const UHD_STREAM_MODE_START_CONTINUOUS: c_int = 97;
pub const UHD_STREAM_MODE_STOP_CONTINUOUS: c_int = 111;

#[repr(C)]
pub struct uhd_stream_cmd_t {
    pub stream_mode: c_int,
    pub num_samps: usize,
    pub stream_now: bool,
    pub time_spec_full_secs: i64,
    pub time_spec_frac_secs: f64,
}

pub const UHD_TUNE_REQUEST_POLICY_AUTO: c_int = 65;

#[repr(C)]
pub struct uhd_tune_request_t {
    pub target_freq: f64,
    pub rf_freq_policy: c_int,
    pub rf_freq: f64,
    pub dsp_freq_policy: c_int,
    pub dsp_freq: f64,
    pub args: *mut c_char,
}

#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct uhd_tune_result_t {
    pub clipped_rf_freq: f64,
    pub target_rf_freq: f64,
    pub actual_rf_freq: f64,
    pub target_dsp_freq: f64,
    pub actual_dsp_freq: f64,
}

pub const UHD_RX_METADATA_ERROR_CODE_NONE: c_int = 0x0;
pub const UHD_RX_METADATA_ERROR_CODE_TIMEOUT: c_int = 0x1;
pub const UHD_RX_METADATA_ERROR_CODE_LATE_COMMAND: c_int = 0x2;
pub const UHD_RX_METADATA_ERROR_CODE_BROKEN_CHAIN: c_int = 0x4;
pub const UHD_RX_METADATA_ERROR_CODE_OVERFLOW: c_int = 0x8;
pub const UHD_RX_METADATA_ERROR_CODE_ALIGNMENT: c_int = 0xC;
pub const UHD_RX_METADATA_ERROR_CODE_BAD_PACKET: c_int = 0xF;

pub const UHD_ASYNC_METADATA_EVENT_CODE_BURST_ACK: c_int = 0x1;
pub const UHD_ASYNC_METADATA_EVENT_CODE_UNDERFLOW: c_int = 0x2;
pub const UHD_ASYNC_METADATA_EVENT_CODE_SEQ_ERROR: c_int = 0x4;
pub const UHD_ASYNC_METADATA_EVENT_CODE_TIME_ERROR: c_int = 0x8;
pub const UHD_ASYNC_METADATA_EVENT_CODE_UNDERFLOW_IN_PACKET: c_int = 0x10;
pub const UHD_ASYNC_METADATA_EVENT_CODE_SEQ_ERROR_IN_BURST: c_int = 0x20;

unsafe extern "C" {
    pub fn uhd_get_last_error(error_out: *mut c_char, strbuffer_len: usize) -> uhd_error;

    pub fn uhd_string_vector_make(h: *mut uhd_string_vector_handle) -> uhd_error;
    pub fn uhd_string_vector_free(h: *mut uhd_string_vector_handle) -> uhd_error;
    pub fn uhd_string_vector_size(h: uhd_string_vector_handle, size_out: *mut usize) -> uhd_error;
    pub fn uhd_string_vector_at(
        h: uhd_string_vector_handle,
        index: usize,
        value_out: *mut c_char,
        strbuffer_len: usize,
    ) -> uhd_error;

    pub fn uhd_usrp_find(args: *const c_char, strings_out: *mut uhd_string_vector_handle) -> uhd_error;
    pub fn uhd_usrp_make(h: *mut uhd_usrp_handle, args: *const c_char) -> uhd_error;
    pub fn uhd_usrp_free(h: *mut uhd_usrp_handle) -> uhd_error;
    pub fn uhd_usrp_get_pp_string(h: uhd_usrp_handle, out: *mut c_char, len: usize) -> uhd_error;
    pub fn uhd_usrp_get_master_clock_rate(h: uhd_usrp_handle, mboard: usize, out: *mut f64) -> uhd_error;
    pub fn uhd_usrp_get_time_now(h: uhd_usrp_handle, mboard: usize, full: *mut i64, frac: *mut f64) -> uhd_error;
    pub fn uhd_usrp_set_time_now(h: uhd_usrp_handle, full: i64, frac: f64, mboard: usize) -> uhd_error;
    pub fn uhd_usrp_set_time_unknown_pps(h: uhd_usrp_handle, full: i64, frac: f64) -> uhd_error;
    pub fn uhd_usrp_set_command_time(h: uhd_usrp_handle, full: i64, frac: f64, mboard: usize) -> uhd_error;
    pub fn uhd_usrp_clear_command_time(h: uhd_usrp_handle, mboard: usize) -> uhd_error;
    pub fn uhd_usrp_set_time_source(h: uhd_usrp_handle, source: *const c_char, mboard: usize) -> uhd_error;
    pub fn uhd_usrp_set_clock_source(h: uhd_usrp_handle, source: *const c_char, mboard: usize) -> uhd_error;

    pub fn uhd_usrp_get_rx_num_channels(h: uhd_usrp_handle, out: *mut usize) -> uhd_error;
    pub fn uhd_usrp_set_rx_rate(h: uhd_usrp_handle, rate: f64, chan: usize) -> uhd_error;
    pub fn uhd_usrp_get_rx_rate(h: uhd_usrp_handle, chan: usize, out: *mut f64) -> uhd_error;
    pub fn uhd_usrp_set_rx_freq(
        h: uhd_usrp_handle,
        req: *mut uhd_tune_request_t,
        chan: usize,
        res: *mut uhd_tune_result_t,
    ) -> uhd_error;
    pub fn uhd_usrp_get_rx_freq(h: uhd_usrp_handle, chan: usize, out: *mut f64) -> uhd_error;
    pub fn uhd_usrp_set_rx_gain(h: uhd_usrp_handle, gain: f64, chan: usize, name: *const c_char) -> uhd_error;
    pub fn uhd_usrp_get_rx_gain(h: uhd_usrp_handle, chan: usize, name: *const c_char, out: *mut f64) -> uhd_error;
    pub fn uhd_usrp_set_rx_antenna(h: uhd_usrp_handle, ant: *const c_char, chan: usize) -> uhd_error;

    pub fn uhd_usrp_get_tx_num_channels(h: uhd_usrp_handle, out: *mut usize) -> uhd_error;
    pub fn uhd_usrp_set_tx_rate(h: uhd_usrp_handle, rate: f64, chan: usize) -> uhd_error;
    pub fn uhd_usrp_get_tx_rate(h: uhd_usrp_handle, chan: usize, out: *mut f64) -> uhd_error;
    pub fn uhd_usrp_set_tx_freq(
        h: uhd_usrp_handle,
        req: *mut uhd_tune_request_t,
        chan: usize,
        res: *mut uhd_tune_result_t,
    ) -> uhd_error;
    pub fn uhd_usrp_get_tx_freq(h: uhd_usrp_handle, chan: usize, out: *mut f64) -> uhd_error;
    pub fn uhd_usrp_set_tx_gain(h: uhd_usrp_handle, gain: f64, chan: usize, name: *const c_char) -> uhd_error;
    pub fn uhd_usrp_get_tx_gain(h: uhd_usrp_handle, chan: usize, name: *const c_char, out: *mut f64) -> uhd_error;
    pub fn uhd_usrp_set_tx_antenna(h: uhd_usrp_handle, ant: *const c_char, chan: usize) -> uhd_error;

    pub fn uhd_rx_streamer_make(h: *mut uhd_rx_streamer_handle) -> uhd_error;
    pub fn uhd_rx_streamer_free(h: *mut uhd_rx_streamer_handle) -> uhd_error;
    pub fn uhd_usrp_get_rx_stream(
        h: uhd_usrp_handle,
        args: *mut uhd_stream_args_t,
        out: uhd_rx_streamer_handle,
    ) -> uhd_error;
    pub fn uhd_rx_streamer_max_num_samps(h: uhd_rx_streamer_handle, out: *mut usize) -> uhd_error;
    pub fn uhd_rx_streamer_recv(
        h: uhd_rx_streamer_handle,
        buffs: *mut *mut c_void,
        samps_per_buff: usize,
        md: *mut uhd_rx_metadata_handle,
        timeout: f64,
        one_packet: bool,
        items_recvd: *mut usize,
    ) -> uhd_error;
    pub fn uhd_rx_streamer_issue_stream_cmd(h: uhd_rx_streamer_handle, cmd: *const uhd_stream_cmd_t) -> uhd_error;
    pub fn uhd_rx_streamer_last_error(h: uhd_rx_streamer_handle, out: *mut c_char, len: usize) -> uhd_error;

    pub fn uhd_rx_metadata_make(h: *mut uhd_rx_metadata_handle) -> uhd_error;
    pub fn uhd_rx_metadata_free(h: *mut uhd_rx_metadata_handle) -> uhd_error;
    pub fn uhd_rx_metadata_has_time_spec(h: uhd_rx_metadata_handle, out: *mut bool) -> uhd_error;
    pub fn uhd_rx_metadata_time_spec(h: uhd_rx_metadata_handle, full: *mut i64, frac: *mut f64) -> uhd_error;
    pub fn uhd_rx_metadata_error_code(h: uhd_rx_metadata_handle, out: *mut c_int) -> uhd_error;
    pub fn uhd_rx_metadata_out_of_sequence(h: uhd_rx_metadata_handle, out: *mut bool) -> uhd_error;
    pub fn uhd_rx_metadata_strerror(h: uhd_rx_metadata_handle, out: *mut c_char, len: usize) -> uhd_error;

    pub fn uhd_tx_streamer_make(h: *mut uhd_tx_streamer_handle) -> uhd_error;
    pub fn uhd_tx_streamer_free(h: *mut uhd_tx_streamer_handle) -> uhd_error;
    pub fn uhd_usrp_get_tx_stream(
        h: uhd_usrp_handle,
        args: *mut uhd_stream_args_t,
        out: uhd_tx_streamer_handle,
    ) -> uhd_error;
    pub fn uhd_tx_streamer_max_num_samps(h: uhd_tx_streamer_handle, out: *mut usize) -> uhd_error;
    pub fn uhd_tx_streamer_send(
        h: uhd_tx_streamer_handle,
        buffs: *mut *const c_void,
        samps_per_buff: usize,
        md: *mut uhd_tx_metadata_handle,
        timeout: f64,
        items_sent: *mut usize,
    ) -> uhd_error;
    pub fn uhd_tx_streamer_recv_async_msg(
        h: uhd_tx_streamer_handle,
        md: *mut uhd_async_metadata_handle,
        timeout: f64,
        valid: *mut bool,
    ) -> uhd_error;
    pub fn uhd_tx_streamer_last_error(h: uhd_tx_streamer_handle, out: *mut c_char, len: usize) -> uhd_error;

    pub fn uhd_tx_metadata_make(
        h: *mut uhd_tx_metadata_handle,
        has_time_spec: bool,
        full: i64,
        frac: f64,
        start_of_burst: bool,
        end_of_burst: bool,
    ) -> uhd_error;
    pub fn uhd_tx_metadata_free(h: *mut uhd_tx_metadata_handle) -> uhd_error;

    pub fn uhd_async_metadata_make(h: *mut uhd_async_metadata_handle) -> uhd_error;
    pub fn uhd_async_metadata_free(h: *mut uhd_async_metadata_handle) -> uhd_error;
    pub fn uhd_async_metadata_channel(h: uhd_async_metadata_handle, out: *mut usize) -> uhd_error;
    pub fn uhd_async_metadata_has_time_spec(h: uhd_async_metadata_handle, out: *mut bool) -> uhd_error;
    pub fn uhd_async_metadata_time_spec(h: uhd_async_metadata_handle, full: *mut i64, frac: *mut f64) -> uhd_error;
    pub fn uhd_async_metadata_event_code(h: uhd_async_metadata_handle, out: *mut c_int) -> uhd_error;
}
