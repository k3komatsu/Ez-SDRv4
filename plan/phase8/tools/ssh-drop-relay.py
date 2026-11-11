"""The client's end of ssh-drop-tx.sh's server wrapper: starts the ssh command and logs the request headers the client sends.

usage: ssh-drop-relay.py LOG PIDFILE -- COMMAND [ARG...]
       ssh-drop-relay.py --self-check

COMMAND runs with the relay's standard output as its own (the replies do not pass through the relay), and its pid goes to
PIDFILE, so the script drops the link by killing it. Everything the client writes is passed on unchanged. LOG gets one line
for each request header, `<time.time_ns()>\\t<op>\\t<body_bytes>`, stamped when the relay read the header: the script's evidence
that the server was inside a request when the link dropped (ezsdr.protocol 2 frames: a JSON header line, then body_bytes).
The relay exits when COMMAND does, so the client sees its server gone as it would without the relay.
"""

import json
import os
import subprocess
import sys
import tempfile
import threading
import time


def header(line):
    """The op and body size of a request header line; `?` and 0 for a line that is not one."""
    try:
        frame = json.loads(line)
        return str(frame["request"]["op"]), int(frame.get("body_bytes", 0))
    except (ValueError, KeyError, TypeError):
        return "?", 0


def relay(log, pidfile, command):
    child = subprocess.Popen(command, stdin=subprocess.PIPE)
    with open(pidfile, "w", encoding="utf-8") as file:
        file.write(f"{child.pid}\n")
    # The command now holds the only write end of the reply pipe: its death is the client's end of output.
    os.close(1)
    threading.Thread(target=lambda: os._exit(0 if child.wait() == 0 else 1), daemon=True).start()
    source = sys.stdin.buffer
    with open(log, "a", buffering=1, encoding="utf-8") as out:
        try:
            while True:
                line = source.readline()
                if not line:
                    break
                stamp = time.time_ns()
                op, body = header(line)
                out.write(f"{stamp}\t{op}\t{body}\n")
                child.stdin.write(line)
                while body > 0:
                    chunk = source.read(min(body, 65536))
                    if not chunk:
                        break
                    child.stdin.write(chunk)
                    body -= len(chunk)
                child.stdin.flush()
            child.stdin.close()
        except BrokenPipeError:
            pass
    return child.wait()


def self_check():
    frames = (
        b'{"request":{"op":"hello","protocol":2},"body_bytes":0}\n'
        b'{"request":{"op":"submit"},"body_bytes":5}\nhello'
        b'{"request":{"op":"advance","by":{}},"body_bytes":0}\n'
        b"not json\n"
    )
    with tempfile.TemporaryDirectory(prefix="ssh-drop-relay-") as directory:
        log, pidfile, copy = (os.path.join(directory, name) for name in ("log", "pid", "copy"))
        with open(copy, "wb") as sink:
            done = subprocess.run([sys.executable, __file__, log, pidfile, "--", "cat"], input=frames, stdout=sink, timeout=30)
        assert done.returncode == 0, done.returncode
        assert open(copy, "rb").read() == frames, "the relay changed what the client wrote"
        rows = [line.split("\t") for line in open(log, encoding="utf-8").read().splitlines()]
        assert [row[1:] for row in rows] == [["hello", "0"], ["submit", "5"], ["advance", "0"], ["?", "0"]], rows
        stamps = [int(row[0]) for row in rows]
        assert stamps == sorted(stamps) and abs(time.time_ns() - stamps[-1]) < 60 * 10**9, stamps
        assert open(pidfile, encoding="utf-8").read().strip().isdigit()
    print("ssh-drop-relay self-check passed: the bytes unchanged, one log line per header (the body counted, not stored), the pid written")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-check"]:
        self_check()
    elif len(sys.argv) > 4 and sys.argv[3] == "--":
        sys.exit(relay(sys.argv[1], sys.argv[2], sys.argv[4:]))
    else:
        sys.exit(__doc__)
