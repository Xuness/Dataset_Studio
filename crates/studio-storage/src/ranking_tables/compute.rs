//! Phase-specific readers avoid hydrating provenance and large strings when
//! the current calculation does not consume them. Complete rows remain intact.
use super::*;

fn evidence(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<RankingDuplicateEvidence>> {
    row.get::<_, Option<String>>(index)?
        .map(|value| {
            serde_json::from_str(&value).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })
        .transpose()
}

impl RankingResultTable {
    pub fn validation_page(&self, after: Option<u64>) -> Result<Vec<RankingScores>> {
        let extra = if self.is_v2()? { "v2_json" } else { "NULL" };
        let sql = format!(
            "SELECT ordinal,rating,eligibility,g,c,a,v,t,main_score,rescue_score,main_rank,rescue_rank,selected_route,duplicate_of,{extra} FROM scores WHERE ordinal>?1 ORDER BY ordinal LIMIT 512"
        );
        self.db
            .prepare_cached(&sql)
            .map_err(db_error)?
            .query_map([after.map(|n| n as i64).unwrap_or(-1)], |r| {
                let v2 = r
                    .get::<_, Option<String>>(14)?
                    .map(|v| {
                        serde_json::from_str(&v).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                14,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        })
                    })
                    .transpose()?;
                Ok(RankingScores {
                    ordinal: unsigned(r, 0)?,
                    rating: r.get(1)?,
                    eligibility: decode_enum(r, 2)?,
                    g: r.get(3)?,
                    c: r.get(4)?,
                    a: r.get(5)?,
                    v: r.get(6)?,
                    t: r.get(7)?,
                    main_score: r.get(8)?,
                    rescue_score: r.get(9)?,
                    main_rank: optional_unsigned(r, 10)?,
                    rescue_rank: optional_unsigned(r, 11)?,
                    selected_route: decode_enum(r, 12)?,
                    duplicate_of: optional_unsigned(r, 13)?,
                    v2,
                    ..Default::default()
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
}

impl RankingInputTable {
    /// Exactly the fields used by eligibility() and flags(). Empty strings in
    /// optional identities and the artist sentinel represent presence only.
    pub fn classification_page(&self, after: Option<u64>) -> Result<Vec<RankingInput>> {
        let evidence_column = if self.has_evidence()? {
            "i.evidence_json"
        } else {
            "NULL"
        };
        let sql = format!(
            "SELECT i.ordinal,i.rating,i.record_id IS NOT NULL,i.observation_id IS NOT NULL,i.created_at_us,i.observed_at_us,i.time_quality,i.fav_count,i.up_score,i.down_score,i.score,json_array_length(i.artists)>0,i.stored_width,i.stored_height,i.dimension_basis,i.tags_known,i.rating_conflict,i.source_issues,i.is_banned,d.duplicate_of,{evidence_column} FROM input_rows i LEFT JOIN duplicate_members d USING(ordinal) WHERE i.ordinal>?1 ORDER BY i.ordinal LIMIT 512"
        );
        self.db
            .prepare_cached(&sql)
            .map_err(db_error)?
            .query_map([after.map(|n| n as i64).unwrap_or(-1)], |r| {
                Ok(RankingInput {
                    ordinal: unsigned(r, 0)?,
                    rating: r.get(1)?,
                    record_id: r.get::<_, bool>(2)?.then(String::new),
                    observation_id: r.get::<_, bool>(3)?.then(String::new),
                    created_at_us: r.get(4)?,
                    observed_at_us: r.get(5)?,
                    time_quality: r.get(6)?,
                    fav_count: r.get(7)?,
                    up_score: r.get(8)?,
                    down_score: r.get(9)?,
                    score: r.get(10)?,
                    artists: if r.get::<_, bool>(11)? {
                        vec![String::new()]
                    } else {
                        vec![]
                    },
                    stored_width: r.get(12)?,
                    stored_height: r.get(13)?,
                    dimension_basis: r.get(14)?,
                    tags_known: r.get(15)?,
                    rating_conflict: r.get(16)?,
                    source_issues: r.get(17)?,
                    is_banned: r.get(18)?,
                    duplicate_of: optional_unsigned(r, 19)?,
                    evidence: evidence(r, 20)?,
                    ..Default::default()
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }

    pub fn eligible_page(
        &self,
        after: Option<u64>,
        rating: &str,
        p: &RankingParameters,
    ) -> Result<Vec<RankingInput>> {
        if !matches!(rating, "g" | "s" | "q" | "e") || !p.ratings.iter().any(|r| r == rating) {
            return Ok(vec![]);
        }
        let sql = format!(
            "SELECT {} FROM input_rows i LEFT JOIN duplicate_members d USING(ordinal) WHERE i.ordinal>?1 AND i.rating=?2 AND i.record_id IS NOT NULL AND i.observation_id IS NOT NULL AND d.duplicate_of IS NULL AND (?3 IS NULL OR (i.stored_width>=?3 AND i.stored_height>=?3)) AND (?4=0 OR coalesce(i.is_banned,0)=0) ORDER BY i.ordinal LIMIT 512",
            input_columns(self.is_v2()?, self.has_evidence()?)
        );
        self.db
            .prepare_cached(&sql)
            .map_err(db_error)?
            .query_map(
                params![
                    after.map(|n| n as i64).unwrap_or(-1),
                    rating,
                    p.minimum_stored_side,
                    p.exclude_banned
                ],
                |r| read_input(r, 0),
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }

    /// Identity/rating coverage, eligibility and v2 formula validation only
    /// consume these fields. The whole-file digest still protects every byte.
    pub fn validation_page(
        &self,
        after: Option<u64>,
        p: &RankingParameters,
    ) -> Result<Vec<RankingInput>> {
        let v2 = self.is_v2()?;
        let tags = if v2 {
            "CASE WHEN i.record_id IS NOT NULL AND i.observation_id IS NOT NULL AND d.duplicate_of IS NULL AND (?2 IS NULL OR (i.stored_width>=?2 AND i.stored_height>=?2)) AND (?3=0 OR coalesce(i.is_banned,0)=0) THEN i.tags END"
        } else {
            "NULL"
        };
        let sql = format!(
            "SELECT i.ordinal,i.rating,i.record_id IS NOT NULL,i.observation_id IS NOT NULL,i.created_at_us,i.stored_width,i.stored_height,i.is_banned,d.duplicate_of,{tags} FROM input_rows i LEFT JOIN duplicate_members d USING(ordinal) WHERE i.ordinal>?1 ORDER BY i.ordinal LIMIT 512"
        );
        let values = if v2 {
            vec![
                SqlValue::Integer(after.map(|n| n as i64).unwrap_or(-1)),
                p.minimum_stored_side
                    .map(|n| SqlValue::Integer(i64::from(n)))
                    .unwrap_or(SqlValue::Null),
                SqlValue::Integer(i64::from(p.exclude_banned)),
            ]
        } else {
            vec![SqlValue::Integer(after.map(|n| n as i64).unwrap_or(-1))]
        };
        self.db
            .prepare_cached(&sql)
            .map_err(db_error)?
            .query_map(rusqlite::params_from_iter(values), |r| {
                Ok(RankingInput {
                    ordinal: unsigned(r, 0)?,
                    rating: r.get(1)?,
                    record_id: r.get::<_, bool>(2)?.then(String::new),
                    observation_id: r.get::<_, bool>(3)?.then(String::new),
                    created_at_us: r.get(4)?,
                    stored_width: r.get(5)?,
                    stored_height: r.get(6)?,
                    is_banned: r.get(7)?,
                    duplicate_of: optional_unsigned(r, 8)?,
                    tags: r.get(9)?,
                    ..Default::default()
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
}
