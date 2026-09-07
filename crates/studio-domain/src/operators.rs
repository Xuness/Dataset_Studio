use crate::{AssetKey, Error, Result, ScopeRef, validate_id};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorRun {
    pub operator_id: String,
    pub operator_version: u32,
    pub parameters_version: u32,
    pub parameters: Value,
}
impl Default for OperatorRun {
    fn default() -> Self {
        Self {
            operator_id: "core.manifest".into(),
            operator_version: 1,
            parameters_version: 1,
            parameters: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSubmission {
    pub idempotency_key: String,
    pub run: OperatorRun,
    pub scope: ScopeRef,
    #[serde(default)]
    pub delay_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScalarInput {
    StoredBytes,
    /// The origin observation of the lexicographically first linked source record.
    /// This explicit rule never substitutes another historical observation.
    OriginWidth,
    Artifact {
        artifact_id: String,
    },
}
impl ScalarInput {
    pub fn validate(&self) -> Result<()> {
        if let Self::Artifact { artifact_id } = self {
            validate_id(artifact_id)?;
        }
        Ok(())
    }
    pub fn field_id(&self) -> String {
        match self {
            Self::StoredBytes => "stored.bytes".into(),
            Self::OriginWidth => "source.origin.width".into(),
            Self::Artifact { artifact_id } => format!("project.{artifact_id}.value"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScalarValue {
    Available { value: String },
    Missing { reason: String },
    Failed { code: String, message: String },
    Uncomputed { reason: String },
}
impl ScalarValue {
    pub fn integer(value: i64) -> Self {
        Self::Available {
            value: value.to_string(),
        }
    }
    pub fn as_integer(&self) -> Result<Option<i64>> {
        match self {
            Self::Available { value } => value
                .parse()
                .map(Some)
                .map_err(|_| Error::new("FIELD_VALUE_INVALID", "标量值不是有效的 64 位整数")),
            _ => Ok(None),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldBasis {
    pub field_id: String,
    pub subject: String,
    pub rule: String,
    pub source_version: Option<String>,
    pub record_id: Option<String>,
    pub observation_id: Option<String>,
    pub artifact_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenField {
    pub input: ScalarInput,
    pub value: ScalarValue,
    pub basis: FieldBasis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDescriptor {
    pub id: String,
    pub name: String,
    pub value_type: String,
    pub default_value: Value,
    pub required: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputDescriptor {
    pub id: String,
    pub kind: String,
    pub schema_version: u32,
    pub subject: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorCapabilities {
    pub cancel: bool,
    pub checkpoint: bool,
    pub retry: bool,
    pub deterministic: bool,
    pub item_failures: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub cpu_slots: u32,
    pub memory_bytes: u64,
    pub media_reads: bool,
    pub gpu: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalarRow {
    pub key: AssetKey,
    pub ordinal: u64,
    pub value: ScalarValue,
    pub basis: Vec<FieldBasis>,
}
