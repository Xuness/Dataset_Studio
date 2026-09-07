mod compiler;
mod fields;
use crate::{
    SourceRouter,
    danbooru::{Catalog, err},
    demo_asset,
    duckdb::{Runtime, Session},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use studio_application::{QueryAdapter, SourceAdapter};
use studio_domain::*;

#[derive(Default)]
pub struct QueryReader {
    runtime: Runtime,
}
impl QueryReader {
    pub fn new(dll: PathBuf) -> Self {
        Self {
            runtime: Runtime::new(dll),
        }
    }
    /// Diagnostic only: explain the same controlled compiler used by the worker.
    pub fn explain(&self, source: &Source, spec: QuerySpec) -> Result<serde_json::Value> {
        let spec = spec.normalize()?;
        self.fields(source)?.validate(&spec)?;
        if source.kind != "danbooru" {
            return Err(Error::new(
                "QUERY_UNSUPPORTED",
                "查询计划诊断用于 Danbooru 索引",
            ));
        }
        let catalog = Catalog::open(source)?;
        let storage_sql = format!(
            "SELECT sha256 FROM objects WHERE {} ORDER BY sha256",
            compiler::storage_predicates(&spec)?
        );
        let storage_plan = catalog
            .connection()
            .prepare(&format!("EXPLAIN QUERY PLAN {storage_sql}"))
            .map_err(err)?
            .query_map([], |r| r.get::<_, String>(3))
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let metadata = if spec.uses_metadata() {
            let db = self.runtime.open(&catalog.analysis_path()?)?;
            let sequence = analysis_sequence(&db, &catalog)?;
            let sql = compiler::metadata_sql(&spec)?;
            let plan = db.query(&format!("EXPLAIN {sql}"))?;
            Some(serde_json::json!({"sql":sql,"plan":plan,"analysis_sequence":sequence}))
        } else {
            None
        };
        catalog.verify_unchanged(source)?;
        Ok(
            serde_json::json!({"spec":spec,"catalog_revision":catalog.revision,"storage_sql":storage_sql,"storage_plan":storage_plan,"metadata":metadata,"limits":{"native_memory":"256MB","native_threads":1,"source_budget_seconds":600,"batch_rows":512},"consistency":"read transactions plus matching watermarks and end fences; not a historical snapshot"}),
        )
    }
}
fn analysis_sequence(db: &Session, catalog: &Catalog) -> Result<String> {
    db.query("BEGIN TRANSACTION")?;
    let rows = db.query("SELECT CAST(MAX(seq) AS VARCHAR) FROM applied")?;
    let sequence = rows
        .first()
        .and_then(|r| r.first())
        .and_then(|v| v.clone())
        .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "分析索引缺少水位"))?;
    if sequence != catalog.sequence.to_string() {
        return Err(Error::new(
            "SOURCE_CHANGED",
            "存储与分析索引水位不同，请等待来源更新完成",
        ));
    }
    Ok(sequence)
}
fn version(source: &Source, catalog: &Catalog, sequence: Option<String>) -> QuerySourceVersion {
    QuerySourceVersion {
        source_id: source.id.clone(),
        catalog_revision: catalog.revision.clone(),
        consistency: if sequence.is_some() {
            "request_transactions_matched_watermarks".into()
        } else {
            "catalog_read_transaction".into()
        },
        analysis_sequence: sequence,
    }
}
fn check(cancelled: &AtomicBool, start: Instant) -> Result<()> {
    if cancelled.load(Ordering::Acquire) {
        return Err(Error::new("CANCELLED", "查询构建已取消"));
    }
    if start.elapsed() >= Duration::from_secs(600) {
        return Err(Error::new(
            "SOURCE_TIMEOUT",
            "单个来源查询超过 10 分钟预算，请缩小条件后重试",
        ));
    }
    Ok(())
}
fn assert_version(current: &QuerySourceVersion, expected: &QuerySourceVersion) -> Result<()> {
    if current != expected {
        return Err(Error::new(
            "SOURCE_CHANGED",
            "查询依据的来源版本已变化，请重新计算",
        ));
    }
    Ok(())
}
impl QueryAdapter for QueryReader {
    fn fields(&self, source: &Source) -> Result<FieldDirectory> {
        fields::directory(source)
    }
    fn query_version(&self, source: &Source, spec: &QuerySpec) -> Result<QuerySourceVersion> {
        self.fields(source)?.validate(spec)?;
        if source.kind == "demo" {
            return Ok(QuerySourceVersion {
                source_id: source.id.clone(),
                catalog_revision: SourceRouter.probe(source)?.revision,
                analysis_sequence: None,
                consistency: "immutable_demo".into(),
            });
        }
        let catalog = Catalog::open(source)?;
        let sequence = if spec.uses_metadata() {
            let db = self.runtime.open(&catalog.analysis_path()?)?;
            Some(analysis_sequence(&db, &catalog)?)
        } else {
            None
        };
        catalog.verify_unchanged(source)?;
        Ok(version(source, &catalog, sequence))
    }
    fn execute_query(
        &self,
        source: &Source,
        spec: &QuerySpec,
        expected: &QuerySourceVersion,
        cancelled: Arc<AtomicBool>,
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<()> {
        let start = Instant::now();
        self.fields(source)?.validate(spec)?;
        check(&cancelled, start)?;
        if source.kind == "demo" {
            assert_version(&self.query_version(source, spec)?, expected)?;
            // Use the same NULL/comparison semantics as catalog queries in the small demo.
            let db = rusqlite::Connection::open_in_memory().map_err(err)?;
            db.execute_batch(
                "CREATE TABLE objects(sha256 TEXT PRIMARY KEY,length INTEGER,stored_ext TEXT)",
            )
            .map_err(err)?;
            for n in 1..=32 {
                let asset = demo_asset(source, n);
                db.execute(
                    "INSERT INTO objects VALUES (?1,?2,?3)",
                    rusqlite::params![asset.key.asset_id, asset.bytes as i64, asset.extension],
                )
                .map_err(err)?;
            }
            let mut statement = db
                .prepare(&format!(
                    "SELECT sha256 FROM objects WHERE {} ORDER BY sha256",
                    compiler::storage_predicates(spec)?
                ))
                .map_err(err)?;
            let keys = statement
                .query_map([], |r| {
                    Ok(AssetKey {
                        source_id: source.id.clone(),
                        asset_id: r.get(0)?,
                    })
                })
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            check(&cancelled, start)?;
            return sink(&keys, keys.len() as u64);
        }
        let catalog = Catalog::open(source)?;
        let predicates = compiler::storage_predicates(spec)?;
        if spec.uses_metadata() {
            let db = self
                .runtime
                .open_query(&catalog.analysis_path()?, cancelled.clone())?;
            let sequence = analysis_sequence(&db, &catalog)?;
            assert_version(&version(source, &catalog, Some(sequence)), expected)?;
            let mut lookup = catalog
                .connection()
                .prepare(&format!(
                    "SELECT 1 FROM objects WHERE sha256=?1 AND {predicates}"
                ))
                .map_err(err)?;
            db.stream_ids(&compiler::metadata_sql(spec)?, &mut |ids| {
                check(&cancelled, start)?;
                let mut keys = Vec::with_capacity(ids.len());
                for id in ids {
                    if id.len() != 64
                        || !id
                            .bytes()
                            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                    {
                        return Err(Error::new(
                            "SOURCE_FORMAT_ERROR",
                            "分析索引返回了无效的存储身份",
                        ));
                    }
                    if lookup.exists([id]).map_err(err)? {
                        keys.push(AssetKey {
                            source_id: source.id.clone(),
                            asset_id: id.clone(),
                        });
                    }
                }
                sink(&keys, ids.len() as u64)
            })?;
        } else {
            assert_version(&version(source, &catalog, None), expected)?;
            let cancel = cancelled.clone();
            catalog
                .connection()
                .progress_handler(
                    10000,
                    Some(move || {
                        cancel.load(Ordering::Acquire)
                            || start.elapsed() >= Duration::from_secs(600)
                    }),
                )
                .map_err(err)?;
            let mut statement = catalog
                .connection()
                .prepare(&format!(
                    "SELECT sha256 FROM objects WHERE {predicates} ORDER BY sha256"
                ))
                .map_err(err)?;
            let mut rows = statement.query([]).map_err(err)?;
            let mut keys = Vec::with_capacity(512);
            loop {
                let row = rows.next();
                check(&cancelled, start)?;
                let Some(row) = row.map_err(err)? else {
                    break;
                };
                keys.push(AssetKey {
                    source_id: source.id.clone(),
                    asset_id: row.get(0).map_err(err)?,
                });
                if keys.len() == 512 {
                    sink(&keys, keys.len() as u64)?;
                    keys.clear();
                }
            }
            if !keys.is_empty() {
                sink(&keys, keys.len() as u64)?;
            }
        }
        check(&cancelled, start)?;
        catalog.verify_unchanged(source)
    }
}

#[cfg(test)]
mod tests;
