use crate::{AssetKey, Error, Result, ScopeRef, ScopeTarget, validate_id};
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
    In,
    HasAllTags,
    HasAnyTags,
    HasNoTags,
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
    TextList(Vec<String>),
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
    PostIdAsc,
    PostIdDesc,
}
impl QueryOrder {
    pub fn by_post(self) -> bool {
        matches!(self, Self::PostIdAsc | Self::PostIdDesc)
    }
    pub fn descending(self) -> bool {
        matches!(self, Self::AssetKeyDesc | Self::PostIdDesc)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySpec {
    pub version: u32,
    pub source_ids: Vec<String>,
    pub conditions: Vec<QueryCondition>,
    pub observation_rule: ObservationRule,
    pub order: QueryOrder,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_scope: Option<ScopeRef>,
}
impl QuerySpec {
    pub fn normalize(mut self) -> Result<Self> {
        if !matches!(self.version, 1..=3) {
            return Err(Error::new(
                "QUERY_VERSION_UNSUPPORTED",
                "查询协议版本不兼容",
            ));
        }
        if self.order.by_post() && self.version < 3 {
            return Err(Error::new(
                "QUERY_VERSION_UNSUPPORTED",
                "帖子 ID 排序需要查询版本 3",
            ));
        }
        if self.source_ids.is_empty() || self.source_ids.len() > 8 || self.conditions.len() > 12 {
            return Err(Error::invalid("查询需要 1–8 个来源，最多 12 个联合条件"));
        }
        for id in &self.source_ids {
            validate_id(id)?;
        }
        let extended = self.input_scope.is_some()
            || self.conditions.iter().any(|c| {
                matches!(
                    c.operator,
                    QueryOperator::In
                        | QueryOperator::HasAllTags
                        | QueryOperator::HasAnyTags
                        | QueryOperator::HasNoTags
                ) || matches!(c.value, Some(QueryValue::TextList(_)))
            });
        if extended && self.version < 2 {
            return Err(Error::new(
                "QUERY_VERSION_UNSUPPORTED",
                "集合与范围筛选需要查询版本 2",
            ));
        }
        if let Some(scope) = &self.input_scope {
            scope.validate_project(&scope.project_id)?;
            if let ScopeTarget::Source { source_id, .. } = &scope.target
                && self.source_ids != [source_id.clone()]
            {
                return Err(Error::invalid("查询来源与输入数据湖不一致"));
            }
        }
        for condition in &mut self.conditions {
            if let Some(QueryValue::TextList(values)) = &mut condition.value {
                if values.is_empty() || values.len() > 64 {
                    return Err(Error::invalid("集合条件需要 1–64 个值"));
                }
                values.sort();
                values.dedup();
            }
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
    /// Membership and predicates are project-owned snapshots. Post-ID ordering
    /// still depends on current source associations and is deliberately excluded.
    pub fn uses_only_fixed_project_data(&self) -> bool {
        !self.order.by_post()
            && matches!(
                self.input_scope.as_ref().map(|s| &s.target),
                Some(ScopeTarget::Workset { .. } | ScopeTarget::QueryResult { .. })
            )
            && self
                .conditions
                .iter()
                .all(|c| c.field.starts_with("project."))
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
                (FieldType::Text, Some(QueryValue::TextList(values))) => {
                    condition.operator == QueryOperator::In
                        && !values.is_empty()
                        && values.len() <= 64
                        && values
                            .iter()
                            .all(|value| value.len() <= 256 && !value.chars().any(char::is_control))
                }
                (FieldType::Tags, Some(QueryValue::TextList(values))) => {
                    matches!(
                        condition.operator,
                        QueryOperator::HasAllTags
                            | QueryOperator::HasAnyTags
                            | QueryOperator::HasNoTags
                    ) && !values.is_empty()
                        && values.len() <= 64
                        && values.iter().all(|value| {
                            !value.is_empty()
                                && value.len() <= 256
                                && !value.chars().any(char::is_whitespace)
                                && !value.chars().any(char::is_control)
                        })
                }
                (FieldType::Text | FieldType::Tags, Some(QueryValue::Text(value))) => {
                    !matches!(
                        condition.operator,
                        QueryOperator::In
                            | QueryOperator::HasAllTags
                            | QueryOperator::HasAnyTags
                            | QueryOperator::HasNoTags
                    ) && value.len() <= 256
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
    #[serde(default)]
    pub cache: QueryCacheInfo,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryCacheInfo {
    pub mode: String,
    pub evaluated_objects: u64,
    pub changed_members: u64,
    #[serde(default)]
    pub tier: QueryCacheTier,
    #[serde(default)]
    pub fixed: bool,
    #[serde(default)]
    pub session_only: bool,
    #[serde(default)]
    pub basis_ratings: Vec<String>,
    #[serde(default)]
    pub candidate_records: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryCacheTier {
    LongTerm,
    #[default]
    Temporary,
}
impl QueryCacheTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LongTerm => "long_term",
            Self::Temporary => "temporary",
        }
    }
    pub fn for_spec(spec: &QuerySpec) -> Self {
        let lake = spec
            .input_scope
            .as_ref()
            .is_none_or(|s| matches!(s.target, ScopeTarget::Source { .. }));
        if lake
            && spec.observation_rule == ObservationRule::CurrentPost
            && !spec.conditions.is_empty()
            && spec.conditions.iter().all(|c| c.field == "rating")
        {
            Self::LongTerm
        } else {
            Self::Temporary
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultPage {
    pub keys: Vec<AssetKey>,
    pub next: Option<AssetKey>,
}
