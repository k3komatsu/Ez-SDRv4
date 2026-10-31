"""Transmits a tone through the lab server until ssh-drop-tx.sh drops the link.

The profile is the bench binding of the Rust test helpers (crates/ezsdr-radio-uhd/tests/common/mod.rs, bench_profile):
UHD Module 0.4.0, profile x3xx 0.1.0, slot A's board (EZSDR_DROP_BOARD) in both directions, the rx0 feed into the
capture Sink, and the bench's RF envelope (1 GHz ± 1 MHz). The Session takes the server's default Lease, Attached.

The script sets EZSDR_SERVER to its ssh wrapper and EZSDR_DROP_OUT to its output directory; the markers it waits for
are written there: tx-started (with the Session's directory in session-dir) and driver-done.
"""

import os
import time

import numpy as np

import ezsdr

out = os.environ["EZSDR_DROP_OUT"]
hold = float(os.environ.get("EZSDR_DROP_HOLD", "120"))
args = os.environ["EZSDR_UHD_ARGS"]
slot = [f"0:A:{os.environ['EZSDR_DROP_BOARD']}:0"]
version = {"major": 0, "minor": 4, "patch": 0}


def note(name, text=""):
    with open(os.path.join(out, name), "w", encoding="utf-8") as file:
        file.write(text + "\n")


profile = {
    "version": 1,
    "bindings": {
        "radio": {
            "module": {"id": "ezsdr.radio.uhd", "version": version},
            "profile": {"name": "x3xx", "version": {"major": 0, "minor": 1, "patch": 0}},
            "selector": {"args": args, "id": "usrp", "rx": slot, "tx": slot, "link_gbps": [10]},
        },
        "rec": {
            "module": {"id": "ezsdr.sink.capture", "version": {"major": 1, "minor": 2, "patch": 0}},
            "selector": {"dir": "/bench/capture"},
            "feed": {"port": {"component": "radio", "port": "rx0"}, "policy": "drop_oldest", "capacity": 64},
        },
    },
    "authority": "radio",
    "placements": {
        "links": [{
            "link": {"id": "ezsdr.link.host", "version": {"major": 1, "minor": 0, "patch": 0}},
            "from": {"component": "radio", "port": "rx0"},
            "to": {"component": "rec", "port": "in"},
        }]
    },
    "environment": {
        "ezsdr.time": {"class": "hardware_in_loop", "start_lead_ns": 2000000000},
        "ezsdr.rf_path": {"path": "cabled"},
        "radio.rf_envelope": {"allowed_bands": [{"lo_hz": 999e6, "hi_hz": 1001e6}], "max_gain_db": 0.0, "tx_enabled": [True]},
    },
}

# 100 kHz at 1 MS/s, amplitude 0.4 (below the bench's 0.5 bound), on the profile's 1 GHz.
t = np.arange(1000)
wave = (0.4 * np.exp(2j * np.pi * 100e3 * t / 1e6)).astype(np.complex64)

try:
    sdr = ezsdr.connect(profile=profile, runs_dir="/bench/runs")
    sdr.set("radio", "radio.tx.c0.frequency_hz", 1e9)
    sdr.tx_channels[0].stream = 0
    sdr.tx.repeat(wave)
    note("session-dir", sdr.dir)
    note("tx-started", "ok")
    time.sleep(hold)
    sdr.close()
    note("driver-done", "closed")
except Exception as error:
    note("driver-done", f"{type(error).__name__}: {error}")
