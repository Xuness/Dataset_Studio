use image::ImageReader;
use std::io::Cursor;
use studio_application::Media;
use studio_domain::*;

pub fn thumbnail(media: Media, edge: u32) -> Result<Media> {
    let mut reader = ImageReader::new(Cursor::new(&media.bytes))
        .with_guessed_format()
        .map_err(Error::io)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(40000);
    limits.max_image_height = Some(40000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| Error::new("MEDIA_DECODE_ERROR", e.to_string()))?
        .thumbnail(edge.clamp(96, 1600), edge.clamp(96, 1600));
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 86)
        .encode_image(&image)
        .map_err(Error::io)?;
    Ok(Media {
        bytes,
        content_type: "image/jpeg".into(),
    })
}
