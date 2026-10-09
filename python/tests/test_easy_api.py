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
from decimal import Decimal
from fractions import Fraction
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
            # The transmit clock the enable starts begins the profile's 50 ms start lead
            # later, and the untimed repeat there, on time (RS-19, RM-25).
            sdr.sleep(0.06)
            y = sdr.rx.capture(3000)
        self.assertEqual((y.shape, y.dtype), ((3000,), np.complex64))
        _, phasor = rotation(y, x)
        self.assertAlmostEqual(abs(phasor), 1.0, places=5)
        manifest = sdr.manifest
        self.assertEqual([e for e in manifest["events"]["delivered"] if e["kind"] == "radio.TIME_ERROR"], [])
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
            "requirements": {"vocabularies": [{"id": "radio", "major": 2}, {"id": "sink", "major": 1}]},
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
                self.assertEqual(result.termination, {"kind": "stopped", "cause": {"kind": "client"}})
                self.assertEqual(result.capture("rec").shape, (1000,))
                results.append(result)
        children = sdr.manifest["run"]["children"]
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
                "requirements": {"vocabularies": [{"id": "radio", "major": 2}, {"id": "sink", "major": 1}]},
                "resources": {"radio": {"kind": "radio.device", "requires": {}}},
                "outputs": [{"id": name, "kind": "sink.capture", "params": {"sink.capture_samples": 1},
                             "feed": {"port": {"component": "radio", "port": "rx"}, "policy": "drop_oldest", "capacity": 64}}],
            }
            result = sdr.run(spec, profile=profile, duration=0.001)
            self.assertEqual((result.termination["kind"], result.termination["stage"]), ("failed", "validate"))
            self.assertTrue(result.termination["reason"], result.termination)
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
            self.assertEqual(ended.exception.termination, {"kind": "stopped", "cause": {"kind": "client"}})
        self.assertEqual(sdr.manifest["termination"]["reason"], {"kind": "stopped", "cause": {"kind": "client"}})

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

    def test_ea_16_repeat_at_an_instant(self) -> None:
        x = ramp()
        with self.connect() as sdr:
            at = sdr.after(0.2)
            entry = sdr.tx.repeat(x, at=at)
            y = sdr.rx.capture(3000, at=dict(at, ticks=at["ticks"] - 1_000_000))
        self.assertEqual(entry["action"]["at"], at)
        # Silence until `at`, then the waveform from its first sample (1 Msps on a 1 GHz root),
        # after `x310-like`'s 45-sample transmit path delay (MR-3, RM-23); on time, so no TIME_ERROR.
        start = 1000 + 45
        np.testing.assert_allclose(y[:start], 0, atol=1e-6)
        offset, _ = rotation(y[start:], x)
        self.assertEqual(offset, 0)
        counts = {c["kind"]: c["count"] for c in sdr.manifest["events"]["counters"] if c["source"]["path"] == "radio"}
        self.assertEqual(counts["radio.TIME_ERROR"], 0)

    def test_ea_16_aligned_gives_its_calls_one_instant(self) -> None:
        x = ramp()
        with self.connect() as sdr:
            at = sdr.after(0.2)
            with sdr.aligned(at):
                sent = sdr.tx.repeat(x)
                request = sdr.rx.request(3000)
                with self.assertRaises(ezsdr.Error):
                    sdr.rx.capture(10)
                with self.assertRaises(ezsdr.Error):
                    with sdr.aligned(at):
                        pass
            y = sdr.rx.result(request)
            later = sdr.tx.repeat(x)
        self.assertEqual((sent["action"]["at"], request.entry["action"]["at"]), (at, at))
        self.assertIsNone(later["action"]["at"], "the block's instant ends with it")
        # Both begin at `at`: the waveform's first sample after the 45-sample path delay.
        np.testing.assert_allclose(y[:45], 0, atol=1e-6)
        offset, _ = rotation(y[45:], x)
        self.assertEqual(offset, 0)

    def test_ea_16_next_at_finds_the_waveform_s_start_again(self) -> None:
        x = ramp()
        with self.connect() as sdr:
            at0 = sdr.after(0.2)
            with sdr.aligned(at0):
                sdr.tx.repeat(x)
            before = sdr.sleep(0.3713)
            at = sdr.rx.next_at(at0, len(x))
            y = sdr.rx.capture(len(x), at=at)
            sdr.rx.stop()
            with self.assertRaises(ezsdr.Error):
                sdr.rx.next_at(at0, len(x))
        self.assertEqual((at["ticks"] - at0["ticks"]) % (len(x) * 1000), 0, "whole periods of 1 000 samples, 1 000 ticks each")
        self.assertGreaterEqual(at["ticks"], before["ticks"] + 50_000_000, "at least 50 ms ahead")
        # The waveform's start, 45 samples later after the path delay.
        offset, _ = rotation(y, x)
        self.assertEqual(offset, len(x) - 45)

    # -- Phase 7, VE-6

    def test_sleep_returns_the_instant(self) -> None:
        with self.connect() as sdr:
            t = sdr.sleep(0.01)
            self.assertEqual(sdr.now, t)
            later = dict(t, ticks=t["ticks"] + 1_234_567)
            self.assertEqual(sdr.wait_until(later), later)
            self.assertEqual(sdr.now, later)

    def test_ea_16_next_pps_is_refused_in_simulation(self) -> None:
        # The default profile's simulated root has an `arbitrary` epoch, which `connected`
        # names (EA-10): its tick zero is no PPS edge.
        with self.connect() as sdr:
            self.assertEqual(sdr._epoch["kind"], "arbitrary")
            with self.assertRaises(ezsdr.Error):
                sdr.next_pps()

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

    def test_ea_16_sleep_reaches_the_instant_after_names(self) -> None:
        # EA-16, EA-12 (spec 20, VF-2): one reading and one rounding, so `sleep(d)` stands where
        # `after(d)` names from the same `now`; the float product put 0.0041 a tick later.
        with self.connect() as sdr:
            a = sdr.after(0.0041)
            t = sdr.sleep(0.0041)
        self.assertEqual(t, a)

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

    def test_ea_16_rx_stop_then_start(self) -> None:
        # EA-16, RM-12, RM-21 (spec 22, VH-1, VH-6): after rx.stop() a change starts nothing;
        # rx.start() starts the stream again on a new SampleClock at the configuration in
        # force; calls act in the order made.
        with self.connect() as sdr:
            sdr.rx.stop()
            sdr.sleep(0.002)
            sdr.rx.sample_rate = 2e6
            sdr.sleep(0.06)
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(1000, timeout=0.2)
            sdr.rx.start()
            self.assertEqual(len(sdr.rx.capture(1000)), 1000)
            sdr.rx.stop()
            sdr.sleep(0.01)
            at = sdr.after(0.02)
            sdr.rx.start(at=at)
            self.assertEqual(len(sdr.rx.capture(1000)), 1000)
            sdr.rx.sample_rate = 1e6
            self.assertEqual(len(sdr.rx.capture(1000)), 1000)
            sdr.rx.start()
            sdr.rx.stop()
            sdr.sleep(0.1)
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(1000, timeout=0.2)
            sdr.rx.stop()
            sdr.rx.start()
            self.assertEqual(len(sdr.rx.capture(1000)), 1000)
            self.assertEqual(len(sdr.effective["radio"]), 10)
        # The first segment, stopped at T0 before its first sample, has no clock (RM-25); the
        # four others end at their cuts, the last at the Session's close.
        clocks = [c for c in sdr.manifest["clocks"]["sample_clocks"] if c["stream"]["path"] == "radio/rx"]
        self.assertEqual(len(clocks), 4, clocks)
        self.assertTrue(all(c.get("ended_at") is not None for c in clocks), clocks)
        self.assertEqual([c["root_ticks_per_tick"]["num"] for c in clocks], [500, 500, 1000, 1000], clocks)
        # The start, received 20 ms before its `at`, is ready then: its clock begins the start
        # lead after its receipt, the later of that and `at` (RM-25).
        lead, received = 50_000_000, at["ticks"] - 20_000_000
        self.assertLessEqual(abs(clocks[1]["origin"]["ticks"] - max(at["ticks"], received + lead)), 500, (clocks, at))

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
        self.assertEqual(manifest["termination"]["reason"], {"kind": "stopped", "cause": {"kind": "client_disconnect"}})
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
    """Seconds as the package reads them, and the requests they become, over a mocked
    connection (EA-16; spec 20, VF-2)."""

    ROOT = {"node": 0, "local": 1}

    def session(self, reply: dict, rate: tuple = (1_000_000_000, 1), epoch: dict = None) -> ezsdr.Session:
        connection = Mock()
        connection.call.return_value = (reply, b"")
        fed = {"feed": {"port": {"component": "radio", "port": "rx"}, "policy": "drop_oldest", "capacity": 64}}
        at = {"domain": self.ROOT, "ticks": 0}
        connected = {
            "run": "test", "dir": "", "profile": {"bindings": {"radio": {}, "rec": fed}},
            "start_instant": at, "now": at, "root_rate": {"num": rate[0], "den": rate[1]},
        }
        if epoch is not None:
            connected["root_epoch"] = epoch
        return ezsdr.Session(connection, connected)

    def callers(self, sdr: ezsdr.Session, seconds: object) -> dict:
        handle = ezsdr.session.CaptureRequest("rec", 0, 1, 0, {})
        rx = ezsdr.session.Rx(sdr, "radio", "rx")
        return {
            "sleep": lambda: sdr.sleep(seconds),
            "wait_for": lambda: sdr.wait_for(["test.EVENT"], seconds),
            "result": lambda: rx.result(handle, seconds),
            "run": lambda: sdr.run({}, duration=seconds),
            "capture": lambda: rx.capture(1, timeout=seconds),
            "after": lambda: sdr.after(seconds),
        }

    def test_ea_16_durations_go_as_root_durations(self) -> None:
        # One exact reading and one rounding up, sent as ticks of the 1 GHz root (`run` as
        # nanoseconds); the last two cases are read under numpy's legacy printing, where `str`
        # would write 0.3 for the first.
        reply = {"now": {}, "index": None, "entry": {"outcome": {"kind": "admitted"}}, "manifest": {}, "path": ""}
        cases = [
            (0.0, 0, False), (1e-12, 1, False), (1e-9, 1, False), (1.0000000001e-9, 2, False),
            (0.0041, 4_100_000, False), (np.float64(0.0041), 4_100_000, False),
            (np.float32(0.001), 1_000_000, False), (np.float32(0.1), 100_000_000, False),
            (np.int64(2), 2_000_000_000, False), (Fraction(1, 3), 333_333_334, False),
            (Decimal("9223372036.854775807"), 2**63 - 1, False),
            (np.float64(0.1) + np.float64(0.2), 300_000_001, True),
            # Under legacy printing `repr(np.float64(0.1))` is 0.10000000000000001 on numpy 1
            # and 2 alike; its own shortest decimal is 0.1.
            (np.float64(0.1), 100_000_000, True),
        ]
        for seconds, expected, legacy in cases:
            for caller, field in [("sleep", "by"), ("wait_for", "within"), ("result", "within"), ("run", "duration_ns")]:
                with self.subTest(seconds=repr(seconds), caller=caller):
                    sdr = self.session(reply)
                    call = self.callers(sdr, seconds)[caller]
                    with np.printoptions(legacy="1.13" if legacy else False):
                        if caller == "result":
                            with self.assertRaises(ezsdr.CaptureTimeout):
                                call()
                        else:
                            call()
                    sdr._connection.call.assert_called_once()
                    sent = sdr._connection.call.call_args.args[0][field]
                    self.assertEqual(sent, expected if caller == "run" else {"domain": self.ROOT, "ticks": expected})

    def test_ea_16_bad_seconds_fail_before_a_request(self) -> None:
        # A value of another type, a non-finite or negative duration, and one past its field
        # raise before anything is sent, `capture`'s timeout and `after` included.
        six = ["sleep", "wait_for", "result", "run", "capture", "after"]
        cases = [(value, TypeError, six) for value in ["1", True, np.bool_(True), np.timedelta64(1, "s")]]
        cases += [(None, TypeError, ["sleep", "wait_for", "after"])]
        cases += [(value, ValueError, six) for value in [float("nan"), float("inf"), -float("inf"), Decimal("NaN"), Decimal("-Infinity")]]
        cases += [(value, ValueError, six[:5]) for value in [-1.0, -1e-12, 1e308]]
        cases += [(Decimal("9223372036.854775808"), ValueError, ["sleep", "wait_for", "result", "capture"])]
        cases += [(Decimal("18446744073.709551616"), ValueError, ["run"])]
        for seconds, error, callers in cases:
            for caller in callers:
                with self.subTest(seconds=repr(seconds), caller=caller):
                    sdr = self.session({})
                    with self.assertRaises(error):
                        self.callers(sdr, seconds)[caller]()
                    sdr._connection.call.assert_not_called()

    def test_ea_16_after_reads_the_decimal(self) -> None:
        for seconds, ticks in [(0.0041, 4_100_000), (np.float64(0.0041), 4_100_000), (np.float32(0.0041), 4_100_000), (Decimal("0.0041"), 4_100_000), (-0.001, -1_000_000)]:
            with self.subTest(seconds=repr(seconds)):
                sdr = self.session({"now": {"domain": self.ROOT, "ticks": 0}})
                self.assertEqual(sdr.after(seconds), {"domain": self.ROOT, "ticks": ticks})

    def test_ea_16_sleep_reaches_after_on_any_root(self) -> None:
        # 5.42e-9 s of a 184.32 MHz root is one tick; through 6 ns it would be two (issue #40).
        sdr = self.session({"now": {"domain": self.ROOT, "ticks": 0}}, rate=(184_320_000, 1))
        sdr.sleep(5.42e-9)
        self.assertEqual(sdr._connection.call.call_args.args[0]["by"], {"domain": self.ROOT, "ticks": 1})
        self.assertEqual(sdr.after(5.42e-9), {"domain": self.ROOT, "ticks": 1})

    def test_ea_16_a_rational_root_rate_is_read_exactly(self) -> None:
        # A root of 10⁹/3 ticks per second: `num // den` loses a tick at 1 s, a float rate at 10⁷ s.
        for seconds, ticks in [(1, 333_333_334), (10_000_000, 3_333_333_333_333_334)]:
            with self.subTest(seconds=seconds):
                sdr = self.session({"now": {"domain": self.ROOT, "ticks": 0}}, rate=(1_000_000_000, 3))
                sdr.sleep(seconds)
                self.assertEqual(sdr._connection.call.call_args.args[0]["by"], {"domain": self.ROOT, "ticks": ticks})
                self.assertEqual(sdr.after(seconds), {"domain": self.ROOT, "ticks": ticks})

    def test_ea_16_next_at_counts_samples_at_a_fractional_rate(self) -> None:
        # 3 MS/s on a 1 GHz root: 1 000/3 ticks a sample, origin at tick 7. A start at tick 100
        # is sample 1; now + 50 ms is tick 50 010 000, first reached by sample 150 030. Periods
        # of 10 samples from sample 1 give sample 150 031, at tick 7 + ceil(150 031 · 1 000/3).
        clock = {"stream": {"node": 0, "path": "radio/rx"}, "root_ticks_per_tick": {"num": 1000, "den": 3},
                 "origin": {"domain": self.ROOT, "ticks": 7}, "ended_at": None}
        sdr = self.session({"now": {"domain": self.ROOT, "ticks": 10_000}, "sample_clocks": [clock]})
        rx = ezsdr.session.Rx(sdr, "radio", "rx")
        at = rx.next_at({"domain": self.ROOT, "ticks": 100}, 10)
        self.assertEqual(at, {"domain": self.ROOT, "ticks": 50_010_341})
        clock["origin"]["ticks"] = 200
        with self.assertRaises(ezsdr.Error):
            rx.next_at({"domain": self.ROOT, "ticks": 100}, 10)

    PPS = {"kind": "pps", "set_by": "ezsdr.radio.uhd.set_time_unknown_pps:addr=192.0.2.1"}

    def test_ea_16_next_pps_counts_whole_seconds_of_a_pps_root(self) -> None:
        # A 200 MHz root whose tick zero is a PPS edge: now 3.97 s; the edge at least `ahead`
        # after it, inclusive, then k − 1 seconds later.
        for now_s, k, ahead, edge_s in [(Fraction(397, 100), 1, 0.05, 5), (Fraction(397, 100), 3, 0.05, 7),
                                        (Fraction(397, 100), 1, 0.02, 4), (Fraction(395, 100), 1, 0.05, 4)]:
            with self.subTest(now=now_s, k=k, ahead=ahead):
                sdr = self.session({"now": {"domain": self.ROOT, "ticks": int(now_s * 200_000_000)}}, rate=(200_000_000, 1), epoch=self.PPS)
                self.assertEqual(sdr.next_pps(k, ahead=ahead), {"domain": self.ROOT, "ticks": edge_s * 200_000_000})

    def test_ea_16_next_pps_refuses_a_root_that_no_pps_set(self) -> None:
        # Refused before a request: no epoch, an epoch not set by a PPS, a second that is not a
        # whole number of ticks, and a k that is not a positive integer (EA-16).
        arbitrary = {"kind": "arbitrary", "set_by": "ezsdr.sim.virtual"}
        for epoch, rate, k in [(None, (200_000_000, 1), 1), (arbitrary, (200_000_000, 1), 1),
                               (self.PPS, (1_000_000_000, 3), 1), (self.PPS, (200_000_000, 1), 0),
                               (self.PPS, (200_000_000, 1), 1.5)]:
            with self.subTest(epoch=epoch, rate=rate, k=k):
                sdr = self.session({"now": {"domain": self.ROOT, "ticks": 0}}, rate=rate, epoch=epoch)
                with self.assertRaises(ezsdr.Error):
                    sdr.next_pps(k)
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
            {"op": "wait_for", "kinds": [ezsdr.session.CAPTURE_WRITTEN, ezsdr.session.REQUEST_REJECTED], "from": 2, "within": {"domain": self.ROOT, "ticks": 1}},
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
                at = {"domain": {"node": 0, "local": 1}, "ticks": 0}
                sdr = ezsdr.Session(connection, {
                    "run": "test", "dir": "", "profile": {"bindings": bindings}, "start_instant": at, "now": at,
                    "root_rate": {"num": 1_000_000_000, "den": 1},
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
