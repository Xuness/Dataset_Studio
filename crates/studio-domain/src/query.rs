use crate::{AssetKey, Error, Result, validate_id};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Integer,
    Boolean,
    Tags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryOperator {
    Eq,
    Ne,
    Gte,
    Lte,
    HasTag,
    IsMissing,
    IsPresent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum QueryValue {
    Text(String),
    Integer(String),
    Boolean(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryCondition {
    pub field: String,
    pub operator: QueryOperator,
    pub value: Option<QueryValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationRule {
    /// Only the observation and stored object associated by current_posts participate.
    CurrentPost,
    /// A single observation from any post associated with this stored object must satisfy ALL metadata conditions.
    AnyObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryOrder {
    AssetKeyAsc,
    AssetKeyDesc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySpec {
    pub version: u32,
    pub source_ids: Vec<String>,
    pub conditions: Vec<QueryCondition>,
    pub observation_rule: ObservationRule,
    pub order: QueryOrder,
}
impl QuerySpec {
    pub fn normalize(mut self) -> Result<Self> {
        if self.version != 1 {
            return Err(Error::new(
                "QUERY_VERSION_UNSUPPORTED",
                "查询协议版本不兼容",
            ));
        }
        if self.source_ids.is_empty() || self.source_ids.len() > 8 || self.conditions.len() > 12 {
            return Err(Error::invalid("查询需要 1–8 个来源，最多 12 个联合条件"));
        }
        for id in &self.source_ids {
            validate_id(id)?;
        }
        self.source_ids.sort();
        self.source_ids.dedup();
        // Conditions are a conjunction, so canonical order has no effect on semantics.
        self.conditions
            .sort_by_cached_key(|c| serde_json::to_string(c).unwrap_or_default());
        self.conditions.dedup();
        Ok(self)
    }
    pub fn uses_metadata(&self) -> bool {
        self.conditions
            .iter()
            .any(|c| !c.field.starts_with("stored.") && c.field != "asset.id")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDefinition {
    pub id: String,
    pub name: String,
    pub field_type: FieldType,
    pub unit: Option<String>,
    pub missing: String,
    pub basis: String,
    pub display: bool,
    pub operators: Vec<QueryOperator>,
    pub sortable: bool,
    pub cost: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDirectory {
    pub version: u32,
    pub source_id: String,
    pub fields: Vec<FieldDefinition>,
    pub observation_rules: Vec<ObservationRule>,
    pub orders: Vec<QueryOrder>,
    pub max_conditions: usize,
}
impl FieldDirectory {
    pub fn validate(&self, spec: &QuerySpec) -> Result<()> {
        if !self.orders.contains(&spec.order)
            || !self.observation_rules.contains(&spec.observation_rule)
        {
            return Err(Error::new(
                "QUERY_UNSUPPORTED",
                "来源不支持该排序或观察判定规则",
            ));
        }
        for condition in &spec.conditions {
            let field = self
                .fields
                .iter()
                .find(|f| f.id == condition.field)
                .ok_or_else(|| {
                    Error::new(
                        "QUERY_UNSUPPORTED",
                        format!("来源不支持字段 {}", condition.field),
                    )
                })?;
            if !field.operators.contains(&condition.operator) {
                return Err(Error::new(
                    "QUERY_UNSUPPORTED",
                    format!("字段 {} 不支持该操作", field.name),
                ));
            }
            if matches!(
                condition.operator,
                QueryOperator::IsMissing | QueryOperator::IsPresent
            ) {
                if condition.value.is_some() {
                    return Err(Error::invalid("缺失判定不能附带值"));
                }
                continue;
            }
            let valid = match (&field.field_type, &condition.value) {
                (FieldType::Text | FieldType::Tags, Some(QueryValue::Text(value))) => {
                    value.len() <= 256
                        && !value.chars().any(char::is_control)
                        && (condition.operator != QueryOperator::HasTag
                            || (!value.is_empty() && !value.chars().any(char::is_whitespace)))
                }
                (FieldType::Integer, Some(QueryValue::Integer(value))) => {
                    value.parse::<i64>().is_ok_and(|n| n.to_string() == *value)
                }
                (FieldType::Boolean, Some(QueryValue::Boolean(_))) => true,
                _ => false,
            };
            if !valid {
                return Err(Error::invalid(format!(
                    "字段 {} 的条件值类型或格式不正确",
                    field.name
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySourceVersion {
    pub source_id: String,
    pub catalog_revision: String,
    pub analysis_sequence: Option<String>,
    pub consistency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryDefinition {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub revision: u64,
    pub spec: QuerySpec,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultState {
    Queued,
    Running,
    Ready,
    Cancelled,
    Failed,
    Interrupted,
    Released,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub id: String,
    pub project_id: String,
    pub definition_id: Option<String>,
    pub definition_revision: Option<u64>,
    pub spec: QuerySpec,
    pub source_versions: Vec<QuerySourceVersion>,
    pub state: ResultState,
    /// Native rows processed can exceed the unique stored-object count.
    pub processed: u64,
    /// Exact only after successful completion; never a capped count.
    pub count: Option<u64>,
    pub created_at: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultPage {
    pub keys: Vec<AssetKey>,
    pub next: Option<AssetKey>,
}
