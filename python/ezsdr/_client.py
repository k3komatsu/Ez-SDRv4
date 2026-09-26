"""The server process and the frames of ezsdr.protocol 1 (spec 16 EA-2, EA-3, EA-5, A8)."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from typing import Any, Optional, Tuple

PROTOCOL = 1


class Error(Exception):
    """Base class of every ezsdr exception."""


class ServerError(Error):
    """The server refused a request (EA-5); ``kind`` is the protocol's error kind."""

    def __init__(self, kind: str, message: str, reply: dict):
        super().__init__(f"{kind}: {message}")
        self.kind = kind
        self.message = message
        self.reply = reply


class ProtocolError(ServerError):
    """A frame or request out of the protocol, or a version the server does not speak."""


class RunEnded(ServerError):
    """The Run has ended; ``termination`` says how."""

    def __init__(self, kind: str, message: str, reply: dict):
        super().__init__(kind, message, reply)
        self.termination = reply.get("termination")


def _raise(error: dict) -> None:
    kind, message = error["kind"], error["message"]
    if kind == "ended":
        raise RunEnded(kind, message, error)
    if kind in ("protocol", "unsupported_protocol"):
        raise ProtocolError(kind, message, error)
    raise ServerError(kind, message, error)


def find_server(server: Optional[str] = None) -> str:
    """``server``, else ``EZSDR_SERVER``, else ``ezsdr-server`` on ``PATH`` (A8)."""
    path = server or os.environ.get("EZSDR_SERVER") or shutil.which("ezsdr-server")
    if not path:
        raise Error(
            "ezsdr-server was not found: build it with `cargo build -p ezsdr-server` "
            "and set EZSDR_SERVER to the binary, or put it on PATH"
        )
    return path


class Connection:
    """One server process, spoken to one request at a time (EA-2)."""

    def __init__(self, server: Optional[str] = None, runs_dir: Optional[str] = None):
        args = [find_server(server)]
        if runs_dir is not None:
            args += ["--runs-dir", os.fspath(runs_dir)]
        self._process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        try:
            self.call({"op": "hello", "protocol": PROTOCOL})
        except BaseException:
            self.close()
            raise

    def call(self, request: dict, body: bytes = b"") -> Tuple[dict, bytes]:
        """Sends one request and returns the result and the reply's body."""
        header = json.dumps(
            {"request": request, "body_bytes": len(body)}, separators=(",", ":"), allow_nan=False
        )
        stdin, stdout = self._process.stdin, self._process.stdout
        if stdin is None or stdout is None or stdin.closed:
            raise ProtocolError("protocol", "the connection is closed", {})
        try:
            stdin.write(header.encode("utf-8") + b"\n" + body)
            stdin.flush()
        except BrokenPipeError:
            raise ProtocolError("protocol", "the server has exited", {}) from None
        line = stdout.readline()
        if not line:
            raise ProtocolError("protocol", f"the server closed its output (exit {self._process.poll()})", {})
        frame = json.loads(line)
        data = _read_exactly(stdout, frame.get("body_bytes", 0))
        reply = frame["reply"]
        if "error" in reply:
            _raise(reply["error"])
        return reply["result"], data

    def close(self) -> Optional[int]:
        """Closes the server's input and waits for it to exit."""
        if self._process.stdin is not None and not self._process.stdin.closed:
            try:
                self._process.stdin.close()
            except BrokenPipeError:
                pass
        try:
            return self._process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self._process.kill()
            return self._process.wait()
        finally:
            if self._process.stdout is not None:
                self._process.stdout.close()


def _read_exactly(stream: Any, count: int) -> bytes:
    chunks, remaining = [], count
    while remaining > 0:
        chunk = stream.read(remaining)
        if not chunk:
            raise ProtocolError("protocol", "the server's output ended inside a body", {})
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)
