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
from contextlib import contextmanager
from decimal import Decimal
from fractions import Fraction
from typing import Any, Dict, Iterable, Iterator, List, Optional, Sequence, Tuple

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


class Target:
    """What an action addresses, as the Kernel's ``Target`` document (KC-23, EA-16): a
    resource with a sub-path below it, an output (a recorder) or a graph component. A
    plain string passed to ``Session.set`` or ``Session.stop`` is always a resource
    target, ``"radio/rx"`` being ``Target.resource("radio", "rx")``."""

    @staticmethod
    def resource(name: str, path: str = "") -> dict:
        return {"kind": "resource", "resource": name, "path": path}

    @staticmethod
    def output(name: str) -> dict:
        return {"kind": "output", "output": name}

    @staticmethod
    def component(name: str) -> dict:
        return {"kind": "component", "component": name}


def _target(target: Any) -> dict:
    """A ``Target`` document, or a string as a resource target (EA-16)."""
    if isinstance(target, str):
        name, _, path = target.partition("/")
        return Target.resource(name, path)
    return target


def _seconds(value: Any) -> Fraction:
    """``value`` seconds as an exact rational (EA-16, *Seconds*): a binary float as the
    shortest decimal that rounds to it in its own format, an integer, ``Decimal`` or
    ``Fraction`` as its exact value. Another type raises ``TypeError``, a NaN or an infinity
    ``ValueError`` (spec 20, VF-2; issue #40)."""
    if isinstance(value, (bool, np.bool_, np.timedelta64)):
        raise TypeError(f"seconds cannot be a {type(value).__name__}")
    if isinstance(value, np.floating):
        if not np.isfinite(value):
            raise ValueError(f"seconds must be finite, not {value}")
        return Fraction(np.format_float_positional(value, unique=True, trim="-"))
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError(f"seconds must be finite, not {value}")
        return Fraction(repr(float(value)))
    if isinstance(value, Decimal):
        if not value.is_finite():
            raise ValueError(f"seconds must be finite, not {value}")
        return Fraction(value)
    if isinstance(value, Fraction):
        return value
    if isinstance(value, (int, np.integer)):
        return Fraction(int(value))
    raise TypeError(f"seconds must be a number, not {type(value).__name__}")


def _count(value: Any, rate: Fraction, most: int) -> int:
    """``ceil(s · rate)`` for a duration of ``value`` seconds (EA-16): a negative one, or one
    whose count exceeds ``most``, raises ``ValueError``."""
    s = _seconds(value)
    if s < 0:
        raise ValueError(f"a duration cannot be negative: {value} s")
    count = math.ceil(s * rate)
    if count > most:
        raise ValueError(f"a duration of {value} s exceeds its field's {most}")
    return count


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
    broken = any(
        m.get("gaps") or m.get("channel_gaps")
        or len(m.get("valid", [])) != int(m["channels"])
        or any(
            len(segments) != 1
            or segments[0]["start"] != m["first"]
            or segments[0]["len"] != m["end"]["ticks"] - m["first"]["ticks"]
            for segments in m.get("valid", [])
        )
        for m in maps
    )
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
        """Submits ``Stop`` for this side's stream: after ``rx.stop()`` or ``stop("<radio>")``
        the receive stream stays stopped through every parameter change until ``rx.start()``,
        and after ``tx.stop()`` or ``stop("<radio>")`` the next ``repeat`` transmits (RM-16,
        RM-21; EA-16)."""
        return self._session.stop(Target.resource(self._radio, self._direction))


class CaptureRequest:
    """A capture request the recorder has been sent and not yet answered (EA-17):
    ``Rx.request`` makes one, ``Rx.result`` waits for its samples. The recorder's answer
    names the Action it answers, one of the entry's ``dispatched`` ids (HD-16)."""

    def __init__(self, recorder: str, n: int, start: int, entry: dict):
        self.recorder = recorder
        self.n = n
        self.entry = entry
        self._start = start


class Rx(_Side):
    """A radio's receive side."""

    def start(self, at: Optional[dict] = None) -> dict:
        """Submits ``radio.start_rx`` on this radio's receive stream with ``at``, a
        ``TimePoint`` as ``capture`` takes — without one, the Session's current instant, so
        that the start takes effect when the radio receives it — and raises ``Rejected`` if
        the entry is rejected; returns the entry. A receive stream ``stop()`` ended runs again
        from then, on a new SampleClock that begins the radio's start lead later with the
        configuration then in force; on a running stream it changes nothing. Calls act in
        the order made: ``stop()`` then ``start()`` restarts the stream, ``start()`` then
        ``stop()`` leaves it stopped (RM-12, RM-21; EA-16)."""
        action = {
            "kind": "vocabulary",
            "ns": "radio",
            "verb": "start_rx",
            "target": Target.resource(self._radio, "rx"),
            "at": at if at is not None else self._session._aligned or self._session.now,
            "params": {},
        }
        return _admitted(self._session.submit(action))

    def next_at(self, start: dict, period: int, ahead: Any = 0.05) -> dict:
        """The first instant ``start`` plus a whole number of ``period`` receive samples that
        is at least ``ahead`` seconds after the Run's current instant, as a ``TimePoint`` on
        the primary root — where a capture of a waveform repeated from ``start`` begins at its
        first sample again (v3's ``alignSize``; EA-16). It counts on the receive SampleClock
        in force, so it is exact at any rate, and raises ``Error`` when that clock is not the
        one ``start`` fell on (a rate change or a ``stop()`` since) or the stream is stopped."""
        if int(period) <= 0:
            raise Error(f"EA-16: next_at needs a positive period, not {period}")
        if start.get("domain") != self._session._root:
            raise Error("EA-16: next_at needs a start on the primary root, as after() names")
        ahead = self._session._duration(ahead)["ticks"]
        status = self._session._status()
        clocks = [c for c in status["sample_clocks"] if c["stream"]["path"] == f"{self._radio}/rx"]
        if not clocks:
            raise Error(f"EA-16: {self._radio}/rx has no SampleClock yet")
        clock = clocks[-1]
        origin = clock["origin"]["ticks"]
        if clock["ended_at"] is not None or origin > start["ticks"]:
            raise Error(f"EA-16: {self._radio}/rx is not on the SampleClock it ran at start (stopped, or its rate changed since)")
        per = Fraction(clock["root_ticks_per_tick"]["num"], clock["root_ticks_per_tick"]["den"])
        first = math.ceil((start["ticks"] - origin) / per)
        earliest = math.ceil((status["now"]["ticks"] + ahead - origin) / per)
        index = first + max(0, math.ceil(Fraction(earliest - first, int(period)))) * int(period)
        return {"domain": start["domain"], "ticks": origin + math.ceil(index * per)}

    def capture(self, n: int, at: Optional[dict] = None, timeout: Optional[float] = None) -> np.ndarray:
        """Captures ``n`` samples from this radio's recorder and returns them (EA-17).
        ``at`` is a ``TimePoint``; ``timeout`` is in seconds of Run time."""
        if self._session._aligned is not None:
            raise Error("EA-16: capture waits, so it is refused inside aligned; use request, and result after the block")
        if timeout is not None:
            # Read before the request, which would otherwise be sent first (EA-16).
            self._session._duration(timeout)
        return self.result(self.request(n, at), timeout)

    def request(self, n: int, at: Optional[dict] = None) -> CaptureRequest:
        """Asks the recorder for ``n`` samples and returns at once (EA-17). Requests are
        served one after another, each from the end of the one before it (HD-10), so
        requests made before the earlier ones are answered capture contiguous samples."""
        recorder = self._session._recorder(self._radio)
        start = self._session._status()["events"]
        action = {
            "kind": "vocabulary",
            "ns": "sink",
            "verb": "capture",
            "target": Target.output(recorder),
            "at": at if at is not None else self._session._aligned,
            "params": {"sink.capture_samples": int(n)},
        }
        entry = _admitted(self._session.submit(action))
        return CaptureRequest(recorder, int(n), start, entry)

    def result(self, handle: CaptureRequest, timeout: Optional[float] = None) -> np.ndarray:
        """Waits, in Run time, for ``handle``'s capture and returns its samples (EA-17);
        ``timeout`` defaults to ``n / rate + 1`` seconds."""
        n, entry, start = handle.n, handle.entry, handle._start
        if timeout is None:
            timeout = n / float(self.sample_rate) + 1.0
        dispatched = entry["outcome"]["dispatched"]
        wait = {"within": self._session._duration(timeout)}
        while True:
            request = {"op": "wait_for", "kinds": [CAPTURE_WRITTEN, REQUEST_REJECTED], "from": start}
            result, _ = self._session._call({**request, **wait})
            if result["index"] is None:
                raise CaptureTimeout(f"no capture of {n} samples was written within {timeout} s of Run time")
            # Waiting again keeps the first wait's deadline (Review H, P2-5).
            wait = {"until": result["horizon"]}
            start = result["index"] + 1
            event = result["event"]
            # The answer names the Action it answers (HD-16).
            if event["payload"].get("action") not in dispatched:
                continue
            if event["kind"] == REQUEST_REJECTED:
                raise Rejected(entry, event)
            artifact = event["payload"]["artifact"]
            return samples(self._session.read(artifact), artifact)


class Tx(_Side):
    """A radio's transmit side."""

    def repeat(self, x: Any, at: Optional[dict] = None) -> dict:
        """Transmits ``x`` repeatedly: sets ``radio.tx.channels`` to its channel count when
        it differs, then submits ``radio.start_repeat`` with its bytes (EA-16). ``at`` is a
        ``TimePoint``, as ``capture`` takes, at which the first sample goes out — a whole
        second of a root whose epoch is ``pps`` is a PPS edge (UR-7, ``next_pps``); without
        one, ``aligned``'s instant inside its block, else the next instant the radio allows."""
        channels, data = _cf32(x)
        if self.channels != channels:
            self.channels = channels
        action = {
            "kind": "vocabulary",
            "ns": "radio",
            "verb": "start_repeat",
            "target": Target.resource(self._radio, "tx"),
            "at": at if at is not None else self._session._aligned,
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

    def __init__(self, session: "Session", entry: dict, manifest: dict, path: Optional[str]):
        self._session = session
        self.entry = entry
        self.manifest = manifest
        self.path = path

    @property
    def termination(self) -> dict:
        return self.manifest["termination"]["reason"]

    def capture(self, output: str) -> np.ndarray:
        """The artifact the child's output ``output`` wrote, as ``Rx.capture`` returns one:
        the output's own capture, whose id is the output's (HD-10)."""
        for artifact in self.manifest["artifacts"].get(output, []):
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
        # The primary root and its nominal rate, on which a Duration's ticks count (EA-12).
        self._root: dict = connected["now"]["domain"]
        self._rate = Fraction(connected["root_rate"]["num"], connected["root_rate"]["den"])
        # What the primary root's tick zero is, the Kernel's EpochRef (TM-3, EA-10).
        self._epoch: Optional[dict] = connected.get("root_epoch")
        self._waited = 0
        # The instant `aligned` gives the timed calls in its block (EA-16).
        self._aligned: Optional[dict] = None
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

    def _duration(self, seconds: Any) -> dict:
        """``seconds`` as a ``Duration`` on the primary root: ``ceil(s · root_rate)`` ticks,
        at most 2⁶³ − 1 (EA-12, EA-16)."""
        return {"domain": self._root, "ticks": _count(seconds, self._rate, 2**63 - 1)}

    # -- time and state

    @property
    def now(self) -> dict:
        """The Run's current instant, as a ``TimePoint`` document."""
        return self._status()["now"]

    @property
    def effective(self) -> Dict[str, Dict[str, Any]]:
        """The effective configuration per fragment: what the Kernel admitted (KC-27)."""
        return self._status()["effective"]

    @contextmanager
    def aligned(self, at: dict) -> Iterator[dict]:
        """Within the block, ``tx.repeat``, ``rx.start`` and ``rx.request`` of every radio
        that are given no ``at`` take this one, so that what they start begins at the same
        instant (EA-16): ``with sdr.aligned(at): tx1.repeat(x1); tx2.repeat(x2);
        r = rx1.request(n)``, then ``rx1.result(r)`` after it. ``capture``, which waits, is
        refused inside the block: the Run's time would pass ``at`` before the calls after it."""
        if self._aligned is not None:
            raise Error("EA-16: aligned blocks do not nest")
        self._aligned = at
        try:
            yield at
        finally:
            self._aligned = None

    def after(self, seconds: float) -> dict:
        """The instant ``seconds`` after the Run's current one: ``now`` plus
        ``ceil(s · root_rate)`` ticks, with ``s`` read as EA-16's *Seconds* says before
        ``status`` is asked (so ``0.001`` is exactly a thousandth); it may be negative
        (EA-16, VE-6). On a device-paced Session a capture meant to start at an instant
        names one ahead, ``rx.capture(n, at=sdr.after(0.05))`` (EA-17)."""
        s = _seconds(seconds)
        now = self._status()["now"]
        return {**now, "ticks": now["ticks"] + math.ceil(s * self._rate)}

    def next_pps(self, k: int = 1, ahead: Any = 0.05) -> dict:
        """The ``k``-th PPS edge at least ``ahead`` seconds after the Run's current instant, as
        a ``TimePoint`` on the primary root (EA-16): every whole second of the root is a PPS edge
        when its epoch is ``pps`` (a USRP whose time and clock sources are both not internal,
        UR-7). Raises ``Error`` when the epoch is not ``pps`` (an internal source, a simulation)
        or a second is not a whole number of root ticks."""
        if int(k) != k or k < 1:
            raise Error(f"EA-16: next_pps needs k ≥ 1, not {k}")
        if (self._epoch or {}).get("kind") != "pps":
            raise Error(f"EA-16: the primary root is not a PPS root (epoch {self._epoch})")
        if self._rate.denominator != 1:
            raise Error(f"EA-16: a second is not a whole number of root ticks at {self._rate} ticks/s")
        second = self._rate.numerator
        earliest = self._duration(ahead)["ticks"] + self._status()["now"]["ticks"]
        return {"domain": self._root, "ticks": (-(-earliest // second) + int(k) - 1) * second}

    def sleep(self, seconds: float) -> dict:
        """``run.wait_until(now + seconds)`` in the Run's time (Vision §54); never
        ``time.sleep``. Returns the instant the Run stands at (EA-16, VE-6)."""
        return self._call({"op": "advance", "by": self._duration(seconds)})[0]["now"]

    def wait_until(self, t: dict) -> dict:
        """Advances the Run to the ``TimePoint`` ``t`` (Vision §15) and returns the instant
        it stands at (EA-16, VE-6)."""
        return self._call({"op": "advance", "to": t})[0]["now"]

    def wait_for(self, kinds: Sequence[str], timeout: float) -> Optional[dict]:
        """Waits, in Run time, for the next event of one of ``kinds`` that this method has not
        yet returned (Vision §15); returns it, or ``None`` after ``timeout`` seconds."""
        result, _ = self._call(
            {"op": "wait_for", "kinds": list(kinds), "from": self._waited, "within": self._duration(timeout)}
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
        return self._call({"op": "submit", "action": action}, waveform or b"")[0]["entry"]

    def set(self, target: Any, key: str, value: Any) -> dict:
        """``SetParameter`` of ``target``, a ``Target`` or a resource string; raises
        ``Rejected`` if the Kernel rejects it."""
        return _admitted(self.submit({"kind": "set_parameter", "target": _target(target), "key": key, "value": value}))

    def stop(self, target: Any = None) -> dict:
        """``Stop``: of ``target``, a ``Target`` or a resource string, or of the Session
        itself when it is ``None``."""
        return _admitted(self.submit({"kind": "stop", "target": None if target is None else _target(target)}))

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

    def _recorder(self, radio: str) -> str:
        """The one binding whose feed starts at ``radio``'s receive port (EA-17). With none
        or several it raises before any request is sent: the helper never chooses among
        recorders (spec 20, VF-1; issue #42)."""
        found = []
        for name, binding in self.profile["bindings"].items():
            port = (binding.get("feed") or {}).get("port", {})
            if port.get("component") == radio and port.get("port") == "rx":
                found.append(name)
        if not found:
            raise Error(f"EA-17: no recorder is fed from {radio}'s receive port in the Session's profile")
        if len(found) > 1:
            raise Error(f"EA-17: {len(found)} recorders are fed from {radio}'s receive port {sorted(found)}; submit sink.capture naming one")
        return found[0]

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
            request["duration_ns"] = _count(duration, 10**9, 2**64 - 1)
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
