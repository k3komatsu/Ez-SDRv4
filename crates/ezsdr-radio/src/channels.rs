//! The description built from a layout of mixed daughterboards, one channel and
//! stream key per logical channel (RM-27…RM-32, design 33 D1…D9).
//!
//! It reads the selector's layout — `"<timekeeper>:<slot>:<board>:<frontend>"` per logical
//! channel — and exposes each channel's configuration under its own numbered key (RM-27).

use std::collections::BTreeMap;

use ezsdr_kernel::binding::satisfies;
use ezsdr_kernel::contract::{DataContractId, Port, PortDirection};
use ezsdr_kernel::id::{MemoryDomainId, ResourceId};
use ezsdr_kernel::module_api::{
    CoerceReport, ModuleError, ProfileRef, RejectedRequest, Requested, Resource, UpdateClass, Version,
};
use ezsdr_kernel::spec::{CapabilityValue, Constraint, Coercion, Ident, Key, Namespace, Scalar, Value};

use crate::device::{coerce_antenna, coerce_numeric, num, rejected, Grid};
use crate::timeline::Channels;
use crate::x3xx::{self, FrontEnd, Image, ProviderTerms, Slot};
use crate::{
    keys, Concrete, PerformanceEnvelope, RadioEnvelope, TimingEnvelope, RX_STREAM_KIND,
    TX_STREAM_KIND,
};

/// The profile name of a description that has no motherboard (RM-29).
pub const IDEAL_PROFILE: &str = "ideal";
/// Its version (RM-29).
pub const IDEAL_PROFILE_VERSION: Version = Version::new(2, 0, 0);
/// The rate every stream defaults to, in samples per second (RM-30).
pub const DEFAULT_SAMPLE_RATE_HZ: f64 = 1_000_000.0;

/// One logical channel's front end in a layout (RM-28).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Element {
    /// The timekeeper it belongs to; 0 in this stage.
    pub timekeeper: u32,
    /// Its slot.
    pub slot: Slot,
    /// Its daughterboard kind.
    pub front_end: FrontEnd,
    /// UHD's front end number on that board.
    pub frontend: u32,
}

/// A binding's layout: the mixed boards of the `x3xx` selector, or the `ideal`
/// description's channel counts (RM-28).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Layout {
    /// An `x3xx` layout of mixed daughterboards.
    X3xx {
        /// The master clock; the only accepted value is [`x3xx::MASTER_CLOCK_HZ`].
        master_clock_hz: u64,
        /// The FPGA image.
        image: Image,
        /// The number of 10 GbE links.
        links: u8,
        /// The speed of one link, 10 or 1 GbE.
        link_gbps: u8,
        /// The receive channels, in order; the position is the logical channel number.
        rx: Vec<Element>,
        /// The transmit channels, in order.
        tx: Vec<Element>,
    },
    /// The `ideal` description's channel counts.
    Ideal {
        /// Receive channels.
        rx: u32,
        /// Transmit channels.
        tx: u32,
    },
}

/// One logical channel's front end values (RM-29).
#[derive(Clone, PartialEq, Debug)]
pub struct Channel {
    /// The selector element, absent for `ideal`.
    pub element: Option<Element>,
    /// The tuning range.
    pub frequency: Grid,
    /// The gain range.
    pub gain: Grid,
    /// The antenna names this direction offers.
    pub antennas: Vec<String>,
    /// RM-30's default antenna.
    pub default_antenna: String,
    /// `radio.{rx,tx}.c<n>.phase_behavior_on_retune`.
    pub phase_behavior_on_retune: String,
    /// RM-30's default frequency.
    pub default_frequency_hz: f64,
    /// `radio.{rx,tx}.c<n>.path_delay_samples` (RM-23).
    pub path_delay_samples: i64,
}

/// A radio description built from a layout (RM-29): the motherboard's values and one
/// [`Channel`] per logical channel and direction.
#[derive(Clone, PartialEq, Debug)]
pub struct Description {
    /// The profile this describes (RM-20).
    pub profile: ProfileRef,
    /// Every stream's rate grid.
    pub rates: Grid,
    /// Whether an `Eq` rate must be a whole number of hertz.
    pub whole_hertz_rates: bool,
    /// The receive channels, in logical-number order.
    pub rx: Vec<Channel>,
    /// The transmit channels, in logical-number order.
    pub tx: Vec<Channel>,
    /// `radio.full_duplex`.
    pub full_duplex: bool,
    /// `radio.hardware_time`.
    pub hardware_time: bool,
    /// `radio.tx.repeat_max_samples`.
    pub repeat_max_samples: u64,
    /// `radio.tx.repeat_align_samples`.
    pub repeat_align_samples: u64,
    /// `radio.rx.block_len`.
    pub block_len: u32,
    /// The TimingEnvelope (RM-6).
    pub timing: TimingEnvelope,
    /// The PerformanceEnvelope (RM-7).
    pub performance: PerformanceEnvelope,
}

fn ident(name: &str) -> Ident {
    Ident::parse(name).expect("a selector key")
}

/// The one-element integer list `selector[name]` holds, or `default` when absent
/// (RM-28).
fn one_int(selector: &BTreeMap<Ident, Value>, name: &str, allowed: &[i64], default: i64) -> Result<i64, String> {
    match selector.get(&ident(name)) {
        None => Ok(default),
        Some(Value::List(values)) => match values.as_slice() {
            [Scalar::Int(value)] if allowed.contains(value) => Ok(*value),
            [Scalar::Int(value)] => Err(format!(
                "the layout's `{name}` at timekeeper 0 must be one of {allowed:?}, not {value}"
            )),
            _ => Err(format!(
                "the layout's `{name}` must be a one-element list, one entry per timekeeper"
            )),
        },
        Some(_) => Err(format!("the layout's `{name}` must be a list of integers")),
    }
}

/// Reads the `x3xx` selector's layout and shared values (RM-28).
pub fn from_selector(selector: &BTreeMap<Ident, Value>) -> Result<Layout, String> {
    let master_clock_hz = match selector.get(&ident("master_clock_hz")) {
        None => x3xx::MASTER_CLOCK_HZ,
        Some(Value::List(values)) => match values.as_slice() {
            [Scalar::Int(other)] if *other == x3xx::MASTER_CLOCK_HZ as i64 => x3xx::MASTER_CLOCK_HZ,
            [Scalar::Int(other)] => {
                return Err(format!(
                    "the layout's `master_clock_hz` at timekeeper 0 must be {}, not {other}",
                    x3xx::MASTER_CLOCK_HZ
                ))
            }
            _ => {
                return Err(format!(
                    "the layout's `master_clock_hz` must be the one-element list [{}]",
                    x3xx::MASTER_CLOCK_HZ
                ))
            }
        },
        Some(_) => return Err("the layout's `master_clock_hz` must be a list of integers".to_owned()),
    };
    let image = match selector.get(&ident("image")) {
        None => Image::Hg,
        Some(Value::List(values)) => match values.as_slice() {
            [Scalar::Str(name)] => {
                Image::parse(name).ok_or_else(|| format!("`{name}` is not an FPGA image; `hg` or `xg`"))?
            }
            _ => return Err("the layout's `image` must be the one-element list [`hg`|`xg`]".to_owned()),
        },
        Some(_) => return Err("the layout's `image` must be a list of strings".to_owned()),
    };
    let links = one_int(selector, "links", &[1, 2], 1)? as u8;
    let link_gbps = one_int(selector, "link_gbps", &[1, 10], if image == Image::Hg { 1 } else { 10 })? as u8;

    let list = |name: &str| -> Result<Vec<Element>, String> {
        let values = match selector.get(&ident(name)) {
            None => {
                return Err(format!(
                    "the `x3xx` selector needs `{name}`, its channel layout"
                ))
            }
            Some(Value::List(values)) => values,
            Some(_) => return Err(format!("the layout's `{name}` must be a list of strings")),
        };
        values
            .iter()
            .enumerate()
            .map(|(position, value)| match value {
                Scalar::Str(raw) => element(raw, name, position),
                _ => Err(format!(
                    "the layout's `{name}` element at position {position} must be a string"
                )),
            })
            .collect()
    };
    let rx = list("rx")?;
    let tx = list("tx")?;
    let layout = Layout::X3xx { master_clock_hz, image, links, link_gbps, rx, tx };
    check_layout(&layout)?;
    Ok(layout)
}

/// Reads the `ideal` selector's channel counts (RM-28): integers 0…64, one channel for
/// receive and none for transmit when absent.
pub fn ideal_from_selector(selector: &BTreeMap<Ident, Value>) -> Result<Layout, String> {
    let count = |name: &str, default: u32| -> Result<u32, String> {
        match selector.get(&ident(name)) {
            None => Ok(default),
            Some(Value::Scalar(Scalar::Int(value))) if (0..=64).contains(value) => Ok(*value as u32),
            Some(Value::Scalar(Scalar::Int(value))) => Err(format!(
                "the `ideal` selector's `{name}` is a channel count in 0…64, not {value}"
            )),
            Some(_) => Err(format!(
                "the `ideal` selector's `{name}` is a channel count, an integer"
            )),
        }
    };
    Ok(Layout::Ideal { rx: count("rx", 1)?, tx: count("tx", 0)? })
}

/// Parses one `"<timekeeper>:<slot>:<board>:<frontend>"` element, naming its position
/// and value in every refusal (RM-28).
fn element(raw: &str, direction: &str, position: usize) -> Result<Element, String> {
    let fields: Vec<&str> = raw.split(':').collect();
    let Ok([timekeeper, slot, board, frontend]) = <[&str; 4]>::try_from(fields.as_slice()) else {
        return Err(format!(
            "the layout's `{direction}` element at position {position} `{raw}` is not `<timekeeper>:<slot>:<board>:<frontend>`"
        ));
    };
    let timekeeper: u32 = timekeeper.parse().map_err(|_| {
        format!("the layout's `{direction}` element at position {position} `{raw}` has no timekeeper number")
    })?;
    if timekeeper != 0 {
        return Err(format!(
            "the layout's `{direction}` element at position {position} `{raw}` names timekeeper {timekeeper}; this stage has one device, 0"
        ));
    }
    let slot = Slot::parse(slot).ok_or_else(|| {
        format!("the layout's `{direction}` element at position {position} `{raw}` names slot `{slot}`, not A or B")
    })?;
    let front_end = FrontEnd::parse(board).ok_or_else(|| {
        format!(
            "the layout's `{direction}` element at position {position} `{raw}` names the board `{board}`, which this stage does not describe"
        )
    })?;
    let frontend: u32 = frontend.parse().map_err(|_| {
        format!("the layout's `{direction}` element at position {position} `{raw}` has no front end number")
    })?;
    if !front_end.frontends().contains(&frontend) {
        return Err(format!(
            "the layout's `{direction}` element at position {position} `{raw}` names front end {frontend}; `{}` has {:?}",
            front_end.name(),
            front_end.frontends()
        ));
    }
    let direction_ok = if direction == "rx" { front_end.receives() } else { front_end.transmits() };
    if !direction_ok {
        let carries = if direction == "rx" { "receive" } else { "transmit" };
        return Err(format!(
            "the layout's `{direction}` element at position {position} `{raw}` names `{}`, which does not {carries}",
            front_end.name()
        ));
    }
    Ok(Element { timekeeper, slot, front_end, frontend })
}

/// RM-28's layout constraints: one board kind per slot and direction, no repeated
/// front end, no second board kind beside a duplex board, and the clock-compatibility
/// table over every pair of board kinds on one timekeeper.
fn check_layout(layout: &Layout) -> Result<(), String> {
    let Layout::X3xx { rx, tx, .. } = layout else {
        return Ok(());
    };
    if rx.is_empty() && tx.is_empty() {
        return Err("the layout names no front end".to_owned());
    }
    for (direction, elements) in [("rx", rx), ("tx", tx)] {
        let mut kinds: BTreeMap<Slot, FrontEnd> = BTreeMap::new();
        let mut seen: Vec<(Slot, u32)> = Vec::new();
        for (position, element) in elements.iter().enumerate() {
            if let Some(other) = kinds.insert(element.slot, element.front_end) {
                if other != element.front_end {
                    return Err(format!(
                        "the layout's `{direction}` element at position {position} `{}` front end {} sits in slot {:?}, which already carries `{}`",
                        element.front_end.name(),
                        element.frontend,
                        element.slot,
                        other.name()
                    ));
                }
            }
            x3xx::check_repeat(direction, position, element.slot, element.frontend, element.front_end.name(), &seen)?;
            seen.push((element.slot, element.frontend));
        }
    }
    for (position, rx_element) in rx.iter().enumerate() {
        let Some(tx_element) = tx
            .iter()
            .find(|tx_element| tx_element.slot == rx_element.slot && tx_element.timekeeper == rx_element.timekeeper)
        else {
            continue;
        };
        let duplex = rx_element.front_end.board().is_some() || tx_element.front_end.board().is_some();
        if !duplex {
            continue;
        }
        if rx_element.front_end != tx_element.front_end {
            return Err(format!(
                "the layout's `rx` element at position {position} (timekeeper {}, slot {:?}, `{}` front end {}) sits in a slot that also carries `{}`; a duplex board carries both directions and admits no other board there",
                rx_element.timekeeper,
                rx_element.slot,
                rx_element.front_end.name(),
                rx_element.frontend,
                tx_element.front_end.name()
            ));
        }
    }
    // A daughterboard's clock is shared per direction across both slots, so the
    // compatibility table is consulted for every pair of different board kinds on one
    // timekeeper (RM-28).
    let elements: Vec<&Element> = rx.iter().chain(tx.iter()).collect();
    for (position, element) in elements.iter().enumerate() {
        for other in &elements[position + 1..] {
            if element.timekeeper == other.timekeeper
                && element.front_end != other.front_end
                && !x3xx::boards_compatible(element.front_end, other.front_end)
            {
                return Err(format!(
                    "the layout's boards `{}` and `{}` cannot share timekeeper {}'s clocks",
                    element.front_end.name(),
                    other.front_end.name(),
                    element.timekeeper
                ));
            }
        }
    }
    Ok(())
}

impl Description {
    /// The description `layout` describes (RM-29). `terms` are the Provider's own values
    /// on `x3xx`; `ideal` has none.
    pub fn new(layout: &Layout, block_len: u32, terms: &ProviderTerms) -> Description {
        match layout {
            Layout::X3xx { master_clock_hz, link_gbps, rx, tx, .. } => {
                Description {
                    profile: x3xx::profile_ref(),
                    rates: Grid::Values(x3xx::rates(*master_clock_hz)),
                    whole_hertz_rates: false,
                    rx: rx.iter().map(|element| Channel::of(*element, true, *master_clock_hz)).collect(),
                    tx: tx.iter().map(|element| Channel::of(*element, false, *master_clock_hz)).collect(),
                    full_duplex: true,
                    hardware_time: true,
                    repeat_max_samples: terms.repeat_max_samples,
                    repeat_align_samples: terms.repeat_align_samples,
                    block_len,
                    timing: x3xx::timing(terms),
                    performance: x3xx::performance(*link_gbps),
                }
            }
            Layout::Ideal { rx, tx } => Description {
                profile: ProfileRef {
                    name: IDEAL_PROFILE.to_owned(),
                    version: IDEAL_PROFILE_VERSION,
                },
                rates: Grid::Integer { lo: 1, hi: 1_000_000_000 },
                whole_hertz_rates: true,
                rx: (0..*rx).map(|_| Channel::ideal(true)).collect(),
                tx: (0..*tx).map(|_| Channel::ideal(false)).collect(),
                full_duplex: true,
                hardware_time: true,
                repeat_max_samples: u64::from(u32::MAX),
                repeat_align_samples: 1,
                block_len,
                timing: TimingEnvelope {
                    min_timed_command_lead_ns: 0,
                    startup_latency_ns: 0,
                    command_queue_depth: i64::from(u32::MAX),
                    release_window_ns: 0,
                    overflow_restart_gap_ns: 0,
                    rx_start_lead_ns: 0,
                    tx_start_lead_ns: 0,
                    tx_in_flight_ns: 0,
                },
                performance: PerformanceEnvelope {
                    rx_bytes_per_s: 1_i64 << 62,
                    tx_bytes_per_s: 1_i64 << 62,
                    wire_bytes_per_sample: 8,
                },
            },
        }
    }

    /// The envelope a radio Provider records (RM-20).
    pub fn envelope(&self) -> RadioEnvelope {
        RadioEnvelope {
            profile: self.profile.clone(),
            timing: self.timing,
            performance: self.performance,
        }
    }

    /// RM-29's tree: one stream node per channel and direction, and one receive port per
    /// receive stream. Channel and timekeeper nodes are not built.
    pub fn tree(&self, device: &ResourceId, memory_domain: MemoryDomainId) -> Resource {
        let stream = |direction: &str, k: usize, kind: &str| Resource {
            id: device
                .child(&format!("{direction}/s{k}"))
                .expect("a valid stream id"),
            kind: Namespace::parse(kind).expect("a radio stream kind"),
            capabilities: BTreeMap::new(),
            children: Vec::new(),
            ports: Vec::new(),
            memory_domains: Vec::new(),
            shareable: false,
        };
        Resource {
            id: device.clone(),
            kind: Namespace::parse(crate::DEVICE_KIND).expect("the radio device kind"),
            capabilities: self.capabilities(),
            children: (0..self.rx.len())
                .map(|k| stream("rx", k, RX_STREAM_KIND))
                .chain((0..self.tx.len()).map(|k| stream("tx", k, TX_STREAM_KIND)))
                .collect(),
            ports: (0..self.rx.len())
                .map(|k| Port {
                    name: Ident::parse(&format!("rx{k}")).expect("a receive port name"),
                    direction: PortDirection::Out,
                    contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32"),
                })
                .collect(),
            memory_domains: vec![memory_domain],
            shareable: false,
        }
    }

    pub(crate) fn capabilities(&self) -> BTreeMap<Key, CapabilityValue> {
        let range = |min, max| CapabilityValue::Range { min, max };
        let one = |value| CapabilityValue::One { value };
        let mut capabilities = BTreeMap::new();
        for direction in [Direction::Rx, Direction::Tx] {
            let channels = direction.channels(self);
            let count = channels.len() as i64;
            for (number, channel) in channels.iter().enumerate() {
                let number = number as u32;
                capabilities.insert(
                    channel_setting(direction, number, "frequency_hz"),
                    range(num(channel.frequency.min()), num(channel.frequency.max())),
                );
                capabilities.insert(
                    channel_setting(direction, number, "gain_db"),
                    range(num(channel.gain.min()), num(channel.gain.max())),
                );
                capabilities.insert(
                    channel_setting(direction, number, "antenna"),
                    CapabilityValue::AnyOf {
                        values: channel.antennas.iter().map(|name| Scalar::from(name.as_str())).collect(),
                    },
                );
                capabilities.insert(
                    channel_setting(direction, number, "stream"),
                    range(Scalar::Int(-1), Scalar::Int(count - 1)),
                );
                capabilities.insert(
                    channel_setting(direction, number, "frequency_step_hz"),
                    one(num(channel.frequency.step())),
                );
                capabilities.insert(
                    channel_setting(direction, number, "gain_step_db"),
                    one(num(channel.gain.step())),
                );
                capabilities.insert(
                    channel_setting(direction, number, "phase_behavior_on_retune"),
                    one(Scalar::from(channel.phase_behavior_on_retune.as_str())),
                );
                capabilities.insert(
                    channel_setting(direction, number, "path_delay_samples"),
                    one(Scalar::Int(channel.path_delay_samples)),
                );
            }
            for k in 0..channels.len() {
                capabilities.insert(stream_setting(direction, k as u32), rates(&self.rates));
            }
        }
        for (name, value) in [
            (keys::FULL_DUPLEX, Scalar::Bool(self.full_duplex)),
            (keys::HARDWARE_TIME, Scalar::Bool(self.hardware_time)),
            (
                keys::TX_REPEAT_MAX_SAMPLES,
                Scalar::Int(i64::try_from(self.repeat_max_samples).expect("a profile limit fits i64")),
            ),
            (keys::TX_REPEAT_ALIGN_SAMPLES, Scalar::Int(self.repeat_align_samples as i64)),
            (keys::RX_BLOCK_LEN, Scalar::Int(i64::from(self.block_len))),
            (keys::MIN_TIMED_COMMAND_LEAD_NS, Scalar::Int(self.timing.min_timed_command_lead_ns)),
            (keys::STARTUP_LATENCY_NS, Scalar::Int(self.timing.startup_latency_ns)),
            (keys::COMMAND_QUEUE_DEPTH, Scalar::Int(self.timing.command_queue_depth)),
            (keys::RELEASE_WINDOW_NS, Scalar::Int(self.timing.release_window_ns)),
            (keys::OVERFLOW_RESTART_GAP_NS, Scalar::Int(self.timing.overflow_restart_gap_ns)),
            (keys::RX_START_LEAD_NS, Scalar::Int(self.timing.rx_start_lead_ns)),
            (keys::TX_START_LEAD_NS, Scalar::Int(self.timing.tx_start_lead_ns)),
            (keys::TX_IN_FLIGHT_NS, Scalar::Int(self.timing.tx_in_flight_ns)),
            (keys::RX_BYTES_PER_S, Scalar::Int(self.performance.rx_bytes_per_s)),
            (keys::TX_BYTES_PER_S, Scalar::Int(self.performance.tx_bytes_per_s)),
            (keys::WIRE_BYTES_PER_SAMPLE, Scalar::Int(self.performance.wire_bytes_per_sample)),
        ] {
            capabilities.insert(Key::parse(name).expect("a declared radio key"), one(value));
        }
        capabilities
    }

    /// RM-30's defaults: only the receive channel `c0` is in a stream, every other
    /// channel is in none, and every stream starts at 1 Msps.
    pub fn defaults(&self) -> BTreeMap<Key, Value> {
        let mut defaults = BTreeMap::new();
        for direction in [Direction::Rx, Direction::Tx] {
            let channels = direction.channels(self);
            for (number, channel) in channels.iter().enumerate() {
                let number = number as u32;
                defaults.insert(
                    channel_setting(direction, number, "frequency_hz"),
                    Value::from(num(channel.default_frequency_hz)),
                );
                defaults.insert(channel_setting(direction, number, "gain_db"), Value::from(num(0.0)));
                defaults.insert(
                    channel_setting(direction, number, "antenna"),
                    Value::from(channel.default_antenna.as_str()),
                );
                defaults.insert(
                    channel_setting(direction, number, "stream"),
                    Value::from(default_stream(direction, number)),
                );
            }
            for k in 0..channels.len() {
                defaults.insert(
                    stream_setting(direction, k as u32),
                    Value::from(num(DEFAULT_SAMPLE_RATE_HZ)),
                );
            }
        }
        defaults
    }

    /// RM-3, RM-30: the set of channels `configuration` puts in stream `k` of one
    /// direction — the numbered channels whose `radio.d.c<n>.stream` is `k`, smallest
    /// number first, which is the order a block's channels come in.
    pub fn stream_channels(&self, configuration: &BTreeMap<Key, Value>, rx: bool, k: u32) -> Channels {
        let direction = Direction::of(rx);
        let mut channels = Channels::first(0);
        for number in 0..direction.channels(self).len() as u32 {
            if stream_of(configuration, direction, number) == i64::from(k) {
                channels = channels.with(number);
            }
        }
        channels
    }

    /// RM-30: the stream `configuration` puts channel `number` of one direction in, or −1
    /// when it is in none — the membership [`Description::stream_channels`] reads, read
    /// backwards, so the two cannot drift apart.
    pub fn stream_of(&self, configuration: &BTreeMap<Key, Value>, rx: bool, number: u32) -> i64 {
        stream_of(configuration, Direction::of(rx), number)
    }

    /// RM-30's, RM-31's and RM-32's coercion; pure (MA-11).
    pub fn coerce(
        &self,
        device: &ResourceId,
        request: &Requested,
    ) -> Result<CoerceReport, ModuleError> {
        if request.resource != *device {
            return Err(ModuleError::rejected(format!(
                "RM-30: the request names {}, not this device",
                request.resource
            )));
        }
        let mut report = CoerceReport::default();
        let capabilities = self.capabilities();
        for (key, constraint) in &request.constraints {
            match crate::concrete(key) {
                Some(Concrete::Channel { rx, number, leaf }) => {
                    self.coerce_channel(Direction::of(rx), number, leaf, key, constraint, &mut report)
                }
                Some(Concrete::Stream { rx, number }) => {
                    self.coerce_stream_rate(Direction::of(rx), number, key, constraint, &mut report)
                }
                None => match capabilities.get(key) {
                    Some(capability) => coerce_capability(key, constraint, capability, &mut report),
                    None => report
                        .rejected
                        .push(rejected(key, constraint, "RM-4: not a radio key".to_owned())),
                },
            }
        }
        let mut effective = self.defaults();
        for (key, value) in &report.applied {
            if is_configuration(key) {
                effective.insert(key.clone(), value.clone());
            }
        }
        for (violation, reason) in self.violations(&effective) {
            let key = self.attributed(&violation, request).unwrap_or(violation);
            report.applied.remove(&key);
            if report.rejected.iter().any(|rejection| rejection.key == key) {
                continue;
            }
            let requested = request
                .constraints
                .get(&key)
                .cloned()
                .unwrap_or(Constraint::Present {});
            report.rejected.push(RejectedRequest { key, requested, reason });
        }
        report.warnings.clear();
        Ok(report)
    }

    fn coerce_channel(
        &self,
        direction: Direction,
        number: u32,
        leaf: &str,
        key: &Key,
        constraint: &Constraint,
        report: &mut CoerceReport,
    ) {
        let channels = direction.channels(self);
        let Some(channel) = channels.get(number as usize) else {
            report.rejected.push(rejected(
                key,
                constraint,
                format!("RM-4: no such channel c{number}"),
            ));
            return;
        };
        let result = match leaf {
            "frequency_hz" => coerce_numeric(
                key,
                constraint,
                &channel.frequency,
                Some(&Value::from(num(channel.default_frequency_hz))),
                false,
            ),
            "gain_db" => coerce_numeric(
                key,
                constraint,
                &channel.gain,
                Some(&Value::from(num(0.0))),
                false,
            ),
            "antenna" => coerce_antenna(
                constraint,
                &channel.antennas,
                Some(&Value::from(channel.default_antenna.as_str())),
            ),
            "stream" => coerce_stream(constraint, channels.len() as i64 - 1, default_stream(direction, number)),
            _ => {
                let capability = match leaf {
                    "frequency_step_hz" => CapabilityValue::One { value: num(channel.frequency.step()) },
                    "gain_step_db" => CapabilityValue::One { value: num(channel.gain.step()) },
                    "phase_behavior_on_retune" => CapabilityValue::One {
                        value: Scalar::from(channel.phase_behavior_on_retune.as_str()),
                    },
                    _ => CapabilityValue::One { value: Scalar::Int(channel.path_delay_samples) },
                };
                coerce_capability(key, constraint, &capability, report);
                return;
            }
        };
        match result {
            Ok((value, coercion)) => {
                report.applied.insert(key.clone(), value);
                if let Some(coercion) = coercion {
                    report.coercions.push(coercion);
                }
            }
            Err(reason) => report.rejected.push(rejected(key, constraint, reason)),
        }
    }

    fn coerce_stream_rate(
        &self,
        direction: Direction,
        number: u32,
        key: &Key,
        constraint: &Constraint,
        report: &mut CoerceReport,
    ) {
        if number as usize >= direction.channels(self).len() {
            report.rejected.push(rejected(
                key,
                constraint,
                format!("RM-4: no such stream s{number}"),
            ));
            return;
        }
        match coerce_numeric(
            key,
            constraint,
            &self.rates,
            Some(&Value::from(num(DEFAULT_SAMPLE_RATE_HZ))),
            self.whole_hertz_rates,
        ) {
            Ok((value, coercion)) => {
                report.applied.insert(key.clone(), value);
                if let Some(coercion) = coercion {
                    report.coercions.push(coercion);
                }
            }
            Err(reason) => report.rejected.push(rejected(key, constraint, reason)),
        }
    }

    /// RM-31's and RM-32's judgement of a whole configuration (RM-30): each violation's
    /// key and why, with no request. RM-31 names the receive channel's `antenna` key and
    /// RM-32 the direction's first stream that holds a channel, its rate key.
    pub fn violations(&self, configuration: &BTreeMap<Key, Value>) -> Vec<(Key, String)> {
        let mut violations = self.full_duplex(configuration);
        violations.extend(self.throughput(configuration));
        violations
    }

    /// The key of `request` an instance-wide violation is charged to (RM-30): the
    /// violation's own key when the request names it, otherwise the key of the change the
    /// request carries that caused it — RM-31's transmit channel's `stream` key, or
    /// RM-32's rate key, or its channel's `stream` key. None when the request names no
    /// key of the violation.
    fn attributed(&self, violation: &Key, request: &Requested) -> Option<Key> {
        if request.constraints.contains_key(violation) {
            return Some(violation.clone());
        }
        let key = match crate::concrete(violation)? {
            Concrete::Channel { rx: true, number, leaf: "antenna" } => self.duplex_stream_key(number),
            Concrete::Stream { rx, .. } => self
                .direction_keys(Direction::of(rx))
                .into_iter()
                .find(|key| request.constraints.contains_key(key)),
            _ => None,
        }?;
        request.constraints.contains_key(&key).then_some(key)
    }

    /// The transmit channel's `stream` key on the duplex board that carries receive
    /// channel `number` (RM-31).
    fn duplex_stream_key(&self, number: u32) -> Option<Key> {
        let rx = self.rx.get(number as usize)?.element?;
        self.tx.iter().enumerate().find_map(|(tx_number, tx)| {
            let tx = tx.element?;
            (tx.timekeeper == rx.timekeeper && tx.slot == rx.slot && tx.front_end == rx.front_end)
                .then(|| channel_setting(Direction::Tx, tx_number as u32, "stream"))
        })
    }

    /// One direction's stream keys, the rate of each stream first, then each channel's
    /// membership (RM-32).
    fn direction_keys(&self, direction: Direction) -> Vec<Key> {
        let count = direction.channels(self).len();
        (0..count)
            .map(|k| stream_setting(direction, k as u32))
            .chain((0..count).map(|number| channel_setting(direction, number as u32, "stream")))
            .collect()
    }

    /// RM-31 on `configuration`: every receive channel of a duplex board whose transmit
    /// channel is in a stream has antenna `RX2`.
    fn full_duplex(&self, configuration: &BTreeMap<Key, Value>) -> Vec<(Key, String)> {
        let mut violations: Vec<(Key, String)> = Vec::new();
        for (tx_number, tx) in self.tx.iter().enumerate() {
            let Some(tx_element) = tx.element else {
                continue;
            };
            let Some(board) = tx_element.front_end.board() else {
                continue;
            };
            if stream_of(configuration, Direction::Tx, tx_number as u32) < 0 {
                continue;
            }
            for (rx_number, rx) in self.rx.iter().enumerate() {
                let Some(rx_element) = rx.element else {
                    continue;
                };
                if rx_element.timekeeper != tx_element.timekeeper
                    || rx_element.slot != tx_element.slot
                    || rx_element.front_end != tx_element.front_end
                {
                    continue;
                }
                let Some(antenna) = configuration
                    .get(&channel_setting(Direction::Rx, rx_number as u32, "antenna"))
                    .and_then(Value::as_scalar)
                    .and_then(text)
                else {
                    continue;
                };
                if antenna == "RX2" {
                    continue;
                }
                let key = channel_setting(Direction::Rx, rx_number as u32, "antenna");
                let reason = format!(
                    "RM-31: receive channel c{rx_number} of `{}` cannot use antenna {antenna} while transmit channel c{tx_number} is in a stream",
                    board.name()
                );
                if !violations.iter().any(|(existing, _)| *existing == key) {
                    violations.push((key, reason));
                }
            }
        }
        violations
    }

    /// RM-32 on `configuration`: in each direction, the channels in a stream at its rate,
    /// against one link's bytes per second.
    fn throughput(&self, configuration: &BTreeMap<Key, Value>) -> Vec<(Key, String)> {
        let mut violations = Vec::new();
        for direction in [Direction::Rx, Direction::Tx] {
            let count = direction.channels(self).len();
            let mut need = 0.0;
            let mut first = None;
            for k in 0..count {
                let members = (0..count)
                    .filter(|number| stream_of(configuration, direction, *number as u32) == k as i64)
                    .count();
                let rate = configuration
                    .get(&stream_setting(direction, k as u32))
                    .and_then(Value::as_scalar)
                    .and_then(numeric)
                    .unwrap_or(0.0);
                if members > 0 && first.is_none() {
                    first = Some(k as u32);
                }
                need += members as f64 * rate;
            }
            let need = need * self.performance.wire_bytes_per_sample as f64;
            let cap = direction.bytes_per_s(&self.performance) as f64;
            if need <= cap {
                continue;
            }
            let Some(k) = first else {
                continue;
            };
            violations.push((
                stream_setting(direction, k),
                format!("RM-32: {} needs {need} B/s, one link carries {cap}", direction.name()),
            ));
        }
        violations
    }
}

impl Channel {
    fn of(element: Element, receive: bool, master_clock_hz: u64) -> Channel {
        let front_end = element.front_end;
        let (rx_antennas, tx_antennas) = front_end.antennas();
        Channel {
            element: Some(element),
            frequency: front_end.frequency(master_clock_hz),
            gain: front_end.gain(),
            antennas: if receive { rx_antennas } else { tx_antennas },
            default_antenna: front_end.default_antenna(receive),
            phase_behavior_on_retune: front_end.phase_behavior_on_retune().to_owned(),
            default_frequency_hz: front_end.default_frequency_hz(),
            path_delay_samples: front_end.path_delay_samples(receive),
        }
    }

    fn ideal(receive: bool) -> Channel {
        Channel {
            element: None,
            frequency: Grid::Step { lo: 0.0, hi: 1_000_000_000_000.0, step: 0.0 },
            gain: Grid::Step { lo: -200.0, hi: 200.0, step: 0.0 },
            antennas: if receive {
                vec!["RX2".to_owned(), "TX/RX".to_owned()]
            } else {
                vec!["TX/RX".to_owned()]
            },
            default_antenna: if receive { "RX2" } else { "TX/RX" }.to_owned(),
            phase_behavior_on_retune: "deterministic".to_owned(),
            default_frequency_hz: 1_000_000_000.0,
            path_delay_samples: 0,
        }
    }
}

/// One direction of a description (RM-27).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Direction {
    Rx,
    Tx,
}

impl Direction {
    /// The direction an RM-27 key's `rx` flag names (RM-27).
    fn of(rx: bool) -> Direction {
        if rx {
            Direction::Rx
        } else {
            Direction::Tx
        }
    }

    fn name(self) -> &'static str {
        match self {
            Direction::Rx => "rx",
            Direction::Tx => "tx",
        }
    }

    fn channels(self, description: &Description) -> &[Channel] {
        match self {
            Direction::Rx => &description.rx,
            Direction::Tx => &description.tx,
        }
    }

    fn bytes_per_s(self, performance: &PerformanceEnvelope) -> i64 {
        match self {
            Direction::Rx => performance.rx_bytes_per_s,
            Direction::Tx => performance.tx_bytes_per_s,
        }
    }
}

/// Whether `key` is one of RM-30's configuration keys, which `effective` holds.
pub fn is_configuration(key: &Key) -> bool {
    match crate::concrete(key) {
        Some(Concrete::Channel { leaf, .. }) => {
            matches!(leaf, "frequency_hz" | "gain_db" | "antenna" | "stream")
        }
        Some(Concrete::Stream { .. }) => true,
        None => false,
    }
}

/// RM-27's update class of the layout configuration key `key`, read from the one
/// `KeyDecl` table ([`crate::radio_keys`]) through the declaration the concrete spelling
/// stands for; `None` when `key` is no layout key or its declaration states no class. A
/// Provider refuses an update of such a key, and one whose stated class differs.
pub fn update_class(key: &Key) -> Option<UpdateClass> {
    let (direction, word, leaf) = match crate::concrete(key)? {
        Concrete::Channel { rx, leaf, .. } => (Direction::of(rx).name(), keys::CHANNEL, leaf),
        Concrete::Stream { rx, .. } => (Direction::of(rx).name(), keys::STREAM, keys::STREAM_LEAF),
    };
    let declaration = Key::parse(&format!("radio.{direction}.{word}.{leaf}")).expect("a declared radio key");
    crate::radio_keys().into_iter().find(|decl| decl.key == declaration)?.update_class
}

fn channel_setting(direction: Direction, number: u32, leaf: &str) -> Key {
    Key::parse(&keys::channel(direction.name(), number, leaf)).expect("a declared radio key")
}

fn stream_setting(direction: Direction, number: u32) -> Key {
    Key::parse(&keys::stream(direction.name(), number)).expect("a declared radio key")
}

fn default_stream(direction: Direction, number: u32) -> i64 {
    if direction == Direction::Rx && number == 0 { 0 } else { -1 }
}

fn stream_of(effective: &BTreeMap<Key, Value>, direction: Direction, number: u32) -> i64 {
    effective
        .get(&channel_setting(direction, number, "stream"))
        .and_then(Value::as_scalar)
        .and_then(integer)
        .unwrap_or(-1)
}

fn rates(grid: &Grid) -> CapabilityValue {
    match grid {
        Grid::Values(values) => CapabilityValue::AnyOf {
            values: values.iter().rev().map(|rate| num(*rate)).collect(),
        },
        grid => CapabilityValue::Range { min: num(grid.min()), max: num(grid.max()) },
    }
}

fn coerce_capability(
    key: &Key,
    constraint: &Constraint,
    capability: &CapabilityValue,
    report: &mut CoerceReport,
) {
    match satisfies(constraint, capability) {
        Ok(true) => {
            if let CapabilityValue::One { value } = capability {
                report.applied.insert(key.clone(), Value::from(value.clone()));
            }
        }
        _ => report.rejected.push(rejected(
            key,
            constraint,
            format!("RM-8: the device declares {capability:?}"),
        )),
    }
}

fn coerce_stream(constraint: &Constraint, max: i64, default: i64) -> Result<(Value, Option<Coercion>), String> {
    let grid = Grid::Integer { lo: -1, hi: max };
    let number = match constraint {
        Constraint::Eq { value: Scalar::Int(value) }
        | Constraint::Min { value: Scalar::Int(value) }
        | Constraint::Max { value: Scalar::Int(value) } => *value as f64,
        Constraint::Range { min, max: upper } => {
            let lower = min.as_ref().map(integer_bound).transpose()?.unwrap_or(-1) as f64;
            let upper = upper.as_ref().map(integer_bound).transpose()?.unwrap_or(max) as f64;
            let Some(value) = grid.at_or_above(lower).filter(|value| *value <= upper) else {
                return Err(format!("RM-8: no stream satisfies {constraint:?}"));
            };
            return Ok((Value::from(value as i64), None));
        }
        Constraint::Set { values } => {
            let Some(value) = values
                .iter()
                .filter_map(|value| integer_bound(value).ok())
                .find(|value| *value >= -1 && *value <= max)
            else {
                return Err(format!("RM-8: no stream satisfies {constraint:?}"));
            };
            return Ok((Value::from(value), None));
        }
        Constraint::Present {} => return Ok((Value::from(default), None)),
        _ => return Err("RM-4: a channel's stream must be an integer".to_owned()),
    };
    let selected = match constraint {
        Constraint::Eq { .. } => grid.nearest(number),
        Constraint::Min { .. } => grid.at_or_above(number),
        Constraint::Max { .. } => grid.at_or_below(number),
        _ => None,
    }
    .filter(|value| value.fract() == 0.0)
    .ok_or_else(|| format!("RM-8: stream is outside -1..={max}"))?;
    Ok((Value::from(selected as i64), None))
}

fn integer_bound(value: &Scalar) -> Result<i64, String> {
    match value {
        Scalar::Int(value) => Ok(*value),
        _ => Err("RM-4: a channel's stream must be an integer".to_owned()),
    }
}

fn integer(scalar: &Scalar) -> Option<i64> {
    match scalar {
        Scalar::Int(value) => Some(*value),
        _ => None,
    }
}

fn numeric(scalar: &Scalar) -> Option<f64> {
    match scalar {
        Scalar::Int(value) => Some(*value as f64),
        Scalar::Num(value) => Some(value.get()),
        _ => None,
    }
}

fn text(scalar: &Scalar) -> Option<&str> {
    match scalar {
        Scalar::Str(value) => Some(value),
        _ => None,
    }
}
