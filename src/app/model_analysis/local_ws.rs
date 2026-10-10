//! FORK(no-cloud): a minimal RFC 6455 WebSocket *client* over an arbitrary
//! local byte stream.
//!
//! Codex's settings control socket speaks JSON-RPC in WebSocket frames, and
//! `codex app-server proxy` tunnels it over its own stdin/stdout. AIW used
//! the `tungstenite` crate for that framing; AIW links no HTTP/WebSocket
//! library at all (see `NO_CLOUD.md`), so the few pieces the settings
//! connection needs live here: the HTTP Upgrade handshake, masked client
//! frames, and reading text/control frames. There is no networking in this
//! module — the caller hands it the proxy's pipes (or, in tests, a
//! `UnixStream` pair).
//!
//! Everything is non-blocking friendly: `WouldBlock` from the stream is
//! reported as "not yet" (`Ok(false)` / `Ok(None)`) and buffered state is
//! kept, so the caller can poll against its own deadline.

use anyhow::{Context, Result, bail, ensure};
use base64ct::{Base64, Encoding};
use sha1::{Digest, Sha1};
use std::io::{ErrorKind, Read, Write};

const ACCEPT_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_HANDSHAKE_BYTES: usize = 16 * 1024;
/// Same ceiling the settings connection configured on tungstenite.
pub(super) const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

const OP_CONTINUATION: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Frame {
    Text(String),
    Binary,
    Ping,
    Pong,
}

pub(super) struct WsClient<S: Read + Write> {
    io: S,
    key: String,
    open: bool,
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    /// A data message whose final fragment hasn't arrived yet.
    fragment: Option<(u8, Vec<u8>)>,
}

impl<S: Read + Write> WsClient<S> {
    /// Queue the Upgrade request; [`handshake_step`](Self::handshake_step)
    /// sends it and waits for the answer.
    pub(super) fn new(io: S) -> Self {
        let key = Base64::encode_string(uuid::Uuid::new_v4().as_bytes());
        let request = format!(
            "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
             Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n\r\n"
        );
        Self {
            io,
            key,
            open: false,
            inbuf: Vec::new(),
            outbuf: request.into_bytes(),
            fragment: None,
        }
    }

    /// Advance the handshake. `Ok(true)` once the server has accepted it,
    /// `Ok(false)` while the stream would block.
    pub(super) fn handshake_step(&mut self) -> Result<bool> {
        if self.open {
            return Ok(true);
        }
        if !self.flush()? {
            return Ok(false);
        }
        loop {
            if let Some(end) = find(&self.inbuf, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&self.inbuf[..end]).into_owned();
                self.inbuf.drain(..end + 4);
                check_response(&head, &expected_accept(&self.key))?;
                self.open = true;
                return Ok(true);
            }
            ensure!(
                self.inbuf.len() <= MAX_HANDSHAKE_BYTES,
                "WebSocket handshake response too large"
            );
            if !self.fill()? {
                return Ok(false);
            }
        }
    }

    /// Queue one masked text message; [`flush`](Self::flush) sends it.
    pub(super) fn queue_text(&mut self, text: &str) {
        self.queue_frame(OP_TEXT, text.as_bytes());
    }

    /// Write whatever is queued. `Ok(true)` when everything is out,
    /// `Ok(false)` if the stream would block (the rest stays queued — a
    /// frame is never queued twice).
    pub(super) fn flush(&mut self) -> Result<bool> {
        while !self.outbuf.is_empty() {
            match self.io.write(&self.outbuf) {
                Ok(0) => bail!("Codex control connection closed"),
                Ok(n) => {
                    self.outbuf.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        match self.io.flush() {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// The next complete message or control frame, `Ok(None)` if the stream
    /// would block first. A ping is answered with a pong automatically.
    pub(super) fn read(&mut self) -> Result<Option<Frame>> {
        ensure!(self.open, "WebSocket handshake not finished");
        loop {
            while let Some((fin, opcode, payload)) = self.take_frame()? {
                match opcode {
                    OP_TEXT | OP_BINARY => {
                        ensure!(
                            self.fragment.is_none(),
                            "Interleaved Codex control messages"
                        );
                        if fin {
                            return deliver(opcode, payload).map(Some);
                        }
                        self.fragment = Some((opcode, payload));
                    }
                    OP_CONTINUATION => {
                        let (opcode, mut message) = self
                            .fragment
                            .take()
                            .context("Unexpected Codex control continuation frame")?;
                        message.extend_from_slice(&payload);
                        ensure!(
                            message.len() <= MAX_MESSAGE_BYTES,
                            "Codex control message exceeds limit"
                        );
                        if fin {
                            return deliver(opcode, message).map(Some);
                        }
                        self.fragment = Some((opcode, message));
                    }
                    OP_CLOSE => bail!("Codex closed the control connection"),
                    OP_PING => {
                        self.queue_frame(OP_PONG, &payload);
                        self.flush()?;
                        return Ok(Some(Frame::Ping));
                    }
                    OP_PONG => return Ok(Some(Frame::Pong)),
                    other => bail!("Unknown Codex control frame opcode {other:#x}"),
                }
            }
            if !self.fill()? {
                return Ok(None);
            }
        }
    }

    /// Read once from the stream. `Ok(false)` on `WouldBlock`; end of stream
    /// is an error — the caller is always waiting for something.
    fn fill(&mut self) -> Result<bool> {
        let mut buffer = [0; 8192];
        loop {
            match self.io.read(&mut buffer) {
                Ok(0) => bail!("Codex control connection closed"),
                Ok(n) => {
                    self.inbuf.extend_from_slice(&buffer[..n]);
                    return Ok(true);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Pop one whole frame off the input buffer, if one is there.
    fn take_frame(&mut self) -> Result<Option<(bool, u8, Vec<u8>)>> {
        let b = &self.inbuf;
        if b.len() < 2 {
            return Ok(None);
        }
        ensure!(b[0] & 0x70 == 0, "Unexpected WebSocket extension bits");
        let fin = b[0] & 0x80 != 0;
        let opcode = b[0] & 0x0f;
        let masked = b[1] & 0x80 != 0;
        let mut len = u64::from(b[1] & 0x7f);
        let mut at = 2;
        if len == 126 {
            if b.len() < 4 {
                return Ok(None);
            }
            len = u64::from(u16::from_be_bytes([b[2], b[3]]));
            at = 4;
        } else if len == 127 {
            if b.len() < 10 {
                return Ok(None);
            }
            len = u64::from_be_bytes(b[2..10].try_into().unwrap());
            at = 10;
        }
        ensure!(
            len <= MAX_MESSAGE_BYTES as u64,
            "Codex control frame exceeds limit"
        );
        let mask = if masked {
            if b.len() < at + 4 {
                return Ok(None);
            }
            let mask = [b[at], b[at + 1], b[at + 2], b[at + 3]];
            at += 4;
            Some(mask)
        } else {
            None
        };
        let len = len as usize;
        if b.len() < at + len {
            return Ok(None);
        }
        let mut payload = b[at..at + len].to_vec();
        if let Some(mask) = mask {
            apply_mask(&mut payload, mask);
        }
        self.inbuf.drain(..at + len);
        Ok(Some((fin, opcode, payload)))
    }

    fn queue_frame(&mut self, opcode: u8, payload: &[u8]) {
        let out = &mut self.outbuf;
        out.push(0x80 | opcode);
        let len = payload.len();
        if len < 126 {
            out.push(0x80 | len as u8);
        } else if len <= usize::from(u16::MAX) {
            out.push(0x80 | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            out.push(0x80 | 127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }
        // Clients must mask every frame (RFC 6455 §5.3).
        let mask: [u8; 4] = uuid::Uuid::new_v4().as_bytes()[..4].try_into().unwrap();
        out.extend_from_slice(&mask);
        let start = out.len();
        out.extend_from_slice(payload);
        apply_mask(&mut out[start..], mask);
    }
}

fn deliver(opcode: u8, payload: Vec<u8>) -> Result<Frame> {
    if opcode == OP_BINARY {
        return Ok(Frame::Binary);
    }
    String::from_utf8(payload)
        .map(Frame::Text)
        .context("Codex control message is not UTF-8")
}

fn apply_mask(bytes: &mut [u8], mask: [u8; 4]) {
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn expected_accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(ACCEPT_GUID.as_bytes());
    Base64::encode_string(&hasher.finalize())
}

fn check_response(head: &str, expected_accept: &str) -> Result<()> {
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let mut parts = status.split_whitespace();
    ensure!(
        parts.next().is_some_and(|v| v.starts_with("HTTP/1.")) && parts.next() == Some("101"),
        "WebSocket upgrade refused: {status}"
    );
    let accept = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("sec-websocket-accept"))
        .map(|(_, value)| value.trim())
        .context("WebSocket upgrade response has no Sec-WebSocket-Accept")?;
    ensure!(
        accept == expected_accept,
        "WebSocket upgrade response has the wrong Sec-WebSocket-Accept"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    /// Client against tungstenite's reference server (a dev-dependency only).
    fn pair(
        server_script: impl FnOnce(&mut tungstenite::WebSocket<UnixStream>) + Send + 'static,
    ) -> (WsClient<UnixStream>, std::thread::JoinHandle<()>) {
        let (client, server) = UnixStream::pair().unwrap();
        let handle = std::thread::spawn(move || {
            let mut server = tungstenite::accept(server).unwrap();
            server_script(&mut server);
        });
        client.set_nonblocking(true).unwrap();
        let mut ws = WsClient::new(client);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ws.handshake_step().unwrap() {
            assert!(Instant::now() < deadline, "handshake timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        (ws, handle)
    }

    fn next(ws: &mut WsClient<UnixStream>) -> Result<Frame> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(frame) = ws.read()? {
                return Ok(frame);
            }
            assert!(Instant::now() < deadline, "read timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn send(ws: &mut WsClient<UnixStream>, text: &str) {
        ws.queue_text(text);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ws.flush().unwrap() {
            assert!(Instant::now() < deadline, "write timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn round_trips_short_medium_and_long_messages() {
        let medium = "m".repeat(300);
        let long = "l".repeat(70_000);
        let (mut ws, server) = pair(|server| {
            for _ in 0..3 {
                let message = server.read().unwrap();
                server.send(message).unwrap();
            }
        });
        for text in ["hi", medium.as_str(), long.as_str()] {
            send(&mut ws, text);
            assert_eq!(next(&mut ws).unwrap(), Frame::Text(text.to_string()));
        }
        server.join().unwrap();
    }

    #[test]
    fn answers_pings_and_reports_close_as_an_error() {
        let (mut ws, server) = pair(|server| {
            server
                .send(tungstenite::Message::Ping(b"are you there".to_vec().into()))
                .unwrap();
            // The pong (echoing the ping) arrives before the client's next message.
            match server.read().unwrap() {
                tungstenite::Message::Pong(payload) => assert_eq!(&payload[..], b"are you there"),
                other => panic!("expected the pong, got {other:?}"),
            }
            match server.read().unwrap() {
                tungstenite::Message::Text(text) => assert_eq!(text.as_str(), "after ping"),
                other => panic!("expected the text message, got {other:?}"),
            }
            let _ = server.close(None);
            let _ = server.flush();
        });
        assert_eq!(next(&mut ws).unwrap(), Frame::Ping);
        send(&mut ws, "after ping");
        assert!(next(&mut ws).is_err());
        server.join().unwrap();
    }

    #[test]
    fn reassembles_fragmented_text() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let mut ws = WsClient::new(client);
        ws.open = true;
        // "hel" (not final) + "lo" (final continuation), unmasked from a server.
        server
            .write_all(&[0x01, 3, b'h', b'e', b'l', 0x80, 2, b'l', b'o'])
            .unwrap();
        assert_eq!(ws.read().unwrap(), Some(Frame::Text("hello".into())));
    }

    #[test]
    fn rejects_a_wrong_accept_key() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let mut ws = WsClient::new(client);
        server
            .write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
                  Connection: Upgrade\r\nSec-WebSocket-Accept: bogus\r\n\r\n",
            )
            .unwrap();
        assert!(ws.handshake_step().is_err());
    }

    #[test]
    fn accept_key_matches_the_rfc_example() {
        assert_eq!(
            expected_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }
}
