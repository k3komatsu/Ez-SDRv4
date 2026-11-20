//! X3xx rate-grid and UHD rate-set behavior (design 33 D5).

use ezsdr_radio::device::{Grid, rate_set_matches};
use ezsdr_radio::x3xx::{self, Board, FrontEnd};

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
fn rx_fill_samples_is_the_table_by_half_band_count() {
    // MR-35, UR-33: 2 / 13 / 18 / 24 for 0 / 1 / 2 / 3-or-more factors of two in D.
    let cases: [(u64, u64); 17] =
        [(1, 2), (2, 13), (3, 2), (4, 18), (6, 13), (8, 24), (12, 18), (16, 24), (24, 24), (25, 2), (32, 24), (248, 24), (250, 13), (255, 2), (1020, 18), (1024, 24), (2040, 24)];
    for (decimation, head) in cases {
        assert_eq!(x3xx::rx_fill_samples(decimation), head, "D {decimation}");
    }
    // Every decimation of the DDC against a class derived here by halving, the table spelled out apart from the function.
    for &decimation in x3xx::decimations() {
        let (mut odd, mut twos) = (decimation, 0usize);
        while odd % 2 == 0 {
            odd /= 2;
            twos += 1;
        }
        let expected = match twos {
            0 => 2,
            1 => 13,
            2 => 18,
            _ => 24,
        };
        assert_eq!(x3xx::rx_fill_samples(decimation), expected, "D {decimation}");
    }
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

#[test]
fn x3xx_the_transmit_grid_is_the_ddc_s_up_to_decimation_200() {
    // The transmit grid is the top of the device grid: N = 200 (1 MS/s) up to N = 1 (200 MS/s), ascending (a `Values` grid is).
    let tx = x3xx::tx_rates(200_000_000);
    assert_eq!(tx.len(), 200);
    assert_eq!(tx[0], 1_000_000.0);
    assert_eq!(*tx.last().unwrap(), 200_000_000.0);
    assert!(tx.windows(2).all(|pair| pair[0] < pair[1]), "strictly ascending");
    for (i, rate) in tx.iter().enumerate() {
        assert_eq!(*rate, 200e6 / (200 - i) as f64, "N = {}", 200 - i);
    }
    assert_eq!(x3xx::rates(200_000_000).len(), 639, "the device grid is unchanged");
    let grid = Grid::Values(tx);
    assert_eq!((grid.min(), grid.max()), (1_000_000.0, 200_000_000.0));
    assert_eq!(grid.nearest(1_000_000.0), Some(1_000_000.0));
    assert_eq!(grid.nearest(200_000_000.0), Some(200_000_000.0));
    assert_eq!(grid.nearest(500_000.0), None);
}

/// RM-8: the grids the tests below walk, one of each kind. The `Step` grids are the multiples of `step` inside `[lo, hi]` (not from `lo`).
fn grids() -> [(&'static str, Grid); 4] {
    [
        ("values", Grid::Values(vec![1.0, 2.5, 4.0])),
        ("integer", Grid::Integer { lo: 2, hi: 5 }),
        ("continuous", Grid::Step { lo: 1.0, hi: 4.0, step: 0.0 }),
        ("step", Grid::Step { lo: 3.0, hi: 9.0, step: 1.5 }),
    ]
}

#[test]
fn rm_08_a_grid_contains_its_values_and_both_ends_and_nothing_else() {
    // (grid, value, contained): the ends are in, a hair outside is out, and a value that is not finite is out of every grid, the continuous one included.
    let cases: [(&str, f64, bool); 33] = [
        ("values", 1.0, true),
        ("values", 2.5, true),
        ("values", 4.0, true),
        ("values", 2.0, false),
        ("values", 0.9, false),
        ("values", 4.1, false),
        ("integer", 2.0, true),
        ("integer", 5.0, true),
        ("integer", 3.0, true),
        ("integer", 2.5, false),
        ("integer", 1.0, false),
        ("integer", 6.0, false),
        ("continuous", 1.0, true),
        ("continuous", 2.7, true),
        ("continuous", 4.0, true),
        ("continuous", 0.9, false),
        ("continuous", 4.1, false),
        ("step", 3.0, true),
        ("step", 4.5, true),
        ("step", 9.0, true),
        ("step", 3.75, false),
        ("step", 1.5, false),
        ("step", 10.5, false),
        ("step", 0.0, false),
        ("values", f64::NAN, false),
        ("integer", f64::NAN, false),
        ("continuous", f64::NAN, false),
        ("step", f64::NAN, false),
        ("continuous", f64::INFINITY, false),
        ("continuous", f64::NEG_INFINITY, false),
        ("step", f64::INFINITY, false),
        ("integer", f64::INFINITY, false),
        ("values", f64::NEG_INFINITY, false),
    ];
    let grids = grids();
    for (name, value, expected) in cases {
        let grid = &grids.iter().find(|(n, _)| *n == name).unwrap().1;
        assert_eq!(grid.contains(value), expected, "{name} contains {value}");
    }
}

#[test]
fn rm_08_nearest_rounds_to_the_grid_with_ties_to_the_lower_and_is_none_off_either_end() {
    let nearest = |name: &str, value: f64| grids().into_iter().find(|(n, _)| *n == name).unwrap().1.nearest(value);
    // A `Values` grid: the nearest member, a tie to the lower, the ends themselves, nothing past them.
    assert_eq!(nearest("values", 1.0), Some(1.0));
    assert_eq!(nearest("values", 1.7), Some(1.0));
    assert_eq!(nearest("values", 1.8), Some(2.5));
    assert_eq!(nearest("values", 4.0), Some(4.0));
    assert_eq!(Grid::Values(vec![1.0, 2.0]).nearest(1.5), Some(1.0), "a tie goes to the lower");
    for outside in [0.9, 4.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(nearest("values", outside), None, "{outside}");
    }
    // An `Integer` grid keeps an integer and refuses a fraction (it rounds nothing).
    assert_eq!(nearest("integer", 3.0), Some(3.0));
    assert_eq!(nearest("integer", 2.0), Some(2.0));
    assert_eq!(nearest("integer", 5.0), Some(5.0));
    assert_eq!(nearest("integer", 2.5), None);
    assert_eq!(nearest("integer", 6.0), None);
    // A continuous `Step` grid (step 0) is its interval.
    assert_eq!(nearest("continuous", 2.7), Some(2.7));
    assert_eq!(nearest("continuous", 4.1), None);
    // A stepped grid: the nearest multiple, a tie to the lower, a multiple that falls outside `[lo, hi]` is none.
    let unit = Grid::Step { lo: 0.0, hi: 10.0, step: 1.0 };
    assert_eq!((unit.nearest(2.4), unit.nearest(2.5), unit.nearest(2.6)), (Some(2.0), Some(2.0), Some(3.0)));
    assert_eq!((unit.nearest(0.0), unit.nearest(10.0)), (Some(0.0), Some(10.0)));
    assert_eq!(Grid::Step { lo: 2.2, hi: 2.4, step: 1.0 }.nearest(2.3), None, "its nearest multiple, 2, is below lo");
    assert_eq!(Grid::Step { lo: 2.6, hi: 2.8, step: 1.0 }.nearest(2.7), None, "its nearest multiple, 3, is above hi");
    assert_eq!(nearest("step", 4.4), Some(4.5));
    assert_eq!(nearest("step", 3.0), Some(3.0));
    assert_eq!(nearest("step", 9.0), Some(9.0));
}

#[test]
fn rm_08_at_or_above_and_at_or_below_return_the_bound_itself_when_it_is_on_the_grid() {
    // (grid, bound, least value at or above it, greatest value at or below it): the bound on the grid is both; between two values, the one on each side; outside the grid, the end on one
    // side and none on the other.
    let cases: [(&str, f64, Option<f64>, Option<f64>); 28] = [
        ("values", 1.0, Some(1.0), Some(1.0)),
        ("values", 1.1, Some(2.5), Some(1.0)),
        ("values", 3.9, Some(4.0), Some(2.5)),
        ("values", 4.0, Some(4.0), Some(4.0)),
        ("values", 0.0, Some(1.0), None),
        ("values", 5.0, None, Some(4.0)),
        ("integer", 2.0, Some(2.0), Some(2.0)),
        ("integer", 2.1, Some(3.0), Some(2.0)),
        ("integer", 4.9, Some(5.0), Some(4.0)),
        ("integer", 5.0, Some(5.0), Some(5.0)),
        ("integer", 1.5, Some(2.0), None),
        ("integer", 5.5, None, Some(5.0)),
        ("continuous", 1.0, Some(1.0), Some(1.0)),
        ("continuous", 2.5, Some(2.5), Some(2.5)),
        ("continuous", 4.0, Some(4.0), Some(4.0)),
        ("continuous", 0.0, Some(1.0), None),
        ("continuous", 5.0, None, Some(4.0)),
        ("continuous", 0.9, Some(1.0), None),
        ("continuous", 4.1, None, Some(4.0)),
        ("step", 3.0, Some(3.0), Some(3.0)),
        ("step", 3.1, Some(4.5), Some(3.0)),
        ("step", 4.4, Some(4.5), Some(3.0)),
        ("step", 4.5, Some(4.5), Some(4.5)),
        ("step", 8.9, Some(9.0), Some(7.5)),
        ("step", 9.0, Some(9.0), Some(9.0)),
        ("step", 2.9, Some(3.0), None),
        ("step", 9.1, None, Some(9.0)),
        ("step", 0.0, Some(3.0), None),
    ];
    let grids = grids();
    for (name, bound, above, below) in cases {
        let grid = &grids.iter().find(|(n, _)| *n == name).unwrap().1;
        assert_eq!(grid.at_or_above(bound), above, "{name} at or above {bound}");
        assert_eq!(grid.at_or_below(bound), below, "{name} at or below {bound}");
    }
    assert_eq!(grids.iter().find(|(n, _)| *n == "step").unwrap().1.at_or_below(10.0), Some(9.0));
    for (name, grid) in &grids {
        for bound in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!((grid.at_or_above(bound), grid.at_or_below(bound)), (None, None), "{name} {bound}");
        }
    }
}

#[test]
fn x3xx_decimation_is_exact_and_refuses_what_is_off_the_grid_or_not_a_rate() {
    let mcr = 200_000_000;
    assert_eq!(x3xx::decimation(mcr, 200e6), Some(1), "the master clock itself is decimation 1");
    assert_eq!(x3xx::decimation(mcr, 100e6), Some(2));
    assert_eq!(x3xx::decimation(mcr, 1e6), Some(200));
    // A hair off a grid rate is off the grid (the comparison is exact), and a rate above the master clock needs a decimation below 1.
    assert_eq!(x3xx::decimation(mcr, 200e6 / 3.0 * (1.0 + 1e-12)), None);
    assert_eq!(x3xx::decimation(mcr, 400e6), None);
    assert_eq!(x3xx::decimation(mcr, 1e9), None);
    for not_a_rate in [0.0, -1e6, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(x3xx::decimation(mcr, not_a_rate), None, "{not_a_rate}");
    }
}

#[test]
fn rm_28_a_front_end_receives_or_transmits_by_its_kind() {
    // (front end, receives, transmits): the three duplex boards do both, the Basic and LF boards one each.
    for (front_end, receives, transmits) in [
        (FrontEnd::Ubx, true, true),
        (FrontEnd::Obx, true, true),
        (FrontEnd::Cbx, true, true),
        (FrontEnd::BasicRx, true, false),
        (FrontEnd::LfRx, true, false),
        (FrontEnd::BasicTx, false, true),
        (FrontEnd::LfTx, false, true),
    ] {
        assert_eq!((front_end.receives(), front_end.transmits()), (receives, transmits), "{}", front_end.name());
    }
}

#[test]
fn ur_05_a_board_is_found_by_the_start_of_its_uhd_front_end_name() {
    assert_eq!([Board::Ubx, Board::Obx, Board::Cbx].map(Board::uhd_name), ["UBX", "OBX", "CBX"]);
    // UHD's names are `<kind> RX`, `<kind> TX` and `CBX-120 TX`; a name is no board's when none of the three starts it, and the empty name starts none.
    for (name, board) in [("UBX RX", Some(Board::Ubx)), ("OBX TX", Some(Board::Obx)), ("CBX-120 TX", Some(Board::Cbx)), ("CBX", Some(Board::Cbx)), ("LFRX", None), ("Basic RX", None), ("", None)] {
        assert_eq!(Board::of_front_end(name), board, "{name:?}");
    }
}
