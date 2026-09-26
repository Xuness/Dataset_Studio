//! Storage-independent site semantics. Normalized lake values are never remapped here.
use studio_domain::*;

#[derive(Clone, Copy)]
pub struct SiteProfile {
    pub kind: &'static str,
    pub name: &'static str,
    pub normalizer: Option<&'static str>,
    pub absent_fields: &'static [&'static str],
}
pub const SITES: [SiteProfile; 3] = [
    SiteProfile {
        kind: "danbooru",
        name: "Danbooru",
        normalizer: None,
        absent_fields: &[],
    },
    SiteProfile {
        kind: "yandere",
        name: "Yandere",
        normalizer: Some("hf_yandere_v1"),
        absent_fields: &[
            "fav_count",
            "pixiv_id",
            "is_deleted",
            "is_banned",
            "is_pending",
            "is_flagged",
            "tag_string_general",
            "tag_string_artist",
            "tag_string_copyright",
            "tag_string_meta",
        ],
    },
    SiteProfile {
        kind: "gelbooru",
        name: "Gelbooru",
        normalizer: Some("hf_gelbooru_v1"),
        absent_fields: &[
            "fav_count",
            "pixiv_id",
            "is_deleted",
            "is_banned",
            "is_pending",
            "is_flagged",
            "tag_string_general",
            "tag_string_artist",
            "tag_string_copyright",
            "tag_string_meta",
            "file_ext",
            "file_size",
        ],
    },
];
pub fn site(kind: &str) -> Option<SiteProfile> {
    SITES.iter().copied().find(|s| s.kind == kind)
}
pub fn is_canonical(source: &Source) -> bool {
    site(&source.kind).is_some()
}
pub fn descriptor(kind: &str) -> Result<SourceDescriptor> {
    if kind == "demo" {
        return Ok(SourceDescriptor {
            version: 1,
            backend_id: "generated_demo_v1".into(),
            display_name: "内置参考资料".into(),
            site_id: None,
            semantics_version: "demo-v1".into(),
            capabilities: SourceCapabilities {
                browse: true,
                media: true,
                metadata: true,
                query: true,
                ..Default::default()
            },
            projections: vec!["origin_groups_v1".into()],
        });
    }
    let profile =
        site(kind).ok_or_else(|| Error::new("SOURCE_FORMAT_UNSUPPORTED", "未注册的数据源类型"))?;
    let mut projections = vec!["origin_width_v1".into(), "origin_groups_v1".into()];
    if profile.kind == "danbooru" {
        projections.extend(["danbooru_ranking_v1".into(), "danbooru_ranking_v2".into()]);
    }
    Ok(SourceDescriptor {
        version: 1,
        backend_id: "canonical_lake_v1".into(),
        display_name: profile.name.into(),
        site_id: Some(kind.into()),
        semantics_version: profile.normalizer.unwrap_or("danbooru-v1").into(),
        capabilities: SourceCapabilities {
            browse: true,
            media: true,
            metadata: true,
            query: true,
            post_order: true,
            relink: true,
            raw_metadata: true,
            incremental: true,
            stored_dimensions: profile.normalizer.is_some(),
        },
        projections,
    })
}

pub const METADATA_FIELDS: &[(&str, &str, &str)] = &[
    ("normalization_issues", "issues_json", "text"),
    ("rating", "rating", "text"),
    ("tags", "tag_string", "tags"),
    ("tags.general", "tag_string_general", "tags"),
    ("tags.artist", "tag_string_artist", "tags"),
    ("tags.character", "tag_string_character", "tags"),
    ("tags.copyright", "tag_string_copyright", "tags"),
    ("tags.meta", "tag_string_meta", "tags"),
    ("source_url", "source", "text"),
    ("source_width", "image_width", "integer"),
    ("source_height", "image_height", "integer"),
    ("source_bytes", "file_size", "integer"),
    ("source_extension", "file_ext", "text"),
    ("source_md5", "md5", "text"),
    ("created_at", "created_at", "timestamp"),
    ("updated_at", "updated_at", "timestamp"),
    ("danbooru.score", "score", "integer"),
    ("danbooru.fav_count", "fav_count", "integer"),
    ("danbooru.uploader_id", "uploader_id", "integer"),
    ("danbooru.parent_id", "parent_id", "integer"),
    ("danbooru.pixiv_id", "pixiv_id", "integer"),
    ("danbooru.is_deleted", "is_deleted", "boolean"),
    ("danbooru.is_banned", "is_banned", "boolean"),
    ("danbooru.is_pending", "is_pending", "boolean"),
    ("danbooru.is_flagged", "is_flagged", "boolean"),
];
pub fn field_label(column: &str) -> Option<&'static str> {
    Some(match column {
        "rating" => "分级",
        "tag_string" => "标签",
        "tag_string_general" => "一般",
        "tag_string_artist" => "画师",
        "tag_string_character" => "角色",
        "tag_string_copyright" => "作品",
        "tag_string_meta" => "元标签",
        "source" => "来源地址",
        "image_width" => "来源宽度",
        "image_height" => "来源高度",
        "file_size" => "来源文件大小",
        "file_ext" => "来源格式",
        "md5" => "来源 MD5",
        "created_at" => "创建时间",
        "updated_at" => "更新时间",
        "score" => "评分",
        "fav_count" => "收藏数",
        "uploader_id" => "上传者 ID",
        "parent_id" => "父条目 ID",
        "pixiv_id" => "Pixiv ID",
        "is_deleted" => "已删除",
        "is_banned" => "已屏蔽",
        "is_pending" => "待审核",
        "is_flagged" => "已标记",
        "issues_json" => "规范化说明",
        _ => return None,
    })
}
pub fn metadata_fields(source: &Source) -> Vec<(String, &'static str, &'static str)> {
    METADATA_FIELDS
        .iter()
        .filter(|(_, col, _)| field_supported(source, col))
        .map(|(name, col, ty)| {
            let name = if let Some(suffix) = name.strip_prefix("danbooru.") {
                format!("{}.{}", source.kind, suffix)
            } else {
                (*name).into()
            };
            (name, *col, *ty)
        })
        .collect()
}
pub fn field_supported(source: &Source, column: &str) -> bool {
    site(&source.kind).is_some_and(|profile| !profile.absent_fields.contains(&column))
}

/// The manifest is producer-owned provenance. Folder names never identify a site.
pub fn detected_site(root: &std::path::Path) -> Result<Option<String>> {
    use std::{
        collections::HashMap,
        sync::{Mutex, OnceLock},
        time::SystemTime,
    };
    type Cache = HashMap<std::path::PathBuf, (u64, SystemTime, String)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let root = root.canonicalize().map_err(Error::io)?;
    let path = root.join("source_manifests/hf-conversion-plan.json");
    let meta = match path.metadata() {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(e)),
    };
    if !meta.is_file() || meta.len() > 2 << 20 {
        return Err(Error::new(
            "SOURCE_FORMAT_ERROR",
            "来源清单不是有界 JSON 文件",
        ));
    }
    let path = path.canonicalize().map_err(Error::io)?;
    if !path.starts_with(&root) {
        return Err(Error::new("SOURCE_PATH_INVALID", "来源清单超出图片湖目录"));
    }
    let modified = meta.modified().map_err(Error::io)?;
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(entries) = cache.lock()
        && let Some((bytes, stamp, kind)) = entries.get(&path)
        && *bytes == meta.len()
        && *stamp == modified
    {
        return Ok(Some(kind.clone()));
    }
    #[derive(serde::Deserialize)]
    struct Manifest {
        site: String,
        normalizer: String,
    }
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(&path).map_err(Error::io)?)
        .map_err(|e| Error::new("SOURCE_FORMAT_ERROR", format!("来源清单无效：{e}")))?;
    let profile = site(&manifest.site)
        .ok_or_else(|| Error::new("SOURCE_FORMAT_UNSUPPORTED", "来源清单中的站点尚未注册"))?;
    if profile.normalizer != Some(manifest.normalizer.as_str()) {
        return Err(Error::new(
            "SOURCE_FORMAT_UNSUPPORTED",
            "来源规范化版本不受支持",
        ));
    }
    if let Ok(mut entries) = cache.lock() {
        if entries.len() >= 64 {
            entries.clear();
        }
        entries.insert(path, (meta.len(), modified, manifest.site.clone()));
    }
    Ok(Some(manifest.site))
}
pub fn validate_site(root: &std::path::Path, kind: &str) -> Result<()> {
    let profile = site(kind)
        .ok_or_else(|| Error::new("SOURCE_FORMAT_UNSUPPORTED", "未注册的 canonical 站点"))?;
    match detected_site(root)? {
        Some(actual) if actual != kind => Err(Error::new(
            "SOURCE_SITE_MISMATCH",
            format!("该数据湖来自 {actual}，与所选类型 {kind} 不符"),
        )),
        None if profile.normalizer.is_some() => Err(Error::new(
            "SOURCE_SITE_UNKNOWN",
            "缺少已转换数据湖的站点来源清单",
        )),
        _ => Ok(()),
    }
}
