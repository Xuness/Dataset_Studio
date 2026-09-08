use studio_domain::*;

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

pub(super) fn asset_predicate(column: &str, assets: &[String]) -> Result<String> {
    if assets.is_empty() || assets.len() > 512 {
        return Err(Error::invalid("查询输入批次需要 1–512 个对象"));
    }
    Ok(format!(
        "{column} IN ({})",
        assets
            .iter()
            .map(|a| quote(a))
            .collect::<Vec<_>>()
            .join(",")
    ))
}
fn predicate(column: &str, condition: &QueryCondition) -> Result<String> {
    use QueryOperator::*;
    if condition.operator == IsMissing {
        return Ok(format!("{column} IS NULL"));
    }
    if condition.operator == IsPresent {
        return Ok(format!("{column} IS NOT NULL"));
    }
    if let Some(QueryValue::TextList(values)) = &condition.value {
        let list = values
            .iter()
            .map(|v| quote(v))
            .collect::<Vec<_>>()
            .join(",");
        return match condition.operator {
            In => Ok(format!("{column} IN ({list})")),
            HasAllTags => Ok(format!("list_has_all(string_split({column},' '),[{list}])")),
            HasAnyTags => Ok(format!("list_has_any(string_split({column},' '),[{list}])")),
            HasNoTags => Ok(format!(
                "NOT list_has_any(string_split({column},' '),[{list}])"
            )),
            _ => Err(Error::invalid("该操作不接受集合值")),
        };
    }
    let value = match &condition.value {
        Some(QueryValue::Text(value)) => quote(value),
        Some(QueryValue::Integer(value)) => value
            .parse::<i64>()
            .map_err(|_| Error::invalid("整数条件无效"))?
            .to_string(),
        Some(QueryValue::Boolean(value)) => value.to_string(),
        Some(QueryValue::TextList(_)) => unreachable!("list handled above"),
        None => return Err(Error::invalid("条件缺少值")),
    };
    if condition.operator == HasTag {
        return Ok(format!("list_contains(string_split({column},' '),{value})"));
    }
    let operator = match condition.operator {
        Eq => "=",
        Ne => "!=",
        Gte => ">=",
        Lte => "<=",
        _ => return Err(Error::invalid("条件操作无效")),
    };
    Ok(format!("{column} {operator} {value}"))
}

pub(super) fn storage_predicates(spec: &QuerySpec) -> Result<String> {
    let mut predicates = Vec::new();
    for condition in &spec.conditions {
        let column = match condition.field.as_str() {
            "asset.id" => "sha256",
            "stored.bytes" => "length",
            "stored.extension" => "stored_ext",
            _ => continue,
        };
        predicates.push(predicate(column, condition)?);
    }
    Ok(if predicates.is_empty() {
        "1=1".into()
    } else {
        predicates.join(" AND ")
    })
}

pub(super) fn metadata_sql(spec: &QuerySpec) -> Result<String> {
    metadata_sql_for_assets(spec, None)
}

pub(super) fn metadata_sql_for_assets(
    spec: &QuerySpec,
    assets: Option<&[String]>,
) -> Result<String> {
    metadata_sql_filtered(spec, assets, false, false)
}
pub(super) fn metadata_sql_for_changed(spec: &QuerySpec) -> Result<String> {
    metadata_sql_filtered(spec, None, true, false)
}
pub(super) fn metadata_sql_for_rating_candidates(spec: &QuerySpec) -> Result<String> {
    if spec
        .conditions
        .iter()
        .all(|condition| condition.field == "rating")
    {
        // Intersection of supported rating clauses is already encoded by the
        // imported, watermark-fenced bases; no source columns remain to inspect.
        return Ok("SELECT lower(hex(sha256)) FROM studio_rating_candidates".into());
    }
    metadata_sql_filtered(spec, None, false, true)
}
pub(super) fn prepare_rating_observations(spec: &QuerySpec) -> Result<String> {
    let mut columns = std::collections::BTreeSet::from(["row_id"]);
    for condition in &spec.conditions {
        let column = match condition.field.as_str() {
            "asset.id" | "stored.bytes" | "stored.extension" => continue,
            "post.id" => "post_id",
            "source.width" => "image_width",
            "source.height" => "image_height",
            "source.extension" => "file_ext",
            "score" => "score",
            "fav_count" => "fav_count",
            "rating" => "rating",
            "tags" => "tag_string",
            "is_deleted" => "is_deleted",
            _ => return Err(Error::new("QUERY_UNSUPPORTED", "字段没有查询实现")),
        };
        columns.insert(column);
    }
    // A separate statement makes the candidate boundary explicit: tag predicates
    // cannot be pushed below the candidate semi-join by the native optimizer.
    Ok(format!(
        "CREATE TEMP TABLE studio_rating_observations AS SELECT {} FROM observations WHERE row_id IN (SELECT row_id FROM studio_rating_candidates)",
        columns.into_iter().collect::<Vec<_>>().join(",")
    ))
}
fn metadata_sql_filtered(
    spec: &QuerySpec,
    assets: Option<&[String]>,
    changed: bool,
    rating_candidates: bool,
) -> Result<String> {
    let identity = if rating_candidates {
        "lower(hex(a.sha256))"
    } else {
        "a.sha256"
    };
    let mut predicates = vec!["a.sha256 IS NOT NULL".to_owned()];
    if changed {
        predicates.push("a.sha256 IN (SELECT sha256 FROM studio_changed)".into());
    }
    if let Some(assets) = assets {
        predicates.push(asset_predicate("a.sha256", assets)?);
    }
    for condition in &spec.conditions {
        let column = match condition.field.as_str() {
            "asset.id" => identity,
            "stored.bytes" | "stored.extension" => continue,
            "post.id" => "o.post_id",
            "source.width" => "o.image_width",
            "source.height" => "o.image_height",
            "source.extension" => "o.file_ext",
            "score" => "o.score",
            "fav_count" => "o.fav_count",
            "rating" => "o.rating",
            "tags" => "o.tag_string",
            "is_deleted" => "o.is_deleted",
            _ => return Err(Error::new("QUERY_UNSUPPORTED", "字段没有查询实现")),
        };
        predicates.push(predicate(column, condition)?);
    }
    let predicate = predicates.join(" AND ");
    if rating_candidates {
        // The base already establishes the current-post to asset association.
        // Rejoining the full source identity tables would discard that benefit.
        return Ok(format!(
            "SELECT {identity} FROM studio_rating_candidates a JOIN studio_rating_observations o ON o.row_id=a.row_id WHERE {predicate}"
        ));
    }
    let observations = if rating_candidates {
        "studio_rating_observations"
    } else {
        "observations"
    };
    // No DISTINCT or global sort here: the project-owned SQLite sink handles both,
    // so the C API can stream identities instead of materializing the whole result.
    Ok(match spec.observation_rule {
        ObservationRule::CurrentPost => format!(
            "SELECT a.sha256 FROM current_posts cp JOIN assets a ON a.asset_id=cp.asset_id JOIN {observations} o ON o.row_id=cp.row_id WHERE {predicate}"
        ),
        ObservationRule::AnyObservation => format!(
            "SELECT a.sha256 FROM assets a JOIN observations o ON o.post_id=a.post_id WHERE {predicate} UNION ALL SELECT a.sha256 FROM assets a JOIN observations o ON o.observation_id=a.observation_id WHERE (a.post_id IS NULL OR o.post_id IS DISTINCT FROM a.post_id) AND {predicate}"
        ),
    })
}
