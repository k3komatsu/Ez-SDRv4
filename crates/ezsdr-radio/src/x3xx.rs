//! The X3xx motherboard and front end descriptions MockRadio and the UHD Module both
//! build, and the flat selector layout both parse (design 33 D5). One place, so the two
//! Providers cannot drift apart (documentation audit D3).

use std::sync::OnceLock;

use ezsdr_kernel::module_api::{ProfileRef, Version};

use crate::device::Grid;
use crate::{PerformanceEnvelope, TimingEnvelope};

/// The one profile name both Providers declare (design 33 D5).
pub const PROFILE: &str = "x3xx";
/// Its version: a profile nothing has measured yet (design 33 D5).
pub const PROFILE_VERSION: Version = Version::new(0, 1, 0);
/// The X3xx master clock, the one UR-5 accepts (RM-3, UR-9).
pub const MASTER_CLOCK_HZ: u64 = 200_000_000;
/// The selector keys both Providers read (design 33 D5). A Provider adds its own keys to
/// these, and refuses every other key of its binding.
pub const SELECTOR_KEYS: [&str; 6] =
    ["master_clock_hz", "image", "links", "link_gbps", "rx", "tx"];

/// The profile this Module's binding must name (design 33 D5).
pub fn profile_ref() -> ProfileRef {
    ProfileRef { name: PROFILE.to_owned(), version: PROFILE_VERSION }
}

/// The decimations an X3xx's DDC and DUC make from the master clock (design 33 D5):
/// `{1} ∪ {2^h · c : 0 ≤ h ≤ 3, 1 ≤ c ≤ 255}`, UHD 4.10's `ddc_block_control.cpp` and
/// `duc_block_control.cpp`, with the X300 standard image's three halfbands and CIC 255.
/// 639 values, ascending; `N = 257…511` odd is not among them.
pub fn decimations() -> &'static [u64] {
    static SET: OnceLock<Vec<u64>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set = vec![1u64];
        for h in 0..=3u32 {
            for c in 1..=255u64 {
                set.push(u64::from(1u32 << h) * c);
            }
        }
        set.sort_unstable();
        set.dedup();
        set
    })
}

/// Whether `n` is a decimation of an X3xx (design 33 D5).
pub fn is_decimation(n: u64) -> bool {
    decimations().binary_search(&n).is_ok()
}

/// The rate grid of a `master_clock_hz` X3xx, ascending by rate, as a `Values` grid of
/// `channels::Description` is — that is `master_clock_hz / N` for the decimations `N` from
/// 2040 down to 1. The `AnyOf` capability MR-4 builds from it is the reverse (ascending
/// `N`).
pub fn rates(master_clock_hz: u64) -> Vec<f64> {
    decimations().iter().rev().map(|n| master_clock_hz as f64 / *n as f64).collect()
}

/// `N` such that `rate` is exactly `master_clock_hz / N` for a decimation `N` (RM-8,
/// UR-12); none when `rate` is off the grid.
pub fn decimation(master_clock_hz: u64, rate: f64) -> Option<u64> {
    if !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    let n = (master_clock_hz as f64 / rate).round();
    if n < 1.0 {
        return None;
    }
    let n = n as u64;
    (is_decimation(n) && master_clock_hz as f64 / n as f64 == rate).then_some(n)
}

/// The decimation a device applies to `rate` by clipping it onto its grid, as UHD's
/// `coerce_decim` does (`meta_range_t::clip(requested, true)`): the decimation nearest
/// the requested `master_clock_hz / rate`, ties to the smaller (UR-33). A rate that is
/// not positive takes the smallest decimation, 1.
pub fn clip_decimation(master_clock_hz: u64, rate: f64) -> u64 {
    if !rate.is_finite() || rate <= 0.0 {
        return 1;
    }
    let wanted = master_clock_hz as f64 / rate;
    let mut best = 1u64;
    let mut best_error = f64::INFINITY;
    for n in decimations() {
        let error = (wanted - *n as f64).abs();
        if error < best_error {
            best_error = error;
            best = *n;
        }
    }
    best
}

/// A daughterboard kind this stage describes (design 33 D5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Board {
    /// A UBX-160, 10 MHz…6 GHz.
    Ubx,
    /// An OBX, 10 MHz…8.4 GHz.
    Obx,
    /// A CBX or CBX-120, 1.2…6 GHz.
    Cbx,
}

impl Board {
    /// The name the selector writes it with (design 33 D5).
    pub fn name(self) -> &'static str {
        match self {
            Board::Ubx => "ubx",
            Board::Obx => "obx",
            Board::Cbx => "cbx",
        }
    }

    /// The board the selector name `name` names, if this stage describes it.
    pub fn parse(name: &str) -> Option<Board> {
        [Board::Ubx, Board::Obx, Board::Cbx].into_iter().find(|board| board.name() == name)
    }

    /// How UHD's names of its front ends begin (`UBX RX`, `OBX TX`, `CBX-120 TX`; UR-5).
    pub fn uhd_name(self) -> &'static str {
        match self {
            Board::Ubx => "UBX",
            Board::Obx => "OBX",
            Board::Cbx => "CBX",
        }
    }

    /// The board UHD's front end name `name` names, if this stage describes it (UR-5).
    pub fn of_front_end(name: &str) -> Option<Board> {
        [Board::Ubx, Board::Obx, Board::Cbx]
            .into_iter()
            .find(|board| name.starts_with(board.uhd_name()))
    }

    /// The tuning range, a `Step` grid of 1 Hz (design 33 D5).
    pub fn frequency(self) -> Grid {
        let (lo, hi) = match self {
            // UHD 4.10's `ubx_freq_range` and `obx_freq_range` (`db_ubx.hpp`, `db_obx.hpp`).
            Board::Ubx => (10_000_000.0, 6_000_000_000.0),
            Board::Obx => (10_000_000.0, 8_400_000_000.0),
            // UHD 4.10's `cbx_freq_range` (`db_sbx_common.hpp`).
            Board::Cbx => (1_200_000_000.0, 6_000_000_000.0),
        };
        Grid::Step { lo, hi, step: 1.0 }
    }

    /// The gain range, a `Step` grid of 0.5 dB, the same on every board of this stage
    /// (design 33 D5).
    pub fn gain(self) -> Grid {
        Grid::Step { lo: 0.0, hi: 31.5, step: 0.5 }
    }

    /// The receive and transmit antenna names (design 33 D5).
    pub fn antennas(self) -> (Vec<String>, Vec<String>) {
        (vec!["RX2".to_owned(), "TX/RX".to_owned()], vec!["TX/RX".to_owned()])
    }

    /// `radio.{rx,tx}.c<n>.phase_behavior_on_retune` (RM-9, design 33 D13).
    pub fn phase_behavior_on_retune(self) -> &'static str {
        match self {
            // UHD's timed LO phase synchronisation is the UBX's and the OBX's only
            // (`db_ubx.cpp`, `obx/obx_expert.cpp`); a CBX has none.
            Board::Ubx | Board::Obx => "random_unless_timed_tune",
            Board::Cbx => "random",
        }
    }

    /// RM-5's default frequency for each direction (design 33 D12).
    pub fn default_frequency_hz(self) -> f64 {
        match self {
            Board::Ubx | Board::Obx => 1_000_000_000.0,
            Board::Cbx => 2_450_000_000.0,
        }
    }
}

/// The path delay from a transmit sample's timestamp to the antenna, in samples (RM-23):
/// the value every board of this stage declares.
pub const TX_PATH_DELAY: i64 = 45;
/// The receive path delay, in samples (RM-23).
pub const RX_PATH_DELAY: i64 = 0;

/// A daughterboard kind a layout may name (RM-28): the three boards that transmit and
/// receive, and the four Basic/LF boards, which carry one direction each.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrontEnd {
    /// A UBX-160.
    Ubx,
    /// An OBX.
    Obx,
    /// A CBX or CBX-120.
    Cbx,
    /// A BasicRX.
    BasicRx,
    /// An LFRX.
    LfRx,
    /// A BasicTX.
    BasicTx,
    /// An LFTX.
    LfTx,
}

impl FrontEnd {
    /// The name the selector writes it with (RM-28).
    pub fn name(self) -> &'static str {
        match self {
            FrontEnd::Ubx => "ubx",
            FrontEnd::Obx => "obx",
            FrontEnd::Cbx => "cbx",
            FrontEnd::BasicRx => "basic_rx",
            FrontEnd::LfRx => "lf_rx",
            FrontEnd::BasicTx => "basic_tx",
            FrontEnd::LfTx => "lf_tx",
        }
    }

    /// The front end the selector name `name` names (RM-28).
    pub fn parse(name: &str) -> Option<FrontEnd> {
        [
            FrontEnd::Ubx,
            FrontEnd::Obx,
            FrontEnd::Cbx,
            FrontEnd::BasicRx,
            FrontEnd::LfRx,
            FrontEnd::BasicTx,
            FrontEnd::LfTx,
        ]
        .into_iter()
        .find(|front_end| front_end.name() == name)
    }

    /// The duplex board of the three, when this is one (RM-28).
    pub fn board(self) -> Option<Board> {
        match self {
            FrontEnd::Ubx => Some(Board::Ubx),
            FrontEnd::Obx => Some(Board::Obx),
            FrontEnd::Cbx => Some(Board::Cbx),
            FrontEnd::BasicRx | FrontEnd::LfRx | FrontEnd::BasicTx | FrontEnd::LfTx => None,
        }
    }

    /// Whether the board carries receive channels (RM-28).
    pub fn receives(self) -> bool {
        matches!(self, FrontEnd::Ubx | FrontEnd::Obx | FrontEnd::Cbx | FrontEnd::BasicRx | FrontEnd::LfRx)
    }

    /// Whether the board carries transmit channels (RM-28).
    pub fn transmits(self) -> bool {
        matches!(self, FrontEnd::Ubx | FrontEnd::Obx | FrontEnd::Cbx | FrontEnd::BasicTx | FrontEnd::LfTx)
    }

    /// UHD's front end numbers the board has (RM-28): the receive-only Basic/LF boards
    /// have `0` and `1`, every other board `0`.
    pub fn frontends(self) -> &'static [u32] {
        match self {
            FrontEnd::BasicRx | FrontEnd::LfRx => &[0, 1],
            _ => &[0],
        }
    }

    /// The tuning range as a `Step` grid of 1 Hz (RM-29). A board with an LO keeps the
    /// values of [`Board`]; a Basic/LF board tunes its DDC or DUC, within the master
    /// clock's half rate (Basic) or the board's 32 MHz (LF).
    pub fn frequency(self, master_clock_hz: u64) -> Grid {
        match self.board() {
            Some(board) => board.frequency(),
            None => {
                let hi = match self {
                    FrontEnd::BasicRx | FrontEnd::BasicTx => master_clock_hz as f64 / 2.0,
                    _ => 32_000_000.0,
                };
                Grid::Step { lo: 0.0, hi, step: 1.0 }
            }
        }
    }

    /// The gain range as a `Step` grid (RM-29): the board's RF stage, and `{0}` for a
    /// Basic/LF board, which has no gain stage of its own.
    pub fn gain(self) -> Grid {
        match self.board() {
            Some(board) => board.gain(),
            None => Grid::Values(vec![0.0]),
        }
    }

    /// The receive and transmit antenna names (RM-29).
    pub fn antennas(self) -> (Vec<String>, Vec<String>) {
        match self.board() {
            Some(board) => board.antennas(),
            None => {
                let names = vec!["A".to_owned(), "B".to_owned(), "AB".to_owned(), "BA".to_owned()];
                (names.clone(), names)
            }
        }
    }

    /// RM-30's default antenna for one direction (RM-29): a board's first antenna name
    /// in that direction, and `AB` for a Basic/LF board.
    pub fn default_antenna(self, receive: bool) -> String {
        match self.board() {
            Some(board) => {
                let (rx, tx) = board.antennas();
                let names = if receive { rx } else { tx };
                names.into_iter().next().expect("a board has an antenna in each direction")
            }
            None => "AB".to_owned(),
        }
    }

    /// `radio.{rx,tx}.c<n>.phase_behavior_on_retune` (RM-9, RM-29): the board's value, and
    /// the weaker of the two phase guarantees for Basic/LF until a measurement confirms
    /// the stronger one.
    pub fn phase_behavior_on_retune(self) -> &'static str {
        match self.board() {
            Some(board) => board.phase_behavior_on_retune(),
            None => "random_unless_timed_tune",
        }
    }

    /// RM-5's default frequency for the board (RM-29).
    pub fn default_frequency_hz(self) -> f64 {
        match self.board() {
            Some(board) => board.default_frequency_hz(),
            None => 0.0,
        }
    }

    /// The path delay of one direction, in samples (RM-23).
    pub fn path_delay_samples(self, receive: bool) -> i64 {
        if receive { RX_PATH_DELAY } else { TX_PATH_DELAY }
    }
}

/// Whether two boards of one timekeeper can meet their clock requirements together
/// (RM-28): the compatibility table, which the layout analysis consults for every pair of
/// different board kinds and which admits every pair of boards this stage describes.
pub fn boards_compatible(_first: FrontEnd, _second: FrontEnd) -> bool {
    true
}

/// A slot a daughterboard sits in (design 33 D5).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Slot {
    /// Slot A.
    A,
    /// Slot B.
    B,
}

impl Slot {
    /// The slot the selector name `name` names.
    pub fn parse(name: &str) -> Option<Slot> {
        match name {
            "A" => Some(Slot::A),
            "B" => Some(Slot::B),
            _ => None,
        }
    }
}

/// The FPGA image a timekeeper runs (design 33 D5). Its value is checked and used by
/// nothing yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Image {
    /// The HG image.
    Hg,
    /// The XG image.
    Xg,
}

impl Image {
    /// The image the selector string `name` names.
    pub fn parse(name: &str) -> Option<Image> {
        match name {
            "hg" => Some(Image::Hg),
            "xg" => Some(Image::Xg),
            _ => None,
        }
    }

    /// Its selector name.
    pub fn name(self) -> &'static str {
        match self {
            Image::Hg => "hg",
            Image::Xg => "xg",
        }
    }
}

/// The TimingEnvelope every `x3xx` description declares (RM-6, RM-20).
pub fn timing(terms: &ProviderTerms) -> TimingEnvelope {
    TimingEnvelope {
        min_timed_command_lead_ns: terms.min_timed_command_lead_ns,
        startup_latency_ns: 2_000_000_000,
        command_queue_depth: 16,
        overflow_restart_gap_ns: 50_000_000,
        rx_start_lead_ns: 50_000_000,
        tx_start_lead_ns: 50_000_000,
    }
}

/// The PerformanceEnvelope of an `x3xx` with one `link_gbps` GbE link (RM-7, RM-20).
pub fn performance(link_gbps: u8) -> PerformanceEnvelope {
    let bytes_per_s = i64::from(link_gbps) * 100_000_000;
    PerformanceEnvelope {
        rx_bytes_per_s: bytes_per_s,
        tx_bytes_per_s: bytes_per_s,
        wire_bytes_per_sample: 4,
    }
}

/// The values a Provider's own Module contributes to a `x3xx` description (design 33 D5):
/// the two that differ between the Providers at this stage, so that a description says
/// whose value each is (the values are levelled in stage 4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProviderTerms {
    /// `radio.timing.min_timed_command_lead_ns`.
    pub min_timed_command_lead_ns: i64,
    /// `radio.tx.repeat_max_samples`.
    pub repeat_max_samples: u64,
    /// `radio.tx.repeat_align_samples`.
    pub repeat_align_samples: u64,
}

/// RM-28 (design 33 D5): no `(slot, front end)` pair named twice in one direction's
/// layout; the one place the judgement and its wording live, read by
/// `channels::check_layout`.
pub(crate) fn check_repeat(direction: &str, position: usize, slot: Slot, frontend: u32, name: &str, seen: &[(Slot, u32)]) -> Result<(), String> {
    if seen.contains(&(slot, frontend)) {
        return Err(format!(
            "the layout's `{direction}` element at position {position} repeats `{name}` front end {frontend} in slot {slot:?}"
        ));
    }
    Ok(())
}
