//! RM-38's shared rule (`ezsdr_radio::rx_link`): the admission of a `Block` link on a receive
//! port, the once-per-segment event, the discard count and the merge of the device loss a block
//! carries (RM-17). Both Providers call it, so these are the pure tests of the one text.

use ezsdr_kernel::stream::BackPressure;
use ezsdr_radio::payloads::RxOverflowCause;
use ezsdr_radio::rx_link::{Discards, LossReport, REASON, Refused, lost_before, refusal};

/// The one refusal text RM-38, MR-7 and UR-12 quote.
const REASON_TEXT: &str = "RM-38: a Block link must be its receive port's only link";

#[test]
fn rm_38_a_block_link_must_be_its_ports_only_link() {
    assert_eq!(REASON, REASON_TEXT, "the literal both Providers put in `prepare`'s refusal");
    assert_eq!(refusal(&[]), None, "no link");
    assert_eq!(refusal(&[BackPressure::Block]), None, "a sole `Block` link");
    assert_eq!(refusal(&[BackPressure::DropOldest, BackPressure::DropNewest, BackPressure::DropOldest]), None, "drop-class links, however many");
    for policies in [vec![BackPressure::Block, BackPressure::DropOldest], vec![BackPressure::DropOldest, BackPressure::Block], vec![BackPressure::Block, BackPressure::Block]] {
        assert_eq!(refusal(&policies), Some(REASON_TEXT), "{policies:?}: a `Block` link beside another");
    }
}

#[test]
fn rm_38_the_event_is_owed_once_per_segment() {
    let mut discards = Discards::default();
    assert_eq!(discards.refuse(2_000), Refused { samples: 2_000, raise_event: true }, "the segment's first discard owes the event");
    assert_eq!(discards.refuse(2_000), Refused { samples: 2_000, raise_event: false }, "a later discard of the same segment owes none");
    assert!(!discards.refuse(1).raise_event);
    // MR-19: the memory is the segment's; the next segment owes its own event (a new `Discards`).
    let mut successor = Discards::default();
    assert!(successor.refuse(2_000).raise_event, "a new receive segment raises its own event");
}

#[test]
fn rm_38_the_count_is_the_samples_refused() {
    // MR-19: the stat adds the block's length, clipped at a cut by the Provider before it gets here.
    let mut discards = Discards::default();
    for len in [1u64, 1_999, 2_000, u64::MAX] {
        assert_eq!(discards.refuse(len).samples, len);
    }
}

#[test]
fn rm_38_loss_reports_merge_to_the_first_sample_the_sum_and_the_overrun() {
    // RM-17: the block-carried event keeps the smallest first lost sample, sums the loss and
    // takes `overrun` over `sequence`, whatever order the reports arrive in.
    let mut report = LossReport { first: 5_000, lost: 1_000, cause: RxOverflowCause::Sequence };
    report.merge(&LossReport { first: 3_000, lost: 2_000, cause: RxOverflowCause::Sequence });
    assert_eq!((report.first, report.lost, report.cause), (3_000, 3_000, RxOverflowCause::Sequence), "the smallest first sample and the sum");

    let mut later_overrun = LossReport { first: 3_000, lost: 2_000, cause: RxOverflowCause::Sequence };
    later_overrun.merge(&LossReport { first: 9_000, lost: 7_000, cause: RxOverflowCause::Overrun });
    assert_eq!((later_overrun.first, later_overrun.lost, later_overrun.cause), (3_000, 9_000, RxOverflowCause::Overrun), "a later overrun wins");

    let mut early_overrun = LossReport { first: 100, lost: 5, cause: RxOverflowCause::Overrun };
    early_overrun.merge(&LossReport { first: 50, lost: 6, cause: RxOverflowCause::Sequence });
    assert_eq!((early_overrun.first, early_overrun.lost, early_overrun.cause), (50, 11, RxOverflowCause::Overrun), "an earlier overrun stays the cause");

    assert_eq!((LossReport { first: 0, lost: 0, cause: RxOverflowCause::Overrun }.payload()), None, "RM-17: a loss of 0 produces no event");
    let payload = early_overrun.payload().expect("a reported loss");
    assert_eq!((payload.cause, payload.lost), (RxOverflowCause::Overrun, 11), "the merged report is the event's payload");
}

#[test]
fn rm_38_loss_before_a_cut_counts_overlapping_gaps_once() {
    // MR-19, MR-27: a cut inside a gap reports the device loss up to the cut; each lost sample
    // is counted once, however the fault intervals overlap.
    assert_eq!(lost_before(&[], 100), 0, "no gap");
    assert_eq!(lost_before(&[(10, 20), (30, 40)], 100), 20, "disjoint gaps");
    assert_eq!(lost_before(&[(10, 30), (20, 40)], 100), 30, "an overlap is counted once");
    assert_eq!(lost_before(&[(10, 40), (20, 30)], 100), 30, "a nested gap is counted once");
    assert_eq!(lost_before(&[(10, 20), (10, 20)], 100), 10, "a repeated gap is counted once");
    assert_eq!(lost_before(&[(10, 20)], 15), 5, "a cut through a gap counts to the cut");
    assert_eq!(lost_before(&[(10, 20), (30, 40)], 35), 15, "the gap before the cut whole, the one it cuts to the cut");
    assert_eq!(lost_before(&[(10, 20), (30, 40)], 20), 10, "a cut at a gap's end loses all of it");
    assert_eq!(lost_before(&[(10, 20), (30, 40)], 10), 0, "a cut at a gap's first sample loses nothing before it");
    assert_eq!(lost_before(&[(30, 40)], 20), 0, "a gap wholly after the cut contributes nothing");
    assert_eq!(lost_before(&[(10, 20), (30, 40)], 5), 0, "a cut before every gap");
}
