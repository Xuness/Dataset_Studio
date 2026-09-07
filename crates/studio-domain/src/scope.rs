use crate::{Error, Result, validate_id};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeRef {
    pub project_id: String,
    pub target: ScopeTarget,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScopeTarget {
    Source { source_id: String, revision: String },
    Workset { collection_id: String },
    QueryResult { result_id: String },
    Selection { revision: u64 },
}
impl ScopeRef {
    pub fn validate_project(&self, project_id: &str) -> Result<()> {
        validate_id(&self.project_id)?;
        if self.project_id != project_id {
            return Err(Error::new(
                "SCOPE_PROJECT_MISMATCH",
                "数据范围不属于当前项目",
            ));
        }
        match &self.target {
            ScopeTarget::Source {
                source_id,
                revision,
            } => {
                validate_id(source_id)?;
                if revision.is_empty() || revision.len() > 512 {
                    return Err(Error::invalid("来源范围需要有效的版本"));
                }
            }
            ScopeTarget::Workset { collection_id } => validate_id(collection_id)?,
            ScopeTarget::QueryResult { result_id } => validate_id(result_id)?,
            ScopeTarget::Selection { revision } if *revision > i64::MAX as u64 => {
                return Err(Error::invalid("选择版本无效"));
            }
            ScopeTarget::Selection { .. } => {}
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeOperation {
    Replace,
    Add,
    Remove,
    Intersect,
}
