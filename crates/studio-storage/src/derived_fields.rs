use crate::*;

pub fn artifact_field_id(field: &str) -> Result<Option<&str>> {
    if !field.starts_with("project.") {
        return Ok(None);
    }
    let id = field
        .strip_prefix("project.")
        .and_then(|f| {
            f.strip_suffix(".value")
                .or_else(|| f.strip_suffix(".rating"))
        })
        .ok_or_else(|| Error::new("QUERY_UNSUPPORTED", "未知项目字段命名空间"))?;
    validate_id(id)?;
    Ok(Some(id))
}
pub fn ranking_field_id(field: &str) -> Result<Option<&str>> {
    let id = artifact_field_id(field)?;
    Ok(id.filter(|_| field.ends_with(".rating")))
}
fn field_artifact(db: &Connection, pid: &str, field: &str, id: &str) -> Result<Artifact> {
    if field.ends_with(".rating") {
        let item = artifacts::read(db, pid, id)?;
        if item.kind != RANKING_KIND || item.state != ArtifactState::Ready {
            return Err(Error::new(
                "ARTIFACT_NOT_READY",
                "评分分级需要可用的排名成果",
            ));
        }
        Ok(item)
    } else {
        artifacts::require_scalar(db, pid, id)
    }
}
pub fn native_spec(spec: &QuerySpec) -> QuerySpec {
    let mut native = spec.clone();
    native
        .conditions
        .retain(|c| !c.field.starts_with("project."));
    native
}
pub(crate) fn references(
    db: &Connection,
    pid: &str,
    kind: &str,
    owner: &str,
    spec: &QuerySpec,
) -> Result<()> {
    db.execute(
        "DELETE FROM artifact_references WHERE owner_kind=?1 AND owner_id=?2",
        params![kind, owner],
    )
    .map_err(db_error)?;
    for condition in &spec.conditions {
        if let Some(id) = artifact_field_id(&condition.field)? {
            field_artifact(db, pid, &condition.field, id)?;
            db.execute(
                "INSERT OR IGNORE INTO artifact_references VALUES (?1,?2,?3)",
                params![kind, owner, id],
            )
            .map_err(db_error)?;
        }
    }
    Ok(())
}
fn definition(item: &Artifact) -> FieldDefinition {
    if item.kind == RANKING_KIND {
        return FieldDefinition {
            id: format!("project.{}.rating", item.id),
            name: format!("{} · 评分分级", item.name),
            field_type: FieldType::Text,
            unit: None,
            missing: "评分时的固定分级；与当前其他关联帖子可能不同".into(),
            basis: format!("immutable ranking artifact {}", item.id),
            display: true,
            operators: vec![QueryOperator::Eq, QueryOperator::In],
            sortable: false,
            cost: "fixed_ranking_rating_index".into(),
        };
    }
    FieldDefinition {id:format!("project.{}.value",item.id),name:format!("{} · {}",item.name,&item.id[..8]),field_type:FieldType::Integer,unit:None,missing:"missing: input field absent; failed: item computation failed; uncomputed/outside coverage never matches comparisons or is_missing; zero is available".into(),basis:format!("asset key; immutable artifact {}; {} covered rows; source field observation basis retained",item.id,item.count.unwrap_or(0)),display:true,operators:vec![QueryOperator::Eq,QueryOperator::Ne,QueryOperator::Gte,QueryOperator::Lte,QueryOperator::IsMissing,QueryOperator::IsPresent],sortable:false,cost:"project_scalar_index; native_candidates_in_bounded_batches".into()}
}
impl SqliteStore {
    pub fn derived_fields(&self, pid: &str, source_id: &str) -> Result<Vec<FieldDefinition>> {
        self.source(pid, source_id)?;
        let p = self.handle(pid)?;
        let db = p.read()?;
        let mut stmt=db.prepare("SELECT a.id FROM artifacts a WHERE a.status='ready' AND (a.kind='ranking_table' OR (a.kind='scalar_columns' AND EXISTS(SELECT 1 FROM artifact_rows r WHERE r.artifact_id=a.id AND r.source_id=?1))) ORDER BY a.created_at DESC,a.id DESC LIMIT 128").map_err(db_error)?;
        let ids = stmt
            .query_map([source_id], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.iter()
            .map(|id| artifacts::read(&db, pid, id).map(|item| definition(&item)))
            .collect()
    }
    pub fn validate_derived(&self, pid: &str, spec: &QuerySpec) -> Result<()> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let mut fields = Vec::new();
        let mut derived = spec.clone();
        derived.conditions.clear();
        for condition in &spec.conditions {
            if let Some(id) = artifact_field_id(&condition.field)? {
                let artifact = field_artifact(&db, pid, &condition.field, id)?;
                if artifact.kind == RANKING_KIND {
                    let valid = match &condition.value {
                        Some(QueryValue::Text(v)) => matches!(v.as_str(), "g" | "s" | "q" | "e"),
                        Some(QueryValue::TextList(v)) => v
                            .iter()
                            .all(|r| matches!(r.as_str(), "g" | "s" | "q" | "e")),
                        _ => false,
                    };
                    if !valid {
                        return Err(Error::invalid("评分分级需要 G、S、Q、E 的值"));
                    }
                }
                fields.push(definition(&artifact));
                derived.conditions.push(condition.clone());
            }
        }
        FieldDirectory {
            version: 1,
            source_id: pid.into(),
            fields,
            observation_rules: vec![
                ObservationRule::AnyObservation,
                ObservationRule::CurrentPost,
            ],
            orders: vec![
                QueryOrder::AssetKeyAsc,
                QueryOrder::AssetKeyDesc,
                QueryOrder::PostIdAsc,
                QueryOrder::PostIdDesc,
            ],
            max_conditions: 12,
        }
        .validate(&derived)
    }
    pub fn filter_derived(
        &self,
        pid: &str,
        spec: &QuerySpec,
        keys: &[AssetKey],
    ) -> Result<Vec<AssetKey>> {
        if keys.len() > 1024 {
            return Err(Error::invalid("派生字段查询批次超出预算"));
        }
        let conditions = spec
            .conditions
            .iter()
            .filter(|c| !c.field.ends_with(".rating"))
            .filter_map(|c| match artifact_field_id(&c.field) {
                Ok(Some(id)) => Some(Ok((id, c))),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<Result<Vec<_>>>()?;
        if conditions.is_empty() {
            return Ok(keys.to_vec());
        }
        let p = self.handle(pid)?;
        let db = p.read()?;
        for (id, _) in &conditions {
            artifacts::require_scalar(&db, pid, id)?;
        }
        let mut kept = Vec::new();
        let mut stmt=db.prepare("SELECT scalar_status,scalar_value FROM artifact_rows WHERE artifact_id=?1 AND source_id=?2 AND asset_id=?3").map_err(db_error)?;
        for key in keys {
            let mut matches = true;
            for (id, condition) in &conditions {
                let row: Option<(String, Option<i64>)> = stmt
                    .query_row(params![id, key.source_id, key.asset_id], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .optional()
                    .map_err(db_error)?;
                let Some((status, value)) = row else {
                    matches = false;
                    break;
                };
                let accepted = match condition.operator {
                    QueryOperator::IsMissing => status == "missing",
                    QueryOperator::IsPresent => status == "available",
                    op => {
                        if status == "available" {
                            if let (Some(value), Some(QueryValue::Integer(expected))) =
                                (value, &condition.value)
                            {
                                let expected = expected
                                    .parse::<i64>()
                                    .map_err(|_| Error::invalid("派生字段需要整数条件"))?;
                                match op {
                                    QueryOperator::Eq => value == expected,
                                    QueryOperator::Ne => value != expected,
                                    QueryOperator::Gte => value >= expected,
                                    QueryOperator::Lte => value <= expected,
                                    _ => false,
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                };
                if !accepted {
                    matches = false;
                    break;
                }
            }
            if matches {
                kept.push(key.clone());
            }
        }
        Ok(kept)
    }
}
