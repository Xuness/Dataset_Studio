use crate::AssetPage;
use serde::{Deserialize, Serialize};
use studio_domain as domain;
use utoipa::ToSchema;

macro_rules! enum_model {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug,Clone,Serialize,Deserialize,ToSchema)]
        #[serde(rename_all="snake_case")]
        pub enum $name { $($variant),+ }
        impl From<domain::$name> for $name { fn from(value:domain::$name)->Self { match value { $(domain::$name::$variant=>Self::$variant),+ } } }
        impl From<$name> for domain::$name { fn from(value:$name)->Self { match value { $($name::$variant=>Self::$variant),+ } } }
    }
}
enum_model!(FieldType {
    Text,
    Integer,
    Boolean,
    Tags
});
enum_model!(QueryOperator {
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
    IsPresent
});
enum_model!(ObservationRule {
    CurrentPost,
    AnyObservation
});
enum_model!(QueryOrder {
    AssetKeyAsc,
    AssetKeyDesc,
    PostIdAsc,
    PostIdDesc
});
enum_model!(ResultState {
    Queued,
    Running,
    Ready,
    Cancelled,
    Failed,
    Interrupted,
    Released
});

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
impl From<domain::QueryValue> for QueryValue {
    fn from(v: domain::QueryValue) -> Self {
        match v {
            domain::QueryValue::Text(v) => Self::Text(v),
            domain::QueryValue::Integer(v) => Self::Integer(v),
            domain::QueryValue::Boolean(v) => Self::Boolean(v),
            domain::QueryValue::TextList(v) => Self::TextList(v),
        }
    }
}
impl From<QueryValue> for domain::QueryValue {
    fn from(v: QueryValue) -> Self {
        match v {
            QueryValue::Text(v) => Self::Text(v),
            QueryValue::Integer(v) => Self::Integer(v),
            QueryValue::Boolean(v) => Self::Boolean(v),
            QueryValue::TextList(v) => Self::TextList(v),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryCondition {
    pub field: String,
    pub operator: QueryOperator,
    pub value: Option<QueryValue>,
}
impl From<domain::QueryCondition> for QueryCondition {
    fn from(c: domain::QueryCondition) -> Self {
        Self {
            field: c.field,
            operator: c.operator.into(),
            value: c.value.map(Into::into),
        }
    }
}
impl From<QueryCondition> for domain::QueryCondition {
    fn from(c: QueryCondition) -> Self {
        Self {
            field: c.field,
            operator: c.operator.into(),
            value: c.value.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct QuerySpec {
    pub version: u32,
    pub source_ids: Vec<String>,
    pub conditions: Vec<QueryCondition>,
    pub observation_rule: ObservationRule,
    pub order: QueryOrder,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_scope: Option<crate::ScopeRef>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RunQuery {
    pub spec: QuerySpec,
}
impl From<domain::QuerySpec> for QuerySpec {
    fn from(q: domain::QuerySpec) -> Self {
        Self {
            version: q.version,
            source_ids: q.source_ids,
            conditions: q.conditions.into_iter().map(Into::into).collect(),
            observation_rule: q.observation_rule.into(),
            order: q.order.into(),
            input_scope: q.input_scope.map(Into::into),
        }
    }
}
impl From<QuerySpec> for domain::QuerySpec {
    fn from(q: QuerySpec) -> Self {
        Self {
            version: q.version,
            source_ids: q.source_ids,
            conditions: q.conditions.into_iter().map(Into::into).collect(),
            observation_rule: q.observation_rule.into(),
            order: q.order.into(),
            input_scope: q.input_scope.map(Into::into),
        }
    }
}

#[derive(Serialize, ToSchema)]
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
impl From<domain::FieldDefinition> for FieldDefinition {
    fn from(f: domain::FieldDefinition) -> Self {
        Self {
            id: f.id,
            name: f.name,
            field_type: f.field_type.into(),
            unit: f.unit,
            missing: f.missing,
            basis: f.basis,
            display: f.display,
            operators: f.operators.into_iter().map(Into::into).collect(),
            sortable: f.sortable,
            cost: f.cost,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct FieldDirectory {
    pub version: u32,
    pub source_id: String,
    pub fields: Vec<FieldDefinition>,
    pub observation_rules: Vec<ObservationRule>,
    pub orders: Vec<QueryOrder>,
    pub max_conditions: usize,
    pub direct_query: bool,
}
impl From<domain::FieldDirectory> for FieldDirectory {
    fn from(f: domain::FieldDirectory) -> Self {
        Self {
            version: f.version,
            source_id: f.source_id,
            fields: f.fields.into_iter().map(Into::into).collect(),
            observation_rules: f.observation_rules.into_iter().map(Into::into).collect(),
            orders: f.orders.into_iter().map(Into::into).collect(),
            max_conditions: f.max_conditions,
            direct_query: f.direct_query,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct QuerySourceVersion {
    pub source_id: String,
    pub semantics_version: Option<String>,
    pub catalog_revision: String,
    pub analysis_sequence: Option<String>,
    pub consistency: String,
}
impl From<domain::QuerySourceVersion> for QuerySourceVersion {
    fn from(v: domain::QuerySourceVersion) -> Self {
        Self {
            source_id: v.source_id,
            semantics_version: v.semantics_version,
            catalog_revision: v.catalog_revision,
            analysis_sequence: v.analysis_sequence,
            consistency: v.consistency,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct QueryDefinition {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub revision: u64,
    pub spec: QuerySpec,
    pub created_at: String,
}
impl From<domain::QueryDefinition> for QueryDefinition {
    fn from(q: domain::QueryDefinition) -> Self {
        Self {
            id: q.id,
            project_id: q.project_id,
            name: q.name,
            revision: q.revision,
            spec: q.spec.into(),
            created_at: q.created_at,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct QueryDefinitions {
    pub items: Vec<QueryDefinition>,
    pub next_cursor: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveQuery {
    pub name: String,
    pub spec: QuerySpec,
    pub expected_revision: Option<u64>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildQuery {
    pub expected_revision: u64,
}
#[derive(Deserialize, ToSchema)]
pub struct QueryListParams {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub order: Option<QueryOrder>,
}

#[derive(Serialize, ToSchema)]
pub struct QueryResult {
    pub id: String,
    pub project_id: String,
    pub definition_id: Option<String>,
    pub definition_revision: Option<u64>,
    pub spec: QuerySpec,
    pub source_versions: Vec<QuerySourceVersion>,
    pub state: ResultState,
    pub processed: u64,
    pub count: Option<u64>,
    pub created_at: String,
    pub error: Option<String>,
    pub cache: QueryCacheInfo,
}
#[derive(Serialize, ToSchema)]
pub struct QueryCacheInfo {
    pub mode: String,
    pub evaluated_objects: u64,
    pub changed_members: u64,
    pub tier: String,
    pub fixed: bool,
    pub session_only: bool,
    pub basis_ratings: Vec<String>,
    pub candidate_records: u64,
}
impl From<domain::QueryResult> for QueryResult {
    fn from(r: domain::QueryResult) -> Self {
        Self {
            id: r.id,
            project_id: r.project_id,
            definition_id: r.definition_id,
            definition_revision: r.definition_revision,
            spec: r.spec.into(),
            source_versions: r.source_versions.into_iter().map(Into::into).collect(),
            state: r.state.into(),
            processed: r.processed,
            count: r.count,
            created_at: r.created_at,
            error: r.error,
            cache: QueryCacheInfo {
                mode: r.cache.mode,
                evaluated_objects: r.cache.evaluated_objects,
                changed_members: r.cache.changed_members,
                tier: r.cache.tier.as_str().into(),
                fixed: r.cache.fixed,
                session_only: r.cache.session_only,
                basis_ratings: r.cache.basis_ratings,
                candidate_records: r.cache.candidate_records,
            },
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct QueryResults {
    pub items: Vec<QueryResult>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct ResultAssets {
    pub result_id: String,
    pub count: Option<u64>,
    pub page: AssetPage,
}
#[derive(Serialize, ToSchema)]
pub struct ResultValidity {
    pub result_id: String,
    pub current: bool,
    pub issue: Option<String>,
    pub newer_available: bool,
}
