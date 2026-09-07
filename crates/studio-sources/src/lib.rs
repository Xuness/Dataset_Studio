mod danbooru;
mod duckdb;
mod metadata;
pub use metadata::MetadataReader;
mod query;
pub use query::QueryReader;
pub mod duckdb_probe;
use image::{ImageEncoder, ImageReader};
use std::io::Cursor;
use studio_application::*;
use studio_domain::*;
pub const DEMO_ID: &str = "c0498544-0f82-4ce5-bd6d-bc6871059ca6";
pub struct SourceRouter;
fn demo_asset(source: &Source, n: usize) -> Asset {
    Asset {
        key: AssetKey {
            source_id: source.id.clone(),
            asset_id: format!("sample-{n:04}"),
        },
        name: format!("参考样本 {n:02}"),
        bytes: 0,
        extension: "png".into(),
        source_name: source.name.clone(),
    }
}
fn demo_number(id: &str) -> Result<usize> {
    id.strip_prefix("sample-")
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| (1..=32).contains(n))
        .ok_or_else(|| Error::new("NOT_FOUND", "示例对象不存在"))
}
impl SourceAdapter for SourceRouter {
    fn probe(&self, source: &Source) -> Result<SourceProbe> {
        match source.kind.as_str() {
            "demo" => Ok(SourceProbe {
                id: DEMO_ID.into(),
                revision: "demo-v1".into(),
                enumeration: "sample_assets".into(),
                count: Some(32),
                index_version: 1,
            }),
            "danbooru" => Ok(danbooru::Catalog::open(source)?.probe()),
            _ => Err(Error::invalid("未知的数据源适配器")),
        }
    }
    fn page(
        &self,
        source: &Source,
        after: Option<&str>,
        limit: usize,
        revision: Option<&str>,
    ) -> Result<AssetPage> {
        let limit = limit.clamp(1, 128);
        if source.kind == "danbooru" {
            return danbooru::Catalog::open(source)?.page(source, after, limit, revision);
        }
        if source.kind != "demo" {
            return Err(Error::invalid("未知的数据源适配器"));
        }
        if revision.is_some_and(|r| r != "demo-v1") {
            return Err(Error::new("SOURCE_CHANGED", "示例版本已变化"));
        }
        let start = after.map(demo_number).transpose()?.unwrap_or(0) + 1;
        let end = (start + limit - 1).min(32);
        Ok(AssetPage {
            items: (start..=end).map(|n| demo_asset(source, n)).collect(),
            next: if end < 32 {
                Some(format!("sample-{end:04}"))
            } else {
                None
            },
            revision: "demo-v1".into(),
        })
    }
    fn freeze(&self, source: &Source, keys: &[AssetKey]) -> Result<Vec<FrozenInput>> {
        if keys.iter().any(|k| k.source_id != source.id) {
            return Err(Error::invalid("输入对象与来源不匹配"));
        }
        if source.kind == "danbooru" {
            let catalog = danbooru::Catalog::open(source)?;
            return keys
                .iter()
                .map(|key| {
                    Ok(FrozenInput {
                        asset: catalog.asset(source, &key.asset_id)?,
                        source_revision: catalog.revision.clone(),
                        fields: Vec::new(),
                    })
                })
                .collect();
        }
        if source.kind != "demo" {
            return Err(Error::invalid("未知的数据源适配器"));
        }
        keys.iter()
            .map(|key| {
                Ok(FrozenInput {
                    asset: demo_asset(source, demo_number(&key.asset_id)?),
                    source_revision: "demo-v1".into(),
                    fields: Vec::new(),
                })
            })
            .collect()
    }
    fn read(&self, source: &Source, id: &str) -> Result<Media> {
        if source.kind == "danbooru" {
            return danbooru::Catalog::open(source)?.read(id);
        }
        if source.kind != "demo" {
            return Err(Error::invalid("未知的数据源适配器"));
        }
        let n = demo_number(id)?;
        let mut image = image::RgbImage::new(640, 800);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let band = (((x as f32 / 47.0 + (n as f32)).sin() * 22.0 + y as f32 / 9.0) as u32) % 80;
            *pixel = image::Rgb([
                ((34 + n * 13 + band as usize) % 165 + 40) as u8,
                ((47 + n * 7 + y as usize / 12) % 130 + 55) as u8,
                ((91 + n * 3 + x as usize / 11) % 105 + 75) as u8,
            ]);
        }
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(image.as_raw(), 640, 800, image::ExtendedColorType::Rgb8)
            .map_err(Error::io)?;
        Ok(Media {
            bytes,
            content_type: "image/png".into(),
        })
    }
}
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
