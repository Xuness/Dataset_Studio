//! Bulk projection of complete representative observations from a read-only lake.
use crate::{canonical::Catalog, duckdb::Runtime, query::compiler::ranking_predicate};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
use studio_domain::*;

#[derive(Default)]
pub struct RankingReader {
    runtime: Runtime,
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
struct ProjectionBatch<'a> {
    rows: mpsc::Receiver<Vec<RankingInput>>,
    worker: thread::ScopedJoinHandle<'a, Result<()>>,
}
impl ProjectionBatch<'_> {
    fn drain(self, sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>) -> Result<()> {
        for rows in self.rows {
            sink(&rows)?;
        }
        self.worker
            .join()
            .map_err(|_| Error::new("INTERNAL_ERROR", "排名输入投影线程异常"))?
    }
}
impl RankingReader {
    pub fn project_source(
        &self,
        source: &Source,
        expected: &QuerySourceVersion,
        parameters: &RankingParameters,
        cancelled: Arc<AtomicBool>,
        sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>,
    ) -> Result<u64> {
        if expected.consistency != "retained_online_snapshot" || !crate::online::available(source) {
            return Err(Error::new(
                "RANKING_SOURCE_UNSUPPORTED",
                "全湖排名需要可保留的来源版本",
            ));
        }
        let view = crate::online::Snapshot::open(
            source,
            Some(&expected.catalog_revision),
            cancelled.clone(),
            None,
        )?;
        if view.version(source) != *expected {
            return Err(Error::new("SOURCE_CHANGED", "全湖排名的固定版本不一致"));
        }
        let total = view.count;
        drop(view);
        if source.kind == "danbooru"
            && let Some(mut population) = crate::online::ranking_source::Population::load(
                source,
                expected,
                self.runtime.query_memory(),
                parameters.minimum_stored_side.is_some(),
                &cancelled,
            )?
        {
            let fallback = population.project(source, expected, parameters, &cancelled, sink)?;
            drop(population);
            if !fallback.is_empty() {
                self.project_chunks(
                    source,
                    expected,
                    &[],
                    parameters,
                    cancelled.clone(),
                    &mut |append| {
                        for (ordinal, asset, basis) in &fallback {
                            append(*ordinal, asset, *basis)?;
                        }
                        Ok(())
                    },
                    sink,
                    4096,
                )?;
            }
            crate::canonical::Catalog::open_at(source, Some(&expected.catalog_revision))?
                .verify_unchanged(source)?;
            return Ok(total);
        }
        let mut count = 0u64;
        self.project(
            source,
            expected,
            &[],
            parameters,
            cancelled.clone(),
            &mut |append| {
                let mut after = String::new();
                loop {
                    studio_application::read_cancelled(&cancelled)?;
                    let view = crate::online::Snapshot::open_bulk(
                        source,
                        Some(&expected.catalog_revision),
                        cancelled.clone(),
                        Some(std::time::Instant::now() + std::time::Duration::from_secs(60)),
                    )?;
                    let keys = view.ranking_keys(&after)?;
                    drop(view);
                    if keys.is_empty() {
                        break;
                    }
                    for key in &keys {
                        append(count, key, 0)?;
                        count += 1;
                        if count > total {
                            return Err(Error::new("INPUT_CHANGED", "全湖排名成员超出发布数量"));
                        }
                    }
                    after = keys.last().expect("nonempty keys").clone();
                }
                Ok(())
            },
            sink,
        )?;
        if count != total {
            return Err(Error::new("INPUT_CHANGED", "全湖排名成员与发布数量不一致"));
        }
        Ok(count)
    }
    pub fn configured(directory: PathBuf, memory_bytes: u64) -> Self {
        Self {
            runtime: Runtime::default()
                .with_query_directory(directory)
                .with_query_memory(memory_bytes),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn project(
        &self,
        source: &Source,
        expected: &QuerySourceVersion,
        bases: &[RankingBasis],
        parameters: &RankingParameters,
        cancelled: Arc<AtomicBool>,
        produce: &mut studio_application::RankingMemberProducer<'_>,
        sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>,
    ) -> Result<()> {
        self.project_chunks(
            source, expected, bases, parameters, cancelled, produce, sink, 32768,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn project_chunks(
        &self,
        source: &Source,
        expected: &QuerySourceVersion,
        bases: &[RankingBasis],
        parameters: &RankingParameters,
        cancelled: Arc<AtomicBool>,
        produce: &mut studio_application::RankingMemberProducer<'_>,
        sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>,
        batch_size: usize,
    ) -> Result<()> {
        if !crate::online::available(source) {
            return self.project_batch(
                source, expected, bases, parameters, cancelled, produce, sink,
            );
        }
        // All representative-observation and duplicate-post operations partition
        // by ordinal. Complete ordinal groups can therefore be projected and
        // released independently instead of retaining an entire lake in DuckDB.
        let batch_size = batch_size.clamp(1, 32768);
        let memory = self.runtime.query_memory();
        let workers = (thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            / 2)
        .clamp(1, 8)
        .min((memory / (1 << 30)).max(1) as usize);
        // Reserve space for the SQLite reader, bounded channel pages and the
        // writer. Each worker has its own share of the one admitted budget.
        let worker_memory = memory
            .saturating_sub((workers as u64 + 1) * 128 * 1024 * 1024)
            .checked_div(workers as u64)
            .unwrap_or(memory)
            .max(32 << 20);
        thread::scope(|scope| {
            let mut pending: VecDeque<ProjectionBatch<'_>> = VecDeque::new();
            let outcome = (|| -> Result<()> {
                let mut schedule = |rows: Vec<(u64, String, u32)>| -> Result<()> {
                    let reader = Self {
                        runtime: self
                            .runtime
                            .for_deadline(self.runtime.deadline())
                            .with_query_memory(worker_memory),
                    };
                    let flag = cancelled.clone();
                    let (send, receive) = mpsc::sync_channel(1);
                    let worker = scope.spawn(move || {
                        if bases.is_empty()
                            && let Some(mut captured) = crate::online::ranking_simple::capture(
                                source,
                                expected,
                                parameters,
                                &rows,
                                flag.clone(),
                            )?
                        {
                            if !captured.fallback.is_empty() {
                                reader.project_batch(
                                    source,
                                    expected,
                                    bases,
                                    parameters,
                                    flag.clone(),
                                    &mut |append| {
                                        for (ordinal, asset, basis) in &captured.fallback {
                                            append(*ordinal, asset, *basis)?;
                                        }
                                        Ok(())
                                    },
                                    &mut |page| {
                                        captured.rows.extend_from_slice(page);
                                        Ok(())
                                    },
                                )?;
                            }
                            captured.rows.sort_unstable_by_key(|r| r.ordinal);
                            for page in captured.rows.chunks(512) {
                                studio_application::read_cancelled(&flag)?;
                                send.send(page.to_vec())
                                    .map_err(|_| Error::new("CANCELLED", "排名输入接收已停止"))?;
                            }
                            return Ok(());
                        }
                        reader.project_batch(
                            source,
                            expected,
                            bases,
                            parameters,
                            flag,
                            &mut |append| {
                                for (ordinal, asset, basis) in &rows {
                                    append(*ordinal, asset, *basis)?;
                                }
                                Ok(())
                            },
                            &mut |page| {
                                send.send(page.to_vec())
                                    .map_err(|_| Error::new("CANCELLED", "排名输入接收已停止"))
                            },
                        )
                    });
                    pending.push_back(ProjectionBatch {
                        rows: receive,
                        worker,
                    });
                    if pending.len() >= workers {
                        pending
                            .pop_front()
                            .expect("pending projection")
                            .drain(sink)?;
                    }
                    Ok(())
                };
                let mut batch: Vec<(u64, String, u32)> = Vec::with_capacity(batch_size);
                let mut last = None;
                produce(&mut |ordinal, asset, basis| {
                    studio_application::read_cancelled(&cancelled)?;
                    if last.is_some_and(|old| old > ordinal)
                        || batch
                            .last()
                            .is_some_and(|(old, key, _)| *old == ordinal && key != asset)
                    {
                        return Err(Error::invalid(
                            "排名成员须按序号递增，同一序号必须指向同一图片",
                        ));
                    }
                    if batch.len() >= batch_size && last.is_some_and(|old| old != ordinal) {
                        schedule(std::mem::replace(
                            &mut batch,
                            Vec::with_capacity(batch_size),
                        ))?;
                    }
                    if batch.len() >= batch_size + 65536 {
                        return Err(Error::new(
                            "READ_BUDGET_EXCEEDED",
                            "单张图片的排名依据数量超过预算",
                        ));
                    }
                    batch.push((ordinal, asset.to_owned(), basis));
                    last = Some(ordinal);
                    Ok(())
                })?;
                if !batch.is_empty() || last.is_none() {
                    schedule(batch)?;
                }
                while let Some(batch) = pending.pop_front() {
                    batch.drain(sink)?;
                }
                Ok(())
            })();
            if outcome.is_err() {
                cancelled.store(true, Ordering::Release);
            }
            // Drop receivers before joining: writers waiting on backpressure
            // must be able to stop when cancellation or a sink failure occurs.
            drop(pending);
            outcome
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn project_batch(
        &self,
        source: &Source,
        expected: &QuerySourceVersion,
        bases: &[RankingBasis],
        parameters: &RankingParameters,
        cancelled: Arc<AtomicBool>,
        produce: &mut studio_application::RankingMemberProducer<'_>,
        sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>,
    ) -> Result<()> {
        let started = std::time::Instant::now();
        if source.kind != "danbooru" {
            return Err(Error::new(
                "RANKING_SOURCE_UNSUPPORTED",
                "元数据排名当前需要 Danbooru 来源",
            ));
        }
        let dimensions = parameters.minimum_stored_side.is_some();
        let include_tags = parameters.v2.is_some();
        let ratings = parameters
            .ratings
            .iter()
            .map(|r| quote(r))
            .collect::<Vec<_>>()
            .join(",");
        let catalog = Catalog::open_at(source, Some(&expected.catalog_revision))?;
        if expected.source_id != source.id || expected.catalog_revision != catalog.revision {
            return Err(Error::new("SOURCE_CHANGED", "排名输入的来源版本已变化"));
        }
        let db = if catalog.online {
            self.runtime.open_transient_population(cancelled.clone())?
        } else {
            self.runtime
                .open_population(&catalog.analysis_path()?, cancelled.clone())?
        };
        db.query("BEGIN TRANSACTION")?;
        if catalog.online {
            crate::online::ranking::prepare(
                &db, source, expected, bases, parameters, cancelled, produce,
            )?;
        } else {
            let sequence = db.query("SELECT CAST(max(seq) AS VARCHAR) FROM applied")?;
            let sequence = sequence.first().and_then(|row| row[0].as_deref());
            if sequence != expected.analysis_sequence.as_deref()
                || sequence != Some(catalog.sequence.to_string().as_str())
            {
                return Err(Error::new(
                    "SOURCE_CHANGED",
                    "排名所需的元数据与目录水位不一致",
                ));
            }
            db.import_ranking_members(produce)?;
            db.query("CREATE TEMP TABLE ranking_scope AS SELECT DISTINCT ordinal,lower(hex(sha256)) AS sha256,basis FROM studio_ranking_scope")?;
            let default = QuerySpec {
                version: 3,
                source_ids: vec![source.id.clone()],
                conditions: vec![],
                observation_rule: ObservationRule::CurrentPost,
                order: QueryOrder::AssetKeyAsc,
                input_scope: None,
            };
            let mut definitions = vec![(0, &default)];
            definitions.extend(
                bases
                    .iter()
                    .filter(|b| b.spec.source_ids.contains(&source.id))
                    .map(|b| (b.index, &b.spec)),
            );
            let columns = "s.ordinal,s.basis,a.asset_id AS record_id,a.observation_id AS origin_observation_id,o.observation_id,o.post_id,o.rating,epoch_us(o.created_at) AS created_at_us,epoch_us(o.observed_at) AS observed_at_us,epoch_us(o.updated_at) AS updated_at_us,o.time_quality,o.source_priority,o.fav_count,o.up_score,o.down_score,o.score,o.tag_string_artist,o.tag_string,o.parent_id,o.is_banned,o.is_deleted,o.is_pending,o.is_flagged,CASE WHEN list_contains(string_split(o.tag_string,' '),'jpeg_artifacts') THEN 1 ELSE 0 END+CASE WHEN list_contains(string_split(o.tag_string,' '),'scan_artifacts') THEN 2 ELSE 0 END AS damage_classes,o.tag_string IS NOT NULL AS tags_known,CASE WHEN length(o.issues_json)>4096 THEN '[\"source_issues_truncated\"]' ELSE o.issues_json END AS source_issues";
            let mut branches = Vec::new();
            for (index, spec) in definitions {
                // Membership is already frozen. The new image policy considers all
                // current posts describing that exact image, even when another post
                // originally matched the query. Explicit historical scopes retain
                // their original observation predicates.
                let predicate = if parameters.duplicate_heat.is_some()
                    && spec.observation_rule == ObservationRule::CurrentPost
                {
                    "1=1".into()
                } else {
                    ranking_predicate(spec)?
                };
                match spec.observation_rule {
                ObservationRule::CurrentPost=>branches.push(format!("SELECT {columns} FROM ranking_scope s JOIN assets a ON a.sha256=s.sha256 JOIN current_posts cp ON cp.asset_id=a.asset_id JOIN observations o ON o.row_id=cp.row_id WHERE s.basis={index} AND ({predicate})")),
                ObservationRule::AnyObservation=>{
                    // A later post observation is usable only when its content hash still matches this asset.
                    branches.push(format!("SELECT {columns} FROM ranking_scope s JOIN assets a ON a.sha256=s.sha256 JOIN observations o ON o.post_id=a.post_id WHERE s.basis={index} AND (o.observation_id=a.observation_id OR (a.source_md5 IS NOT NULL AND o.md5=a.source_md5)) AND ({predicate})"));
                    branches.push(format!("SELECT {columns} FROM ranking_scope s JOIN assets a ON a.sha256=s.sha256 JOIN observations o ON o.observation_id=a.observation_id WHERE s.basis={index} AND (a.post_id IS NULL OR o.post_id IS DISTINCT FROM a.post_id) AND ({predicate})"));
                }
            }
            }
            db.query(&format!(
                "CREATE TEMP TABLE ranking_observations AS {}",
                branches.join(" UNION ALL ")
            ))?;
        }
        let captured = started.elapsed();
        // Day-level snapshots overlap all exact observations within that UTC day.
        // Such a day uses updated_at as the next comparison key for every peer.
        db.query("CREATE TEMP TABLE ranking_observation_order AS SELECT *,CASE WHEN time_quality IN ('exact','date_only') THEN observed_at_us//86400000000 ELSE NULL END AS observed_day,count(DISTINCT record_id) OVER (PARTITION BY ordinal) AS record_count,count(DISTINCT rating) OVER (PARTITION BY ordinal)>1 AS rating_conflict FROM ranking_observations")?;
        db.query("CREATE TEMP TABLE ranking_observation_precision AS SELECT *,max(CASE WHEN time_quality='date_only' THEN 1 ELSE 0 END) OVER (PARTITION BY ordinal,observed_day) AS coarse_day FROM ranking_observation_order")?;
        db.query("DROP TABLE ranking_observations; DROP TABLE ranking_observation_order")?;
        if let Some(policy) = parameters.duplicate_heat {
            crate::ranking_duplicates::prepare(&db, policy)?;
        } else {
            db.query(&format!("CREATE TEMP TABLE ranking_chosen AS SELECT * EXCLUDE(position) FROM (SELECT *,row_number() OVER (PARTITION BY ordinal ORDER BY CASE WHEN rating IN ({ratings}) THEN 0 ELSE 1 END,observed_day DESC NULLS LAST,CASE WHEN coarse_day=0 THEN observed_at_us ELSE NULL END DESC NULLS LAST,updated_at_us DESC NULLS LAST,source_priority DESC NULLS LAST,observation_id,record_id,basis) AS position FROM ranking_observation_precision) WHERE position=1"))?;
        }
        db.query("DROP TABLE ranking_observation_precision")?;
        if dimensions {
            db.query("CREATE TEMP TABLE ranking_direct_dimensions AS SELECT c.ordinal,c.origin_observation_id,TRY_CAST(try(json_extract_string(a.details_json,'$.stored_width')) AS BIGINT) AS w,TRY_CAST(try(json_extract_string(a.details_json,'$.stored_height')) AS BIGINT) AS h FROM ranking_chosen c JOIN assets a ON a.asset_id=c.record_id")?;
            db.query("CREATE TEMP TABLE ranking_raw_dimensions AS SELECT d.ordinal,TRY_CAST(try(json_extract_string(r.source_metadata_json,'$.raw_stored_width')) AS BIGINT) AS w,TRY_CAST(try(json_extract_string(r.source_metadata_json,'$.raw_stored_height')) AS BIGINT) AS h FROM ranking_direct_dimensions d LEFT JOIN raw_metadata r ON r.observation_id=d.origin_observation_id WHERE NOT coalesce(d.w BETWEEN 1 AND 4294967295 AND d.h BETWEEN 1 AND 4294967295,false)")?;
            db.query("CREATE TEMP TABLE ranking_dimensions AS SELECT d.ordinal,CASE WHEN d.w BETWEEN 1 AND 4294967295 AND d.h BETWEEN 1 AND 4294967295 THEN d.w WHEN r.w BETWEEN 1 AND 4294967295 AND r.h BETWEEN 1 AND 4294967295 THEN r.w ELSE NULL END AS stored_width,CASE WHEN d.w BETWEEN 1 AND 4294967295 AND d.h BETWEEN 1 AND 4294967295 THEN d.h WHEN r.w BETWEEN 1 AND 4294967295 AND r.h BETWEEN 1 AND 4294967295 THEN r.h ELSE NULL END AS stored_height,CASE WHEN d.w BETWEEN 1 AND 4294967295 AND d.h BETWEEN 1 AND 4294967295 THEN 'asset_storage_details' WHEN r.w BETWEEN 1 AND 4294967295 AND r.h BETWEEN 1 AND 4294967295 THEN 'asset_origin_raw_metadata' ELSE 'not_recorded' END AS dimension_basis FROM ranking_direct_dimensions d LEFT JOIN ranking_raw_dimensions r USING(ordinal)")?;
        } else {
            db.query("CREATE TEMP TABLE ranking_dimensions AS SELECT ordinal,NULL::BIGINT AS stored_width,NULL::BIGINT AS stored_height,'not_requested' AS dimension_basis FROM ranking_chosen")?;
        }
        let source_id = quote(&source.id);
        let evidence = if parameters.duplicate_heat.is_some() {
            "CAST(c.evidence_json AS JSON)"
        } else {
            "NULL"
        };
        let sql = format!(
            r#"SELECT to_json(struct_pack(
            ordinal:=m.ordinal,source_id:={source_id},asset_id:=m.sha256,record_id:=c.record_id,
            evidence:={evidence},
            observation_id:=c.observation_id,post_id:=c.post_id,rating:=c.rating,
            tags:=CASE WHEN {include_tags} THEN c.tag_string ELSE NULL END,
            created_at_us:=c.created_at_us,observed_at_us:=c.observed_at_us,updated_at_us:=c.updated_at_us,
            time_quality:=coalesce(c.time_quality,'unknown'),source_priority:=c.source_priority,
            fav_count:=c.fav_count,up_score:=c.up_score,down_score:=c.down_score,score:=c.score,
            artists:=list_sort(list_distinct(list_filter(string_split(coalesce(c.tag_string_artist,''),' '),x->x!='' AND x NOT IN ('artist_request','unknown_artist','anonymous_artist','banned_artist')))),
            parent_id:=c.parent_id,stored_width:=d.stored_width,stored_height:=d.stored_height,
            dimension_basis:=coalesce(d.dimension_basis,'not_recorded'),stored_extension:=coalesce(obj.stored_ext,''),stored_bytes:=coalesce(obj.length,0),
            is_banned:=c.is_banned,is_deleted:=c.is_deleted,is_pending:=c.is_pending,is_flagged:=c.is_flagged,
            damage_classes:=coalesce(c.damage_classes,0),tags_known:=coalesce(c.tags_known,false),
            record_count:=coalesce(c.record_count,0),rating_conflict:=coalesce(c.rating_conflict,false),
            basis_ids:=CASE WHEN c.basis IS NULL THEN []::BIGINT[] ELSE [c.basis] END,source_issues:=c.source_issues
        ))::VARCHAR FROM (SELECT DISTINCT ordinal,sha256 FROM ranking_scope) m
        LEFT JOIN ranking_chosen c USING(ordinal) LEFT JOIN ranking_dimensions d USING(ordinal)
        LEFT JOIN objects obj ON obj.sha256=m.sha256 ORDER BY m.ordinal"#
        );
        let prepared = started.elapsed();
        let mut projected_count = 0usize;
        let mut sink_time = std::time::Duration::ZERO;
        db.stream_strings(&sql, 128 * 1024, &mut |rows| {
            let projected = rows
                .iter()
                .map(|row| {
                    serde_json::from_str::<RankingInput>(row).map_err(|e| {
                        Error::new("SOURCE_FORMAT_ERROR", format!("排名字段投影无效：{e}"))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            projected_count += projected.len();
            let sending = std::time::Instant::now();
            let outcome = sink(&projected);
            sink_time += sending.elapsed();
            outcome
        })?;
        tracing::debug!(
            rows = projected_count,
            capture_ms = captured.as_millis(),
            prepare_ms = (prepared - captured).as_millis(),
            stream_ms = (started.elapsed() - prepared).as_millis(),
            sink_ms = sink_time.as_millis(),
            "ranking input projection batch"
        );
        catalog.verify_unchanged(source)?;
        db.query("ROLLBACK")?;
        Ok(())
    }
}
