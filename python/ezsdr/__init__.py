"""Ez-SDR v4 Easy API (design/16-easy-api.md).

    import ezsdr

    with ezsdr.connect() as sdr:
        sdr.tx.repeat(x)
        y = sdr.rx.capture(100_000)

``connect()`` opens a Session on ``ezsdr-server``, which runs it in software unless its
profile binds hardware; the Session's Manifest is ``sdr.manifest`` afterwards.
"""

from ._client import Error, ProtocolError, RunEnded, ServerError
from .session import (
    CaptureRequest,
    CaptureTimeout,
    Radio,
    Rejected,
    RunResult,
    Rx,
    Session,
    Tx,
    connect,
    samples,
    waveform,
)

__version__ = "0.6.0"

__all__ = [
    "CaptureRequest",
    "CaptureTimeout",
    "Error",
    "ProtocolError",
    "Radio",
    "Rejected",
    "RunEnded",
    "RunResult",
    "Rx",
    "ServerError",
    "Session",
    "Tx",
    "connect",
    "samples",
    "waveform",
]
