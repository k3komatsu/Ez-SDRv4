//! Ez-SDR v4 Module ezsdr.sink.capture 1.1.0 (design/10-host-data-path.md).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod sigmf;
pub use sigmf::sigmf_meta;

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use ezsdr_hostmem::interleave;
use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::event::{Action, Event, EventKind, EventSink, Severity};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{
    ActionReceiver, Deployment, Endpoint, KERNEL_API, ModuleDescriptor, ModuleError, ModuleRef,
    Role, Sink, SinkDescriptor, StepOutcome, StopMode, Version, VersionReq,
    VocabularyRequirement,
};
use ezsdr_kernel::plan::{Fragment, PrepareReport};
use ezsdr_kernel::spec::{Ident, Key, Namespace, OutputReq, Value};
use ezsdr_kernel::stream::{
    BlockFlags, BlockHeader, ContinuityBuilder, ContinuityMap, DataLink, DropCarry, StreamError,
};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockRegistry, Converted, Duration, TimeAuthority, TimeError, TimePoint};
use ezsdr_sink::{
    CAPTURE_ARTIFACT_KIND, CAPTURE_SAMPLES, CAPTURE_WRITTEN, CaptureWrittenPayload, REQUEST_REJECTED,
    RequestRejectedPayload,
};

const CF32_CONTRACT: &str = "ezsdr.stream.cf32";
const SC16_CONTRACT: &str = "ezsdr.stream.sc16";

/// The SigMF specification version the metadata follows (HD-15).
const SIGMF_VERSION: &str = "1.2.6";

fn module_ref() -> ModuleRef {
    ModuleRef {
        id: ModuleId::parse("ezsdr.sink.capture").expect("a valid Module id"),
        version: Version::new(1, 2, 0),
    }
}

/// The Module descriptor for `ezsdr.sink.capture` 1.2.0 (HD-7; Phase 6, VD-1).
pub fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: ModuleId::parse("ezsdr.sink.capture").expect("a valid Module id"),
        version: Version::new(1, 2, 0),
        kernel_api: KERNEL_API,
        roles: vec![Role::Sink],
        vocabularies: vec![VocabularyRequirement {
            id: Namespace::parse("sink").expect("a valid Vocabulary namespace"),
            req: VersionReq(Version::new(1, 1, 0)),
        }],
        deployment: Deployment::InProcess {},
        impl_hash: Some(ContentHash::of_bytes(b"ezsdr.sink.capture 1.2.0")),
    }
}

/// The host-memory formats and artifact kind the capture Sink accepts (HD-7).
pub fn sink_descriptor() -> SinkDescriptor {
    SinkDescriptor {
        module: module_ref(),
        kind: Namespace::parse("ezsdr.sink.capture").expect("a valid Sink kind"),
        memory_domains: vec![ezsdr_hostmem::HOST_MEMORY],
        contracts: vec![
            DataContractId::parse(CF32_CONTRACT).expect("a valid contract id"),
            DataContractId::parse(SC16_CONTRACT).expect("a valid contract id"),
        ],
        artifact_kinds: vec![Namespace::parse(CAPTURE_ARTIFACT_KIND).expect("a valid artifact kind")],
    }
}

struct Capture {
    id: Option<Ident>,
    n: u64,
    at: Option<AbsoluteDeadline>,
    written: u64,
    file: Option<(PathBuf, File)>,
    bps: Option<usize>,
    contract: Option<DataContractId>,
    builders: Vec<ContinuityMap>,
    builder: Option<(ContinuityBuilder, ClockDomainId, u16)>,
    started: bool,
    /// The request's number (HD-16); null for the output's own capture.
    request: Option<u64>,
}

impl Capture {
    fn new(id: Option<Ident>, n: u64, at: Option<AbsoluteDeadline>, request: Option<u64>) -> Capture {
        Capture {
            request,
            id,
            n,
            at,
            written: 0,
            file: None,
            bps: None,
            contract: None,
            builders: Vec::new(),
            builder: None,
            started: false,
        }
    }
}

/// Captures requested stream samples into hashed files and continuity records (HD-7…HD-13).
pub struct CaptureSink {
    dir: PathBuf,
    descriptor: SinkDescriptor,
    output: Option<Ident>,
    run: Option<RunId>,
    clocks: Option<Arc<ClockRegistry>>,
    time: Option<Arc<dyn TimeAuthority>>,
    events: Option<Arc<dyn EventSink>>,
    actions: Option<Arc<dyn ActionReceiver>>,
    link: Option<Arc<dyn DataLink>>,
    queue: VecDeque<Capture>,
    done: Vec<ArtifactRef>,
    requests: u32,
    /// Capture requests received so far, accepted or not: the next one's number (HD-16).
    received: u64,
}

impl CaptureSink {
    /// Builds a capture Sink from its binding, refusing unsupported Module content (HD-8).
    pub fn from_binding(binding: &Binding) -> Result<CaptureSink, ModuleError> {
        if binding.module != module_ref() {
            return Err(ModuleError::rejected(
                "HD-8: binding names a different Module version",
            ));
        }
        if binding.profile.is_some() {
            return Err(ModuleError::rejected(
                "HD-8: the capture Sink has no profiles",
            ));
        }
        if binding.selector.len() != 1 {
            return Err(ModuleError::rejected(
                "HD-8: selector must contain exactly the `dir` string",
            ));
        }
        let dir = match binding.selector.get(&Ident::parse("dir").expect("a valid selector key")) {
            Some(Value::Str(dir)) => PathBuf::from(dir),
            _ => {
                return Err(ModuleError::rejected(
                    "HD-8: selector must contain exactly the `dir` string",
                ));
            }
        };
        Ok(CaptureSink {
            dir,
            descriptor: sink_descriptor(),
            output: None,
            run: None,
            clocks: None,
            time: None,
            events: None,
            actions: None,
            link: None,
            queue: VecDeque::new(),
            done: Vec::new(),
            requests: 0,
            received: 0,
        })
    }

    fn reject(&self, message: impl Into<String>) -> ModuleError {
        ModuleError::rejected(message)
    }

    fn event_for_rejection(&self, action: &str, reason: String, request: Option<u64>) {
        let (Some(events), Some(time), Some(output)) = (&self.events, &self.time, &self.output) else {
            return;
        };
        let root = time.primary_root();
        let Ok(now) = time.now(root) else {
            return;
        };
        let Ok(source) = ezsdr_kernel::id::ResourceId::parse(&format!("sink/{output}")) else {
            return;
        };
        let event = Event {
            source,
            time: now,
            severity: Severity::Warning,
            kind: EventKind::parse(REQUEST_REJECTED).expect("a valid Sink event kind"),
            payload: serde_json::to_value(RequestRejectedPayload {
                action: action.to_owned(),
                reason,
                request,
            })
            .expect("a rejection payload is JSON"),
        };
        let _ = events.emit_control(event);
    }

    fn handle_action(&mut self, action: Action) -> Result<(), ModuleError> {
        match action {
            Action::UpdateParameter { key, value, at, .. }
                if key.as_str() == CAPTURE_SAMPLES =>
            {
                let request = self.received;
                self.received += 1;
                match value {
                    Value::Int(n) if n >= 1 => {
                        self.queue
                            .push_back(Capture::new(None, n as u64, at, Some(request)));
                    }
                    _ => self.event_for_rejection(
                        "update_parameter",
                        "HD-14: sink.capture_samples must be a positive Int".to_owned(),
                        Some(request),
                    ),
                }
            }
            Action::Stop { target: Some(target) }
                if self.output.as_ref().is_some_and(|output| {
                    ResourceId::parse(&format!("sink/{output}")).is_ok_and(|sink_target| target == sink_target)
                }) =>
            {
                if self.queue.front().is_some_and(|capture| capture.started) {
                    let carry = self.link.as_ref()
                        .map_or_else(DropCarry::default, |link| link.take_drop_carry());
                    self.finish_front(true, carry)?;
                }
                // A discarded request is answered, so its client does not wait out its
                // timeout (HD-11; Review I, P2-1).
                for discarded in std::mem::take(&mut self.queue) {
                    if discarded.request.is_some() {
                        self.event_for_rejection(
                            "update_parameter",
                            "HD-11: discarded by a Stop for the Sink".to_owned(),
                            discarded.request,
                        );
                    }
                }
            }
            other => {
                let kind = action_kind(&other);
                self.event_for_rejection(
                    kind,
                    format!("HD-14: Action kind `{kind}` is not supported by the capture Sink"),
                    None,
                );
            }
        }
        Ok(())
    }

    fn process_block(&mut self, block: &ezsdr_kernel::stream::BlockRef, carry: DropCarry) -> Result<(), ModuleError> {
        let recording_before_block = self.queue.front().is_some_and(|capture| capture.started);
        let carry = if recording_before_block {
            carry
        } else {
            DropCarry::default()
        };
        let header = block.header().clone();
        let bps = match header.contract.as_str() {
            CF32_CONTRACT => 8,
            SC16_CONTRACT => 4,
            contract => {
                return Err(self.reject(format!(
                    "HD-10: contract {contract} is not supported by the capture Sink"
                )));
            }
        };
        let clocks = self
            .clocks
            .as_ref()
            .expect("prepare installs the ClockRegistry")
            .clone();
        let mut carry_pending = recording_before_block;
        // A capture that starts in this block: the lower bound its leading gap is
        // clipped to (HD-10).
        let mut starting: Option<Option<i64>> = None;
        let mut sample = 0_usize;
        while sample < header.len as usize {
            let Some(at) = self.queue.front().map(|capture| capture.at) else {
                break;
            };
            let is_started = self.queue.front().is_some_and(|capture| capture.started);
            if !is_started {
                let lower = match lower_tick(&header, at, &clocks) {
                    Ok(lower) => lower,
                    Err(error) => {
                        let request = self.queue.pop_front().and_then(|capture| capture.request);
                        self.event_for_rejection(
                            "update_parameter",
                            format!("HD-14: capture start time cannot be converted: {error}"),
                            request,
                        );
                        continue;
                    }
                };
                let Some(start) = sample_at_or_after(sample, &header, lower) else {
                    break;
                };
                sample = start;
                starting = Some(lower);
                self.start_front(bps, header.contract.clone())?;
            }

            let same_contract = self
                .queue
                .front()
                .is_some_and(|capture| capture.contract.as_ref() == Some(&header.contract));
            if !same_contract {
                let trailing_carry = if carry_pending {
                    carry_pending = false;
                    carry
                } else {
                    DropCarry::default()
                };
                self.finish_front(true, trailing_carry)?;
                continue;
            }

            let available = header.len as usize - sample;
            let needed = self
                .queue
                .front()
                .expect("a capture is queued")
                .n
                .saturating_sub(self.queue.front().expect("a capture is queued").written);
            let take = available.min(usize::try_from(needed).unwrap_or(usize::MAX));
            if take == 0 {
                self.finish_front(false, DropCarry::default())?;
                continue;
            }
            let mut overlap = sliced_header(&header, sample, take)?;
            if let Some(lower) = starting.take() {
                clip_leading_gap(&mut overlap, lower);
            }
            let push_carry = if carry_pending {
                carry_pending = false;
                carry
            } else {
                DropCarry::default()
            };
            {
                let capture = self.queue.front_mut().expect("a capture is queued");
                push_continuity(capture, &overlap, push_carry)?;
            }

            let bytes = interleave(block, bps, sample, sample + take);
            let expected_bytes = take
                .checked_mul(header.channels as usize)
                .and_then(|n| n.checked_mul(bps))
                .ok_or_else(|| self.reject("HD-10: capture byte count overflows"))?;
            if bytes.len() != expected_bytes {
                return Err(self.reject(
                    "HD-10: block has no readable host bytes for its declared contract",
                ));
            }
            let capture = self.queue.front_mut().expect("a capture is queued");
            let (path, file) = capture
                .file
                .as_mut()
                .ok_or_else(|| ModuleError::rejected("HD-10: capture file was not opened"))?;
            file.write_all(&bytes).map_err(|error| {
                ModuleError::rejected(format!("HD-10: cannot write {}: {error}", path.display()))
            })?;
            capture.written = capture.written.saturating_add(take as u64);
            sample += take;
            if capture.written == capture.n {
                self.finish_front(false, DropCarry::default())?;
            }
        }
        Ok(())
    }

    fn start_front(&mut self, bps: usize, contract: DataContractId) -> Result<(), ModuleError> {
        let id = if let Some(id) = self.queue.front().and_then(|capture| capture.id.clone()) {
            id
        } else {
            let output = self
                .output
                .as_ref()
                .ok_or_else(|| ModuleError::rejected("HD-10: output id is missing"))?;
            let id = Ident::parse(&format!("{output}_{}", self.requests))
                .map_err(|error| ModuleError::rejected(format!("HD-10: {error}")))?;
            self.requests = self
                .requests
                .checked_add(1)
                .ok_or_else(|| ModuleError::rejected("HD-10: capture request counter overflow"))?;
            id
        };
        let run = self
            .run
            .as_ref()
            .ok_or_else(|| ModuleError::rejected("HD-10: Run id is missing"))?;
        let safe_run: String = run
            .as_str()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        // HD-15: the capture's file is a SigMF Dataset, whatever its contract.
        let file_name = format!("{safe_run}_{id}.sigmf-data");
        let absolute = std::path::absolute(self.dir.join(file_name))
            .map_err(|error| ModuleError::rejected(format!("HD-10: cannot resolve capture path: {error}")))?;
        let file = File::create_new(&absolute).map_err(|error| {
            ModuleError::rejected(format!("HD-10: cannot create {}: {error}", absolute.display()))
        })?;
        let capture = self.queue.front_mut().expect("a capture is queued");
        capture.id = Some(id);
        capture.file = Some((absolute, file));
        capture.bps = Some(bps);
        capture.contract = Some(contract);
        capture.started = true;
        Ok(())
    }

    fn finish_front(&mut self, partial: bool, carry: DropCarry) -> Result<(), ModuleError> {
        let Some(mut capture) = self.queue.pop_front() else {
            return Ok(());
        };
        if let Some((builder, _, _)) = capture.builder.take() {
            capture.builders.push(builder.finish(carry));
        }
        if !capture.started {
            return Ok(());
        }
        let Some((path, mut file)) = capture.file.take() else {
            return Err(ModuleError::rejected("HD-10: started capture has no file"));
        };
        file.flush()
            .map_err(|error| ModuleError::rejected(format!("HD-10: cannot flush {}: {error}", path.display())))?;
        drop(file);
        let bytes = fs::read(&path)
            .map_err(|error| ModuleError::rejected(format!("HD-10: cannot read {}: {error}", path.display())))?;
        let id = capture
            .id
            .take()
            .ok_or_else(|| ModuleError::rejected("HD-10: started capture has no id"))?;
        self.done.push(ArtifactRef {
            id,
            kind: Namespace::parse(CAPTURE_ARTIFACT_KIND).expect("a valid artifact kind"),
            uri: format!("file://{}", path.display()),
            hash: ContentHash::of_bytes(&bytes),
            size_bytes: bytes.len() as u64,
            partial,
            marks: Vec::new(),
            continuity: capture.builders,
        });
        // HD-15: a Recording has one sample rate and one channel count, so only a
        // capture with a single ContinuityMap gets metadata. The data's ArtifactRef is
        // recorded first, so a failed metadata write cannot cost the capture, and the
        // metadata is renamed into place, so a reader never sees half of it.
        let done = self.done.last().expect("the capture was just recorded");
        if let ([map], Some(contract)) = (done.continuity.as_slice(), capture.contract.as_ref()) {
            let clocks = self.clocks.as_ref().expect("prepare installs the ClockRegistry");
            let rate = clocks
                .nominal_rate(map.domain)
                .map_err(|error| ModuleError::rejected(format!("HD-15: {error}")))?;
            let meta = sigmf_meta(map, rate, contract, partial).map_err(ModuleError::rejected)?;
            let meta_path = path.with_extension("sigmf-meta");
            let staged = path.with_extension("sigmf-meta.partial");
            let text = serde_json::to_string_pretty(&meta).expect("SigMF metadata is JSON");
            fs::write(&staged, text)
                .and_then(|()| fs::rename(&staged, &meta_path))
                .map_err(|error| {
                    ModuleError::rejected(format!("HD-15: cannot write {}: {error}", meta_path.display()))
                })?;
        }
        self.announce(self.done.last().expect("the capture was just recorded").clone(), capture.request);
        Ok(())
    }

    /// HD-16: `sink.CAPTURE_WRITTEN` with the artifact just recorded and its request's number.
    fn announce(&self, artifact: ArtifactRef, request: Option<u64>) {
        let (Some(events), Some(time), Some(output)) = (&self.events, &self.time, &self.output) else {
            return;
        };
        let Ok(now) = time.now(time.primary_root()) else {
            return;
        };
        let Ok(source) = ezsdr_kernel::id::ResourceId::parse(&format!("sink/{output}")) else {
            return;
        };
        let _ = events.emit_control(Event {
            source,
            time: now,
            severity: Severity::Info,
            kind: EventKind::parse(CAPTURE_WRITTEN).expect("a valid Sink event kind"),
            payload: serde_json::to_value(CaptureWrittenPayload { artifact, request }).expect("an ArtifactRef is JSON"),
        });
    }
}

impl Sink for CaptureSink {
    fn descriptor(&self) -> &SinkDescriptor {
        &self.descriptor
    }

    fn prepare(&mut self, fragment: &Fragment, ctx: ezsdr_kernel::module_api::PrepareContext) -> Result<PrepareReport, ModuleError> {
        let request: OutputReq = serde_json::from_value(fragment.content.clone()).map_err(|error| {
            ModuleError::rejected(format!("HD-9: fragment content is not an OutputReq: {error}"))
        })?;
        let capture_samples = if request.params.is_empty() {
            None
        } else if request.params.len() == 1 {
            match request
                .params
                .get(&Key::parse(CAPTURE_SAMPLES).expect("a valid Sink key"))
            {
                Some(Value::Int(n)) if *n >= 1 => Some(*n as u64),
                Some(_) => {
                    return Err(ModuleError::rejected(
                        "HD-9: sink.capture_samples must be a positive Int",
                    ));
                }
                None => {
                    return Err(ModuleError::rejected(
                        "HD-9: params may contain only sink.capture_samples",
                    ));
                }
            }
        } else {
            return Err(ModuleError::rejected(
                "HD-9: params may contain only sink.capture_samples",
            ));
        };

        if ctx.links.len() != 1 {
            return Err(ModuleError::rejected(
                "HD-9: the Sink needs exactly one input link",
            ));
        }
        let attached = &ctx.links[0];
        if attached.component != fragment.id || attached.port.as_str() != "in" {
            return Err(ModuleError::rejected(
                "HD-9: the input link must attach to this fragment's `in` port",
            ));
        }
        let Endpoint::StreamIn(link) = &attached.endpoint else {
            return Err(ModuleError::rejected(
                "HD-9: the input link must be a StreamIn endpoint",
            ));
        };
        fs::create_dir_all(&self.dir)
            .map_err(|error| ModuleError::rejected(format!("HD-9: cannot create capture directory: {error}")))?;

        self.output = Some(request.id.clone());
        self.run = Some(ctx.run);
        self.clocks = Some(ctx.clocks);
        self.time = Some(ctx.time);
        self.events = Some(ctx.events);
        self.actions = Some(ctx.actions);
        self.link = Some(link.clone());
        self.queue.clear();
        self.done.clear();
        self.requests = 0;
        self.received = 0;
        if let Some(n) = capture_samples {
            self.queue.push_back(Capture::new(Some(request.id), n, None, None));
        }

        Ok(PrepareReport {
            fragment: fragment.id.clone(),
            effective: BTreeMap::new(),
            coercions: Vec::new(),
            warnings: Vec::new(),
        })
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }

    fn start(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }

    fn step(&mut self, _until: TimePoint) -> Result<StepOutcome, ModuleError> {
        let mut progressed = false;
        if let Some(actions) = self.actions.clone() {
            while let Some(action) = actions.recv() {
                progressed = true;
                self.handle_action(action)?;
            }
        }
        if let Some(link) = self.link.clone() {
            while let Some((block, carry)) = link.receive() {
                progressed = true;
                self.process_block(&block, carry)?;
            }
        }
        Ok(StepOutcome { progressed })
    }

    fn stop(&mut self, _mode: StopMode) -> Result<Vec<ArtifactRef>, ModuleError> {
        if self.queue.front().is_some_and(|capture| capture.started) {
            let carry = self.link.as_ref()
                .map_or_else(DropCarry::default, |link| link.take_drop_carry());
            self.finish_front(true, carry)?;
        }
        self.queue.clear();
        Ok(std::mem::take(&mut self.done))
    }

    fn cleanup(&mut self) {
        self.queue.clear();
        self.done.clear();
        self.link = None;
        self.actions = None;
        self.events = None;
        self.time = None;
        self.clocks = None;
        self.run = None;
        self.output = None;
    }
}

fn action_kind(action: &Action) -> &'static str {
    match action {
        Action::TxBurst { .. } => "tx_burst",
        Action::SetTimer { .. } => "set_timer",
        Action::UpdateParameter { .. } => "update_parameter",
        Action::PeripheralCommand { .. } => "peripheral_command",
        Action::Emit { .. } => "emit",
        Action::Stop { .. } => "stop",
        Action::Abort { .. } => "abort",
    }
}

/// `at` in the block's SampleClock, rounded up (HD-10).
fn lower_tick(
    header: &BlockHeader,
    at: Option<AbsoluteDeadline>,
    clocks: &ClockRegistry,
) -> Result<Option<i64>, TimeError> {
    let Some(at) = at else {
        return Ok(None);
    };
    Ok(Some(match clocks.convert(at.time_point, header.first_sample_time.domain)? {
        Converted::Exact { point } => point.ticks,
        Converted::Inexact { floor, .. } => floor.ticks.checked_add(1).ok_or(TimeError::Overflow)?,
    }))
}

fn sample_at_or_after(from: usize, header: &BlockHeader, lower: Option<i64>) -> Option<usize> {
    // HD-10: requests are served in queue order from the current sample, so the end
    // of the capture before this one is already a lower bound; only `at` adds one.
    let Some(lower) = lower else {
        return Some(from);
    };
    let relative = i128::from(lower) - i128::from(header.first_sample_time.ticks);
    if relative >= i128::from(header.len) {
        return None;
    }
    if relative <= from as i128 {
        return Some(from);
    }
    Some(relative as usize)
}

/// A capture's first header keeps of its leading gap only the samples at or after
/// the capture's start instant `lower`, none without one or when the block starts at
/// it; an unknown `lost` stays unknown (HD-10, SC-13).
fn clip_leading_gap(header: &mut BlockHeader, lower: Option<i64>) {
    if !header.flags.contains(BlockFlags::GAP_BEFORE) {
        return;
    }
    let inside = lower.map_or(0, |lower| header.first_sample_time.ticks.saturating_sub(lower).max(0) as u64);
    if inside > 0 {
        header.lost = header.lost.map(|lost| lost.min(inside));
        return;
    }
    let gap = BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED | BlockFlags::SEQ_DISCONTINUITY | BlockFlags::ALIGNMENT;
    header.flags = BlockFlags::from_bits(header.flags.bits() & !gap.bits());
    header.lost = None;
}

fn sliced_header(header: &BlockHeader, from: usize, len: usize) -> Result<BlockHeader, ModuleError> {
    let offset = i64::try_from(from)
        .map_err(|error| ModuleError::rejected(format!("HD-10: sample offset is invalid: {error}")))?;
    let first_sample_time = header
        .first_sample_time
        .checked_add(Duration::new(header.first_sample_time.domain, offset))
        .map_err(|error| ModuleError::rejected(format!("HD-10: sample time is invalid: {error}")))?;
    let len = u32::try_from(len)
        .map_err(|error| ModuleError::rejected(format!("HD-10: overlap length is invalid: {error}")))?;
    Ok(BlockHeader {
        first_sample_time,
        len,
        channels: header.channels,
        direction: header.direction,
        valid: header.valid,
        flags: if from == 0 { header.flags } else { BlockFlags::NONE },
        lost: if from == 0 { header.lost } else { None },
        contract: header.contract.clone(),
    })
}

fn push_continuity(
    capture: &mut Capture,
    header: &BlockHeader,
    carry: DropCarry,
) -> Result<(), ModuleError> {
    if capture.builder.is_none() {
        let domain = header.first_sample_time.domain;
        let channels = header.channels;
        capture.builder = Some((ContinuityBuilder::new(domain, channels, false), domain, channels));
    }
    let pushed = capture
        .builder
        .as_mut()
        .expect("a continuity builder was created")
        .0
        .push(header, carry);
    match pushed {
        Ok(()) => Ok(()),
        Err(StreamError::DomainChanged { .. } | StreamError::ChannelsChanged { .. }) => {
            // SC-30c: the rejected push left its carry with the outgoing builder.
            let (previous, _, _) = capture.builder.take().expect("a continuity builder exists");
            capture.builders.push(previous.finish(DropCarry::default()));
            let domain = header.first_sample_time.domain;
            let channels = header.channels;
            let mut next = ContinuityBuilder::new(domain, channels, false);
            next.push(header, DropCarry::default())
                .map_err(|error| ModuleError::rejected(format!("HD-10: continuity: {error}")))?;
            capture.builder = Some((next, domain, channels));
            Ok(())
        }
        Err(error) => Err(ModuleError::rejected(format!("HD-10: continuity: {error}"))),
    }
}
