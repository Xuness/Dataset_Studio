use super::*;
use image::{ImageBuffer, Rgb, Rgba};

fn png(width: u32, height: u32, alpha: bool, color: u8) -> Media {
    let image = if alpha {
        DynamicImage::ImageRgba8(ImageBuffer::from_fn(width, height, |_, y| {
            Rgba([color, 80, 120, if y < height / 2 { 0 } else { 255 }])
        }))
    } else {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(width, height, |x, y| {
            Rgb([color, (x % 256) as u8, (y % 256) as u8])
        }))
    };
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    Media {
        bytes: bytes.into_inner(),
        content_type: "image/png".into(),
    }
}
fn prepare(media: Media, edge: Option<u32>, cache: Option<&PreviewCache>) -> PreparedImageInput {
    let hash = hex::encode(Sha256::digest(&media.bytes));
    prepare_image_input(media, &hash, edge, cache).unwrap()
}

#[test]
fn image_input_keeps_aspect_ratio_and_records_the_actual_encoded_bytes() {
    for (width, height, expected) in [
        (512, 384, (256, 192)),
        (384, 512, (192, 256)),
        (1024, 128, (256, 32)),
    ] {
        let output = prepare(png(width, height, false, 200), Some(256), None);
        let decoded = image::load_from_memory(&output.media.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), expected);
        assert_eq!(
            (output.info.width, output.info.height),
            (Some(expected.0), Some(expected.1))
        );
        assert_eq!(
            (output.info.source_width, output.info.source_height),
            (Some(width), Some(height))
        );
        assert_eq!(output.media.content_type, "image/jpeg");
        assert_eq!(output.info.bytes, output.media.bytes.len() as u64);
        assert_eq!(
            output.info.sha256,
            hex::encode(Sha256::digest(&output.media.bytes))
        );
        assert_ne!(output.info.source_sha256, output.info.sha256);
        assert_eq!(output.info.max_edge, Some(256));
        // High-quality encoding retains a known color channel, independent of file size.
        assert!(
            (i16::from(decoded.to_rgb8().get_pixel(expected.0 / 2, expected.1 / 2)[0]) - 200).abs()
                < 5
        );
    }
}

#[test]
fn image_input_original_and_no_upscale_preserve_exact_bytes() {
    for edge in [None, Some(512)] {
        let media = png(300, 200, false, 80);
        let expected = media.bytes.clone();
        let output = prepare(media, edge, None);
        assert_eq!(output.media.bytes, expected);
        assert_eq!(output.info.sha256, output.info.source_sha256);
        assert_eq!(output.info.transform_version, "stored_original_v1");
    }
}

#[test]
fn image_input_preserves_transparency_with_lossless_png() {
    let output = prepare(png(256, 512, true, 200), Some(128), None);
    assert_eq!(output.media.content_type, "image/png");
    let decoded = image::load_from_memory(&output.media.bytes)
        .unwrap()
        .to_rgba8();
    assert_eq!(decoded.dimensions(), (64, 128));
    assert_eq!(decoded.get_pixel(0, 0)[3], 0);
    assert_eq!(decoded.get_pixel(32, 127)[3], 255);
    assert_eq!(decoded.get_pixel(32, 127)[0], 200);
}

#[test]
fn image_input_applies_exif_orientation_before_resizing() {
    let pixels = ImageBuffer::from_pixel(512, 256, Rgb([200_u8, 80, 120]));
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95);
    encoder
        .set_exif_metadata(
            b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec(),
        )
        .unwrap();
    encoder.encode_image(&pixels).unwrap();
    let output = prepare(
        Media {
            bytes,
            content_type: "image/jpeg".into(),
        },
        Some(128),
        None,
    );
    let decoded = image::load_from_memory(&output.media.bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (64, 128));
    assert_eq!(
        (output.info.source_width, output.info.source_height),
        (Some(512), Some(256))
    );
}

#[test]
fn image_input_cache_is_separated_by_source_and_requested_size() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("api-image-cache-")
        .tempdir_in(root)
        .unwrap();
    let cache = PreviewCache::open(dir.path()).unwrap();
    let first = prepare(png(512, 384, false, 80), Some(256), Some(&cache));
    let again = prepare(png(512, 384, false, 80), Some(256), Some(&cache));
    assert_eq!(first.media.bytes, again.media.bytes);
    assert_eq!(first.info, again.info);
    assert_eq!(cache.metrics().hits, 1);
    assert_eq!(cache.metrics().writes, 1);
    let smaller = prepare(png(512, 384, false, 80), Some(128), Some(&cache));
    let changed = prepare(png(512, 384, false, 150), Some(256), Some(&cache));
    assert_ne!(first.info.sha256, smaller.info.sha256);
    assert_ne!(first.info.sha256, changed.info.sha256);
    assert_eq!(cache.metrics().writes, 3);
}

#[test]
fn image_input_rejects_invalid_settings_and_undecodable_transforms() {
    let media = png(256, 128, false, 20);
    let hash = hex::encode(Sha256::digest(&media.bytes));
    assert!(prepare_image_input(media, &hash, Some(0), None).is_err());
    assert!(
        prepare_image_input(
            Media {
                bytes: b"broken".to_vec(),
                content_type: "image/png".into()
            },
            &hash,
            Some(128),
            None
        )
        .is_err()
    );
}
