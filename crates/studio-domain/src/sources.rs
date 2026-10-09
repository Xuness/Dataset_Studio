//! Source contracts describe capabilities, not a closed list of websites.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCapabilities {
    pub browse: bool,
    pub media: bool,
    pub metadata: bool,
    pub query: bool,
    pub post_order: bool,
    #[serde(default)]
    pub identity_summaries: bool,
    pub relink: bool,
    pub raw_metadata: bool,
    pub incremental: bool,
    pub stored_dimensions: bool,
    #[serde(default)]
    pub work_members: bool,
    #[serde(default)]
    pub author_metadata: bool,
    #[serde(default)]
    pub literal_tags: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDescriptor {
    pub version: u32,
    pub backend_id: String,
    pub display_name: String,
    pub site_id: Option<String>,
    pub semantics_version: String,
    pub capabilities: SourceCapabilities,
    pub projections: Vec<String>,
}

impl SourceDescriptor {
    pub fn require_projection(&self, name: &str) -> Result<()> {
        if self.projections.iter().any(|v| v == name) {
            Ok(())
        } else {
            Err(Error::new(
                "RANKING_SOURCE_UNSUPPORTED",
                format!("该来源不支持所需元数据投影：{name}"),
            ))
        }
    }
}

/// A canonical producer's continuity proof. It is independent of a DB connection.
#[derive(Debug, Clone)]
pub struct ChangeAnchor {
    pub generation: String,
    pub sequence: u64,
    pub batch_id: String,
}

/// Tags in canonical metadata are separated by U+0020 only. Other whitespace
/// can be part of a literal source token. Keep the query input bounded.
pub fn valid_source_tag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains(' ')
        && !value.chars().any(|c| {
            c.is_control()
                && !matches!(c, '\t' | '\n' | '\r' | '\u{000b}' | '\u{000c}' | '\u{0085}')
        })
}

pub fn source_tags(value: &str) -> Vec<String> {
    value
        .split(' ')
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_tags_round_trip_without_unicode_whitespace_normalization() {
        for tag in [
            "yaoi\u{3000}translated",
            "large_breasts\tlong_hair",
            "censored\n",
            "a\u{00a0}b",
        ] {
            assert!(valid_source_tag(tag));
            assert_eq!(
                source_tags(&format!("solo {tag} test")),
                ["solo", tag, "test"]
            );
        }
        for tag in ["", "two tags", "a\0b", "a\u{001b}b"] {
            assert!(!valid_source_tag(tag));
        }
    }
}
