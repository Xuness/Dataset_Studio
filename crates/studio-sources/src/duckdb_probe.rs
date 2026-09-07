use std::path::Path;
use studio_domain::*;
pub fn probe(dll: &Path, database: &Path) -> Result<String> {
    let db = crate::duckdb::Session::open(dll, database)?;
    db.query("SELECT version() || ' / applied=' || CAST(MAX(seq) AS VARCHAR) FROM applied")?
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next().flatten())
        .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "分析索引缺少水位"))
}
