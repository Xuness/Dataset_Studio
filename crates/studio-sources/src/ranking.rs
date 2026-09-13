//! Bulk projection of complete representative observations from a read-only lake.
use crate::{danbooru::Catalog, duckdb::Runtime, query::compiler::ranking_predicate};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use studio_domain::*;

#[derive(Default)]
pub struct RankingReader {
    runtime: Runtime,
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
impl RankingReader {
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
        let catalog = Catalog::open(source)?;
        if expected.source_id != source.id || expected.catalog_revision != catalog.revision {
            return Err(Error::new("SOURCE_CHANGED", "排名输入的来源版本已变化"));
        }
        let db = self
            .runtime
            .open_population(&catalog.analysis_path()?, cancelled)?;
        db.query("BEGIN TRANSACTION")?;
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
            let predicate = ranking_predicate(spec)?;
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
        // Day-level snapshots overlap all exact observations within that UTC day.
        // Such a day uses updated_at as the next comparison key for every peer.
        db.query("CREATE TEMP TABLE ranking_observation_order AS SELECT *,CASE WHEN time_quality IN ('exact','date_only') THEN observed_at_us//86400000000 ELSE NULL END AS observed_day,count(DISTINCT record_id) OVER (PARTITION BY ordinal) AS record_count,count(DISTINCT rating) OVER (PARTITION BY ordinal)>1 AS rating_conflict FROM ranking_observations")?;
        db.query("CREATE TEMP TABLE ranking_observation_precision AS SELECT *,max(CASE WHEN time_quality='date_only' THEN 1 ELSE 0 END) OVER (PARTITION BY ordinal,observed_day) AS coarse_day FROM ranking_observation_order")?;
        db.query(&format!("CREATE TEMP TABLE ranking_chosen AS SELECT * EXCLUDE(position) FROM (SELECT *,row_number() OVER (PARTITION BY ordinal ORDER BY CASE WHEN rating IN ({ratings}) THEN 0 ELSE 1 END,observed_day DESC NULLS LAST,CASE WHEN coarse_day=0 THEN observed_at_us ELSE NULL END DESC NULLS LAST,updated_at_us DESC NULLS LAST,source_priority DESC NULLS LAST,observation_id,record_id,basis) AS position FROM ranking_observation_precision) WHERE position=1"))?;
        if dimensions {
            db.query("CREATE TEMP TABLE ranking_direct_dimensions AS SELECT c.ordinal,c.origin_observation_id,TRY_CAST(try(json_extract_string(a.details_json,'$.stored_width')) AS BIGINT) AS w,TRY_CAST(try(json_extract_string(a.details_json,'$.stored_height')) AS BIGINT) AS h FROM ranking_chosen c JOIN assets a ON a.asset_id=c.record_id")?;
            db.query("CREATE TEMP TABLE ranking_raw_dimensions AS SELECT d.ordinal,TRY_CAST(try(json_extract_string(r.source_metadata_json,'$.raw_stored_width')) AS BIGINT) AS w,TRY_CAST(try(json_extract_string(r.source_metadata_json,'$.raw_stored_height')) AS BIGINT) AS h FROM ranking_direct_dimensions d LEFT JOIN raw_metadata r ON r.observation_id=d.origin_observation_id WHERE NOT coalesce(d.w BETWEEN 1 AND 4294967295 AND d.h BETWEEN 1 AND 4294967295,false)")?;
            db.query("CREATE TEMP TABLE ranking_dimensions AS SELECT d.ordinal,CASE WHEN d.w BETWEEN 1 AND 4294967295 AND d.h BETWEEN 1 AND 4294967295 THEN d.w WHEN r.w BETWEEN 1 AND 4294967295 AND r.h BETWEEN 1 AND 4294967295 THEN r.w ELSE NULL END AS stored_width,CASE WHEN d.w BETWEEN 1 AND 4294967295 AND d.h BETWEEN 1 AND 4294967295 THEN d.h WHEN r.w BETWEEN 1 AND 4294967295 AND r.h BETWEEN 1 AND 4294967295 THEN r.h ELSE NULL END AS stored_height,CASE WHEN d.w BETWEEN 1 AND 4294967295 AND d.h BETWEEN 1 AND 4294967295 THEN 'asset_storage_details' WHEN r.w BETWEEN 1 AND 4294967295 AND r.h BETWEEN 1 AND 4294967295 THEN 'asset_origin_raw_metadata' ELSE 'not_recorded' END AS dimension_basis FROM ranking_direct_dimensions d LEFT JOIN ranking_raw_dimensions r USING(ordinal)")?;
        } else {
            db.query("CREATE TEMP TABLE ranking_dimensions AS SELECT ordinal,NULL::BIGINT AS stored_width,NULL::BIGINT AS stored_height,'not_requested' AS dimension_basis FROM ranking_chosen")?;
        }
        let source_id = quote(&source.id);
        let sql = format!(
            r#"SELECT to_json(struct_pack(
            ordinal:=m.ordinal,source_id:={source_id},asset_id:=m.sha256,record_id:=c.record_id,
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
        db.stream_strings(&sql, 128 * 1024, &mut |rows| {
            let projected = rows
                .iter()
                .map(|row| {
                    serde_json::from_str::<RankingInput>(row).map_err(|e| {
                        Error::new("SOURCE_FORMAT_ERROR", format!("排名字段投影无效：{e}"))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            sink(&projected)
        })?;
        catalog.verify_unchanged(source)?;
        db.query("ROLLBACK")?;
        Ok(())
    }
}
