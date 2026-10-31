//! X3xx rate-grid and UHD rate-set behavior (design 33 D5).

use ezsdr_radio::device::{Grid, rate_set_matches};
use ezsdr_radio::x3xx;

#[test]
fn x3xx_the_decimation_set_is_the_ddc_s() {
    // 639 values; `{1}` is in it, the CIC's 255 and three halfbands' 2040 are, and one
    // past the halfbands (4080) and an odd CIC value above 255 (257) are not (design 33 D5).
    let set = x3xx::decimations();
    assert_eq!(set.len(), 639);
    assert_eq!(set[0], 1);
    assert!(x3xx::is_decimation(1) && x3xx::is_decimation(255) && x3xx::is_decimation(256) && x3xx::is_decimation(2040));
    assert!(!x3xx::is_decimation(257) && !x3xx::is_decimation(2041) && !x3xx::is_decimation(4080));
    // The rate grid is ascending and exactly `200e6 / N` (RM-8's `Values`).
    let rates = x3xx::rates(200_000_000);
    assert_eq!(rates.len(), 639);
    assert_eq!(rates[0], 200_000_000.0 / 2040.0);
    assert_eq!(*rates.last().unwrap(), 200_000_000.0);
    assert!(rates.windows(2).all(|pair| pair[0] < pair[1]));
    // `decimation` is exact and refuses what the DDC cannot make; `clip_decimation` is
    // UHD's `coerce_decim` (UR-12, UR-33).
    assert_eq!(x3xx::decimation(200_000_000, 200e6 / 3.0), Some(3));
    assert_eq!(x3xx::decimation(200_000_000, 19.5e6), None);
    assert_eq!(x3xx::decimation(200_000_000, 200e6 / 257.0), None);
    assert_eq!(x3xx::decimation(200_000_000, 200e6 / 2040.0), Some(2040));
    assert_eq!(x3xx::clip_decimation(200_000_000, 200e6 / 257.0), 256);
    assert_eq!(x3xx::clip_decimation(200_000_000, 200e6 / 257.4), 258);
    assert_eq!(x3xx::clip_decimation(200_000_000, 200e6 / 4080.0), 2040);
}

#[test]
fn ur_13_the_rate_set_comparison_expands_ranges_and_ignores_order() {
    // UR-13, design 33 D5: the device's `(start, stop, step)` ranges are expanded and
    // compared as a set, in any order; a range is the values UHD would enumerate, and an
    // interval that is not one value ('step' 0, or start == stop as one) is handled.
    let values = Grid::Values(vec![1.0, 2.5, 4.0]);
    assert!(rate_set_matches(&values, &[(1.0, 1.0, 0.0), (2.5, 2.5, 0.0), (4.0, 4.0, 0.0)]));
    assert!(rate_set_matches(&values, &[(4.0, 4.0, 0.0), (1.0, 2.5, 1.5)]), "a range expands");
    assert!(rate_set_matches(&values, &[(4.0, 4.0, 0.0), (2.5, 2.5, 0.0), (1.0, 1.0, 0.0)]), "the order does not matter");
    assert!(!rate_set_matches(&values, &[(1.0, 1.0, 0.0), (2.5, 2.5, 0.0)]), "a missing value");
    assert!(!rate_set_matches(&values, &[(1.0, 2.5, 1.5), (4.0, 4.0, 0.0), (9.0, 9.0, 0.0)]), "an extra value");
    assert!(!rate_set_matches(&values, &[(1.0, 4.0, 1.0)]), "a different set");
    assert!(rate_set_matches(&Grid::Step { lo: 1.0, hi: 4.0, step: 1.5 }, &[(1.0, 2.5, 1.5), (4.0, 4.0, 0.0)]));
    assert!(!rate_set_matches(&Grid::Step { lo: 0.0, hi: 1.0, step: 0.0 }, &[(1.0, 1.0, 0.0)]), "an interval is not a finite set");
}
