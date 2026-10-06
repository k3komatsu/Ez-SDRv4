"""Phase 6's Python carriers (plan/phase6/00-overview.md §8), run against ezsdr-server.

    cargo build -p ezsdr-server
    EZSDR_SERVER=<target>/debug/ezsdr-server python3 -m unittest discover -s python/tests

The server is required: without it every test fails, naming how to build it (GY-7).
"""

from __future__ import annotations

import copy
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import ezsdr  # noqa: E402

PYTHON_DIR = Path(__file__).resolve().parents[1]


def ramp(n: int = 1000, channel: int = 0) -> np.ndarray:
    """A waveform whose magnitude grows sample by sample, so an offset is unambiguous."""
    k = np.arange(n)
    return (((k + 1) / (2 * n)) * np.exp(1j * (0.01 * k + channel))).astype(np.complex64)


def rotation(y: np.ndarray, x: np.ndarray) -> tuple:
    """The offset at which the repeated ``x`` appears in ``y``, and the unit phasor that
    takes it there (the LO phases of an ``x310-like`` loopback, MR-34), checked on every sample."""
    offset = int(np.argmin(np.abs(np.abs(x) - np.abs(y[0]))))
    shifted = np.resize(np.roll(x, -offset), y.shape)
    phasor = y[0] / shifted[0]
    np.testing.assert_allclose(y, shifted * phasor, atol=1e-5)
    return offset, phasor


class EasyApi(unittest.TestCase):
    def setUp(self) -> None:
        ezsdr._client.find_server()  # fails with the build instruction when absent
        self.runs = tempfile.mkdtemp(prefix="ezsdr-python-")

    def tearDown(self) -> None:
        shutil.rmtree(self.runs, ignore_errors=True)

    def connect(self, profile: dict = None) -> ezsdr.Session:
        return ezsdr.connect(profile, runs_dir=self.runs)

    def default_profile(self) -> dict:
        with self.connect() as sdr:
            return copy.deepcopy(sdr.profile)

    # -- Vision §57 and §58 #13

    def test_v57_repeat_then_capture_in_software(self) -> None:
        x = ramp()
        with self.connect() as sdr:
            sdr.tx.repeat(x)
            sdr.sleep(0.005)
            y = sdr.rx.capture(3000)
        self.assertEqual((y.shape, y.dtype), ((3000,), np.complex64))
        _, phasor = rotation(y, x)
        self.assertAlmostEqual(abs(phasor), 1.0, places=5)
        manifest = sdr.manifest
        # The action log: every call, in order.
        log = manifest["action_log"]
        self.assertEqual([entry["seq"] for entry in log], [0, 1, 2])
        self.assertEqual([entry["action"]["kind"] for entry in log], ["set_parameter", "vocabulary", "vocabulary"])
        self.assertEqual([entry["action"].get("verb") for entry in log[1:]], ["start_repeat", "capture"])
        # The waveform hash.
        digest = "sha256:" + hashlib.sha256(x.astype("<c8").tobytes()).hexdigest()
        self.assertEqual([item["hash"] for item in manifest["inputs"]], [digest])
        self.assertEqual(ezsdr.waveform(x)[0]["hash"], digest)
        # The capture's first sample time and validity flags.
        (artifact,) = manifest["artifacts"]
        continuity = artifact["continuity"][0]
        self.assertIn("ticks", continuity["first"])
        self.assertEqual(continuity["valid"], [[{"start": continuity["first"], "len": 3000}]])
        # The effective configuration, in the `radio` fragment's own report (SB-41).
        (report,) = [r for r in manifest["prepare"]["reports"] if r["fragment"] == "radio"]
        self.assertIn("radio.rx.sample_rate_hz", report["effective"])

    def test_v58_13_the_session_manifest_is_written_and_complete(self) -> None:
        with self.connect() as sdr:
            sdr.tx.repeat(ramp())
            with self.assertRaises(ezsdr.Rejected):
                sdr.rx.sample_rate = 1e12
            sdr.rx.capture(100)
        self.assertIsNotNone(sdr.manifest["hash"], "the Manifest is sealed")
        self.assertEqual(Path(sdr.manifest_path), Path(sdr.dir) / "manifest.json")
        self.assertEqual(json.loads(Path(sdr.manifest_path).read_text()), sdr.manifest)
        outcomes = [entry["outcome"]["kind"] for entry in sdr.manifest["action_log"]]
        self.assertEqual(outcomes, ["admitted", "admitted", "rejected", "admitted"], "the rejected call is logged too")

    def test_v58_16_a_rejected_call_raises_and_is_logged(self) -> None:
        profile = self.default_profile()
        profile["environment"]["radio.rf_envelope"] = {"allowed_bands": [{"lo_hz": 2.4e9, "hi_hz": 2.5e9}]}
        with self.connect(profile) as sdr:
            sdr.tx.frequency = 2.45e9
            sdr.tx.channels = 1  # the envelope judges what may radiate: a transmitting channel
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.tx.frequency = 2.6e9
            self.assertTrue(any(v["check"] == "radio.rf_envelope" and v["key"] == "radio.tx.frequency_hz" for v in raised.exception.violations))
            self.assertEqual(sdr.tx.frequency, 2.45e9, "the rejected value was never applied")
        self.assertEqual(sdr.manifest["action_log"][-1], raised.exception.entry)

    # -- Vision §3's second snippet

    def test_v3_parameters_through_attributes(self) -> None:
        with self.connect() as sdr:
            sdr.rx.frequency = 2.45e9
            sdr.rx.sample_rate = 20e6
            sdr.rx.gain = 20
            self.assertEqual((sdr.rx.frequency, sdr.rx.sample_rate, sdr.rx.gain), (2.45e9, 20e6, 20.0))
            entry = sdr.set("radio", "radio.rx.sample_rate_hz", 19.5e6)
            self.assertEqual(sdr.rx.sample_rate, 20e6, "19.5 Msps is coerced to the x310-like grid")
            coercion = entry["outcome"]["coercions"][0]
            self.assertEqual((coercion["key"], coercion["requested"], coercion["applied"]), ("radio.rx.sample_rate_hz", 19.5e6, 20e6))

    # -- Vision §54

    def test_v54_a_sweep_of_child_runs(self) -> None:
        spec = {
            "version": 1,
            "requirements": {"vocabularies": [{"id": "radio", "major": 1}, {"id": "sink", "major": 1}]},
            "resources": {"radio": {"kind": "radio.device", "requires": {}}},
            "outputs": [{
                "id": "rec", "kind": "sink.capture", "params": {"sink.capture_samples": 1000},
                "feed": {"port": {"component": "radio", "port": "rx"}, "policy": "drop_oldest", "capacity": 64},
            }],
        }
        results = []
        with self.connect() as sdr:
            for gain in (0.0, 10.0, 20.0):
                child = copy.deepcopy(spec)
                child["resources"]["radio"]["requires"]["radio.rx.gain_db"] = {"kind": "eq", "value": gain}
                result = sdr.run(child, duration=0.01)
                self.assertEqual(result.manifest["run"]["parent"], sdr.run_id)
                self.assertEqual(result.termination, {"kind": "stopped", "cause": {"reason": "client"}})
                self.assertEqual(result.capture("rec").shape, (1000,))
                results.append(result)
        children = sdr.manifest["sections"]["ezsdr.children"]
        self.assertEqual([c["seq"] for c in children], [r.entry["seq"] for r in results])
        self.assertEqual([c["run"] for c in children], [r.manifest["run"]["id"] for r in results])

    def test_ea_05_oversized_sink_address_preserves_the_session(self) -> None:
        with self.connect() as sdr:
            name = "a" * 252
            profile = copy.deepcopy(sdr.profile)
            profile["bindings"][name] = profile["bindings"].pop("rec")
            profile["bindings"][name].pop("feed", None)
            for link in profile["placements"]["links"]:
                if link["to"]["component"] == "rec":
                    link["to"]["component"] = name
            spec = {
                "version": 1,
                "requirements": {"vocabularies": [{"id": "radio", "major": 1}, {"id": "sink", "major": 1}]},
                "resources": {"radio": {"kind": "radio.device", "requires": {}}},
                "outputs": [{"id": name, "kind": "sink.capture", "params": {"sink.capture_samples": 1},
                             "feed": {"port": {"component": "radio", "port": "rx"}, "policy": "drop_oldest", "capacity": 64}}],
            }
            result = sdr.run(spec, profile=profile, duration=0.001)
            self.assertEqual(result.termination, {"kind": "failed", "stage": "validate"})
            self.assertEqual(result.manifest["artifacts"], [])
            self.assertEqual(sdr.rx.capture(1).shape, (1,), "parent Session is still usable")

    def test_v54_sleep_is_run_time(self) -> None:
        with self.connect() as sdr:
            before = sdr.now
            began = time.monotonic()
            sdr.sleep(10.0)
            elapsed = time.monotonic() - began
            after = sdr.now
        self.assertEqual(after["ticks"] - before["ticks"], 10_000_000_000, "10 s on the Simulation Engine's nanosecond root")
        self.assertLess(elapsed, 10.0, "virtual time runs faster than the wall clock (§58 #2)")

    # -- Vision §58 #3, #1 and #10

    def test_v58_03_the_same_calls_give_the_same_samples(self) -> None:
        def session() -> np.ndarray:
            with self.connect() as sdr:
                sdr.tx.repeat(ramp())
                sdr.sleep(0.003)
                return sdr.rx.capture(2000)

        np.testing.assert_array_equal(session(), session())

    def test_v58_01_the_same_script_under_another_profile(self) -> None:
        def script(sdr: ezsdr.Session) -> np.ndarray:
            sdr.tx.repeat(ramp())
            sdr.sleep(0.005)
            return sdr.rx.capture(2000)

        with self.connect() as sdr:
            rotated = script(sdr)
        ideal = self.default_profile()
        ideal["bindings"]["radio"]["profile"]["name"] = "ideal"
        with self.connect(ideal) as sdr:
            exact = script(sdr)
        rotation(rotated, ramp())
        offset, phasor = rotation(exact, ramp())
        self.assertEqual(phasor, 1.0, "the ideal profile draws no LO phase")

    # -- EA-16, EA-17

    def test_ea_17_two_channels_round_trip(self) -> None:
        profile = self.default_profile()
        profile["environment"]["sim.channel"]["couplings"].append(
            {"tx": "radio", "tx_channel": 1, "rx": "radio", "rx_channel": 1, "gain_db": 0.0}
        )
        x = np.stack([ramp(channel=0), ramp(channel=1) * 0.5])
        with self.connect(profile) as sdr:
            sdr.rx.channels = 2
            sdr.tx.repeat(x)
            self.assertEqual(sdr.tx.channels, 2)
            sdr.sleep(0.005)
            y = sdr.rx.capture(2000)
        self.assertEqual(y.shape, (2, 2000))
        for channel in range(2):
            rotation(y[channel], x[channel])

    def test_ea_16_events_and_wait_for(self) -> None:
        with self.connect() as sdr:
            entry = sdr.submit({
                "kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"node": 0, "path": "rec"},
                "at": None, "params": {"sink.capture_samples": 500},
            })
            self.assertEqual(entry["outcome"]["kind"], "admitted")
            event = sdr.wait_for([ezsdr.session.CAPTURE_WRITTEN], timeout=1.0)
            self.assertEqual(event["payload"]["artifact"]["size_bytes"], 4000)
            self.assertIn(event, sdr.events())
            self.assertIsNone(sdr.wait_for([ezsdr.session.CAPTURE_WRITTEN], timeout=0.01), "each event is returned once")

    def test_ea_16_errors(self) -> None:
        with self.assertRaises(ezsdr.ServerError) as refused:
            ezsdr.connect({"version": 1}, runs_dir=self.runs)
        self.assertEqual(refused.exception.kind, "refused")
        with self.connect() as sdr:
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(10_000_000, timeout=0.001)
            sdr.stop()
            with self.assertRaises(ezsdr.RunEnded) as ended:
                sdr.rx.gain = 1
            self.assertEqual(ended.exception.termination, {"kind": "stopped", "cause": {"reason": "client"}})
        self.assertEqual(sdr.manifest["termination"]["reason"], {"kind": "stopped", "cause": {"reason": "client"}})

    def test_ea_17_each_capture_gets_its_own_samples(self) -> None:
        # The recorder numbers its requests, and a capture waits for its own (Review H, P0-2).
        with self.connect() as sdr:
            self.assertEqual(sdr.rx.capture(100).shape, (100,))
            self.assertEqual(sdr.rx.capture(200).shape, (200,))
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(1_000_000, timeout=0.001)
            self.assertEqual(sdr.rx.capture(100, timeout=3.0).shape, (100,), "not the timed-out request's million")
            raw = sdr.submit({
                "kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"node": 0, "path": "rec"},
                "at": None, "params": {"sink.capture_samples": 777},
            })
            self.assertEqual(raw["outcome"]["kind"], "admitted")
            self.assertEqual(sdr.rx.capture(100).shape, (100,), "not the raw request's 777")
            # The Kernel routes a capture to the only recorder whatever its target names, and a
            # SetParameter of sink.capture_samples on sink/rec is a request too; a capture the
            # Kernel rejects reaches no recorder (Review I, P0-A, P1-D).
            for action in (
                {"kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"node": 0, "path": "radio/rx"}, "at": None, "params": {"sink.capture_samples": 333}},
                {"kind": "set_parameter", "target": {"node": 0, "path": "sink/rec"}, "key": "sink.capture_samples", "value": 555},
            ):
                self.assertEqual(sdr.submit(action)["outcome"]["kind"], "admitted")
                self.assertEqual(sdr.rx.capture(100).shape, (100,), f"not {action['kind']}'s request")
            rejected = sdr.submit({"kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"node": 0, "path": "rec"}, "at": None, "params": {}})
            self.assertEqual(rejected["outcome"]["kind"], "rejected")
            self.assertEqual(sdr.rx.capture(100).shape, (100,), "a rejected capture takes no number")

    def test_ea_17_a_capture_keeps_its_first_deadline(self) -> None:
        # Another request's announcement inside the timeout does not restart it (Review H,
        # P2-5; Review I, P1-D): the raw 10 000-sample request is written at ~10 ms, this
        # capture's at ~16 ms, and 12 ms is the whole budget.
        with self.connect() as sdr:
            sdr.submit({"kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"node": 0, "path": "rec"}, "at": None, "params": {"sink.capture_samples": 10_000}})
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(5000, timeout=0.012)
            written = [e for e in sdr.events() if e["kind"] == ezsdr.session.CAPTURE_WRITTEN]
            self.assertEqual([e["payload"]["request"] for e in written], [0], "the foreign announcement came inside the timeout")

    def test_ea_17_a_capture_across_a_gap_is_refused(self) -> None:
        # §23: a gap is a flag and a time jump, which one array would hide (Review I, P1-C).
        profile = self.default_profile()
        profile["environment"]["sim.faults"] = [{"at_ns": 2_000_000, "fault": "rx_overflow", "target": "radio"}]
        with self.connect(profile) as sdr:
            with self.assertRaises(ezsdr.Error) as refused:
                sdr.rx.capture(5000)
            self.assertIn("gap", str(refused.exception))

    def test_ea_17_requests_made_ahead_capture_contiguous_samples(self) -> None:
        # Two back-to-back `capture` calls lose the samples between them; requests made
        # before the first is answered are served one after another (HD-10), as v3's
        # receiveRequestOnly/receiveResponseOnly split did.
        with self.connect() as sdr:
            sdr.tx.repeat(ramp())
            sdr.sleep(0.005)
            first, second = sdr.rx.request(1000), sdr.rx.request(1500)
            self.assertEqual((first.number, second.number), (0, 1))
            b = sdr.rx.result(second)
            a = sdr.rx.result(first)
            self.assertEqual((a.shape, b.shape), ((1000,), (1500,)))
            # One continuous stretch of the repeated waveform across the boundary.
            rotation(np.concatenate([a, b]), ramp())
        spans = [(c["first"]["ticks"], c["end"]["ticks"]) for c in (art["continuity"][0] for art in sdr.manifest["artifacts"])]
        self.assertEqual(spans[0][1], spans[1][0], "the second capture starts where the first ends")

    def test_ea_17_a_capture_the_recorder_refuses_raises(self) -> None:
        with self.connect() as sdr:
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.rx.capture(0)
            self.assertEqual(raised.exception.event["kind"], "sink.REQUEST_REJECTED")
            self.assertEqual(raised.exception.entry["outcome"]["kind"], "admitted", "the Kernel admitted it; the recorder refused it")

    def test_ea_17_capture_at_an_instant(self) -> None:
        with self.connect() as sdr:
            at = dict(sdr.now)
            at["ticks"] += 5_000_000
            sdr.rx.capture(1000, at=at)
            t0 = sdr.start_instant["ticks"]
        (artifact,) = sdr.manifest["artifacts"]
        self.assertEqual(artifact["continuity"][0]["first"]["ticks"], (at["ticks"] - t0) // 1000, "the first sample at `at`, 1 Msps from T0")

    # -- Phase 7, VE-6

    def test_sleep_returns_the_instant(self) -> None:
        with self.connect() as sdr:
            t = sdr.sleep(0.01)
            self.assertEqual(sdr.now, t)
            later = dict(t, ticks=t["ticks"] + 1_234_567)
            self.assertEqual(sdr.wait_until(later), later)
            self.assertEqual(sdr.now, later)

    def test_after_names_an_instant_ahead(self) -> None:
        with self.connect() as sdr:
            at = sdr.after(0.001)
            self.assertEqual(at["ticks"], sdr.now["ticks"] + 1_000_000, "a thousandth, not the float nearest it")
            tiny = sdr.after(1.5e-9)
            self.assertEqual(tiny["ticks"], sdr.now["ticks"] + 2, "the ceiling of 1.5 ticks")
            sdr.rx.capture(1000, at=at)
            t0 = sdr.start_instant["ticks"]
        (artifact,) = sdr.manifest["artifacts"]
        self.assertEqual(artifact["continuity"][0]["first"]["ticks"], (at["ticks"] - t0) // 1000, "the capture starts at `after`'s instant")

    def test_ea_17_a_capture_across_a_rate_change_is_refused(self) -> None:
        with self.connect() as sdr:
            sdr.submit({
                "kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"node": 0, "path": "rec"},
                "at": None, "params": {"sink.capture_samples": 20_000},
            })
            sdr.sleep(0.005)
            sdr.rx.sample_rate = 2e6
            event = sdr.wait_for([ezsdr.session.CAPTURE_WRITTEN], timeout=1.0)
            artifact = event["payload"]["artifact"]
            self.assertEqual(len(artifact["continuity"]), 2)
            with self.assertRaises(ezsdr.Error):
                ezsdr.samples(sdr.read(artifact), artifact)

    def test_ea_16_repeat_sets_the_channel_count_only_when_it_differs(self) -> None:
        with self.connect() as sdr:
            sdr.tx.repeat(ramp())
            sdr.tx.repeat(ramp(500))
        kinds = [entry["action"]["kind"] for entry in sdr.manifest["action_log"]]
        self.assertEqual(kinds, ["set_parameter", "vocabulary", "vocabulary"])

    def test_ea_16_close_after_the_server_exited(self) -> None:
        sdr = self.connect()
        sdr._connection._process.stdin.write(b"not json\n")
        sdr._connection._process.stdin.flush()
        sdr._connection._process.wait(timeout=30)
        manifest = sdr.close()
        self.assertEqual(manifest["termination"]["reason"], {"kind": "stopped", "cause": {"reason": "client_disconnect"}})
        self.assertEqual(Path(sdr.manifest_path), Path(sdr.dir) / "manifest.json")

    # -- EA-19

    def test_ea_19_the_examples_run(self) -> None:
        env = dict(os.environ, PYTHONPATH=str(PYTHON_DIR), PYTHONDONTWRITEBYTECODE="1")
        for example in ("minimal.py", "sweep.py"):
            done = subprocess.run(
                [sys.executable, str(PYTHON_DIR / "examples" / example)],
                cwd=self.runs, env=env, capture_output=True, text=True, timeout=300,
            )
            self.assertEqual(done.returncode, 0, f"{example}: {done.stderr}")


class DurationRequests(unittest.TestCase):
    """Characterize seconds-to-nanoseconds at the four request boundaries (RF-001)."""

    def session(self, reply: dict) -> ezsdr.Session:
        connection = Mock()
        connection.call.return_value = (reply, b"")
        return ezsdr.Session(connection, {
            "run": "test", "dir": "", "profile": {}, "start_instant": {}, "now": {},
        })

    def callers(self, sdr: ezsdr.Session, seconds: float) -> dict:
        handle = ezsdr.session.CaptureRequest("rec", 0, 1, 0, {})
        return {
            "sleep": lambda: sdr.sleep(seconds),
            "wait_for": lambda: sdr.wait_for(["test.EVENT"], seconds),
            "run": lambda: sdr.run({}, duration=seconds),
            "capture": lambda: ezsdr.session.Rx(sdr, "radio", "rx").result(handle, seconds),
        }

    def test_durations_round_up_and_clamp_at_each_caller(self) -> None:
        reply = {"now": {}, "index": None, "entry": {"outcome": {"kind": "admitted"}}, "manifest": {}, "path": ""}
        for seconds, expected in [(-1.0, 0), (0.0, 0), (1e-12, 1), (1e-9, 1), (1.0000000001e-9, 2)]:
            for caller, field in [("sleep", "by_ns"), ("wait_for", "within_ns"), ("run", "duration_ns"), ("capture", "within_ns")]:
                with self.subTest(seconds=seconds, caller=caller):
                    sdr = self.session(reply)
                    call = self.callers(sdr, seconds)[caller]
                    if caller == "capture":
                        with self.assertRaises(ezsdr.CaptureTimeout):
                            call()
                    else:
                        call()
                    sdr._connection.call.assert_called_once()
                    request = sdr._connection.call.call_args.args[0]
                    self.assertEqual(request[field], expected)

    def test_nonfinite_durations_fail_before_a_request(self) -> None:
        for seconds, error in [(float("nan"), ValueError), (float("inf"), OverflowError), (-float("inf"), OverflowError), (1e308, OverflowError)]:
            for caller in ["sleep", "wait_for", "run", "capture"]:
                with self.subTest(seconds=seconds, caller=caller):
                    sdr = self.session({})
                    with self.assertRaises(error):
                        self.callers(sdr, seconds)[caller]()
                    sdr._connection.call.assert_not_called()

    def test_a_child_without_duration_omits_the_field(self) -> None:
        sdr = self.session({"entry": {"outcome": {"kind": "admitted"}}, "manifest": {}, "path": ""})
        sdr.run({})
        self.assertEqual(sdr._connection.call.call_args.args[0], {"op": "run_child", "spec": {}, "inputs": []})

    def test_capture_waits_again_with_the_first_horizon(self) -> None:
        sdr = self.session({})
        horizon = {"domain": {"node": 0, "path": "clock"}, "ticks": 12}
        sdr._connection.call.side_effect = [
            ({"index": 3, "horizon": horizon, "event": {"source": {"node": 0, "path": "sink/other"}, "payload": {"request": 0}}}, b""),
            ({"index": None}, b""),
        ]
        handle = ezsdr.session.CaptureRequest("rec", 0, 1, 2, {})
        with self.assertRaises(ezsdr.CaptureTimeout):
            ezsdr.session.Rx(sdr, "radio", "rx").result(handle, 1e-12)
        requests = [call.args[0] for call in sdr._connection.call.call_args_list]
        self.assertEqual(requests, [
            {"op": "wait_for", "kinds": [ezsdr.session.CAPTURE_WRITTEN, ezsdr.session.REQUEST_REJECTED], "from": 2, "within_ns": 1},
            {"op": "wait_for", "kinds": [ezsdr.session.CAPTURE_WRITTEN, ezsdr.session.REQUEST_REJECTED], "from": 4, "until": horizon},
        ])


class RecorderChoice(unittest.TestCase):
    """EA-17's recorder over a mocked connection, as ``DurationRequests`` builds one."""

    def test_ea_17_capture_needs_exactly_one_recorder(self) -> None:
        # EA-17 (spec 20, VF-1): exactly one binding fed from the radio's receive port, or
        # an Error before any request; the helper never chooses among recorders.
        def fed(component: str) -> dict:
            return {"feed": {"port": {"component": component, "port": "rx"}, "policy": "drop_oldest", "capacity": 64}}
        several = "EA-17: 2 recorders are fed from radio's receive port ['rec_a', 'rec_b']; submit sink.capture naming one"
        none = "EA-17: no recorder is fed from radio's receive port in the Session's profile"
        for bindings, error in [
            ({"rec_b": fed("radio"), "rec_a": fed("radio")}, several),
            ({"radio": {}, "rec": {"feed": None}}, none),
            ({"rec_b": fed("other"), "rec_a": fed("radio")}, None),
        ]:
            with self.subTest(bindings=list(bindings)):
                connection = Mock()
                connection.call.return_value = ({"events": 0, "entry": {"outcome": {"kind": "admitted"}}}, b"")
                sdr = ezsdr.Session(connection, {
                    "run": "test", "dir": "", "profile": {"bindings": bindings}, "start_instant": {}, "now": {},
                })
                rx = ezsdr.session.Rx(sdr, "radio", "rx")
                if error is None:
                    self.assertEqual(rx.request(100).recorder, "rec_a")
                    action = connection.call.call_args.args[0]["action"]
                    self.assertEqual((action["ns"], action["verb"], action["target"]["path"]), ("sink", "capture", "rec_a"))
                else:
                    with self.assertRaises(ezsdr.Error) as raised:
                        rx.request(100)
                    self.assertEqual(str(raised.exception), error)
                    connection.call.assert_not_called()


class SampleValidity(unittest.TestCase):
    def test_samples_requires_full_validity_on_every_channel(self) -> None:
        domain = {"node": 0, "local": 7}
        def t(k): return {"domain": domain, "ticks": k}
        data = np.arange(10, dtype=np.complex64).tobytes()
        for start, length in [(2, 3), (0, 3), (0, 5)]:
            with self.subTest(start=start, length=length):
                artifact = {"id": "rec", "continuity": [{
                    "domain": domain, "channels": 2, "first": t(0), "end": t(5),
                    "valid": [[{"start": t(0), "len": 5}], [{"start": t(start), "len": length}]],
                    "gaps": [], "channel_gaps": [],
                }]}
                if (start, length) == (0, 5):
                    np.testing.assert_array_equal(ezsdr.samples(data, artifact),
                        np.arange(10, dtype=np.complex64).reshape(5, 2).T)
                else:
                    with self.assertRaises(ezsdr.Error):
                        ezsdr.samples(data, artifact)


if __name__ == "__main__":
    unittest.main()
