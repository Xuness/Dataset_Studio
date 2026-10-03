//! Direct typed projection for unambiguous current observations. Complex rows
//! retain the SQL reference path, including duplicate posts and unusual values.
use super::*;
use serde_json::Value;
use studio_application::ReadCancellation;

pub(crate) struct Captured {
    pub rows: Vec<RankingInput>,
    pub fallback: Vec<(u64, String, u32)>,
}

pub(crate) fn timestamp(value: Option<&str>) -> std::result::Result<Option<i64>, ()> {
    value
        .map(|s| {
            // Restrict the shortcut to ordinary Gregorian RFC3339 timestamps.
            // DuckDB handles every other representation on the reference path.
            let date = chrono::DateTime::parse_from_rfc3339(s)
                .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f%:z"))
                .map_err(|_| ())?;
            if date.timestamp_subsec_nanos() >= 1_000_000_000 {
                return Err(());
            }
            Ok(date.timestamp_micros())
        })
        .transpose()
}

pub(crate) fn dimensions(value: &str, prefix: &str) -> std::result::Result<Option<(u32, u32)>, ()> {
    let value: Value = serde_json::from_str(value).map_err(|_| ())?;
    let number = |name: &str| -> std::result::Result<Option<u32>, ()> {
        match value.get(format!("{prefix}{name}")) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Ok(n
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| *v > 0)),
            // SQL TRY_CAST accepts other forms, with its own rounding rules.
            // Keep those exact semantics rather than approximate them here.
            _ => Err(()),
        }
    };
    Ok(number("stored_width")?.zip(number("stored_height")?))
}

pub(crate) struct Candidate {
    pub input: RankingInput,
    origin: Option<String>,
    details: Option<String>,
    pub dates: [Option<String>; 3],
}

/// Outliers keep the bounded SQL path. Inspect borrowed text before allocating
/// native rows so one large field cannot inflate every page in the pipeline.
pub(crate) fn row_fits(row: &rusqlite::Row<'_>) -> Result<bool> {
    let mut bytes = 0usize;
    for column in [1, 3, 4, 5, 6, 8, 9, 10, 11, 12, 18, 19] {
        match row.get_ref(column).map_err(sql_error)? {
            rusqlite::types::ValueRef::Null => (),
            rusqlite::types::ValueRef::Text(value) => bytes = bytes.saturating_add(value.len()),
            _ => return Ok(false),
        }
    }
    Ok(bytes <= 64 << 10)
}

pub(crate) fn read_candidate(
    row: &rusqlite::Row<'_>,
    source: &Source,
    ordinal: u64,
    asset_id: String,
    parameters: &RankingParameters,
) -> Result<Candidate> {
    let observation: Option<String> = row.get(6).map_err(sql_error)?;
    let known = row.get::<_, Option<i64>>(26).map_err(sql_error)?.is_some();
    let tags: Option<String> = row.get(19).map_err(sql_error)?;
    let mut artists: Vec<String> = row
        .get::<_, Option<String>>(18)
        .map_err(sql_error)?
        .unwrap_or_default()
        .split(' ')
        .filter(|s| {
            !s.is_empty()
                && !matches!(
                    *s,
                    "artist_request" | "unknown_artist" | "anonymous_artist" | "banned_artist"
                )
        })
        .map(String::from)
        .collect();
    artists.sort();
    artists.dedup();
    let damage_classes = tags
        .as_deref()
        .map(|s| {
            s.split(' ').fold(0, |v, t| {
                v | match t {
                    "jpeg_artifacts" => 1,
                    "scan_artifacts" => 2,
                    _ => 0,
                }
            })
        })
        .unwrap_or(0);
    let issues: Option<String> = row.get(25).map_err(sql_error)?;
    let issues = issues.map(|s| {
        if s.chars().take(4097).count() > 4096 {
            "[\"source_issues_truncated\"]".into()
        } else {
            s
        }
    });
    Ok(Candidate {
        input: RankingInput {
            ordinal,
            source_id: source.id.clone(),
            asset_id,
            record_id: if known {
                row.get(3).map_err(sql_error)?
            } else {
                None
            },
            observation_id: observation,
            post_id: row.get(7).map_err(sql_error)?,
            rating: row.get(8).map_err(sql_error)?,
            time_quality: row
                .get::<_, Option<String>>(12)
                .map_err(sql_error)?
                .unwrap_or_else(|| "unknown".into()),
            source_priority: row.get(13).map_err(sql_error)?,
            fav_count: row.get(14).map_err(sql_error)?,
            up_score: row.get(15).map_err(sql_error)?,
            down_score: row.get(16).map_err(sql_error)?,
            score: row.get(17).map_err(sql_error)?,
            artists,
            parent_id: row.get(20).map_err(sql_error)?,
            dimension_basis: if !known || parameters.minimum_stored_side.is_some() {
                "not_recorded"
            } else {
                "not_requested"
            }
            .into(),
            stored_extension: row
                .get::<_, Option<String>>(1)
                .map_err(sql_error)?
                .unwrap_or_default(),
            stored_bytes: u64::try_from(
                row.get::<_, Option<i64>>(2)
                    .map_err(sql_error)?
                    .unwrap_or_default(),
            )
            .map_err(error)?,
            is_banned: row.get(21).map_err(sql_error)?,
            is_deleted: row.get(22).map_err(sql_error)?,
            is_pending: row.get(23).map_err(sql_error)?,
            is_flagged: row.get(24).map_err(sql_error)?,
            damage_classes,
            tags_known: tags.is_some(),
            record_count: u32::from(known),
            basis_ids: if known { vec![0] } else { vec![] },
            source_issues: issues,
            tags: if parameters.v2.is_some() { tags } else { None },
            ..Default::default()
        },
        origin: if known {
            row.get(4).map_err(sql_error)?
        } else {
            None
        },
        details: row.get(5).map_err(sql_error)?,
        dates: [
            row.get(9).map_err(sql_error)?,
            row.get(10).map_err(sql_error)?,
            row.get(11).map_err(sql_error)?,
        ],
    })
}

/// `None` means this scope is outside the shortcut's complete equivalence domain.
pub(crate) fn capture(
    source: &Source,
    expected: &QuerySourceVersion,
    parameters: &RankingParameters,
    members: &[(u64, String, u32)],
    cancelled: ReadCancellation,
) -> Result<Option<Captured>> {
    if source.kind != "danbooru"
        || members.is_empty()
        || members.iter().any(|r| r.2 != 0)
        || members.windows(2).any(|r| r[0].0 >= r[1].0)
    {
        return Ok(None);
    }
    if members
        .iter()
        .any(|r| r.1.len() != 64 || hex::decode(&r.1).is_err())
    {
        return Err(Error::invalid("排名图片身份无效"));
    }
    let started = Instant::now();
    let view = Snapshot::open_bulk(
        source,
        Some(&expected.catalog_revision),
        cancelled,
        Some(Instant::now() + Duration::from_secs(60)),
    )?;
    if view.version(source) != *expected {
        return Err(Error::new("SOURCE_CHANGED", "排名投影的固定来源版本不一致"));
    }
    let input = serde_json::to_string(members).map_err(Error::io)?;
    let mut statement = view.db.prepare(
        "WITH scope AS (SELECT json_extract(value,'$[0]') ordinal,json_extract(value,'$[1]') sha256 FROM json_each(?1))
         SELECT s.ordinal,o.stored_ext,o.length,a.asset_id,a.observation_id,a.details_json,
         p.observation_id,p.post_id,p.rating,p.created_at,p.observed_at,p.updated_at,
         p.time_quality,p.source_priority,p.fav_count,p.up_score,p.down_score,p.score,
         p.tag_string_artist,p.tag_string,p.parent_id,p.is_banned,p.is_deleted,p.is_pending,p.is_flagged,p.issues_json,p.row_id
         FROM scope s LEFT JOIN visible_objects o ON o.sha256=s.sha256
         LEFT JOIN visible_assets a ON a.sha256=s.sha256
         LEFT JOIN current_posts c ON c.asset_id=a.asset_id
         LEFT JOIN visible_observations p ON p.row_id=c.row_id"
    ).map_err(sql_error)?;
    let mut query = statement.query([input]).map_err(sql_error)?;
    let positions: HashMap<_, _> = members.iter().enumerate().map(|(i, r)| (r.0, i)).collect();
    let mut candidates: Vec<Option<Candidate>> = (0..members.len()).map(|_| None).collect();
    let mut repeated = vec![false; members.len()];
    let mut captured_bytes = 0usize;
    while let Some(row) = query.next().map_err(sql_error)? {
        let ordinal = u64::try_from(row.get::<_, i64>(0).map_err(sql_error)?).map_err(error)?;
        let i = *positions
            .get(&ordinal)
            .ok_or_else(|| error("排名投影返回范围外序号"))?;
        if repeated[i] {
            continue;
        }
        if candidates[i].is_some() {
            repeated[i] = true;
            candidates[i] = None;
            continue;
        }
        if !row_fits(row)? {
            return Ok(None);
        }
        candidates[i] = Some(read_candidate(
            row,
            source,
            ordinal,
            members[i].1.clone(),
            parameters,
        )?);
        let c = candidates[i].as_ref().expect("captured row");
        captured_bytes = captured_bytes
            .saturating_add(1024)
            .saturating_add(c.input.tags.as_ref().map_or(0, String::len))
            .saturating_add(c.input.artists.iter().map(String::len).sum::<usize>())
            .saturating_add(c.details.as_ref().map_or(0, String::len))
            .saturating_add(c.dates.iter().flatten().map(String::len).sum::<usize>());
        if captured_bytes > 128 << 20 {
            return Ok(None);
        }
    }
    drop(query);
    drop(statement);
    let capture_time = started.elapsed();
    let mut raw = view
        .db
        .prepare_cached(
            "SELECT raw_bytes,raw_zlib,raw_sha256 FROM raw_metadata WHERE observation_id=?1",
        )
        .map_err(sql_error)?;
    let mut result = Captured {
        rows: Vec::with_capacity(members.len()),
        fallback: Vec::new(),
    };
    for (i, candidate) in candidates.into_iter().enumerate() {
        view.check()?;
        let Some(mut c) = candidate else {
            result.fallback.push(members[i].clone());
            continue;
        };
        let converted: std::result::Result<Vec<_>, _> =
            c.dates.iter().map(|s| timestamp(s.as_deref())).collect();
        let Ok(dates) = converted else {
            result.fallback.push(members[i].clone());
            continue;
        };
        (
            c.input.created_at_us,
            c.input.observed_at_us,
            c.input.updated_at_us,
        ) = (dates[0], dates[1], dates[2]);
        if parameters.minimum_stored_side.is_some() && c.input.record_id.is_some() {
            let direct = c
                .details
                .as_deref()
                .map(|s| dimensions(s, ""))
                .unwrap_or(Ok(None));
            let Ok(mut size) = direct else {
                result.fallback.push(members[i].clone());
                continue;
            };
            if size.is_some() {
                c.input.dimension_basis = "asset_storage_details".into();
            }
            if size.is_none()
                && let Some(origin) = c.origin
            {
                let raw: Option<(i64, Vec<u8>, String)> = raw
                    .query_row([origin], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .optional()
                    .map_err(sql_error)?;
                if let Some((length, compressed, hash)) = raw.filter(|r| r.0 >= 0) {
                    let body = super::raw::decode(&compressed, length as u64, &hash, 16 << 20)?;
                    let projected: super::dimensions::Dimensions =
                        serde_json::from_str(&body).map_err(error)?;
                    let projected = serde_json::to_string(&projected).map_err(error)?;
                    let Ok(raw_size) = dimensions(&projected, "raw_") else {
                        result.fallback.push(members[i].clone());
                        continue;
                    };
                    size = raw_size;
                    if size.is_some() {
                        c.input.dimension_basis = "asset_origin_raw_metadata".into();
                    }
                }
            }
            if let Some((w, h)) = size {
                c.input.stored_width = Some(w);
                c.input.stored_height = Some(h);
            }
        }
        result.rows.push(c.input);
    }
    tracing::debug!(
        rows = result.rows.len(),
        fallback = result.fallback.len(),
        capture_ms = capture_time.as_millis(),
        finish_ms = (started.elapsed() - capture_time).as_millis(),
        "ranking singleton projection"
    );
    crate::canonical::Catalog::open_at(source, Some(&expected.catalog_revision))?
        .verify_unchanged(source)?;
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_unambiguous_dimensions_take_the_direct_path() {
        assert_eq!(
            dimensions(r#"{"stored_width":640,"stored_height":480}"#, ""),
            Ok(Some((640, 480)))
        );
        assert_eq!(
            dimensions(r#"{"stored_width":0,"stored_height":-1}"#, ""),
            Ok(None)
        );
        assert_eq!(dimensions("{}", ""), Ok(None));
        assert!(dimensions(r#"{"stored_width":"640","stored_height":480}"#, "").is_err());
        assert!(dimensions(r#"{"stored_width":640.5,"stored_height":480}"#, "").is_err());
    }
    #[test]
    fn exact_timestamps_and_fallback_are_explicit() {
        assert_eq!(timestamp(None), Ok(None));
        assert_eq!(
            timestamp(Some("1970-01-01 00:00:00.000001+00:00")),
            Ok(Some(1))
        );
        assert_eq!(timestamp(Some("1969-12-31T23:59:59.999999Z")), Ok(Some(-1)));
        assert_eq!(
            timestamp(Some("1970-01-01T08:00:00.000001+08:00")),
            Ok(Some(1))
        );
        assert!(timestamp(Some("not-a-date")).is_err());
        assert!(timestamp(Some("1970-01-01")).is_err());
    }
}
