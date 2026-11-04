"""Bench step B7 (plan/phase7/bench.md): a Session on a USRP through the Easy API.

    EZSDR_SERVER=target/release/ezsdr-server EZSDR_PROFILE=bench-session.json \
        python3 python/examples/bench_loopback.py

The server must be built with ``--features uhd`` and ``EZSDR_PROFILE`` must name the
bench's Session profile (VE-5); without them the connect is refused.
"""

import numpy as np

import ezsdr

N = 20_000
rng = np.random.default_rng(0)
x = (0.4 * (np.sign(rng.standard_normal(1000)) + 1j * np.sign(rng.standard_normal(1000)))).astype(np.complex64)


def found(y: np.ndarray) -> int:
    """Where the repeated waveform's correlation peaks in ``y``."""
    peak = np.abs(np.correlate(y, x, mode="valid"))
    return int(np.argmax(peak))


with ezsdr.connect(rx={0: ezsdr.Stream([0], sample_rate=19.5e6)}, tx={0: [0]}) as sdr:
    sdr.tx.repeat(x)
    # A capture meant to start at an instant names one ahead of the Run's time (EA-17).
    ahead = sdr.after(0.05)
    y = sdr.rx.capture(N, at=ahead)
    # Whether a capture at an instant just passed still starts there: EA-17's INFERRED case.
    t = sdr.sleep(0.1)
    z = sdr.rx.capture(N, at=t)
    print(f"sample rate {sdr.rx.sample_rate} S/s (19.5 MS/s asked at connect, coerced)")
    # A radio value has no update class: a retune during the Run is refused (RM-27).
    try:
        sdr.set("radio", "radio.tx.c0.frequency_hz", 2.4e9)
        print("the transmit retune was admitted: check radio.tx.c0.frequency_hz's update class")
    except ezsdr.Rejected as rejected:
        print(f"the transmit retune during the Run was refused: {rejected}")

manifest = sdr.manifest
captures = [a for listed in manifest["artifacts"].values() for a in listed if a["continuity"]]
print(f"y: correlation peak at sample {found(y)}; z: at sample {found(z)}")
for artifact in captures[:2]:
    print(f"{artifact['id']}: first sample {artifact['continuity'][0]['first']}")
print(f"asked: y at {ahead}, z at {t}")
time_errors = sum(row["count"] for row in manifest["events"]["counters"] if row["kind"] == "radio.TIME_ERROR")
print(f"TIME_ERROR events: {time_errors} (spike K6: none)")
print(f"Manifest: {sdr.manifest_path}")
