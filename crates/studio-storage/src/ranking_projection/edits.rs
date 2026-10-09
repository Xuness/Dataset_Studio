use super::*;
use crate::ranking_tables::RankingPosition;

fn intersect(a: &mut Option<Vec<String>>, b: &Option<Vec<String>>) {
    if let Some(b) = b {
        if let Some(a) = a {
            a.retain(|v| b.contains(v));
        } else {
            *a = Some(b.clone());
        }
    }
}
fn active(alias: &str, layer: &RankingEditLayer) -> String {
    format!(
        "{alias}.collection_id='{}' AND {alias}.valid_from<={} AND ({alias}.valid_until IS NULL OR {alias}.valid_until>{})",
        layer.collection_id, layer.revision, layer.revision
    )
}
pub(super) fn attach(
    db: &Connection,
    directory: &Path,
    projection: &RankingProjection,
) -> Result<()> {
    if projection.edits.is_empty() && projection.fences().next().is_none() {
        return Ok(());
    }
    db.execute(
        "ATTACH DATABASE ?1 AS workset_edits",
        [readonly_uri(&directory.join("project.sqlite"))?],
    )
    .map_err(db_error)?;
    db.execute_batch("PRAGMA workset_edits.cache_size=-4096;")
        .map_err(db_error)?;
    if projection.fences().next().is_some() {
        db.execute(
            "ATTACH DATABASE ?1 AS workset_members",
            [readonly_uri(&directory.join("members.sqlite"))?],
        )
        .map_err(db_error)?;
        db.execute_batch("PRAGMA workset_members.cache_size=-4096; CREATE TEMP VIEW fixed_project_members AS
          SELECT r.id AS result_id,m.source_id,m.asset_id FROM workset_edits.query_results r JOIN workset_edits.query_member_data m ON m.family_id=r.family_id WHERE r.storage_kind='legacy' AND r.status='ready' AND m.valid_from<=r.member_revision AND (m.valid_until IS NULL OR m.valid_until>r.member_revision)
          UNION ALL SELECT r.id,m.source_id,m.asset_id FROM workset_edits.query_results r JOIN workset_members.datasets d ON d.id=r.id AND d.state='sealed' JOIN workset_members.members m ON m.dataset_id=d.id WHERE r.storage_kind='sealed' AND r.status='ready';").map_err(db_error)?;
    }
    let mut edits = Vec::new();
    let mut extras = Vec::new();
    for (index, layer) in projection.edits.iter().enumerate() {
        let mut clauses = vec![active("e", layer)];
        for newer in &projection.edits[index + 1..] {
            clauses.push(format!("NOT EXISTS(SELECT 1 FROM workset_edits.collection_member_changes n WHERE {} AND n.source_id=e.source_id AND n.asset_id=e.asset_id)",active("n",newer)));
        }
        edits.push(format!(
            "SELECT e.* FROM workset_edits.collection_member_changes e WHERE {}",
            clauses.join(" AND ")
        ));
        let mut ratings = projection.ratings.clone();
        for later in &projection.edits[index + 1..] {
            intersect(&mut ratings, &later.before_ratings);
        }
        clauses.push("e.present=1".into());
        if let Some(ratings) = ratings {
            clauses.push(if ratings.is_empty() {
                "0".into()
            } else {
                format!(
                    "e.rating IN ({})",
                    ratings
                        .iter()
                        .map(|r| format!("'{r}'"))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            });
        }
        for result in projection.member_result.iter().chain(
            projection.edits[index + 1..]
                .iter()
                .filter_map(|l| l.before_result.as_ref()),
        ) {
            clauses.push(format!("EXISTS(SELECT 1 FROM fixed_project_members m WHERE m.result_id='{result}' AND m.source_id=e.source_id AND m.asset_id=e.asset_id)"));
        }
        extras.push(format!(
            "SELECT e.* FROM workset_edits.collection_member_changes e WHERE {}",
            clauses.join(" AND ")
        ));
    }
    if edits.is_empty() {
        edits.push("SELECT * FROM workset_edits.collection_member_changes WHERE 0".into());
        extras.push("SELECT * FROM workset_edits.collection_member_changes WHERE 0".into());
    }
    db.execute_batch(&format!(
        "CREATE TEMP VIEW effective_edits AS {}; CREATE TEMP VIEW extra_scores AS {};",
        edits.join(" UNION ALL "),
        extras.join(" UNION ALL ")
    ))
    .map_err(db_error)?;
    Ok(())
}

impl RankingProjection {
    pub fn fences(&self) -> impl Iterator<Item = &str> {
        self.member_result
            .iter()
            .chain(self.edits.iter().filter_map(|l| l.before_result.as_ref()))
            .map(String::as_str)
    }
}
pub(super) fn small_fences(db: &Connection, recipe: &RankingProjection) -> Result<Vec<String>> {
    let mut small = Vec::new();
    for id in recipe.fences() {
        let count:Option<u64>=db.query_row("SELECT count FROM workset_edits.query_results WHERE id=?1 AND status='ready' AND storage_kind IN ('sealed','legacy')",[id],|r|unsigned(r,0)).optional().map_err(db_error)?;
        let count =
            count.ok_or_else(|| Error::new("RESULT_NOT_READY", "排名浏览的固定成员范围不可用"))?;
        if count <= 4096 {
            small.push(id.into());
        }
    }
    Ok(small)
}

impl RankingProjectionReader {
    pub fn has_edits(&self) -> bool {
        !self.projection.edits.is_empty()
    }
    pub(super) fn base_ratings(&self) -> Option<Vec<String>> {
        let mut ratings = self.projection.ratings.clone();
        for layer in &self.projection.edits {
            intersect(&mut ratings, &layer.before_ratings);
        }
        ratings
    }
    pub(super) fn extra_contains(
        &self,
        predicate: &str,
        values: impl IntoIterator<Item = Value>,
    ) -> Result<bool> {
        if !self.has_edits() {
            return Ok(false);
        }
        self.db
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM extra_scores WHERE {predicate})"),
                rusqlite::params_from_iter(values),
                |r| r.get(0),
            )
            .map_err(db_error)
    }
    pub fn edited_input(&self, ordinal: u64) -> Result<Option<RankingInput>> {
        self.edited_value(ordinal, "input_json")
    }
    pub fn edited_scores(&self, ordinal: u64) -> Result<Option<RankingScores>> {
        self.edited_value(ordinal, "scores_json")
    }
    pub fn edited_input_for_key(&self, key: &AssetKey) -> Result<Option<RankingInput>> {
        if !self.has_edits() {
            return Ok(None);
        }
        // Re-adding a removed member can restore its frozen metadata even if
        // its source is offline. A removal row intentionally has no image data.
        for layer in self.projection.edits.iter().rev() {
            let raw:Option<String>=self.db.query_row("SELECT input_json FROM workset_edits.collection_member_changes WHERE collection_id=?1 AND source_id=?2 AND asset_id=?3 AND valid_from<=?4 AND present=1 AND input_json IS NOT NULL ORDER BY valid_from DESC LIMIT 1",rusqlite::params![layer.collection_id,key.source_id,key.asset_id,layer.revision as i64],|r|r.get(0)).optional().map_err(db_error)?;
            if let Some(raw) = raw {
                return serde_json::from_str(&raw).map(Some).map_err(Error::io);
            }
        }
        Ok(None)
    }
    pub fn appended_keys(&self, source: &str, after: Option<&str>) -> Result<Vec<AssetKey>> {
        if !self.has_edits() {
            return Ok(Vec::new());
        }
        self.db.prepare("SELECT source_id,asset_id FROM extra_scores WHERE source_id=?1 AND asset_id>?2 AND ordinal>=4611686018427387904 ORDER BY asset_id LIMIT 512").map_err(db_error)?
            .query_map(rusqlite::params![source,after.unwrap_or("")],|r|Ok(AssetKey { source_id:r.get(0)?,asset_id:r.get(1)? })).map_err(db_error)?
            .collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)
    }
    fn edited_value<T: serde::de::DeserializeOwned>(
        &self,
        ordinal: u64,
        column: &str,
    ) -> Result<Option<T>> {
        if !self.has_edits() {
            return Ok(None);
        }
        let raw: Option<String> = self
            .db
            .query_row(
                &format!("SELECT {column} FROM extra_scores WHERE ordinal=?1"),
                [ordinal as i64],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        raw.map(|v| serde_json::from_str(&v).map_err(Error::io))
            .transpose()
    }
    pub(super) fn extra_page(
        &self,
        order: RankingOrder,
        descending: bool,
        after: Option<&RankingPosition>,
        limit: usize,
    ) -> Result<Vec<RankingPosition>> {
        if !self.has_edits() {
            return Ok(Vec::new());
        }
        let column = self.column(order);
        let direction = if descending { "DESC" } else { "ASC" };
        let op = if descending { "<" } else { ">" };
        let mut values = Vec::new();
        let predicate = if let Some(after) = after {
            values.extend([
                Value::Integer(i64::from(after.position == i64::MAX)),
                Value::Text(after.group.clone()),
                Value::Integer(after.position),
                Value::Integer(after.ordinal as i64),
            ]);
            format!(
                "(({column}=9223372036854775807)>?1 OR (({column}=9223372036854775807)=?1 AND (coalesce(rating,'z'),{column},ordinal){op}(?2,?3,?4)))"
            )
        } else {
            "1=1".into()
        };
        values.push(Value::Integer(limit.clamp(1, 129) as i64));
        self.read_positions(&format!("SELECT rating,{column},ordinal FROM extra_scores WHERE {predicate} ORDER BY ({column}=9223372036854775807),coalesce(rating,'z') {direction},{column} {direction},ordinal {direction} LIMIT ?{}",values.len()),values)
    }
    pub(super) fn extra_anchor(
        &self,
        base: Option<RankingPosition>,
        order: RankingOrder,
        descending: bool,
        predicate: &str,
        values: Vec<Value>,
    ) -> Result<Option<RankingPosition>> {
        if !self.has_edits() {
            return Ok(base);
        }
        let column = self.column(order);
        let direction = if descending { "DESC" } else { "ASC" };
        let predicate = predicate.replace("RANK_COLUMN", column);
        let mut rows = self.read_positions(&format!("SELECT rating,{column},ordinal FROM extra_scores WHERE {predicate} ORDER BY ({column}=9223372036854775807),coalesce(rating,'z') {direction},{column} {direction},ordinal {direction} LIMIT 1"),values)?;
        rows.extend(base);
        rows.sort_by(|a, b| a.compare_ranked(b, descending));
        Ok(rows.into_iter().next())
    }

    /// Both halves use counted key ranges. Only the delta half is new; the
    /// immutable score indexes still serve the unchanged part of the workset.
    fn position_count(
        &self,
        order: RankingOrder,
        group: &Option<String>,
        unranked: bool,
        ranks: Option<(i64, i64)>,
        ordinals: Option<(i64, i64)>,
    ) -> Result<u64> {
        let column = self.column(order);
        let mut total = 0;
        for extra in [false, true] {
            let (predicate, mut values) = if extra {
                ("1=1".into(), Vec::new())
            } else {
                self.base_predicate()?
            };
            let label = group.as_deref().unwrap_or("z");
            values.push(Value::Text(label.into()));
            let rating = if label == "z" {
                format!("(rating=?{} OR rating IS NULL)", values.len())
            } else {
                format!("rating=?{}", values.len())
            };
            let mut clauses = vec![
                predicate,
                rating,
                format!(
                    "{column}{}9223372036854775807",
                    if unranked { "=" } else { "<" }
                ),
            ];
            if let Some((low, high)) = ranks {
                values.push(Value::Integer(low));
                values.push(Value::Integer(high));
                clauses.push(format!(
                    "{column} BETWEEN ?{} AND ?{}",
                    values.len() - 1,
                    values.len()
                ));
            }
            if let Some((low, high)) = ordinals {
                values.push(Value::Integer(low));
                values.push(Value::Integer(high));
                clauses.push(format!(
                    "ordinal BETWEEN ?{} AND ?{}",
                    values.len() - 1,
                    values.len()
                ));
            }
            let table = if extra {
                "extra_scores".into()
            } else {
                format!("scores{}", self.index_clause(order))
            };
            let predicate = clauses.join(" AND ");
            let count = self
                .db
                .query_row(
                    &format!("SELECT count(*) FROM {table} WHERE {predicate}"),
                    rusqlite::params_from_iter(values.iter()),
                    |r| unsigned(r, 0),
                )
                .map_err(db_error)?;
            // Count the immutable index range and subtract only edited rows;
            // do not perform a project lookup for every unchanged score.
            let removed = if !extra && self.has_edits() {
                self.db.query_row(&format!("SELECT count(*) FROM effective_edits e WHERE EXISTS(SELECT 1 FROM main.scores WHERE main.scores.ordinal=e.ordinal AND ({predicate}))"),rusqlite::params_from_iter(values.iter()),|r|unsigned(r,0)).map_err(db_error)?
            } else {
                0
            };
            total += count
                .checked_sub(removed)
                .ok_or_else(|| Error::invalid("排名位置计数无效"))?;
        }
        Ok(total)
    }
    pub(super) fn locate_edited_position(
        &self,
        position: u64,
        order: RankingOrder,
        descending: bool,
    ) -> Result<Option<RankingPosition>> {
        if position == 0 || position > self.projection.count {
            return Ok(None);
        }
        let last: u64 = self
            .db
            .query_row(
                "SELECT coalesce(max(ordinal),0) FROM extra_scores",
                [],
                |r| unsigned(r, 0),
            )
            .map_err(db_error)?;
        let maximum = self.input_count.max(last) as i64;
        let mut remaining = position;
        for unranked in [false, true] {
            if unranked && order == RankingOrder::Input {
                break;
            }
            for group in self.groups(descending)? {
                let count = self.position_count(order, &group, unranked, None, None)?;
                if remaining > count {
                    remaining -= count;
                    continue;
                }
                if descending {
                    remaining = count - remaining + 1;
                }
                let mut low = 0;
                let mut high = if order == RankingOrder::Input {
                    maximum
                } else {
                    self.input_count as i64
                };
                let rank = if unranked {
                    i64::MAX
                } else {
                    while low < high {
                        let middle = low + (high - low) / 2;
                        let left = self.position_count(
                            order,
                            &group,
                            unranked,
                            Some((low, middle)),
                            None,
                        )?;
                        if left >= remaining {
                            high = middle;
                        } else {
                            remaining -= left;
                            low = middle + 1;
                        }
                    }
                    low
                };
                low = 0;
                high = maximum;
                while low < high {
                    let middle = low + (high - low) / 2;
                    let left = self.position_count(
                        order,
                        &group,
                        unranked,
                        Some((rank, rank)),
                        Some((low, middle)),
                    )?;
                    if left >= remaining {
                        high = middle;
                    } else {
                        remaining -= left;
                        low = middle + 1;
                    }
                }
                return Ok(Some(RankingPosition {
                    group: group.unwrap_or_else(|| "z".into()),
                    position: rank,
                    ordinal: low as u64,
                }));
            }
        }
        Err(Error::invalid("固定排名成员数量与位置不一致"))
    }

    /// The virtual member table still yields bounded source/key batches. Rows
    /// already in the original input retain that ordinal. The virtual reader
    /// merges new textual identities separately so mixed key formats still seek.
    pub(crate) fn member_relation(&self) -> Result<(String, Vec<Value>)> {
        let (predicate, values) = self.predicate()?;
        let matched =
            format!("EXISTS(SELECT 1 FROM main.scores WHERE ordinal=i.ordinal AND ({predicate}))");
        if !self.has_edits() {
            return Ok((
                format!(
                    "SELECT i.source_id,i.asset_id,i.ordinal,i.post_id,{matched} AS matched FROM fixed_input.input_rows i INDEXED BY input_identity"
                ),
                values,
            ));
        }
        Ok((
            format!(
                "SELECT i.source_id,i.asset_id,i.ordinal,i.post_id,({matched} OR EXISTS(SELECT 1 FROM extra_scores e WHERE e.ordinal=i.ordinal)) AS matched FROM fixed_input.input_rows i INDEXED BY input_identity"
            ),
            values,
        ))
    }
    pub(crate) fn small_member_relation(&self) -> Result<(String, Vec<Value>)> {
        let (predicate, values) = self.predicate()?;
        let mut sql = format!(
            "SELECT i.source_id,i.asset_id,i.ordinal,i.post_id FROM (SELECT ordinal FROM scores WHERE {predicate} LIMIT 4097) m JOIN fixed_input.input_rows i ON i.ordinal=m.ordinal"
        );
        if self.has_edits() {
            sql.push_str(" UNION ALL SELECT source_id,asset_id,ordinal,post_id FROM extra_scores");
        }
        Ok((format!("SELECT * FROM ({sql}) LIMIT 4097"), values))
    }
}
