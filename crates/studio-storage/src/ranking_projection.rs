//! Read fixed ranking membership and order directly from immutable materials.
//! The small recipe is durable project data; indexes remain shared by every
//! workset and Rating view referring to the same ranking artifact.
use crate::{db_error, ranking_tables, unsigned};
use rusqlite::{Connection, OpenFlags, OptionalExtension, types::Value};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use studio_domain::*;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RankingProjection {
    pub version: u32,
    pub artifact_id: String,
    pub input_file: String,
    pub score_file: String,
    pub source_ids: Vec<String>,
    pub filter: RankingFilter,
    /// Additional frozen Rating conditions; an empty list means no members.
    pub ratings: Option<Vec<String>>,
    pub count: u64,
    pub workset_id: String,
}

impl RankingProjection {
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.artifact_id)?;
        validate_id(&self.workset_id)?;
        self.filter.validate()?;
        if self.version != 1
            || self.source_ids.is_empty()
            || self.source_ids.len() > 64
            || self.count > i64::MAX as u64
            || self.ratings.as_ref().is_some_and(|values| {
                values.len() > 4
                    || values
                        .iter()
                        .any(|v| !matches!(v.as_str(), "g" | "s" | "q" | "e"))
            })
        {
            return Err(Error::invalid("固定排名成员描述无效"));
        }
        for source in &self.source_ids {
            validate_id(source)?;
        }
        Ok(())
    }
    pub fn files(&self, directory: &Path) -> Result<(PathBuf, PathBuf)> {
        self.validate()?;
        let root = directory
            .join("artifacts")
            .canonicalize()
            .map_err(Error::io)?;
        let checked = |relative: &str| -> Result<PathBuf> {
            let path = Path::new(relative);
            if path.is_absolute()
                || path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err(Error::invalid("排名成员文件路径无效"));
            }
            let full = directory.join(path).canonicalize().map_err(Error::io)?;
            if !full.starts_with(&root) || !full.is_file() {
                return Err(Error::invalid("排名成员文件不在本项目成果目录"));
            }
            Ok(full)
        };
        Ok((checked(&self.input_file)?, checked(&self.score_file)?))
    }
    pub fn fingerprint(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        Ok(hex::encode(Sha256::digest(
            serde_json::to_vec(&(
                self.version,
                &self.artifact_id,
                &self.input_file,
                &self.score_file,
                &self.source_ids,
                &self.filter,
                &self.ratings,
            ))
            .map_err(Error::io)?,
        )))
    }
    pub fn restrict_ratings(&mut self, values: &[String]) {
        let mut kept = values.to_vec();
        kept.sort();
        kept.dedup();
        if let Some(old) = &self.ratings {
            kept.retain(|v| old.contains(v));
        }
        self.ratings = Some(kept);
    }
}

pub(crate) fn readonly_uri(path: &Path) -> Result<String> {
    let mut uri = url::Url::from_file_path(path).map_err(|_| Error::invalid("排名文件路径无效"))?;
    uri.set_query(Some("mode=ro"));
    Ok(uri.to_string())
}

pub(crate) fn open_read(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(db_error)?;
    db.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(db_error)?;
    db.execute_batch("PRAGMA cache_size=-4096; PRAGMA temp_store=FILE;")
        .map_err(db_error)?;
    db.pragma_update(
        None,
        "mmap_size",
        if cfg!(target_pointer_width = "64") {
            128i64 << 30
        } else {
            1i64 << 30
        },
    )
    .map_err(db_error)?;
    Ok(db)
}

pub struct RankingProjectionReader {
    pub(crate) db: Connection,
    pub projection: RankingProjection,
    v2: bool,
    input_count: u64,
    null_rating: bool,
    summary: RankingSummary,
}

impl RankingProjectionReader {
    pub fn open(
        directory: &Path,
        projection: &RankingProjection,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        studio_application::read_cancelled(&cancelled)?;
        let (input, scores) = projection.files(directory)?;
        let checked = ranking_tables::RankingResultTable::open(&scores)?;
        let v2 = checked.is_v2()?;
        let summary = checked.meta::<RankingSummary>("summary")?;
        let input_count = checked
            .known_count(&RankingFilter::default())?
            .ok_or_else(|| Error::invalid("排名材料缺少总数"))?;
        drop(checked);
        let db = open_read(&scores)?;
        db.execute("ATTACH DATABASE ?1 AS fixed_input", [readonly_uri(&input)?])
            .map_err(db_error)?;
        db.execute_batch("PRAGMA fixed_input.cache_size=-4096; PRAGMA query_only=ON;")
            .map_err(db_error)?;
        db.progress_handler(1000, Some(move || cancelled.load(Ordering::Acquire)))
            .map_err(db_error)?;
        let null_rating = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM scores INDEXED BY scores_rating WHERE rating IS NULL)",
                [],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        Ok(Self {
            db,
            projection: projection.clone(),
            v2,
            input_count,
            null_rating,
            summary,
        })
    }
    fn effective_filter(&self) -> RankingFilter {
        let mut filter = self.projection.filter.clone();
        if !self.v2 && matches!(filter.order, RankingOrder::Direct | RankingOrder::Fused) {
            filter.order = RankingOrder::Main;
        }
        filter
    }
    pub(crate) fn predicate(&self) -> Result<(String, Vec<Value>)> {
        let (mut sql, mut values) = ranking_tables::filter_sql(&self.effective_filter())?;
        if let Some(ratings) = &self.projection.ratings {
            if ratings.is_empty() {
                sql.push_str(" AND 0");
            } else {
                let params = ratings
                    .iter()
                    .map(|v| {
                        values.push(Value::Text(v.clone()));
                        format!("?{}", values.len())
                    })
                    .collect::<Vec<_>>();
                sql.push_str(&format!(" AND rating IN ({})", params.join(",")));
            }
        }
        Ok((sql, values))
    }
    pub fn count(&self) -> Result<u64> {
        if self.projection.ratings.as_ref().is_some_and(Vec::is_empty) {
            return Ok(0);
        }
        if let Some(ratings) = &self.projection.ratings {
            let mut count = 0;
            for rating in ratings {
                if self
                    .projection
                    .filter
                    .rating
                    .as_ref()
                    .is_some_and(|r| r != rating)
                {
                    continue;
                }
                let mut part = self.projection.filter.clone();
                part.rating = Some(rating.clone());
                count += self.count_filter(&part)?;
            }
            return Ok(count);
        }
        self.count_filter(&self.projection.filter)
    }
    fn count_filter(&self, filter: &RankingFilter) -> Result<u64> {
        let mut effective = filter.clone();
        if !self.v2 && matches!(effective.order, RankingOrder::Direct | RankingOrder::Fused) {
            effective.order = RankingOrder::Main;
        }
        if self.summary.eligibility_counts.values().sum::<u64>() == self.summary.input_count
            && let Some(count) = ranking_tables::known_summary_count(&self.summary, &effective)?
        {
            return Ok(count);
        }
        if effective == RankingFilter::default()
            || (effective.rating.is_none()
                && effective.route.is_none()
                && effective.eligibility.is_none()
                && !effective.missing_only
                && !effective.selected_only
                && effective.top.is_none())
        {
            return Ok(self.input_count);
        }
        let (predicate, values) = ranking_tables::filter_sql(&effective)?;
        self.db
            .query_row(
                &format!("SELECT count(*) FROM scores WHERE {predicate}"),
                rusqlite::params_from_iter(values),
                |r| unsigned(r, 0),
            )
            .map_err(db_error)
    }
    pub fn contains(&self, ordinal: u64) -> Result<bool> {
        let (predicate, mut values) = self.predicate()?;
        values.push(Value::Integer(ordinal as i64));
        self.db
            .query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM scores WHERE ordinal=?{} AND ({predicate}))",
                    values.len()
                ),
                rusqlite::params_from_iter(values),
                |r| r.get(0),
            )
            .map_err(db_error)
    }
    pub fn contains_key(&self, key: &AssetKey) -> Result<bool> {
        if !self.projection.source_ids.contains(&key.source_id) {
            return Ok(false);
        }
        let (predicate, mut values) = self.predicate()?;
        values.push(Value::Text(key.source_id.clone()));
        let source = values.len();
        values.push(Value::Blob(hex::decode(&key.asset_id).map_err(Error::io)?));
        let asset = values.len();
        self.db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM scores WHERE ordinal IN (SELECT ordinal FROM fixed_input.input_rows WHERE source_id=?{source} AND asset_id=?{asset}) AND ({predicate}))"),rusqlite::params_from_iter(values),|r|r.get(0)).map_err(db_error)
    }
    fn order(&self, order: RankingOrder) -> RankingOrder {
        if !self.v2 && matches!(order, RankingOrder::Direct | RankingOrder::Fused) {
            RankingOrder::Main
        } else {
            order
        }
    }
    fn column(&self, order: RankingOrder) -> &'static str {
        match self.order(order) {
            RankingOrder::Main => "coalesce(main_rank,9223372036854775807)",
            RankingOrder::Rescue => "coalesce(rescue_rank,9223372036854775807)",
            RankingOrder::Input => "ordinal",
            RankingOrder::Direct => "coalesce(direct_rank,9223372036854775807)",
            RankingOrder::Fused => "coalesce(fused_rank,9223372036854775807)",
        }
    }
    fn index(&self, order: RankingOrder) -> &'static str {
        let order = if self.projection.count <= 4096 {
            // A tiny sparse range is cheaper to read through its membership
            // index and sort in bounded memory than to scan a different order
            // through millions of excluded rows on every page.
            match self.projection.filter.order {
                RankingOrder::Input => RankingOrder::Main,
                order => order,
            }
        } else {
            order
        };
        match self.order(order) {
            RankingOrder::Main if self.projection.filter.route.is_some() => "scores_route_main",
            RankingOrder::Main if self.projection.filter.eligibility.is_some() => {
                "scores_eligibility_main"
            }
            RankingOrder::Main => "scores_main",
            RankingOrder::Rescue => "scores_rescue",
            RankingOrder::Input => "scores_rating",
            RankingOrder::Direct => "scores_direct",
            RankingOrder::Fused => "scores_fused",
        }
    }
    fn groups(&self, descending: bool) -> Result<Vec<Option<String>>> {
        let mut groups = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let row: Option<String> = if let Some(after)=&after {
                self.db.query_row("SELECT rating FROM scores INDEXED BY scores_rating WHERE rating>?1 ORDER BY rating LIMIT 1", [after], |r|r.get(0))
            } else {
                self.db.query_row("SELECT rating FROM scores INDEXED BY scores_rating WHERE rating IS NOT NULL ORDER BY rating LIMIT 1", [], |r|r.get(0))
            }.optional().map_err(db_error)?;
            let Some(rating) = row else { break };
            if rating.len() > 128 || groups.len() >= 64 {
                return Err(Error::invalid("排名分级数量无效"));
            }
            after = Some(rating.clone());
            if self
                .projection
                .filter
                .rating
                .as_ref()
                .is_none_or(|r| r == &rating)
                && self
                    .projection
                    .ratings
                    .as_ref()
                    .is_none_or(|values| values.contains(&rating))
            {
                groups.push(Some(rating));
            }
        }
        if self.projection.filter.rating.is_none()
            && self.projection.ratings.is_none()
            && self.null_rating
            && !groups.iter().any(|v| v.as_deref() == Some("z"))
        {
            groups.push(None);
        }
        groups.sort_by(|a, b| a.as_deref().unwrap_or("z").cmp(b.as_deref().unwrap_or("z")));
        if descending {
            groups.reverse();
        }
        Ok(groups)
    }
    fn group_sql(&self, group: &Option<String>, values: &mut Vec<Value>) -> String {
        if let Some(group) = group {
            values.push(Value::Text(group.clone()));
            if group == "z" && self.null_rating {
                format!("(rating=?{} OR rating IS NULL)", values.len())
            } else {
                format!("rating=?{}", values.len())
            }
        } else {
            "rating IS NULL".into()
        }
    }
    fn read_positions(
        &self,
        sql: &str,
        values: Vec<Value>,
    ) -> Result<Vec<ranking_tables::RankingPosition>> {
        self.db
            .prepare(sql)
            .map_err(db_error)?
            .query_map(rusqlite::params_from_iter(values), |r| {
                Ok(ranking_tables::RankingPosition {
                    group: r.get::<_, Option<String>>(0)?.unwrap_or_else(|| "z".into()),
                    position: r.get(1)?,
                    ordinal: unsigned(r, 2)?,
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn page(
        &self,
        order: RankingOrder,
        descending: bool,
        after: Option<&ranking_tables::RankingPosition>,
        limit: usize,
    ) -> Result<Vec<ranking_tables::RankingPosition>> {
        let column = self.column(order);
        let index = self.index(order);
        let direction = if descending { "DESC" } else { "ASC" };
        let op = if descending { "<" } else { ">" };
        let limit = limit.clamp(1, 129);
        let mut rows = Vec::new();
        for group in self.groups(descending)? {
            let label = group.as_deref().unwrap_or("z");
            if after.is_some_and(|p| {
                if descending {
                    label > p.group.as_str()
                } else {
                    label < p.group.as_str()
                }
            }) {
                continue;
            }
            let matching = after.filter(|p| p.group == label);
            // Seek within a tie before moving to the next rank. Ineligible rows
            // can share the sentinel rank by the millions.
            for same_rank in [true, false] {
                if same_rank && matching.is_none() {
                    continue;
                }
                let (predicate, mut values) = self.predicate()?;
                let mut clauses = vec![predicate, self.group_sql(&group, &mut values)];
                if let Some(p) = matching {
                    values.push(Value::Integer(p.position));
                    let rank = values.len();
                    if same_rank {
                        values.push(Value::Integer(p.ordinal as i64));
                        clauses.push(format!("{column}=?{rank} AND ordinal{op}?{}", values.len()));
                    } else {
                        clauses.push(format!("{column}{op}?{rank}"));
                    }
                }
                values.push(Value::Integer((limit - rows.len()) as i64));
                let ordering = if same_rank {
                    format!("ordinal {direction}")
                } else {
                    format!("{column} {direction},ordinal {direction}")
                };
                rows.extend(self.read_positions(&format!("SELECT rating,{column},ordinal FROM scores INDEXED BY {index} WHERE {} ORDER BY {ordering} LIMIT ?{}",clauses.join(" AND "),values.len()),values)?);
                if rows.len() == limit {
                    return Ok(rows);
                }
            }
        }
        Ok(rows)
    }
    pub fn locate_rank(
        &self,
        rank: u64,
        rating: &str,
        order: RankingOrder,
        descending: bool,
    ) -> Result<Option<ranking_tables::RankingPosition>> {
        let (predicate, mut values) = self.predicate()?;
        values.push(Value::Text(rating.into()));
        let rating_arg = values.len();
        values.push(Value::Integer(rank as i64));
        let rank_arg = values.len();
        let direction = if descending { "DESC" } else { "ASC" };
        let column = self.column(order);
        Ok(self.read_positions(&format!("SELECT rating,{column},ordinal FROM scores INDEXED BY {} WHERE ({predicate}) AND rating=?{rating_arg} AND {column}=?{rank_arg} ORDER BY ordinal {direction} LIMIT 1",self.index(order)),values)?.pop())
    }
    pub fn locate(
        &self,
        post: i64,
        order: RankingOrder,
        descending: bool,
    ) -> Result<Option<ranking_tables::RankingPosition>> {
        let (predicate, mut values) = self.predicate()?;
        values.push(Value::Integer(post));
        let arg = values.len();
        let column = self.column(order);
        let direction = if descending { "DESC" } else { "ASC" };
        Ok(self.read_positions(&format!("SELECT rating,{column},ordinal FROM scores WHERE ordinal IN (SELECT ordinal FROM fixed_input.input_rows WHERE post_id=?{arg}) AND ({predicate}) ORDER BY coalesce(rating,'z') {direction},{column} {direction},ordinal {direction} LIMIT 1"),values)?.pop())
    }
    /// Counted seeks avoid a large OFFSET and remain correct for sparse saved
    /// predicates. Rank and ordinal ranges are each at most the input size.
    pub fn locate_position(
        &self,
        position: u64,
        order: RankingOrder,
        descending: bool,
    ) -> Result<Option<ranking_tables::RankingPosition>> {
        if position == 0 || position > self.projection.count {
            return Ok(None);
        }
        let mut remaining = if descending {
            self.projection.count - position + 1
        } else {
            position
        };
        let column = self.column(order);
        let index = self.index(order);
        let groups = self.groups(false)?;
        let single_group = groups.len() == 1;
        for group in groups {
            let (predicate, mut values) = self.predicate()?;
            let group_sql = self.group_sql(&group, &mut values);
            let base = format!("({predicate}) AND {group_sql}");
            let count = if single_group {
                self.projection.count
            } else {
                self.db
                    .query_row(
                        &format!("SELECT count(*) FROM scores INDEXED BY {index} WHERE {base}"),
                        rusqlite::params_from_iter(values.clone()),
                        |r| unsigned(r, 0),
                    )
                    .map_err(db_error)?
            };
            if remaining > count {
                remaining -= count;
                continue;
            }
            let mut low = 0i64;
            let mut high = self.input_count as i64;
            let ranked=self.db.query_row(&format!("SELECT count(*) FROM scores INDEXED BY {index} WHERE {base} AND {column}<9223372036854775807"),rusqlite::params_from_iter(values.clone()),|r|unsigned(r,0)).map_err(db_error)?;
            let rank = if remaining > ranked {
                remaining -= ranked;
                i64::MAX
            } else {
                while low < high {
                    let middle = low + (high - low) / 2;
                    let mut args = values.clone();
                    args.push(Value::Integer(low));
                    let lower = args.len();
                    args.push(Value::Integer(middle));
                    let n=self.db.query_row(&format!("SELECT count(*) FROM scores INDEXED BY {index} WHERE {base} AND {column}>=?{lower} AND {column}<=?{}",args.len()),rusqlite::params_from_iter(args),|r|unsigned(r,0)).map_err(db_error)?;
                    if n >= remaining {
                        high = middle;
                    } else {
                        low = middle + 1;
                        remaining -= n;
                    }
                }
                low
            };
            values.push(Value::Integer(rank));
            let base = format!("{base} AND {column}=?{}", values.len());
            if remaining == 1 {
                return Ok(self.read_positions(&format!("SELECT rating,{column},ordinal FROM scores INDEXED BY {index} WHERE {base} ORDER BY ordinal LIMIT 1"),values)?.pop());
            }
            low = 0;
            high = self.input_count.saturating_sub(1) as i64;
            while low < high {
                let middle = low + (high - low) / 2;
                let mut args = values.clone();
                args.push(Value::Integer(low));
                let lower = args.len();
                args.push(Value::Integer(middle));
                let n=self.db.query_row(&format!("SELECT count(*) FROM scores INDEXED BY {index} WHERE {base} AND ordinal>=?{lower} AND ordinal<=?{}",args.len()),rusqlite::params_from_iter(args),|r|unsigned(r,0)).map_err(db_error)?;
                if n >= remaining {
                    high = middle;
                } else {
                    low = middle + 1;
                    remaining -= n;
                }
            }
            values.push(Value::Integer(low));
            return Ok(self
                .read_positions(
                    &format!(
                        "SELECT rating,{column},ordinal FROM scores WHERE {base} AND ordinal=?{}",
                        values.len()
                    ),
                    values,
                )?
                .pop());
        }
        Ok(None)
    }
}
