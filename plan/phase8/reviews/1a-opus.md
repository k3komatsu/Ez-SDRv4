# Orchestrator review: Phase 8 stage 1a (Opus)

## O1 (major): a `random` channel keeps its drawn phase through a timed tune — the Mock is looser than a CBX

`crates/ezsdr-mock-radio/src/lib.rs:1399-1409`, MR-34 (`design/09-mock-radio.md:197`), test `mr_34_a_random_phase_survives_a_timed_tune` (`crates/ezsdr-mock-radio/tests/mock_channel.rs:738`).

The implementation skips the phase update for `random`, so the channel's phase stays exactly the one drawn at `prepare`. A CBX's PLL relocks on every retune and lands on a new random phase, timed or not. A Mock that keeps the old phase keeps the inter-channel phase offset stable across a retune: an experiment that calibrates the offset once and retunes would pass in simulation and fail on the bench — the exact failure the M12 row of spec 09 names. The test pins the wrong behaviour (the rotation before and after the retunes is the same).

Fix: for `random`, every applied frequency update of a direction gives each channel of that direction a **fresh** phase from its effective instant on, drawn from a per-device `SimRng` stream in application order (deterministic, MR-30; e.g. `SimRng::new(seed, "<device>/lo/retune")` kept in the channel-mode state, draws in application order, rx and tx in the order the updates apply). Rewrite MR-34's sentence and the test: after each of two timed retunes the CBX's rotation differs from the one before it (both channels) and the two channels' offset changes; the UBX test keeps the same constant after every timed tune. Rename the test (e.g. `mr_34_a_random_phase_is_redrawn_by_every_tune`). Re-target P10 to the new code.

## O2 (major): a layout element's slot is not what the UHD Module uses

`crates/ezsdr-radio/src/x3xx.rs` (`from_selector`, the order check), `crates/ezsdr-radio-uhd/src/provider/mod.rs` (UR-5 loop: element `i` ↔ UHD channel `i`).

The UHD Module maps layout element `i` to UHD channel `i` under UHD's default subdev spec (`A:0 B:0`), so `rx: ["0:B:ubx:0"]` is accepted and actually uses slot A. The Mock believes slot B. Today only the board matters, but the layout already says something false and 1b-4 would inherit it.

Fix (1a restriction, replaces the duplicate and order checks): in each direction, element `i` must name slot `[A, B][i]` — the layout is a prefix of UHD's default channel order. Refuse otherwise ("… names slot B at position 0; this stage's layout follows UHD's default channel order A, B"). Update `rm_26_the_layout_parser_checks_the_shared_selector`, `mr_02_from_binding_refusals`, the docs (UR-5's table, MR-2), and the mutation rows (P08/P09 become one or two rows on the new check, each killed).

## O3 (major): an empty `rx` (or `tx`) layout still defaults to one channel

`crates/ezsdr-radio/src/x3xx.rs` `description` takes `DeviceDescription::x310_defaults()`, whose `radio.rx.channels` default is 1. With `rx: []`, `rx_max_channels` is 0, but a Spec that does not request `radio.rx.channels` gets the default 1 — the Mock opens a receive stream on a layout with no receive front end, and the UHD Module would configure UHD channel 0 for receive. Fix: in `description`, the `radio.rx.channels` default is `min(1, rx.len())` (and `radio.tx.channels` stays 0). Test: a `tx`-only layout, a Spec with no `rx` request → `radio.rx.channels` applied 0 on both Providers (add to `d3_the_layout_decides_the_channels_on_both_providers`), plus a mutation row.

## O4 (minor): `rules.py`'s `KNOWN_MISSING` is the wrong mechanism

`plan/maintenance/tools/rules.py:52-60`. `KNOWN_MISSING` is for pre-plan-31 history. The tool's mechanism for a rename is `--renames`. Fix: revert the `KNOWN_MISSING` change; drop the old test names from the 2026-10-17 Changes rows of `design/07` and `design/18` (say "renamed" with the new names only); write the three `old -> new` lines to `~/.cache/ezsdr-fixes/2026-10-17-phase8-1a/renames.txt` and check with `rules.py --base HEAD --renames <that file>`.

## O5 (minor): the Mock rebuilds the whole description on every query

`crates/ezsdr-mock-radio/src/profile.rs`: `timing()`, `performance()`, `random_phase_on_untimed_tune()`, `timed_tune_fixes_phase()` each call `x3xx::description`, which builds the 639-value rate list; `timed_tune_fixes_phase` runs on every applied frequency update. Fix: build the `DeviceDescription` once in `from_binding` and keep it in `Profile` (for `ideal` too); the accessors read it.

## O6 (minor): spec 07 §2's retune-phase bullet contradicts itself

`design/07-radio-model.md` §2: "Retune phase on UBX/CBX/SBX is random unless the retune is timed …, and a CBX has no timed LO phase synchronisation at all". Write it as: UBX and OBX are random unless the retune is timed; a CBX is random on every retune (no timed LO phase synchronisation). Drop SBX (not described at this stage).

## Sound

`x3xx.rs` decimation set and clip (639 values, `coerce_decim` nearest with ties to smaller N); the per-direction start lead through `timeline::start_lead_ns` in both Modules; the server's default profile; UR-5's per-element front-end check; the D3 audit test compares capabilities from both Providers; schema regenerated; the Mock now takes the description's defaults (the CBX's 2.45 GHz).
