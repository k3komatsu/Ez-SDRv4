# Orchestrator review: Phase 8 stage 1b-2b (Opus)

## O1 (minor): an impossible move is booked silently and restarts both streams
`crates/ezsdr-radio/src/timeline.rs` `book_move`. When `channel` is not in the source's set at `e`, is already in the destination's, or is ≥ 64, the function still books a `Cold` item on both lines with unchanged sets: both streams are cut and restart on new SampleClocks for nothing. A Provider bug then shows up as a data gap, not an error.
Fix: `book_move` refuses these before booking, leaving both lines unchanged: return a dedicated error (e.g. `MoveError::{Time(TimeError), NotInSource, AlreadyInDestination, NoSuchChannel}` or an equivalent), documented in RM-34. Tests for the three refusals; a mutation row that drops the check.

## O2 (minor): the configuration a later change starts from is not exposed
`config_at` is private. After a move, a Provider that books a rate change on either line must build that change's `Config` from the set the move left; without the timeline's own reading it keeps its own copy of the membership (the `mech:radio-rule-implemented-twice` risk D3's9 names). Fix: make it a public `Line::config_at(&self, e: i64) -> Config` (the configuration in effect at `e`, pruned state included), say in RM-34 that a Provider builds a later change's configuration from it, and test it after a move and after a prune.

## Sound
`Channels` as a 64-bit set with `first(n)` for today's Providers; equality by set; `is_empty` in place of `> 0`; one instant on both lines with the floor from both lines' earlier `cold` changes, `at`, `now` and a receive line's start; the destination's ready at the source's cut instant (old clock); clone-then-commit for both bookings and both refusals; `refuse_move` refusing only a `seq` on both lines; Mock/UHD following mechanically with `count()` in the Manifest.
