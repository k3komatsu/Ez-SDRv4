//! Framing and reply transport for one client (EA-2…EA-5).

use std::io::{self, BufRead, Read, Write};

use crate::protocol::{ErrorKind, MAX_HEADER_BYTES, ReplyFrame, RequestFrame};

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

/// Serves one client: reads frames from `input`, writes replies to `output` (EA-2).
pub fn serve(input: impl BufRead, mut output: impl Write, config: Config) -> io::Result<Exit> {
    let mut input = input;
    let mut server = Server::new(config);
    loop {
        let mut line = Vec::new();
        let read = (&mut input)
            .take(MAX_HEADER_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            server.disconnect();
            return Ok(Exit::EndOfInput);
        }
        if line.last() != Some(&b'\n') {
            let message = if line.len() > MAX_HEADER_BYTES {
                "EA-2: the header is longer than 16 MiB"
            } else {
                "EA-2: the stream ended inside a header"
            };
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
                write_reply(
                    &mut output,
                    &fail(ErrorKind::Protocol, "EA-2: the header is not a JSON object"),
                )?;
                server.disconnect();
                return Ok(Exit::BadFrame);
            }
        };
        let body_bytes = match header.get("body_bytes") {
            None => 0,
            Some(value) => match value.as_u64() {
                Some(n) => n,
                None => {
                    write_reply(
                        &mut output,
                        &fail(ErrorKind::Protocol, "EA-2: body_bytes is not a count"),
                    )?;
                    server.disconnect();
                    return Ok(Exit::BadFrame);
                }
            },
        };
        // ponytail: no limit on body_bytes; Phase 7's remote listener needs one.
        let mut body = Vec::new();
        let complete = (&mut input)
            .take(body_bytes)
            .read_to_end(&mut body)
            .is_ok_and(|n| n as u64 == body_bytes);
        if !complete {
            write_reply(
                &mut output,
                &fail(ErrorKind::Protocol, "EA-2: the stream ended inside a body"),
            )?;
            server.disconnect();
            return Ok(Exit::BadFrame);
        }
        let frame: RequestFrame = match serde_json::from_value(header) {
            Ok(frame) => frame,
            Err(error) => {
                write_reply(
                    &mut output,
                    &fail(
                        ErrorKind::Protocol,
                        format!("EA-2: not a request frame: {error}"),
                    ),
                )?;
                // Before `hello`, anything but `hello` ends the handshake (EA-3; Review I, P1-B).
                if !server.greeted() {
                    return Ok(Exit::BadFrame);
                }
                continue;
            }
        };
        let handled = server.handle(frame.request, body);
        write_reply(&mut output, &handled)?;
        if handled.exit {
            server.disconnect();
            return Ok(Exit::Replied);
        }
    }
}

fn write_reply(output: &mut impl Write, handled: &Handled) -> io::Result<()> {
    let frame = ReplyFrame {
        reply: handled.reply.clone(),
        body_bytes: handled.body.len() as u64,
    };
    let mut line = serde_json::to_vec(&frame).map_err(io::Error::other)?;
    line.push(b'\n');
    output.write_all(&line)?;
    output.write_all(&handled.body)?;
    output.flush()
}
