use studio_domain::*;

pub(crate) fn directory(source: &Source) -> Result<FieldDirectory> {
    if source.kind != "demo" && !crate::profiles::is_canonical(source) {
        return Err(Error::new("QUERY_UNSUPPORTED", "该来源尚未支持查询"));
    }
    use FieldType::*;
    use QueryOperator::*;
    let mut fields = Vec::new();
    for (id, name, kind, unit, basis, sortable) in [
        (
            "asset.id",
            "存储对象 ID",
            Text,
            None,
            "catalog.objects.sha256",
            true,
        ),
        (
            "stored.bytes",
            "存储文件大小",
            Integer,
            Some("byte"),
            "catalog.objects.length",
            false,
        ),
        (
            "stored.extension",
            "存储格式",
            Text,
            None,
            "catalog.objects.stored_ext",
            false,
        ),
    ] {
        fields.push(FieldDefinition {
            id: id.into(),
            name: name.into(),
            field_type: kind,
            unit: unit.map(Into::into),
            missing: "null_only; empty_text_is_present".into(),
            basis: if source.kind == "demo" {
                "generated_demo".into()
            } else {
                basis.into()
            },
            display: true,
            operators: match kind {
                Integer => vec![Eq, Ne, Gte, Lte, IsMissing, IsPresent],
                _ => vec![Eq, Ne, Gte, Lte, IsMissing, IsPresent],
            },
            sortable,
            cost: if id == "asset.id" {
                "catalog_key_range; metadata_queries_may_scan".into()
            } else {
                "catalog_scan".into()
            },
        });
    }
    if crate::profiles::is_canonical(source) {
        for (id, name, kind, unit, basis) in [
            (
                "post.id",
                "来源帖子 ID",
                Integer,
                None,
                "observations.post_id",
            ),
            (
                "source.width",
                "来源宽度",
                Integer,
                Some("pixel"),
                "observations.image_width",
            ),
            (
                "source.height",
                "来源高度",
                Integer,
                Some("pixel"),
                "observations.image_height",
            ),
            (
                "source.extension",
                "来源格式",
                Text,
                None,
                "observations.file_ext",
            ),
            ("score", "评分", Integer, None, "observations.score"),
            (
                "fav_count",
                "收藏数",
                Integer,
                None,
                "observations.fav_count",
            ),
            ("rating", "分级", Text, None, "observations.rating"),
            ("tags", "标签", Tags, None, "observations.tag_string"),
            (
                "is_deleted",
                "已删除",
                Boolean,
                None,
                "observations.is_deleted",
            ),
        ] {
            if !crate::profiles::field_supported(
                source,
                basis.strip_prefix("observations.").unwrap_or(id),
            ) {
                continue;
            }
            fields.push(FieldDefinition {
                id: id.into(),
                name: crate::profiles::field_label(
                    basis.strip_prefix("observations.").unwrap_or(id),
                )
                .unwrap_or(name)
                .into(),
                field_type: kind,
                unit: unit.map(Into::into),
                missing: "null_only; empty_text_empty_tags_and_false_are_present".into(),
                basis: basis.into(),
                display: true,
                operators: match kind {
                    Integer => vec![Eq, Ne, Gte, Lte, IsMissing, IsPresent],
                    Tags => vec![
                        HasAllTags, HasAnyTags, HasNoTags, HasTag, IsMissing, IsPresent,
                    ],
                    Text if id == "rating" => vec![In, Eq, Ne, IsMissing, IsPresent],
                    _ => vec![Eq, Ne, IsMissing, IsPresent],
                },
                sortable: false,
                cost: "metadata_scan_possible".into(),
            });
        }
        for (id, name) in [("stored.width", "存储宽度"), ("stored.height", "存储高度")] {
            fields.push(FieldDefinition {
                id: id.into(),
                name: name.into(),
                field_type: Integer,
                unit: Some("pixel".into()),
                missing: "not_recorded; never_inferred_from_source_dimensions".into(),
                basis: "not_in_catalog".into(),
                display: true,
                operators: vec![],
                sortable: false,
                cost: "unsupported".into(),
            });
        }
    }
    Ok(FieldDirectory {
        version: 1,
        source_id: source.id.clone(),
        fields,
        observation_rules: vec![
            ObservationRule::CurrentPost,
            ObservationRule::AnyObservation,
        ],
        orders: vec![
            QueryOrder::PostIdDesc,
            QueryOrder::PostIdAsc,
            QueryOrder::AssetKeyAsc,
            QueryOrder::AssetKeyDesc,
        ],
        max_conditions: 12,
        direct_query: source.kind == "demo" || crate::online::available(source),
    })
}
