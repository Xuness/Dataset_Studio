//! Standalone typed ranking materials. Project services supply controlled paths.
use crate::{db_error, unsigned};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, types::Value as SqlValue};
use serde::{Serialize, de::DeserializeOwned};
use std::{cell::Cell, fs, path::Path};
use studio_domain::*;
mod browse;
pub use browse::{PostIdScan, RankingScan};

const INPUT_ID: i64 = 0x4d524931;
const SCORE_ID: i64 = 0x4d525331;
const COMMIT_ROWS: usize = 8192;

#[derive(Default)]
struct WriteBatch {
    rows: Cell<usize>,
}
impl WriteBatch {
    fn begin(&self, db: &Connection) -> Result<()> {
        if db.is_autocommit() {
            db.execute_batch("BEGIN IMMEDIATE;").map_err(db_error)?;
        }
        Ok(())
    }
    fn finish(&self, db: &Connection, count: usize, result: Result<()>) -> Result<()> {
        if result.is_err() {
            let _ = db.execute_batch("ROLLBACK;");
            self.rows.set(0);
            return result;
        }
        self.rows.set(self.rows.get() + count);
        if self.rows.get() >= COMMIT_ROWS {
            self.flush(db)?;
        }
        Ok(())
    }
    fn flush(&self, db: &Connection) -> Result<()> {
        if !db.is_autocommit() {
            db.execute_batch("COMMIT;").map_err(db_error)?;
        }
        self.rows.set(0);
        Ok(())
    }
}
const INPUT_COLUMNS: &str = "ordinal,source_id,asset_id,record_id,observation_id,post_id,rating,created_at_us,observed_at_us,updated_at_us,time_quality,source_priority,fav_count,up_score,down_score,score,artists,parent_id,stored_width,stored_height,dimension_basis,stored_extension,stored_bytes,is_banned,is_deleted,is_pending,is_flagged,damage_classes,tags_known,record_count,rating_conflict,basis_ids,source_issues";
const SCORE_COLUMNS: &str = "ordinal,rating,eligibility,missing_flags,g,c,a,v,t,local_percentile,local_count,support_k,cohort_level,time_reason,artist_support,main_score,rescue_score,main_rank,rescue_rank,selected_route,duplicate_of";
fn score_index(filter: &RankingFilter, order: RankingOrder) -> &'static str {
    match order {
        RankingOrder::Input => "scores_rating",
        RankingOrder::Rescue => "scores_rescue",
        RankingOrder::Direct => "scores_direct",
        RankingOrder::Fused => "scores_fused",
        RankingOrder::Main if filter.route.is_some() => "scores_route_main",
        RankingOrder::Main if filter.eligibility.is_some() => "scores_eligibility_main",
        RankingOrder::Main => "scores_main",
    }
}
fn input_columns(v2: bool) -> String {
    INPUT_COLUMNS
        .split(',')
        .map(|s| format!("i.{s}"))
        .chain(std::iter::once("d.duplicate_of".into()))
        .chain(std::iter::once(if v2 { "i.tags" } else { "NULL" }.into()))
        .collect::<Vec<_>>()
        .join(",")
}
fn decode<T: DeserializeOwned>(r: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<T> {
    let value: String = r.get(index)?;
    serde_json::from_str(&value).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn decode_enum<T: DeserializeOwned>(r: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<T> {
    let value: String = r.get(index)?;
    serde_json::from_value(serde_json::Value::String(value)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn optional_unsigned(r: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    r.get::<_, Option<i64>>(index)?
        .map(|v| u64::try_from(v).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, v)))
        .transpose()
}
#[cfg(test)]
#[path = "ranking_v2_tests.rs"]
mod v2_material_tests;

fn enum_text(v: &impl Serialize) -> Result<String> {
    serde_json::to_value(v)
        .map_err(Error::io)?
        .as_str()
        .map(String::from)
        .ok_or_else(|| Error::invalid("无效排名状态"))
}
fn bytes(v: &str) -> Result<Vec<u8>> {
    if v.len() != 64 {
        return Err(Error::invalid("排名材料身份必须是 SHA-256"));
    }
    let bytes = hex::decode(v).map_err(Error::io)?;
    if bytes.len() != 32 {
        return Err(Error::invalid("排名材料身份无效"));
    }
    Ok(bytes)
}
fn connect(path: &Path, id: i64, create: bool, writable: bool) -> Result<Connection> {
    if create {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(Error::io)?;
    }
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = Connection::open_with_flags(
        path,
        flags | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_error)?;
    db.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(db_error)?;
    db.execute_batch("PRAGMA cache_size=-32768; PRAGMA temp_store=FILE;")
        .map_err(db_error)?;
    if create {
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE material_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;").map_err(db_error)?;
        db.pragma_update(None, "application_id", id)
            .map_err(db_error)?;
        db.pragma_update(None, "user_version", 1)
            .map_err(db_error)?;
    } else {
        let actual: i64 = db
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(db_error)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        if actual != id || !matches!(version, 1 | 2) {
            return Err(Error::new(
                "RANKING_FORMAT_UNSUPPORTED",
                "排名材料格式不兼容",
            ));
        }
    }
    if !writable {
        db.execute_batch("PRAGMA query_only=ON;")
            .map_err(db_error)?;
    }
    Ok(db)
}
fn put_meta(db: &Connection, key: &str, value: &impl Serialize) -> Result<()> {
    db.execute("INSERT INTO material_meta VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,serde_json::to_string(value).map_err(Error::io)?]).map_err(db_error)?;
    Ok(())
}
fn get_meta<T: DeserializeOwned>(db: &Connection, key: &str) -> Result<T> {
    let value: String = db
        .query_row("SELECT value FROM material_meta WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .map_err(db_error)?;
    serde_json::from_str(&value).map_err(Error::io)
}

fn read_input(r: &rusqlite::Row<'_>, o: usize) -> rusqlite::Result<RankingInput> {
    Ok(RankingInput {
        tags: r.get(o + 34)?,
        ordinal: unsigned(r, o)?,
        source_id: r.get(o + 1)?,
        asset_id: hex::encode(r.get::<_, Vec<u8>>(o + 2)?),
        record_id: r.get::<_, Option<Vec<u8>>>(o + 3)?.map(hex::encode),
        observation_id: r.get::<_, Option<Vec<u8>>>(o + 4)?.map(hex::encode),
        post_id: r.get(o + 5)?,
        rating: r.get(o + 6)?,
        created_at_us: r.get(o + 7)?,
        observed_at_us: r.get(o + 8)?,
        updated_at_us: r.get(o + 9)?,
        time_quality: r.get(o + 10)?,
        source_priority: r.get(o + 11)?,
        fav_count: r.get(o + 12)?,
        up_score: r.get(o + 13)?,
        down_score: r.get(o + 14)?,
        score: r.get(o + 15)?,
        artists: decode(r, o + 16)?,
        parent_id: r.get(o + 17)?,
        stored_width: r.get(o + 18)?,
        stored_height: r.get(o + 19)?,
        dimension_basis: r.get(o + 20)?,
        stored_extension: r.get(o + 21)?,
        stored_bytes: unsigned(r, o + 22)?,
        is_banned: r.get(o + 23)?,
        is_deleted: r.get(o + 24)?,
        is_pending: r.get(o + 25)?,
        is_flagged: r.get(o + 26)?,
        damage_classes: r.get(o + 27)?,
        tags_known: r.get(o + 28)?,
        record_count: r.get(o + 29)?,
        rating_conflict: r.get(o + 30)?,
        basis_ids: decode(r, o + 31)?,
        source_issues: r.get(o + 32)?,
        duplicate_of: optional_unsigned(r, o + 33)?,
    })
}
fn read_scores(r: &rusqlite::Row<'_>, o: usize) -> rusqlite::Result<RankingScores> {
    Ok(RankingScores {
        v2: r
            .get::<_, Option<String>>(o + 21)?
            .map(|value| {
                serde_json::from_str(&value).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        o + 21,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })
            })
            .transpose()?,
        ordinal: unsigned(r, o)?,
        rating: r.get(o + 1)?,
        eligibility: decode_enum(r, o + 2)?,
        missing_flags: decode(r, o + 3)?,
        g: r.get(o + 4)?,
        c: r.get(o + 5)?,
        a: r.get(o + 6)?,
        v: r.get(o + 7)?,
        t: r.get(o + 8)?,
        local_percentile: r.get(o + 9)?,
        local_count: unsigned(r, o + 10)?,
        support_k: r.get(o + 11)?,
        cohort_level: r.get(o + 12)?,
        time_reason: r.get(o + 13)?,
        artist_support: unsigned(r, o + 14)?,
        main_score: r.get(o + 15)?,
        rescue_score: r.get(o + 16)?,
        main_rank: optional_unsigned(r, o + 17)?,
        rescue_rank: optional_unsigned(r, o + 18)?,
        selected_route: decode_enum(r, o + 19)?,
        duplicate_of: optional_unsigned(r, o + 20)?,
    })
}

pub struct RankingInputTable {
    db: Connection,
    batch: WriteBatch,
}
impl RankingInputTable {
    pub fn create(path: &Path) -> Result<Self> {
        let db = connect(path, INPUT_ID, true, true)?;
        db.execute_batch("CREATE TABLE input_rows(ordinal INTEGER PRIMARY KEY,source_id TEXT NOT NULL,asset_id BLOB NOT NULL,record_id BLOB,observation_id BLOB,post_id INTEGER,rating TEXT,created_at_us INTEGER,observed_at_us INTEGER,updated_at_us INTEGER,time_quality TEXT NOT NULL,source_priority INTEGER,fav_count INTEGER,up_score INTEGER,down_score INTEGER,score INTEGER,artists TEXT NOT NULL,parent_id INTEGER,stored_width INTEGER,stored_height INTEGER,dimension_basis TEXT NOT NULL,stored_extension TEXT NOT NULL,stored_bytes INTEGER NOT NULL,is_banned INTEGER,is_deleted INTEGER,is_pending INTEGER,is_flagged INTEGER,damage_classes INTEGER NOT NULL,tags_known INTEGER NOT NULL,record_count INTEGER NOT NULL,rating_conflict INTEGER NOT NULL,basis_ids TEXT NOT NULL,source_issues TEXT); CREATE UNIQUE INDEX input_identity ON input_rows(source_id,asset_id); CREATE INDEX input_rating ON input_rows(rating,ordinal); CREATE TABLE members(ordinal INTEGER PRIMARY KEY,source_id TEXT NOT NULL,asset_id BLOB NOT NULL,bases TEXT NOT NULL); CREATE INDEX members_source ON members(source_id,ordinal);").map_err(db_error)?;
        Ok(Self {
            db,
            batch: WriteBatch::default(),
        })
    }
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            db: connect(path, INPUT_ID, false, false)?,
            batch: WriteBatch::default(),
        })
    }
    pub fn set_meta(&self, key: &str, value: &impl Serialize) -> Result<()> {
        self.flush()?;
        put_meta(&self.db, key, value)
    }
    pub fn flush(&self) -> Result<()> {
        self.batch.flush(&self.db)
    }
    pub fn finalize(&self, ratings: &[String]) -> Result<()> {
        self.flush()?;
        // At most one source identity represents the same bytes across a multi-source scope.
        let wanted = ratings
            .iter()
            .map(|v| format!("'{}'", v.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        self.db.execute_batch("CREATE INDEX IF NOT EXISTS input_hash ON input_rows(asset_id,ordinal); CREATE INDEX IF NOT EXISTS input_post ON input_rows(post_id,ordinal) WHERE post_id IS NOT NULL; CREATE TABLE IF NOT EXISTS duplicate_members(ordinal INTEGER PRIMARY KEY,duplicate_of INTEGER NOT NULL); DELETE FROM duplicate_members;").map_err(db_error)?;
        self.db.execute_batch(&format!("WITH duplicates AS (SELECT *,CASE WHEN time_quality IN ('exact','date_only') THEN observed_at_us/86400000000 ELSE NULL END AS observed_day FROM input_rows WHERE asset_id IN (SELECT asset_id FROM input_rows GROUP BY asset_id HAVING count(*)>1)),precision AS (SELECT *,max(CASE WHEN time_quality='date_only' THEN 1 ELSE 0 END) OVER (PARTITION BY asset_id,observed_day) AS coarse_day FROM duplicates) INSERT INTO duplicate_members SELECT ordinal,representative FROM (SELECT ordinal,first_value(ordinal) OVER (PARTITION BY asset_id ORDER BY record_id IS NULL,CASE WHEN rating IN ({wanted}) THEN 0 ELSE 1 END,observed_day DESC NULLS LAST,CASE WHEN coarse_day=0 THEN observed_at_us ELSE NULL END DESC NULLS LAST,updated_at_us DESC NULLS LAST,source_priority DESC NULLS LAST,observation_id,record_id,source_id,ordinal) AS representative FROM precision) WHERE ordinal!=representative;")).map_err(db_error)?;
        Ok(())
    }
    pub fn meta<T: DeserializeOwned>(&self, key: &str) -> Result<T> {
        get_meta(&self.db, key)
    }
    pub fn append_members(&mut self, rows: &[(u64, AssetKey, Vec<u32>)]) -> Result<()> {
        if rows.len() > 512 {
            return Err(Error::invalid("排名成员批次超出范围"));
        }
        if rows.is_empty() {
            return Ok(());
        }
        self.batch.begin(&self.db)?;
        let result = (|| -> Result<()> {
            let mut stmt = self
                .db
                .prepare_cached("INSERT INTO members VALUES (?1,?2,?3,?4)")
                .map_err(db_error)?;
            for (ordinal, key, basis) in rows {
                stmt.execute(params![
                    *ordinal as i64,
                    key.source_id,
                    bytes(&key.asset_id)?,
                    serde_json::to_string(basis).map_err(Error::io)?
                ])
                .map_err(db_error)?;
            }
            Ok(())
        })();
        self.batch.finish(&self.db, rows.len(), result)
    }
    pub fn members(
        &self,
        source: &str,
        after: Option<u64>,
    ) -> Result<Vec<(u64, AssetKey, Vec<u32>)>> {
        let mut stmt=self.db.prepare("SELECT ordinal,asset_id,bases FROM members WHERE source_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT 512").map_err(db_error)?;
        stmt.query_map(
            params![source, after.map(|n| n as i64).unwrap_or(-1)],
            |r| {
                Ok((
                    unsigned(r, 0)?,
                    AssetKey {
                        source_id: source.into(),
                        asset_id: hex::encode(r.get::<_, Vec<u8>>(1)?),
                    },
                    decode(r, 2)?,
                ))
            },
        )
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }
    pub fn append(&mut self, rows: &[RankingInput]) -> Result<()> {
        if rows.len() > 512 {
            return Err(Error::invalid("排名字段批次超出范围"));
        }
        if rows.is_empty() {
            return Ok(());
        }
        self.batch.begin(&self.db)?;
        let result = (|| -> Result<()> {
            let mut stmt=self.db.prepare_cached(&format!("INSERT INTO input_rows({INPUT_COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33)")).map_err(db_error)?;
            let v2 = self.is_v2()?;
            let mut extra = if v2 {
                Some(
                    self.db
                        .prepare_cached("UPDATE input_rows SET tags=?1 WHERE ordinal=?2")
                        .map_err(db_error)?,
                )
            } else {
                None
            };
            for v in rows {
                let asset = bytes(&v.asset_id)?;
                let record = v.record_id.as_deref().map(bytes).transpose()?;
                let observation = v.observation_id.as_deref().map(bytes).transpose()?;
                stmt.execute(params![
                    v.ordinal as i64,
                    v.source_id,
                    asset,
                    record,
                    observation,
                    v.post_id,
                    v.rating,
                    v.created_at_us,
                    v.observed_at_us,
                    v.updated_at_us,
                    v.time_quality,
                    v.source_priority,
                    v.fav_count,
                    v.up_score,
                    v.down_score,
                    v.score,
                    serde_json::to_string(&v.artists).map_err(Error::io)?,
                    v.parent_id,
                    v.stored_width,
                    v.stored_height,
                    v.dimension_basis,
                    v.stored_extension,
                    i64::try_from(v.stored_bytes)
                        .map_err(|_| Error::invalid("存储大小超出范围"))?,
                    v.is_banned,
                    v.is_deleted,
                    v.is_pending,
                    v.is_flagged,
                    v.damage_classes,
                    v.tags_known,
                    v.record_count,
                    v.rating_conflict,
                    serde_json::to_string(&v.basis_ids).map_err(Error::io)?,
                    v.source_issues
                ])
                .map_err(db_error)?;
                if let Some(extra) = extra.as_mut() {
                    extra
                        .execute(params![v.tags, v.ordinal as i64])
                        .map_err(db_error)?;
                }
            }
            Ok(())
        })();
        self.batch.finish(&self.db, rows.len(), result)
    }
    pub fn count(&self) -> Result<u64> {
        self.db
            .query_row("SELECT count(*) FROM input_rows", [], |r| unsigned(r, 0))
            .map_err(db_error)
    }
    pub fn verify_members(&self, expected: u64) -> Result<()> {
        let counts: (u64, u64) = self
            .db
            .query_row(
                "SELECT (SELECT count(*) FROM members),(SELECT count(*) FROM input_rows)",
                [],
                |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
            )
            .map_err(db_error)?;
        let invalid:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM members m LEFT JOIN input_rows i USING(ordinal) WHERE i.ordinal IS NULL OR m.source_id!=i.source_id OR m.asset_id!=i.asset_id)",[],|r|r.get(0)).map_err(db_error)?;
        if counts != (expected, expected) || invalid {
            return Err(Error::new("INPUT_INVALID", "固定元数据与任务成员不一致"));
        }
        Ok(())
    }
    pub fn page(&self, after: Option<u64>, rating: Option<&str>) -> Result<Vec<RankingInput>> {
        let sql = format!(
            "SELECT {} FROM input_rows i LEFT JOIN duplicate_members d USING(ordinal) WHERE i.ordinal>?1 {} ORDER BY i.ordinal LIMIT 512",
            input_columns(self.is_v2()?),
            if rating.is_some() {
                "AND i.rating=?2"
            } else {
                ""
            }
        );
        let mut stmt = self.db.prepare(&sql).map_err(db_error)?;
        let mut values = vec![SqlValue::Integer(after.map(|n| n as i64).unwrap_or(-1))];
        if let Some(rating) = rating {
            values.push(SqlValue::Text(rating.into()));
        }
        stmt.query_map(rusqlite::params_from_iter(values), |r| read_input(r, 0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn row(&self, ordinal: u64) -> Result<RankingInput> {
        self.db.query_row(&format!("SELECT {} FROM input_rows i LEFT JOIN duplicate_members d USING(ordinal) WHERE i.ordinal=?1",input_columns(self.is_v2()?)),[ordinal as i64],|r|read_input(r,0)).map_err(db_error)
    }
    pub fn rating_counts(&self) -> Result<Vec<(Option<String>, u64)>> {
        let mut stmt = self
            .db
            .prepare("SELECT rating,count(*) FROM input_rows GROUP BY rating ORDER BY rating")
            .map_err(db_error)?;
        stmt.query_map([], |r| Ok((r.get(0)?, unsigned(r, 1)?)))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn duplicate_of(&self, ordinal: u64) -> Result<Option<u64>> {
        self.db
            .query_row(
                "SELECT duplicate_of FROM duplicate_members WHERE ordinal=?1",
                [ordinal as i64],
                |r| unsigned(r, 0),
            )
            .optional()
            .map_err(db_error)
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct RankingPosition {
    pub group: String,
    pub position: i64,
    pub ordinal: u64,
}
pub struct RankingResultTable {
    db: Connection,
    batch: WriteBatch,
}
impl RankingResultTable {
    pub fn create(path: &Path) -> Result<Self> {
        let db = connect(path, SCORE_ID, true, true)?;
        db.execute_batch("CREATE TABLE scores(ordinal INTEGER PRIMARY KEY,rating TEXT,eligibility TEXT NOT NULL,missing_flags TEXT NOT NULL,g REAL,c REAL,a REAL,v REAL,t REAL,local_percentile REAL,local_count INTEGER NOT NULL,support_k REAL,cohort_level INTEGER,time_reason TEXT NOT NULL,artist_support INTEGER NOT NULL,main_score REAL,rescue_score REAL,main_rank INTEGER,rescue_rank INTEGER,selected_route TEXT NOT NULL,duplicate_of INTEGER); CREATE INDEX scores_rating ON scores(rating,ordinal);").map_err(db_error)?;
        Ok(Self {
            db,
            batch: WriteBatch::default(),
        })
    }
    pub fn resume(path: &Path) -> Result<Self> {
        Ok(Self {
            db: connect(path, SCORE_ID, false, true)?,
            batch: WriteBatch::default(),
        })
    }
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            db: connect(path, SCORE_ID, false, false)?,
            batch: WriteBatch::default(),
        })
    }
    pub fn set_meta(&self, key: &str, value: &impl Serialize) -> Result<()> {
        self.flush()?;
        put_meta(&self.db, key, value)
    }
    pub fn flush(&self) -> Result<()> {
        self.batch.flush(&self.db)
    }
    pub fn meta<T: DeserializeOwned>(&self, key: &str) -> Result<T> {
        get_meta(&self.db, key)
    }
    pub fn append(&mut self, rows: &[RankingScores]) -> Result<()> {
        if rows.len() > 512 {
            return Err(Error::invalid("排名成果批次超出范围"));
        }
        if rows.is_empty() {
            return Ok(());
        }
        self.batch.begin(&self.db)?;
        let result = (|| -> Result<()> {
            let mut stmt=self.db.prepare_cached(&format!("INSERT INTO scores({SCORE_COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)")).map_err(db_error)?;
            let mut extra = if self.is_v2()? {
                Some(self.db.prepare_cached("UPDATE scores SET v2_json=?1,direct_rank=?2,fused_rank=?3,created_year=?4,year_rank=?5 WHERE ordinal=?6").map_err(db_error)?)
            } else {
                None
            };
            for v in rows {
                for x in [
                    v.g,
                    v.c,
                    v.a,
                    v.v,
                    v.t,
                    v.local_percentile,
                    v.support_k,
                    v.main_score,
                    v.rescue_score,
                ]
                .into_iter()
                .flatten()
                {
                    if !x.is_finite() {
                        return Err(Error::new("RANKING_INVALID", "排名成果出现非有限数值"));
                    }
                }
                stmt.execute(params![
                    v.ordinal as i64,
                    v.rating,
                    enum_text(&v.eligibility)?,
                    serde_json::to_string(&v.missing_flags).map_err(Error::io)?,
                    v.g,
                    v.c,
                    v.a,
                    v.v,
                    v.t,
                    v.local_percentile,
                    v.local_count as i64,
                    v.support_k,
                    v.cohort_level,
                    v.time_reason,
                    v.artist_support as i64,
                    v.main_score,
                    v.rescue_score,
                    v.main_rank.map(|x| x as i64),
                    v.rescue_rank.map(|x| x as i64),
                    enum_text(&v.selected_route)?,
                    v.duplicate_of.map(|x| x as i64)
                ])
                .map_err(db_error)?;
                if let Some(extra) = extra.as_mut() {
                    extra
                        .execute(params![
                            v.v2.map(|x| serde_json::to_string(&x).map_err(Error::io))
                                .transpose()?,
                            v.v2.map(|x| x.direct_rank as i64),
                            v.v2.map(|x| x.fused_rank as i64),
                            v.v2.and_then(|x| x.created_year),
                            v.v2.map(|x| x.year_rank as i64),
                            v.ordinal as i64
                        ])
                        .map_err(db_error)?;
                }
            }
            Ok(())
        })();
        self.batch.finish(&self.db, rows.len(), result)
    }
    pub fn delete_rating(&self, rating: &str) -> Result<()> {
        self.flush()?;
        self.db
            .execute("DELETE FROM scores WHERE rating=?1", [rating])
            .map_err(db_error)?;
        Ok(())
    }
    pub fn delete_eligible_rating(&self, rating: &str) -> Result<()> {
        self.flush()?;
        self.db
            .execute(
                "DELETE FROM scores WHERE rating=?1 AND eligibility='eligible'",
                [rating],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn reset(&self) -> Result<()> {
        self.flush()?;
        self.db
            .execute("DELETE FROM scores", [])
            .map_err(db_error)?;
        Ok(())
    }
    pub fn count(&self) -> Result<u64> {
        self.db
            .query_row("SELECT count(*) FROM scores", [], |r| unsigned(r, 0))
            .map_err(db_error)
    }
    pub fn finish(&self, summary: &RankingSummary) -> Result<()> {
        self.flush()?;
        self.db.execute_batch("CREATE INDEX IF NOT EXISTS scores_main ON scores(rating,coalesce(main_rank,9223372036854775807),ordinal); CREATE INDEX IF NOT EXISTS scores_rescue ON scores(rating,coalesce(rescue_rank,9223372036854775807),ordinal); CREATE INDEX IF NOT EXISTS scores_route_main ON scores(rating,selected_route,coalesce(main_rank,9223372036854775807),ordinal); CREATE INDEX IF NOT EXISTS scores_eligibility_main ON scores(rating,eligibility,coalesce(main_rank,9223372036854775807),ordinal);").map_err(db_error)?;
        if self.is_v2()? {
            self.db.execute_batch("CREATE INDEX IF NOT EXISTS scores_direct ON scores(rating,coalesce(direct_rank,9223372036854775807),ordinal); CREATE INDEX IF NOT EXISTS scores_fused ON scores(rating,coalesce(fused_rank,9223372036854775807),ordinal); CREATE INDEX IF NOT EXISTS scores_year ON scores(rating,created_year,year_rank,ordinal);").map_err(db_error)?;
        }
        put_meta(&self.db, "summary", summary)?;
        put_meta(&self.db, "complete", &true)?;
        Ok(())
    }
    pub fn counts(&self, column: &str) -> Result<std::collections::BTreeMap<String, u64>> {
        if !matches!(column, "eligibility" | "selected_route") {
            return Err(Error::invalid("不支持该统计字段"));
        }
        let mut stmt = self
            .db
            .prepare(&format!(
                "SELECT {column},count(*) FROM scores GROUP BY {column}"
            ))
            .map_err(db_error)?;
        stmt.query_map([], |r| Ok((r.get(0)?, unsigned(r, 1)?)))
            .map_err(db_error)?
            .collect::<std::result::Result<_, _>>()
            .map_err(db_error)
    }
    pub fn missing_counts(&self) -> Result<std::collections::BTreeMap<String, u64>> {
        let mut stmt=self.db.prepare("SELECT j.value,count(*) FROM scores s,json_each(s.missing_flags) j GROUP BY j.value").map_err(db_error)?;
        stmt.query_map([], |r| Ok((r.get(0)?, unsigned(r, 1)?)))
            .map_err(db_error)?
            .collect::<std::result::Result<_, _>>()
            .map_err(db_error)
    }
    pub fn all_page(&self, after: Option<u64>) -> Result<Vec<RankingScores>> {
        let mut stmt = self
            .db
            .prepare(&format!(
                "SELECT {} FROM scores WHERE ordinal>?1 ORDER BY ordinal LIMIT 512",
                self.score_columns()?
            ))
            .map_err(db_error)?;
        stmt.query_map([after.map(|n| n as i64).unwrap_or(-1)], |r| {
            read_scores(r, 0)
        })
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }
    pub fn row(&self, ordinal: u64) -> Result<RankingScores> {
        self.db
            .query_row(
                &format!(
                    "SELECT {} FROM scores WHERE ordinal=?1",
                    self.score_columns()?
                ),
                [ordinal as i64],
                |r| read_scores(r, 0),
            )
            .map_err(db_error)
    }
    pub fn filtered_page(
        &self,
        filter: &RankingFilter,
        after: Option<&RankingPosition>,
        limit: usize,
    ) -> Result<(Vec<RankingScores>, Option<RankingPosition>)> {
        let limit = limit.clamp(1, 128);
        let mut after = after.cloned();
        let mut picked = Vec::new();
        loop {
            let scan = self.browse_scan(filter, filter.order, false, after.as_ref(), 512)?;
            for (row, matches) in scan.rows {
                after = Some(RankingPosition::for_scores(&row, filter.order));
                if matches {
                    picked.push(row);
                }
                if picked.len() > limit {
                    picked.truncate(limit);
                    let next = picked
                        .last()
                        .map(|row| RankingPosition::for_scores(row, filter.order));
                    return Ok((picked, next));
                }
            }
            if !scan.more {
                return Ok((picked, None));
            }
        }
    }
    pub fn cancel_reads(&self, cancelled: studio_application::ReadCancellation) -> Result<()> {
        self.db
            .progress_handler(
                1000,
                Some(move || cancelled.load(std::sync::atomic::Ordering::Acquire)),
            )
            .map_err(db_error)
    }
    pub fn known_count(&self, filter: &RankingFilter) -> Result<Option<u64>> {
        filter.validate()?;
        let summary = self.meta::<RankingSummary>("summary")?;
        if filter.route.is_some() || filter.missing_only || filter.selected_only {
            return Ok(None);
        }
        if filter.top.is_none() && filter.eligibility == Some(RankingEligibility::Eligible) {
            let eligible = summary
                .ratings
                .iter()
                .filter(|r| filter.rating.as_ref().is_none_or(|v| v == &r.rating));
            return Ok(Some(eligible.map(|r| r.eligible).sum()));
        }
        if filter.top.is_none() && filter.rating.is_none() {
            return Ok(Some(match &filter.eligibility {
                None => summary.input_count,
                Some(value) => summary
                    .eligibility_counts
                    .get(&enum_text(value)?)
                    .copied()
                    .unwrap_or(0),
            }));
        }
        Ok(None)
    }
    pub fn count_scan(&self, filter: &RankingFilter, from: u64, total: u64) -> Result<(u64, u64)> {
        if from > total || total > i64::MAX as u64 {
            return Err(Error::invalid("排名统计位置无效"));
        }
        let to = from.saturating_add(262_144).min(total);
        let (predicate, mut values) = filter_sql(&self.compatible_filter(filter)?)?;
        values.push(SqlValue::Integer(from as i64));
        let lower = values.len();
        values.push(SqlValue::Integer(to as i64));
        let count = self.db.query_row(&format!("SELECT coalesce(sum(CASE WHEN ({predicate}) THEN 1 ELSE 0 END),0) FROM scores WHERE ordinal>=?{lower} AND ordinal<?{}", values.len()), rusqlite::params_from_iter(values), |r| unsigned(r,0)).map_err(db_error)?;
        Ok((to, count))
    }
    pub fn filtered_count(&self, filter: &RankingFilter) -> Result<u64> {
        if !self.is_v2()? && matches!(filter.order, RankingOrder::Direct | RankingOrder::Fused) {
            return self.filtered_count(&self.compatible_filter(filter)?);
        }
        filter.validate()?;
        if let Some(count) = self.known_count(filter)? {
            return Ok(count);
        }
        let summary = self.meta::<RankingSummary>("summary")?;
        if filter.top.is_some() && filter.rating.is_none() {
            // Ranks exist only in the summary's eligible rating populations.
            // Constrain each rating so the expression index can seek to top N.
            return summary.ratings.iter().try_fold(0, |count, rating| {
                let mut part = filter.clone();
                part.rating = Some(rating.rating.clone());
                self.filtered_count(&part).map(|n| count + n)
            });
        }
        let (sql, values) = filter_sql(&self.compatible_filter(filter)?)?;
        let index = if filter.top.is_some() {
            format!(
                " INDEXED BY {}",
                score_index(
                    filter,
                    match filter.order {
                        RankingOrder::Input => RankingOrder::Main,
                        order => order,
                    }
                )
            )
        } else {
            String::new()
        };
        self.db
            .query_row(
                &format!("SELECT count(*) FROM scores{index} WHERE {sql}"),
                rusqlite::params_from_iter(values),
                |r| unsigned(r, 0),
            )
            .map_err(db_error)
    }
}
pub(crate) fn filter_sql(f: &RankingFilter) -> Result<(String, Vec<SqlValue>)> {
    f.validate()?;
    let mut clauses = vec!["1=1".to_owned()];
    let mut values = Vec::new();
    for (column, value) in [
        ("rating", f.rating.clone()),
        (
            "selected_route",
            f.route.as_ref().map(enum_text).transpose()?,
        ),
        (
            "eligibility",
            f.eligibility.as_ref().map(enum_text).transpose()?,
        ),
    ] {
        if let Some(value) = value {
            values.push(SqlValue::Text(value));
            clauses.push(format!("{column}=?{}", values.len()));
        }
    }
    if f.missing_only {
        clauses.push("missing_flags!='[]'".into());
    }
    if f.selected_only {
        clauses.push("selected_route IN ('main','rescue','audit')".into());
    }
    if let Some(top) = f.top {
        if f.rating.is_none() {
            // Only these eligible populations receive ranks. Explicit alternatives
            // let SQLite seek each rating prefix in pre-existing expression indexes.
            clauses.push("rating IN ('e','g','q','s')".into());
        }
        let rank = match f.order {
            RankingOrder::Rescue => "rescue_rank",
            RankingOrder::Direct => "direct_rank",
            RankingOrder::Fused => "fused_rank",
            _ => "main_rank",
        };
        values.push(SqlValue::Integer(top as i64));
        clauses.push(format!(
            "{rank} IS NOT NULL AND coalesce({rank},9223372036854775807)<=?{}",
            values.len()
        ));
    }
    Ok((clauses.join(" AND "), values))
}

impl RankingInputTable {
    pub fn is_v2(&self) -> Result<bool> {
        self.db
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .map(|v| v == 2)
            .map_err(db_error)
    }
    pub fn create_v2(path: &Path) -> Result<Self> {
        let table = Self::create(path)?;
        table
            .db
            .execute_batch("ALTER TABLE input_rows ADD COLUMN tags TEXT; PRAGMA user_version=2;")
            .map_err(db_error)?;
        Ok(table)
    }
}
impl RankingResultTable {
    pub fn is_v2(&self) -> Result<bool> {
        self.db
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .map(|v| v == 2)
            .map_err(db_error)
    }
    fn compatible_filter(&self, filter: &RankingFilter) -> Result<RankingFilter> {
        let mut f = filter.clone();
        if !self.is_v2()? && matches!(f.order, RankingOrder::Direct | RankingOrder::Fused) {
            f.order = RankingOrder::Main;
        }
        Ok(f)
    }
    fn score_columns(&self) -> Result<String> {
        Ok(format!(
            "{SCORE_COLUMNS},{}",
            if self.is_v2()? { "v2_json" } else { "NULL" }
        ))
    }
    pub fn create_v2(path: &Path) -> Result<Self> {
        let table = Self::create(path)?;
        table.db.execute_batch("ALTER TABLE scores ADD COLUMN v2_json TEXT; ALTER TABLE scores ADD COLUMN direct_rank INTEGER; ALTER TABLE scores ADD COLUMN fused_rank INTEGER; ALTER TABLE scores ADD COLUMN created_year INTEGER; ALTER TABLE scores ADD COLUMN year_rank INTEGER; PRAGMA user_version=2;").map_err(db_error)?;
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_pages_preserve_group_null_tie_filter_and_cursor_semantics() {
        let directory = tempfile::tempdir().unwrap();
        let mut table =
            RankingResultTable::create(&directory.path().join("scores.sqlite")).unwrap();
        let mut summary = RankingSummary {
            input_count: 512,
            ..Default::default()
        };
        let ratings = [
            Some(""),
            Some("e"),
            Some("g"),
            Some("q"),
            Some("s"),
            None,
            Some("z"),
            Some("zz"),
        ];
        let rows = (0..512u64)
            .map(|ordinal| {
                let rating = ratings[ordinal as usize % ratings.len()];
                let eligible = matches!(rating, Some("e" | "g" | "q" | "s")) && ordinal % 11 != 0;
                let eligibility = if eligible {
                    RankingEligibility::Eligible
                } else {
                    RankingEligibility::RatingUnknown
                };
                *summary
                    .eligibility_counts
                    .entry(enum_text(&eligibility).unwrap())
                    .or_default() += 1;
                if eligible {
                    summary.eligible_count += 1;
                    let rating = rating.unwrap();
                    if let Some(part) = summary
                        .ratings
                        .iter_mut()
                        .find(|part| part.rating == rating)
                    {
                        part.eligible += 1;
                    } else {
                        summary.ratings.push(RankingRatingSummary {
                            rating: rating.into(),
                            eligible: 1,
                            ..Default::default()
                        });
                    }
                }
                RankingScores {
                    ordinal,
                    rating: rating.map(str::to_owned),
                    eligibility,
                    main_rank: eligible.then_some(1 + ordinal % 29),
                    rescue_rank: eligible.then_some(1 + (ordinal * 7) % 31),
                    selected_route: if eligible {
                        [
                            RankingRoute::Main,
                            RankingRoute::Rescue,
                            RankingRoute::Audit,
                            RankingRoute::BudgetRejected,
                        ][(ordinal / 8) as usize % 4]
                    } else {
                        RankingRoute::Ineligible
                    },
                    missing_flags: if ordinal % 3 == 0 {
                        vec!["artist_unknown".into()]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        table.append(&rows).unwrap();
        table.finish(&summary).unwrap();
        for order in [
            RankingOrder::Main,
            RankingOrder::Rescue,
            RankingOrder::Input,
        ] {
            for rating in [None, Some("g".to_owned())] {
                for variant in 0..8 {
                    let filter = RankingFilter {
                        order,
                        rating: rating.clone(),
                        eligibility: (variant == 1)
                            .then_some(RankingEligibility::Eligible)
                            .or_else(|| {
                                (variant == 7).then_some(RankingEligibility::RatingUnknown)
                            }),
                        top: if variant == 2 {
                            Some(3)
                        } else if variant == 3 {
                            Some(i64::MAX as u64)
                        } else {
                            None
                        },
                        route: (variant == 4).then_some(RankingRoute::Main),
                        selected_only: variant == 5,
                        missing_only: variant == 6,
                    };
                    // Independent historical SQL oracle, including its exact null semantics.
                    let rank = if order == RankingOrder::Rescue {
                        "rescue_rank"
                    } else {
                        "main_rank"
                    };
                    let position = if order == RankingOrder::Input {
                        "ordinal".into()
                    } else {
                        format!("coalesce({rank},9223372036854775807)")
                    };
                    let mut predicate = String::from("1=1");
                    if rating.is_some() {
                        predicate.push_str(" AND rating='g'");
                    }
                    if variant == 1 {
                        predicate.push_str(" AND eligibility='eligible'");
                    }
                    if variant == 7 {
                        predicate.push_str(" AND eligibility='rating_unknown'");
                    }
                    if let Some(top) = filter.top {
                        predicate.push_str(&format!(" AND {rank}<={top}"));
                    }
                    if variant == 4 {
                        predicate.push_str(" AND selected_route='main'");
                    }
                    if variant == 5 {
                        predicate.push_str(" AND selected_route IN ('main','rescue','audit')");
                    }
                    if variant == 6 {
                        predicate.push_str(" AND missing_flags!='[]'");
                    }
                    let expected = table.db.prepare(&format!("SELECT ordinal FROM scores WHERE {predicate} ORDER BY coalesce(rating,'z'),{position},ordinal")).unwrap().query_map([], |r| unsigned(r, 0)).unwrap().collect::<std::result::Result<Vec<_>, _>>().unwrap();
                    assert_eq!(
                        table.filtered_count(&filter).unwrap(),
                        expected.len() as u64
                    );
                    let mut actual = Vec::new();
                    let mut after = None;
                    loop {
                        let (page, next) = table.filtered_page(&filter, after.as_ref(), 7).unwrap();
                        actual.extend(page.iter().map(|row| row.ordinal));
                        assert!(actual.len() <= 512, "cursor must advance");
                        after = next;
                        if after.is_none() {
                            break;
                        }
                    }
                    assert_eq!(
                        actual, expected,
                        "order={order:?}, rating={rating:?}, variant={variant}"
                    );
                }
            }
        }
    }

    #[test]
    fn buffered_rows_commit_at_boundaries_and_rollback_unfinished_batches() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scores.sqlite");
        let mut table = RankingResultTable::create(&path).unwrap();
        let rows = |start| {
            (start..start + 512)
                .map(|ordinal| RankingScores {
                    ordinal,
                    ..Default::default()
                })
                .collect::<Vec<_>>()
        };
        table.append(&rows(0)).unwrap();
        assert_eq!(RankingResultTable::open(&path).unwrap().count().unwrap(), 0);
        table.flush().unwrap();
        assert_eq!(
            RankingResultTable::open(&path).unwrap().count().unwrap(),
            512
        );
        table.append(&rows(512)).unwrap();
        assert!(table.append(&rows(0)).is_err());
        assert!(table.db.is_autocommit());
        assert_eq!(table.count().unwrap(), 512);
        table.append(&rows(512)).unwrap();
        drop(table);
        let table = RankingResultTable::resume(&path).unwrap();
        assert_eq!(table.count().unwrap(), 512);
        let second = directory.path().join("automatic.sqlite");
        let mut table = RankingResultTable::create(&second).unwrap();
        for batch in 0..16 {
            table.append(&rows(batch * 512)).unwrap();
        }
        assert!(table.db.is_autocommit());
        assert_eq!(
            RankingResultTable::open(&second).unwrap().count().unwrap(),
            8192
        );
    }

    #[test]
    fn cross_source_duplicates_respect_date_precision_and_unknown_time() {
        let directory = tempfile::tempdir().unwrap();
        let table = RankingInputTable::create(&directory.path().join("input.sqlite")).unwrap();
        let sql = "INSERT INTO input_rows(ordinal,source_id,asset_id,record_id,observation_id,rating,observed_at_us,updated_at_us,time_quality,artists,dimension_basis,stored_extension,stored_bytes,damage_classes,tags_known,record_count,rating_conflict,basis_ids) VALUES (?1,?2,zeroblob(32),?3,?3,'g',?4,?5,?6,'[]','not_requested','png',1,0,1,1,0,'[]')";
        table
            .db
            .execute(
                sql,
                params![1, "source-a", vec![1u8; 32], 0i64, 100i64, "date_only"],
            )
            .unwrap();
        table
            .db
            .execute(
                sql,
                params![
                    2,
                    "source-b",
                    vec![2u8; 32],
                    43_200_000_000i64,
                    99i64,
                    "exact"
                ],
            )
            .unwrap();
        table.finalize(&["g".into()]).unwrap();
        assert_eq!(table.row(2).unwrap().duplicate_of, Some(1));
        // A later UTC day is newer even when its source update field is older.
        table
            .db
            .execute(
                "UPDATE input_rows SET observed_at_us=86400000000 WHERE ordinal=2",
                [],
            )
            .unwrap();
        table.finalize(&["g".into()]).unwrap();
        assert_eq!(table.row(1).unwrap().duplicate_of, Some(2));
        // An unqualified timestamp cannot masquerade as a precise observation.
        table
            .db
            .execute(
                "UPDATE input_rows SET time_quality='unknown' WHERE ordinal=2",
                [],
            )
            .unwrap();
        table.finalize(&["g".into()]).unwrap();
        assert_eq!(table.row(2).unwrap().duplicate_of, Some(1));
    }
}
