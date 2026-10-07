"""Vision §54's loop: one child Run per gain, each with its own Manifest (EA-19)."""

import ezsdr


def build_experiment(gain_db):
    """A fully expanded ExperimentSpec: receive 10 000 samples at the given gain."""
    return {
        "version": 1,
        "requirements": {"vocabularies": [{"id": "radio", "major": 2}, {"id": "sink", "major": 1}]},
        "resources": {
            "radio": {
                "kind": "radio.device",
                "requires": {"radio.rx.gain_db": {"kind": "eq", "value": gain_db}},
            }
        },
        "outputs": [
            {
                "id": "rec",
                "kind": "sink.capture",
                "params": {"sink.capture_samples": 10_000},
                "feed": {"port": {"component": "radio", "port": "rx"}, "policy": "drop_oldest", "capacity": 64},
            }
        ],
    }


with ezsdr.connect() as sdr:
    for gain in (0.0, 10.0, 20.0):
        result = sdr.run(build_experiment(gain), duration=0.02)
        y = result.capture("rec")
        print(f"gain {gain:4.1f} dB: {len(y)} samples, Manifest {result.path}")
