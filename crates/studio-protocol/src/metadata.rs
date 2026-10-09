use crate::{AssetKey, domain};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct AssetSummary {
    pub site_name: Option<String>,
    /// available, unlinked, preparing, unavailable, or unsupported.
    pub status: String,
    pub post_ids: Vec<String>,
    pub post_count: Option<String>,
    pub version: Option<String>,
    pub issue: Option<String>,
}
impl From<domain::AssetSummary> for AssetSummary {
    fn from(value: domain::AssetSummary) -> Self {
        Self {
            site_name: None,
            status: if value.post_count == 0 {
                "unlinked"
            } else {
                "available"
            }
            .into(),
            post_ids: value.post_ids,
            post_count: Some(value.post_count.to_string()),
            version: Some(value.version),
            issue: None,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct AssetSummaryEntry {
    pub key: AssetKey,
    pub summary: AssetSummary,
}
#[derive(Serialize, ToSchema)]
pub struct AssetSummaries {
    pub items: Vec<AssetSummaryEntry>,
    pub preparing: bool,
}
#[derive(Serialize, ToSchema)]
pub struct ReadVersion {
    pub token: String,
    pub library_id: String,
    pub generation: String,
    pub catalog_sequence: String,
    pub analysis_sequence: String,
    pub consistency: String,
}
impl From<domain::ReadVersion> for ReadVersion {
    fn from(v: domain::ReadVersion) -> Self {
        Self {
            token: v.token,
            library_id: v.library_id,
            generation: v.generation,
            catalog_sequence: v.catalog_sequence,
            analysis_sequence: v.analysis_sequence,
            consistency: v.consistency,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AssetRecord {
    pub record_id: String,
    pub origin_observation_id: Option<String>,
    pub post_id: Option<String>,
    pub source_md5: Option<String>,
    pub storage_profile: Option<String>,
    pub work_id: Option<String>,
    pub media_id: Option<String>,
    pub manifest_id: Option<String>,
    pub ordinal: Option<u32>,
    pub kind: Option<String>,
    pub representation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_origin: Option<PinOrigin>,
}
#[derive(Serialize, ToSchema)]
pub struct PinOrigin {
    pub pin_id: String,
    pub media_id: String,
    pub manifest_id: String,
    pub role: String,
}
impl From<domain::AssetRecord> for AssetRecord {
    fn from(v: domain::AssetRecord) -> Self {
        Self {
            record_id: v.record_id,
            origin_observation_id: v.origin_observation_id,
            post_id: v.post_id,
            source_md5: v.source_md5,
            storage_profile: v.storage_profile,
            work_id: v.media_origin.as_ref().map(|m| m.work_id.clone()),
            media_id: v.media_origin.as_ref().map(|m| m.media_id.clone()),
            manifest_id: v.media_origin.as_ref().map(|m| m.manifest_id.clone()),
            ordinal: v.media_origin.as_ref().map(|m| m.ordinal),
            kind: v.media_origin.as_ref().map(|m| m.kind.clone()),
            representation: v.media_origin.map(|m| m.representation),
            pin_origin: v.pin_origin.map(|p| PinOrigin {
                pin_id: p.pin_id,
                media_id: p.media_id,
                manifest_id: p.manifest_id,
                role: p.role,
            }),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct MetadataObject {
    pub key: AssetKey,
    pub name: String,
    pub bytes: String,
    pub extension: String,
    pub source_name: String,
}
impl From<domain::Asset> for MetadataObject {
    fn from(a: domain::Asset) -> Self {
        Self {
            key: a.key.into(),
            name: a.name,
            bytes: a.bytes.to_string(),
            extension: a.extension,
            source_name: a.source_name,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct MetadataOverview {
    pub object: MetadataObject,
    pub stored_width: Option<u32>,
    pub stored_height: Option<u32>,
    pub dimensions_evidence: String,
    pub records: Vec<AssetRecord>,
    pub next_cursor: Option<String>,
    pub version: ReadVersion,
}
impl From<domain::MetadataOverview> for MetadataOverview {
    fn from(v: domain::MetadataOverview) -> Self {
        Self {
            object: v.object.into(),
            stored_width: v.stored_width,
            stored_height: v.stored_height,
            dimensions_evidence: v.dimensions_evidence,
            records: v.records.into_iter().map(Into::into).collect(),
            next_cursor: v.next_cursor,
            version: v.version.into(),
        }
    }
}
#[derive(Serialize, ToSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MetadataValue {
    Text(String),
    Integer(String),
    Boolean(bool),
    Tags(Vec<String>),
    Timestamp(String),
}
impl From<domain::MetadataValue> for MetadataValue {
    fn from(v: domain::MetadataValue) -> Self {
        match v {
            domain::MetadataValue::Text(v) => Self::Text(v),
            domain::MetadataValue::Integer(v) => Self::Integer(v),
            domain::MetadataValue::Boolean(v) => Self::Boolean(v),
            domain::MetadataValue::Tags(v) => Self::Tags(v),
            domain::MetadataValue::Timestamp(v) => Self::Timestamp(v),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct MetadataField {
    pub name: String,
    pub label: Option<String>,
    pub value: Option<MetadataValue>,
    pub provenance: String,
    pub missing_reason: Option<String>,
    pub truncated: bool,
}
impl From<domain::MetadataField> for MetadataField {
    fn from(v: domain::MetadataField) -> Self {
        Self {
            name: v.name,
            label: v.label,
            value: v.value.map(Into::into),
            provenance: v.provenance,
            missing_reason: v.missing_reason,
            truncated: v.truncated,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct Observation {
    pub observation_id: String,
    pub row_id: String,
    pub post_id: Option<String>,
    pub relation: String,
    pub source_key: Option<String>,
    pub source_kind: Option<String>,
    pub observed_at: Option<String>,
    pub time_quality: Option<String>,
    pub ingested_at: Option<String>,
    pub commit_sequence: Option<String>,
    pub fields: Vec<MetadataField>,
}
impl From<domain::Observation> for Observation {
    fn from(v: domain::Observation) -> Self {
        Self {
            observation_id: v.observation_id,
            row_id: v.row_id,
            post_id: v.post_id,
            relation: v.relation,
            source_key: v.source_key,
            source_kind: v.source_kind,
            observed_at: v.observed_at,
            time_quality: v.time_quality,
            ingested_at: v.ingested_at,
            commit_sequence: v.commit_sequence,
            fields: v.fields.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct ObservationPage {
    pub record_id: String,
    pub items: Vec<Observation>,
    pub next_cursor: Option<String>,
    pub version: ReadVersion,
}
impl From<domain::ObservationPage> for ObservationPage {
    fn from(v: domain::ObservationPage) -> Self {
        Self {
            record_id: v.record_id,
            items: v.items.into_iter().map(Into::into).collect(),
            next_cursor: v.next_cursor,
            version: v.version.into(),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct RawMetadata {
    pub observation_id: String,
    pub format: Option<String>,
    pub schema_id: Option<String>,
    pub schema: Option<RawSourceSchema>,
    pub json: Option<String>,
    pub bytes: Option<String>,
    pub status: String,
    pub version: ReadVersion,
}
impl From<domain::RawMetadata> for RawMetadata {
    fn from(v: domain::RawMetadata) -> Self {
        Self {
            observation_id: v.observation_id,
            format: v.format,
            schema_id: v.schema_id,
            schema: v.schema.map(|s| RawSourceSchema {
                format: s.format,
                encoding: s.encoding,
                bytes: s.bytes,
                data: s.data,
            }),
            json: v.json,
            bytes: v.bytes,
            status: v.status,
            version: v.version.into(),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct RawSourceSchema {
    pub format: String,
    pub encoding: String,
    pub bytes: String,
    pub data: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MetadataQuery {
    pub observation_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub version: Option<String>,
}
impl From<MetadataQuery> for domain::MetadataRequest {
    fn from(v: MetadataQuery) -> Self {
        Self {
            observation_id: v.observation_id,
            cursor: v.cursor,
            limit: v.limit,
            version: v.version,
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RawMetadataQuery {
    pub version: String,
}
