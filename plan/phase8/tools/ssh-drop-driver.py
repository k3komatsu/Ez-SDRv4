"""Transmits a tone through the lab server until ssh-drop-tx.sh drops the link.

The profile is the bench binding of the Rust test helpers (crates/ezsdr-radio-uhd/tests/common/mod.rs, bench_profile):
UHD Module 0.4.0, profile x3xx 0.1.0, slot A's board (EZSDR_DROP_BOARD) in both directions, the rx0 feed into the
capture Sink, and the bench's RF envelope (1 GHz ± 1 MHz). The Session takes the server's default Lease, Attached.

The script sets EZSDR_SERVER to its ssh wrapper and EZSDR_DROP_OUT to its output directory; the markers it waits for
are written there: tx-started (with the Session's directory in session-dir), wait-start (time.time_ns() just before the
wait) and driver-done.

EZSDR_DROP_WAIT is how the Session waits for the drop: `time` (the default) is time.sleep, so the server is idle between
requests when the link drops; `sleep` is sdr.sleep, so the server is inside one `advance` request (#89).

    ssh-drop-driver.py --self-check     the parsing of EZSDR_DROP_WAIT, with no server and no device
"""

import os
import sys
import time


def wait_mode(value):
    """EZSDR_DROP_WAIT: `time` when unset or empty, else `time` or `sleep`; anything else stops the run before a transmission."""
    mode = value or "time"
    if mode not in ("time", "sleep"):
        raise SystemExit(f"ssh-drop-driver: EZSDR_DROP_WAIT must be time or sleep, not {value!r}")
    return mode


if sys.argv[1:] == ["--self-check"]:
    assert [wait_mode(v) for v in (None, "", "time", "sleep")] == ["time", "time", "time", "sleep"]
    for bad in ("Sleep", "sleep ", "1", "advance"):
        try:
            wait_mode(bad)
        except SystemExit:
            continue
        raise AssertionError(f"EZSDR_DROP_WAIT={bad!r} was accepted")
    print("ssh-drop-driver self-check passed: time (unset, empty, time) and sleep; any other value refused")
    sys.exit(0)

mode = wait_mode(os.environ.get("EZSDR_DROP_WAIT"))

import numpy as np  # noqa: E402

import ezsdr  # noqa: E402

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
    sdr = ezsdr.connect(profile=profile, runs_dir="/bench/runs", tx={0: ezsdr.Stream([0], frequency={0: 1e9})})
    sdr.tx.repeat(wave)
    note("session-dir", sdr.dir)
    note("tx-started", "ok")
    note("wait-start", str(time.time_ns()))
    if mode == "sleep":
        sdr.sleep(hold)
    else:
        time.sleep(hold)
    sdr.close()
    note("driver-done", "closed")
except Exception as error:
    note("driver-done", f"{type(error).__name__}: {error}")
