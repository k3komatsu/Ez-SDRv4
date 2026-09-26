"""Vision §3's first snippet: repeat a waveform, capture what comes back (EA-19)."""

import numpy as np

import ezsdr

x = np.exp(2j * np.pi * np.arange(1000) / 100).astype(np.complex64) * 0.5

with ezsdr.connect() as sdr:
    sdr.tx.repeat(x)
    y = sdr.rx.capture(100_000)

print(f"captured {len(y)} samples, mean power {np.mean(np.abs(y) ** 2):.4f}")
print(f"Manifest: {sdr.manifest_path} ({len(sdr.manifest['action_log'])} logged calls)")
