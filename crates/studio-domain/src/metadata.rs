use crate::Asset;

#[derive(Debug, Clone)]
pub struct ReadVersion {
    pub token: String,
    pub library_id: String,
    pub generation: String,
    pub catalog_sequence: String,
    pub analysis_sequence: String,
    /// Per-request transactions with matched watermarks, not a historical snapshot.
    pub consistency: String,
}
#[derive(Debug, Clone)]
pub struct AssetRecord {
    pub record_id: String,
    pub origin_observation_id: Option<String>,
    pub post_id: Option<String>,
    pub source_md5: Option<String>,
    pub storage_profile: Option<String>,
    pub media_origin: Option<MediaOrigin>,
}

#[derive(Debug, Clone)]
pub struct MediaOrigin {
    pub work_id: String,
    pub media_id: String,
    pub manifest_id: String,
    pub ordinal: u32,
    pub kind: String,
    pub representation: String,
}

/// A bounded identity summary for one listed stored object.
#[derive(Debug, Clone)]
pub struct AssetSummary {
    pub asset_id: String,
    pub post_ids: Vec<String>,
    pub post_count: u64,
    pub version: String,
}
#[derive(Debug, Clone)]
pub struct MetadataOverview {
    pub object: Asset,
    pub stored_width: Option<u32>,
    pub stored_height: Option<u32>,
    pub dimensions_evidence: String,
    pub records: Vec<AssetRecord>,
    pub next_cursor: Option<String>,
    pub version: ReadVersion,
}
#[derive(Debug, Clone)]
pub enum MetadataValue {
    Text(String),
    Integer(String),
    Boolean(bool),
    Tags(Vec<String>),
    Timestamp(String),
}
#[derive(Debug, Clone)]
pub struct MetadataField {
    pub name: String,
    pub label: Option<String>,
    pub value: Option<MetadataValue>,
    pub provenance: String,
    pub missing_reason: Option<String>,
    pub truncated: bool,
}
#[derive(Debug, Clone)]
pub struct Observation {
    pub observation_id: String,
    pub row_id: String,
    pub post_id: Option<String>,
    /// asset_origin or same_post; a same_post observation need not describe this blob.
    pub relation: String,
    pub source_key: Option<String>,
    pub source_kind: Option<String>,
    pub observed_at: Option<String>,
    pub time_quality: Option<String>,
    pub ingested_at: Option<String>,
    pub commit_sequence: Option<String>,
    pub fields: Vec<MetadataField>,
}
#[derive(Debug, Clone)]
pub struct ObservationPage {
    pub record_id: String,
    pub items: Vec<Observation>,
    pub next_cursor: Option<String>,
    pub version: ReadVersion,
}
#[derive(Debug, Clone)]
pub struct RawMetadata {
    pub observation_id: String,
    pub format: Option<String>,
    pub schema_id: Option<String>,
    pub schema: Option<RawSourceSchema>,
    pub json: Option<String>,
    pub bytes: Option<String>,
    /// available, missing, or too_large. Oversized JSON is never returned partially.
    pub status: String,
    pub version: ReadVersion,
}
#[derive(Debug, Clone)]
pub struct RawSourceSchema {
    pub format: String,
    pub encoding: String,
    pub bytes: String,
    pub data: Option<String>,
}
#[derive(Debug, Clone, Default)]
pub struct MetadataRequest {
    pub observation_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum SourceRelationKind {
    Work,
    WorkMedia,
    Author,
    AuthorWorks,
}

#[derive(Debug, Clone)]
pub struct SourceRelationRequest {
    pub kind: SourceRelationKind,
    pub id: String,
    pub version: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub manifest_id: Option<String>,
    pub recipe_id: Option<String>,
}
