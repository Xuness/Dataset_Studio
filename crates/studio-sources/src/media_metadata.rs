//! Native metadata reader for canonical-media-v2. Every relation uses one leased snapshot.
use crate::online::{Snapshot, sql_error};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use studio_application::ReadCancellation;
use studio_domain::*;

pub(crate) struct Read<'a> {
    pub source: &'a Source,
    pub snapshot: Snapshot,
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
    Error::new("NOT_FOUND", "来源记录不存在或不属于此对象")
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
pub(crate) fn source_id(value: &str) -> Result<()> {
    if !value.is_empty()
        && value.len() <= 20
        && !value.starts_with('0')
        && value.bytes().all(|b| b.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(Error::invalid("作品或作者 ID 必须为十进制字符串"))
    }
}
fn field(name: &str, value: Option<MetadataValue>, provenance: &str) -> MetadataField {
    MetadataField {
        name: name.into(),
        label: None,
        missing_reason: value.is_none().then(|| "not_reported".into()),
        value,
        provenance: provenance.into(),
        truncated: false,
    }
}
fn text_field(name: &str, value: Option<String>, provenance: &str) -> MetadataField {
    let truncated = value.as_ref().is_some_and(|v| v.len() > 8192);
    let value = value.map(|v| {
        v.chars()
            .scan(0, |bytes, c| {
                *bytes += c.len_utf8();
                (*bytes <= 8192).then_some(c)
            })
            .collect()
    });
    let mut f = field(name, value.map(MetadataValue::Text), provenance);
    f.truncated = truncated;
    f
}
impl<'a> Read<'a> {
    fn object_row(
        &self,
        sql: &str,
        values: &[&dyn rusqlite::ToSql],
    ) -> Result<Option<serde_json::Value>> {
        let mut stmt = self.snapshot.db.prepare(sql).map_err(sql_error)?;
        let columns = stmt
            .column_names()
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>();
        stmt.query_row(values, |row| {
            let mut value = serde_json::Map::new();
            for (i, name) in columns.iter().enumerate() {
                let field = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => serde_json::json!(n),
                    rusqlite::types::ValueRef::Text(v) => {
                        serde_json::Value::String(String::from_utf8_lossy(v).into_owned())
                    }
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                value.insert(name.clone(), field);
            }
            Ok(serde_json::Value::Object(value))
        })
        .optional()
        .map_err(sql_error)
    }
    pub fn relation(&self, request: SourceRelationRequest) -> Result<serde_json::Value> {
        use serde_json::json;
        source_id(&request.id)?;
        let limit = request.limit.unwrap_or(50);
        if !(1..=200).contains(&limit) {
            return Err(Error::invalid("来源成员页需要 1–200 条"));
        }
        let sequence = self.snapshot.sequence as i64;
        let result = match request.kind {
            SourceRelationKind::Work => {
                let current=self.object_row("SELECT work_id,observation_id,manifest_id,manifest_state FROM current_works WHERE work_id=?1", &[&request.id])?.ok_or_else(missing)?;
                let observation = if let Some(id) = current["observation_id"].as_str() {
                    self.object_row("SELECT observation_id,work_id,capture_id,observed_at,author_id,work_type,title,caption_html,page_count,created_at,updated_at,source_fields_json FROM visible_works WHERE observation_id=?1", &[&id])?
                } else {
                    None
                };
                let manifest = if let Some(id) = current["manifest_id"].as_str() {
                    self.object_row("SELECT manifest_id,work_id,capture_id,detail_observation_id,context_id,observed_at,kind,expected_count,item_count,complete,reason FROM visible_manifests WHERE manifest_id=?1", &[&id])?
                } else {
                    None
                };
                json!({"work_id":request.id,"observation":observation,"manifest":manifest,"manifest_state":current["manifest_state"],"version":self.snapshot.revision})
            }
            SourceRelationKind::WorkMedia => {
                let current = self
                    .object_row(
                        "SELECT manifest_id,manifest_state FROM current_works WHERE work_id=?1",
                        &[&request.id],
                    )?
                    .ok_or_else(missing)?;
                let mid = request
                    .manifest_id
                    .as_deref()
                    .or_else(|| current["manifest_id"].as_str());
                if let Some(mid) = mid {
                    sha(mid)?;
                    let manifest=self.object_row("SELECT context_id,observed_at FROM visible_manifests WHERE manifest_id=?1 AND work_id=?2", &[&mid,&request.id])?.ok_or_else(missing)?;
                    let selector = format!(
                        "work-media:{}:{}:{}",
                        request.id,
                        mid,
                        request.recipe_id.as_deref().unwrap_or("")
                    );
                    let after = self
                        .after(request.cursor.as_deref(), &selector, "-1")?
                        .parse::<i64>()
                        .map_err(|_| Error::invalid("媒体位置游标无效"))?;
                    let mut stmt=self.snapshot.db.prepare("SELECT media_id FROM visible_media WHERE manifest_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT ?3").map_err(sql_error)?;
                    let ids = stmt
                        .query_map(params![mid, after, (limit + 1) as i64], |r| {
                            r.get::<_, String>(0)
                        })
                        .map_err(sql_error)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(sql_error)?;
                    let mut items = Vec::new();
                    for id in ids.iter().take(limit) {
                        let mut item=self.object_row("SELECT media_id,work_id,slot_key,ordinal,kind,width,height,source_variant FROM visible_media WHERE media_id=?1", &[&id])?.ok_or_else(missing)?;
                        let bindings=self.snapshot.db.prepare("SELECT a.asset_id,a.sha256,a.representation,a.recipe_id,a.evidence,a.last_verified_at,o.media_category FROM current_media_assets v JOIN visible_assets a ON a.asset_id=v.asset_id JOIN main.objects o ON o.sha256=a.sha256 WHERE v.media_id=?1 AND (?2 IS NULL OR v.recipe_id=?2 OR v.representation IN ('original','poster')) ORDER BY v.representation,v.recipe_id LIMIT 9").map_err(sql_error)?.query_map(params![id,request.recipe_id],|r|Ok(json!({"record_id":r.get::<_,String>(0)?,"object_sha256":r.get::<_,String>(1)?,"representation":r.get::<_,String>(2)?,"recipe_id":r.get::<_,String>(3)?,"evidence":r.get::<_,String>(4)?,"last_verified_at":r.get::<_,Option<String>>(5)?,"browsable_image":r.get::<_,String>(6)?=="image"}))).map_err(sql_error)?.collect::<rusqlite::Result<Vec<_>>>().map_err(sql_error)?;
                        if bindings.len() > 8 {
                            return Err(Error::new("METADATA_LIMIT", "请指定一个配方读取媒体表示"));
                        }
                        item["availability"] = json!(if bindings.is_empty() {
                            "missing"
                        } else if item["kind"] == "ugoira" && bindings.len() < 2 {
                            "partial"
                        } else {
                            "available"
                        });
                        item["bindings"] = json!(bindings);
                        items.push(item);
                    }
                    let next = if ids.len() > limit {
                        Some(self.next(
                            &selector,
                            &items.last().expect("nonempty page")["ordinal"].to_string(),
                        )?)
                    } else {
                        None
                    };
                    json!({"work_id":request.id,"manifest_id":mid,"manifest_state":current["manifest_state"],"context_id":manifest["context_id"],"observed_at":manifest["observed_at"],"items":items,"next_cursor":next,"version":self.snapshot.revision})
                } else {
                    json!({"work_id":request.id,"manifest_id":null,"manifest_state":"missing","context_id":null,"observed_at":null,"items":[],"next_cursor":null,"version":self.snapshot.revision})
                }
            }
            SourceRelationKind::Author => {
                let author=self.object_row("SELECT observation_id,author_id,capture_id,observed_at,display_name,profile_json FROM author_observations WHERE author_id=?1 AND commit_seq<=?2 AND normalizer_version='pixiv-web-v1' ORDER BY observed_at DESC,observation_id DESC LIMIT 1", &[&request.id,&sequence])?.ok_or_else(missing)?;
                json!({"author_id":request.id,"observation":author,"version":self.snapshot.revision})
            }
            SourceRelationKind::AuthorWorks => {
                let snapshot=self.object_row("SELECT snapshot_id,context_id,observed_at,traversal_exhausted FROM discovery_snapshots WHERE root_kind='author' AND root_id=?1 AND relation='author_works' AND commit_seq<=?2 ORDER BY observed_at DESC,snapshot_id DESC LIMIT 1", &[&request.id,&sequence])?.ok_or_else(missing)?;
                let sid = snapshot["snapshot_id"].as_str().ok_or_else(missing)?;
                let selector = format!("author-works:{}:{sid}", request.id);
                let after = self
                    .after(request.cursor.as_deref(), &selector, "-1")?
                    .parse::<i64>()
                    .map_err(|_| Error::invalid("作者目录游标无效"))?;
                let mut stmt=self.snapshot.db.prepare("SELECT ordinal,target_id FROM discovery_members WHERE snapshot_id=?1 AND target_kind='work' AND ordinal>?2 ORDER BY ordinal LIMIT ?3").map_err(sql_error)?;
                let rows = stmt
                    .query_map(params![sid, after, (limit + 1) as i64], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(sql_error)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(sql_error)?;
                let mut items = Vec::new();
                for (_, id) in rows.iter().take(limit) {
                    let current=self.object_row("SELECT cw.observation_id,cw.manifest_id,cw.manifest_state,w.title FROM current_works cw LEFT JOIN visible_works w ON w.observation_id=cw.observation_id WHERE cw.work_id=?1", &[id])?;
                    items.push(json!({"work_id":id,"current":current}));
                }
                let next = if rows.len() > limit {
                    Some(self.next(&selector, &rows[limit - 1].0.to_string())?)
                } else {
                    None
                };
                json!({"author_id":request.id,"snapshot_id":sid,"observed_at":snapshot["observed_at"],"context_id":snapshot["context_id"],"traversal_exhausted":snapshot["traversal_exhausted"].as_i64()==Some(1),"items":items,"next_cursor":next,"version":self.snapshot.revision})
            }
        };
        if serde_json::to_vec(&result).map_err(Error::io)?.len() > 2 * 1024 * 1024 {
            return Err(Error::new("METADATA_LIMIT", "来源关系响应超过 2 MiB"));
        }
        Ok(result)
    }
    pub fn open(
        source: &'a Source,
        version: Option<&str>,
        cancelled: ReadCancellation,
        deadline: Option<Instant>,
    ) -> Result<Self> {
        let snapshot = Snapshot::open(source, version, cancelled, deadline)?;
        if snapshot.pointer.schema_version != 3 {
            return Err(Error::new("SOURCE_FORMAT_UNSUPPORTED", "需要在线格式 3"));
        }
        Ok(Self { source, snapshot })
    }
    pub fn version(&self) -> ReadVersion {
        let s = &self.snapshot;
        ReadVersion {
            token: format!(
                "metadata-v3:{}:{}:{}",
                self.source.id, s.pointer.generation, s.sequence
            ),
            library_id: self.source.id.clone(),
            generation: s.pointer.generation.clone(),
            catalog_sequence: s.sequence.to_string(),
            analysis_sequence: s.sequence.to_string(),
            consistency: "retained_online_snapshot".into(),
        }
    }
    pub fn after(&self, cursor: Option<&str>, subject: &str, default: &str) -> Result<String> {
        let Some(value) = cursor else {
            return Ok(default.into());
        };
        if value.len() > 4096 {
            return Err(Error::invalid("来源游标过大"));
        }
        let c: Cursor = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(value)
                .map_err(|_| Error::invalid("来源游标无效"))?,
        )
        .map_err(|_| Error::invalid("来源游标无效"))?;
        if c.source != self.source.id || c.version != self.snapshot.revision || c.subject != subject
        {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "游标的来源、版本或筛选条件已变化",
            ));
        }
        Ok(c.after)
    }
    pub fn next(&self, subject: &str, after: &str) -> Result<String> {
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
        self.snapshot.db.query_row("SELECT a.asset_id,a.media_id,a.recipe_id,m.work_id,m.manifest_id,m.ordinal,m.kind,a.representation FROM visible_assets a JOIN visible_media m USING(media_id) WHERE a.sha256=?1 AND a.asset_id=?2",params![asset,record],|r|Ok(AssetRecord {
            record_id:r.get(0)?,origin_observation_id:Some(r.get(1)?),post_id:Some(r.get(3)?),source_md5:None,storage_profile:Some(r.get(2)?),
            media_origin:Some(MediaOrigin {work_id:r.get(3)?,media_id:r.get(1)?,manifest_id:r.get(4)?,ordinal:r.get(5)?,kind:r.get(6)?,representation:r.get(7)?}),pin_origin:None
        })).optional().map_err(sql_error)?.ok_or_else(missing)
    }
    pub fn metadata(&self, asset: &str, request: MetadataRequest) -> Result<MetadataOverview> {
        sha(asset)?;
        let subject = format!("records:{asset}");
        let after = self.after(request.cursor.as_deref(), &subject, "")?;
        let limit = request.limit.unwrap_or(50).clamp(1, 200);
        let mut stmt=self.snapshot.db.prepare("SELECT asset_id FROM visible_assets WHERE sha256=?1 AND asset_id>?2 ORDER BY asset_id LIMIT ?3").map_err(sql_error)?;
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
            .map(|id| self.record(asset, id))
            .collect::<Result<_>>()?;
        let (bytes,ext,width,height):(i64,String,Option<u32>,Option<u32>)=self.snapshot.db.query_row("SELECT length,stored_ext,stored_width,stored_height FROM visible_objects WHERE sha256=?1",[asset],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql_error)?.ok_or_else(missing)?;
        Ok(MetadataOverview {
            object: Asset {
                key: AssetKey {
                    source_id: self.source.id.clone(),
                    asset_id: asset.into(),
                },
                name: asset.into(),
                bytes: bytes as u64,
                extension: ext,
                source_name: self.source.name.clone(),
            },
            stored_width: width,
            stored_height: height,
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
        let mut stmt=self.snapshot.db.prepare("SELECT DISTINCT m.work_id FROM visible_assets a JOIN visible_media m USING(media_id) WHERE a.sha256=?1 ORDER BY length(m.work_id),m.work_id LIMIT 8").map_err(sql_error)?;
        assets.iter().map(|asset| {
            sha(asset)?;
            let post_ids=stmt.query_map([asset],|r|r.get(0)).map_err(sql_error)?.collect::<rusqlite::Result<_>>().map_err(sql_error)?;
            let post_count:i64=self.snapshot.db.query_row("SELECT count(DISTINCT m.work_id) FROM visible_assets a JOIN visible_media m USING(media_id) WHERE a.sha256=?1",[asset],|r|r.get(0)).map_err(sql_error)?;
            Ok(AssetSummary {asset_id:asset.clone(),post_ids,post_count:post_count as u64,version:self.version().token})
        }).collect()
    }
    pub fn observations(
        &self,
        asset: &str,
        record_id: &str,
        request: MetadataRequest,
    ) -> Result<ObservationPage> {
        let record = self.record(asset, record_id)?;
        let origin = record.media_origin.as_ref().expect("media record");
        let subject = format!("observations:{asset}:{record_id}");
        let after = self.after(request.cursor.as_deref(), &subject, "")?;
        if request.cursor.is_some() && request.observation_id.is_some() {
            return Err(Error::invalid("指定观察不能同时分页"));
        }
        let focus = request.observation_id.as_deref();
        if let Some(id) = focus {
            sha(id)?;
        }
        let limit = request.limit.unwrap_or(20).clamp(1, 50);
        let mut stmt=self.snapshot.db.prepare("SELECT key,id FROM (SELECT '0:'||media_id AS key,media_id AS id FROM visible_media WHERE media_id=?1 UNION ALL SELECT '1:'||observation_id,observation_id FROM visible_works WHERE work_id=?2) WHERE key>?3 AND (?4 IS NULL OR id=?4) ORDER BY key LIMIT ?5").map_err(sql_error)?;
        let ids = stmt
            .query_map(
                params![
                    origin.media_id,
                    origin.work_id,
                    after,
                    focus,
                    (limit + 1) as i64
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        let mut items = Vec::new();
        for (_, id) in ids.iter().take(limit) {
            if id == &origin.media_id {
                let (width,height,variant,observed,seq):(Option<i64>,Option<i64>,String,String,i64)=self.snapshot.db.query_row("SELECT m.width,m.height,m.source_variant,f.observed_at,m.commit_seq FROM visible_media m JOIN visible_manifests f USING(manifest_id) WHERE media_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(sql_error)?;
                items.push(Observation {
                    observation_id: id.clone(),
                    row_id: id.clone(),
                    post_id: Some(origin.work_id.clone()),
                    relation: "asset_origin".into(),
                    source_key: Some(origin.manifest_id.clone()),
                    source_kind: Some("pixiv_media_manifest".into()),
                    observed_at: Some(observed),
                    time_quality: Some("captured_at".into()),
                    ingested_at: None,
                    commit_sequence: Some(seq.to_string()),
                    fields: vec![
                        field(
                            "source_width",
                            width.map(|v| MetadataValue::Integer(v.to_string())),
                            "media_entries.width",
                        ),
                        field(
                            "source_height",
                            height.map(|v| MetadataValue::Integer(v.to_string())),
                            "media_entries.height",
                        ),
                        field(
                            "source_variant",
                            Some(MetadataValue::Text(variant)),
                            "media_entries.source_variant",
                        ),
                    ],
                });
            } else {
                let row=self.snapshot.db.query_row("SELECT title,caption_html,author_id,observed_at,commit_seq,source_fields_json FROM visible_works WHERE observation_id=?1 AND work_id=?2",params![id,origin.work_id],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?))).map_err(sql_error)?;
                let values: serde_json::Value = serde_json::from_str(&row.5).map_err(Error::io)?;
                let mut fields = vec![
                    text_field("title", row.0, "work_observations.title"),
                    text_field("caption_html", row.1, "work_observations.caption_html"),
                    text_field("author_id", row.2, "work_observations.author_id"),
                ];
                for key in [
                    "x_restrict",
                    "ai_type",
                    "bookmark_count",
                    "view_count",
                    "like_count",
                ] {
                    fields.push(field(
                        &format!("pixiv.{key}"),
                        values["pixiv"][key]
                            .as_i64()
                            .map(|v| MetadataValue::Integer(v.to_string())),
                        "work_observations.source_fields_json",
                    ));
                }
                let mut tags=self.snapshot.db.prepare("SELECT t.tag FROM work_tags wt JOIN tags t USING(tag_id) WHERE wt.observation_id=?1 ORDER BY wt.ordinal LIMIT 257").map_err(sql_error)?.query_map([id],|r|r.get::<_,String>(0)).map_err(sql_error)?.collect::<rusqlite::Result<Vec<_>>>().map_err(sql_error)?;
                let truncated = tags.len() > 256;
                tags.truncate(256);
                let mut tag_field = field(
                    "tags",
                    Some(MetadataValue::Tags(tags)),
                    "work_tags.literal_tag",
                );
                tag_field.truncated = truncated;
                fields.push(tag_field);
                items.push(Observation {
                    observation_id: id.clone(),
                    row_id: id.clone(),
                    post_id: Some(origin.work_id.clone()),
                    relation: "same_work".into(),
                    source_key: None,
                    source_kind: Some("pixiv_work_detail".into()),
                    observed_at: Some(row.3),
                    time_quality: Some("captured_at".into()),
                    ingested_at: None,
                    commit_sequence: Some(row.4.to_string()),
                    fields,
                });
            }
        }
        let next_cursor = if ids.len() > limit {
            Some(self.next(&subject, &ids[limit - 1].0)?)
        } else {
            None
        };
        Ok(ObservationPage {
            record_id: record_id.into(),
            items,
            next_cursor,
            version: self.version(),
        })
    }
    pub fn raw(&self, asset: &str, record_id: &str, observation_id: &str) -> Result<RawMetadata> {
        sha(observation_id)?;
        let record = self.record(asset, record_id)?;
        let origin = record.media_origin.as_ref().expect("media record");
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
                    "SELECT capture_id FROM visible_works WHERE observation_id=?1 AND work_id=?2",
                    params![observation_id, origin.work_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql_error)?
        };
        let capture = capture.ok_or_else(missing)?;
        let (format,size,hash,body):(String,i64,String,Option<Vec<u8>>)=self.snapshot.db.query_row("SELECT raw_format,raw_bytes,raw_sha256,CASE WHEN raw_bytes<=131072 THEN raw_zlib END FROM captures WHERE capture_id=?1 AND commit_seq<=?2",params![capture,self.snapshot.sequence as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql_error)?.ok_or_else(missing)?;
        let json = body
            .map(|v| crate::online::raw::decode(&v, size as u64, &hash, 131072))
            .transpose()?;
        Ok(RawMetadata {
            observation_id: observation_id.into(),
            format: Some(format!("pixiv-{format}-response/v1")),
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
