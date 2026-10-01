//! Bounded SQLite captures feed a private DuckDB analytical workspace. No native
//! lake file is opened, and no SQLite read transaction spans the whole population.
use super::*;
use crate::duckdb::Session;
use rusqlite::types::Value;
use studio_application::{RankingMemberProducer, ReadCancellation};

const FACTS: &str = "s.ordinal,s.basis,a.asset_id,a.observation_id,p.observation_id,p.post_id,p.rating,p.created_at,p.observed_at,p.updated_at,p.time_quality,p.source_priority,p.fav_count,p.up_score,p.down_score,p.score,p.tag_string_artist,p.tag_string,p.parent_id,p.is_banned,p.is_deleted,p.is_pending,p.is_flagged,p.issues_json";

fn transfer(
    view: &Snapshot,
    db: &Session,
    table: &str,
    sql: &str,
    values: Vec<Value>,
) -> Result<()> {
    let mut statement = view.db.prepare(sql).map_err(sql_error)?;
    let width = statement.column_count();
    let mut rows = statement
        .query(rusqlite::params_from_iter(values.iter()))
        .map_err(sql_error)?;
    let mut batch = Vec::new();
    let mut bytes = 0usize;
    while let Some(row) = rows.next().map_err(sql_error)? {
        let row = (0..width)
            .map(|i| row.get::<_, Value>(i).map_err(sql_error))
            .collect::<Result<Vec<_>>>()?;
        bytes += row
            .iter()
            .map(|v| match v {
                Value::Text(s) => s.len(),
                Value::Blob(v) => v.len(),
                _ => 8,
            })
            .sum::<usize>();
        batch.push(row);
        if batch.len() >= 512 || bytes >= 1 << 20 {
            db.append_values(table, &batch)?;
            batch.clear();
            bytes = 0;
        }
    }
    if !batch.is_empty() {
        db.append_values(table, &batch)?;
    }
    Ok(())
}

fn capture(
    db: &Session,
    source: &Source,
    expected: &QuerySourceVersion,
    bases: &[RankingBasis],
    parameters: &RankingParameters,
    rows: &[(u64, String, u32)],
    cancelled: ReadCancellation,
) -> Result<()> {
    let view = Snapshot::open(
        source,
        Some(&expected.catalog_revision),
        cancelled,
        Some(Instant::now() + Duration::from_secs(60)),
    )?;
    let input = serde_json::to_string(rows).map_err(Error::io)?;
    let scope = rows
        .iter()
        .map(|(ordinal, sha, basis)| {
            Ok(vec![
                Value::Integer(i64::try_from(*ordinal).map_err(Error::io)?),
                Value::Text(sha.clone()),
                Value::Integer(i64::from(*basis)),
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    db.append_values("online_scope", &scope)?;
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
    for (index, spec) in definitions {
        if !rows.iter().any(|r| r.2 == index) {
            continue;
        }
        let (predicate, mut values) = if parameters.duplicate_heat.is_some()
            && spec.observation_rule == ObservationRule::CurrentPost
        {
            (String::from("1"), Vec::new())
        } else {
            view.ranking_predicate(spec)?
        };
        values.push(Value::Text(input.clone()));
        let input_index = values.len();
        let scoped = format!(
            "WITH scoped AS(SELECT json_extract(value,'$[0]') ordinal,json_extract(value,'$[1]') sha256,json_extract(value,'$[2]') basis FROM json_each(?{input_index}))"
        );
        let sql = match spec.observation_rule {
            ObservationRule::CurrentPost => format!(
                "{scoped} SELECT {FACTS} FROM scoped s JOIN visible_assets a ON a.sha256=s.sha256 JOIN current_posts cp ON cp.asset_id=a.asset_id JOIN visible_observations p ON p.row_id=cp.row_id WHERE s.basis={index} AND ({predicate})"
            ),
            ObservationRule::AnyObservation => format!(
                "{scoped} SELECT {FACTS} FROM scoped s JOIN visible_assets a ON a.sha256=s.sha256 JOIN visible_observations p ON p.post_id=a.post_id WHERE s.basis={index} AND (p.observation_id=a.observation_id OR (a.source_md5 IS NOT NULL AND p.md5=a.source_md5)) AND ({predicate}) UNION ALL SELECT {FACTS} FROM scoped s JOIN visible_assets a ON a.sha256=s.sha256 JOIN visible_observations p ON p.observation_id=a.observation_id WHERE s.basis={index} AND (a.post_id IS NULL OR p.post_id IS DISTINCT FROM a.post_id) AND ({predicate})"
            ),
        };
        transfer(&view, db, "online_facts", &sql, values)?;
    }
    let ids = rows
        .iter()
        .map(|r| &r.1)
        .collect::<std::collections::BTreeSet<_>>();
    let ids = serde_json::to_string(&ids).map_err(Error::io)?;
    transfer(
        &view,
        db,
        "online_objects",
        "SELECT sha256,stored_ext,length FROM visible_objects WHERE sha256 IN (SELECT value FROM json_each(?1))",
        vec![Value::Text(ids.clone())],
    )?;
    if parameters.minimum_stored_side.is_some() {
        transfer(
            &view,
            db,
            "online_assets",
            "SELECT asset_id,observation_id,details_json FROM visible_assets WHERE sha256 IN (SELECT value FROM json_each(?1))",
            vec![Value::Text(ids.clone())],
        )?;
        // Valid stored dimensions already satisfy the downstream projection. Only
        // missing/ambiguous dimensions need the complete compressed source record.
        let mut statement=view.db.prepare("SELECT DISTINCT r.observation_id,r.raw_bytes,r.raw_zlib,r.raw_sha256 FROM visible_assets a JOIN raw_metadata r ON r.observation_id=a.observation_id WHERE a.sha256 IN (SELECT value FROM json_each(?1)) AND NOT coalesce(CASE WHEN json_valid(a.details_json) THEN json_type(a.details_json,'$.stored_width')='integer' AND json_type(a.details_json,'$.stored_height')='integer' AND json_extract(a.details_json,'$.stored_width') BETWEEN 1 AND 4294967295 AND json_extract(a.details_json,'$.stored_height') BETWEEN 1 AND 4294967295 ELSE 0 END,0)").map_err(sql_error)?;
        let mut rows = statement.query([ids]).map_err(sql_error)?;
        let mut raw = Vec::new();
        while let Some(row) = rows.next().map_err(sql_error)? {
            let size: i64 = row.get(1).map_err(sql_error)?;
            if size < 0 {
                continue;
            }
            if size > 16 << 20 {
                return Err(Error::new(
                    "READ_BUDGET_EXCEEDED",
                    "排名原始尺寸记录超过 16 MiB 预算",
                ));
            }
            let compressed: Vec<u8> = row.get(2).map_err(sql_error)?;
            let hash: String = row.get(3).map_err(sql_error)?;
            let body = super::raw::decode(&compressed, size as u64, &hash, 16 << 20)?;
            let json: serde_json::Value = serde_json::from_str(&body).map_err(error)?;
            let projected = serde_json::json!({"raw_stored_width":json.get("raw_stored_width"),"raw_stored_height":json.get("raw_stored_height")});
            raw.push(vec![
                Value::Text(row.get(0).map_err(sql_error)?),
                Value::Text(projected.to_string()),
            ]);
            if raw.len() == 512 {
                db.append_values("online_raw", &raw)?;
                raw.clear();
            }
        }
        if !raw.is_empty() {
            db.append_values("online_raw", &raw)?;
        }
    }
    Ok(())
}

pub(crate) fn prepare(
    db: &Session,
    source: &Source,
    expected: &QuerySourceVersion,
    bases: &[RankingBasis],
    parameters: &RankingParameters,
    cancelled: ReadCancellation,
    produce: &mut RankingMemberProducer<'_>,
) -> Result<()> {
    db.query("CREATE TEMP TABLE online_scope(ordinal BIGINT,sha256 VARCHAR,basis BIGINT);
      CREATE TEMP TABLE online_facts(ordinal BIGINT,basis BIGINT,record_id VARCHAR,origin_observation_id VARCHAR,observation_id VARCHAR,post_id BIGINT,rating VARCHAR,created_at VARCHAR,observed_at VARCHAR,updated_at VARCHAR,time_quality VARCHAR,source_priority BIGINT,fav_count BIGINT,up_score BIGINT,down_score BIGINT,score BIGINT,tag_string_artist VARCHAR,tag_string VARCHAR,parent_id BIGINT,is_banned BOOLEAN,is_deleted BOOLEAN,is_pending BOOLEAN,is_flagged BOOLEAN,issues_json VARCHAR);
      CREATE TEMP TABLE online_objects(sha256 VARCHAR,stored_ext VARCHAR,length BIGINT);
      CREATE TEMP TABLE online_assets(asset_id VARCHAR,observation_id VARCHAR,details_json VARCHAR);
      CREATE TEMP TABLE online_raw(observation_id VARCHAR,source_metadata_json VARCHAR)")?;
    let mut batch = Vec::new();
    produce(&mut |ordinal, sha, basis| {
        studio_application::read_cancelled(&cancelled)?;
        if sha.len() != 64 || hex::decode(sha).is_err() {
            return Err(Error::invalid("排名图片身份无效"));
        }
        batch.push((ordinal, sha.to_owned(), basis));
        if batch.len() == 512 {
            capture(
                db,
                source,
                expected,
                bases,
                parameters,
                &batch,
                cancelled.clone(),
            )?;
            batch.clear();
        }
        Ok(())
    })?;
    if !batch.is_empty() {
        capture(db, source, expected, bases, parameters, &batch, cancelled)?;
    }
    db.query("CREATE TEMP TABLE ranking_scope AS SELECT DISTINCT * FROM online_scope;
      CREATE TEMP TABLE objects AS SELECT DISTINCT * FROM online_objects;
      CREATE TEMP TABLE assets AS SELECT DISTINCT * FROM online_assets;
      CREATE TEMP TABLE raw_metadata AS SELECT DISTINCT * FROM online_raw;
      CREATE TEMP TABLE ranking_observations AS SELECT * EXCLUDE(created_at,observed_at,updated_at,issues_json),epoch_us(try_cast(created_at AS TIMESTAMPTZ)) AS created_at_us,epoch_us(try_cast(observed_at AS TIMESTAMPTZ)) AS observed_at_us,epoch_us(try_cast(updated_at AS TIMESTAMPTZ)) AS updated_at_us,
      CASE WHEN list_contains(string_split(tag_string,' '),'jpeg_artifacts') THEN 1 ELSE 0 END+CASE WHEN list_contains(string_split(tag_string,' '),'scan_artifacts') THEN 2 ELSE 0 END AS damage_classes,tag_string IS NOT NULL AS tags_known,CASE WHEN length(issues_json)>4096 THEN '[\"source_issues_truncated\"]' ELSE issues_json END AS source_issues FROM (SELECT DISTINCT * FROM online_facts);
      DROP TABLE online_scope; DROP TABLE online_objects; DROP TABLE online_assets; DROP TABLE online_raw; DROP TABLE online_facts")?;
    Ok(())
}
