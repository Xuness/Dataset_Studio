use studio_application::llm::LlmCallResult;
use studio_domain::llm::LlmFailure;

/// Byte framing tolerates split UTF-8, CRLF, comments and multiple data lines.
#[derive(Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
    data: Vec<String>,
    frame_bytes: usize,
}
impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> LlmCallResult<Vec<String>> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = std::str::from_utf8(&line)
                .map_err(|_| LlmFailure::new("LLM_INVALID_STREAM", "事件不是有效 UTF-8"))?
                .trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(self.data.join("\n"));
                    self.data.clear();
                }
                self.frame_bytes = 0;
            } else if let Some(value) = line.strip_prefix("data:") {
                self.frame_bytes += value.len();
                self.data
                    .push(value.strip_prefix(' ').unwrap_or(value).into());
            }
            if self.frame_bytes > 1024 * 1024 {
                return Err(LlmFailure::new(
                    "LLM_RESPONSE_LIMIT",
                    "单个流式事件超过 1 MiB",
                ));
            }
        }
        if self.buffer.len() + self.frame_bytes > 1024 * 1024 {
            return Err(LlmFailure::new("LLM_RESPONSE_LIMIT", "流式缓冲超过 1 MiB"));
        }
        Ok(events)
    }
    pub fn finish(&self) -> LlmCallResult<()> {
        if self.buffer.iter().any(|b| !b.is_ascii_whitespace()) || !self.data.is_empty() {
            return Err(LlmFailure::new(
                "LLM_STREAM_INTERRUPTED",
                "流式响应结束于不完整事件",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_utf8_crlf_comments_and_multiple_lines() {
        let data = ": keepalive\r\ndata: {\"text\":\r\ndata: \"中文\"}\r\n\r\n".as_bytes();
        let mut decoder = SseDecoder::default();
        let mut values = Vec::new();
        for chunk in data.chunks(1) {
            values.extend(decoder.push(chunk).unwrap());
        }
        assert_eq!(values, vec!["{\"text\":\n\"中文\"}"]);
        decoder.finish().unwrap();
    }
    #[test]
    fn truncated_frame_and_oversized_frame_fail() {
        let mut decoder = SseDecoder::default();
        decoder.push(b"data: {\"x\":1}\n").unwrap();
        assert!(decoder.finish().is_err());
        assert!(
            SseDecoder::default()
                .push(&vec![b'x'; 1024 * 1024 + 1])
                .is_err()
        );
    }
}
