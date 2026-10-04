use super::*;
use std::{cmp::Ordering, collections::BTreeSet};
#[cfg(test)]
mod tests;

pub struct PostIdScan {
    pub ordinals: Vec<u64>,
    pub next_ordinal: u64,
}

pub struct RankingScan {
    pub rows: Vec<(RankingScores, bool)>,
    pub more: bool,
}

impl RankingResultTable {
    pub fn rating(&self, ordinal: u64) -> Result<Option<String>> {
        self.db
            .prepare_cached("SELECT rating FROM scores WHERE ordinal=?1")
            .map_err(db_error)?
            .query_row([ordinal as i64], |r| r.get(0))
            .map_err(db_error)
    }
    pub fn rating_ordinals(&self, rating: &str, after: Option<u64>) -> Result<Vec<u64>> {
        let mut s = self.db.prepare_cached("SELECT ordinal FROM scores INDEXED BY scores_rating WHERE rating=?1 AND ordinal>?2 ORDER BY ordinal LIMIT 512").map_err(db_error)?;
        s.query_map(
            params![rating, after.map(|n| n as i64).unwrap_or(-1)],
            |r| unsigned(r, 0),
        )
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }
}

impl RankingInputTable {
    pub fn ordinal_for_key(&self, key: &AssetKey) -> Result<Option<u64>> {
        self.db
            .prepare_cached("SELECT ordinal FROM input_rows WHERE source_id=?1 AND asset_id=?2")
            .map_err(db_error)?
            .query_row(params![key.source_id, bytes(&key.asset_id)?], |r| {
                unsigned(r, 0)
            })
            .optional()
            .map_err(db_error)
    }

    pub fn post_for_key(&self, key: &AssetKey) -> Result<Option<(u64, Option<i64>)>> {
        self.db
            .prepare_cached(
                "SELECT ordinal,post_id FROM input_rows WHERE source_id=?1 AND asset_id=?2",
            )
            .map_err(db_error)?
            .query_row(params![key.source_id, bytes(&key.asset_id)?], |r| {
                Ok((unsigned(r, 0)?, r.get(1)?))
            })
            .optional()
            .map_err(db_error)
    }

    pub fn key(&self, ordinal: u64) -> Result<AssetKey> {
        self.db
            .prepare_cached("SELECT source_id,asset_id FROM input_rows WHERE ordinal=?1")
            .map_err(db_error)?
            .query_row([ordinal as i64], |r| {
                Ok(AssetKey {
                    source_id: r.get(0)?,
                    asset_id: hex::encode(r.get::<_, Vec<u8>>(1)?),
                })
            })
            .map_err(db_error)
    }

    /// Old immutable inputs need no rewrite: each continuation scans at most
    /// max_rows ordinal positions and returns at most 512 matches.
    pub fn post_id_scan(
        &self,
        post_id: i64,
        after: u64,
        total: u64,
        max_rows: u64,
    ) -> Result<PostIdScan> {
        if post_id <= 0 || after > total || total > i64::MAX as u64 {
            return Err(Error::invalid("无效的 Danbooru ID 定位范围"));
        }
        let columns = self
            .db
            .prepare("PRAGMA index_info(input_post)")
            .map_err(db_error)?
            .query_map([], |r| r.get::<_, Option<String>>(2))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let indexed = columns == vec![Some("post_id".into()), Some("ordinal".into())];
        let end = if indexed {
            total
        } else {
            after.saturating_add(max_rows.clamp(1, 262_144)).min(total)
        };
        let sql = if indexed {
            "SELECT ordinal FROM input_rows INDEXED BY input_post WHERE post_id=?1 AND ordinal>=?2 AND ordinal<?3 ORDER BY ordinal LIMIT 513"
        } else {
            "SELECT ordinal FROM input_rows WHERE ordinal>=?2 AND ordinal<?3 AND post_id=?1 ORDER BY ordinal LIMIT 513"
        };
        let mut ordinals = self
            .db
            .prepare(sql)
            .map_err(db_error)?
            .query_map(params![post_id, after as i64, end as i64], |r| {
                unsigned(r, 0)
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let next_ordinal = if ordinals.len() > 512 {
            ordinals.truncate(512);
            ordinals[511] + 1
        } else {
            end
        };
        Ok(PostIdScan {
            ordinals,
            next_ordinal,
        })
    }
}

impl RankingPosition {
    /// Workset display keeps missing ranks after every ranked member, even
    /// when Rating, rank and ordinal are traversed in descending order.
    pub fn compare_ranked(&self, other: &Self, descending: bool) -> Ordering {
        (self.position == i64::MAX)
            .cmp(&(other.position == i64::MAX))
            .then_with(|| self.compare(other, descending))
    }

    pub fn for_scores(scores: &RankingScores, order: RankingOrder) -> Self {
        let value = match order {
            RankingOrder::Main => scores.main_rank,
            RankingOrder::Rescue => scores.rescue_rank,
            RankingOrder::Input => Some(scores.ordinal),
            RankingOrder::Direct => scores.v2.map(|v| v.direct_rank).or(scores.main_rank),
            RankingOrder::Fused => scores.v2.map(|v| v.fused_rank).or(scores.main_rank),
        };
        Self {
            group: scores.rating.clone().unwrap_or_else(|| "z".into()),
            position: value
                .and_then(|v| i64::try_from(v).ok())
                .unwrap_or(i64::MAX),
            ordinal: scores.ordinal,
        }
    }

    pub fn compare(&self, other: &Self, descending: bool) -> Ordering {
        let result = (&self.group, self.position, self.ordinal).cmp(&(
            &other.group,
            other.position,
            other.ordinal,
        ));
        if descending { result.reverse() } else { result }
    }
}

impl RankingResultTable {
    pub fn matches_filter(&self, ordinal: u64, filter: &RankingFilter) -> Result<bool> {
        let (predicate, mut values) = filter_sql(&self.compatible_filter(filter)?)?;
        values.push(SqlValue::Integer(ordinal as i64));
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

    // Distinct prefixes are found with index seeks, without scanning every score.
    fn browse_groups(&self, rating: Option<&str>) -> Result<Vec<String>> {
        if let Some(rating) = rating {
            return Ok(vec![rating.into()]);
        }
        let mut groups = BTreeSet::new();
        let mut after: Option<String> = None;
        loop {
            let value: Option<String> = if let Some(after) = &after {
                self.db.query_row("SELECT rating FROM scores INDEXED BY scores_rating WHERE rating>?1 ORDER BY rating LIMIT 1", [after], |r| r.get(0))
            } else {
                self.db.query_row("SELECT rating FROM scores INDEXED BY scores_rating WHERE rating IS NOT NULL ORDER BY rating LIMIT 1", [], |r| r.get(0))
            }.optional().map_err(db_error)?;
            let Some(value) = value else {
                break;
            };
            if value.len() > 128 || groups.len() >= 64 {
                return Err(Error::new(
                    "RANKING_FORMAT_UNSUPPORTED",
                    "排名分组数量或长度超出范围",
                ));
            }
            groups.insert(value.clone());
            after = Some(value);
        }
        if self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM scores INDEXED BY scores_rating WHERE rating IS NULL)",
                [],
                |r| r.get::<_, bool>(0),
            )
            .map_err(db_error)?
        {
            groups.insert("z".into());
        }
        Ok(groups.into_iter().collect())
    }

    /// Membership and view order are separate. Conditions unsupported by the
    /// chosen ordering index are evaluated as a returned flag, so a sparse
    /// workset cannot turn one bounded page into an unbounded filtered scan.
    pub fn browse_scan(
        &self,
        membership: &RankingFilter,
        order: RankingOrder,
        descending: bool,
        after: Option<&RankingPosition>,
        limit: usize,
    ) -> Result<RankingScan> {
        let effective_membership = self.compatible_filter(membership)?;
        let membership = &effective_membership;
        membership.validate()?;
        let order = if !self.is_v2()? && matches!(order, RankingOrder::Direct | RankingOrder::Fused)
        {
            RankingOrder::Main
        } else {
            order
        };
        let columns = self.score_columns()?;
        let (predicate, values) = filter_sql(membership)?;
        let position = match order {
            RankingOrder::Main => "coalesce(main_rank,9223372036854775807)",
            RankingOrder::Rescue => "coalesce(rescue_rank,9223372036854775807)",
            RankingOrder::Input => "ordinal",
            RankingOrder::Direct => "coalesce(direct_rank,9223372036854775807)",
            RankingOrder::Fused => "coalesce(fused_rank,9223372036854775807)",
        };
        let limit = limit.clamp(1, 512);
        let mut groups = self.browse_groups(membership.rating.as_deref())?;
        if descending {
            groups.reverse();
        }
        let direction = if descending { "DESC" } else { "ASC" };
        let comparison = if descending { "<" } else { ">" };
        let mut output = Vec::new();
        let mut more = false;
        for (index, group) in groups.iter().enumerate() {
            if after.is_some_and(|p| {
                if descending {
                    group > &p.group
                } else {
                    group < &p.group
                }
            }) {
                continue;
            }
            let remaining = limit + 1 - output.len();
            let mut bucket = Vec::new();
            let mut variants = vec![Some(group.as_str())];
            if group == "z" && membership.rating.is_none() {
                variants.push(None);
            }
            for variant in variants {
                let mut params = values.clone();
                let mut conditions = Vec::new();
                if let Some(rating) = variant {
                    params.push(SqlValue::Text(rating.into()));
                    conditions.push(format!("rating=?{}", params.len()));
                } else {
                    conditions.push("rating IS NULL".into());
                }
                if order == RankingOrder::Main {
                    if let Some(route) = &membership.route {
                        params.push(SqlValue::Text(enum_text(route)?));
                        conditions.push(format!("selected_route=?{}", params.len()));
                    } else if let Some(eligibility) = &membership.eligibility {
                        params.push(SqlValue::Text(enum_text(eligibility)?));
                        conditions.push(format!("eligibility=?{}", params.len()));
                    }
                }
                let top_order = match membership.order {
                    RankingOrder::Input => RankingOrder::Main,
                    order => order,
                };
                if let Some(top) = membership.top.filter(|_| order == top_order) {
                    params.push(SqlValue::Integer(top as i64));
                    conditions.push(format!("{position}<=?{}", params.len()));
                }
                if let Some(after) = after.filter(|p| &p.group == group) {
                    params.push(SqlValue::Integer(after.position));
                    let p = params.len();
                    params.push(SqlValue::Integer(after.ordinal as i64));
                    conditions.push(format!("{position}{comparison}=?{p} AND ({position}{comparison}?{p} OR ordinal{comparison}?{})", params.len()));
                }
                params.push(SqlValue::Integer(remaining as i64));
                let sql = format!(
                    "SELECT {columns},CASE WHEN ({predicate}) THEN 1 ELSE 0 END FROM scores INDEXED BY {} WHERE {} ORDER BY {position} {direction},ordinal {direction} LIMIT ?{}",
                    score_index(membership, order),
                    conditions.join(" AND "),
                    params.len()
                );
                let mut statement = self.db.prepare(&sql).map_err(db_error)?;
                let rows = statement
                    .query_map(rusqlite::params_from_iter(params), |r| {
                        Ok((read_scores(r, 0)?, r.get::<_, bool>(22)?))
                    })
                    .map_err(db_error)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(db_error)?;
                bucket.extend(rows);
            }
            bucket.sort_by(|(a, _), (b, _)| {
                RankingPosition::for_scores(a, order)
                    .compare(&RankingPosition::for_scores(b, order), descending)
            });
            bucket.truncate(remaining);
            output.extend(bucket);
            if output.len() > limit {
                more = true;
                break;
            }
            if output.len() == limit && index + 1 < groups.len() {
                more = true;
                break;
            }
        }
        if output.len() > limit {
            output.truncate(limit);
        }
        Ok(RankingScan { rows: output, more })
    }
}
