"""The Easy API: a Session and its radios (spec 16 EA-16, EA-17).

Every change is a Kernel ``SessionAction`` the server submits, and every advance of
time is the server's (EA-1). What this module adds are names — ``rx.frequency`` for
the Radio Model key ``radio.rx.frequency_hz`` — and ``capture``, composed of a submit,
a wait and a read.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

import numpy as np

from ._client import Connection, Error, ProtocolError

CAPTURE_WRITTEN = "sink.CAPTURE_WRITTEN"
REQUEST_REJECTED = "sink.REQUEST_REJECTED"


class Rejected(Error):
    """The Kernel rejected a call, and logged it (Vision §3); ``entry`` is the log entry."""

    def __init__(self, entry: dict, event: Optional[dict] = None):
        outcome = entry.get("outcome", {}) if entry else {}
        self.entry = entry
        self.event = event
        self.violations = outcome.get("violations", [])
        reasons = "; ".join(f"{v['check']}: {v['reason']}" for v in self.violations)
        if event is not None:
            reasons = f"{event['kind']}: {event['payload'].get('reason', event['payload'])}"
        super().__init__(reasons or "rejected")


class CaptureTimeout(Error):
    """No capture was written within the timeout, in Run time."""


def _rid(path: str) -> dict:
    return {"node": 0, "path": path}


def _admitted(entry: dict) -> dict:
    if entry["outcome"]["kind"] != "admitted":
        raise Rejected(entry)
    return entry


def waveform(x: Any, id: str = "waveform") -> Tuple[dict, bytes]:
    """An ``ArtifactRef`` document for ``x`` and its bytes (EA-16, EA-17), for a Spec's
    ``inputs`` or schedule; the hash is the Kernel's (``sha256:`` of the bytes)."""
    data = _cf32(x)[1]
    digest = "sha256:" + hashlib.sha256(data).hexdigest()
    reference = {
        "id": id,
        "kind": "ezsdr.input",
        "uri": f"mem:{digest}",
        "hash": digest,
        "size_bytes": len(data),
        "partial": False,
        "marks": [],
        "continuity": [],
    }
    return reference, data


def _cf32(x: Any) -> Tuple[int, bytes]:
    """The channel count and the little-endian, channel-interleaved cf32 bytes (EA-17)."""
    array = np.asarray(x)
    if array.ndim == 1:
        return 1, array.astype("<c8").tobytes()
    if array.ndim == 2:
        return array.shape[0], np.ascontiguousarray(array.T).astype("<c8").tobytes()
    raise Error("a waveform is a 1-D array or a (channels, n) array")


def samples(data: bytes, artifact: dict) -> np.ndarray:
    """A capture artifact's bytes as ``complex64``: ``(n,)`` for one channel, ``(channels, n)``
    otherwise, with the channel count its continuity records (EA-17)."""
    maps = artifact.get("continuity", [])
    counts = {int(m["channels"]) for m in maps}
    if len(counts) != 1:
        raise Error(f"artifact {artifact['id']} does not have one channel count: {sorted(counts)}")
    domains = {json.dumps(m["domain"], sort_keys=True) for m in maps}
    if len(domains) != 1:
        raise Error(
            f"artifact {artifact['id']} spans {len(domains)} SampleClocks (a rate change, §23): "
            "one array would hide it; read its bytes with Session.read and split them by its continuity"
        )
    broken = any(m.get("gaps") or m.get("channel_gaps") or any(len(segments) != 1 for segments in m.get("valid", [])) for m in maps)
    if broken:
        raise Error(
            f"artifact {artifact['id']} has a gap or an invalid stretch (§23: a gap is a flag and a time jump): "
            "one array would hide it; read its bytes with Session.read and split them by its continuity"
        )
    channels = counts.pop()
    frames = np.frombuffer(data, dtype="<c8").reshape(-1, channels).T
    return frames[0].copy() if channels == 1 else frames.copy()


_PARAMS = {
    "frequency": ("frequency_hz", float),
    "sample_rate": ("sample_rate_hz", float),
    "gain": ("gain_db", float),
    "antenna": ("antenna", str),
    "channels": ("channels", int),
}


def _param(name: str) -> property:
    suffix, convert = _PARAMS[name]

    def get(self: "_Side") -> Any:
        return self._session.effective[self._radio][self._key(suffix)]

    def set_(self: "_Side", value: Any) -> None:
        self._session.set(self._radio, self._key(suffix), convert(value))

    return property(get, set_, doc=f"The effective ``radio.<rx|tx>.{suffix}``; setting it submits SetParameter.")


class _Side:
    def __init__(self, session: "Session", radio: str, direction: str):
        self._session = session
        self._radio = radio
        self._direction = direction

    def _key(self, suffix: str) -> str:
        return f"radio.{self._direction}.{suffix}"

    frequency = _param("frequency")
    sample_rate = _param("sample_rate")
    gain = _param("gain")
    antenna = _param("antenna")
    channels = _param("channels")

    def stop(self) -> dict:
        """Submits ``Stop`` for this side's stream."""
        return self._session.stop(f"{self._radio}/{self._direction}")


class CaptureRequest:
    """A capture request the recorder has been sent and not yet answered (EA-17):
    ``Rx.request`` makes one, ``Rx.result`` waits for its samples."""

    def __init__(self, recorder: str, number: int, n: int, start: int, entry: dict):
        self.recorder = recorder
        self.number = number
        self.n = n
        self.entry = entry
        self._start = start


class Rx(_Side):
    """A radio's receive side."""

    def capture(self, n: int, at: Optional[dict] = None, timeout: Optional[float] = None) -> np.ndarray:
        """Captures ``n`` samples from this radio's recorder and returns them (EA-17).
        ``at`` is a ``TimePoint``; ``timeout`` is in seconds of Run time."""
        return self.result(self.request(n, at), timeout)

    def request(self, n: int, at: Optional[dict] = None) -> CaptureRequest:
        """Asks the recorder for ``n`` samples and returns at once (EA-17). Requests are
        served one after another, each from the end of the one before it (HD-10), so
        requests made before the earlier ones are answered capture contiguous samples."""
        recorder = self._session._recorder(self._radio)
        start = self._session._status()["events"]
        # The Sink numbers the capture requests it receives (HD-16); this one is the next.
        number = self._session._captures.get(recorder, 0)
        action = {
            "kind": "vocabulary",
            "ns": "sink",
            "verb": "capture",
            "target": _rid(recorder),
            "at": at,
            "params": {"sink.capture_samples": int(n)},
        }
        entry = _admitted(self._session.submit(action))
        return CaptureRequest(recorder, number, int(n), start, entry)

    def result(self, handle: CaptureRequest, timeout: Optional[float] = None) -> np.ndarray:
        """Waits, in Run time, for ``handle``'s capture and returns its samples (EA-17);
        ``timeout`` defaults to ``n / rate + 1`` seconds."""
        n, number, entry, start = handle.n, handle.number, handle.entry, handle._start
        if timeout is None:
            timeout = n / float(self.sample_rate) + 1.0
        source = _rid(f"sink/{handle.recorder}")
        wait = {"within_ns": max(0, math.ceil(timeout * 1e9))}
        while True:
            request = {"op": "wait_for", "kinds": [CAPTURE_WRITTEN, REQUEST_REJECTED], "from": start}
            result, _ = self._session._call({**request, **wait})
            if result["index"] is None:
                raise CaptureTimeout(f"no capture of {n} samples was written within {timeout} s of Run time")
            # Waiting again keeps the first wait's deadline (Review H, P2-5).
            wait = {"until": result["horizon"]}
            start = result["index"] + 1
            event = result["event"]
            if event["source"] != source or event["payload"].get("request") != number:
                continue
            if event["kind"] == REQUEST_REJECTED:
                raise Rejected(entry, event)
            artifact = event["payload"]["artifact"]
            return samples(self._session.read(artifact), artifact)


class Tx(_Side):
    """A radio's transmit side."""

    def repeat(self, x: Any) -> dict:
        """Transmits ``x`` repeatedly from the next instant the radio allows: sets
        ``radio.tx.channels`` to its channel count when it differs, then submits
        ``radio.start_repeat`` with its bytes (EA-16)."""
        channels, data = _cf32(x)
        if self.channels != channels:
            self.channels = channels
        action = {
            "kind": "vocabulary",
            "ns": "radio",
            "verb": "start_repeat",
            "target": _rid(f"{self._radio}/tx"),
            "at": None,
            "params": {},
        }
        return _admitted(self._session.submit(action, data))


class Radio:
    """A radio binding of the Session's profile."""

    def __init__(self, session: "Session", name: str):
        self.name = name
        self.rx = Rx(session, name, "rx")
        self.tx = Tx(session, name, "tx")


class RunResult:
    """What ``Session.run`` returns: the Session's log entry and the child's Manifest."""

    def __init__(self, session: "Session", entry: dict, manifest: dict, path: str):
        self._session = session
        self.entry = entry
        self.manifest = manifest
        self.path = path

    @property
    def termination(self) -> dict:
        return self.manifest["termination"]["reason"]

    def capture(self, output: str) -> np.ndarray:
        """The artifact the child's output ``output`` wrote, as ``Rx.capture`` returns one."""
        for artifact in self.manifest["artifacts"]:
            if artifact["id"] == output:
                return samples(self._session.read(artifact), artifact)
        raise Error(f"the child Run wrote no artifact {output}")


class Session:
    """A Session: a Run whose Spec is implicit and whose Manifest records an action log
    (Vision §3). Leaving the ``with`` block ends it and keeps its Manifest."""

    def __init__(self, connection: Connection, connected: dict):
        self._connection = connection
        self.run_id: str = connected["run"]
        self.dir: str = connected["dir"]
        self.profile: dict = connected["profile"]
        self.start_instant: dict = connected["start_instant"]
        self._now: dict = connected["now"]
        self._waited = 0
        # Capture requests admitted per recorder: the Sink's next request number (HD-16).
        self._captures: Dict[str, int] = {}
        self.manifest: Optional[dict] = None
        self.manifest_path: Optional[str] = None

    # -- the protocol

    def _call(self, request: dict, body: bytes = b"") -> Tuple[dict, bytes]:
        result, data = self._connection.call(request, body)
        if "now" in result:
            self._now = result["now"]
        return result, data

    def _status(self) -> dict:
        return self._call({"op": "status"})[0]

    # -- time and state

    @property
    def now(self) -> dict:
        """The Run's current instant, as a ``TimePoint`` document."""
        return self._status()["now"]

    @property
    def effective(self) -> Dict[str, Dict[str, Any]]:
        """The effective configuration per fragment: what the Kernel admitted (KC-27)."""
        return self._status()["effective"]

    def sleep(self, seconds: float) -> None:
        """``run.wait_until(now + seconds)`` in the Run's time (Vision §54); never ``time.sleep``."""
        self._call({"op": "advance", "by_ns": max(0, math.ceil(seconds * 1e9))})

    def wait_until(self, t: dict) -> None:
        """Advances the Run to the ``TimePoint`` ``t`` (Vision §15)."""
        self._call({"op": "advance", "to": t})

    def wait_for(self, kinds: Sequence[str], timeout: float) -> Optional[dict]:
        """Waits, in Run time, for the next event of one of ``kinds`` that this method has not
        yet returned (Vision §15); returns it, or ``None`` after ``timeout`` seconds."""
        result, _ = self._call(
            {"op": "wait_for", "kinds": list(kinds), "from": self._waited, "within_ns": max(0, math.ceil(timeout * 1e9))}
        )
        if result["index"] is None:
            return None
        self._waited = result["index"] + 1
        return result["event"]

    def events(self, start: int = 0) -> List[dict]:
        """The events the Run has delivered, from index ``start``."""
        return self._call({"op": "events", "from": start})[0]["events"]

    # -- actions

    def submit(self, action: dict, waveform: Optional[bytes] = None) -> dict:
        """Submits any ``SessionAction`` document and returns its log entry, admitted or rejected."""
        entry = self._call({"op": "submit", "action": action}, waveform or b"")[0]["entry"]
        recorder = self._capture_recorder(action) if entry["outcome"]["kind"] == "admitted" else None
        if recorder is not None:
            # One admitted capture request is one request the recorder numbers (HD-16).
            self._captures[recorder] = self._captures.get(recorder, 0) + 1
        return entry

    def _capture_recorder(self, action: dict) -> Optional[str]:
        """The recorder an admitted action hands a capture request to, as the Kernel routes
        it (RS-14): a ``sink.capture`` goes to the only recorder when one is bound, or else to
        the one its target names; a ``SetParameter`` of ``sink.capture_samples`` to the
        recorder ``sink/<recorder>`` names (Review I, P0-A)."""
        recorders = self.recorders()
        path = (action.get("target") or {}).get("path", "")
        named = path[len("sink/"):] if path.startswith("sink/") else path
        if action.get("kind") == "vocabulary" and (action.get("ns"), action.get("verb")) == ("sink", "capture"):
            if len(recorders) == 1:
                return recorders[0]
            return named if named in recorders else None
        if action.get("kind") == "set_parameter" and action.get("key") == "sink.capture_samples":
            return named if named in recorders else None
        return None

    def set(self, target: str, key: str, value: Any) -> dict:
        """``SetParameter``; raises ``Rejected`` if the Kernel rejects it."""
        return _admitted(self.submit({"kind": "set_parameter", "target": _rid(target), "key": key, "value": value}))

    def stop(self, target: Optional[str] = None) -> dict:
        """``Stop``: of ``target``, or of the Session itself when it is ``None``."""
        return _admitted(self.submit({"kind": "stop", "target": None if target is None else _rid(target)}))

    # -- radios

    def radios(self) -> List[str]:
        """The fragments whose effective configuration holds Radio Model keys."""
        return sorted(name for name, config in self.effective.items() if any(k.startswith("radio.") for k in config))

    def radio(self, name: Optional[str] = None) -> Radio:
        """The named radio, or the Session's only one."""
        if name is None:
            radios = self.radios()
            if len(radios) != 1:
                raise Error(f"the Session has {len(radios)} radios {radios}; name one")
            name = radios[0]
        return Radio(self, name)

    @property
    def rx(self) -> Rx:
        return self.radio().rx

    @property
    def tx(self) -> Tx:
        return self.radio().tx

    def recorders(self) -> List[str]:
        """The bindings of the Session's profile that record a feed."""
        return sorted(name for name, binding in self.profile["bindings"].items() if binding.get("feed"))

    def _recorder(self, radio: str) -> str:
        """The binding whose feed starts at ``radio``'s receive port (EA-17)."""
        for name, binding in self.profile["bindings"].items():
            port = (binding.get("feed") or {}).get("port", {})
            if port.get("component") == radio and port.get("port") == "rx":
                return name
        raise Error(f"no recorder is fed from {radio}'s receive port in the Session's profile")

    # -- artifacts and child Runs

    def read(self, artifact: Any) -> bytes:
        """The bytes of an artifact (an ``ArtifactRef`` document or its URI) this Session produced."""
        uri = artifact["uri"] if isinstance(artifact, dict) else artifact
        return self._call({"op": "read", "uri": uri})[1]

    def run(
        self,
        spec: dict,
        profile: Optional[dict] = None,
        inputs: Iterable[Tuple[dict, bytes]] = (),
        duration: Optional[float] = None,
    ) -> RunResult:
        """Runs ``spec`` as a child Run under this Session's Lease (Vision §54): for
        ``duration`` seconds after its start, or until its schedule stops it."""
        pairs = list(inputs)
        request: Dict[str, Any] = {"op": "run_child", "spec": spec, "inputs": [len(data) for _, data in pairs]}
        if profile is not None:
            request["profile"] = profile
        if duration is not None:
            request["duration_ns"] = max(0, math.ceil(duration * 1e9))
        result, _ = self._call(request, b"".join(data for _, data in pairs))
        entry = _admitted(result["entry"])
        return RunResult(self, entry, result["manifest"], result["path"])

    # -- the end

    def close(self) -> Optional[dict]:
        """Ends the Session and returns its Manifest, also kept in ``manifest``."""
        if self.manifest is None:
            try:
                result, _ = self._call({"op": "finish"})
                self.manifest, self.manifest_path = result["manifest"], result["path"]
            except ProtocolError:
                # The server has gone; every exit of it writes the Manifest (EA-15).
                written = os.path.join(self.dir, "manifest.json")
                if not os.path.exists(written):
                    raise
                with open(written, encoding="utf-8") as file:
                    self.manifest, self.manifest_path = json.load(file), written
            finally:
                self._connection.close()
        return self.manifest

    def __enter__(self) -> "Session":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()


def connect(
    profile: Optional[dict] = None,
    *,
    server: Optional[str] = None,
    runs_dir: Optional[str] = None,
    lease: Optional[dict] = None,
) -> Session:
    """Opens a Session (Vision §3): the server's default profile when ``profile`` is
    ``None``. The Session stands at T0, the instant its devices start."""
    connection = Connection(server, runs_dir)
    request: Dict[str, Any] = {"op": "connect"}
    if profile is not None:
        request["profile"] = profile
    if lease is not None:
        request["lease"] = lease
    try:
        result, _ = connection.call(request)
    except BaseException:
        connection.close()
        raise
    return Session(connection, result)
