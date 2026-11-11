"""Phase 6's Python carriers (plan/phase6/00-overview.md §8), run against ezsdr-server.

    cargo build -p ezsdr-server
    EZSDR_SERVER=<target>/debug/ezsdr-server python3 -m unittest discover -s python/tests

The server is required: without it every test fails, naming how to build it (GY-7).
"""

from __future__ import annotations

import copy
import faulthandler
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from decimal import Decimal
from fractions import Fraction
import unittest.mock
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
    """The offset and unit phasor for a non-silent repeated ``x`` (MR-34), checked per sample."""
    assert np.min(np.abs(y)) > 1e-6, "capture is silent"
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

    def connect(self, profile: dict = None, **streams: object) -> ezsdr.Session:
        streams.setdefault("rx", {0: [0]})  # a Session receives only through the streams it asks for (RM-30)
        return ezsdr.connect(profile, runs_dir=self.runs, **streams)

    def default_profile(self) -> dict:
        with self.connect() as sdr:
            return copy.deepcopy(sdr.profile)

    def wait_for_tx_signal(self, sdr: ezsdr.Session) -> None:
        # An untimed repeat starts 2 ms after it is admitted (RS-19's earliest instant) and x3xx's
        # TX path is 45 samples (46 µs at 1 Msps): after 3 ms of Run time the waveform is out.
        sdr.sleep(0.003)

    def add_recorder(self, profile: dict, stream: int) -> str:
        name = f"rec{stream}"
        binding = copy.deepcopy(profile["bindings"]["rec"])
        binding["selector"]["dir"] = str(Path(self.runs) / f"captures-{stream}")
        binding["feed"]["port"]["port"] = f"rx{stream}"
        profile["bindings"][name] = binding
        link = copy.deepcopy(profile["placements"]["links"][0])
        link["from"]["port"] = f"rx{stream}"
        link["to"]["component"] = name
        profile["placements"]["links"].append(link)
        return name

    # -- Vision §57 and §58 #13

    def test_v57_repeat_then_capture_in_software(self) -> None:
        x = ramp()
        with self.connect(tx={0: [0]}) as sdr:
            sdr.tx.repeat(x)
            # The transmit clock starts with the Session and the untimed repeat 2 ms after it is
            # admitted (RS-19, RM-25); 60 ms of Run time puts the waveform on the air.
            sdr.sleep(0.06)
            y = sdr.rx.capture(3000)
        self.assertEqual((y.shape, y.dtype), ((3000,), np.complex64))
        _, phasor = rotation(y, x)
        self.assertAlmostEqual(abs(phasor), 1.0, places=5)
        manifest = sdr.manifest
        self.assertEqual([e for e in manifest["events"]["delivered"] if e["kind"] == "radio.TIME_ERROR"], [])
        # The action log: every call, in order.
        log = manifest["action_log"]
        self.assertEqual([entry["seq"] for entry in log], [0, 1])
        self.assertEqual([entry["action"]["kind"] for entry in log], ["vocabulary", "vocabulary"])
        self.assertEqual([entry["action"].get("verb") for entry in log], ["start_repeat", "capture"])
        # The waveform hash.
        digest = "sha256:" + hashlib.sha256(x.astype("<c8").tobytes()).hexdigest()
        self.assertEqual([item["hash"] for item in manifest["inputs"]], [digest])
        self.assertEqual(ezsdr.waveform(x)[0]["hash"], digest)
        # The capture's first sample time and validity flags.
        (artifact,) = manifest["artifacts"]["rec"]
        continuity = artifact["continuity"][0]
        self.assertIn("ticks", continuity["first"])
        self.assertEqual(continuity["valid"], [[{"start": continuity["first"], "len": 3000}]])
        # The effective configuration, in the `radio` fragment's own report (SB-41).
        report = manifest["prepare"]["reports"]["radio"]
        self.assertIn("radio.rx.s0.sample_rate_hz", report["effective"])

    def test_v58_13_the_session_manifest_is_written_and_complete(self) -> None:
        with self.connect(tx={0: [0]}) as sdr:
            sdr.tx.repeat(ramp())
            with self.assertRaises(ezsdr.Rejected):
                sdr.set("radio", "radio.rx.s0.sample_rate_hz", 1e12)
            sdr.rx.capture(100)
        self.assertIsNotNone(sdr.manifest["hash"], "the Manifest is sealed")
        self.assertEqual(Path(sdr.manifest_path), Path(sdr.dir) / "manifest.json")
        self.assertEqual(json.loads(Path(sdr.manifest_path).read_text()), sdr.manifest)
        outcomes = [entry["outcome"]["kind"] for entry in sdr.manifest["action_log"]]
        self.assertEqual(outcomes, ["admitted", "rejected", "admitted"], "the rejected call is logged too")

    def test_v58_16_a_rejected_call_raises_and_is_logged(self) -> None:
        # A transmit value outside the RF envelope is refused at admission, on the stream it is judged for (RM-19, RM-21).
        profile = self.default_profile()
        profile["environment"]["radio.rf_envelope"] = {"allowed_bands": [{"lo_hz": 2.4e9, "hi_hz": 2.5e9}]}
        with self.connect(profile, tx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}) as sdr:
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.set("radio", "radio.tx.c0.frequency_hz", 2.6e9)
            self.assertTrue(any(v["check"] == "radio.rf_envelope" and v["key"] == "radio.tx.c0.frequency_hz" for v in raised.exception.violations), raised.exception.violations)
            self.assertEqual(sdr.tx_channels[0].frequency, 2.45e9, "the rejected value was never applied")
        self.assertEqual(sdr.manifest["action_log"][-1], raised.exception.entry)

    # -- Vision §3's second snippet

    def test_v3_parameters_through_attributes(self) -> None:
        # A receive channel's frequency, gain and antenna are asked for at connect, read through the attributes, and set on a stopped stream
        # (RM-21, EA-16); the rate waits for the bench, and a transmit value is taken while its bursts do not hold it (note 47 stage 4).
        stream = ezsdr.Stream([0], sample_rate=19.5e6, frequency={0: 2.45e9}, gain={0: 20}, antenna={0: "RX2"})
        with self.connect(rx={0: stream}, tx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}) as sdr:
            channel = sdr.rx_channels[0]
            self.assertEqual((channel.frequency, sdr.rx.sample_rate, channel.gain, channel.antenna), (2.45e9, 20e6, 20.0, "RX2"), "19.5 Msps is coerced to the x3xx grid at connect")
            # While the stream streams the Provider refuses what the Kernel admits (KC-27): `Rejected`, with its event.
            for name, value in (("frequency", 2.4e9), ("gain", 10), ("antenna", "RX2")):
                with self.assertRaises(ezsdr.Rejected) as refused:
                    setattr(channel, name, value)
                self.assertEqual(refused.exception.event["kind"], "radio.COMMAND_REJECTED")
                self.assertIn("streaming", str(refused.exception))
            with self.assertRaises(ezsdr.Error) as refused:
                sdr.rx.sample_rate = 10e6
            self.assertIn("held-open streamer", str(refused.exception))
            sdr.tx_channels[0].frequency = 2.4e9
            self.assertEqual(sdr.tx_channels[0].frequency, 2.4e9, "a transmit value on an idle stream is taken")
            self.assertEqual(sdr.rx.sample_rate, 20e6, "nothing changed")
        self.assertEqual([entry["action"]["kind"] for entry in sdr.manifest["action_log"]], ["set_parameter"] * 4, "the client refusals submit nothing")

    def test_ea_16_channel_keys_and_stream_membership(self) -> None:
        # The channels' values and stream membership are asked for at connect; the setters refuse during the Run
        # (a client Error naming connect), and the Kernel's refusal stays pinned through `set` (RS-52).
        tx = ezsdr.Stream([0], frequency={0: 2.45e9}, gain={0: 10}, antenna={0: "TX/RX"})
        with self.connect(rx={1: ezsdr.Stream([0], sample_rate=20e6, frequency={0: 2.45e9}, gain={0: 10}, antenna={0: "RX2"})}, tx={0: tx}) as sdr:
            rx, tx = sdr.rx_channels[0], sdr.tx_channels[0]
            self.assertEqual((rx.frequency, rx.gain, rx.antenna, rx.stream), (2.45e9, 10.0, "RX2", 1))
            self.assertEqual((tx.frequency, tx.gain, tx.antenna, tx.stream), (2.45e9, 10.0, "TX/RX", 0))
            self.assertEqual((sdr.rx_streams[0].channels, sdr.rx_streams[1].channels), ([], [0]))
            self.assertEqual((sdr.tx_streams[0].channels, sdr.tx_streams[1].channels), ([0], []))
            self.assertEqual(sdr.rx_streams[1].sample_rate, 20e6)
            for setter in (lambda: setattr(rx, "stream", 0), lambda: setattr(sdr.tx_streams[0], "sample_rate", 10e6), lambda: setattr(sdr.rx_streams[1], "sample_rate", 10e6)):
                with self.assertRaises(ezsdr.Error):
                    setter()
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.set("radio", "radio.rx.c0.stream", 0)
            self.assertTrue(any(v["check"] == "ezsdr.update_class" and v["key"] == "radio.rx.c0.stream" for v in raised.exception.violations), raised.exception.violations)
            self.assertEqual(rx.stream, 1, "the rejected value was never applied")
        self.assertEqual([entry["action"]["kind"] for entry in sdr.manifest["action_log"]], ["set_parameter"])
        self.assertEqual(sdr.manifest["action_log"][-1]["outcome"]["kind"], "rejected")

    def test_ea_16_repeat_with_a_frequency_on_a_busy_stream_raises_and_sends_no_burst(self) -> None:
        # S13 (EA-16, RM-21): the Provider refuses a transmit value for a stream that is playing, so the call raises before its burst; the
        # playing burst is not stopped and no second burst is logged.
        with self.connect(tx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}) as sdr:
            sdr.tx.repeat(ramp())
            sdr.sleep(0.01)
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.tx.repeat(ramp(), frequency={0: 2.46e9})
            self.assertEqual(raised.exception.event["kind"], "radio.COMMAND_REJECTED")
        self.assertEqual([entry["action"].get("verb") for entry in sdr.manifest["action_log"]].count("start_repeat"), 1, "no second burst is logged")

    def test_ea_16_a_transmit_value_on_an_empty_stream_is_used_by_the_next_repeat(self) -> None:
        # S1 (EA-16, RM-21): after a repeat stops and its in-flight window passes, a transmit value is taken and the next repeat radiates it.
        x = ramp()
        with self.retune_session() as sdr:
            sdr.tx.repeat(x)
            sdr.sleep(0.01)
            sdr.tx.stop()
            sdr.sleep(0.02)  # past the 10 ms window (x3xx tx_in_flight_ns)
            sdr.tx_channels[0].frequency = 2.0e9
            sdr.tx.repeat(x)
            sdr.sleep(0.01)
            y = sdr.rx.capture(1000)
        rotation(y, x)

    def test_ea_16_a_transmit_value_for_a_booked_stream_is_rejected_and_the_burst_keeps_its_value(self) -> None:
        # S2 (EA-16, RM-21): a repeat booked for the future holds the stream; a value before its start is refused, and the burst radiates the value it had.
        x = ramp()
        with self.connect(rx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}, tx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}) as sdr:
            at = sdr.after(0.05)
            sdr.tx.repeat(x, at=at)
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.tx_channels[0].frequency = 2.0e9
            self.assertEqual(raised.exception.event["kind"], "radio.COMMAND_REJECTED")
            y = sdr.rx.capture(3000, at=dict(at, ticks=at["ticks"] - 1_000_000))
        offset, _ = rotation(y[1000 + 45:], x)
        self.assertEqual(offset, 0)

    def test_ea_16_a_transmit_value_after_a_stop_waits_for_the_in_flight_window(self) -> None:
        # S3 (EA-16, RM-16, RM-21): a value is refused while the repeat plays and within the in-flight window after its Stop, and taken after it.
        with self.retune_session() as sdr:
            sdr.tx.repeat(ramp())
            sdr.sleep(0.01)
            with self.assertRaises(ezsdr.Rejected):
                sdr.tx_channels[0].frequency = 2.0e9
            sdr.tx.stop()
            sdr.sleep(0.001)
            with self.assertRaises(ezsdr.Rejected):
                sdr.tx_channels[0].frequency = 2.0e9
            sdr.sleep(0.02)
            sdr.tx_channels[0].frequency = 2.0e9
            self.assertEqual(sdr.tx_channels[0].frequency, 2.0e9)

    def test_ea_16_the_last_transmit_value_before_a_repeat_is_the_one_it_transmits(self) -> None:
        # S4 (EA-16, RM-21): two values on an empty stream; the repeat radiates the last one, and the connect value is not heard.
        x = ramp()
        with self.retune_session(rx={0: ezsdr.Stream([0], frequency={0: 2.1e9})}) as sdr:
            sdr.tx_channels[0].frequency = 2.2e9
            sdr.tx_channels[0].frequency = 2.1e9
            sdr.tx.repeat(x)
            sdr.sleep(0.01)
            y = sdr.rx.capture(1000)
        rotation(y, x)

    def test_ea_16_a_value_for_an_empty_transmit_stream_leaves_the_busy_one_undisturbed(self) -> None:
        # S6 (EA-16, RM-21): stream 0 repeats while stream 1 is empty; a value for stream 1 is taken and the repeat on stream 0 radiates its own waveform.
        x = ramp()
        with self.connect(rx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}, tx={0: ezsdr.Stream([0], frequency={0: 2.45e9}), 1: [1]}) as sdr:
            sdr.tx_streams[0].repeat(x)
            sdr.sleep(0.01)
            sdr.tx_channels[1].frequency = 2.0e9
            self.assertEqual(sdr.tx_channels[1].frequency, 2.0e9)
            y = sdr.rx.capture(1000)
        rotation(y, x)

    def test_ea_16_repeat_validates_every_channel_before_sending(self) -> None:
        # Sol 8 (EA-16): every channel and value is checked before the first request, so a bad argument sends nothing.
        bad = [
            {"frequency": {0: 2.46e9, 5: 2.47e9}},  # channel 5 is not in the stream
            {"frequency": {0: 2.46e9}, "gain": {0: "loud"}},  # a gain is a number
            {"antenna": {0: 3}},  # an antenna is a string
        ]
        with self.connect(tx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}) as sdr:
            for kwargs in bad:
                with self.assertRaises(ezsdr.Error, msg=str(kwargs)):
                    sdr.tx.repeat(ramp(), **kwargs)
            with self.assertRaises(ezsdr.Error):
                sdr.tx.repeat(np.zeros((2, 100), dtype=np.complex64), frequency={0: 2.46e9})
        self.assertEqual(sdr.manifest["action_log"], [], "nothing was sent")

    def test_ea_16_an_admitted_first_value_followed_by_a_refused_second_sends_no_burst(self) -> None:
        # Sol 8 (EA-16, RM-19): the frequency is admitted and stays staged; the gain the envelope refuses stops the call before the burst.
        profile = self.default_profile()
        profile["environment"]["radio.rf_envelope"] = {"allowed_bands": [{"lo_hz": 2.4e9, "hi_hz": 2.5e9}], "max_gain_db": 10.0}
        with self.connect(profile, tx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}) as sdr:
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.tx.repeat(ramp(), frequency={0: 2.46e9}, gain={0: 20.0})
            self.assertTrue(any(v["check"] == "radio.rf_envelope" for v in raised.exception.violations), raised.exception.violations)
            self.assertEqual(sdr.tx_channels[0].frequency, 2.46e9, "the admitted value stays staged")
        actions = [(entry["action"].get("key") or entry["action"].get("verb"), entry["outcome"]["kind"]) for entry in sdr.manifest["action_log"]]
        self.assertEqual(actions, [("radio.tx.c0.frequency_hz", "admitted"), ("radio.tx.c0.gain_db", "rejected")], "no burst is sent")

    def test_ea_16_a_channel_gain_on_an_empty_stream_is_accepted_before_a_repeat(self) -> None:
        with self.connect(tx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}) as sdr:
            sdr.tx_channels[0].gain = 5.0
            self.assertEqual(sdr.tx_channels[0].gain, 5.0)
            sdr.tx.repeat(ramp())
        self.assertEqual([entry["outcome"]["kind"] for entry in sdr.manifest["action_log"]], ["admitted", "admitted"])

    def test_ea_16_a_transmit_sample_rate_names_the_bench_condition(self) -> None:
        with self.connect(tx={0: ezsdr.Stream([0], frequency={0: 2.45e9})}) as sdr:
            with self.assertRaises(ezsdr.Error) as refused:
                sdr.tx_streams[0].sample_rate = 10e6
        self.assertIn("held-open streamer", str(refused.exception))
        self.assertEqual(sdr.manifest["action_log"], [], "nothing was sent")

    def test_ea_16_stream_channels_are_read_only(self) -> None:
        with self.connect() as sdr:
            stream = sdr.rx_streams[0]
            self.assertEqual(stream.channels, [0])
            with self.assertRaises(AttributeError):
                stream.channels = [1]
            with self.assertRaises(ezsdr.Error):
                sdr.rx_channels[1].stream = 0
            self.assertEqual(stream.channels, [0], "membership is fixed during the Run")

    def test_ea_16_radio_side_is_stream_zero(self) -> None:
        # The streams differ in rate at connect (the setters refuse during the Run), so an alias of another stream shows.
        rx = {0: ezsdr.Stream([0], sample_rate=2e6), 1: ezsdr.Stream([1], sample_rate=20e6)}
        tx = {0: ezsdr.Stream([0], sample_rate=2e6), 1: ezsdr.Stream([1], sample_rate=20e6)}
        with self.connect(rx=rx, tx=tx) as sdr:
            radio = sdr.radio()
            self.assertIs(radio.rx, radio.rx_streams[0])
            self.assertIs(radio.tx, radio.tx_streams[0])
            self.assertEqual((sdr.rx.sample_rate, sdr.tx.sample_rate), (2e6, 2e6))
            self.assertEqual((sdr.rx_streams[1].sample_rate, sdr.tx_streams[1].sample_rate), (20e6, 20e6))
            self.assertEqual(sdr.rx_channels[0].stream, radio.rx_channels[0].stream)
            self.assertEqual(sdr.rx_streams[0].channels, radio.rx_streams[0].channels)

    def test_ea_16_alias_without_stream_zero_raises_error(self) -> None:
        profile = self.default_profile()
        profile["bindings"]["radio"]["profile"] = {"name": "ideal", "version": {"major": 2, "minor": 0, "patch": 0}}
        profile["bindings"]["radio"]["selector"] = {"id": "radio", "rx": 1, "tx": 0}
        profile["environment"]["sim.channel"]["couplings"] = []
        with self.connect(profile) as sdr:
            with self.assertRaises(ezsdr.Error):
                _ = sdr.tx

    def test_ea_16_stream_one_actions_target_their_nodes(self) -> None:
        with self.connect(rx={0: [0], 1: [1]}, tx={1: [1]}) as sdr:
            started = sdr.rx_streams[1].start()
            stopped = sdr.rx_streams[1].stop()
            repeated = sdr.tx_streams[1].repeat(ramp())
        self.assertEqual(started["action"]["target"]["path"], "rx/s1")
        self.assertEqual(stopped["action"]["target"]["path"], "rx/s1")
        self.assertEqual(repeated["action"]["target"]["path"], "tx/s1")

    def test_ea_16_repeat_refuses_a_channel_count_mismatch_before_submit(self) -> None:
        with self.connect(tx={0: [0]}) as sdr:
            with self.assertRaises(ezsdr.Error) as raised:
                sdr.tx_streams[0].repeat(np.stack([ramp(), ramp()]))
            self.assertIn("1 channels", str(raised.exception))
            self.assertIn("waveform has 2", str(raised.exception))
        actions = [entry["action"] for entry in sdr.manifest["action_log"]]
        self.assertEqual(actions, [], "the stream's channels were set at connect, and the refused call sent nothing")

    def test_ea_17_stream_one_capture_uses_its_recorder(self) -> None:
        profile = self.default_profile()
        name = self.add_recorder(profile, 1)
        with self.connect(profile, rx={0: [0], 1: [1]}) as sdr:
            self.assertEqual(sdr.rx_streams[1].capture(100).shape, (100,))
        self.assertEqual(set(sdr.manifest["artifacts"]), {name})

    # -- Vision §54

    def test_v54_a_sweep_of_child_runs(self) -> None:
        spec = {
            "version": 1,
            "requirements": {"vocabularies": [{"id": "radio", "major": 2}, {"id": "sink", "major": 1}]},
            "resources": {"radio": {"kind": "radio.device", "requires": {"radio.rx.c0.stream": {"kind": "eq", "value": 0}}}},
            "outputs": [{
                "id": "rec", "kind": "sink.capture", "params": {"sink.capture_samples": 1000},
                "feed": {"port": {"component": "radio", "port": "rx0"}, "policy": "drop_oldest", "capacity": 64},
            }],
        }
        results = []
        with self.connect() as sdr:
            for gain in (0.0, 10.0, 20.0):
                child = copy.deepcopy(spec)
                child["resources"]["radio"]["requires"]["radio.rx.c0.gain_db"] = {"kind": "eq", "value": gain}
                result = sdr.run(child, duration=0.01)
                self.assertEqual(result.manifest["run"]["parent"], sdr.run_id)
                self.assertEqual(result.termination, {"kind": "stopped", "cause": {"kind": "client"}})
                self.assertEqual(result.capture("rec").shape, (1000,))
                results.append(result)
        children = sdr.manifest["run"]["children"]
        self.assertEqual([c["seq"] for c in children], [r.entry["seq"] for r in results])
        self.assertEqual([c["run"] for c in children], [r.manifest["run"]["id"] for r in results])

    def test_ea_05_a_failed_child_preserves_the_session(self) -> None:
        # An output id needs no derived address (SB-22), so this long one is valid; its
        # capture file's name is not, and the child fails while running (HD-10).
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
                "resources": {"radio": {"kind": "radio.device", "requires": {"radio.rx.c0.stream": {"kind": "eq", "value": 0}}}},
                "outputs": [{"id": name, "kind": "sink.capture", "params": {"sink.capture_samples": 1},
                              "feed": {"port": {"component": "radio", "port": "rx0"}, "policy": "drop_oldest", "capacity": 64}}],
            }
            result = sdr.run(spec, profile=profile, duration=0.001)
            self.assertEqual((result.termination["kind"], result.termination["stage"]), ("failed", "run"))
            self.assertTrue(result.termination["reason"], result.termination)
            self.assertEqual(result.manifest["artifacts"], {})
            self.assertEqual(sdr.rx.capture(1).shape, (1,), "parent Session is still usable")

    def test_v54_sleep_is_run_time(self) -> None:
        with self.connect() as sdr:
            before = sdr.now
            sdr.sleep(10.0)
            after = sdr.now
        self.assertEqual(after["ticks"] - before["ticks"], 10_000_000_000, "10 s on the Simulation Engine's nanosecond root")

    def test_v58_02_virtual_time_runs_faster_than_the_wall_clock(self) -> None:
        # §58 #2 on a scene with nothing to compute: with the receive stream stopped, a
        # free-running Authority crosses 10 s at once and a wall-paced one takes 10 s.
        with self.connect() as sdr:
            sdr.rx.stop()
            before = sdr.now
            began = time.monotonic()
            sdr.sleep(10.0)
            elapsed = time.monotonic() - began
            after = sdr.now
        self.assertEqual(after["ticks"] - before["ticks"], 10_000_000_000)
        self.assertLess(elapsed, 10.0, "virtual time runs faster than the wall clock (§58 #2)")

    # -- Vision §58 #3, #1 and #10

    def test_v58_03_the_same_calls_give_the_same_samples(self) -> None:
        def session() -> np.ndarray:
            with self.connect(tx={0: [0]}) as sdr:
                sdr.tx.repeat(ramp())
                sdr.sleep(0.003)
                return sdr.rx.capture(2000)

        np.testing.assert_array_equal(session(), session())

    def test_v58_01_the_same_script_under_another_profile(self) -> None:
        def script(sdr: ezsdr.Session) -> np.ndarray:
            sdr.tx.repeat(ramp())
            self.wait_for_tx_signal(sdr)
            return sdr.rx.capture(2000)

        with self.connect(tx={0: [0]}) as sdr:
            rotated = script(sdr)
        ideal = self.default_profile()
        ideal["bindings"]["radio"]["profile"] = {"name": "ideal", "version": {"major": 2, "minor": 0, "patch": 0}}
        # `ideal` uses channel counts rather than an X3xx physical layout (design 33 D5).
        ideal["bindings"]["radio"]["selector"] = {"id": "radio", "rx": 1, "tx": 2}
        with self.connect(ideal, tx={0: [0]}) as sdr:
            self.assertEqual((len(sdr.rx_channels), len(sdr.tx_channels), len(sdr.tx_streams)), (1, 2, 2))
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
        with self.connect(profile, rx={0: [0, 1]}, tx={0: [0, 1]}) as sdr:
            sdr.tx.repeat(x)
            self.assertEqual(sdr.tx_streams[0].channels, [0, 1])
            self.wait_for_tx_signal(sdr)
            y = sdr.rx.capture(2000)
        self.assertEqual(y.shape, (2, 2000))
        for channel in range(2):
            rotation(y[channel], x[channel])

    def test_ea_16_events_and_wait_for(self) -> None:
        with self.connect() as sdr:
            entry = sdr.submit({
                "kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"kind": "output", "output": "rec"},
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
                sdr.set("radio", "radio.rx.c0.gain_db", 1)
            self.assertEqual(ended.exception.termination, {"kind": "stopped", "cause": {"kind": "client"}})
            # An error reply is read whole and abandons nothing (EA-16).
            self.assertIsInstance(sdr.events(), list)
        self.assertEqual(sdr.manifest["termination"]["reason"], {"kind": "stopped", "cause": {"kind": "client"}})

    def test_ea_17_each_capture_gets_its_own_samples(self) -> None:
        # The recorder's answer names the Action it answers, and a capture waits for its own
        # (Review H, P0-2; spec 27 §3).
        with self.connect() as sdr:
            self.assertEqual(sdr.rx.capture(100).shape, (100,))
            self.assertEqual(sdr.rx.capture(200).shape, (200,))
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(1_000_000, timeout=0.001)
            self.assertEqual(sdr.rx.capture(100, timeout=3.0).shape, (100,), "not the timed-out request's million")
            raw = sdr.submit({
                "kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"kind": "output", "output": "rec"},
                "at": None, "params": {"sink.capture_samples": 777},
            })
            self.assertEqual(raw["outcome"]["kind"], "admitted")
            self.assertEqual(sdr.rx.capture(100).shape, (100,), "not the raw request's 777")
            # A SetParameter of sink.capture_samples on the output is a request too; a capture
            # naming a resource goes to that resource, which refuses the key (#54; RS-14,
            # RS-17).
            for action, outcome in (
                ({"kind": "set_parameter", "target": {"kind": "output", "output": "rec"}, "key": "sink.capture_samples", "value": 555}, "admitted"),
                ({"kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"kind": "resource", "resource": "radio", "path": "rx/s0"}, "at": None, "params": {"sink.capture_samples": 333}}, "rejected"),
            ):
                self.assertEqual(sdr.submit(action)["outcome"]["kind"], outcome)
                self.assertEqual(sdr.rx.capture(100).shape, (100,), f"not {action['kind']}'s request")
            rejected = sdr.submit({"kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"kind": "output", "output": "rec"}, "at": None, "params": {}})
            self.assertEqual(rejected["outcome"]["kind"], "rejected")
            self.assertEqual(sdr.rx.capture(100).shape, (100,), "after a rejected capture")

    def test_ea_17_a_capture_keeps_its_first_deadline(self) -> None:
        # Another request's announcement inside the timeout does not restart it (Review H,
        # P2-5; Review I, P1-D): the raw 10 000-sample request is written at ~10 ms, this
        # capture's at ~16 ms, and 12 ms is the whole budget.
        with self.connect() as sdr:
            raw = sdr.submit({"kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"kind": "output", "output": "rec"}, "at": None, "params": {"sink.capture_samples": 10_000}})
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(5000, timeout=0.012)
            written = [e for e in sdr.events() if e["kind"] == ezsdr.session.CAPTURE_WRITTEN]
            self.assertEqual([e["payload"]["action"] for e in written], raw["outcome"]["dispatched"], "the foreign announcement came inside the timeout")

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
        with self.connect(tx={0: [0]}) as sdr:
            sdr.tx.repeat(ramp())
            self.wait_for_tx_signal(sdr)
            first, second = sdr.rx.request(1000), sdr.rx.request(1500)
            self.assertNotEqual(first.entry["outcome"]["dispatched"], second.entry["outcome"]["dispatched"])
            b = sdr.rx.result(second)
            a = sdr.rx.result(first)
            self.assertEqual((a.shape, b.shape), ((1000,), (1500,)))
            # One continuous stretch of the repeated waveform across the boundary.
            rotation(np.concatenate([a, b]), ramp())
        spans = [(c["first"]["ticks"], c["end"]["ticks"]) for c in (art["continuity"][0] for art in sdr.manifest["artifacts"]["rec"])]
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
        (artifact,) = sdr.manifest["artifacts"]["rec"]
        self.assertEqual(artifact["continuity"][0]["first"]["ticks"], (at["ticks"] - t0) // 1000, "the first sample at `at`, 1 Msps from T0")

    def test_ea_16_repeat_at_an_instant(self) -> None:
        x = ramp()
        with self.connect(tx={0: [0]}) as sdr:
            at = sdr.after(0.2)
            entry = sdr.tx.repeat(x, at=at)
            y = sdr.rx.capture(3000, at=dict(at, ticks=at["ticks"] - 1_000_000))
        self.assertEqual(entry["action"]["at"], at)
        # Silence until `at`, then the waveform from its first sample (1 Msps on a 1 GHz root),
        # after the `x3xx` 45-sample transmit path delay (MR-3, RM-23); on time, so no TIME_ERROR.
        start = 1000 + 45
        np.testing.assert_allclose(y[:start], 0, atol=1e-6)
        offset, _ = rotation(y[start:], x)
        self.assertEqual(offset, 0)
        counts = {c["kind"]: c["count"] for c in sdr.manifest["events"]["counters"] if c["source"] == {"kind": "node", "node": {"node": 0, "path": "radio"}}}
        self.assertEqual(counts["radio.TIME_ERROR"], 0)

    def test_ea_16_aligned_gives_its_calls_one_instant(self) -> None:
        x = ramp()
        with self.connect(tx={0: [0]}) as sdr:
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
            # The stream is still playing `sent`; `later` is untimed, so it starts after the Stop's region (RM-15, F36).
            sdr.tx.stop()
            later = sdr.tx.repeat(x)
        self.assertEqual((sent["action"]["at"], request.entry["action"]["at"]), (at, at))
        self.assertIsNone(later["action"]["at"], "the block's instant ends with it")
        # Both begin at `at`: the waveform's first sample after the 45-sample path delay.
        np.testing.assert_allclose(y[:45], 0, atol=1e-6)
        offset, _ = rotation(y[45:], x)
        self.assertEqual(offset, 0)

    def test_ea_16_switch_inside_aligned_takes_the_block_instant(self) -> None:
        # F35 (EA-16, RM-37): `switch` takes the block's instant like `repeat`, and the Provider accepts it on a playing repeat.
        x = ramp()
        with self.connect(tx={0: [0]}) as sdr:
            sdr.tx.repeat(x)
            sdr.sleep(0.01)
            at = sdr.after(0.1)
            with sdr.aligned(at):
                switched = sdr.tx.switch(ramp(channel=1))
        self.assertEqual(switched["action"]["at"], at)
        self.assertEqual(switched["outcome"]["kind"], "admitted")

    def test_ea_16_a_repeat_right_after_stop_transmits(self) -> None:
        # F36 (EA-16, RM-15, RM-16): after `stop()` the next `repeat` is untimed. It starts at the end of the Stop's region, with no
        # TIME_ERROR, so it transmits and is not refused.
        x = ramp()
        with self.connect(tx={0: [0]}) as sdr:
            sdr.tx.repeat(x)
            sdr.tx.stop()
            later = sdr.tx.repeat(x)
            sdr.sleep(0.06)
            y = sdr.rx.capture(3000)
        self.assertIsNone(later["action"]["at"])
        self.assertEqual([e for e in sdr.manifest["events"]["delivered"] if e["kind"] == "radio.TIME_ERROR"], [])
        _, phasor = rotation(y, x)
        self.assertAlmostEqual(abs(phasor), 1.0, places=5)

    def test_ea_16_second_repeat_on_a_playing_stream_raises_rejected(self) -> None:
        # RM-15 (sequence 2): an untimed repeat on a playing repeat is refused with the overlap reason, and no TIME_ERROR.
        x = ramp()
        with self.connect(tx={0: [0]}) as sdr:
            sdr.tx.repeat(x)
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.tx.repeat(x)
        self.assertIn("overlaps an accepted booking", str(raised.exception))
        self.assertEqual([e for e in sdr.manifest["events"]["delivered"] if e["kind"] == "radio.TIME_ERROR"], [])

    def test_ea_16_next_at_finds_the_waveform_s_start_again(self) -> None:
        x = ramp()
        with self.connect(tx={0: [0]}) as sdr:
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
        (artifact,) = sdr.manifest["artifacts"]["rec"]
        self.assertEqual(artifact["continuity"][0]["first"]["ticks"], (at["ticks"] - t0) // 1000, "the capture starts at `after`'s instant")

    def test_ea_16_sleep_reaches_the_instant_after_names(self) -> None:
        # EA-16, EA-12 (spec 20, VF-2): one reading and one rounding, so `sleep(d)` stands where
        # `after(d)` names from the same `now`; the float product put 0.0041 a tick later.
        with self.connect() as sdr:
            a = sdr.after(0.0041)
            t = sdr.sleep(0.0041)
        self.assertEqual(t, a)

    def test_ea_17_a_capture_across_a_restart_is_refused(self) -> None:
        with self.connect() as sdr:
            sdr.submit({
                "kind": "vocabulary", "ns": "sink", "verb": "capture", "target": {"kind": "output", "output": "rec"},
                "at": None, "params": {"sink.capture_samples": 20_000},
            })
            sdr.sleep(0.005)
            sdr.rx.stop()
            sdr.sleep(0.002)
            sdr.rx.start()
            event = sdr.wait_for([ezsdr.session.CAPTURE_WRITTEN], timeout=1.0)
            artifact = event["payload"]["artifact"]
            self.assertEqual(len(artifact["continuity"]), 2)
            with self.assertRaises(ezsdr.Error):
                ezsdr.samples(sdr.read(artifact), artifact)

    def test_ea_16_rx_stop_then_start(self) -> None:
        # EA-16, RM-12, RM-21 (spec 22, VH-1, VH-6): after rx.stop() nothing starts the stream;
        # rx.start() starts it again on a new SampleClock at the configuration in force, and
        # is refused while the stream streams; calls act in the order made.
        with self.connect() as sdr:
            sdr.rx.stop()
            sdr.sleep(0.062)
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(1000, timeout=0.2)
            sdr.rx.start()
            self.assertEqual(len(sdr.rx.capture(1000)), 1000)
            sdr.rx.stop()
            sdr.sleep(0.01)
            at = sdr.after(0.02)
            sdr.rx.start(at=at)
            self.assertEqual(len(sdr.rx.capture(1000)), 1000)
            # RM-21: the stream is streaming, so a start is refused with the Provider's reason and changes nothing.
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.rx.start()
            self.assertIn("streaming", str(raised.exception))
            sdr.rx.stop()
            sdr.sleep(0.1)
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(1000, timeout=0.2)
            sdr.rx.stop()
            sdr.rx.start()
            self.assertEqual(len(sdr.rx.capture(1000)), 1000)
            self.assertEqual(len(sdr.effective["radio"]), 20)
        # The first segment, stopped at T0 before its first sample, has no clock (RM-25); the
        # three others end at their cuts, the last at the Session's close.
        clocks = [c for c in sdr.manifest["clocks"]["sample_clocks"] if c["stream"]["path"] == "radio/rx/s0"]
        self.assertEqual(len(clocks), 3, clocks)
        self.assertTrue(all(c.get("ended_at") is not None for c in clocks), clocks)
        self.assertEqual([c["root_ticks_per_tick"]["num"] for c in clocks], [1000, 1000, 1000], clocks)
        # The start, received 20 ms before its `at`, is ready then: its clock begins the start
        # lead after its receipt, the later of that and `at` (RM-25).
        lead, received = 50_000_000, at["ticks"] - 20_000_000
        self.assertLessEqual(abs(clocks[1]["origin"]["ticks"] - max(at["ticks"], received + lead)), 1000, (clocks, at))

    def test_ea_16_repeat_uses_stream_membership_without_setting_it(self) -> None:
        with self.connect(tx={0: [0]}) as sdr:
            sdr.tx.repeat(ramp())
            sdr.sleep(0.01)
            sdr.tx.switch(ramp(500))
        actions = [entry["action"] for entry in sdr.manifest["action_log"]]
        self.assertEqual([action["kind"] for action in actions], ["vocabulary", "vocabulary"])
        self.assertEqual([action.get("verb") for action in actions], ["start_repeat", "start_repeat"])
        self.assertEqual([action.get("key") for action in actions], [None, None])
        self.assertEqual(actions[0]["target"], {"kind": "resource", "resource": "radio", "path": "tx/s0"})
        self.assertEqual(actions[1]["params"], {"radio.repeat_operation": "switch"}, "the switch's only parameter is its mark")

    # -- connect's initial streams (EA-10, EA-16, RM-30)

    def test_ea_16_default_connect_has_no_stream(self) -> None:
        # RM-30: no channel is in a stream unless connect says so; the default profile's recorder is
        # linked to rx0 all the same, and the Session connects and nothing flows.
        with ezsdr.connect(runs_dir=self.runs) as sdr:
            self.assertEqual([c.stream for c in sdr.rx_channels], [-1, -1])
            self.assertEqual([c.stream for c in sdr.tx_channels], [-1, -1])
            self.assertEqual((sdr.rx_streams[0].channels, sdr.tx_streams[0].channels), ([], []))
            self.assertEqual(sdr._status()["sample_clocks"], [])
            with self.assertRaises(ezsdr.Error) as raised:
                sdr.rx  # the alias reads a receive stream that holds no channel
            self.assertIn("connect(rx=", str(raised.exception))
            with self.assertRaises(ezsdr.Error) as raised:
                sdr.rx.capture(10)
            self.assertIn("connect(rx=", str(raised.exception))
            with self.assertRaises(ezsdr.Error) as raised:
                sdr.tx.repeat(ramp())
            self.assertIn("0 channels", str(raised.exception))
            sdr.sleep(0.1)
        self.assertEqual(sdr.manifest["action_log"], [])
        self.assertEqual(sdr.manifest["artifacts"], {})

    def test_ea_16_connect_direction_maps(self) -> None:
        # A direction's map is its whole channel set; a direction left out, `{}` and `[]` are in no stream.
        def sets(**streams: object) -> tuple:
            with ezsdr.connect(runs_dir=self.runs, **streams) as sdr:
                return (
                    [c.stream for c in sdr.rx_channels], [c.stream for c in sdr.tx_channels],
                    [s.channels for s in sdr.rx_streams], [s.channels for s in sdr.tx_streams],
                )
        self.assertEqual(sets(tx={0: [0, 1]}), ([-1, -1], [0, 0], [[], []], [[0, 1], []]))
        self.assertEqual(sets(tx={}), ([-1, -1], [-1, -1], [[], []], [[], []]))
        self.assertEqual(sets(rx={}, tx={}), ([-1, -1], [-1, -1], [[], []], [[], []]))
        self.assertEqual(sets(rx={1: [1]}), ([-1, 1], [-1, -1], [[], [1]], [[], []]))
        self.assertEqual(sets(rx={1: [1], 0: [0]}, tx={0: []}), ([0, 1], [-1, -1], [[0], [1]], [[], []]))

    def test_ea_16_a_transmit_rate_below_1_msps_is_refused_at_connect(self) -> None:
        # KL-40: transmit rates start at 1 MS/s on x3xx (RM-8: 0.5 MS/s is outside the transmit grid, never coerced to 1 MS/s); receive keeps 0.5 MS/s.
        with self.assertRaises(ezsdr.Error):
            self.connect(tx={0: ezsdr.Stream([0], sample_rate=0.5e6)})
        with self.connect(tx={0: ezsdr.Stream([0], sample_rate=1e6)}) as sdr:
            self.assertEqual(sdr.tx.sample_rate, 1e6)
        with self.connect(rx={0: ezsdr.Stream([0], sample_rate=0.5e6)}) as sdr:
            self.assertEqual(sdr.rx.sample_rate, 0.5e6)

    def test_ea_16_connect_names_a_stream(self) -> None:
        # A name is the Easy API's: the server numbers it, the reply carries the table, and the
        # Kernel's Actions and the log keep numbers.
        with ezsdr.connect(rx={"main": ezsdr.Stream([0], sample_rate=2e6), 0: [1]}, tx={"out": [0]}, runs_dir=self.runs) as sdr:
            main = sdr.rx_streams["main"]
            self.assertEqual((main.channels, main.sample_rate), ([0], 2e6))
            self.assertIs(type(main), ezsdr.Rx)
            self.assertEqual(main._number, 1, "the explicit 0 is kept and the name takes the next number")
            self.assertEqual(sdr.rx_streams[0].channels, [1])
            self.assertEqual(sdr.tx_streams["out"]._number, 0)
            with self.assertRaises(KeyError):
                sdr.rx_streams["out"]  # a transmit name is not a receive stream
            with self.assertRaises(KeyError):
                sdr.tx_streams["nobody"]
            entry = main.start()
            self.assertEqual(entry["action"]["target"]["path"], "rx/s1", "the log names the number")

    def test_ea_16_a_stream_name_is_used_once_across_both_directions(self) -> None:
        with self.assertRaises(ezsdr.Error) as raised:
            ezsdr.connect(rx={"main": [0]}, tx={"main": [0]}, runs_dir=self.runs)
        self.assertIn("used twice", str(raised.exception))
        with ezsdr.connect(rx={"main": [0]}, tx={"out": [0]}, runs_dir=self.runs) as sdr:
            self.assertEqual((sdr.rx_streams["main"]._number, sdr.tx_streams["out"]._number), (0, 0))

    def test_ea_16_connect_orders_samples_by_logical_channel(self) -> None:
        # RM-3: a stream's samples come in logical channel order whatever order connect lists them in.
        profile = self.default_profile()
        profile["bindings"]["radio"]["profile"] = {"name": "ideal", "version": {"major": 2, "minor": 0, "patch": 0}}
        profile["bindings"]["radio"]["selector"] = {"id": "radio", "rx": 3, "tx": 2}
        profile["environment"]["sim.channel"]["couplings"] = [
            {"tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0},
            {"tx": "radio", "tx_channel": 1, "rx": "radio", "rx_channel": 2, "gain_db": 0.0},
        ]
        x = np.stack([ramp(channel=0), ramp(channel=1) * 0.5])
        with ezsdr.connect(profile, rx={0: [2, 0]}, tx={0: [1, 0]}, runs_dir=self.runs) as sdr:
            self.assertEqual((sdr.rx_streams[0].channels, sdr.tx_streams[0].channels), ([0, 2], [0, 1]))
            sdr.tx.repeat(x)
            self.wait_for_tx_signal(sdr)
            y = sdr.rx.capture(2000)
        self.assertEqual(y.shape, (2, 2000))
        for row in range(2):
            rotation(y[row], x[row])

    def named_profile(self, rx: list, tx: list, couplings: list) -> dict:
        """The default profile with the radio's layout lists and the simulated channel replaced (RM-28)."""
        profile = self.default_profile()
        profile["bindings"]["radio"]["selector"]["rx"] = rx
        profile["bindings"]["radio"]["selector"]["tx"] = tx
        profile["environment"]["sim.channel"]["couplings"] = [
            {"tx": "radio", "tx_channel": t, "rx": "radio", "rx_channel": r, "gain_db": 0.0} for t, r in couplings
        ]
        return profile

    def test_ea_16_a_channel_is_reached_by_its_name_through_a_server(self) -> None:
        profile = self.named_profile(["0:A:ubx:0#loop-rx", "0:B:ubx:0"], ["0:A:ubx:0#loop-tx", "0:B:ubx:0#spare-tx"], [(0, 0)])
        stream = ezsdr.Stream(["loop-rx"], gain={"loop-rx": 12}, frequency={0: 2.45e9})
        with self.connect(profile, rx={"main": stream}, tx={0: ["loop-tx"]}) as sdr:
            self.assertEqual(sdr.rx_channels["loop-rx"].stream, 0)
            self.assertEqual((sdr.rx_channels["loop-rx"].gain, sdr.rx_channels["loop-rx"].frequency), (12.0, 2.45e9))
            self.assertEqual((sdr.rx_channels[1].stream, sdr.tx_channels["loop-tx"].stream, sdr.tx_channels["spare-tx"].stream), (-1, 0, -1))
            self.assertEqual(sdr.rx_channels["loop-rx"]._number, sdr.rx_channels[0]._number)
            with self.assertRaises(KeyError) as raised:
                sdr.rx_channels["nowhere"]
            self.assertIn("['loop-rx']", str(raised.exception), "only the named entry is known; the second is reached by number")
            with self.assertRaises(KeyError):
                sdr.rx_channels["loop-tx"]
            self.assertEqual(sdr.rx_streams["main"].channels, [0])
        # A name the server does not know is refused with its text, before any Run.
        for streams, message in (({"rx": {0: ["nowhere"]}}, "no channel is named \"nowhere\""), ({"rx": {0: ["loop-tx"]}}, "is a transmit channel, not a receive channel")):
            with self.subTest(streams=streams):
                with self.assertRaises(ezsdr.Error) as raised:
                    self.connect(profile, **streams)
                self.assertIn(message, str(raised.exception))

    def test_ea_16_one_program_of_names_runs_on_two_layouts(self) -> None:
        def program(sdr: ezsdr.Session) -> np.ndarray:
            sdr.tx_streams["out"].repeat(ramp())
            self.wait_for_tx_signal(sdr)
            return sdr.rx_streams["main"].capture(2000)

        streams = {"rx": {"main": ezsdr.Stream(["loop-rx"])}, "tx": {"out": ["loop-tx"]}}
        first = self.named_profile(["0:A:ubx:0#loop-rx", "0:B:ubx:0#spare-rx"], ["0:A:ubx:0#loop-tx", "0:B:ubx:0#spare-tx"], [(0, 0)])
        # The same names on other numbers, and the coupling that is the same wire on them.
        second = self.named_profile(["0:B:ubx:0#spare-rx", "0:A:ubx:0#loop-rx"], ["0:B:ubx:0#spare-tx", "0:A:ubx:0#loop-tx"], [(1, 1)])
        for profile, number in ((first, 0), (second, 1)):
            with self.subTest(loop_rx=number):
                with self.connect(profile, **streams) as sdr:
                    self.assertEqual((sdr.rx_channels["loop-rx"]._number, sdr.tx_channels["loop-tx"]._number), (number, number))
                    self.assertEqual((sdr.rx_streams["main"].channels, sdr.tx_streams["out"].channels), ([number], [number]), "the stream holds the named channel, whatever its number")
                    _, phasor = rotation(program(sdr), ramp())
                    self.assertAlmostEqual(abs(phasor), 1.0, places=5)

    def test_ea_16_connect_initial_values(self) -> None:
        # The values a Session asks for are its first ones: no logged call, and the effective
        # configuration holds them as asked, a rate the grid lacks coerced like a SetParameter's.
        stream = ezsdr.Stream([0], sample_rate=19.5e6, frequency={0: 2.45e9}, gain={0: 10}, antenna={0: "TX/RX"})
        with ezsdr.connect(rx={0: stream}, runs_dir=self.runs) as sdr:
            channel = sdr.rx_channels[0]
            self.assertEqual((channel.frequency, channel.gain, channel.antenna, channel.stream), (2.45e9, 10.0, "TX/RX", 0))
            self.assertEqual(sdr.rx.sample_rate, 20e6, "19.5 Msps is coerced to the x3xx grid, as a SetParameter's would be")
            self.assertEqual(len(sdr.rx.capture(100)), 100)
        self.assertEqual([entry["action"].get("verb") for entry in sdr.manifest["action_log"]], ["capture"])
        preview = sdr.manifest["admission"]["coercions_preview"]
        self.assertTrue(any(p["coercion"]["key"] == "radio.rx.s0.sample_rate_hz" and p["coercion"]["requested"] == 19.5e6 and p["coercion"]["applied"] == 20e6 for p in preview), preview)

    def test_ea_16_connect_refuses_the_duplex_antenna_conflict(self) -> None:
        # RM-31: a transmit channel in a stream and its board's receive antenna on TX/RX.
        with self.assertRaises(ezsdr.RunEnded) as raised:
            ezsdr.connect(rx={0: ezsdr.Stream([0], antenna={0: "TX/RX"})}, tx={0: [0]}, runs_dir=self.runs)
        self.assertEqual(raised.exception.termination["kind"], "failed")
        self.assertEqual(raised.exception.termination["stage"], "validate")
        self.assertIn("RM-31", raised.exception.termination["reason"])
        with ezsdr.connect(rx={0: ezsdr.Stream([0], antenna={0: "TX/RX"})}, tx={}, runs_dir=self.runs) as sdr:
            self.assertEqual(sdr.rx_channels[0].antenna, "TX/RX")

    def test_ea_16_a_refused_connect_leaves_no_session(self) -> None:
        with self.assertRaises(ezsdr.ServerError) as raised:
            ezsdr.connect(rx={0: [7]}, runs_dir=self.runs)
        self.assertIn("no such channel c7", str(raised.exception))
        with ezsdr.connect(rx={0: [1]}, runs_dir=self.runs) as sdr:
            self.assertEqual(sdr.rx_streams[0].channels, [1], "the next connect is a new Session")

    def test_ea_16_connect_receive_only(self) -> None:
        with self.connect(tx={}) as sdr:
            self.assertEqual(sdr.tx_streams[0].channels, [])
            self.assertEqual(len(sdr.rx.capture(100)), 100)
            with self.assertRaises(ezsdr.Error):
                sdr.tx.repeat(ramp())

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

    # -- note 47 stage 3: receive values on a stopped stream (RM-21, EA-16)

    def retune_session(self, **rx_streams: object) -> ezsdr.Session:
        """A loopback whose transmitter radiates at 2.45 GHz and whose receiver is tuned to 2 GHz at connect (CH-4: it hears nothing)."""
        rx = rx_streams.pop("rx", {0: ezsdr.Stream([0], frequency={0: 2.0e9})})
        return self.connect(rx=rx, tx={0: ezsdr.Stream([0], frequency={0: 2.45e9})})

    def test_ea_16_rx_start_retunes_a_stopped_stream(self) -> None:
        x = ramp()
        with self.retune_session() as sdr:
            sdr.tx.repeat(x)
            sdr.sleep(0.01)
            np.testing.assert_allclose(sdr.rx.capture(1000), 0, atol=1e-9)
            sdr.rx.stop()
            sdr.rx.start(frequency={0: 2.45e9})
            y = sdr.rx.capture(1000)
            self.assertEqual(sdr.rx_channels[0].frequency, 2.45e9)
        rotation(y, x)
        kinds = [(entry["action"]["kind"], entry["action"].get("key") or entry["action"].get("verb")) for entry in sdr.manifest["action_log"] if entry["action"].get("verb") != "capture"]
        self.assertEqual(kinds[-3:], [("stop", None), ("set_parameter", "radio.rx.c0.frequency_hz"), ("vocabulary", "start_rx")])

    def test_ea_16_a_value_for_a_streaming_stream_is_rejected_and_admitted_value_differs(self) -> None:
        # S2 and KC-27: the Kernel admits the value and the Provider refuses it; the exception carries its event, matched by the Action's id; the
        # admitted value reads back while the stream still uses what it had, which a later start shows.
        x = ramp()
        with self.retune_session() as sdr:
            sdr.tx.repeat(x)
            sdr.sleep(0.01)
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.rx_channels[0].frequency = 2.45e9
            entry, event = raised.exception.entry, raised.exception.event
            self.assertEqual(event["kind"], "radio.COMMAND_REJECTED")
            self.assertIn(event["payload"]["action"], entry["outcome"]["dispatched"])
            self.assertIn("streaming", str(raised.exception))
            self.assertEqual(sdr.rx_channels[0].frequency, 2.45e9, "the admitted value (KC-27)")
            sdr.rx.stop()
            sdr.rx.start()
            np.testing.assert_allclose(sdr.rx.capture(1000), 0, atol=1e-9)  # the stream uses 2 GHz: the refused value was not kept
        self.assertEqual(sdr.manifest["action_log"][-4]["action"]["kind"], "set_parameter")
        self.assertEqual(sdr.manifest["action_log"][-4], entry)

    def test_ea_16_a_raw_submit_does_not_raise_for_a_provider_refusal(self) -> None:
        with self.retune_session() as sdr:
            entry = sdr.submit({"kind": "set_parameter", "target": ezsdr.Target.resource("radio"), "key": "radio.rx.c0.frequency_hz", "value": 2.45e9})
            self.assertEqual(entry["outcome"]["kind"], "admitted")
            self.assertEqual(sdr.events(0)[-1]["kind"], "radio.COMMAND_REJECTED")

    def test_ea_16_start_stops_on_a_provider_refusal_of_an_admitted_value(self) -> None:
        # The Kernel admits the first value, the Provider refuses it (the stream streams): no second value and no start_rx are sent.
        stream = ezsdr.Stream([0, 1], frequency={0: 2.0e9, 1: 2.0e9})
        with self.retune_session(rx={0: stream}) as sdr:
            before = len(sdr.manifest["action_log"]) if sdr.manifest else 0
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.rx.start(frequency={0: 2.1e9, 1: 2.2e9})
            self.assertEqual(raised.exception.event["kind"], "radio.COMMAND_REJECTED")
            first = raised.exception.entry
            self.assertIn(raised.exception.event["payload"]["action"], first["outcome"]["dispatched"])
            with self.assertRaises(ezsdr.Rejected):
                sdr.rx.start(frequency={0: 2.1e9}, gain={0: 3.0})
            self.assertEqual(sdr.rx_channels[1].frequency, 2.0e9, "the second value was never sent")
            refusals = [e for e in sdr.events(0) if e["kind"] == "radio.COMMAND_REJECTED"]
            self.assertEqual(len(refusals), 2)
        log = [entry["action"] for entry in sdr.manifest["action_log"]]
        self.assertEqual([a["kind"] for a in log], ["set_parameter", "set_parameter"], log)
        self.assertEqual([a["key"] for a in log], ["radio.rx.c0.frequency_hz"] * 2, "no gain, no second channel and no start_rx")

    def test_ea_16_a_refused_value_stops_the_start_and_keeps_the_staged_ones(self) -> None:
        # S9: the Kernel refuses the second value (beyond the board's range) at admission; the first stays staged, there is no rollback.
        stream = ezsdr.Stream([0, 1], frequency={0: 2.0e9, 1: 2.0e9})
        x = ramp()
        with self.retune_session(rx={0: stream}) as sdr:
            sdr.tx.repeat(x)
            sdr.rx.stop()
            with self.assertRaises(ezsdr.Rejected) as raised:
                sdr.rx.start(frequency={0: 2.45e9, 1: 9.0e12})
            self.assertEqual(raised.exception.violations[0]["key"], "radio.rx.c1.frequency_hz")
            sdr.sleep(0.1)
            with self.assertRaises(ezsdr.CaptureTimeout):
                sdr.rx.capture(1000, timeout=0.2)
            sdr.rx.start()
            y = sdr.rx.capture(1000)
        rotation(y[0] if y.ndim == 2 else y, x)
        log = [(entry["action"]["kind"], entry["outcome"]["kind"]) for entry in sdr.manifest["action_log"] if entry["action"].get("verb") != "capture"]
        self.assertEqual(log[-4:], [("stop", "admitted"), ("set_parameter", "admitted"), ("set_parameter", "rejected"), ("vocabulary", "admitted")], "start_rx only after the corrected start")

    def test_ea_16_the_channel_setters_send_one_update_on_a_stopped_stream(self) -> None:
        with self.retune_session() as sdr:
            sdr.rx.stop()
            sdr.rx_channels[0].frequency = 2.3e9
            sdr.rx_channels[0].gain = 6.0
            sdr.rx_channels[0].antenna = "RX2"  # TX/RX would meet RM-31's duplex rule beside the transmit stream
            self.assertEqual((sdr.rx_channels[0].frequency, sdr.rx_channels[0].gain, sdr.rx_channels[0].antenna), (2.3e9, 6.0, "RX2"))
        self.assertEqual([entry["action"]["key"] for entry in sdr.manifest["action_log"][-3:]], ["radio.rx.c0.frequency_hz", "radio.rx.c0.gain_db", "radio.rx.c0.antenna"])

    def test_ea_16_a_rate_cannot_change_before_the_bench_confirms_it(self) -> None:
        with self.retune_session() as sdr:
            sdr.rx.stop()
            sent = len(sdr.manifest["action_log"]) if sdr.manifest else 0
            for call in (lambda: sdr.rx.start(sample_rate=1e6), lambda: setattr(sdr.rx, "sample_rate", 1e6)):
                with self.assertRaises(ezsdr.Error) as refused:
                    call()
                self.assertNotIsInstance(refused.exception, ezsdr.Rejected)
                self.assertIn("held-open streamer", str(refused.exception))
            with self.assertRaises(ezsdr.Rejected) as raw:
                sdr.set("radio", "radio.rx.s0.sample_rate_hz", 1e6)
            self.assertTrue(any(v["check"] == "ezsdr.update_class" for v in raw.exception.violations))
        self.assertEqual([entry["action"]["kind"] for entry in sdr.manifest["action_log"]], ["stop", "set_parameter"], "the client refusals sent nothing")

    def test_ea_16_start_inside_aligned_gives_the_start_the_block_s_instant_and_the_values_none(self) -> None:
        with self.retune_session() as sdr:
            sdr.rx.stop()
            at = sdr.after(0.2)
            with sdr.aligned(at):
                sdr.rx.start(frequency={0: 2.45e9})
        log = sdr.manifest["action_log"]
        self.assertIsNone(log[-2]["action"].get("at"), "a staged value takes no instant (RM-21)")
        self.assertEqual(log[-1]["action"]["at"], at)

    def test_ea_16_a_scheduled_receive_value_is_refused_by_the_provider(self) -> None:
        # Values change through Session actions only: a Spec's scheduled one carries an instant, which RM-21 refuses at run time.
        with self.retune_session() as sdr:
            entry, refused = sdr._submit({"kind": "set_parameter", "target": ezsdr.Target.resource("radio"), "key": "radio.rx.c0.frequency_hz", "value": 2.1e9})
            self.assertEqual(entry["outcome"]["kind"], "admitted")
            self.assertEqual(len(refused), 1)


class ConnectRequests(unittest.TestCase):
    """What ``connect`` sends and what it refuses before sending anything, over a mocked
    connection (EA-10, EA-16)."""

    ROOT = {"node": 0, "local": 1}

    def frame(self, **kwargs: object) -> dict:
        """The request ``connect`` makes for ``kwargs``, with the connection mocked."""
        at = {"domain": self.ROOT, "ticks": 0}
        connected = {"run": "test", "dir": "", "profile": {"bindings": {}}, "start_instant": at, "now": at, "root_rate": {"num": 1, "den": 1}}
        with unittest.mock.patch("ezsdr.session.Connection") as connection:
            connection.return_value.call.return_value = (connected, b"")
            connection.return_value.abandoned = False
            ezsdr.connect(**kwargs)
            connection.return_value.call.assert_called_once()
            return connection.return_value.call.call_args.args[0]

    def refused(self, **kwargs: object) -> str:
        with unittest.mock.patch("ezsdr.session.Connection") as connection:
            with self.assertRaises(ezsdr.Error) as raised:
                ezsdr.connect(**kwargs)
            connection.assert_not_called()  # not even the server was started
        return str(raised.exception)

    def test_ea_16_connect_sends_nothing_without_streams(self) -> None:
        self.assertEqual(self.frame(), {"op": "connect"})

    def test_ea_16_connect_sends_every_direction_it_was_given(self) -> None:
        # `{}` is sent as an empty list, not dropped: a direction that was written is a direction.
        self.assertEqual(self.frame(rx={})["initial"], {"radio": {"rx": [], "tx": []}})
        self.assertEqual(self.frame(tx={})["initial"], {"radio": {"rx": [], "tx": []}})
        stream = ezsdr.Stream([0, 1], sample_rate=20e6, frequency={0: 2.45e9, 1: 2.46e9}, gain={0: 10}, antenna={1: "RX2"})
        frame = self.frame(rx={0: stream, "aux": [2]}, tx={1: [0]})
        self.assertEqual(frame["initial"]["radio"]["rx"], [
            {"number": 0, "channels": [0, 1], "sample_rate": 20e6, "frequency": {"0": 2.45e9, "1": 2.46e9}, "gain": {"0": 10.0}, "antenna": {"1": "RX2"}},
            {"name": "aux", "channels": [2]},
        ])
        self.assertEqual(frame["initial"]["radio"]["tx"], [{"number": 1, "channels": [0]}])

    def test_ea_16_a_name_is_the_same_rule_as_the_servers(self) -> None:
        # Full-width digits are not the digits 0-9, so `"１２"` is a name (the server's rule, EA-10a).
        self.assertEqual(self.frame(rx={"\uff11\uff12": [0]})["initial"]["radio"]["rx"], [{"name": "\uff11\uff12", "channels": [0]}])

    def test_ea_16_connect_radio_names_the_target(self) -> None:
        self.assertEqual(list(self.frame(rx={0: [0]}, radio="second")["initial"]), ["second"])
        self.assertEqual(list(self.frame(rx={0: [0]})["initial"]), ["radio"])

    def test_ea_16_connect_refuses_a_bad_map_before_any_request(self) -> None:
        cases = {
            "a channel in two streams": ({"rx": {0: [0], 1: [0]}}, "c0 is in two places"),
            "a channel twice": ({"rx": {0: [1, 1]}}, "c1 is in two places"),
            "a bool channel": ({"rx": {0: [True]}}, "non-negative integer"),
            "a float channel": ({"rx": {0: [0.0]}}, "non-negative integer"),
            "a negative channel": ({"tx": {0: [-1]}}, "non-negative integer"),
            "a channel name that is no name": ({"rx": {0: ["a b"]}}, "'a b' is not a channel name"),
            "a digit-only channel name": ({"rx": {0: ["12"]}}, "'12' is not a channel name"),
            "a channel name twice": ({"rx": {0: ["loop-rx", "loop-rx"]}}, "'loop-rx' is in two places"),
            "a channel name in two streams": ({"rx": {0: ["loop-rx"], 1: ["loop-rx"]}}, "'loop-rx' is in two places"),
            "a negative stream": ({"rx": {-1: [0]}}, "non-negative integer"),
            "a bool stream": ({"rx": {True: [0]}}, "non-negative integer"),
            "an empty name": ({"rx": {"": [0]}}, "is not a stream name"),
            "a digit-only name": ({"rx": {"3": [0]}}, "is not a stream name"),
            "a name with a colon": ({"rx": {"a:b": [0]}}, "is not a stream name"),
            "a name with a hash": ({"rx": {"a#b": [0]}}, "is not a stream name"),
            "a name with a space": ({"rx": {"a b": [0]}}, "is not a stream name"),
            "a name with a tab": ({"rx": {"a\tb": [0]}}, "is not a stream name"),
            "a name with a file separator": ({"rx": {"\x1c": [0]}}, "is not a stream name"),
            "a name of an ideographic space": ({"rx": {"\u3000": [0]}}, "is not a stream name"),
            "a name with a next-line control": ({"rx": {"a\x85": [0]}}, "is not a stream name"),
            "a name with DEL": ({"rx": {"a\x7f": [0]}}, "is not a stream name"),
            "a name of the ten digits": ({"rx": {"0123456789": [0]}}, "is not a stream name"),
            "a radio that is empty": ({"rx": {0: [0]}, "radio": ""}, "non-empty string"),
            "a radio that is not text": ({"rx": {0: [0]}, "radio": 3}, "non-empty string"),
            "a radio with no stream to set up": ({"radio": "second"}, "none was given"),
            "one name in both directions": ({"rx": {"main": [0]}, "tx": {"main": [0]}}, "used twice"),
            "a stream that is a number": ({"rx": {0: 3}}, "a list"),
            "channels that are a string": ({"rx": {0: "01"}}, "a list"),
            "a direction that is a list": ({"rx": [0]}, "dict"),
            "a value outside the stream": ({"rx": {0: ezsdr.Stream([0], gain={1: 5})}}, "not a channel of rx stream 0"),
            "a value keyed by a name no channel of the stream is": ({"rx": {0: ezsdr.Stream(["x"], gain={"y": 5})}}, "names 'y', which is not a channel of rx stream 0"),
            "a value keyed by a bad name": ({"rx": {0: ezsdr.Stream(["x"], gain={"a b": 5})}}, "'a b' is not a channel name"),
            "a bool value key": ({"rx": {0: ezsdr.Stream(["x"], gain={True: 5})}}, "non-negative integer"),
            "a bool frequency": ({"rx": {0: ezsdr.Stream([0], frequency={0: True})}}, "finite number"),
            "a NaN rate": ({"rx": {0: ezsdr.Stream([0], sample_rate=float("nan"))}}, "finite number"),
            "an infinite gain": ({"tx": {0: ezsdr.Stream([0], gain={0: float("inf")})}}, "finite number"),
            "a string frequency": ({"rx": {0: ezsdr.Stream([0], frequency={0: "2.4e9"})}}, "finite number"),
            "an antenna that is not text": ({"rx": {0: ezsdr.Stream([0], antenna={0: 1})}}, "must be a string"),
            "frequencies that are a list": ({"rx": {0: ezsdr.Stream([0], frequency=[1.0])}}, "dict keyed by channel"),
        }
        for what, (streams, message) in cases.items():
            with self.subTest(what):
                self.assertIn(message, self.refused(**streams))

    def test_ea_16_connect_takes_channel_names(self) -> None:
        # A `str` is a name and an `int` a number, anywhere a channel is written (RM-28, EA-16).
        stream = ezsdr.Stream([0, "loop-rx"], gain={"loop-rx": 5, 0: 6}, antenna={"loop-rx": "RX2"})
        frame = self.frame(rx={"main": stream}, tx={0: ["loop-tx"]})
        self.assertEqual(frame["initial"]["radio"]["rx"], [{"name": "main", "channels": [0, "loop-rx"], "gain": {"loop-rx": 5.0, "0": 6.0}, "antenna": {"loop-rx": "RX2"}}])
        self.assertEqual(frame["initial"]["radio"]["tx"], [{"number": 0, "channels": ["loop-tx"]}])

    def test_ea_16_connect_leaves_a_mixed_reference_to_the_server(self) -> None:
        # Whether a number and a name are one channel only the selector says, so neither is refused here.
        for stream in (ezsdr.Stream([0], gain={"loop-rx": 5}), ezsdr.Stream(["loop-rx"], gain={0: 5}), ezsdr.Stream([0, "loop-rx"], gain={"loop-rx": 5, 0: 6})):
            with self.subTest(stream=stream):
                self.assertEqual(len(self.frame(rx={0: stream})["initial"]["radio"]["rx"]), 1)

    def test_ea_16_the_name_rule_matches_the_server(self) -> None:
        # One case list for the Rust parser, the server and this client (crates/ezsdr-radio/tests/fixtures/names.json).
        cases = json.loads((Path(__file__).resolve().parents[2] / "crates/ezsdr-radio/tests/fixtures/names.json").read_text())
        for name in cases["good"]:
            with self.subTest(good=name):
                self.assertEqual(self.frame(rx={0: [name]})["initial"]["radio"]["rx"][0]["channels"], [name])
                self.assertEqual(self.frame(rx={name: [0]})["initial"]["radio"]["rx"][0]["name"], name)
        for name in cases["bad"]:
            with self.subTest(bad=name):
                self.assertIn("is not a channel name", self.refused(rx={0: [name]}))
                self.assertIn("is not a stream name", self.refused(rx={name: [0]}))

    def test_ea_16_a_channel_is_reached_by_its_name(self) -> None:
        at = {"domain": self.ROOT, "ticks": 0}
        session = ezsdr.Session(Mock(), {
            "run": "test", "dir": "", "profile": {"bindings": {}}, "start_instant": at, "now": at, "root_rate": {"num": 1, "den": 1},
            "channels": {"radio": {"rx": {"loop-rx": 0, "late": 2}, "tx": {"loop-tx": 1}}, "second": {"rx": {}, "tx": {}}},
        })
        effective = {f"radio.{d}.c{n}.stream": -1 for d in ("rx", "tx") for n in range(3)}
        session._call = Mock(return_value=({"effective": {"radio": effective, "second": {"radio.rx.c0.stream": -1}}}, b""))
        radio = session.radio("radio")
        self.assertEqual((radio.rx_channels["loop-rx"]._number, radio.rx_channels["late"]._number, radio.tx_channels["loop-tx"]._number), (0, 2, 1))
        self.assertEqual((radio.rx_channels["loop-rx"]._direction, radio.tx_channels["loop-tx"]._direction), ("rx", "tx"))
        self.assertEqual((radio.rx_channels[1]._number, len(radio.rx_channels), [c._number for c in radio.tx_channels]), (1, 3, [0, 1, 2]), "numbers, length and iteration are the list's")
        with self.assertRaises(KeyError) as raised:
            radio.rx_channels["nowhere"]
        self.assertIn("['late', 'loop-rx']", str(raised.exception))
        with self.assertRaises(KeyError):
            radio.rx_channels["loop-tx"]  # a transmit name is no receive channel
        with self.assertRaises(KeyError):
            radio.tx_channels["loop-rx"]
        self.assertEqual(session.radio("second").rx_channels._names, {}, "a radio that names nothing is reached by number only")

    def test_ea_16_a_name_is_looked_up_in_its_own_direction(self) -> None:
        session = ezsdr.Session(Mock(), {
            "run": "test", "dir": "", "profile": {"bindings": {}}, "start_instant": {"domain": self.ROOT, "ticks": 0}, "now": {"domain": self.ROOT, "ticks": 0},
            "root_rate": {"num": 1, "den": 1}, "streams": {"radio": {"rx": {"main": 1}, "tx": {"main": 0}}},
        })
        session._call = Mock(return_value=({"effective": {"radio": {
            "radio.rx.c0.stream": 0, "radio.rx.s0.sample_rate_hz": 1e6, "radio.rx.s1.sample_rate_hz": 1e6,
            "radio.tx.c0.stream": 0, "radio.tx.s0.sample_rate_hz": 1e6, "radio.tx.s1.sample_rate_hz": 1e6,
        }}}, b""))
        self.assertEqual((session.radio("radio").rx_streams["main"]._number, session.radio("radio").tx_streams["main"]._number), (1, 0))


class DurationRequests(unittest.TestCase):
    """Seconds as the package reads them, and the requests they become, over a mocked
    connection (EA-16; spec 20, VF-2)."""

    ROOT = {"node": 0, "local": 1}

    def session(self, reply: dict, rate: tuple = (1_000_000_000, 1), epoch: dict = None) -> ezsdr.Session:
        connection = Mock()
        connection.abandoned = False
        connection.call.return_value = (reply, b"")
        fed = {"feed": {"port": {"component": "radio", "port": "rx0"}, "policy": "drop_oldest", "capacity": 64}}
        at = {"domain": self.ROOT, "ticks": 0}
        connected = {
            "run": "test", "dir": "", "profile": {"bindings": {"radio": {}, "rec": fed}},
            "start_instant": at, "now": at, "root_rate": {"num": rate[0], "den": rate[1]},
        }
        if epoch is not None:
            connected["root_epoch"] = epoch
        return ezsdr.Session(connection, connected)

    def callers(self, sdr: ezsdr.Session, seconds: object) -> dict:
        handle = ezsdr.session.CaptureRequest("rec", 0, 1, {"outcome": {"kind": "admitted", "dispatched": []}})
        rx = ezsdr.session.Rx(sdr, "radio", 0)
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
        other = {"stream": {"node": 0, "path": "radio/rx/s0"}, "root_ticks_per_tick": {"num": 100, "den": 1},
                 "origin": {"domain": self.ROOT, "ticks": 0}, "ended_at": None}
        clock = {"stream": {"node": 0, "path": "radio/rx/s1"}, "root_ticks_per_tick": {"num": 1000, "den": 3},
                 "origin": {"domain": self.ROOT, "ticks": 7}, "ended_at": None}
        sdr = self.session({"now": {"domain": self.ROOT, "ticks": 10_000}, "sample_clocks": [clock, other]})
        rx = ezsdr.session.Rx(sdr, "radio", 1)
        at = rx.next_at({"domain": self.ROOT, "ticks": 100}, 10)
        self.assertEqual(at, {"domain": self.ROOT, "ticks": 50_010_341})
        clock["origin"]["ticks"] = 200
        with self.assertRaises(ezsdr.Error):
            rx.next_at({"domain": self.ROOT, "ticks": 100}, 10)

    def test_ea_16_collections_follow_effective_keys_without_selector(self) -> None:
        effective = {
            "radio.rx.c0.stream": 0,
            "radio.rx.c1.stream": 1,
            "radio.rx.s0.sample_rate_hz": 1e6,
            "radio.rx.s1.sample_rate_hz": 2e6,
            "radio.tx.c0.stream": 0,
            "radio.tx.s0.sample_rate_hz": 1e6,
        }
        sdr = self.session({"effective": {"radio": effective}})
        sdr.profile["bindings"]["radio"]["selector"] = {}
        radio = sdr.radio("radio")
        self.assertEqual((len(radio.rx_channels), len(radio.rx_streams), len(radio.tx_channels), len(radio.tx_streams)), (2, 2, 1, 1))
        self.assertIs(radio.tx, radio.tx_streams[0])
        sdr._connection.call.assert_called_once_with({"op": "status"}, b"")
        sdr._connection.reset_mock()
        self.assertEqual(radio.rx_streams[0].channels, [0])
        sdr._connection.call.assert_called_once_with({"op": "status"}, b"")

    def test_ea_16_missing_layout_key_raises_error(self) -> None:
        sdr = self.session({"effective": {"radio": {"radio.full_duplex": True}}})
        with self.assertRaisesRegex(ezsdr.Error, "EA-16: radio has no key radio.rx.s0.sample_rate_hz"):
            ezsdr.session.Rx(sdr, "radio", 0).sample_rate

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
        # The first answer is another requester's, a Reactor's say (action 4): it is skipped,
        # not counted, and the wait goes on for this request's own (HD-16).
        sdr = self.session({})
        horizon = {"domain": {"node": 0, "path": "clock"}, "ticks": 12}
        sdr._connection.call.side_effect = [
            ({"index": 3, "horizon": horizon, "event": {"source": {"kind": "output", "output": "rec"}, "payload": {"action": 4}}}, b""),
            ({"index": None}, b""),
        ]
        handle = ezsdr.session.CaptureRequest("rec", 1, 2, {"outcome": {"kind": "admitted", "dispatched": [5]}})
        with self.assertRaises(ezsdr.CaptureTimeout):
            ezsdr.session.Rx(sdr, "radio", 0).result(handle, 1e-12)
        requests = [call.args[0] for call in sdr._connection.call.call_args_list]
        self.assertEqual(requests, [
            {"op": "wait_for", "kinds": [ezsdr.session.CAPTURE_WRITTEN, ezsdr.session.REQUEST_REJECTED], "from": 2, "within": {"domain": self.ROOT, "ticks": 1}},
            {"op": "wait_for", "kinds": [ezsdr.session.CAPTURE_WRITTEN, ezsdr.session.REQUEST_REJECTED], "from": 4, "until": horizon},
        ])


class StartValues(unittest.TestCase):
    """`Rx.start`'s values over a mocked connection (EA-16): what is checked before the first request, and what is sent in which order."""

    AT = {"domain": {"node": 0, "local": 1}, "ticks": 0}

    def session(self, refused_on: int = 0) -> tuple:
        """A Session over a connection that answers `status` with a radio holding rx channels 0 and 1 in stream 0, channel 2 in stream 1, a
        named channel `loop-rx` (number 1), and every other request with an admitted entry; the `refused_on`-th submit (1-based) also
        returns a Provider refusal."""
        sent: list = []
        effective = {
            "radio.rx.c0.stream": 0, "radio.rx.c1.stream": 0, "radio.rx.c2.stream": 1, "radio.rx.s0.sample_rate_hz": 1e6, "radio.rx.s1.sample_rate_hz": 1e6,
            "radio.tx.c0.stream": 0, "radio.tx.s0.sample_rate_hz": 1e6,
        }

        def call(request: dict, body: bytes = b"") -> tuple:
            sent.append(request)
            if request["op"] == "status":
                return ({"now": self.AT, "effective": {"radio": effective}, "events": 0, "sample_clocks": []}, b"")
            submits = [r for r in sent if r["op"] == "submit"]
            event = {"kind": "radio.COMMAND_REJECTED", "payload": {"action": len(submits), "kind": "update_parameter", "reason": "streaming"}}
            refused = [event] if len(submits) == refused_on else []
            return ({"entry": {"outcome": {"kind": "admitted", "dispatched": [len(submits)]}, "action": request["action"]}, "refused": refused, "now": self.AT, "events": 0}, b"")

        connection = Mock()
        connection.call.side_effect = call
        sdr = ezsdr.Session(connection, {"run": "t", "dir": "", "profile": {"bindings": {}}, "start_instant": self.AT, "now": self.AT, "root_rate": {"num": 1_000_000_000, "den": 1},
                                         "channels": {"radio": {"rx": {"loop-rx": 1}, "tx": {"loop-tx": 0}}}})
        return sdr, sent

    def submits(self, sent: list) -> list:
        return [r["action"] for r in sent if r["op"] == "submit"]

    def test_ea_16_start_checks_everything_before_sending(self) -> None:
        bad = {
            "a channel in another stream": {"frequency": {0: 1e9, 2: 1e9}},
            "a channel the radio does not have": {"frequency": {0: 1e9, 9: 1e9}},
            "a name of another direction": {"frequency": {0: 1e9, "loop-tx": 1e9}},
            "one channel twice": {"gain": {1: 3.0, "loop-rx": 4.0}},
            "a string frequency": {"frequency": {0: 1e9, 1: "2.4e9"}},
            "a non-finite gain": {"gain": {0: 1.0, 1: float("nan")}},
            "a bool frequency": {"frequency": {0: True}},
            "a number as antenna": {"antenna": {0: "RX2", 1: 5}},
            "a list for frequency": {"frequency": [1e9]},
            "a bool channel": {"frequency": {True: 1e9}},
            # `channels[-2]` is channel 1 by list position: a negative number must not reach another channel.
            "a negative channel number": {"frequency": {0: 1e9, -2: 2e9}},
            "a float channel": {"frequency": {0: 1e9, 1.0: 2e9}},
            "a None channel": {"frequency": {0: 1e9, None: 2e9}},
        }
        for what, values in bad.items():
            with self.subTest(what):
                sdr, sent = self.session()
                with self.assertRaises(ezsdr.Error) as raised:
                    ezsdr.session.Rx(sdr, "radio", 0).start(**values)
                self.assertNotIsInstance(raised.exception, ezsdr.Rejected)
                self.assertEqual(self.submits(sent), [], "nothing was sent, not even the first valid value")
        with self.assertRaises(ezsdr.Error) as raised:
            sdr, sent = self.session()
            ezsdr.session.Rx(sdr, "radio", 0).start(sample_rate=1e6)
        self.assertIn("held-open streamer", str(raised.exception))
        self.assertEqual(self.submits(sent), [])

    def test_ea_16_start_takes_values_keyed_by_channel_number_or_name_and_sends_them_in_order(self) -> None:
        sdr, sent = self.session()
        ezsdr.session.Rx(sdr, "radio", 0).start(frequency={1: 2.0e9, 0: 1.0e9}, gain={"loop-rx": 5.0}, antenna={0: "RX2"})
        actions = self.submits(sent)
        self.assertEqual([(a["key"], a["value"]) for a in actions[:-1]], [
            ("radio.rx.c0.frequency_hz", 1.0e9), ("radio.rx.c1.frequency_hz", 2.0e9), ("radio.rx.c1.gain_db", 5.0), ("radio.rx.c0.antenna", "RX2"),
        ], "frequencies, then gains, then antennas, each by channel number; a name is its channel")
        self.assertEqual((actions[-1]["kind"], actions[-1]["verb"]), ("vocabulary", "start_rx"))
        self.assertTrue(all(a["target"] == ezsdr.Target.resource("radio") for a in actions[:-1]))
        self.assertTrue(all("at" not in a for a in actions[:-1]), "a value carries no instant")

    def test_ea_16_start_stops_at_the_first_rejected_call(self) -> None:
        sdr, sent = self.session(refused_on=2)
        with self.assertRaises(ezsdr.Rejected) as raised:
            ezsdr.session.Rx(sdr, "radio", 0).start(frequency={0: 1.0e9, 1: 2.0e9}, gain={0: 1.0})
        self.assertEqual(raised.exception.event["kind"], "radio.COMMAND_REJECTED")
        self.assertEqual(raised.exception.event["payload"]["action"], 2)
        self.assertEqual([a["key"] for a in self.submits(sent)], ["radio.rx.c0.frequency_hz", "radio.rx.c1.frequency_hz"], "no gain and no start_rx after the refusal")

    def test_ea_16_switch_sends_a_marked_repeat(self) -> None:
        sdr, sent = self.session()
        ezsdr.session.Tx(sdr, "radio", 0).switch(np.ones(100, dtype=np.complex64))
        (action,) = self.submits(sent)
        self.assertEqual((action["kind"], action["ns"], action["verb"]), ("vocabulary", "radio", "start_repeat"))
        self.assertEqual(action["target"], ezsdr.Target.resource("radio", "tx/s0"))
        self.assertEqual(action["params"], {"radio.repeat_operation": "switch"}, "the mark, and no value")

    def test_ea_16_switch_refuses_non_finite_and_wrong_channels_before_sending(self) -> None:
        for what, x in {
            "a NaN sample": np.array([1.0, np.nan], dtype=np.complex64),
            "an infinite sample": np.array([1.0, np.inf], dtype=np.complex64),
            "two channels for one channel": np.ones((2, 100), dtype=np.complex64),
        }.items():
            with self.subTest(what):
                sdr, sent = self.session()
                with self.assertRaises(ezsdr.Error) as raised:
                    ezsdr.session.Tx(sdr, "radio", 0).switch(x)
                self.assertNotIsInstance(raised.exception, ezsdr.Rejected)
                self.assertEqual(self.submits(sent), [], "nothing was sent")

    def test_ea_16_a_switch_the_provider_refuses_raises_rejected(self) -> None:
        # F34 (EA-16, RM-37): a switch the Provider refuses at booking raises `Rejected` with its event, as a repeat does.
        sdr, sent = self.session(refused_on=1)
        with self.assertRaises(ezsdr.Rejected) as raised:
            ezsdr.session.Tx(sdr, "radio", 0).switch(np.ones(100, dtype=np.complex64))
        self.assertEqual(raised.exception.event["kind"], "radio.COMMAND_REJECTED")
        self.assertEqual([a["verb"] for a in self.submits(sent)], ["start_repeat"])

    def test_ea_16_second_plain_repeat_on_a_playing_stream_raises_rejected(self) -> None:
        sdr, sent = self.session(refused_on=2)
        tx = ezsdr.session.Tx(sdr, "radio", 0)
        tx.repeat(np.ones(100, dtype=np.complex64))
        with self.assertRaises(ezsdr.Rejected) as raised:
            tx.repeat(np.ones(100, dtype=np.complex64))
        self.assertEqual(raised.exception.event["kind"], "radio.COMMAND_REJECTED")
        self.assertEqual([a["verb"] for a in self.submits(sent)], ["start_repeat", "start_repeat"])

    def test_ea_16_a_provider_refusal_raises_rejected_with_its_event_and_the_entry_is_the_admitted_one(self) -> None:
        sdr, _ = self.session(refused_on=1)
        with self.assertRaises(ezsdr.Rejected) as raised:
            sdr.set("radio", "radio.rx.c0.frequency_hz", 1.0e9)
        self.assertEqual(raised.exception.entry["outcome"]["kind"], "admitted")
        self.assertEqual(raised.exception.event["payload"]["reason"], "streaming")
        self.assertIn("streaming", str(raised.exception))
        sdr, _ = self.session(refused_on=1)
        self.assertEqual(sdr.submit({"kind": "stop", "target": None})["outcome"]["kind"], "admitted", "a raw submit returns the entry and does not raise")

    def test_ea_16_a_run_that_ended_during_a_value_raises_run_ended(self) -> None:
        sdr, _ = self.session()
        sdr._connection.call.side_effect = ezsdr.RunEnded("ended", "the Run has ended", {"termination": {"kind": "stopped"}})
        with self.assertRaises(ezsdr.RunEnded):
            sdr.set("radio", "radio.rx.c0.frequency_hz", 1.0e9)


class Targets(unittest.TestCase):
    """The typed targets the wire carries (KC-23, EA-16; spec 27)."""

    def test_ea_16_set_and_stop_take_a_target_or_a_resource_string(self) -> None:
        self.assertEqual(ezsdr.Target.resource("radio", "rx/s0"), {"kind": "resource", "resource": "radio", "path": "rx/s0"})
        self.assertEqual(ezsdr.Target.output("rec"), {"kind": "output", "output": "rec"})
        self.assertEqual(ezsdr.Target.component("c1"), {"kind": "component", "component": "c1"})
        sdr = DurationRequests.session(DurationRequests(), {"entry": {"outcome": {"kind": "admitted"}}})
        sent = lambda: sdr._connection.call.call_args.args[0]["action"]["target"]
        # A string is always a resource target, never a role read from a prefix.
        for string, target in [("radio", ezsdr.Target.resource("radio")), ("radio/rx/s0", ezsdr.Target.resource("radio", "rx/s0")),
                               ("sink/rec", ezsdr.Target.resource("sink", "rec"))]:
            sdr.set(string, "radio.rx.c0.gain_db", 1.0)
            self.assertEqual(sent(), target)
            sdr.stop(string)
            self.assertEqual(sent(), target)
        sdr.stop(ezsdr.Target.output("rec"))
        self.assertEqual(sent(), {"kind": "output", "output": "rec"})
        sdr.stop()
        self.assertIsNone(sent())


class RecorderChoice(unittest.TestCase):
    """EA-17's recorder over a mocked connection, as ``DurationRequests`` builds one."""

    def test_ea_17_capture_needs_exactly_one_recorder(self) -> None:
        # EA-17 (spec 20, VF-1): exactly one binding fed from the radio's receive port, or
        # an Error before any request; the helper never chooses among recorders.
        def fed(component: str) -> dict:
            return {"feed": {"port": {"component": component, "port": "rx0"}, "policy": "drop_oldest", "capacity": 64}}
        several = "EA-17: 2 recorders are fed from radio's receive port rx0 ['rec_a', 'rec_b']; submit sink.capture naming one"
        none = "EA-17: no recorder is fed from radio's receive port rx0 in the Session's profile"
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
                rx = ezsdr.session.Rx(sdr, "radio", 0)
                if error is None:
                    self.assertEqual(rx.request(100).recorder, "rec_a")
                    action = connection.call.call_args.args[0]["action"]
                    self.assertEqual((action["ns"], action["verb"], action["target"]), ("sink", "capture", {"kind": "output", "output": "rec_a"}))
                else:
                    with self.assertRaises(ezsdr.Error) as raised:
                        rx.request(100)
                    self.assertEqual(str(raised.exception), error)
                    connection.call.assert_not_called()


class ChildCapture(unittest.TestCase):
    def test_run_result_capture_is_the_output_s_own_capture(self) -> None:
        # HD-10: the output's own capture has the output's id; a Reactor's request under
        # the same output (`capture_<n>`) is not it, so the call raises without reading.
        session = Mock()
        manifest = {"artifacts": {"rec": [{"id": "capture_7", "kind": "sink.capture"}]}}
        with self.assertRaises(ezsdr.Error):
            ezsdr.session.RunResult(session, {}, manifest, None).capture("rec")
        session.read.assert_not_called()


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

    def test_ea_17_a_channel_with_no_valid_stretch_is_refused(self) -> None:
        # EA-17, §23: a channel with no valid stretch is a channel gap; one array
        # would hide it.
        domain = {"node": 0, "local": 7}
        def t(k): return {"domain": domain, "ticks": k}
        data = np.arange(8, dtype=np.complex64).tobytes()
        artifact = {"id": "rec", "continuity": [{
            "domain": domain, "channels": 2, "first": t(0), "end": t(4),
            "valid": [[{"start": t(0), "len": 4}], []],
            "gaps": [], "channel_gaps": [],
        }]}
        with self.assertRaises(ezsdr.Error):
            ezsdr.samples(data, artifact)

    def test_ea_17_two_valid_stretches_on_a_channel_are_refused(self) -> None:
        # EA-17: a channel with more than one valid stretch holds a break between
        # them; one array would hide it.
        domain = {"node": 0, "local": 7}
        def t(k): return {"domain": domain, "ticks": k}
        data = np.arange(4, dtype=np.complex64).tobytes()
        artifact = {"id": "rec", "continuity": [{
            "domain": domain, "channels": 1, "first": t(0), "end": t(4),
            "valid": [[{"start": t(0), "len": 2}, {"start": t(2), "len": 2}]],
            "gaps": [], "channel_gaps": [],
        }]}
        with self.assertRaises(ezsdr.Error):
            ezsdr.samples(data, artifact)

    def test_ea_17_a_channel_count_change_is_refused(self) -> None:
        # EA-17, SC-30a: a capture whose continuity spans two channel counts would
        # read as one shape; the channel count of its maps must be one.
        domain = {"node": 0, "local": 7}
        def t(k): return {"domain": domain, "ticks": k}
        data = np.arange(6, dtype=np.complex64).tobytes()
        one = {"domain": domain, "channels": 1, "first": t(0), "end": t(2),
               "valid": [[{"start": t(0), "len": 2}]], "gaps": [], "channel_gaps": []}
        two = {"domain": domain, "channels": 2, "first": t(0), "end": t(2),
               "valid": [[{"start": t(0), "len": 2}], [{"start": t(0), "len": 2}]],
               "gaps": [], "channel_gaps": []}
        artifact = {"id": "rec", "continuity": [one, two]}
        with self.assertRaises(ezsdr.Error):
            ezsdr.samples(data, artifact)


class _Interrupting:
    """A pass-through to one of the server's pipes that raises ``KeyboardInterrupt`` once, at the
    first call of ``on`` (``readline``, ``read`` or ``write``; a ``write`` first passes half of
    its bytes on)."""

    def __init__(self, stream: object, on: str):
        self._stream, self._on, self._armed = stream, on, True

    @property
    def closed(self) -> bool:
        return self._stream.closed

    def close(self) -> None:
        self._stream.close()

    def flush(self) -> None:
        self._stream.flush()

    def readline(self) -> bytes:
        self._hit("readline")
        return self._stream.readline()

    def read(self, count: int) -> bytes:
        self._hit("read")
        return self._stream.read(count)

    def write(self, data: bytes) -> int:
        if self._armed and self._on == "write":
            self._armed = False
            self._stream.write(data[: len(data) // 2])
            raise KeyboardInterrupt
        return self._stream.write(data)

    def _hit(self, name: str) -> None:
        if self._armed and self._on == name:
            self._armed = False
            raise KeyboardInterrupt


class Departure(unittest.TestCase):
    """An exception while a reply is awaited is the client's departure: both pipes close, the
    Session ends, and ``close()`` sends no ``finish`` (EA-15, EA-16), against the real server."""

    def setUp(self) -> None:
        ezsdr._client.find_server()  # fails with the build instruction when absent
        self.runs = tempfile.mkdtemp(prefix="ezsdr-python-")
        # A hung mutant ends the process rather than the run (the guard is only the backstop).
        faulthandler.dump_traceback_later(60, exit=True)

    def tearDown(self) -> None:
        faulthandler.cancel_dump_traceback_later()
        shutil.rmtree(self.runs, ignore_errors=True)

    def connect(self) -> ezsdr.Session:
        return ezsdr.connect(runs_dir=self.runs, rx={0: [0]})

    def interrupt(self, sdr: ezsdr.Session, pipe: str, on: str) -> object:
        """Installs the interrupting wrapper on ``pipe`` and returns the real pipe."""
        process = sdr._connection._process
        real = getattr(process, pipe)
        setattr(process, pipe, _Interrupting(real, on))
        return real

    def departed(self, sdr: ezsdr.Session, stdin: object, stdout: object) -> dict:
        self.assertTrue(stdin.closed and stdout.closed, "both pipes are closed when the call ends")
        manifest = sdr.close()
        self.assertEqual(manifest["termination"]["reason"], {"kind": "stopped", "cause": {"kind": "client_disconnect"}})
        self.assertEqual(Path(sdr.manifest_path), Path(sdr.dir) / "manifest.json")
        return manifest

    def test_ea_16_an_interrupted_call_closes_both_pipes_and_ends_the_session(self) -> None:
        sdr = self.connect()
        stdin, stdout = sdr._connection._process.stdin, self.interrupt(sdr, "stdout", "readline")
        with self.assertRaises(KeyboardInterrupt):
            sdr.sleep(10**6)
        self.assertTrue(stdin.closed and stdout.closed, "both pipes are closed when the call ends")
        with self.assertRaises(ezsdr.ProtocolError):
            sdr.events()
        manifest = sdr.close()
        self.assertEqual(manifest["termination"]["reason"], {"kind": "stopped", "cause": {"kind": "client_disconnect"}})
        self.assertEqual(Path(sdr.manifest_path), Path(sdr.dir) / "manifest.json")
        self.assertEqual(sdr._connection._process.returncode, 0)
        self.assertEqual(sdr.close(), manifest, "close() is repeatable after an abandon")

    def test_ea_16_an_exception_while_the_request_is_written_abandons_too(self) -> None:
        sdr = self.connect()
        stdin, stdout = self.interrupt(sdr, "stdin", "write"), sdr._connection._process.stdout
        with self.assertRaises(KeyboardInterrupt):
            sdr.sleep(1)
        # The server saw half a header and then the end of its input.
        self.departed(sdr, stdin, stdout)

    def test_ea_16_an_interruption_while_the_reply_body_is_read_abandons(self) -> None:
        sdr = self.connect()
        sdr.rx.capture(100)
        written = [e for e in sdr.events() if e["kind"] == ezsdr.session.CAPTURE_WRITTEN]
        stdin, stdout = sdr._connection._process.stdin, self.interrupt(sdr, "stdout", "read")
        with self.assertRaises(KeyboardInterrupt):
            sdr.read(written[0]["payload"]["artifact"])
        self.departed(sdr, stdin, stdout)

    def test_ea_16_close_after_an_interrupt_keeps_no_manifest_that_is_elsewhere(self) -> None:
        sdr = self.connect()
        self.interrupt(sdr, "stdout", "readline")
        with self.assertRaises(KeyboardInterrupt):
            sdr.sleep(10**6)
        sdr.dir = os.path.join(self.runs, "on-another-host")  # the server's host holds its Manifest, not this one
        self.assertIsNone(sdr.close())
        self.assertIsNone(sdr.manifest_path)
        self.assertEqual(sdr._connection._process.returncode, 0)

    def test_ea_16_the_server_runs_in_a_session_of_its_own(self) -> None:
        with self.connect() as sdr:
            pid = sdr._connection._process.pid
            self.assertEqual(os.getsid(pid), pid, "the server leads a session of its own")
            self.assertNotEqual(os.getsid(pid), os.getsid(0))

    def test_ea_16_ctrl_c_to_the_client_group_does_not_kill_the_server(self) -> None:
        script = (
            "import ezsdr, sys\n"
            "with ezsdr.connect(runs_dir=sys.argv[1], rx={0: [0]}) as sdr:\n"
            "    print(sdr.dir, flush=True)\n"
            "    sdr.sleep(10**6)\n"
        )
        env = dict(os.environ, PYTHONPATH=str(PYTHON_DIR), PYTHONDONTWRITEBYTECODE="1")
        child = subprocess.Popen(
            [sys.executable, "-c", script, self.runs], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, start_new_session=True,
        )
        try:
            directory = child.stdout.readline().strip()
            self.assertTrue(directory, "the child connected")
            # A SIGINT that lands just before the child blocks in its read is deferred until that read returns
            # (CPython sets a flag and does not wake the syscall), so Ctrl-C is pressed again until the child ends,
            # as a user would. The server holds the child's stderr, so `communicate` returns after both have exited.
            for _ in range(15):
                os.killpg(child.pid, signal.SIGINT)
                try:
                    child.communicate(timeout=3)
                    break
                except subprocess.TimeoutExpired:
                    continue
            self.assertIsNotNone(child.returncode, "the child ended")
        finally:
            if child.poll() is None:
                child.kill()
                child.communicate()
        # `client` when the signal came before the advance was written: either way the server lived to write it.
        written = json.loads((Path(directory) / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(written["termination"]["reason"]["kind"], "stopped")
        self.assertIn(written["termination"]["reason"]["cause"]["kind"], ("client_disconnect", "client"))


class DepartureClose(unittest.TestCase):
    """What ``Session.close`` does after a call that ended while its reply was awaited, over a
    mocked connection (EA-16)."""

    ROOT = {"node": 0, "local": 1}
    DISCONNECTED = {"termination": {"reason": {"kind": "stopped", "cause": {"kind": "client_disconnect"}}}}

    def setUp(self) -> None:
        self.dir = tempfile.mkdtemp(prefix="ezsdr-python-")

    def tearDown(self) -> None:
        shutil.rmtree(self.dir, ignore_errors=True)

    def session(self, connection: object) -> ezsdr.Session:
        at = {"domain": self.ROOT, "ticks": 0}
        connected = {"run": "test", "dir": self.dir, "profile": {"bindings": {}}, "start_instant": at, "now": at, "root_rate": {"num": 1, "den": 1}}
        return ezsdr.Session(connection, connected)

    def writing(self, text: str) -> Mock:
        """A connection whose ``close`` (the wait for the server's exit) writes ``manifest.json``."""
        connection = Mock()
        connection.close.side_effect = lambda: (Path(self.dir) / "manifest.json").write_text(text, encoding="utf-8")
        return connection

    def test_ea_16_close_after_an_abandon_sends_no_finish_and_reads_after_the_exit(self) -> None:
        connection = self.writing(json.dumps(self.DISCONNECTED))
        connection.abandoned = True
        sdr = self.session(connection)
        self.assertEqual(sdr.close(), self.DISCONNECTED)
        connection.call.assert_not_called()
        connection.close.assert_called_once()

    def test_ea_16_close_reads_the_manifest_after_the_server_exited(self) -> None:
        connection = self.writing(json.dumps(self.DISCONNECTED))
        connection.abandoned = False
        connection.call.side_effect = ezsdr.ProtocolError("protocol", "the server has exited", {})
        sdr = self.session(connection)
        self.assertEqual(sdr.close(), self.DISCONNECTED)
        self.assertEqual(sdr.manifest_path, os.path.join(self.dir, "manifest.json"))

    def test_ea_16_close_after_an_abandon_leaves_no_manifest_when_none_is_readable(self) -> None:
        for written in (None, "{"):
            with self.subTest(written=written):
                connection = Mock() if written is None else self.writing(written)
                connection.abandoned = True
                sdr = self.session(connection)
                self.assertIsNone(sdr.close())
                self.assertIsNone(sdr.manifest_path)
                self.assertIsNone(sdr.close(), "and again")
                connection.call.assert_not_called()

    def test_ea_16_a_close_whose_own_finish_fails_raises_unless_a_manifest_was_written(self) -> None:
        connection = Mock()
        connection.abandoned = False
        connection.call.side_effect = ezsdr.ProtocolError("protocol", "the server has exited", {})
        with self.assertRaises(ezsdr.ProtocolError):
            self.session(connection).close()
        connection.close.assert_called_once()

    def test_ea_16_a_server_that_does_not_exit_is_killed_and_close_keeps_no_manifest(self) -> None:
        connection = object.__new__(ezsdr._client.Connection)
        connection.abandoned = True
        connection._process = Mock()
        connection._process.wait.side_effect = [subprocess.TimeoutExpired("ezsdr-server", 30), -9]
        sdr = self.session(connection)
        self.assertIsNone(sdr.close())
        connection._process.kill.assert_called_once_with()
        self.assertEqual(connection._process.wait.call_args_list, [unittest.mock.call(timeout=30), unittest.mock.call()])
        connection._process.stdout.close.assert_called_once_with()


if __name__ == "__main__":
    unittest.main()
