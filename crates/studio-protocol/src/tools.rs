use crate::{AssetKey, QuerySourceVersion, ScopeRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use studio_domain as domain;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OperatorRun {
    pub operator_id: String,
    pub operator_version: u32,
    pub parameters_version: u32,
    pub parameters: Value,
}
impl From<OperatorRun> for domain::OperatorRun {
    fn from(v: OperatorRun) -> Self {
        Self {
            operator_id: v.operator_id,
            operator_version: v.operator_version,
            parameters_version: v.parameters_version,
            parameters: v.parameters,
        }
    }
}
impl From<domain::OperatorRun> for OperatorRun {
    fn from(v: domain::OperatorRun) -> Self {
        Self {
            operator_id: v.operator_id,
            operator_version: v.operator_version,
            parameters_version: v.parameters_version,
            parameters: v.parameters,
        }
    }
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolSubmission {
    pub idempotency_key: String,
    pub run: OperatorRun,
    pub scope: ScopeRef,
    #[serde(default)]
    pub delay_ms: u64,
}
impl From<ToolSubmission> for domain::ToolSubmission {
    fn from(v: ToolSubmission) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            run: v.run.into(),
            scope: v.scope.into(),
            delay_ms: v.delay_ms,
        }
    }
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScalarInput {
    StoredBytes,
    OriginWidth,
    Artifact { artifact_id: String },
}
impl From<domain::ScalarInput> for ScalarInput {
    fn from(v: domain::ScalarInput) -> Self {
        match v {
            domain::ScalarInput::StoredBytes => Self::StoredBytes,
            domain::ScalarInput::OriginWidth => Self::OriginWidth,
            domain::ScalarInput::Artifact { artifact_id } => Self::Artifact { artifact_id },
        }
    }
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScalarValue {
    Available { value: String },
    Missing { reason: String },
    Failed { code: String, message: String },
    Uncomputed { reason: String },
}
impl From<domain::ScalarValue> for ScalarValue {
    fn from(v: domain::ScalarValue) -> Self {
        match v {
            domain::ScalarValue::Available { value } => Self::Available { value },
            domain::ScalarValue::Missing { reason } => Self::Missing { reason },
            domain::ScalarValue::Failed { code, message } => Self::Failed { code, message },
            domain::ScalarValue::Uncomputed { reason } => Self::Uncomputed { reason },
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct JobRun {
    pub run: OperatorRun,
    pub fields: Vec<ScalarInput>,
    pub source_versions: Vec<QuerySourceVersion>,
}
impl From<domain::JobRun> for JobRun {
    fn from(v: domain::JobRun) -> Self {
        Self {
            run: v.run.into(),
            fields: v.fields.into_iter().map(Into::into).collect(),
            source_versions: v.source_versions.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct ParameterDescriptor {
    pub id: String,
    pub name: String,
    pub value_type: String,
    pub default_value: Value,
    pub required: bool,
}
#[derive(Serialize, ToSchema)]
pub struct OutputDescriptor {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub schema_version: u32,
    pub subject: String,
}
#[derive(Serialize, ToSchema)]
pub struct OperatorCapabilities {
    pub cancel: bool,
    pub checkpoint: bool,
    pub retry: bool,
    pub deterministic: bool,
    pub item_failures: bool,
}
#[derive(Serialize, ToSchema)]
pub struct ResourceRequirements {
    pub cpu_slots: u32,
    pub memory_bytes: u64,
    pub media_reads: bool,
    pub gpu: bool,
}
#[derive(Serialize, ToSchema)]
pub struct OperatorDescriptor {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub parameters_version: u32,
    pub parameters: Vec<ParameterDescriptor>,
    pub input_scopes: Vec<String>,
    pub outputs: Vec<OutputDescriptor>,
    pub capabilities: OperatorCapabilities,
    pub resources: ResourceRequirements,
}
impl From<domain::OperatorDescriptor> for OperatorDescriptor {
    fn from(v: domain::OperatorDescriptor) -> Self {
        Self {
            id: v.id,
            name: v.name,
            version: v.version,
            parameters_version: v.parameters_version,
            parameters: v.parameters.into_iter().map(Into::into).collect(),
            input_scopes: v.input_scopes,
            outputs: v.outputs.into_iter().map(Into::into).collect(),
            capabilities: v.capabilities.into(),
            resources: v.resources.into(),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct Operators {
    pub protocol_version: u32,
    pub items: Vec<OperatorDescriptor>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    Legacy,
    Publishing,
    Ready,
    Released,
    Unavailable,
}
impl From<domain::ArtifactState> for ArtifactState {
    fn from(v: domain::ArtifactState) -> Self {
        match v {
            domain::ArtifactState::Legacy => Self::Legacy,
            domain::ArtifactState::Publishing => Self::Publishing,
            domain::ArtifactState::Ready => Self::Ready,
            domain::ArtifactState::Released => Self::Released,
            domain::ArtifactState::Unavailable => Self::Unavailable,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct ArtifactFile {
    pub path: String,
    pub bytes: Option<String>,
    pub sha256: Option<String>,
    pub media_type: String,
}
impl From<domain::ArtifactFile> for ArtifactFile {
    fn from(v: domain::ArtifactFile) -> Self {
        Self {
            path: v.path,
            bytes: v.bytes.map(|n| n.to_string()),
            sha256: v.sha256,
            media_type: v.media_type,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct ArtifactProvenance {
    pub run: Option<OperatorRun>,
    pub input_scope: Option<ScopeRef>,
    pub input_sha256: Option<String>,
    pub attempt: Option<u32>,
    pub input_artifacts: Vec<String>,
    pub fields_frozen: bool,
    pub evidence: String,
}
impl From<domain::ArtifactProvenance> for ArtifactProvenance {
    fn from(v: domain::ArtifactProvenance) -> Self {
        Self {
            run: v.run.map(Into::into),
            input_scope: v.input_scope.map(Into::into),
            input_sha256: v.input_sha256,
            attempt: v.attempt,
            input_artifacts: v.input_artifacts,
            fields_frozen: v.fields_frozen,
            evidence: v.evidence,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct Artifact {
    pub id: String,
    pub project_id: String,
    pub job_id: String,
    pub output_id: String,
    pub name: String,
    pub kind: String,
    pub schema_version: u32,
    pub state: ArtifactState,
    pub count: Option<u64>,
    pub created_at: String,
    pub files: Vec<ArtifactFile>,
    pub provenance: ArtifactProvenance,
    pub issue: Option<String>,
}
impl From<domain::Artifact> for Artifact {
    fn from(v: domain::Artifact) -> Self {
        Self {
            id: v.id,
            project_id: v.project_id,
            job_id: v.job_id,
            output_id: v.output_id,
            name: v.name,
            kind: v.kind,
            schema_version: v.schema_version,
            state: v.state.into(),
            count: v.count,
            created_at: v.created_at,
            files: v.files.into_iter().map(Into::into).collect(),
            provenance: v.provenance.into(),
            issue: v.issue,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct Artifacts {
    pub items: Vec<Artifact>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct ArtifactRow {
    pub key: AssetKey,
    pub ordinal: u64,
    pub scalar: Option<ScalarValue>,
    pub data: Value,
}
impl From<domain::ArtifactRow> for ArtifactRow {
    fn from(v: domain::ArtifactRow) -> Self {
        Self {
            key: v.key.into(),
            ordinal: v.ordinal,
            scalar: v.scalar.map(Into::into),
            data: v.data,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct ArtifactPage {
    pub artifact_id: String,
    pub items: Vec<ArtifactRow>,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct Draft {
    pub project_id: String,
    pub module_id: String,
    pub instance_id: String,
    pub schema_version: u32,
    pub revision: u64,
    pub updated_at: String,
    pub value: Value,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveDraft {
    pub schema_version: u32,
    pub expected_revision: u64,
    pub value: Value,
}
impl From<SaveDraft> for domain::SaveDraft {
    fn from(v: SaveDraft) -> Self {
        Self {
            schema_version: v.schema_version,
            expected_revision: v.expected_revision,
            value: v.value,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct MaybeDraft {
    pub draft: Option<Draft>,
}
#[derive(Serialize, ToSchema)]
pub struct Preference {
    pub key: String,
    pub schema_version: u32,
    pub revision: u64,
    pub value: Value,
}
#[derive(Serialize, ToSchema)]
pub struct MaybePreference {
    pub preference: Option<Preference>,
}

macro_rules! direct {
    ($name:ident {$($field:ident),+}) => {impl From<domain::$name> for $name{fn from(v:domain::$name)->Self{Self{$($field:v.$field),+}}}};
}
direct!(ParameterDescriptor {
    id,
    name,
    value_type,
    default_value,
    required
});
direct!(OutputDescriptor {
    id,
    name,
    kind,
    schema_version,
    subject
});
direct!(OperatorCapabilities {
    cancel,
    checkpoint,
    retry,
    deterministic,
    item_failures
});
direct!(ResourceRequirements {
    cpu_slots,
    memory_bytes,
    media_reads,
    gpu
});
direct!(Draft {
    project_id,
    module_id,
    instance_id,
    schema_version,
    revision,
    updated_at,
    value
});
direct!(Preference {
    key,
    schema_version,
    revision,
    value
});
