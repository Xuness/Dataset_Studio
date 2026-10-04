use crate::ranking_tables::RankingPosition;
use crate::{db_error, unsigned};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
};
use studio_application::read_cancelled;
use studio_domain::*;
#[cfg(test)]
#[path = "ranked_index_tests.rs"]
mod tests;

// One bookmark per page-sized block keeps random positioning bounded even in
// sparse scopes, without a second dense copy of every ranking order.
const POSITION_STRIDE: u64 = 128;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RankedIndexMeta {
    pub version: u32,
    pub key: String,
    pub scope: ScopeRef,
    pub count: u64,
}
#[derive(Clone)]
pub struct RankedIndexPlan {
    pub requested_scope: ScopeRef,
    pub meta: RankedIndexMeta,
    pub project: PathBuf,
    pub input: PathBuf,
    pub scores: PathBuf,
}
#[derive(Default)]
pub struct RankedIndexProgress {
    pub completed: AtomicU64,
    pub phase: AtomicU8,
}
pub struct RankedIndex {
    db: Connection,
    pub meta: RankedIndexMeta,
}
fn uri(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix("\\\\?\\")
        .unwrap_or(&text)
        .replace('\\', "/");
    format!(
        "file:{}?mode=ro",
        text.replace('%', "%25")
            .replace('?', "%3F")
            .replace('#', "%23")
    )
}
fn invalid() -> Error {
    Error::new(
        "RANKING_INDEX_INVALID",
        "当前范围的排名索引不完整或版本不匹配",
    )
}
fn build_error(error: rusqlite::Error) -> Error {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull) {
        Error::new(
            "CACHE_BUDGET_EXCEEDED",
            "排名索引空间不足或超过可用缓存预算",
        )
    } else {
        db_error(error)
    }
}
fn order_parts(order: RankingOrder) -> (&'static str, &'static str) {
    match order {
        RankingOrder::Main => ("main", "main_rank"),
        RankingOrder::Rescue => ("rescue", "rescue_rank"),
        RankingOrder::Input => ("input", "ordinal"),
        RankingOrder::Direct => ("direct", "direct_rank"),
        RankingOrder::Fused => ("fused", "fused_rank"),
    }
}
impl RankedIndex {
    /// Only the private output is writable. Project members and both immutable
    /// ranking files are attached read-only and detached before index sorting.
    pub fn build(
        output: &Path,
        plan: &RankedIndexPlan,
        memory_bytes: u64,
        max_bytes: u64,
        cancelled: Arc<AtomicBool>,
        progress: Arc<RankedIndexProgress>,
    ) -> Result<()> {
        plan.meta
            .scope
            .validate_project(&plan.meta.scope.project_id)?;
        read_cancelled(&cancelled)?;
        if max_bytes < 65536 {
            return Err(Error::new(
                "CACHE_BUDGET_EXCEEDED",
                "当前范围排名索引的可用缓存预算不足",
            ));
        }
        let db = Connection::open(output).map_err(db_error)?;
        let kib = (memory_bytes / 8 / 1024).clamp(2048, 65536);
        db.execute_batch(&format!("PRAGMA main.journal_mode=OFF; PRAGMA main.synchronous=OFF; PRAGMA main.cache_size=-{kib}; PRAGMA temp_store=FILE; CREATE TABLE members(ordinal INTEGER PRIMARY KEY,rating TEXT NOT NULL,main_rank INTEGER NOT NULL,rescue_rank INTEGER NOT NULL,post_id INTEGER); CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;")).map_err(db_error)?;
        if plan.meta.version >= 2 {
            db.execute_batch("ALTER TABLE members ADD COLUMN direct_rank INTEGER NOT NULL DEFAULT 9223372036854775807; ALTER TABLE members ADD COLUMN fused_rank INTEGER NOT NULL DEFAULT 9223372036854775807;").map_err(db_error)?;
        }
        let page_size: i64 = db
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(db_error)?;
        db.pragma_update(
            None,
            "max_page_count",
            (max_bytes / page_size as u64).min(i64::MAX as u64) as i64,
        )
        .map_err(db_error)?;
        for (name, path) in [
            ("scope_db", &plan.project),
            ("fixed_input", &plan.input),
            ("fixed_scores", &plan.scores),
        ] {
            db.execute(&format!("ATTACH DATABASE ?1 AS {name}"), [uri(path)])
                .map_err(db_error)?;
            db.execute_batch(&format!("PRAGMA {name}.cache_size=-{kib};"))
                .map_err(db_error)?;
        }
        let flag = cancelled.clone();
        db.progress_handler(1000, Some(move || flag.load(Ordering::Acquire)))
            .map_err(db_error)?;
        let counts = progress.clone();
        db.update_hook(Some(move |action, database: &str, table: &str, _| {
            if action == rusqlite::hooks::Action::SQLITE_INSERT
                && database == "main"
                && table == "members"
            {
                counts.completed.fetch_add(1, Ordering::Relaxed);
            }
        }))
        .map_err(db_error)?;
        let attached_members = crate::result_store::attach_scope_reader(
            &db,
            plan.project
                .parent()
                .ok_or_else(|| Error::invalid("项目路径无效"))?,
        )?;
        let (relation, column, id) = match &plan.meta.scope.target {
            ScopeTarget::Workset { collection_id } => (
                if attached_members {
                    "scope_collection_members"
                } else {
                    "scope_db.collection_members"
                },
                "collection_id",
                collection_id,
            ),
            ScopeTarget::QueryResult { result_id } => (
                if attached_members {
                    "scope_result_members"
                } else {
                    "scope_db.result_members"
                },
                "result_id",
                result_id,
            ),
            _ => return Err(Error::invalid("该范围不支持排名索引")),
        };
        progress.phase.store(1, Ordering::Release);
        let extra_columns = if plan.meta.version >= 2 {
            ",coalesce(s.direct_rank,9223372036854775807),coalesce(s.fused_rank,9223372036854775807)"
        } else {
            ""
        };
        let sql = format!(
            "INSERT INTO members SELECT i.ordinal,coalesce(s.rating,'z'),coalesce(s.main_rank,9223372036854775807),coalesce(s.rescue_rank,9223372036854775807),i.post_id{extra_columns} FROM {relation} m CROSS JOIN fixed_input.input_rows i INDEXED BY input_identity CROSS JOIN fixed_scores.scores s WHERE m.{column}=?1 AND i.source_id=m.source_id AND i.asset_id=unhex(m.asset_id) AND s.ordinal=i.ordinal"
        );
        let outcome = (|| {
            let copied = db.execute(&sql, [id]).map_err(build_error)? as u64;
            if copied != plan.meta.count {
                return Err(Error::new(
                    "ARTIFACT_INVALID",
                    "固定成员与原排名输入不一致，未发布排名索引",
                ));
            }
            if attached_members {
                db.execute_batch("DROP VIEW scope_collection_members; DROP VIEW scope_result_members; DETACH DATABASE scope_members;").map_err(db_error)?;
            }
            db.execute_batch("DETACH DATABASE scope_db; DETACH DATABASE fixed_input; DETACH DATABASE fixed_scores;").map_err(db_error)?;
            db.execute_batch("CREATE TABLE rank_positions(order_name TEXT NOT NULL,sequence INTEGER NOT NULL,ordinal INTEGER NOT NULL,PRIMARY KEY(order_name,sequence)) WITHOUT ROWID;").map_err(build_error)?;
            let mut orders = vec![
                RankingOrder::Main,
                RankingOrder::Rescue,
                RankingOrder::Input,
            ];
            if plan.meta.version >= 2 {
                orders.extend([RankingOrder::Direct, RankingOrder::Fused]);
            }
            for (index, order) in orders.into_iter().enumerate() {
                read_cancelled(&cancelled)?;
                progress.phase.store(2 + index as u8 * 2, Ordering::Release);
                let (name, column) = order_parts(order);
                let keys = if matches!(order, RankingOrder::Input) {
                    "rating,ordinal".into()
                } else {
                    format!("rating,{column},ordinal")
                };
                db.execute_batch(&format!("CREATE INDEX ordered_{name} ON members({keys});"))
                    .map_err(build_error)?;
                progress.phase.store(3 + index as u8 * 2, Ordering::Release);
                db.execute_batch(&format!(
                    "CREATE INDEX post_{name} ON members(post_id,{keys}) WHERE post_id IS NOT NULL;"
                ))
                .map_err(build_error)?;
                db.execute(&format!("INSERT INTO rank_positions SELECT ?1,sequence,ordinal FROM (SELECT row_number() OVER (ORDER BY {keys}) AS sequence,ordinal FROM members INDEXED BY ordered_{name}) WHERE (sequence-1)%{POSITION_STRIDE}=0"), [name]).map_err(build_error)?;
            }
            read_cancelled(&cancelled)?;
            progress.phase.store(12, Ordering::Release);
            db.execute(
                "INSERT INTO meta VALUES ('complete',?1)",
                [serde_json::to_string(&plan.meta).map_err(Error::io)?],
            )
            .map_err(db_error)?;
            db.execute(
                "INSERT INTO meta VALUES ('position_stride',?1)",
                [POSITION_STRIDE.to_string()],
            )
            .map_err(db_error)?;
            Ok(())
        })();
        read_cancelled(&cancelled)?;
        outcome
    }
    pub fn open(
        path: &Path,
        expected: &RankedIndexMeta,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(db_error)?;
        db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-4096;")
            .map_err(db_error)?;
        db.progress_handler(1000, Some(move || cancelled.load(Ordering::Acquire)))
            .map_err(db_error)?;
        let raw: String = db
            .query_row("SELECT value FROM meta WHERE key='complete'", [], |r| {
                r.get(0)
            })
            .map_err(|_| invalid())?;
        let meta: RankedIndexMeta = serde_json::from_str(&raw).map_err(|_| invalid())?;
        if &meta != expected {
            return Err(invalid());
        }
        let stride: String = db
            .query_row(
                "SELECT value FROM meta WHERE key='position_stride'",
                [],
                |r| r.get(0),
            )
            .map_err(|_| invalid())?;
        if stride != POSITION_STRIDE.to_string() {
            return Err(invalid());
        }
        db.prepare("SELECT ordinal FROM rank_positions WHERE order_name=?1 AND sequence=?2")
            .map_err(|_| invalid())?;
        let indices:i64=db.query_row("SELECT count(*) FROM sqlite_schema WHERE type='index' AND name IN ('ordered_main','ordered_rescue','ordered_input','post_main','post_rescue','post_input')",[],|r|r.get(0)).map_err(|_|invalid())?;
        if indices != 6 {
            return Err(invalid());
        }
        if meta.version >= 2 {
            let extra:i64=db.query_row("SELECT count(*) FROM sqlite_schema WHERE type='index' AND name IN ('ordered_direct','ordered_fused','post_direct','post_fused')",[],|r|r.get(0)).map_err(|_|invalid())?;
            if extra != 4 {
                return Err(invalid());
            }
        }
        Ok(Self { db, meta })
    }
    pub fn metadata(path: &Path) -> Result<RankedIndexMeta> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(db_error)?;
        let raw: String = db
            .query_row("SELECT value FROM meta WHERE key='complete'", [], |r| {
                r.get(0)
            })
            .map_err(|_| invalid())?;
        serde_json::from_str(&raw).map_err(|_| invalid())
    }
    /// Rebind only application-owned derived metadata, without copying members.
    pub fn rebind(path: &Path, old: &RankedIndexMeta, new: &RankedIndexMeta) -> Result<()> {
        let checked = Self::open(path, old, Arc::new(AtomicBool::new(false)))?;
        drop(checked);
        let db = Connection::open(path).map_err(db_error)?;
        db.execute(
            "UPDATE meta SET value=?1 WHERE key='complete'",
            [serde_json::to_string(new).map_err(Error::io)?],
        )
        .map_err(db_error)?;
        Ok(())
    }
    pub fn contains(&self, ordinal: u64) -> Result<bool> {
        self.db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM members WHERE ordinal=?1)",
                [ordinal as i64],
                |r| r.get(0),
            )
            .map_err(|_| invalid())
    }
    pub fn locate(
        &self,
        post: i64,
        order: RankingOrder,
        descending: bool,
    ) -> Result<Option<RankingPosition>> {
        let order = if self.meta.version < 2
            && matches!(order, RankingOrder::Direct | RankingOrder::Fused)
        {
            RankingOrder::Main
        } else {
            order
        };
        let (name, column) = order_parts(order);
        let direction = if descending { "DESC" } else { "ASC" };
        self.db.query_row(&format!("SELECT rating,{column},ordinal FROM members INDEXED BY post_{name} WHERE post_id=?1 ORDER BY rating {direction},{column} {direction},ordinal {direction} LIMIT 1"), [post], |r| Ok(RankingPosition { group:r.get(0)?, position:r.get(1)?, ordinal:unsigned(r,2)? })).optional().map_err(|_|invalid())
    }
    /// Exact original rank in one frozen Rating, restricted to fixed members.
    pub fn locate_rank(
        &self,
        rank: u64,
        rating: &str,
        order: RankingOrder,
        descending: bool,
    ) -> Result<Option<RankingPosition>> {
        if rank == 0 || rank >= i64::MAX as u64 || matches!(order, RankingOrder::Input) {
            return Ok(None);
        }
        let order = if self.meta.version < 2
            && matches!(order, RankingOrder::Direct | RankingOrder::Fused)
        {
            RankingOrder::Main
        } else {
            order
        };
        let (name, column) = order_parts(order);
        let direction = if descending { "DESC" } else { "ASC" };
        self.db.query_row(&format!("SELECT rating,{column},ordinal FROM members INDEXED BY ordered_{name} WHERE rating=?1 AND {column}=?2 ORDER BY ordinal {direction} LIMIT 1"), params![rating,rank as i64], |r| Ok(RankingPosition { group:r.get(0)?, position:r.get(1)?, ordinal:unsigned(r,2)? })).optional().map_err(|_|invalid())
    }
    /// One-based position in the displayed order. Seek the nearest bookmark,
    /// then read at most 127 following members, independent of the target rank.
    pub fn locate_position(
        &self,
        position: u64,
        order: RankingOrder,
        descending: bool,
    ) -> Result<Option<RankingPosition>> {
        if position == 0 || position > self.meta.count {
            return Ok(None);
        }
        let position = if descending {
            self.meta.count - position + 1
        } else {
            position
        };
        let order = if self.meta.version < 2
            && matches!(order, RankingOrder::Direct | RankingOrder::Fused)
        {
            RankingOrder::Main
        } else {
            order
        };
        let (name, column) = order_parts(order);
        let sequence = (position - 1) / POSITION_STRIDE * POSITION_STRIDE + 1;
        let start = self.db.query_row(&format!("SELECT m.rating,m.{column},m.ordinal FROM rank_positions p JOIN members m ON m.ordinal=p.ordinal WHERE p.order_name=?1 AND p.sequence=?2"), params![name, sequence as i64], |r| Ok(RankingPosition { group:r.get(0)?, position:r.get(1)?, ordinal:unsigned(r,2)? })).map_err(|_|invalid())?;
        let remaining = (position - sequence) as usize;
        if remaining == 0 {
            return Ok(Some(start));
        }
        let mut page = self.page(order, false, Some(&start), remaining)?;
        if page.len() != remaining {
            return Err(invalid());
        }
        Ok(page.pop())
    }
    pub fn page(
        &self,
        order: RankingOrder,
        descending: bool,
        after: Option<&RankingPosition>,
        limit: usize,
    ) -> Result<Vec<RankingPosition>> {
        let order = if self.meta.version < 2
            && matches!(order, RankingOrder::Direct | RankingOrder::Fused)
        {
            RankingOrder::Main
        } else {
            order
        };
        let (name, column) = order_parts(order);
        let direction = if descending { "DESC" } else { "ASC" };
        let op = if descending { "<" } else { ">" };
        let mut groups = vec!["e", "g", "q", "s", "z"];
        if descending {
            groups.reverse();
        }
        let mut rows = Vec::new();
        let limit = limit.clamp(1, 129);
        for group in groups {
            if after.is_some_and(|p| {
                if descending {
                    group > p.group.as_str()
                } else {
                    group < p.group.as_str()
                }
            }) {
                continue;
            }
            let matching_group = after.filter(|p| p.group == group);
            // Split equality and strict-rank seek so large tie/null-rank groups
            // also start at the ordinal, instead of filtering a long prefix.
            if let Some(p) = matching_group {
                let mut stmt = self.db.prepare(&format!("SELECT rating,{column},ordinal FROM members INDEXED BY ordered_{name} WHERE rating=?1 AND {column}=?2 AND ordinal{op}?3 ORDER BY ordinal {direction} LIMIT ?4")).map_err(|_|invalid())?;
                let values = stmt
                    .query_map(
                        params![
                            group,
                            p.position,
                            p.ordinal as i64,
                            (limit - rows.len()) as i64
                        ],
                        |r| {
                            Ok(RankingPosition {
                                group: r.get(0)?,
                                position: r.get(1)?,
                                ordinal: unsigned(r, 2)?,
                            })
                        },
                    )
                    .map_err(|_| invalid())?;
                rows.extend(
                    values
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(|_| invalid())?,
                );
                if rows.len() == limit {
                    break;
                }
            }
            let predicate = if matching_group.is_some() {
                format!(" AND {column}{op}?2")
            } else {
                " AND ?2 IS NULL".into()
            };
            let mut stmt = self.db.prepare(&format!("SELECT rating,{column},ordinal FROM members INDEXED BY ordered_{name} WHERE rating=?1{predicate} ORDER BY {column} {direction},ordinal {direction} LIMIT ?3")).map_err(|_|invalid())?;
            let values = stmt
                .query_map(
                    params![
                        group,
                        matching_group.map(|p| p.position),
                        (limit - rows.len()) as i64
                    ],
                    |r| {
                        Ok(RankingPosition {
                            group: r.get(0)?,
                            position: r.get(1)?,
                            ordinal: unsigned(r, 2)?,
                        })
                    },
                )
                .map_err(|_| invalid())?;
            rows.extend(
                values
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| invalid())?,
            );
            if rows.len() == limit {
                break;
            }
        }
        Ok(rows)
    }
}
