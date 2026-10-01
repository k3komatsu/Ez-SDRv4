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


with ezsdr.connect() as sdr:
    sdr.tx.repeat(x)
    # A capture meant to start at an instant names one ahead of the Run's time (EA-17).
    ahead = sdr.after(0.05)
    y = sdr.rx.capture(N, at=ahead)
    # Whether a capture at an instant just passed still starts there: EA-17's INFERRED case.
    t = sdr.sleep(0.1)
    z = sdr.rx.capture(N, at=t)
    sdr.rx.sample_rate = 19.5e6
    print(f"sample rate {sdr.rx.sample_rate} S/s (coerced, a new SampleClock)")
    # The RF envelope limits what is emitted (RM-19): a transmit retune outside it.
    try:
        sdr.tx.frequency = 2.4e9
        print("the transmit retune outside the RF envelope was admitted: check radio.rf_envelope")
    except ezsdr.Rejected as rejected:
        print(f"the transmit retune outside the RF envelope was refused: {rejected}")

manifest = sdr.manifest
captures = [a for a in manifest["artifacts"] if a["continuity"]]
print(f"y: correlation peak at sample {found(y)}; z: at sample {found(z)}")
for artifact in captures[:2]:
    print(f"{artifact['id']}: first sample {artifact['continuity'][0]['first']}")
print(f"asked: y at {ahead}, z at {t}")
time_errors = sum(row["count"] for row in manifest["events"]["counters"] if row["kind"] == "radio.TIME_ERROR")
print(f"TIME_ERROR events: {time_errors} (spike K6: none)")
print(f"Manifest: {sdr.manifest_path}")
