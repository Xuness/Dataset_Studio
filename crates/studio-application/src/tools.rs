use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use studio_domain::*;

/// Operators only transform frozen input. Source and project I/O belong to services.
pub trait Operator: Send + Sync {
    fn descriptor(&self) -> OperatorDescriptor;
    fn normalize(&self, parameters: Value) -> Result<Value>;
    fn required_fields(&self, parameters: &Value) -> Result<Vec<ScalarInput>>;
    fn row(&self, input: &FrozenInput, ordinal: u64, parameters: &Value) -> Result<Value>;
    fn output_row(&self, output_id: &str, row: &Value) -> Result<Option<Value>> {
        if output_id == "data" {
            Ok(Some(row.clone()))
        } else {
            Err(Error::new("OUTPUT_UNSUPPORTED", "算子未实现声明的成果输出"))
        }
    }
}

#[derive(Default)]
pub struct OperatorRegistry {
    operators: BTreeMap<(String, u32), Arc<dyn Operator>>,
}
impl OperatorRegistry {
    pub fn register(&mut self, operator: Arc<dyn Operator>) -> Result<()> {
        let descriptor = operator.descriptor();
        if descriptor.id.is_empty()
            || descriptor.id.len() > 120
            || !descriptor
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || descriptor.version == 0
            || descriptor.parameters_version == 0
            || descriptor.outputs.is_empty()
        {
            return Err(Error::invalid("无效的算子注册描述"));
        }
        let key = (descriptor.id, descriptor.version);
        if self.operators.contains_key(&key) {
            return Err(Error::new("OPERATOR_DUPLICATE", "算子身份和版本已经注册"));
        }
        self.operators.insert(key, operator);
        Ok(())
    }
    pub fn descriptors(&self) -> Vec<OperatorDescriptor> {
        self.operators
            .values()
            .map(|operator| operator.descriptor())
            .collect()
    }
    pub fn resolve(&self, run: &OperatorRun) -> Result<Arc<dyn Operator>> {
        let operator = self
            .operators
            .get(&(run.operator_id.clone(), run.operator_version))
            .ok_or_else(|| Error::new("OPERATOR_UNAVAILABLE", "所需算子版本尚未注册"))?;
        if operator.descriptor().parameters_version != run.parameters_version {
            return Err(Error::new(
                "PARAMETERS_VERSION_UNSUPPORTED",
                "算子参数模式不兼容",
            ));
        }
        Ok(operator.clone())
    }
    pub fn normalize(&self, mut run: OperatorRun) -> Result<OperatorRun> {
        run.parameters = self.resolve(&run)?.normalize(run.parameters)?;
        Ok(run)
    }
}

pub trait ArtifactRepository: Send + Sync {
    fn artifacts(
        &self,
        project_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Artifact>>;
    fn artifact(&self, project_id: &str, artifact_id: &str) -> Result<Artifact>;
    fn artifact_page(
        &self,
        project_id: &str,
        artifact_id: &str,
        after: Option<&AssetKey>,
        limit: usize,
    ) -> Result<ArtifactPage>;
    fn artifact_scalar(
        &self,
        project_id: &str,
        artifact_id: &str,
        key: &AssetKey,
    ) -> Result<ScalarValue>;
}

pub trait DraftRepository: Send + Sync {
    fn draft(&self, project_id: &str, module_id: &str, instance_id: &str) -> Result<Option<Draft>>;
    fn save_draft(
        &self,
        project_id: &str,
        module_id: &str,
        instance_id: &str,
        request: SaveDraft,
    ) -> Result<Draft>;
    fn preference(&self, key: &str) -> Result<Option<Preference>>;
    fn save_preference(&self, key: &str, request: SaveDraft) -> Result<Preference>;
}
