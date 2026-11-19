//! Incremental server-sent-events decoder. Yields the `data` payload of each event.

#[derive(Debug, Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
    data: Vec<String>,
}

impl SseDecoder {
    /// Feed raw bytes; collects the payloads of every event completed by them into `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<String>) {
        self.buf.extend_from_slice(bytes);
        // Slice lines out by read offset and drain once, instead of one drain per line.
        let mut pos = 0;
        while let Some(nl) = self.buf[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| pos + i)
        {
            let mut end = nl;
            if end > pos && self.buf[end - 1] == b'\r' {
                end -= 1;
            }
            // The line is scanned in place; only a kept `data:` payload is copied out.
            let Self { buf, data } = self;
            line(data, &buf[pos..end], out);
            pos = nl + 1;
        }
        self.buf.drain(..pos);
    }

    /// Collect an event left unterminated at end of stream into `out`.
    pub fn finish(&mut self, out: &mut Vec<String>) {
        let rest = std::mem::take(&mut self.buf);
        if !rest.is_empty() {
            line(&mut self.data, trim_cr(&rest), out);
        }
        line(&mut self.data, &[], out);
    }
}

/// A trailing partial line's `\r` characters, as `str::trim_end_matches('\r')` trims.
fn trim_cr(mut line: &[u8]) -> &[u8] {
    while line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    line
}

/// One SSE line, borrowed straight out of the receive buffer: a blank line dispatches
/// the event, a `data:` line accumulates its payload, everything else is ignored.
fn line(data: &mut Vec<String>, line: &[u8], out: &mut Vec<String>) {
    if line.is_empty() {
        if !data.is_empty() {
            out.push(std::mem::take(data).join("\n"));
        }
        return;
    }
    if let Some(rest) = line.strip_prefix(b"data:".as_slice()) {
        let rest = rest.strip_prefix(b" ".as_slice()).unwrap_or(rest);
        data.push(String::from_utf8_lossy(rest).into_owned());
    }
    // `event:`, `id:`, `retry:` and `:` comments are not used by the chat protocols.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(d: &mut SseDecoder, bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        d.feed(bytes, &mut out);
        out
    }

    #[test]
    fn splits_across_chunks_and_line_endings() {
        let mut d = SseDecoder::default();
        assert!(feed(&mut d, b": keep-alive\n\nda").is_empty());
        assert_eq!(
            feed(&mut d, b"ta: {\"a\":1}\r\n\r\ndata: x\n"),
            vec!["{\"a\":1}"]
        );
        assert_eq!(feed(&mut d, b"data: y\n\n"), vec!["x\ny"]);
        assert!(feed(&mut d, b"data: [DONE]").is_empty());
        let mut out = Vec::new();
        d.finish(&mut out);
        assert_eq!(out, vec!["[DONE]"]);
    }

    #[test]
    fn multibyte_split_is_preserved() {
        let mut d = SseDecoder::default();
        let s = "data: héllo\n\n".as_bytes();
        let (a, b) = s.split_at(8);
        assert!(feed(&mut d, a).is_empty());
        assert_eq!(feed(&mut d, b), vec!["héllo"]);
    }

    #[test]
    fn ignored_lines_and_trailing_carriage_return() {
        let mut d = SseDecoder::default();
        assert_eq!(
            feed(
                &mut d,
                b"event: message\nid: 7\nretry: 10\n: comment\ndata: a\ndata: b\n\n"
            ),
            vec!["a\nb"]
        );
        // An unterminated line with a `\r` still dispatches at end of stream.
        assert!(feed(&mut d, b"data: rest\r").is_empty());
        let mut out = Vec::new();
        d.finish(&mut out);
        assert_eq!(out, vec!["rest"]);
    }
}
