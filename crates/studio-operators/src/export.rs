//! File export plan. The operator only names each frozen member; the engine
//! export service reads originals and writes the destination folder.
use super::*;

pub const EXPORT_OPERATOR: &str = "core.export_files";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportMetadata {
    #[default]
    None,
    Tags,
    Full,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TagStyle {
    /// `long hair, 1girl`
    #[default]
    CommaSpaces,
    /// `long_hair, 1girl`
    Comma,
    /// `long_hair 1girl`
    Space,
}
impl TagStyle {
    pub fn format(self, tags: &[String]) -> String {
        match self {
            Self::CommaSpaces => tags
                .iter()
                .map(|t| t.replace('_', " "))
                .collect::<Vec<_>>()
                .join(", "),
            Self::Comma => tags.join(", "),
            Self::Space => tags.join(" "),
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportParameters {
    pub destination: String,
    #[serde(default)]
    pub metadata: ExportMetadata,
    #[serde(default)]
    pub tag_style: TagStyle,
    #[serde(default = "yes")]
    pub manifest: bool,
}
fn yes() -> bool {
    true
}
impl ExportParameters {
    pub fn parse(parameters: &Value) -> Result<Self> {
        let mut params: Self = serde_json::from_value(parameters.clone())
            .map_err(|_| Error::invalid("导出参数需要目标文件夹、元数据级别和标签格式"))?;
        params.destination = params.destination.trim().to_owned();
        let path = std::path::Path::new(&params.destination);
        if params.destination.is_empty() || !path.is_absolute() {
            return Err(Error::invalid("导出目标必须是绝对路径的文件夹"));
        }
        if params.destination.len() > 1024 {
            return Err(Error::invalid("导出目标路径过长"));
        }
        Ok(params)
    }
}

/// `000001_original.png`: the frozen ordinal keeps names unique and stable.
pub fn planned_file_name(ordinal: u64, asset: &Asset) -> String {
    let extension = sanitize(&asset.extension.to_ascii_lowercase());
    let mut stem = sanitize(&asset.name);
    if !extension.is_empty() {
        let suffix = format!(".{extension}");
        if stem.to_ascii_lowercase().ends_with(&suffix) {
            stem.truncate(stem.len() - suffix.len());
        }
    }
    let stem = truncate(stem.trim_end_matches(['.', ' ']), 120);
    let stem = if stem.is_empty() {
        asset.key.asset_id.chars().take(16).collect()
    } else {
        stem
    };
    if extension.is_empty() {
        format!("{:06}_{stem}", ordinal + 1)
    } else {
        format!("{:06}_{stem}.{extension}", ordinal + 1)
    }
}
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect()
}
fn truncate(text: &str, max_bytes: usize) -> String {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

pub(crate) struct Export;
impl Operator for Export {
    fn descriptor(&self) -> OperatorDescriptor {
        let mut descriptor = descriptor(
            EXPORT_OPERATOR,
            "导出原图",
            "file_export",
            vec![
                ParameterDescriptor {
                    id: "destination".into(),
                    name: "目标文件夹".into(),
                    value_type: "directory".into(),
                    default_value: json!(""),
                    required: true,
                },
                ParameterDescriptor {
                    id: "metadata".into(),
                    name: "附带元数据".into(),
                    value_type: "enum".into(),
                    default_value: json!("none"),
                    required: false,
                },
                ParameterDescriptor {
                    id: "tag_style".into(),
                    name: "标签格式".into(),
                    value_type: "enum".into(),
                    default_value: json!("comma_spaces"),
                    required: false,
                },
                ParameterDescriptor {
                    id: "manifest".into(),
                    name: "写入 manifest.jsonl".into(),
                    value_type: "boolean".into(),
                    default_value: json!(true),
                    required: false,
                },
            ],
        );
        descriptor.outputs[0].name = "导出计划".into();
        descriptor.resources.media_reads = true;
        descriptor
    }
    fn normalize(&self, parameters: Value) -> Result<Value> {
        serde_json::to_value(ExportParameters::parse(&parameters)?).map_err(Error::io)
    }
    fn required_fields(&self, parameters: &Value) -> Result<Vec<ScalarInput>> {
        ExportParameters::parse(parameters)?;
        Ok(Vec::new())
    }
    fn row(&self, input: &FrozenInput, ordinal: u64, parameters: &Value) -> Result<Value> {
        ExportParameters::parse(parameters)?;
        Ok(json!({
            "schema_version": 1,
            "ordinal": ordinal,
            "asset": input.asset,
            "source_revision": input.source_revision,
            "file": planned_file_name(ordinal, &input.asset),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn asset(name: &str, extension: &str) -> Asset {
        Asset {
            key: AssetKey {
                source_id: "s".into(),
                asset_id: "0123456789abcdef0123".into(),
            },
            name: name.into(),
            bytes: 1,
            extension: extension.into(),
            source_name: "lake".into(),
        }
    }
    #[test]
    fn file_names_are_numbered_safe_and_keep_one_extension() {
        assert_eq!(
            planned_file_name(0, &asset("12345_p0.PNG", "png")),
            "000001_12345_p0.png"
        );
        assert_eq!(
            planned_file_name(41, &asset("a:b/c?.jpg", "jpg")),
            "000042_a_b_c_.jpg"
        );
        assert_eq!(
            planned_file_name(2, &asset("...", "webp")),
            "000003_0123456789abcdef.webp"
        );
        let long = "长".repeat(100);
        let name = planned_file_name(0, &asset(&long, "png"));
        assert!(name.len() <= 7 + 120 + 4 && name.ends_with(".png"));
    }
    #[test]
    fn tag_styles_and_parameters() {
        let tags = vec!["long_hair".to_owned(), "1girl".to_owned()];
        assert_eq!(TagStyle::CommaSpaces.format(&tags), "long hair, 1girl");
        assert_eq!(TagStyle::Comma.format(&tags), "long_hair, 1girl");
        assert_eq!(TagStyle::Space.format(&tags), "long_hair 1girl");
        assert!(ExportParameters::parse(&json!({"destination": "relative"})).is_err());
        assert!(ExportParameters::parse(&json!({"destination": "", "metadata": "tags"})).is_err());
    }
}
