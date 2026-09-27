//! UR-3 against libuhd itself, with no device attached (feature `uhd`).
#![cfg(feature = "uhd")]

#[test]
fn uhd_api_find_returns() {
    // An empty list on a machine with no USRP; the call itself must not fail.
    let found = ezsdr_radio_uhd::uhd_find("").unwrap();
    println!("uhd_usrp_find: {found:?}");
}

#[test]
fn uhd_api_open_of_an_absent_device_is_an_error_with_uhd_s_text() {
    // 192.0.2.1 is TEST-NET-1 (RFC 5737): nothing answers there.
    let error = ezsdr_radio_uhd::open("addr=192.0.2.1").err().expect("no device answers");
    assert!(error.starts_with("uhd_usrp_make: UHD error "), "{error}");
    let text = error.split_once(": ").and_then(|(_, rest)| rest.split_once(": ")).map_or("", |(_, text)| text);
    assert!(!text.trim().is_empty(), "UHD's own text: {error}");
}

#[test]
fn uhd_api_struct_sizes_match_the_headers() {
    // Measured with `cc` against UHD 4.10.0's headers on arm64 macOS (LP64); x86_64
    // Linux lays the four out alike.
    assert_eq!(
        ezsdr_radio_uhd::uhd_struct_sizes(),
        [("uhd_stream_args_t", 40), ("uhd_stream_cmd_t", 40), ("uhd_tune_request_t", 48), ("uhd_tune_result_t", 40)]
    );
}

#[test]
fn uhd_api_a_streamer_is_freed_once() {
    // `rx_open`'s and `tx_open`'s own path, without a device: a double free aborts.
    for _ in 0..3 {
        ezsdr_radio_uhd::uhd_streamer_lifecycle().unwrap();
    }
}
