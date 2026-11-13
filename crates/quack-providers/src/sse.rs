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
            let line = String::from_utf8_lossy(&self.buf[pos..end]).into_owned();
            self.line(&line, out);
            pos = nl + 1;
        }
        self.buf.drain(..pos);
    }

    /// Collect an event left unterminated at end of stream into `out`.
    pub fn finish(&mut self, out: &mut Vec<String>) {
        if !self.buf.is_empty() {
            let rest = String::from_utf8_lossy(&std::mem::take(&mut self.buf)).into_owned();
            self.line(rest.trim_end_matches('\r'), out);
        }
        self.line("", out);
    }

    fn line(&mut self, line: &str, out: &mut Vec<String>) {
        if line.is_empty() {
            if !self.data.is_empty() {
                out.push(std::mem::take(&mut self.data).join("\n"));
            }
            return;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            self.data
                .push(rest.strip_prefix(' ').unwrap_or(rest).to_owned());
        }
        // `event:`, `id:`, `retry:` and `:` comments are not used by the chat protocols.
    }
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
}
