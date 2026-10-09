//! Pinterest facts are read independently at one retained serving sequence.
use crate::online::{Snapshot, sql_error};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use studio_application::ReadCancellation;
use studio_domain::*;

const FEATURES: [&str; 4] = [
    "pinterest-pins-v1",
    "raw-captures-v1",
    "receipt-replay-v1",
    "sha256-objects-v1",
];
pub(crate) fn validate_format(value: &serde_json::Value) -> Result<()> {
    let features = value["required_features"].as_array();
    if value["site"] != "pinterest"
        || value["schema_set"] != "pinterest-media-v1"
        || !features.is_some_and(|v| {
            v.len() == FEATURES.len()
                && FEATURES
                    .iter()
                    .all(|f| v.iter().any(|x| x.as_str() == Some(f)))
        })
    {
        return Err(Error::new(
            "SOURCE_FORMAT_UNSUPPORTED",
            "不支持该 Pinterest 事实格式或必需能力",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn pinterest_requires_its_complete_supported_feature_set() {
        let supported = json!({"site":"pinterest","schema_set":"pinterest-media-v1","required_features":FEATURES});
        validate_format(&supported).unwrap();
        for (key, value) in [
            ("site", json!("pixiv")),
            ("schema_set", json!("canonical-media-v2")),
            ("required_features", json!(["pinterest-pins-v1"])),
            (
                "required_features",
                json!([
                    "pinterest-pins-v1",
                    "raw-captures-v1",
                    "receipt-replay-v1",
                    "sha256-objects-v1",
                    "future-v2"
                ]),
            ),
        ] {
            let mut invalid = supported.clone();
            invalid[key] = value;
            assert_eq!(
                validate_format(&invalid).unwrap_err().code,
                "SOURCE_FORMAT_UNSUPPORTED"
            );
        }
    }
}
pub(crate) struct Read<'a> {
    source: &'a Source,
    snapshot: Snapshot,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    source: String,
    version: String,
    subject: String,
    after: String,
}
fn missing() -> Error {
    Error::new("NOT_FOUND", "Pin 来源记录不存在或不属于此对象")
}
fn sha(value: &str) -> Result<()> {
    if value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(Error::invalid("来源记录身份必须为 SHA-256"))
    }
}
fn field(name: &str, label: &str, value: Option<MetadataValue>, provenance: &str) -> MetadataField {
    MetadataField {
        name: name.into(),
        label: Some(label.into()),
        missing_reason: value.is_none().then(|| "not_reported".into()),
        value,
        provenance: provenance.into(),
        truncated: false,
    }
}
fn text(name: &str, label: &str, value: Option<String>, provenance: &str) -> MetadataField {
    let truncated = value.as_ref().is_some_and(|v| v.len() > 4096);
    let mut f = field(
        name,
        label,
        value.map(|v| {
            MetadataValue::Text(
                v.chars()
                    .scan(0, |bytes, c| {
                        *bytes += c.len_utf8();
                        (*bytes <= 4096).then_some(c)
                    })
                    .collect(),
            )
        }),
        provenance,
    );
    f.truncated = truncated;
    f
}
impl<'a> Read<'a> {
    pub fn open(
        source: &'a Source,
        version: Option<&str>,
        cancelled: ReadCancellation,
        deadline: Option<Instant>,
    ) -> Result<Self> {
        let snapshot = Snapshot::open(source, version, cancelled, deadline)?;
        if snapshot.pointer.schema_version != 4 {
            return Err(Error::new(
                "SOURCE_FORMAT_UNSUPPORTED",
                "需要 Pinterest 在线格式 4",
            ));
        }
        Ok(Self { source, snapshot })
    }
    fn version(&self) -> ReadVersion {
        ReadVersion {
            token: format!(
                "metadata-v4:{}:{}:{}",
                self.source.id, self.snapshot.pointer.generation, self.snapshot.sequence
            ),
            library_id: self.source.id.clone(),
            generation: self.snapshot.pointer.generation.clone(),
            catalog_sequence: self.snapshot.sequence.to_string(),
            analysis_sequence: self.snapshot.sequence.to_string(),
            consistency: "retained_online_snapshot".into(),
        }
    }
    fn after(&self, cursor: Option<&str>, subject: &str) -> Result<String> {
        let Some(cursor) = cursor else {
            return Ok(String::new());
        };
        if cursor.len() > 4096 {
            return Err(Error::invalid("Pinterest 游标过大"));
        }
        let c: Cursor = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(cursor)
                .map_err(|_| Error::invalid("Pinterest 游标无效"))?,
        )
        .map_err(|_| Error::invalid("Pinterest 游标无效"))?;
        if c.source != self.source.id || c.version != self.snapshot.revision || c.subject != subject
        {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "游标的来源、版本或筛选条件已变化",
            ));
        }
        Ok(c.after)
    }
    fn next(&self, subject: &str, after: &str) -> Result<String> {
        Ok(URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&Cursor {
                source: self.source.id.clone(),
                version: self.snapshot.revision.clone(),
                subject: subject.into(),
                after: after.into(),
            })
            .map_err(Error::io)?,
        ))
    }
    fn record(&self, asset: &str, record: &str) -> Result<AssetRecord> {
        sha(asset)?;
        sha(record)?;
        self.snapshot.db.query_row("SELECT a.asset_id,m.media_id,m.pin_id,m.manifest_id,m.role,f.observation_id
            FROM visible_assets a JOIN visible_media m USING(media_id) JOIN visible_manifests f USING(manifest_id)
            JOIN visible_objects o ON o.sha256=a.sha256 WHERE a.sha256=?1 AND a.asset_id=?2", params![asset, record], |r| {
            Ok(AssetRecord { record_id: r.get(0)?, origin_observation_id: r.get(5)?, post_id: r.get(2)?,
                source_md5: None, storage_profile: Some("original".into()), media_origin: None,
                pin_origin: Some(PinOrigin { pin_id: r.get(2)?, media_id: r.get(1)?, manifest_id: r.get(3)?, role: r.get(4)? }) })
        }).optional().map_err(sql_error)?.ok_or_else(missing)
    }
    pub fn metadata(&self, asset: &str, request: MetadataRequest) -> Result<MetadataOverview> {
        sha(asset)?;
        let subject = format!("records:{asset}");
        let after = self.after(request.cursor.as_deref(), &subject)?;
        let limit = request.limit.unwrap_or(50).clamp(1, 100);
        let mut stmt = self.snapshot.db.prepare("SELECT asset_id FROM visible_assets WHERE sha256=?1 AND asset_id>?2 ORDER BY asset_id LIMIT ?3").map_err(sql_error)?;
        let ids = stmt
            .query_map(params![asset, after, (limit + 1) as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        let next_cursor = if ids.len() > limit {
            Some(self.next(&subject, &ids[limit - 1])?)
        } else {
            None
        };
        let records = ids
            .iter()
            .take(limit)
            .map(|r| self.record(asset, r))
            .collect::<Result<_>>()?;
        let (bytes, extension, width, height): (i64,String,u32,u32) = self.snapshot.db.query_row(
            "SELECT length,stored_ext,stored_width,stored_height FROM visible_objects WHERE sha256=?1", [asset],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql_error)?.ok_or_else(missing)?;
        Ok(MetadataOverview {
            object: Asset {
                key: AssetKey {
                    source_id: self.source.id.clone(),
                    asset_id: asset.into(),
                },
                name: asset.into(),
                bytes: bytes as u64,
                extension,
                source_name: self.source.name.clone(),
            },
            stored_width: Some(width),
            stored_height: Some(height),
            dimensions_evidence: "validated_stored_bytes".into(),
            records,
            next_cursor,
            version: self.version(),
        })
    }
    pub fn summaries(&self, assets: &[String]) -> Result<Vec<AssetSummary>> {
        if assets.is_empty() || assets.len() > 128 {
            return Err(Error::invalid("身份摘要需要 1–128 个对象"));
        }
        let mut stmt = self.snapshot.db.prepare("SELECT DISTINCT m.pin_id FROM visible_assets a JOIN visible_media m USING(media_id)
            WHERE a.sha256=?1 ORDER BY length(m.pin_id),m.pin_id LIMIT 8").map_err(sql_error)?;
        assets.iter().map(|asset| {
            sha(asset)?;
            let post_ids = stmt.query_map([asset], |r| r.get(0)).map_err(sql_error)?.collect::<rusqlite::Result<_>>().map_err(sql_error)?;
            let post_count: i64 = self.snapshot.db.query_row("SELECT count(DISTINCT m.pin_id) FROM visible_assets a JOIN visible_media m USING(media_id) WHERE a.sha256=?1", [asset], |r| r.get(0)).map_err(sql_error)?;
            Ok(AssetSummary { asset_id:asset.clone(), post_ids,post_count: post_count as u64,version:self.version().token })
        }).collect()
    }
    fn pin_fields(&self, id: &str) -> Result<Vec<MetadataField>> {
        let mut fields = Vec::new();
        // JSON projection is bounded before crossing SQLite; a response may contain many unrelated fields.
        let mut query = self.snapshot.db.prepare("SELECT json_type(fields_json,?2),substr(CAST(json_extract(fields_json,?2) AS TEXT),1,4097)
            FROM visible_pins WHERE observation_id=?1").map_err(sql_error)?;
        for (key, label) in [
            ("title", "标题"),
            ("description", "描述"),
            ("image_signature", "来源图像签名"),
            ("link", "外部链接"),
            ("domain", "链接域名"),
            ("created_at", "来源创建时间"),
            ("is_ai_generated", "来源 AI 标记"),
            ("repin_count", "保存数"),
            ("comment_count", "评论数"),
            ("alt_text", "替代文字"),
        ] {
            let (kind, value): (Option<String>, Option<String>) = query
                .query_row(params![id, format!("$.{key}")], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .map_err(sql_error)?;
            let mut f = text(
                &format!("pinterest.{key}"),
                label,
                value.clone(),
                &format!("pin_observations.fields_json.{key}"),
            );
            match kind.as_deref() {
                None => f.missing_reason = Some("not_in_response".into()),
                Some("null") => f.missing_reason = Some("explicit_null".into()),
                Some("true") => f.value = Some(MetadataValue::Boolean(true)),
                Some("false") => f.value = Some(MetadataValue::Boolean(false)),
                Some("integer") => f.value = value.map(MetadataValue::Integer),
                _ => {}
            }
            fields.push(f);
        }
        let (capture,context,kind,field_set): (String,String,String,String) = self.snapshot.db.query_row(
            "SELECT p.capture_id,c.context_id,p.observation_kind,p.field_set FROM visible_pins p JOIN captures c USING(capture_id) WHERE p.observation_id=?1",
            [id],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(sql_error)?;
        for (name, label, value) in [
            ("capture_id", "响应记录", capture.clone()),
            ("context_id", "可见性上下文", context),
            ("observation_kind", "观察类型", kind),
            ("field_set", "请求字段集", field_set),
        ] {
            fields.push(text(
                &format!("pinterest.{name}"),
                label,
                Some(value),
                "pin_observations/captures",
            ));
        }
        let mut relations = self.snapshot.db.prepare("SELECT r.role,e.kind,substr(e.source_id,1,4097),
            substr(COALESCE(json_extract(o.fields_json,'$.name'),json_extract(o.fields_json,'$.full_name'),json_extract(o.fields_json,'$.username')),1,4097)
            FROM source_relations r JOIN source_entities e USING(entity_id)
            LEFT JOIN entity_observations o ON o.entity_id=r.entity_id AND o.capture_id=r.capture_id AND o.commit_seq<=?3
            WHERE r.capture_id=?1 AND r.pin_id=(SELECT pin_id FROM visible_pins WHERE observation_id=?2) AND r.commit_seq<=?3
            ORDER BY r.role,e.entity_id LIMIT 17").map_err(sql_error)?;
        let rows = relations
            .query_map(params![capture, id, self.snapshot.sequence as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        if rows.len() > 16 {
            return Err(Error::new("METADATA_LIMIT", "Pin 来源关系超过页面预算"));
        }
        for (role, kind, source_id, name) in rows {
            let label = match role.as_str() {
                "saved_to" => "所属画板",
                "pinner" => "保存账号",
                "origin_pinner" => "原始保存账号",
                "section" => "画板分区",
                _ => "来源关系",
            };
            fields.push(text(
                &format!("pinterest.{role}"),
                label,
                Some(format!(
                    "{kind} {source_id}{}",
                    name.map(|v| format!(" · {v}")).unwrap_or_default()
                )),
                "source_relations/entity_observations",
            ));
        }
        Ok(fields)
    }
    pub fn observations(
        &self,
        asset: &str,
        record_id: &str,
        request: MetadataRequest,
    ) -> Result<ObservationPage> {
        let record = self.record(asset, record_id)?;
        let origin = record.pin_origin.as_ref().expect("Pin origin");
        let subject = format!("observations:{asset}:{record_id}");
        let after = self.after(request.cursor.as_deref(), &subject)?;
        if request.cursor.is_some() && request.observation_id.is_some() {
            return Err(Error::invalid("指定观察不能同时分页"));
        }
        if let Some(id) = &request.observation_id {
            sha(id)?;
        }
        let limit = request.limit.unwrap_or(10).clamp(1, 10);
        let mut query=self.snapshot.db.prepare("SELECT key,id FROM (SELECT '0:'||media_id key,media_id id FROM visible_media WHERE media_id=?1
            UNION ALL SELECT '1:'||observation_id,observation_id FROM
              (SELECT observation_id FROM visible_pins WHERE pin_id=?2
               AND observation_id>CASE WHEN substr(?3,1,2)='1:' THEN substr(?3,3) ELSE '' END
               AND (?4 IS NULL OR observation_id=?4) ORDER BY observation_id LIMIT ?5))
            WHERE key>?3 AND (?4 IS NULL OR id=?4) ORDER BY key LIMIT ?5").map_err(sql_error)?;
        let ids = query
            .query_map(
                params![
                    origin.media_id,
                    origin.pin_id,
                    after,
                    request.observation_id,
                    (limit + 1) as i64
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        let mut items = Vec::new();
        for (_, id) in ids.iter().take(limit) {
            let (at, seq, fields, relation, kind) = if id == &origin.media_id {
                let (at,seq,width,height,url,path,signature,md5,etag): (String,i64,u32,u32,String,String,Option<String>,String,Option<String>) = self.snapshot.db.query_row(
                    "SELECT f.observed_at,m.commit_seq,m.width,m.height,substr(m.source_url,1,4097),m.field_path,substr(m.image_signature,1,4097),q.download_md5,substr(q.cdn_etag,1,4097)
                    FROM visible_media m JOIN visible_manifests f USING(manifest_id) JOIN visible_assets a USING(media_id)
                    JOIN visible_acquisitions q USING(acquisition_id) WHERE m.media_id=?1 AND a.asset_id=?2",params![id,record_id],
                    |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(sql_error)?;
                let fields = vec![
                    field(
                        "source_width",
                        "来源宽度",
                        Some(MetadataValue::Integer(width.to_string())),
                        "media_entries.width",
                    ),
                    field(
                        "source_height",
                        "来源高度",
                        Some(MetadataValue::Integer(height.to_string())),
                        "media_entries.height",
                    ),
                    text(
                        "pinterest.original_url",
                        "原图地址",
                        Some(url),
                        "media_entries.source_url",
                    ),
                    text(
                        "pinterest.field_path",
                        "原图字段路径",
                        Some(path),
                        "media_entries.field_path",
                    ),
                    text(
                        "pinterest.image_signature",
                        "来源图像签名",
                        signature,
                        "media_entries.image_signature",
                    ),
                    text(
                        "download.md5",
                        "下载内容 MD5",
                        Some(md5),
                        "acquisitions.download_md5",
                    ),
                    text("download.etag", "CDN ETag", etag, "acquisitions.cdn_etag"),
                ];
                (at, seq, fields, "asset_origin", "pinterest_media_manifest")
            } else {
                let (at,seq):(String,i64)=self.snapshot.db.query_row("SELECT observed_at,commit_seq FROM visible_pins WHERE observation_id=?1 AND pin_id=?2",params![id,origin.pin_id],|r| Ok((r.get(0)?,r.get(1)?))).map_err(sql_error)?;
                (
                    at,
                    seq,
                    self.pin_fields(id)?,
                    if record.origin_observation_id.as_ref() == Some(id) {
                        "asset_origin"
                    } else {
                        "same_pin"
                    },
                    "pinterest_pin_detail",
                )
            };
            items.push(Observation {
                observation_id: id.clone(),
                row_id: id.clone(),
                post_id: Some(origin.pin_id.clone()),
                relation: relation.into(),
                source_key: Some(origin.manifest_id.clone()),
                source_kind: Some(kind.into()),
                observed_at: Some(at),
                time_quality: Some("captured_at".into()),
                ingested_at: None,
                commit_sequence: Some(seq.to_string()),
                fields,
            });
        }
        Ok(ObservationPage {
            record_id: record_id.into(),
            items,
            next_cursor: if ids.len() > limit {
                Some(self.next(&subject, &ids[limit - 1].0)?)
            } else {
                None
            },
            version: self.version(),
        })
    }
    pub fn raw(&self, asset: &str, record_id: &str, observation_id: &str) -> Result<RawMetadata> {
        sha(observation_id)?;
        let record = self.record(asset, record_id)?;
        let origin = record.pin_origin.as_ref().expect("Pin origin");
        let capture: Option<String> = if observation_id == origin.media_id {
            self.snapshot
                .db
                .query_row(
                    "SELECT capture_id FROM visible_manifests WHERE manifest_id=?1",
                    [&origin.manifest_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql_error)?
        } else {
            self.snapshot
                .db
                .query_row(
                    "SELECT capture_id FROM visible_pins WHERE observation_id=?1 AND pin_id=?2",
                    params![observation_id, origin.pin_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql_error)?
        };
        let capture = capture.ok_or_else(missing)?;
        let (format,size,hash,body):(String,i64,String,Option<Vec<u8>>)=self.snapshot.db.query_row(
            "SELECT raw_format,raw_bytes,raw_sha256,CASE WHEN raw_bytes<=131072 THEN raw_zlib END FROM captures WHERE capture_id=?1 AND commit_seq<=?2",
            params![capture,self.snapshot.sequence as i64],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql_error)?.ok_or_else(missing)?;
        let json = body
            .map(|v| crate::online::raw::decode(&v, size as u64, &hash, 131072))
            .transpose()?;
        Ok(RawMetadata {
            observation_id: observation_id.into(),
            format: Some(format!("pinterest-{format}-response/v1")),
            schema_id: None,
            schema: None,
            bytes: Some(size.to_string()),
            status: if json.is_some() {
                "available"
            } else {
                "too_large"
            }
            .into(),
            json,
            version: self.version(),
        })
    }
}
