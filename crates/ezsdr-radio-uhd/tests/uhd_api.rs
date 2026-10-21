//! UR-3 against libuhd itself, with no device attached (feature `uhd`).
#![cfg(feature = "uhd")]

static UHD_API_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn uhd_api_find_returns() {
    let _test = UHD_API_TEST_LOCK.lock().unwrap();
    // An empty list on a machine with no USRP; the call itself must not fail.
    let found = ezsdr_radio_uhd::uhd_find("").unwrap();
    println!("uhd_usrp_find: {found:?}");
}

#[test]
fn uhd_api_open_of_an_absent_device_is_an_error_with_uhd_s_text() {
    let _test = UHD_API_TEST_LOCK.lock().unwrap();
    // 192.0.2.1 is TEST-NET-1 (RFC 5737): nothing answers there.
    let error = ezsdr_radio_uhd::open("addr=192.0.2.1").err().expect("no device answers");
    assert!(error.starts_with("uhd_usrp_make: UHD error "), "{error}");
    let text = error.split_once(": ").and_then(|(_, rest)| rest.split_once(": ")).map_or("", |(_, text)| text);
    assert!(!text.trim().is_empty(), "UHD's own text: {error}");
}

#[test]
fn uhd_api_struct_sizes_match_the_headers() {
    let _test = UHD_API_TEST_LOCK.lock().unwrap();
    // Measured with `cc` against UHD 4.10.0's headers on arm64 macOS (LP64); x86_64
    // Linux lays the six out alike (`uhd/usrp/usrp.h`, `uhd/types/ranges.h`,
    // `uhd/types/usrp_info.h`).
    assert_eq!(
        ezsdr_radio_uhd::uhd_struct_sizes(),
        [
            ("uhd_stream_args_t", 40), ("uhd_stream_cmd_t", 40), ("uhd_tune_request_t", 48),
            ("uhd_tune_result_t", 40), ("uhd_range_t", 24), ("uhd_usrp_rx_info_t", 64),
        ]
    );
}

#[test]
fn uhd_api_a_streamer_is_freed_once() {
    let _test = UHD_API_TEST_LOCK.lock().unwrap();
    // `rx_open`'s and `tx_open`'s own path, without a device: each handle's make/free runs,
    // and UHD's own free stores NULL in the slot it is given (`usrp_c.cpp:97-101`), so the
    // freed state is what a second free through that slot would find (a copy of a handle
    // freed twice still aborts).
    for _ in 0..3 {
        ezsdr_radio_uhd::uhd_streamer_lifecycle().unwrap();
    }
}

#[test]
fn uhd_api_a_refused_subdev_spec_is_freed_once() {
    let _test = UHD_API_TEST_LOCK.lock().unwrap();
    // `set_subdev_spec`'s own path, without a device (UR-3), against real libuhd: a three-part
    // token is refused with `uhd_subdev_spec_make`'s error and UHD's own text, and an accepted
    // markup's path (make, then free) runs. The frees themselves leave no witness here —
    // `uhd_subdev_spec_free` stores NULL, so a second free through the same slot deletes
    // nothing (`subdev_spec_c.cpp:41`) —; the count each path frees is pinned by the replaced
    // calls of `src/uhd.rs`'s `uhd_a_subdev_spec_is_freed_once_on_every_path` (feature `uhd`).
    for _ in 0..3 {
        let error = ezsdr_radio_uhd::uhd_subdev_spec_lifecycle("A:0:1").expect_err("a three-part token is refused");
        assert!(error.message.starts_with("uhd_subdev_spec_make: UHD error "), "{}", error.message);
        assert!(error.message.contains("invalid subdev-spec markup string"), "{}", error.message);
        ezsdr_radio_uhd::uhd_subdev_spec_lifecycle("A:0 B:1").unwrap();
    }
}
