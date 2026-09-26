//! The frames of `ezsdr.protocol` 1 (EA-2…EA-6).

use std::collections::BTreeMap;

use ezsdr_kernel::event::{Event, EventKind};
use ezsdr_kernel::id::RunId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::run::{Lease, RunState, Termination};
use ezsdr_kernel::session::{LogEntry, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Value};
use ezsdr_kernel::time::TimePoint;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The protocol versions this server speaks (EA-3).
pub const SUPPORTED: [u32; 1] = [1];
/// The longest header line a peer may send, in bytes (EA-2).
pub const MAX_HEADER_BYTES: usize = 16 * 1024 * 1024;

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// A client's header line (EA-2).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestFrame {
    /// The request.
    pub request: Request,
    /// How many raw bytes follow the header line (EA-2).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub body_bytes: u64,
}

/// The server's header line (EA-2).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplyFrame {
    /// The reply.
    pub reply: Reply,
    /// How many raw bytes follow the header line (EA-2).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub body_bytes: u64,
}

/// What a client asks (EA-4).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// The first request: the protocol version the client speaks (EA-3).
    Hello {
        /// The version.
        protocol: u32,
    },
    /// Opens the Session, with the default profile when `profile` is absent (EA-9, EA-10).
    Connect {
        /// A BindingProfile document.
        #[serde(default)]
        profile: Option<serde_json::Value>,
        /// The Lease; Attached when absent.
        #[serde(default)]
        lease: Option<Lease>,
    },
    /// Submits one Session Action; the body, if any, is its waveform (EA-11).
    Submit {
        /// The action.
        action: SessionAction,
    },
    /// Advances the Run to `to`, or by `by_ns` nanoseconds; exactly one is given (EA-12).
    Advance {
        /// An instant on the primary root or a domain related to it.
        #[serde(default)]
        to: Option<TimePoint>,
        /// A duration in nanoseconds.
        #[serde(default)]
        by_ns: Option<u64>,
    },
    /// Waits for an event of one of `kinds` at or after index `from` (EA-12, KC-29b).
    WaitFor {
        /// The kinds that end the wait.
        kinds: Vec<EventKind>,
        /// The first event index to consider.
        from: usize,
        /// How long to wait, in nanoseconds of Run time; or else
        #[serde(default)]
        within_ns: Option<u64>,
        /// the instant to wait until (a previous reply's `horizon`, so that a client that
        /// waits again keeps its deadline).
        #[serde(default)]
        until: Option<TimePoint>,
    },
    /// Reads the delivered events from index `from` (EA-12, KC-29a).
    Events {
        /// The first index.
        from: usize,
    },
    /// Reports the Run's state (EA-12).
    Status {},
    /// Reads an artifact this server's Runs reported (EA-13).
    Read {
        /// The artifact's URI.
        uri: String,
    },
    /// Runs a child Spec Run; the body is the inputs' bytes, concatenated (EA-14).
    RunChild {
        /// The ExperimentSpec document.
        spec: serde_json::Value,
        /// The child's BindingProfile document; derived from the Session's when absent.
        #[serde(default)]
        profile: Option<serde_json::Value>,
        /// The size of each input in the body, in order.
        #[serde(default)]
        inputs: Vec<u64>,
        /// How long the child runs after its T0, in nanoseconds.
        #[serde(default)]
        duration_ns: Option<u64>,
    },
    /// Ends the Session and returns its Manifest (EA-15).
    Finish {},
}

/// A result or an error (EA-4, EA-5).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    /// The request was carried out.
    Result(Response),
    /// It was not (EA-5).
    Error(ProtocolError),
}

/// What a carried-out request returns (EA-4).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    /// The versions (EA-3).
    Hello {
        /// The protocol version agreed.
        protocol: u32,
        /// `ezsdr-server <version>`.
        server: String,
        /// The Kernel API version the server's Modules are built against (MA-2).
        kernel_api: String,
    },
    /// The Session is open and stands at its T0 (EA-10).
    Connected {
        /// The Run's id.
        run: RunId,
        /// The Run's current instant, equal to `start_instant`.
        now: TimePoint,
        /// T0.
        start_instant: TimePoint,
        /// The Session directory (EA-8).
        dir: String,
        /// The BindingProfile document the Session runs with.
        profile: serde_json::Value,
        /// The effective configuration (KC-27).
        effective: BTreeMap<Ident, BTreeMap<Key, Value>>,
    },
    /// The logged entry, admitted or rejected (EA-11).
    Submitted {
        /// The entry.
        entry: LogEntry,
        /// The Run's instant after the submission.
        now: TimePoint,
        /// The number of events delivered so far.
        events: usize,
    },
    /// Where the Run stands (EA-12).
    Advanced {
        /// The Run's instant.
        now: TimePoint,
        /// The number of events delivered so far.
        events: usize,
    },
    /// The first matching event, if one came (EA-12).
    Waited {
        /// Its index.
        index: Option<usize>,
        /// The event.
        event: Option<Event>,
        /// The Run's instant.
        now: TimePoint,
        /// The instant the wait would have stood at.
        horizon: TimePoint,
        /// The number of events delivered so far.
        events: usize,
    },
    /// The events from the requested index (EA-12).
    Events {
        /// The events.
        events: Vec<Event>,
        /// The index the next event will take.
        next: usize,
    },
    /// The Run's state (EA-12).
    Status {
        /// The Run's id.
        run: RunId,
        /// Its lifecycle state.
        state: RunState,
        /// Its instant.
        now: TimePoint,
        /// The effective configuration (KC-27).
        effective: BTreeMap<Ident, BTreeMap<Key, Value>>,
        /// The number of events delivered so far.
        events: usize,
    },
    /// The artifact's bytes follow (EA-13).
    Read {
        /// Their number.
        size: u64,
    },
    /// The child Run's entry and, when admitted, its Manifest (EA-14).
    Ran {
        /// The Session's log entry.
        entry: LogEntry,
        /// The child's Manifest.
        manifest: Option<Box<Manifest>>,
        /// Where it was written.
        path: Option<String>,
    },
    /// The Session's Manifest (EA-15).
    Finished {
        /// The Manifest.
        manifest: Box<Manifest>,
        /// Where it was written.
        path: String,
    },
}

/// Why a request was not carried out (EA-5).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// A frame or request out of the protocol.
    Protocol,
    /// A protocol version this server does not speak.
    UnsupportedProtocol,
    /// Documents refused before a Run exists.
    Refused,
    /// `RunHandleError::Malformed`.
    Malformed,
    /// `RunHandleError::Ended`.
    Ended,
    /// `RunHandleError::NotOnPrimaryRoot`.
    NotOnPrimaryRoot,
    /// An artifact this server's Runs did not report.
    NotFound,
    /// A file could not be read or written.
    Io,
}

/// An error reply (EA-5).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProtocolError {
    /// Which kind.
    pub kind: ErrorKind,
    /// What happened.
    pub message: String,
    /// The versions this server speaks, for `unsupported_protocol`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported: Option<Vec<u32>>,
    /// How the Run ended, for `ended`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination: Option<Termination>,
}

impl ProtocolError {
    /// An error of `kind` with `message`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> ProtocolError {
        ProtocolError { kind, message: message.into(), supported: None, termination: None }
    }
}

/// The committed schemas of the two frames (EA-6).
pub fn document_schemas() -> BTreeMap<&'static str, serde_json::Value> {
    let schema = |value: schemars::Schema| serde_json::to_value(value).expect("a generated schema is JSON");
    BTreeMap::from([
        ("request_frame", schema(ezsdr_kernel::schema::generator().into_root_schema_for::<RequestFrame>())),
        ("reply_frame", schema(ezsdr_kernel::schema::generator().into_root_schema_for::<ReplyFrame>())),
    ])
}
