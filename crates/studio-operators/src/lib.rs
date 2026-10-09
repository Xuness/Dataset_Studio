//! Built-in computations have no source, filesystem, HTTP, or window access.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use studio_application::{Operator, OperatorRegistry};
use studio_domain::*;
pub mod export;
pub mod ranking;
pub mod ranking_v2;

pub fn registry() -> Result<OperatorRegistry> {
    let mut registry = OperatorRegistry::default();
    registry.register(Arc::new(Manifest))?;
    registry.register(Arc::new(Scalar))?;
    registry.register(Arc::new(export::Export))?;
    registry.register(Arc::new(ranking::MetaRecall))?;
    registry.register(Arc::new(ranking_v2::MetaRecallV2))?;
    Ok(registry)
}
fn descriptor(
    id: &str,
    name: &str,
    kind: &str,
    parameters: Vec<ParameterDescriptor>,
) -> OperatorDescriptor {
    OperatorDescriptor {
        id: id.into(),
        name: name.into(),
        version: 1,
        parameters_version: 1,
        parameters,
        input_scopes: ["source", "workset", "query_result", "selection"]
            .map(String::from)
            .into(),
        outputs: vec![OutputDescriptor {
            id: "data".into(),
            name: if kind == "manifest" {
                "数据清单"
            } else {
                "标量值"
            }
            .into(),
            kind: kind.into(),
            schema_version: 1,
            subject: "asset".into(),
        }],
        capabilities: OperatorCapabilities {
            cancel: true,
            checkpoint: true,
            retry: true,
            deterministic: true,
            item_failures: true,
        },
        resources: ResourceRequirements {
            cpu_slots: 1,
            memory_bytes: 32 * 1024 * 1024,
            media_reads: false,
            gpu: false,
        },
    }
}
fn fields(fields: Vec<ScalarInput>) -> Result<Vec<ScalarInput>> {
    if fields.len() > 8 {
        return Err(Error::invalid("每个算子最多固定 8 个字段"));
    }
    let mut unique = Vec::new();
    for field in fields {
        field.validate()?;
        if !unique.contains(&field) {
            unique.push(field);
        }
    }
    Ok(unique)
}
fn require<'a>(input: &'a FrozenInput, field: &ScalarInput) -> Result<&'a FrozenField> {
    input
        .fields
        .iter()
        .find(|f| &f.input == field)
        .ok_or_else(|| Error::new("INPUT_FIELD_MISSING", "固定输入缺少算子声明的必需字段"))
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestParameters {
    #[serde(default)]
    fields: Vec<ScalarInput>,
}
struct Manifest;
impl Operator for Manifest {
    fn descriptor(&self) -> OperatorDescriptor {
        descriptor(
            "core.manifest",
            "生成数据清单",
            "manifest",
            vec![ParameterDescriptor {
                id: "fields".into(),
                name: "附加固定字段".into(),
                value_type: "scalar_inputs".into(),
                default_value: json!([]),
                required: false,
            }],
        )
    }
    fn normalize(&self, parameters: Value) -> Result<Value> {
        let mut params: ManifestParameters = serde_json::from_value(parameters)
            .map_err(|_| Error::invalid("清单参数需要 fields 字段列表"))?;
        params.fields = fields(params.fields)?;
        serde_json::to_value(params).map_err(Error::io)
    }
    fn required_fields(&self, parameters: &Value) -> Result<Vec<ScalarInput>> {
        let params: ManifestParameters =
            serde_json::from_value(self.normalize(parameters.clone())?).map_err(Error::io)?;
        Ok(params.fields)
    }
    fn row(&self, input: &FrozenInput, ordinal: u64, parameters: &Value) -> Result<Value> {
        let fields = self
            .required_fields(parameters)?
            .iter()
            .map(|field| require(input, field))
            .collect::<Result<Vec<_>>>()?;
        let mut row = json!({"schema_version":1,"ordinal":ordinal,"asset":input.asset,"source_revision":input.source_revision});
        if !fields.is_empty() {
            row["fields"] = json!(fields);
        }
        Ok(row)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScalarParameters {
    input: ScalarInput,
    #[serde(default = "one")]
    multiplier: String,
    #[serde(default = "zero")]
    addend: String,
}
fn one() -> String {
    "1".into()
}
fn zero() -> String {
    "0".into()
}
fn integer(text: &str) -> Result<i64> {
    text.parse()
        .map_err(|_| Error::invalid("参数必须是 64 位整数的十进制字符串"))
}
struct Scalar;
impl Operator for Scalar {
    fn descriptor(&self) -> OperatorDescriptor {
        let mut descriptor = descriptor(
            "core.scalar",
            "计算标量字段",
            "scalar_columns",
            vec![
                ParameterDescriptor {
                    id: "input".into(),
                    name: "输入字段".into(),
                    value_type: "scalar_input".into(),
                    default_value: json!({"kind":"stored_bytes"}),
                    required: true,
                },
                ParameterDescriptor {
                    id: "multiplier".into(),
                    name: "乘数".into(),
                    value_type: "integer".into(),
                    default_value: json!("1"),
                    required: false,
                },
                ParameterDescriptor {
                    id: "addend".into(),
                    name: "加数".into(),
                    value_type: "integer".into(),
                    default_value: json!("0"),
                    required: false,
                },
            ],
        );
        descriptor.outputs.push(OutputDescriptor {
            id: "failures".into(),
            name: "单项失败".into(),
            kind: "item_failures".into(),
            schema_version: 1,
            subject: "asset".into(),
        });
        descriptor
    }
    fn output_row(&self, output_id: &str, row: &Value) -> Result<Option<Value>> {
        match output_id {
            "data" => Ok(Some(row.clone())),
            "failures" => Ok((row["scalar"]["status"] == "failed").then(|| row.clone())),
            _ => Err(Error::new("OUTPUT_UNSUPPORTED", "算子未声明该成果输出")),
        }
    }
    fn normalize(&self, parameters: Value) -> Result<Value> {
        let mut params: ScalarParameters = serde_json::from_value(parameters)
            .map_err(|_| Error::invalid("标量参数需要输入字段、整数乘数和加数"))?;
        params.input.validate()?;
        params.multiplier = integer(&params.multiplier)?.to_string();
        params.addend = integer(&params.addend)?.to_string();
        serde_json::to_value(params).map_err(Error::io)
    }
    fn required_fields(&self, parameters: &Value) -> Result<Vec<ScalarInput>> {
        let params: ScalarParameters =
            serde_json::from_value(self.normalize(parameters.clone())?).map_err(Error::io)?;
        Ok(vec![params.input])
    }
    fn row(&self, input: &FrozenInput, ordinal: u64, parameters: &Value) -> Result<Value> {
        let params: ScalarParameters =
            serde_json::from_value(self.normalize(parameters.clone())?).map_err(Error::io)?;
        let field = require(input, &params.input)?;
        let value = if let Some(value) = field.value.as_integer()? {
            match value
                .checked_mul(integer(&params.multiplier)?)
                .and_then(|v| v.checked_add(integer(&params.addend).ok()?))
            {
                Some(value) => ScalarValue::integer(value),
                None => ScalarValue::Failed {
                    code: "INTEGER_OVERFLOW".into(),
                    message: "标量计算超出 64 位整数范围".into(),
                },
            }
        } else {
            field.value.clone()
        };
        Ok(
            json!({"schema_version":1,"ordinal":ordinal,"asset":input.asset,"source_revision":input.source_revision,"scalar":value,"basis":[field.basis]}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(value: ScalarValue) -> FrozenInput {
        FrozenInput {
            asset: Asset {
                key: AssetKey {
                    source_id: new_id(),
                    asset_id: "object".into(),
                },
                name: "fixture".into(),
                bytes: 4,
                extension: "png".into(),
                source_name: "fixture".into(),
            },
            source_revision: "fixed-v1".into(),
            fields: vec![FrozenField {
                input: ScalarInput::StoredBytes,
                value,
                basis: FieldBasis {
                    field_id: "stored.bytes".into(),
                    subject: "asset".into(),
                    rule: "stored_object".into(),
                    source_version: Some("fixed-v1".into()),
                    record_id: None,
                    observation_id: None,
                    artifact_id: None,
                },
            }],
        }
    }
    #[test]
    fn registry_and_parameter_versions_are_enforced() {
        let mut registry = registry().unwrap();
        assert_eq!(registry.descriptors().len(), 5);
        assert!(
            registry
                .descriptors()
                .iter()
                .any(|d| d.id == RANKING_OPERATOR)
        );
        assert!(
            registry
                .descriptors()
                .iter()
                .any(|d| d.id == RANKING_V2_OPERATOR)
        );
        assert_eq!(
            registry.register(Arc::new(Manifest)).unwrap_err().code,
            "OPERATOR_DUPLICATE"
        );
        assert_eq!(
            registry
                .resolve(&OperatorRun {
                    operator_version: 99,
                    ..Default::default()
                })
                .err()
                .unwrap()
                .code,
            "OPERATOR_UNAVAILABLE"
        );
        assert_eq!(
            registry
                .resolve(&OperatorRun {
                    parameters_version: 2,
                    ..Default::default()
                })
                .err()
                .unwrap()
                .code,
            "PARAMETERS_VERSION_UNSUPPORTED"
        );
        assert!(
            Scalar
                .normalize(json!({"input":{"kind":"stored_bytes"},"multiplier":"bad"}))
                .is_err()
        );
        assert!(Manifest.normalize(json!({"unknown":true})).is_err());
        assert!(
            Scalar
                .row(
                    &FrozenInput {
                        fields: vec![],
                        ..input(ScalarValue::integer(4))
                    },
                    0,
                    &json!({"input":{"kind":"stored_bytes"}})
                )
                .is_err()
        );
    }
    #[test]
    fn scalar_values_missing_and_failures_remain_distinct() {
        let params = json!({"input":{"kind":"stored_bytes"},"multiplier":"3","addend":"-2"});
        let row = Scalar
            .row(&input(ScalarValue::integer(4)), 0, &params)
            .unwrap();
        assert_eq!(row["scalar"], json!({"status":"available","value":"10"}));
        for value in [
            ScalarValue::Missing {
                reason: "not recorded".into(),
            },
            ScalarValue::Uncomputed {
                reason: "outside coverage".into(),
            },
        ] {
            assert_eq!(
                Scalar.row(&input(value.clone()), 0, &params).unwrap()["scalar"],
                json!(value)
            );
        }
        assert_eq!(
            Scalar
                .row(&input(ScalarValue::integer(i64::MAX)), 0, &params)
                .unwrap()["scalar"]["code"],
            "INTEGER_OVERFLOW"
        );
        assert_eq!(
            Manifest
                .row(&input(ScalarValue::integer(4)), 0, &json!({}))
                .unwrap()["asset"]["bytes"],
            4
        );
    }
}
