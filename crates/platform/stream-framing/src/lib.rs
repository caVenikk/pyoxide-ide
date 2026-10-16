//! `Content-Length` framing of JSON messages over a byte stream: a header
//! block of `Name: value` lines ended by an empty line, then exactly
//! `Content-Length` bytes of JSON. The base protocol of both the Language
//! Server Protocol and the Debug Adapter Protocol (ADR 0010, 0015), used by
//! `lsp-client`, `dap-client` and their fake servers.
//!
//! The decoder takes the stream in chunks of any size (a header or a
//! multi-byte character may be split between chunks) and never panics on
//! malformed input: junk lines between frames are skipped, a header block
//! without a usable length is reported and dropped, and memory stays
//! bounded by the frame size limit.

use std::fmt;

use serde_json::Value;

/// The longest header line kept while waiting for its end; longer junk
/// without line breaks is dropped.
const MAX_HEADER_LINE: usize = 64 * 1024;

/// The largest frame accepted. A server's largest answers (thousands of
/// diagnostics, a broad completion list) are a few megabytes.
pub const MAX_CONTENT_LENGTH: usize = 256 * 1024 * 1024;

/// A frame with `body` as its content.
pub fn encode(body: &[u8]) -> Vec<u8> {
    let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    frame.extend_from_slice(body);
    frame
}

/// Why bytes of the stream were dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// A header block without a valid `Content-Length` (its text, shortened).
    MissingLength(String),
    /// A `Content-Length` above [`MAX_CONTENT_LENGTH`].
    TooLarge(usize),
    /// A line longer than the header limit, without a line break.
    LongLine,
    /// The content is not JSON.
    Json(String),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::MissingLength(header) => {
                write!(f, "a message header without Content-Length: {header}")
            }
            FrameError::TooLarge(length) => write!(f, "a message of {length} bytes is too large"),
            FrameError::LongLine => f.write_str("output that is not a message header"),
            FrameError::Json(error) => write!(f, "a message that is not JSON: {error}"),
        }
    }
}

/// Parses frames from chunks of the stream.
///
/// ```
/// let mut decoder = stream_framing::FrameDecoder::new();
/// decoder.push(&stream_framing::encode(br#"{"seq": 1}"#));
/// assert_eq!(decoder.next_frame(), Some(Ok(serde_json::json!({"seq": 1}))));
/// assert_eq!(decoder.next_frame(), None);
/// ```
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    /// Start of the unconsumed bytes in `buf`.
    pos: usize,
    /// Where the search for the next line break resumes (an index into
    /// `buf`, at or after `pos`).
    scan: usize,
    state: State,
    /// The header block read so far: lines seen, the length found.
    header: Header,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Headers,
    Body(usize),
}

#[derive(Debug, Default)]
struct Header {
    lines: usize,
    length: Option<usize>,
    invalid_length: bool,
    text: String,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a chunk of the stream; take the frames it completes with
    /// [`FrameDecoder::next_frame`].
    pub fn push(&mut self, chunk: &[u8]) {
        if self.pos > 0 && (self.pos == self.buf.len() || self.pos >= self.buf.len() / 2) {
            self.buf.drain(..self.pos);
            self.scan -= self.pos;
            self.pos = 0;
        }
        self.buf.extend_from_slice(chunk);
    }

    /// Bytes received but not yet part of a returned frame.
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// The next complete frame: its JSON content, or why some bytes were
    /// dropped. `None` when more input is needed.
    pub fn next_frame(&mut self) -> Option<Result<Value, FrameError>> {
        loop {
            match self.state {
                State::Body(length) => {
                    let available = self.buf.len() - self.pos;
                    if available < length {
                        self.buf.reserve(length - available);
                        return None;
                    }
                    let body = &self.buf[self.pos..self.pos + length];
                    let parsed = serde_json::from_slice::<Value>(body)
                        .map_err(|e| FrameError::Json(e.to_string()));
                    self.pos += length;
                    self.scan = self.pos;
                    self.state = State::Headers;
                    return Some(parsed);
                }
                State::Headers => {
                    let newline = self.buf[self.scan..].iter().position(|&b| b == b'\n');
                    let Some(newline) = newline else {
                        self.scan = self.buf.len();
                        if self.buf.len() - self.pos > MAX_HEADER_LINE {
                            self.pos = self.buf.len();
                            self.scan = self.pos;
                            self.header = Header::default();
                            return Some(Err(FrameError::LongLine));
                        }
                        return None;
                    };
                    let end = self.scan + newline;
                    let mut line = &self.buf[self.pos..end];
                    if let Some(stripped) = line.strip_suffix(b"\r") {
                        line = stripped;
                    }
                    let line_is_empty = line.is_empty();
                    if !line_is_empty {
                        self.header.add(line);
                    }
                    self.pos = end + 1;
                    self.scan = self.pos;
                    if !line_is_empty {
                        continue;
                    }
                    let header = std::mem::take(&mut self.header);
                    match header.length {
                        Some(length) if length > MAX_CONTENT_LENGTH => {
                            return Some(Err(FrameError::TooLarge(length)));
                        }
                        Some(length) if !header.invalid_length => {
                            self.state = State::Body(length);
                        }
                        _ if header.lines == 0 => {
                            // An empty line between frames: nothing to report.
                        }
                        _ => return Some(Err(FrameError::MissingLength(header.text))),
                    }
                }
            }
        }
    }
}

impl Header {
    /// The longest header text kept for an error message.
    const TEXT_LIMIT: usize = 200;

    /// Takes one header line. The `Content-Length` header is also found
    /// after junk on the same line: after a frame with a wrong length the
    /// rest of its content and the next header share a line, and the
    /// stream gets back in step there.
    fn add(&mut self, line: &[u8]) {
        const NAME: &[u8] = b"content-length";
        self.lines += 1;
        if self.text.len() < Self::TEXT_LIMIT {
            if !self.text.is_empty() {
                self.text.push_str("\\r\\n");
            }
            let shown = &line[..line.len().min(Self::TEXT_LIMIT)];
            self.text.push_str(&String::from_utf8_lossy(shown));
        }
        let Some(at) = line
            .windows(NAME.len())
            .rposition(|w| w.eq_ignore_ascii_case(NAME))
        else {
            return;
        };
        let Some(value) = line[at + NAME.len()..]
            .trim_ascii_start()
            .strip_prefix(b":")
        else {
            return;
        };
        let value = std::str::from_utf8(value.trim_ascii())
            .ok()
            .and_then(|v| v.parse::<usize>().ok());
        match value {
            Some(length) => {
                self.length = Some(length);
                self.invalid_length = false;
            }
            None => self.invalid_length = true,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn frame(value: &Value) -> Vec<u8> {
        encode(&serde_json::to_vec(value).unwrap())
    }

    fn decode_all(decoder: &mut FrameDecoder) -> Vec<Result<Value, FrameError>> {
        std::iter::from_fn(|| decoder.next_frame()).collect()
    }

    /// A small deterministic generator (xorshift64*) for the randomised
    /// tests; no extra dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    #[test]
    fn one_frame_in_one_chunk() {
        let mut decoder = FrameDecoder::new();
        decoder.push(&frame(&json!({"jsonrpc": "2.0", "id": 1, "result": null})));
        assert_eq!(
            decode_all(&mut decoder),
            [Ok(json!({"jsonrpc": "2.0", "id": 1, "result": null}))]
        );
        assert_eq!(decoder.buffered(), 0);
    }

    #[test]
    fn frames_split_at_every_byte() {
        let messages = [
            json!({"method": "a", "params": {"text": "héllo wörld 🦀 中文"}}),
            json!({"id": 7, "result": [1, 2, 3]}),
            json!({"method": "b"}),
        ];
        let stream: Vec<u8> = messages.iter().flat_map(frame).collect();
        for split in 1..stream.len() {
            let mut decoder = FrameDecoder::new();
            let mut out = Vec::new();
            decoder.push(&stream[..split]);
            out.extend(decode_all(&mut decoder));
            decoder.push(&stream[split..]);
            out.extend(decode_all(&mut decoder));
            let out: Vec<Value> = out.into_iter().map(Result::unwrap).collect();
            assert_eq!(out, messages, "split at {split}");
        }
        // Byte by byte.
        let mut decoder = FrameDecoder::new();
        let mut out = Vec::new();
        for byte in &stream {
            decoder.push(std::slice::from_ref(byte));
            out.extend(decode_all(&mut decoder).into_iter().map(Result::unwrap));
        }
        assert_eq!(out, messages);
    }

    #[test]
    fn headers_in_any_case_with_content_type_and_bare_line_feeds() {
        let body = br#"{"id":1,"result":"x"}"#;
        let mut stream = Vec::new();
        stream.extend_from_slice(
            b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length:  21 \r\n\r\n",
        );
        stream.extend_from_slice(body);
        stream.extend_from_slice(b"CONTENT-LENGTH: 21\n\n");
        stream.extend_from_slice(body);
        let mut decoder = FrameDecoder::new();
        decoder.push(&stream);
        assert_eq!(
            decode_all(&mut decoder),
            [
                Ok(json!({"id": 1, "result": "x"})),
                Ok(json!({"id": 1, "result": "x"}))
            ]
        );
    }

    #[test]
    fn junk_between_frames_is_skipped_or_reported() {
        let good = frame(&json!({"id": 1, "result": 2}));
        let mut stream = Vec::new();
        stream.extend_from_slice(b"\r\n\r\nstarting server...\n");
        stream.extend_from_slice(&good);
        stream.extend_from_slice(b"Foo: bar\r\n\r\n");
        stream.extend_from_slice(b"Content-Length: abc\r\n\r\n");
        stream.extend_from_slice(&good);
        let mut decoder = FrameDecoder::new();
        decoder.push(&stream);
        let out = decode_all(&mut decoder);
        assert_eq!(out.len(), 4, "{out:?}");
        // The junk line and the next header form one block with a length.
        assert_eq!(out[0], Ok(json!({"id": 1, "result": 2})));
        assert!(matches!(&out[1], Err(FrameError::MissingLength(h)) if h == "Foo: bar"));
        assert!(matches!(&out[2], Err(FrameError::MissingLength(_))));
        assert_eq!(out[3], Ok(json!({"id": 1, "result": 2})));
    }

    #[test]
    fn invalid_json_and_utf8_are_reported_and_the_stream_goes_on() {
        let mut stream = encode(b"{not json");
        stream.extend_from_slice(&encode(b"\"\xff\xfe\""));
        stream.extend_from_slice(&frame(&json!({"method": "ok"})));
        let mut decoder = FrameDecoder::new();
        decoder.push(&stream);
        let out = decode_all(&mut decoder);
        assert!(matches!(out[0], Err(FrameError::Json(_))));
        assert!(matches!(out[1], Err(FrameError::Json(_))));
        assert_eq!(out[2], Ok(json!({"method": "ok"})));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn oversized_lengths_and_endless_lines_are_dropped() {
        let mut decoder = FrameDecoder::new();
        decoder.push(format!("Content-Length: {}\r\n\r\n", MAX_CONTENT_LENGTH + 1).as_bytes());
        assert_eq!(
            decoder.next_frame(),
            Some(Err(FrameError::TooLarge(MAX_CONTENT_LENGTH + 1)))
        );
        decoder.push(&vec![b'x'; MAX_HEADER_LINE + 10]);
        assert_eq!(decoder.next_frame(), Some(Err(FrameError::LongLine)));
        assert_eq!(decoder.buffered(), 0);
        decoder.push(&frame(&json!({"method": "after"})));
        assert_eq!(decoder.next_frame(), Some(Ok(json!({"method": "after"}))));
        // A length that does not fit in usize is not a length.
        decoder.push(b"Content-Length: 99999999999999999999999999\r\n\r\n");
        assert!(matches!(
            decoder.next_frame(),
            Some(Err(FrameError::MissingLength(_)))
        ));
    }

    #[test]
    fn a_large_frame_in_small_chunks() {
        let text: String = "abc中文🦀".repeat(200_000);
        let message = json!({"id": 3, "result": {"text": text}});
        let stream = frame(&message);
        assert!(stream.len() > 2_000_000);
        let mut decoder = FrameDecoder::new();
        let mut out = Vec::new();
        for chunk in stream.chunks(8192) {
            decoder.push(chunk);
            out.extend(decode_all(&mut decoder));
        }
        assert_eq!(out, [Ok(message)]);
        assert_eq!(decoder.buffered(), 0);
    }

    #[test]
    fn back_in_step_after_a_wrong_length() {
        // The first frame claims fewer bytes than it has: the rest of its
        // content runs into the next header without a line break.
        let mut stream = b"Content-Length: 5\r\n\r\n{\"id\":1,\"result\":2}".to_vec();
        stream.extend(frame(&json!({"id": 2, "result": 3})));
        stream.extend(frame(&json!({"id": 3, "result": 4})));
        let mut decoder = FrameDecoder::new();
        decoder.push(&stream);
        let ok: Vec<Value> = decode_all(&mut decoder)
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(
            ok,
            [json!({"id": 2, "result": 3}), json!({"id": 3, "result": 4})]
        );
    }

    /// Random valid frames mixed with random junk, cut into random chunks:
    /// never a panic, and junk costs at most the one frame it runs into.
    #[test]
    fn fuzzed_streams_never_panic() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for round in 0..300 {
            let mut stream = Vec::new();
            let mut expected = Vec::new();
            let mut junk = 0;
            for _ in 0..rng.below(8) + 1 {
                match rng.below(5) {
                    0 => {
                        // Random bytes, sometimes ended by empty lines.
                        let n = rng.below(300);
                        stream.extend((0..n).map(|_| rng.next() as u8));
                        if rng.below(2) == 0 {
                            stream.extend_from_slice(b"\r\n\r\n");
                        }
                        junk += 1;
                    }
                    1 => {
                        // A header with a wrong length followed by a body.
                        stream.extend_from_slice(
                            format!("Content-Length: {}\r\n\r\n", rng.below(20)).as_bytes(),
                        );
                        stream.extend_from_slice(br#"{"id":"junk"}"#);
                        junk += 1;
                    }
                    _ => {
                        let value =
                            json!({"id": round, "result": {"n": rng.next() as u32, "s": "é🦀"}});
                        stream.extend(frame(&value));
                        expected.push(value);
                    }
                }
            }
            let mut decoder = FrameDecoder::new();
            let mut decoded = Vec::new();
            let mut at = 0;
            while at < stream.len() {
                let n = (rng.below(64) + 1).min(stream.len() - at);
                decoder.push(&stream[at..at + n]);
                at += n;
                decoded.extend(decode_all(&mut decoder).into_iter().filter_map(Result::ok));
            }
            // The expected frames that were decoded come in order, and junk
            // lost at most one frame each.
            let mut from = 0;
            let mut found = 0;
            for e in &expected {
                if let Some(i) = decoded[from..].iter().position(|d| d == e) {
                    from += i + 1;
                    found += 1;
                }
            }
            assert!(
                expected.len() - found <= junk,
                "round {round}: {found} of {} with {junk} junk parts",
                expected.len()
            );
            assert!(decoder.buffered() <= MAX_HEADER_LINE + 64);
        }
        // Pure noise in random chunks.
        for _ in 0..200 {
            let mut decoder = FrameDecoder::new();
            for _ in 0..rng.below(50) {
                let n = rng.below(200);
                let chunk: Vec<u8> = (0..n)
                    .map(|_| match rng.below(6) {
                        0 => b'\n',
                        1 => b':',
                        2 => b'\r',
                        _ => rng.next() as u8,
                    })
                    .collect();
                decoder.push(&chunk);
                let _ = decode_all(&mut decoder);
                decoder.push(b"Content-Length: 2\r\n\r\n{}");
                let _ = decode_all(&mut decoder);
            }
        }
    }
}
