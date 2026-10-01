//! A single lossless, bounded raw-record reader for details and analytical captures.
use flate2::{Decompress, FlushDecompress, Status};
use sha2::{Digest, Sha256};
use studio_domain::{Error, Result};

pub(crate) fn decode(bytes: &[u8], expected: u64, hash: &str, maximum: u64) -> Result<String> {
    if expected > maximum {
        return Err(Error::new("READ_BUDGET_EXCEEDED", "原始元数据超过解压预算"));
    }
    let capacity = usize::try_from(expected)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| Error::new("READ_BUDGET_EXCEEDED", "原始元数据长度无效"))?;
    let mut body = vec![0; capacity];
    let mut decoder = Decompress::new(true);
    let status = decoder
        .decompress(bytes, &mut body, FlushDecompress::Finish)
        .map_err(super::error)?;
    if status != Status::StreamEnd
        || decoder.total_out() != expected
        || decoder.total_in() != bytes.len() as u64
    {
        return Err(super::error("原始元数据压缩内容或长度校验失败"));
    }
    body.truncate(capacity - 1);
    if hex::encode(Sha256::digest(&body)) != hash {
        return Err(super::error("原始元数据摘要校验失败"));
    }
    String::from_utf8(body).map_err(super::error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    fn encoded(body: &[u8]) -> (Vec<u8>, String) {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(body).unwrap();
        (encoder.finish().unwrap(), hex::encode(Sha256::digest(body)))
    }

    #[test]
    fn preserves_original_bytes_and_rejects_corruption_truncation_and_trailing_streams() {
        let text = "{\"中文\":1.000000000001,\"id\":1152921504606846979}";
        let (compressed, hash) = encoded(text.as_bytes());
        assert_eq!(
            decode(&compressed, text.len() as u64, &hash, 1024).unwrap(),
            text
        );
        assert!(
            decode(
                &compressed[..compressed.len() - 1],
                text.len() as u64,
                &hash,
                1024
            )
            .is_err()
        );
        assert!(decode(&compressed, text.len() as u64, &"0".repeat(64), 1024).is_err());
        let mut extra = compressed.clone();
        extra.extend_from_slice(&compressed);
        assert!(decode(&extra, text.len() as u64, &hash, 1024).is_err());
        let (empty, hash) = encoded(b"");
        assert_eq!(decode(&empty, 0, &hash, 0).unwrap(), "");
    }

    #[test]
    fn allocation_and_decoded_output_are_bounded() {
        let body = vec![b'x'; 1024 * 1024];
        let (compressed, hash) = encoded(&body);
        assert!(decode(&compressed, body.len() as u64, &hash, 1024).is_err());
        assert!(decode(&compressed, 1024, &hash, 1024).is_err());
        assert_eq!(
            decode(&compressed, body.len() as u64, &hash, body.len() as u64)
                .unwrap()
                .len(),
            body.len()
        );
    }
}
