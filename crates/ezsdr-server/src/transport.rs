//! Framing and reply transport for one client (EA-2…EA-5).

use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::protocol::{ErrorKind, MAX_HEADER_BYTES, ReplyFrame, RequestFrame};
use crate::watch::Watch;

use super::{Config, Handled, Server, fail};

/// Why the server stopped reading.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exit {
    /// It sent a reply after which it exits (`finish`, a refused handshake, a failed connect).
    Replied,
    /// Its input ended.
    EndOfInput,
    /// A frame could not be read (EA-5).
    BadFrame,
}

/// Serves one client: reads frames from `input`, writes replies to `output` (EA-2). It
/// reads on the calling thread, so it sees the input end only after the frames before it:
/// for an input that holds every request (a test's).
pub fn serve(input: impl BufRead, output: impl Write, config: Config) -> io::Result<Exit> {
    frames(input, output, Server::new(config))
}

/// As [`serve`], but reads `input` on a thread of its own, so it sees the input end, or
/// fail, while it serves a request too: then it calls the disconnect signal of each Run it
/// is running and serves no further request (EA-15).
pub fn serve_watched(input: impl Read + Send + 'static, output: impl Write, config: Config) -> io::Result<Exit> {
    let watch = Arc::new(Watch::default());
    let (to, from) = mpsc::channel();
    let firing = watch.clone();
    // Not joined: after `finish` the client may keep its end open (N3: the channel is unbounded,
    // so the end is never held behind frames the server has not taken).
    std::thread::Builder::new().name("ezsdr-input".to_owned()).spawn(move || forward(input, &to, &firing))?;
    frames(BufReader::new(Received { from, chunk: Cursor::new(Vec::new()) }), output, Server::watched(config, watch))
}

/// The input thread: forwards what it reads until the input ends or fails, then fires.
fn forward(mut input: impl Read, to: &Sender<Vec<u8>>, watch: &Watch) {
    let mut buffer = vec![0; 64 * 1024];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                if to.send(buffer[..n].to_vec()).is_err() {
                    // The server has stopped reading: it is ending anyway.
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    watch.fire();
}

/// The framing loop's input under `serve_watched`: what the input thread forwarded.
struct Received {
    from: Receiver<Vec<u8>>,
    chunk: Cursor<Vec<u8>>,
}

impl Read for Received {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            let n = self.chunk.read(buffer)?;
            if n > 0 || buffer.is_empty() {
                return Ok(n);
            }
            match self.from.recv() {
                Ok(chunk) => self.chunk = Cursor::new(chunk),
                Err(_) => return Ok(0),
            }
        }
    }
}

fn frames(mut input: impl BufRead, mut output: impl Write, mut server: Server) -> io::Result<Exit> {
    loop {
        let mut line = Vec::new();
        let read = (&mut input).take(MAX_HEADER_BYTES as u64 + 1).read_until(b'\n', &mut line)?;
        if read == 0 {
            server.disconnect();
            return Ok(Exit::EndOfInput);
        }
        if line.last() != Some(&b'\n') {
            let message = if line.len() > MAX_HEADER_BYTES { "EA-2: the header is longer than 16 MiB" } else { "EA-2: the stream ended inside a header" };
            write_reply(&mut output, &fail(ErrorKind::Protocol, message))?;
            server.disconnect();
            return Ok(Exit::BadFrame);
        }
        // The frame in two steps: a JSON object whose `body_bytes` is readable frames the
        // stream, so a request that then fails to decode costs only that request
        // (Review H, P1-1); a header that is not such an object loses the framing.
        let header: serde_json::Value = match serde_json::from_slice(&line) {
            Ok(header @ serde_json::Value::Object(_)) => header,
            Ok(_) | Err(_) => {
                write_reply(&mut output, &fail(ErrorKind::Protocol, "EA-2: the header is not a JSON object"))?;
                server.disconnect();
                return Ok(Exit::BadFrame);
            }
        };
        let body_bytes = match header.get("body_bytes") {
            None => 0,
            Some(value) => match value.as_u64() {
                Some(n) => n,
                None => {
                    write_reply(&mut output, &fail(ErrorKind::Protocol, "EA-2: body_bytes is not a count"))?;
                    server.disconnect();
                    return Ok(Exit::BadFrame);
                }
            },
        };
        // ponytail: no limit on body_bytes; Phase 7's remote listener needs one.
        let mut body = Vec::new();
        let complete = (&mut input).take(body_bytes).read_to_end(&mut body).is_ok_and(|n| n as u64 == body_bytes);
        if !complete {
            write_reply(&mut output, &fail(ErrorKind::Protocol, "EA-2: the stream ended inside a body"))?;
            server.disconnect();
            return Ok(Exit::BadFrame);
        }
        let frame: RequestFrame = match serde_json::from_value(header) {
            Ok(frame) => frame,
            Err(error) => {
                write_reply(&mut output, &fail(ErrorKind::Protocol, format!("EA-2: not a request frame: {error}")))?;
                // Before `hello`, anything but `hello` ends the handshake (EA-3; Review I, P1-B).
                if !server.greeted() {
                    return Ok(Exit::BadFrame);
                }
                continue;
            }
        };
        // A2: a frame read after the input ended is not served; the end is.
        if server.input_ended() {
            server.disconnect();
            return Ok(Exit::EndOfInput);
        }
        let handled = server.handle(frame.request, body);
        write_reply(&mut output, &handled)?;
        if handled.exit {
            server.disconnect();
            return Ok(Exit::Replied);
        }
    }
}

fn write_reply(output: &mut impl Write, handled: &Handled) -> io::Result<()> {
    let frame = ReplyFrame { reply: handled.reply.clone(), body_bytes: handled.body.len() as u64 };
    let mut line = serde_json::to_vec(&frame).map_err(io::Error::other)?;
    line.push(b'\n');
    output.write_all(&line)?;
    output.write_all(&handled.body)?;
    output.flush()
}
