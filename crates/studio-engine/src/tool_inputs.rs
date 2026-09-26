use crate::sources::{SourceRead, SourceService};
use std::sync::{Arc, atomic::AtomicBool};
use studio_application::{ArtifactRepository, QueryAdapter, QueryRepository};
use studio_domain::*;
use studio_storage::SqliteStore;

fn version_spec(source_id: &str, fields: &[ScalarInput], population: bool) -> QuerySpec {
    QuerySpec {
        version: 1,
        source_ids: vec![source_id.into()],
        conditions: if population || fields.contains(&ScalarInput::OriginWidth) {
            vec![QueryCondition {
                field: "source.width".into(),
                operator: QueryOperator::IsPresent,
                value: None,
            }]
        } else {
            vec![]
        },
        observation_rule: ObservationRule::AnyObservation,
        order: QueryOrder::AssetKeyAsc,
        input_scope: None,
    }
}
pub fn capture(
    store: &SqliteStore,
    pid: &str,
    scope: &ScopeRef,
    run: OperatorRun,
    sources: &SourceService,
) -> Result<JobRun> {
    let registry = studio_operators::registry()?;
    let run = registry.normalize(run)?;
    let fields = registry.resolve(&run)?.required_fields(&run.parameters)?;
    for field in &fields {
        if let ScalarInput::Artifact { artifact_id } = field {
            let artifact = store.artifact(pid, artifact_id)?;
            if artifact.state != ArtifactState::Ready
                || artifact.kind != "scalar_columns"
                || artifact.schema_version != 1
            {
                return Err(Error::new("ARTIFACT_NOT_READY", "所需标量成果尚不可用"));
            }
        }
    }
    let read = sources.background(
        ReadClass::NativeQuery,
        METADATA_MEMORY_BYTES,
        Arc::new(AtomicBool::new(false)),
    )?;
    let reader = read.query(METADATA_MEMORY_BYTES, false);
    let retained = match &scope.target {
        ScopeTarget::QueryResult { result_id } => store
            .query_result(pid, result_id)?
            .source_versions
            .into_iter()
            .filter(|v| v.consistency == "retained_online_snapshot")
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let source_versions = store
        .scope_source_ids(pid, scope)?
        .into_iter()
        .map(|id| {
            let source = store.source(pid, &id)?;
            if is_ranking_operator(&run.operator_id) {
                sources.descriptor(&source)?.require_projection(
                    if run.operator_id == RANKING_V2_OPERATOR {
                        "danbooru_ranking_v2"
                    } else {
                        "danbooru_ranking_v1"
                    },
                )?;
            }
            let spec = version_spec(&id, &fields, is_ranking_operator(&run.operator_id));
            if let Some(expected) = retained.iter().find(|v| v.source_id == id) {
                reader.validate_version(&source, expected)?;
                return reader.read_version_at(
                    &source,
                    Some(&expected.catalog_revision),
                    spec.uses_metadata(),
                );
            }
            if let ScopeTarget::Source { revision, .. } = &scope.target {
                return reader.read_version_at(&source, Some(revision), spec.uses_metadata());
            }
            reader.query_version(&source, &spec)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(JobRun {
        run,
        fields,
        source_versions,
    })
}
pub fn validate_versions(
    store: &SqliteStore,
    pid: &str,
    frozen: &JobRun,
    sources: &SourceService,
) -> Result<()> {
    let read = sources.background(
        ReadClass::NativeQuery,
        METADATA_MEMORY_BYTES,
        Arc::new(AtomicBool::new(false)),
    )?;
    let reader = read.query(METADATA_MEMORY_BYTES, false);
    for expected in &frozen.source_versions {
        reader.validate_version(&store.source(pid, &expected.source_id)?, expected)?;
    }
    Ok(())
}
pub fn project_fields(
    store: &SqliteStore,
    pid: &str,
    source: &Source,
    item: &mut FrozenInput,
    frozen: &JobRun,
    metadata: &SourceRead,
) -> Result<()> {
    if let Some(expected) = frozen
        .source_versions
        .iter()
        .find(|v| v.source_id == source.id)
        && expected.catalog_revision != item.source_revision
    {
        return Err(Error::new("SOURCE_CHANGED", "固定输入与提交版本不一致"));
    }
    for field in &frozen.fields {
        let projected = match field {
            ScalarInput::OriginWidth => {
                let expected = frozen
                    .source_versions
                    .iter()
                    .find(|v| v.source_id == source.id)
                    .ok_or_else(|| Error::new("INPUT_FIELD_MISSING", "元数据投影缺少固定版本"))?;
                metadata.origin_width(source, &item.asset.key.asset_id, expected)?
            }
            ScalarInput::StoredBytes => FrozenField {
                input: field.clone(),
                value: ScalarValue::integer(
                    i64::try_from(item.asset.bytes)
                        .map_err(|_| Error::new("FIELD_VALUE_INVALID", "对象字节数超出标量范围"))?,
                ),
                basis: FieldBasis {
                    field_id: field.field_id(),
                    subject: "asset".into(),
                    rule: "stored_object".into(),
                    source_version: Some(item.source_revision.clone()),
                    record_id: None,
                    observation_id: None,
                    artifact_id: None,
                },
            },
            ScalarInput::Artifact { artifact_id } => FrozenField {
                input: field.clone(),
                value: store.artifact_scalar(pid, artifact_id, &item.asset.key)?,
                basis: FieldBasis {
                    field_id: field.field_id(),
                    subject: "asset".into(),
                    rule: "immutable_artifact_asset_key; outside_coverage_is_uncomputed".into(),
                    source_version: None,
                    record_id: None,
                    observation_id: None,
                    artifact_id: Some(artifact_id.clone()),
                },
            },
        };
        item.fields.push(projected);
    }
    Ok(())
}
