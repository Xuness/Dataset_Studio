use crate::{
    danbooru::Catalog,
    demo_asset, demo_number,
    duckdb::{Runtime, Session},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use studio_application::MetadataAdapter;
use studio_domain::*;

#[derive(Default)]
pub struct MetadataReader {
    runtime: Runtime,
}
impl MetadataReader {
    pub fn new(dll: PathBuf) -> Self {
        Self {
            runtime: Runtime::new(dll),
        }
    }
    pub fn freeze_origin_width(
        &self,
        source: &Source,
        asset: &str,
        expected: &QuerySourceVersion,
    ) -> Result<FrozenField> {
        let session = ReadSession::open(self, source, asset, None)?;
        if expected.source_id != source.id
            || expected.catalog_revision != session.catalog.revision
            || expected.analysis_sequence.as_deref()
                != Some(session.version.analysis_sequence.as_str())
        {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "元数据已变化，不能替换提交时所需的字段版本",
            ));
        }
        let rows = session.db.query(&format!("SELECT a.asset_id,a.observation_id,CAST(o.image_width AS VARCHAR) FROM (SELECT asset_id,observation_id FROM assets WHERE sha256={} ORDER BY asset_id LIMIT 1) a LEFT JOIN observations o ON o.observation_id=a.observation_id LIMIT 1", quote(asset)))?;
        let record = rows.first().and_then(|r| r[0].clone());
        let observation = rows.first().and_then(|r| r[1].clone());
        let value = match rows.first().and_then(|r| r[2].as_deref()) {
            Some(text) => ScalarValue::integer(
                text.parse()
                    .map_err(|_| Error::new("FIELD_VALUE_INVALID", "来源宽度不是有效整数"))?,
            ),
            None => ScalarValue::Missing {
                reason: "first_linked_record_origin_width_absent".into(),
            },
        };
        session.finish(source)?;
        Ok(FrozenField {
            input: ScalarInput::OriginWidth,
            value,
            basis: FieldBasis {
                field_id: "source.origin.width".into(),
                subject: "asset".into(),
                rule: "lexicographically_first_linked_record_origin_observation".into(),
                source_version: Some(session.version.token),
                record_id: record,
                observation_id: observation,
                artifact_id: None,
            },
        })
    }
}
fn source_error(e: Error) -> Error {
    if e.code == "IO_ERROR" {
        Error::new(
            "SOURCE_UNAVAILABLE",
            format!("数据源路径不可用：{}", e.message),
        )
    } else {
        e
    }
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn sha(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(Error::invalid("需要小写 SHA-256 标识"));
    }
    Ok(())
}
fn required(row: &[Option<String>], i: usize) -> Result<String> {
    row.get(i)
        .cloned()
        .flatten()
        .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "索引缺少必要身份字段"))
}
struct ReadSession {
    catalog: Catalog,
    db: Session,
    version: ReadVersion,
}
impl ReadSession {
    fn open(
        reader: &MetadataReader,
        source: &Source,
        asset: &str,
        expected: Option<&str>,
    ) -> Result<Self> {
        if source.kind != "danbooru" {
            return Err(Error::new(
                "METADATA_UNSUPPORTED",
                "该来源尚未提供元数据检查",
            ));
        }
        sha(asset)?;
        let catalog = Catalog::open(source).map_err(source_error)?;
        catalog.asset(source, asset)?;
        let db = reader
            .runtime
            .open(&catalog.analysis_path().map_err(source_error)?)?;
        db.query("BEGIN TRANSACTION")?;
        let applied = db.query("SELECT CAST(MAX(seq) AS VARCHAR) FROM applied")?;
        let seq = applied
            .first()
            .and_then(|r| r[0].as_deref())
            .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "分析索引缺少水位"))?;
        if seq != catalog.sequence.to_string() {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "存储索引和分析索引水位不同，等待来源更新完成后重试",
            ));
        }
        let version = ReadVersion {
            token: format!("metadata-v1:{}:{}:{seq}", source.id, catalog.generation),
            library_id: source.id.clone(),
            generation: catalog.generation.clone(),
            catalog_sequence: catalog.sequence.to_string(),
            analysis_sequence: seq.into(),
            consistency: "request_transactions_matched_watermarks".into(),
        };
        if expected.is_some_and(|v| v != version.token) {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "元数据版本已变化，请重新加载当前对象",
            ));
        }
        Ok(Self {
            catalog,
            db,
            version,
        })
    }
    fn finish(&self, source: &Source) -> Result<()> {
        self.catalog.verify_unchanged(source).map_err(source_error)
    }
    fn record(&self, asset: &str, id: &str) -> Result<AssetRecord> {
        sha(id)?;
        let rows=self.db.query(&format!("SELECT asset_id,observation_id,CAST(post_id AS VARCHAR),source_md5,storage_profile FROM assets WHERE asset_id={} AND sha256={} LIMIT 2",quote(id),quote(asset)))?;
        if rows.len() != 1 {
            return Err(Error::new("NOT_FOUND", "该来源记录不属于当前存储对象"));
        }
        record(&rows[0])
    }
}
fn record(row: &[Option<String>]) -> Result<AssetRecord> {
    Ok(AssetRecord {
        record_id: required(row, 0)?,
        origin_observation_id: row[1].clone(),
        post_id: row[2].clone(),
        source_md5: row[3].clone(),
        storage_profile: row[4].clone(),
    })
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    source: String,
    asset: String,
    record: Option<String>,
    version: String,
    after: String,
}
fn after(
    request: &MetadataRequest,
    source: &Source,
    asset: &str,
    record: Option<&str>,
    version: &ReadVersion,
) -> Result<Option<String>> {
    let Some(cursor) = &request.cursor else {
        return Ok(None);
    };
    if cursor.len() > 4096 {
        return Err(Error::invalid("元数据游标过长"));
    }
    let cursor: Cursor = URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
        .ok_or_else(|| Error::invalid("元数据游标无效"))?;
    if cursor.source != source.id || cursor.asset != asset || cursor.record.as_deref() != record {
        return Err(Error::invalid("元数据游标不属于当前对象或来源记录"));
    }
    if cursor.version != version.token {
        return Err(Error::new(
            "SOURCE_CHANGED",
            "分页版本已变化，请重新读取元数据",
        ));
    }
    Ok(Some(cursor.after))
}
fn next(
    source: &Source,
    asset: &str,
    record: Option<&str>,
    version: &ReadVersion,
    after: String,
) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Cursor {
            source: source.id.clone(),
            asset: asset.into(),
            record: record.map(Into::into),
            version: version.token.clone(),
            after,
        })
        .map_err(Error::io)?,
    ))
}
// Public common semantics and namespaced source fields. Never infer stored dimensions
// from observations.image_width/image_height. Large strings are bounded in SQL.
const FIELDS: &[(&str, &str, &str)] = &[
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
fn field(name: &str, column: &str, kind: &str, value: &Option<String>) -> Result<MetadataField> {
    let truncated = value.as_ref().is_some_and(|v| v.chars().count() > 8192);
    let value = value
        .as_ref()
        .map(|v| v.chars().take(8192).collect::<String>());
    let missing = value.is_none();
    let value = value
        .map(|v| {
            Ok(match kind {
                "integer" => {
                    v.parse::<i64>()
                        .map_err(|_| Error::new("SOURCE_FORMAT_ERROR", "整数元数据无效"))?;
                    MetadataValue::Integer(v)
                }
                "boolean" => MetadataValue::Boolean(
                    v.parse::<bool>()
                        .map_err(|_| Error::new("SOURCE_FORMAT_ERROR", "布尔元数据无效"))?,
                ),
                "tags" => MetadataValue::Tags(v.split_whitespace().map(Into::into).collect()),
                "timestamp" => MetadataValue::Timestamp(v),
                _ => MetadataValue::Text(v),
            })
        })
        .transpose()?;
    Ok(MetadataField {
        name: name.into(),
        value,
        provenance: format!("observations.{column}"),
        missing_reason: missing.then(|| "not_recorded".into()),
        truncated,
    })
}
fn observation_predicate(record: &AssetRecord) -> String {
    if let Some(post) = &record.post_id {
        format!("post_id={}", quote(post))
    } else if let Some(origin) = &record.origin_observation_id {
        format!("observation_id={}", quote(origin))
    } else {
        "FALSE".into()
    }
}
impl MetadataAdapter for MetadataReader {
    fn summaries(
        &self,
        source: &Source,
        asset_ids: &[String],
        cancelled: studio_application::ReadCancellation,
    ) -> Result<Vec<AssetSummary>> {
        if source.kind != "danbooru" {
            return Err(Error::new("METADATA_UNSUPPORTED", "该来源没有帖子身份摘要"));
        }
        if asset_ids.is_empty() || asset_ids.len() > 128 {
            return Err(Error::invalid("身份摘要需要 1–128 个对象"));
        }
        for id in asset_ids {
            sha(id)?;
        }
        let catalog = Catalog::open(source).map_err(source_error)?;
        let db = self
            .runtime
            .open_metadata(&catalog.analysis_path()?, cancelled)?;
        db.query("BEGIN TRANSACTION")?;
        let watermarks = db.query("SELECT CAST(MAX(seq) AS VARCHAR) FROM applied")?;
        let sequence = watermarks
            .first()
            .and_then(|r| r[0].as_deref())
            .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "分析索引缺少水位"))?;
        if sequence != catalog.sequence.to_string() {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "存储与分析索引版本不同，请稍后读取身份",
            ));
        }
        let version = format!(
            "metadata-v1:{}:{}:{sequence}",
            source.id, catalog.generation
        );
        let ids = asset_ids
            .iter()
            .map(|id| quote(id))
            .collect::<Vec<_>>()
            .join(",");
        // Filter identities before aggregation; output is at most eight posts per
        // requested image. The count remains exact when more links are present.
        let sql = format!(
            "WITH linked AS (SELECT sha256,post_id FROM assets WHERE sha256 IN ({ids}) AND post_id IS NOT NULL GROUP BY sha256,post_id), ranked AS (SELECT sha256,post_id,row_number() OVER (PARTITION BY sha256 ORDER BY post_id) AS ordinal,count(*) OVER (PARTITION BY sha256) AS linked_count FROM linked) SELECT sha256,CAST(post_id AS VARCHAR),CAST(linked_count AS VARCHAR) FROM ranked WHERE ordinal<=8 ORDER BY sha256,post_id LIMIT 1024"
        );
        let rows = db.query_bounded(&sql, 1024)?;
        let mut summaries = asset_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    AssetSummary {
                        asset_id: id.clone(),
                        post_ids: Vec::new(),
                        post_count: 0,
                        version: version.clone(),
                    },
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        for row in rows {
            let id = required(&row, 0)?;
            let summary = summaries
                .get_mut(&id)
                .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "身份摘要超出请求范围"))?;
            summary.post_ids.push(required(&row, 1)?);
            summary.post_count = required(&row, 2)?
                .parse()
                .map_err(|_| Error::new("SOURCE_FORMAT_ERROR", "帖子关联数量无效"))?;
        }
        catalog.verify_unchanged(source)?;
        Ok(summaries.into_values().collect())
    }
    fn metadata(
        &self,
        source: &Source,
        asset: &str,
        request: MetadataRequest,
    ) -> Result<MetadataOverview> {
        if source.kind == "demo" {
            let n = demo_number(asset)?;
            let version = demo_version(source, &request)?;
            if request.cursor.is_some() {
                return Err(Error::invalid("示例元数据没有后续页"));
            }
            return Ok(MetadataOverview {
                object: demo_asset(source, n),
                stored_width: Some(640),
                stored_height: Some(800),
                dimensions_evidence: "demo_generator".into(),
                records: vec![AssetRecord {
                    record_id: asset.into(),
                    origin_observation_id: Some(asset.into()),
                    post_id: None,
                    source_md5: None,
                    storage_profile: Some("generated-demo".into()),
                }],
                next_cursor: None,
                version,
            });
        }
        let session = ReadSession::open(self, source, asset, request.version.as_deref())?;
        let limit = request.limit.unwrap_or(20).clamp(1, 50);
        let after = after(&request, source, asset, None, &session.version)?;
        if let Some(value) = &after {
            sha(value)?;
        }
        let rows=session.db.query(&format!("SELECT asset_id,observation_id,CAST(post_id AS VARCHAR),source_md5,storage_profile FROM assets WHERE sha256={} AND asset_id>{} ORDER BY asset_id LIMIT {}",quote(asset),quote(after.as_deref().unwrap_or("")),limit+1))?;
        let more = rows.len() > limit;
        let records = rows
            .iter()
            .take(limit)
            .map(|r| record(r))
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            Some(next(
                source,
                asset,
                None,
                &session.version,
                records.last().expect("nonempty page").record_id.clone(),
            )?)
        } else {
            None
        };
        let object = session.catalog.asset(source, asset)?;
        session.finish(source)?;
        Ok(MetadataOverview {
            object,
            stored_width: None,
            stored_height: None,
            dimensions_evidence: "not_inspected".into(),
            records,
            next_cursor,
            version: session.version,
        })
    }
    fn observations(
        &self,
        source: &Source,
        asset: &str,
        record_id: &str,
        request: MetadataRequest,
    ) -> Result<ObservationPage> {
        if source.kind == "demo" {
            let n = demo_number(asset)?;
            let version = demo_version(source, &request)?;
            if record_id != asset {
                return Err(Error::new("NOT_FOUND", "示例记录不属于当前对象"));
            }
            if request.cursor.is_some() {
                return Err(Error::invalid("示例观察没有后续页"));
            }
            return Ok(ObservationPage {
                record_id: record_id.into(),
                items: vec![Observation {
                    observation_id: asset.into(),
                    row_id: n.to_string(),
                    post_id: None,
                    relation: "asset_origin".into(),
                    source_key: Some("studio-demo-generator".into()),
                    source_kind: Some("generated_demo".into()),
                    observed_at: None,
                    time_quality: Some("not_observed".into()),
                    ingested_at: None,
                    commit_sequence: None,
                    fields: vec![MetadataField {
                        name: "demo.sample_number".into(),
                        value: Some(MetadataValue::Integer(n.to_string())),
                        provenance: "studio.demo_generator".into(),
                        missing_reason: None,
                        truncated: false,
                    }],
                }],
                next_cursor: None,
                version,
            });
        }
        let session = ReadSession::open(self, source, asset, request.version.as_deref())?;
        let record = session.record(asset, record_id)?;
        let limit = request.limit.unwrap_or(10).clamp(1, 10);
        let after = after(&request, source, asset, Some(record_id), &session.version)?;
        let after = after
            .as_deref()
            .unwrap_or("0")
            .parse::<u64>()
            .map_err(|_| Error::invalid("观察游标无效"))?;
        if after > i64::MAX as u64 {
            return Err(Error::invalid("观察游标超出范围"));
        }
        let projection = FIELDS
            .iter()
            .map(|(_, column, _)| format!("left(CAST({column} AS VARCHAR),8193)"))
            .collect::<Vec<_>>()
            .join(",");
        let rows=session.db.query(&format!("SELECT observation_id,CAST(row_id AS VARCHAR),CAST(post_id AS VARCHAR),source_key,source_kind,CAST(observed_at AS VARCHAR),time_quality,CAST(ingested_at AS VARCHAR),CAST(commit_seq AS VARCHAR),{projection} FROM observations WHERE ({}) AND row_id>{after} ORDER BY row_id LIMIT {}",observation_predicate(&record),limit+1))?;
        let more = rows.len() > limit;
        let items = rows
            .iter()
            .take(limit)
            .map(|r| {
                let id = required(r, 0)?;
                Ok(Observation {
                    relation: if record.origin_observation_id.as_deref() == Some(&id) {
                        "asset_origin"
                    } else {
                        "same_post"
                    }
                    .into(),
                    observation_id: id,
                    row_id: required(r, 1)?,
                    post_id: r[2].clone(),
                    source_key: r[3].clone(),
                    source_kind: r[4].clone(),
                    observed_at: r[5].clone(),
                    time_quality: r[6].clone(),
                    ingested_at: r[7].clone(),
                    commit_sequence: r[8].clone(),
                    fields: FIELDS
                        .iter()
                        .enumerate()
                        .map(|(i, (name, column, kind))| field(name, column, kind, &r[i + 9]))
                        .collect::<Result<_>>()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            Some(next(
                source,
                asset,
                Some(record_id),
                &session.version,
                items.last().expect("nonempty page").row_id.clone(),
            )?)
        } else {
            None
        };
        session.finish(source)?;
        Ok(ObservationPage {
            record_id: record_id.into(),
            items,
            next_cursor,
            version: session.version,
        })
    }
    fn raw_metadata(
        &self,
        source: &Source,
        asset: &str,
        record_id: &str,
        observation_id: &str,
        version: &str,
    ) -> Result<RawMetadata> {
        if source.kind == "demo" {
            demo_number(asset)?;
            let version = demo_version(
                source,
                &MetadataRequest {
                    version: Some(version.into()),
                    ..Default::default()
                },
            )?;
            if record_id != asset || observation_id != asset {
                return Err(Error::new("NOT_FOUND", "示例观察不存在"));
            }
            return Ok(RawMetadata {
                observation_id: observation_id.into(),
                format: None,
                schema_id: None,
                json: None,
                bytes: None,
                status: "missing".into(),
                version,
            });
        }
        sha(observation_id)?;
        let session = ReadSession::open(self, source, asset, Some(version))?;
        let record = session.record(asset, record_id)?;
        if session
            .db
            .query(&format!(
                "SELECT observation_id FROM observations WHERE observation_id={} AND ({}) LIMIT 1",
                quote(observation_id),
                observation_predicate(&record)
            ))?
            .is_empty()
        {
            return Err(Error::new("NOT_FOUND", "该观察不属于当前来源记录"));
        }
        let rows=session.db.query(&format!("SELECT source_metadata_format,source_schema_id,CAST(octet_length(encode(source_metadata_json)) AS VARCHAR),CASE WHEN octet_length(encode(source_metadata_json))<=131072 THEN source_metadata_json ELSE NULL END FROM raw_metadata WHERE observation_id={} LIMIT 1",quote(observation_id)))?;
        let (format, schema_id, bytes, json, status) = if let Some(row) = rows.first() {
            (
                row[0].clone(),
                row[1].clone(),
                row[2].clone(),
                row[3].clone(),
                if row[2].is_none() {
                    "missing"
                } else if row[3].is_none() {
                    "too_large"
                } else {
                    "available"
                },
            )
        } else {
            (None, None, None, None, "missing")
        };
        session.finish(source)?;
        Ok(RawMetadata {
            observation_id: observation_id.into(),
            format,
            schema_id,
            bytes,
            json,
            status: status.into(),
            version: session.version,
        })
    }
}
fn demo_version(source: &Source, request: &MetadataRequest) -> Result<ReadVersion> {
    if request
        .version
        .as_deref()
        .is_some_and(|v| v != "demo-metadata-v1")
    {
        return Err(Error::new("SOURCE_CHANGED", "示例元数据版本已变化"));
    }
    Ok(ReadVersion {
        token: "demo-metadata-v1".into(),
        library_id: source.id.clone(),
        generation: "demo".into(),
        catalog_sequence: "1".into(),
        analysis_sequence: "1".into(),
        consistency: "deterministic_demo".into(),
    })
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
