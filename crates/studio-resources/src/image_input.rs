//! API image preparation uses its own cache namespace and never edits lake objects.
use crate::PreviewCache;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageReader, imageops::FilterType};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use studio_application::Media;
use studio_domain::{Error, ImageInputInfo, Result, validate_image_max_edge};

/// One image is decoded at a time. Covers source, orientation, Lanczos float scratch,
/// destination and encoding buffers under the pixel/decoder limits below.
pub const IMAGE_INPUT_WORKSPACE_BYTES: u64 = 256 << 20;
const MAX_PIXELS: u64 = 8 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 64 << 20;
const MAX_ENCODE_BYTES: usize = 16 << 20;
const RECIPE: &str = "api-image-image02510-lanczos3-jpeg95-png-exif-icc-v1";
const CACHE_MAGIC: &[u8; 8] = b"APIIMG01";

pub struct PreparedImageInput {
    pub media: Media,
    pub info: ImageInputInfo,
}

fn reader(bytes: &[u8]) -> Result<ImageReader<Cursor<&[u8]>>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(Error::io)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(40000);
    limits.max_image_height = Some(40000);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    Ok(reader)
}

fn decode_error(e: impl std::fmt::Display) -> Error {
    Error::new("MEDIA_DECODE_ERROR", e.to_string())
}

fn original_info(media: &Media, source_sha256: &str, max_edge: Option<u32>) -> ImageInputInfo {
    // Reading dimensions does not decode pixels. Unknown headers retain the original
    // pass-through behavior; requesting a transform requires a decodable image.
    let dimensions = reader(&media.bytes)
        .and_then(|r| r.into_dimensions().map_err(decode_error))
        .ok();
    ImageInputInfo {
        source_sha256: source_sha256.into(),
        sha256: source_sha256.into(),
        source_width: dimensions.map(|v| v.0),
        source_height: dimensions.map(|v| v.1),
        width: dimensions.map(|v| v.0),
        height: dimensions.map(|v| v.1),
        source_bytes: media.bytes.len() as u64,
        bytes: media.bytes.len() as u64,
        content_type: media.content_type.clone(),
        max_edge,
        transform_version: "stored_original_v1".into(),
    }
}

/// The caller verifies the source hash before this boundary, including on cache hits.
/// No provider-specific parameters are needed: every protocol receives the same bytes.
pub fn prepare_image_input(
    media: Media,
    source_sha256: &str,
    max_edge: Option<u32>,
    cache: Option<&PreviewCache>,
) -> Result<PreparedImageInput> {
    validate_image_max_edge(max_edge)?;
    let mut info = original_info(&media, source_sha256, max_edge);
    let Some(edge) = max_edge else {
        return Ok(PreparedImageInput { media, info });
    };
    if let (Some(w), Some(h)) = (info.width, info.height)
        && w.max(h) <= edge
    {
        // Also avoids recompression when a small image already meets the requested cap.
        return Ok(PreparedImageInput { media, info });
    }
    let key = hex::encode(Sha256::digest(format!("{RECIPE}:{source_sha256}:{edge}")));
    if let Some(cache) = cache
        && let Ok(Some(saved)) = cache.get(&key, true)
        && let Some(prepared) = unpack(&saved.bytes, source_sha256, edge)
    {
        return Ok(prepared);
    }
    let mut decoder = reader(&media.bytes)?.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS || decoder.total_bytes() > MAX_DECODE_BYTES
    {
        return Err(Error::new(
            "MEDIA_DECODE_LIMIT",
            "API 缩图源图超过 8388608 像素或 64 MiB 解码预算",
        ));
    }
    info.source_width = Some(width);
    info.source_height = Some(height);
    let orientation = decoder.orientation().map_err(decode_error)?;
    let profile = decoder.icc_profile().map_err(decode_error)?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    decoded.apply_orientation(orientation);
    let resized = decoded.resize(edge, edge, FilterType::Lanczos3);
    info.width = Some(resized.width());
    info.height = Some(resized.height());
    let mut output = BoundedBuffer(Vec::new());
    if resized.color().has_alpha() {
        let mut encoder = image::codecs::png::PngEncoder::new_with_quality(
            &mut output,
            image::codecs::png::CompressionType::Default,
            image::codecs::png::FilterType::Adaptive,
        );
        if let Some(profile) = profile {
            encoder.set_icc_profile(profile).map_err(decode_error)?;
        }
        encoder
            .write_image(
                resized.as_bytes(),
                resized.width(),
                resized.height(),
                resized.color().into(),
            )
            .map_err(decode_error)?;
        info.content_type = "image/png".into();
    } else {
        let rgb = resized.to_rgb8();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 95);
        if let Some(profile) = profile {
            encoder.set_icc_profile(profile).map_err(decode_error)?;
        }
        encoder.encode_image(&rgb).map_err(decode_error)?;
        info.content_type = "image/jpeg".into();
    }
    info.bytes = output.0.len() as u64;
    info.sha256 = hex::encode(Sha256::digest(&output.0));
    info.transform_version = RECIPE.into();
    let prepared = PreparedImageInput {
        media: Media {
            bytes: output.0,
            content_type: info.content_type.clone(),
        },
        info,
    };
    // Shares the existing bounded/rebuildable image cache and its clear/quota controls.
    // Cache errors never prevent preparing a request from its verified lake source.
    if let Some(cache) = cache
        && let Ok(header) = serde_json::to_vec(&prepared.info)
    {
        let mut packed = Vec::with_capacity(12 + header.len() + prepared.media.bytes.len());
        packed.extend_from_slice(CACHE_MAGIC);
        packed.extend_from_slice(&(header.len() as u32).to_le_bytes());
        packed.extend_from_slice(&header);
        packed.extend_from_slice(&prepared.media.bytes);
        let _ = cache.put(&key, &packed);
    }
    Ok(prepared)
}

fn unpack(bytes: &[u8], source_sha256: &str, edge: u32) -> Option<PreparedImageInput> {
    if bytes.get(..8)? != CACHE_MAGIC {
        return None;
    }
    let len = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?) as usize;
    if len > 16384 {
        return None;
    }
    let info: ImageInputInfo = serde_json::from_slice(bytes.get(12..12 + len)?).ok()?;
    let data = bytes.get(12 + len..)?;
    if info.source_sha256 != source_sha256
        || info.max_edge != Some(edge)
        || info.transform_version != RECIPE
        || info.bytes != data.len() as u64
        || info.sha256 != hex::encode(Sha256::digest(data))
    {
        return None;
    }
    Some(PreparedImageInput {
        media: Media {
            bytes: data.to_vec(),
            content_type: info.content_type.clone(),
        },
        info,
    })
}

struct BoundedBuffer(Vec<u8>);
impl Write for BoundedBuffer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() > MAX_ENCODE_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("API 缩图编码超过 16 MiB 上限"));
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
