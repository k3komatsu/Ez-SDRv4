//! The X3xx motherboard and front end descriptions MockRadio and the UHD Module both
//! build, and the flat selector layout both parse (design 33 D5). One place, so the two
//! Providers cannot drift apart (documentation audit D3).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use ezsdr_kernel::module_api::{ProfileRef, Version};
use ezsdr_kernel::spec::{Ident, Scalar, Value};

use crate::device::{DeviceDescription, Grid};
use crate::{keys, PerformanceEnvelope, TimingEnvelope};

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
/// `DeviceDescription` is — that is `master_clock_hz / N` for the decimations `N` from
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

    /// Whether the board's channels keep a known phase relation, `radio.{rx,tx}.coherent`
    /// (RM-9): the UBX and the OBX do after a timed tune, a CBX aligns its channels in
    /// samples but not in phase (design 33 D13).
    pub fn coherent(self) -> bool {
        !matches!(self, Board::Cbx)
    }

    /// `radio.phase_behavior_on_retune` (RM-9, design 33 D13).
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

/// One logical channel's front end: `"<timekeeper>:<slot>:<board>:<frontend>"` (design 33
/// D5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Element {
    /// The timekeeper it belongs to; 0 in this stage.
    pub timekeeper: u32,
    /// Its slot.
    pub slot: Slot,
    /// Its daughterboard's kind.
    pub board: Board,
    /// UHD's front end number on that board; 0 in this stage.
    pub frontend: u32,
}

/// The layout and the other shared selector values one device's binding declares (design
/// 33 D5).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Selection {
    /// The master clock; the only accepted value is [`MASTER_CLOCK_HZ`].
    pub master_clock_hz: u64,
    /// The FPGA image; checked, used by nothing yet.
    pub image: Image,
    /// The number of 10 GbE links; checked, used by nothing yet.
    pub links: u8,
    /// The speed of one link, 10 or 1 GbE: what the transport's bytes per second are.
    pub link_gbps: u8,
    /// The one daughterboard kind the layout names (design 33 D5).
    pub board: Board,
    /// The receive channels, in order; the position is the logical channel number.
    pub rx: Vec<Element>,
    /// The transmit channels, in order.
    pub tx: Vec<Element>,
}

impl Selection {
    /// A selection of `board`'s front ends in `rx` and `tx` slot order, with 200 MHz, the
    /// HG image, one link and 10 GbE: what a test builds without a selector.
    pub fn of(board: Board, rx: &[Slot], tx: &[Slot]) -> Selection {
        let elements = |slots: &[Slot]| {
            slots
                .iter()
                .map(|slot| Element { timekeeper: 0, slot: *slot, board, frontend: 0 })
                .collect()
        };
        Selection {
            master_clock_hz: MASTER_CLOCK_HZ,
            image: Image::Hg,
            links: 1,
            link_gbps: 10,
            board,
            rx: elements(rx),
            tx: elements(tx),
        }
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

/// Reads the shared selector keys of `selector` and checks the layout (design 33 D5).
/// The reason is returned without a rule prefix; a Provider prefixes its own.
pub fn from_selector(selector: &BTreeMap<Ident, Value>) -> Result<Selection, String> {
    let get = |name: &str| selector.get(&Ident::parse(name).expect("a selector key"));
    let master_clock_hz = match get("master_clock_hz") {
        None => MASTER_CLOCK_HZ,
        Some(Value::List(values)) => match values.as_slice() {
            [Scalar::Int(other)] if *other == MASTER_CLOCK_HZ as i64 => MASTER_CLOCK_HZ,
            [Scalar::Int(other)] => {
                return Err(format!(
                    "the layout's master clock must be {MASTER_CLOCK_HZ} Hz, not {other}"
                ))
            }
            _ => {
                return Err(format!(
                    "the layout's `master_clock_hz` must be the one-element list [{MASTER_CLOCK_HZ}]"
                ))
            }
        },
        Some(_) => {
            return Err("the layout's `master_clock_hz` must be a list of integers".to_owned())
        }
    };
    let image = match get("image") {
        None => Image::Hg,
        Some(Value::List(values)) => match values.as_slice() {
            [Scalar::Str(name)] => Image::parse(name)
                .ok_or_else(|| format!("`{name}` is not an FPGA image; `hg` or `xg`"))?,
            _ => return Err("the layout's `image` must be the one-element list [`hg`|`xg`]".to_owned()),
        },
        Some(_) => return Err("the layout's `image` must be a list of strings".to_owned()),
    };
    let number = |name: &str, allowed: &[i64], default: i64| -> Result<i64, String> {
        match get(name) {
            None => Ok(default),
            Some(Value::List(values)) => match values.as_slice() {
                [Scalar::Int(value)] if allowed.contains(value) => Ok(*value),
                [Scalar::Int(value)] => {
                    Err(format!("the layout's `{name}` must be one of {allowed:?}, not {value}"))
                }
                _ => Err(format!("the layout's `{name}` must be a one-element list of integers")),
            },
            Some(_) => Err(format!("the layout's `{name}` must be a list of integers")),
        }
    };
    let links = number("links", &[1, 2], 1)? as u8;
    let link_gbps = number("link_gbps", &[1, 10], 10)? as u8;

    let list = |name: &str| -> Result<Vec<Element>, String> {
        let values = match get(name) {
            None => return Err(format!("the `x3xx` selector needs `{name}`, its channel layout")),
            Some(Value::List(values)) => values,
            Some(_) => return Err(format!("the layout's `{name}` must be a list of strings")),
        };
        values
            .iter()
            .map(|value| match value {
                Scalar::Str(raw) => element(raw),
                _ => Err(format!("the layout's `{name}` must hold strings")),
            })
            .collect()
    };
    let rx = list("rx")?;
    let tx = list("tx")?;

    if rx.is_empty() && tx.is_empty() {
        return Err("the layout names no front end".to_owned());
    }
    let board = rx.first().or_else(|| tx.first()).expect("a nonempty layout").board;
    if let Some(other) = rx.iter().chain(&tx).find(|element| element.board != board) {
        return Err(format!(
            "the layout names the boards `{}` and `{}`; one device carries one kind",
            board.name(),
            other.board.name()
        ));
    }
    for (direction, elements) in [("rx", &rx), ("tx", &tx)] {
        // This stage's layout is a prefix of UHD's default channel order: element `i` is
        // slot A at position 0, slot B at position 1, and there is no third (design 33 D5).
        const ORDER: [Slot; 2] = [Slot::A, Slot::B];
        for (position, element) in elements.iter().enumerate() {
            if ORDER.get(position) != Some(&element.slot) {
                return Err(format!(
                    "the layout's `{direction}` element at position {position} names slot {:?}; this stage's layout follows UHD's default channel order, slot A at position 0 and slot B at position 1",
                    element.slot
                ));
            }
        }
    }
    Ok(Selection { master_clock_hz, image, links, link_gbps, board, rx, tx })
}

/// Parses one `"<timekeeper>:<slot>:<board>:<frontend>"` element (design 33 D5).
fn element(raw: &str) -> Result<Element, String> {
    let fields: Vec<&str> = raw.split(':').collect();
    let Ok([timekeeper, slot, board, frontend]) = <[&str; 4]>::try_from(fields.as_slice()) else {
        return Err(format!(
            "the layout element `{raw}` is not `<timekeeper>:<slot>:<board>:<frontend>`"
        ));
    };
    let timekeeper: u32 = timekeeper
        .parse()
        .map_err(|_| format!("the layout element `{raw}` has no timekeeper number"))?;
    if timekeeper != 0 {
        return Err(format!(
            "the layout element `{raw}` names timekeeper {timekeeper}; this stage has one device, 0"
        ));
    }
    let slot = Slot::parse(slot)
        .ok_or_else(|| format!("the layout element `{raw}` names slot `{slot}`, not A or B"))?;
    let board = Board::parse(board).ok_or_else(|| {
        format!("the layout element `{raw}` names the board `{board}`, which this stage does not describe")
    })?;
    let frontend: u32 = frontend
        .parse()
        .map_err(|_| format!("the layout element `{raw}` has no front end number"))?;
    if frontend != 0 {
        return Err(format!(
            "the layout element `{raw}` names front end {frontend}; this stage has 0"
        ));
    }
    Ok(Element { timekeeper, slot, board, frontend })
}

/// The `DeviceDescription` of a `x3xx` motherboard with `selection`'s front ends (RM-26,
/// design 33 D5). `block_len` is the Provider's, and `terms` are the Module's own values.
pub fn description(selection: &Selection, block_len: u32, terms: &ProviderTerms) -> DeviceDescription {
    let board = selection.board;
    let mut defaults = DeviceDescription::x310_defaults();
    for name in [keys::RX_FREQUENCY_HZ, keys::TX_FREQUENCY_HZ] {
        let value = Scalar::try_from(board.default_frequency_hz()).expect("a finite default");
        defaults.insert(
            ezsdr_kernel::spec::Key::parse(name).expect("a radio key"),
            Value::from(value),
        );
    }
    // RM-5's default receive channel count is one; a layout with no receive front end
    // requests none, so the Mock opens no receive stream on it (design 33 D5).
    let rx_default = i64::from(!selection.rx.is_empty());
    defaults.insert(
        ezsdr_kernel::spec::Key::parse(keys::RX_CHANNELS).expect("a radio key"),
        Value::from(rx_default),
    );
    let (rx_antennas, tx_antennas) = board.antennas();
    let bytes_per_s = i64::from(selection.link_gbps) * 100_000_000;
    DeviceDescription {
        profile: profile_ref(),
        rates: Grid::Values(rates(selection.master_clock_hz)),
        whole_hertz_rates: false,
        frequency: board.frequency(),
        gain: board.gain(),
        rx_max_channels: selection.rx.len() as i64,
        tx_max_channels: selection.tx.len() as i64,
        rx_antennas,
        tx_antennas,
        coherent: board.coherent(),
        full_duplex: true,
        hardware_time: true,
        phase_behavior_on_retune: board.phase_behavior_on_retune().to_owned(),
        repeat_max_samples: terms.repeat_max_samples,
        repeat_align_samples: terms.repeat_align_samples,
        block_len,
        tx_path_delay_samples: 45,
        rx_path_delay_samples: 0,
        timing: TimingEnvelope {
            min_timed_command_lead_ns: terms.min_timed_command_lead_ns,
            startup_latency_ns: 2_000_000_000,
            command_queue_depth: 16,
            overflow_restart_gap_ns: 50_000_000,
            rx_start_lead_ns: 50_000_000,
            tx_start_lead_ns: 50_000_000,
        },
        performance: PerformanceEnvelope {
            rx_bytes_per_s: bytes_per_s,
            tx_bytes_per_s: bytes_per_s,
            wire_bytes_per_sample: 4,
        },
        defaults,
    }
}
