//! UR-22, UR-23, MR-15, MR-25 (note 49 §5.4, E3a): `ezsdr_radio::segment`, the pure segmentation decisions of transmit. Each test is one table of
//! inputs and results written out from the rules, never read from a Provider.

use ezsdr_radio::segment::{Extent, Pending, Plan, SEND_CAP_MAX, Waveform, extent, handed_end, plan_send, send_cap};
use ezsdr_radio::staging::{Bookings, CLOSE_GAP, HELD_TAIL_FRONT, Overlap};
use ezsdr_radio::x3xx::{PACKETS, PacketRules};

const LEAD: i64 = 400_000;
const CHUNK: i64 = 2_500;

fn finite(len: i64) -> Waveform {
    Waveform { start: 0, len, repeat: false, tiled: false }
}

fn repeat(len: i64) -> Waveform {
    Waveform { start: 0, len, repeat: true, tiled: false }
}

fn tiled(len: i64) -> Waveform {
    Waveform { start: 0, len, repeat: true, tiled: true }
}

fn ext(waveform: Waveform, next: i64, held_cut: Option<i64>) -> Extent {
    extent(waveform, next, CHUNK, held_cut, None).expect("a send is left")
}

/// A send of `ext` with `avail` samples of window, no hold unless `open_ended` (the stream's clock is a constant: the end of the send is
/// either past the lead or not).
fn plan_with(rules: &PacketRules, ext: &Extent, device_sob: bool, avail: i64, open_ended: bool) -> Option<Plan> {
    plan_send(rules, ext, device_sob, ext.next + avail, 0, LEAD, |_| if open_ended { LEAD + 1 } else { LEAD })
}

fn plan(ext: &Extent, device_sob: bool, avail: i64, open_ended: bool) -> Option<Plan> {
    plan_with(&PACKETS, ext, device_sob, avail, open_ended)
}

fn count_of(ext: &Extent, device_sob: bool, avail: i64) -> Option<i64> {
    plan(ext, device_sob, avail, false).map(|plan| plan.count)
}

#[test]
fn e3_p_s_send_cap() {
    // A quarter of the window in samples, between one first packet and 20 000.
    assert_eq!(send_cap(2_000_000, 200, &PACKETS), 2_500);
    assert_eq!(send_cap(2_000_000, 1, &PACKETS), SEND_CAP_MAX, "the upper clamp");
    assert_eq!(send_cap(200_000, 200, &PACKETS), 250);
    assert_eq!(send_cap(20_000, 200, &PACKETS), 32, "the lower clamp");
    assert_eq!(send_cap(0, 200, &PACKETS), 32);
    assert_eq!(send_cap(2_000_000, 0, &PACKETS), SEND_CAP_MAX, "no rate is not a division by zero");
}

#[test]
fn e3_p_e_extent() {
    // finite 100: from its start, and from 40
    let e = ext(finite(100), 0, None);
    assert_eq!((e.offset, e.wanted, e.ends_waveform(e.wanted), e.ends_at_held(e.wanted)), (0, 100, true, false));
    let e = ext(finite(100), 40, None);
    assert_eq!((e.offset, e.wanted, e.ends_waveform(e.wanted), e.ends_at_held(e.wanted)), (40, 60, true, false));
    // finite 5 000 in two chunks
    let e = ext(finite(5_000), 0, None);
    assert_eq!((e.offset, e.wanted, e.ends_waveform(e.wanted)), (0, 2_500, false));
    let e = ext(finite(5_000), 2_500, None);
    assert_eq!((e.offset, e.wanted, e.ends_waveform(e.wanted)), (2_500, 2_500, true));
    // an untiled repeat stops at its wrap, which is no end
    let e = ext(repeat(1_000), 1_500, None);
    assert_eq!((e.offset, e.wanted, e.ends_waveform(e.wanted), e.ends_at_held(e.wanted)), (500, 500, false, false));
    // a tiled repeat crosses its wraps and keeps its phase
    let e = ext(tiled(7), 10, None);
    assert_eq!((e.offset, e.wanted, e.ends_waveform(e.wanted)), (3, 2_500, false));
    let e = ext(Waveform { start: 100, len: 30, repeat: true, tiled: true }, 150, None);
    assert_eq!((e.offset, e.wanted), (20, 2_500));
    // the next held start cuts the send, and the Kernel burst ends there
    let e = ext(finite(100), 0, Some(60));
    assert_eq!((e.wanted, e.ends_waveform(e.wanted), e.ends_at_held(e.wanted)), (60, false, true));
    let e = ext(finite(100), 0, Some(100));
    assert_eq!((e.wanted, e.ends_waveform(e.wanted), e.ends_at_held(e.wanted)), (100, true, true), "a cut at the waveform's end is both");
    let e = ext(finite(100), 0, Some(150));
    assert_eq!((e.wanted, e.ends_waveform(e.wanted), e.ends_at_held(e.wanted)), (100, true, false), "a cut past the end ends nothing");
    let e = ext(repeat(1_000), 0, Some(7));
    assert_eq!((e.wanted, e.ends_waveform(e.wanted), e.ends_at_held(e.wanted)), (7, false, true));
    assert_eq!(extent(finite(100), 10, CHUNK, Some(10), None), None, "a cut at the next sample leaves nothing");
    assert_eq!(extent(finite(100), 10, CHUNK, Some(3), None), None);
    // a hard cut (the Mock's clock or Stop) ends no Kernel burst
    let e = extent(finite(100), 0, CHUNK, Some(60), Some(30)).unwrap();
    assert_eq!((e.wanted, e.ends_waveform(e.wanted), e.ends_at_held(e.wanted)), (30, false, false));
    let e = extent(finite(100), 0, CHUNK, Some(60), Some(80)).unwrap();
    assert_eq!((e.wanted, e.ends_at_held(e.wanted)), (60, true));
    let e = extent(finite(100), 0, CHUNK, Some(60), Some(60)).unwrap();
    assert_eq!((e.wanted, e.ends_at_held(e.wanted)), (60, true));
    let e = extent(finite(100), 0, CHUNK, None, Some(40)).unwrap();
    assert_eq!((e.wanted, e.ends_waveform(e.wanted), e.ends_at_held(e.wanted)), (40, false, false));
    assert_eq!(extent(finite(100), 0, CHUNK, None, Some(0)), None);
    // nothing left, and no panic at the end of the clock
    assert_eq!(extent(finite(100), 100, CHUNK, None, None), None);
    assert_eq!(extent(finite(0), 0, CHUNK, None, None), None);
    assert_eq!(extent(finite(100), i64::MAX, CHUNK, None, None), None);
    let e = extent(tiled(1_000), i64::MAX - 5, CHUNK, Some(i64::MAX), None).unwrap();
    assert_eq!(e.wanted, 5);
    assert!(e.ends_at_held(5));
    assert!(extent(repeat(1_000), i64::MAX, CHUNK, None, None).is_some());
}

#[test]
fn e3_p_w_the_window_wait_and_count() {
    // a device burst's first send waits for 32 samples of window, or all it wants when that is less
    let long = ext(finite(10_000), 0, None);
    for (avail, want) in [(28, None), (31, None), (32, Some(32)), (2_500, Some(2_500)), (3_000, Some(2_500))] {
        assert_eq!(count_of(&long, true, avail), want, "a first send of 10 000 with {avail} samples of window");
    }
    for len in [1, 3, 31, 32] {
        let e = ext(finite(len), 0, None);
        assert_eq!(count_of(&e, true, len - 1), None, "a burst of {len} waits for all {len}");
        assert_eq!(count_of(&e, true, len), Some(len));
    }
    let e = ext(finite(33), 0, None);
    assert_eq!(count_of(&e, true, 31), None);
    assert_eq!(count_of(&e, true, 32), Some(32), "a burst of 33 sends its first 32");
    // a tiled repeat's first send
    let e = ext(tiled(5), 0, None);
    for (avail, want) in [(31, None), (32, Some(32)), (1_000, Some(1_000)), (2_500, Some(2_500))] {
        assert_eq!(count_of(&e, true, avail), want, "a tiled repeat with {avail}");
    }
    // a continuation waits for all it wants
    for (avail, want) in [(2_499, None), (2_500, Some(2_500)), (3_000, Some(2_500))] {
        assert_eq!(count_of(&long, false, avail), want, "a continuation of 2 500 with {avail}");
    }
    let two = ext(finite(100), 98, None);
    assert_eq!((count_of(&two, false, 1), count_of(&two, false, 2)), (None, Some(2)), "a continuation of two samples");
    // a first send cut by a held start to 20 waits for those 20
    let cut = ext(finite(100), 0, Some(20));
    assert_eq!((count_of(&cut, true, 19), count_of(&cut, true, 20)), (None, Some(20)));
    // no window (W = 0) hands nothing over, never a panic
    for (case, e) in [("finite", &long), ("tiled", &ext(tiled(5), 0, None)), ("two", &two), ("cut", &cut)] {
        for avail in [0, -5] {
            assert_eq!((count_of(e, true, avail), count_of(e, false, avail)), (None, None), "{case} with {avail}");
        }
    }
}

fn one(plan: Option<Plan>) -> Plan {
    plan.expect("a send")
}

#[allow(clippy::too_many_arguments)]
fn p(count: i64, eob: bool, short_first: bool, hold: bool, device_eob: bool, sending: i64, pad: i64, closes_short: bool) -> Plan {
    Plan { count, eob, short_first, hold, device_eob, sending, pad, device_count: sending + pad, closes_short }
}

#[test]
fn e3_p_f_flags_and_padding() {
    const BIG: i64 = 10_000;
    // a lone burst of 1..=4 samples is one closed device burst, padded to four, with or without a booking in its lead
    for open_ended in [false, true] {
        for (len, pad) in [(1, 3), (2, 2), (3, 1), (4, 0)] {
            let got = one(plan(&ext(finite(len), 0, None), true, BIG, open_ended));
            assert_eq!(got, p(len, true, true, false, true, len, pad, true), "a lone burst of {len}, open_ended {open_ended}");
            assert_eq!(got.device_count, 4);
        }
        // 5, 31 and 32 samples: one closed send, never held, never padded
        for len in [5, 31, 32] {
            assert_eq!(one(plan(&ext(finite(len), 0, None), true, BIG, open_ended)), p(len, true, true, false, true, len, 0, true), "a lone burst of {len}, open_ended {open_ended}");
        }
    }
    // 33 samples: the first 32 now and the last held, when no booking can follow in time; whole when one can
    assert_eq!(one(plan(&ext(finite(33), 0, None), true, BIG, true)), p(33, true, false, true, false, 32, 0, false));
    assert_eq!(one(plan(&ext(finite(33), 0, None), true, BIG, false)), p(33, true, false, false, true, 33, 0, false));
    // 33 samples with a window of 32: the first 32 as a closed-length send that does not close
    assert_eq!(one(plan(&ext(finite(33), 0, None), true, 32, true)), p(32, false, true, false, false, 32, 0, false));
    assert_eq!(one(plan(&ext(finite(40), 0, None), true, BIG, false)), p(40, true, false, false, true, 40, 0, false), "a closing 40 fronts at its end plus three");
    // a continuation that closes: one sample is padded to two, two is sent as it is, and a lead away its last sample is held
    assert_eq!(one(plan(&ext(finite(100), 99, None), false, BIG, false)), p(1, true, false, false, true, 1, 1, false));
    assert_eq!(one(plan(&ext(finite(100), 98, None), false, BIG, false)), p(2, true, false, false, true, 2, 0, false));
    assert_eq!(one(plan(&ext(finite(100), 99, None), false, BIG, true)), p(1, true, false, true, false, 0, 0, false));
    // a Kernel end at a held start: the device burst goes on; a short first send closes, as UR-22 says
    assert_eq!(one(plan(&ext(finite(100), 0, Some(50)), true, BIG, true)), p(50, true, false, false, false, 50, 0, false));
    assert_eq!(one(plan(&ext(finite(100), 0, Some(20)), true, BIG, true)), p(20, true, true, false, true, 20, 0, true));
    assert_eq!(one(plan(&ext(repeat(1_000), 0, Some(100)), true, BIG, true)), p(100, true, false, false, false, 100, 0, false), "a repeat cut at a switch boundary");
    assert_eq!(one(plan(&ext(repeat(1_000), 0, None), true, BIG, true)), p(1_000, false, false, false, false, 1_000, 0, false), "a wrap ends nothing");
    // the sample counts are per channel: one plan serves any number of channels (the callers multiply nothing)
    assert_eq!(one(plan(&ext(finite(2), 0, None), true, BIG, false)).device_count, 4);
}

#[test]
fn e3_p_o_the_hold_deadline() {
    // a burst of 100 at 5 000 ends at 5 100, a lead away exactly when `now` is 620 000 root ticks (200 a sample)
    let e = ext(Waveform { start: 5_000, len: 100, repeat: false, tiled: false }, 5_000, None);
    let at = |now: i64| one(plan_send(&PACKETS, &e, true, 5_000 + 10_000, now, LEAD, |k| k * 200));
    assert!(!at(620_000).hold, "exactly a lead away: no hold, a direct close");
    assert!(at(620_000).device_eob);
    assert!(at(619_999).hold, "one tick more than a lead: held");
    assert!(!at(619_999).device_eob);
    assert!(!at(620_001).hold, "less than a lead: no hold");
    assert!(!at(1_000_000).hold, "an end already past is not held");
    assert!(at(0).hold);
}

fn pending(start: i64, len: i64, repeat: bool, open: bool) -> Pending {
    Pending { start, len, repeat, open }
}

const WE: i64 = 15_000;

fn h(start: i64, len: i64, repeat: bool, open: bool) -> Option<i64> {
    handed_end(&PACKETS, pending(start, len, repeat, open), WE)
}

#[test]
fn e3_p_h_handed_end() {
    // the open burst has begun: one sample of window suffices, and it takes what fits
    assert_eq!(h(5_000, 1_000, true, true), Some(WE), "an open repeat to the window's edge");
    assert_eq!(h(WE - 5, 1_000, true, true), Some(WE));
    assert_eq!(h(WE - 5, 100, false, true), Some(WE));
    assert_eq!(h(WE, 1_000, true, true), None, "an open booking at the edge has no sample of window");
    assert_eq!(h(WE, 100, false, true), None);
    assert_eq!(h(6_000, 100, false, true), Some(6_100), "a finite end inside the window is its own");
    // a held repeat's first send is 32 samples, whatever its length
    assert_eq!(h(WE - 32, 1_000, true, false), Some(WE));
    assert_eq!(h(WE - 31, 1_000, true, false), None);
    assert_eq!(h(WE - 32, 5, true, false), Some(WE));
    assert_eq!(h(WE - 20, 5, true, false), None, "a repeat of 5 still needs 32, not its length");
    assert_eq!(h(WE - 40, 5, true, false), Some(WE), "and counts to the edge, never to a finite end");
    // a held finite burst's first send is min(len, 32)
    for (len, start, want) in
        [(20, WE - 20, Some(WE)), (20, WE - 19, None), (32, WE - 32, Some(WE)), (32, WE - 31, None), (33, WE - 32, Some(WE)), (33, WE - 31, None), (1, WE - 1, Some(WE)), (1, WE, None)]
    {
        assert_eq!(h(start, len, false, false), want, "a finite burst of {len} at {start}");
    }
    assert_eq!(h(WE - 50, 100, false, false), Some(WE), "a burst that ends past the edge counts to the edge");
    assert_eq!(h(WE - 80, 30, false, false), Some(WE - 50), "and one that ends inside it to its own end");
    // starting at or after the edge: nothing
    assert_eq!(h(WE + 10, 1_000, true, false), None);
    assert_eq!(h(WE + 3, 100, false, false), None);
    // the next held start is not read (F3a: a repeat cut by a switch before 32 samples is still counted from its own first send)
    assert_eq!(h(WE - 20, 1_000, true, false), None);
    assert_eq!(h(WE - 40, 1_000, true, false), Some(WE));
    // W = 0: the edge is the Stop sample, and nothing exceeds it
    for start in 4_900..5_010 {
        for (len, repeat, open) in [(1, false, false), (20, false, true), (100, false, false), (1_000, true, false), (1_000, true, true), (5, true, false)] {
            assert!(handed_end(&PACKETS, pending(start, len, repeat, open), 5_000).is_none_or(|end| end <= 5_000), "start {start}, len {len}");
        }
    }
}

#[test]
fn e3_p_r_the_rules_are_one_value() {
    assert_eq!((PACKETS.min_open_first_send, PACKETS.min_first_send), (32, 4));
    // other values: the wait, the short bound, the gap choice, the pad, the send cap and the handed rule all follow them
    let rules = PacketRules { min_open_first_send: 40, min_first_send: 6 };
    let long = ext(finite(10_000), 0, None);
    let wait = |avail| plan_with(&rules, &long, true, avail, false).map(|plan| plan.count);
    assert_eq!((wait(39), wait(40)), (None, Some(40)), "the first-send wait");
    let lone = |len| one(plan_with(&rules, &ext(finite(len), 0, None), true, 10_000, false));
    assert_eq!((lone(40).short_first, lone(41).short_first), (true, false), "the short bound");
    assert_eq!((lone(40).closes_short, lone(41).closes_short), (true, false), "the gap choice");
    assert_eq!(lone(41).device_count, 41);
    let two = lone(2);
    assert_eq!((two.pad, two.device_count), (4, 6), "the pad");
    assert_eq!(send_cap(0, 200, &rules), 40, "the send cap's floor");
    assert_eq!((handed_end(&rules, pending(WE - 40, 1_000, true, false), WE), handed_end(&rules, pending(WE - 39, 1_000, true, false), WE)), (Some(WE), None));
}

/// The record's front for `head` samples at 0 and, when given, a piece joined at the head's end: the first sample a one-sample booking is not refused at.
fn record_front(gap: i64, head: i64, piece: Option<i64>) -> i64 {
    let build = || {
        let mut book = Bookings::unarmed();
        book.arm(gap);
        book.accept(0, Some(head), 0, |_| i64::MAX, i64::MIN).unwrap();
        if let Some(piece) = piece {
            book.accept(head, Some(piece), 0, |_| i64::MAX, i64::MIN).unwrap();
        }
        book
    };
    let end = head + piece.unwrap_or(0);
    (end + 1..end + 100).find(|&k| build().accept(k, Some(1), 0, |_| i64::MAX, i64::MIN) != Err(Overlap::Booking)).expect("a front")
}

/// The front the device burst's close leaves, as the executor computes it: a direct close at the send's padded end plus the gap, a held tail's
/// close (its sample, two zeros, the close gap) at the held tail's position plus that.
fn close_front(plan: &Plan, end: i64, gap: i64) -> i64 {
    if plan.hold { (end - 1) + 2 + CLOSE_GAP } else { end + plan.pad + if plan.closes_short { gap } else { CLOSE_GAP } }
}

#[test]
fn e3_p_f2_the_close_front_against_the_record() {
    assert_eq!(HELD_TAIL_FRONT, 1 + CLOSE_GAP);
    for gap in [3, 15, 30] {
        for len in 1..=70 {
            for open_ended in [false, true] {
                // a lone head
                let got = one(plan(&ext(finite(len), 0, None), true, 10_000, open_ended));
                assert!(got.device_eob || got.hold, "a lone head closes now or after its held tail");
                let (device, record) = (close_front(&got, len, gap), record_front(gap, len, None));
                assert!(device <= record, "head {len}, gap {gap}, open_ended {open_ended}: the device front {device} outruns the record's {record}");
                if len <= 32 || got.hold {
                    assert_eq!(device, record, "head {len}, gap {gap}, open_ended {open_ended}: a short head and a held close front where the record says");
                }
                // a piece of `len` joined behind a head of 40
                let piece = Waveform { start: 40, len, repeat: false, tiled: false };
                let got = one(plan(&ext(piece, 40, None), false, 10_000, open_ended));
                let (device, record) = (close_front(&got, 40 + len, gap), record_front(gap, 40, Some(len)));
                assert!(device <= record, "piece {len}, gap {gap}, open_ended {open_ended}: the device front {device} outruns the record's {record}");
                if got.hold || len == 1 {
                    assert_eq!(device, record, "piece {len}: a held or one-sample close fronts where the record says");
                }
            }
        }
    }
}
