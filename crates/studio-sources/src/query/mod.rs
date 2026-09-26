pub(crate) mod changes;
pub(crate) mod compiler;
mod fields;
use crate::{
    SourceRouter,
    canonical::{Catalog, err},
    demo_asset,
    duckdb::{Runtime, Session},
};
pub use changes::ChangeAnchor;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use studio_application::{QueryAdapter, SourceAdapter};
use studio_domain::*;

#[derive(Default)]
pub struct QueryReader {
    runtime: Runtime,
    rating_cache: Option<Arc<crate::RatingCache>>,
    candidate_rows: AtomicU64,
    candidate_ratings: Mutex<Vec<String>>,
}
impl QueryReader {
    pub fn with_deadline(mut self, deadline: Option<Instant>) -> Self {
        self.runtime = self.runtime.with_deadline(deadline);
        self
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Changed identities and matched identities have independent bounded sinks"
    )]
    pub fn execute_delta(
        &self,
        source: &Source,
        spec: &QuerySpec,
        expected: &QuerySourceVersion,
        previous: &ChangeAnchor,
        cancelled: Arc<AtomicBool>,
        affected: &mut dyn FnMut(&[AssetKey]) -> Result<()>,
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<bool> {
        if !crate::profiles::is_canonical(source) {
            return Ok(false);
        }
        self.fields(source)?.validate(spec)?;
        let catalog = Catalog::open(source)?;
        let db = self
            .runtime
            .open_query(&catalog.analysis_path()?, cancelled.clone())?;
        let sequence = analysis_sequence(&db, &catalog)?;
        let current = version(source, &catalog, spec.uses_metadata().then_some(sequence));
        assert_version(&current, expected)?;
        let Some(changed) = changes::change_sql(&db, &catalog, previous)? else {
            return Ok(false);
        };
        db.query(&format!("CREATE TEMP TABLE studio_changed AS {changed}"))?;
        db.stream_ids(
            "SELECT sha256 FROM studio_changed ORDER BY sha256",
            &mut |ids| {
                let keys = checked_keys(source, ids)?;
                affected(&keys)
            },
        )?;
        let sql = if spec.uses_metadata() {
            compiler::metadata_sql_for_changed(spec)?
        } else {
            format!(
                "SELECT sha256 FROM objects WHERE {} AND sha256 IN (SELECT sha256 FROM studio_changed)",
                compiler::storage_predicates(spec)?
            )
        };
        stream_native(&db, &catalog, source, spec, &sql, &cancelled, sink)?;
        catalog.verify_unchanged(source)?;
        Ok(true)
    }

    pub fn new(dll: PathBuf) -> Self {
        Self {
            runtime: Runtime::new(dll),
            ..Self::default()
        }
    }
    pub fn with_query_directory(path: PathBuf) -> Self {
        Self {
            runtime: Runtime::default().with_query_directory(path),
            ..Self::default()
        }
    }
    pub fn with_query_memory(mut self, bytes: u64) -> Self {
        self.runtime = self.runtime.with_query_memory(bytes);
        self
    }
    pub fn with_rating_cache(mut self, cache: Arc<crate::RatingCache>) -> Self {
        self.rating_cache = Some(cache);
        self
    }
    pub fn rating_usage(&self) -> Result<(Vec<String>, u64)> {
        Ok((
            self.candidate_ratings
                .lock()
                .map_err(|_| Error::new("INTERNAL_ERROR", "分级复用状态不可用"))?
                .clone(),
            self.candidate_rows.load(Ordering::Relaxed),
        ))
    }
    /// Diagnostic only: explain the same controlled compiler used by the worker.
    pub fn explain(&self, source: &Source, spec: QuerySpec) -> Result<serde_json::Value> {
        let spec = spec.normalize()?;
        self.fields(source)?.validate(&spec)?;
        if !crate::profiles::is_canonical(source) {
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
            serde_json::json!({"spec":spec,"catalog_revision":catalog.revision,"storage_sql":storage_sql,"storage_plan":storage_plan,"metadata":metadata,"limits":{"native_memory_bytes":QUERY_MEMORY_BYTES,"native_threads":2,"temporary_disk_bytes":QUERY_TEMP_BYTES,"source_budget_seconds":600,"batch_rows":512},"consistency":"read transactions plus matching watermarks and end fences; not a historical snapshot"}),
        )
    }
}
pub(crate) fn analysis_sequence(db: &Session, catalog: &Catalog) -> Result<String> {
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
        semantics_version: sequence.as_ref().and_then(|_| {
            crate::profiles::site(&source.kind)?
                .normalizer
                .map(str::to_owned)
        }),
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
    fn execute_query_keys(
        &self,
        source: &Source,
        spec: &QuerySpec,
        expected: &QuerySourceVersion,
        cancelled: Arc<AtomicBool>,
        keys: &[AssetKey],
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<()> {
        QueryReader::execute_query_keys(self, source, spec, expected, cancelled, keys, sink)
    }
    fn execute_delta(
        &self,
        source: &Source,
        spec: &QuerySpec,
        expected: &QuerySourceVersion,
        previous: &ChangeAnchor,
        cancelled: Arc<AtomicBool>,
        affected: &mut dyn FnMut(&[AssetKey]) -> Result<()>,
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<bool> {
        QueryReader::execute_delta(
            self, source, spec, expected, previous, cancelled, affected, sink,
        )
    }
    fn explain(&self, source: &Source, spec: QuerySpec) -> Result<serde_json::Value> {
        QueryReader::explain(self, source, spec)
    }
    fn rating_usage(&self) -> Result<(Vec<String>, u64)> {
        QueryReader::rating_usage(self)
    }
    fn fields(&self, source: &Source) -> Result<FieldDirectory> {
        fields::directory(source)
    }
    fn query_version(&self, source: &Source, spec: &QuerySpec) -> Result<QuerySourceVersion> {
        self.fields(source)?.validate(spec)?;
        self.read_version(source, spec.uses_metadata())
    }
    fn read_version(&self, source: &Source, metadata: bool) -> Result<QuerySourceVersion> {
        if source.kind == "demo" {
            return Ok(QuerySourceVersion {
                semantics_version: None,
                source_id: source.id.clone(),
                catalog_revision: SourceRouter.probe(source)?.revision,
                analysis_sequence: None,
                consistency: "immutable_demo".into(),
            });
        }
        let catalog = Catalog::open(source)?;
        let sequence = if metadata {
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
        self.execute_query_inner(source, spec, expected, cancelled, None, sink)
    }
}

impl QueryReader {
    pub fn execute_query_keys(
        &self,
        source: &Source,
        spec: &QuerySpec,
        expected: &QuerySourceVersion,
        cancelled: Arc<AtomicBool>,
        keys: &[AssetKey],
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<()> {
        if keys.is_empty() || keys.len() > 512 || keys.iter().any(|k| k.source_id != source.id) {
            return Err(Error::invalid("查询输入批次无效"));
        }
        self.execute_query_inner(source, spec, expected, cancelled, Some(keys), sink)
    }

    fn execute_query_inner(
        &self,
        source: &Source,
        spec: &QuerySpec,
        expected: &QuerySourceVersion,
        cancelled: Arc<AtomicBool>,
        keys: Option<&[AssetKey]>,
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<()> {
        let start = Instant::now();
        self.fields(source)?.validate(spec)?;
        check(&cancelled, start)?;
        let only_ids = keys.map(|keys| keys.iter().map(|k| k.asset_id.clone()).collect::<Vec<_>>());
        let mut predicates = compiler::storage_predicates(spec)?;
        if let Some(ids) = &only_ids {
            predicates.push_str(" AND ");
            predicates.push_str(&compiler::asset_predicate("sha256", ids)?);
        }
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
                    predicates
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
        if spec.uses_metadata() {
            let db = self
                .runtime
                .open_query(&catalog.analysis_path()?, cancelled.clone())?;
            let sequence = analysis_sequence(&db, &catalog)?;
            assert_version(&version(source, &catalog, Some(sequence)), expected)?;
            let sql = if keys.is_none()
                && let Some(cache) = &self.rating_cache
                && let Some(ratings) = crate::rating_candidates(spec)
            {
                let count = cache.import(
                    source,
                    &ratings,
                    &db,
                    &catalog.generation,
                    catalog.sequence,
                    spec,
                )?;
                self.candidate_rows.fetch_add(count, Ordering::Relaxed);
                let mut used = self
                    .candidate_ratings
                    .lock()
                    .map_err(|_| Error::new("INTERNAL_ERROR", "分级复用状态不可用"))?;
                used.extend(ratings);
                used.sort();
                used.dedup();
                if !crate::RatingCache::filters_metadata(spec) {
                    db.query(&compiler::prepare_rating_observations(spec)?)?;
                    compiler::metadata_sql_for_rating_candidates(spec)?
                } else {
                    "SELECT lower(hex(sha256)) FROM studio_rating_candidates ORDER BY sha256".into()
                }
            } else {
                compiler::metadata_sql_for_assets(spec, only_ids.as_deref())?
            };
            stream_native(&db, &catalog, source, spec, &sql, &cancelled, sink)?;
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

fn checked_keys(source: &Source, ids: &[String]) -> Result<Vec<AssetKey>> {
    ids.iter()
        .map(|id| {
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
            Ok(AssetKey {
                source_id: source.id.clone(),
                asset_id: id.clone(),
            })
        })
        .collect()
}
fn stream_native(
    db: &Session,
    catalog: &Catalog,
    source: &Source,
    spec: &QuerySpec,
    sql: &str,
    cancelled: &AtomicBool,
    sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
) -> Result<()> {
    let placeholders = std::iter::repeat_n("?", 512).collect::<Vec<_>>().join(",");
    let predicates = compiler::storage_predicates(spec)?;
    let mut lookup=catalog.connection().prepare(&format!("SELECT sha256 FROM objects WHERE sha256 IN ({placeholders}) AND {predicates} ORDER BY sha256")).map_err(err)?;
    db.stream_ids(sql, &mut |ids| {
        studio_application::read_cancelled(cancelled)?;
        checked_keys(source, ids)?;
        let parameters = ids
            .iter()
            .map(|id| Some(id.as_str()))
            .chain(std::iter::repeat_n(None, 512 - ids.len()));
        let keys = lookup
            .query_map(rusqlite::params_from_iter(parameters), |r| {
                Ok(AssetKey {
                    source_id: source.id.clone(),
                    asset_id: r.get(0)?,
                })
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        sink(&keys, ids.len() as u64)
    })
}

#[cfg(test)]
mod tests;
