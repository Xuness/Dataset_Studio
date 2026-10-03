use super::*;
use rusqlite::types::Value;
use studio_application::ReadCancellation;

fn field(field: &str, version: u32) -> Result<&'static str> {
    if version == 3 {
        return Ok(match field {
            "asset.id" => "o.sha256",
            "stored.bytes" => "o.length",
            "stored.extension" => "o.stored_ext",
            "stored.width" => "o.stored_width",
            "stored.height" => "o.stored_height",
            "work.id" => "p.work_id",
            "author.id" => "p.author_id",
            "work.type" => "p.work_type",
            "work.title" => "p.title",
            "source.width" => "m.width",
            "source.height" => "m.height",
            "tags" => "p.observation_id",
            "pixiv.x_restrict" => "json_extract(p.source_fields_json,'$.pixiv.x_restrict')",
            "pixiv.ai_type" => "json_extract(p.source_fields_json,'$.pixiv.ai_type')",
            "pixiv.bookmark_count" => "json_extract(p.source_fields_json,'$.pixiv.bookmark_count')",
            "pixiv.view_count" => "json_extract(p.source_fields_json,'$.pixiv.view_count')",
            "pixiv.like_count" => "json_extract(p.source_fields_json,'$.pixiv.like_count')",
            _ => return Err(Error::new("QUERY_UNSUPPORTED", "媒体湖不支持该字段")),
        });
    }
    Ok(match field {
        "asset.id" => "o.sha256",
        "stored.bytes" => "o.length",
        "stored.extension" => "o.stored_ext",
        "post.id" => "p.post_id",
        "source.width" => "p.image_width",
        "source.height" => "p.image_height",
        "source.extension" => "p.file_ext",
        "score" => "p.score",
        "fav_count" => "p.fav_count",
        "rating" => "p.rating",
        "tags" => "p.tag_string",
        "is_deleted" => "p.is_deleted",
        _ => return Err(Error::new("QUERY_UNSUPPORTED", "在线查询不支持该字段")),
    })
}
fn bind(values: &mut Vec<Value>, value: Value) -> String {
    values.push(value);
    format!("?{}", values.len())
}
fn value(value: &QueryValue) -> Result<Value> {
    Ok(match value {
        QueryValue::Text(v) => Value::Text(v.clone()),
        QueryValue::Integer(v) => Value::Integer(v.parse().map_err(error)?),
        QueryValue::Boolean(v) => Value::Integer(i64::from(*v)),
        _ => return Err(Error::invalid("需要单个条件值")),
    })
}
struct Predicates {
    storage: String,
    metadata: String,
    values: Vec<Value>,
    positive: Option<String>,
    impossible: bool,
}
impl Snapshot {
    pub(crate) fn ranking_keys(&self, after: &str) -> Result<Vec<String>> {
        let mut statement = self
            .db
            .prepare(
                "SELECT sha256 FROM visible_objects WHERE sha256>?1 ORDER BY sha256 LIMIT 32768",
            )
            .map_err(sql_error)?;
        statement
            .query_map([after], |r| r.get(0))
            .map_err(sql_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql_error)
    }
    fn candidate_seed(
        &self,
        spec: &QuerySpec,
        predicates: &Predicates,
    ) -> Result<Option<Vec<i64>>> {
        let post = spec
            .conditions
            .iter()
            .find(|c| c.field == "post.id" && c.operator == QueryOperator::Eq);
        let (sql, args) =
            if let Some(QueryValue::Integer(post)) = post.and_then(|c| c.value.as_ref()) {
                (
                    "SELECT row_id FROM visible_observations WHERE post_id=?1 LIMIT 5001",
                    Value::Integer(post.parse().map_err(Error::io)?),
                )
            } else if let Some(expression) = &predicates.positive {
                (
                    "SELECT rowid FROM tag_index WHERE tag_index MATCH ?1 LIMIT 5001",
                    Value::Text(expression.clone()),
                )
            } else {
                return Ok(None);
            };
        let rows = self
            .db
            .prepare(sql)
            .map_err(sql_error)?
            .query_map([args], |r| r.get::<_, i64>(0))
            .map_err(sql_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        Ok((rows.len() <= 5000).then_some(rows))
    }
    pub fn indexed_keys(&self, source: &Source, spec: &QuerySpec) -> Result<Option<Vec<AssetKey>>> {
        let mut predicates = self.predicates(spec)?;
        if predicates.impossible {
            return Ok(Some(vec![]));
        }
        if let Some(QueryValue::Text(id)) = spec
            .conditions
            .iter()
            .find(|c| c.field == "asset.id" && c.operator == QueryOperator::Eq)
            .and_then(|c| c.value.as_ref())
        {
            return self
                .filter_keys(
                    source,
                    spec,
                    &[AssetKey {
                        source_id: source.id.clone(),
                        asset_id: id.clone(),
                    }],
                )
                .map(Some);
        }
        let Some(seed) = self.candidate_seed(spec, &predicates)? else {
            return Ok(None);
        };
        let input = bind(
            &mut predicates.values,
            Value::Text(serde_json::to_string(&seed).map_err(Error::io)?),
        );
        let matched = self.matching(
            spec,
            &predicates.metadata,
            false,
            Some(&format!("SELECT value FROM json_each({input})")),
        );
        let sql = format!(
            "SELECT o.sha256 FROM visible_objects o WHERE o.sha256 IN ({matched}) AND ({}) LIMIT 16385",
            predicates.storage
        );
        let keys = self
            .db
            .prepare(&sql)
            .map_err(sql_error)?
            .query_map(rusqlite::params_from_iter(predicates.values.iter()), |r| {
                Ok(AssetKey {
                    source_id: source.id.clone(),
                    asset_id: r.get(0)?,
                })
            })
            .map_err(sql_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        Ok((keys.len() <= 16384).then_some(keys))
    }
    pub(super) fn ranking_predicate(&self, spec: &QuerySpec) -> Result<(String, Vec<Value>)> {
        let mut spec = spec.clone();
        spec.conditions.retain(|c| {
            !c.field.starts_with("stored.")
                && !c.field.starts_with("project.")
                && c.field != "asset.id"
        });
        let predicates = self.predicates(&spec)?;
        Ok((predicates.metadata, predicates.values))
    }
    pub fn filter_keys(
        &self,
        source: &Source,
        spec: &QuerySpec,
        keys: &[AssetKey],
    ) -> Result<Vec<AssetKey>> {
        let mut predicates = self.predicates(spec)?;
        if predicates.impossible {
            return Ok(Vec::new());
        }
        let ids = keys.iter().map(|k| k.asset_id.clone()).collect::<Vec<_>>();
        let keys = bind(
            &mut predicates.values,
            Value::Text(serde_json::to_string(&ids).map_err(Error::io)?),
        );
        let metadata = if spec.uses_metadata() {
            format!(
                " AND EXISTS({})",
                self.matching(spec, &predicates.metadata, true, None)
            )
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT o.sha256 FROM visible_objects o WHERE o.sha256 IN (SELECT value FROM json_each({keys})) AND ({}){metadata} ORDER BY o.sha256",
            predicates.storage
        );
        self.db
            .prepare(&sql)
            .map_err(sql_error)?
            .query_map(rusqlite::params_from_iter(predicates.values.iter()), |r| {
                Ok(AssetKey {
                    source_id: source.id.clone(),
                    asset_id: r.get(0)?,
                })
            })
            .map_err(sql_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql_error)
    }
    /// Capture identities and browse order in one read. A per-object lookup via
    /// a fresh connection for every small sink batch defeats a sequential scan.
    pub(crate) fn stream_all_hits_window(
        &self,
        source: &Source,
        after: i64,
        sink: &mut dyn FnMut(&[QueryHit], u64) -> Result<()>,
    ) -> Result<Option<i64>> {
        let end = after.saturating_add(32768);
        let mut statement = self.db.prepare(
            "SELECT o.sha256,v.post_id FROM visible_objects o LEFT JOIN object_order v ON v.sha256=o.sha256 WHERE o.object_row>?1 AND o.object_row<=?2",
        ).map_err(sql_error)?;
        let mut rows = statement.query([after, end]).map_err(sql_error)?;
        let mut batch = Vec::with_capacity(512);
        while let Some(row) = rows.next().map_err(sql_error)? {
            batch.push(QueryHit {
                key: AssetKey {
                    source_id: source.id.clone(),
                    asset_id: row.get(0).map_err(sql_error)?,
                },
                post_id: row.get(1).map_err(sql_error)?,
            });
            if batch.len() == 512 {
                sink(&batch, batch.len() as u64)?;
                batch.clear();
            }
        }
        if !batch.is_empty() {
            sink(&batch, batch.len() as u64)?;
        }
        let more: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM visible_objects WHERE object_row>?1)",
                [end],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        Ok(more.then_some(end))
    }
    pub fn stream_window(
        &self,
        source: &Source,
        spec: &QuerySpec,
        after: i64,
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<Option<i64>> {
        let mut predicates = self.predicates(spec)?;
        if predicates.impossible {
            return Ok(None);
        }
        let end = after.saturating_add(32768);
        let begin = bind(&mut predicates.values, Value::Integer(after));
        let stop = bind(&mut predicates.values, Value::Integer(end));
        let metadata = if spec.uses_metadata() {
            format!(
                " AND EXISTS({})",
                self.matching(spec, &predicates.metadata, true, None)
            )
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT o.sha256 FROM visible_objects o WHERE o.object_row>{begin} AND o.object_row<={stop} AND ({}){metadata}",
            predicates.storage
        );
        let mut stmt = self.db.prepare(&sql).map_err(sql_error)?;
        let mut rows = stmt
            .query(rusqlite::params_from_iter(predicates.values.iter()))
            .map_err(sql_error)?;
        let mut batch = Vec::new();
        while let Some(row) = rows.next().map_err(sql_error)? {
            batch.push(AssetKey {
                source_id: source.id.clone(),
                asset_id: row.get(0).map_err(sql_error)?,
            });
            if batch.len() == 512 {
                sink(&batch, batch.len() as u64)?;
                batch.clear();
            }
        }
        if !batch.is_empty() {
            sink(&batch, batch.len() as u64)?;
        }
        let more: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM visible_objects WHERE object_row>?1)",
                [end],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        Ok(more.then_some(end))
    }
    fn predicates(&self, spec: &QuerySpec) -> Result<Predicates> {
        let mut storage = Vec::new();
        let mut metadata = Vec::new();
        let mut values = Vec::new();
        let mut positives = Vec::new();
        let mut impossible = false;
        for condition in &spec.conditions {
            let column = field(&condition.field, self.pointer.schema_version)?;
            use QueryOperator::*;
            let sql = match condition.operator {
                IsMissing => format!("{column} IS NULL"),
                IsPresent => format!("{column} IS NOT NULL"),
                HasTag | HasAllTags | HasAnyTags | HasNoTags => {
                    let tags = match &condition.value {
                        Some(QueryValue::Text(v)) => vec![v.clone()],
                        Some(QueryValue::TextList(v)) => v.clone(),
                        _ => return Err(Error::invalid("标签条件无效")),
                    };
                    let mut terms = Vec::new();
                    let mut absent = 0;
                    for tag in tags {
                        let id: Option<i64> = self
                            .db
                            .query_row("SELECT tag_id FROM tags WHERE tag=?1", [&tag], |r| r.get(0))
                            .optional()
                            .map_err(sql_error)?;
                        if let Some(id) = id {
                            terms.push(format!("t{id:x}"));
                        } else {
                            absent += 1;
                        }
                    }
                    let all = matches!(condition.operator, HasTag | HasAllTags);
                    if (all && absent > 0) || terms.is_empty() {
                        if condition.operator == HasNoTags {
                            format!("{column} IS NOT NULL")
                        } else {
                            impossible = true;
                            "0".into()
                        }
                    } else {
                        let expression =
                            format!("({})", terms.join(if all { " AND " } else { " OR " }));
                        if condition.operator != HasNoTags {
                            positives.push(expression.clone());
                        }
                        let parameter = bind(&mut values, Value::Text(expression));
                        let exists = format!(
                            "EXISTS(SELECT 1 FROM tag_index WHERE rowid=p.row_id AND tag_index MATCH {parameter})"
                        );
                        if condition.operator == HasNoTags {
                            format!("{column} IS NOT NULL AND NOT {exists}")
                        } else {
                            exists
                        }
                    }
                }
                In => {
                    let Some(QueryValue::TextList(list)) = &condition.value else {
                        return Err(Error::invalid("集合条件无效"));
                    };
                    let entries = list
                        .iter()
                        .map(|v| bind(&mut values, Value::Text(v.clone())))
                        .collect::<Vec<_>>();
                    format!("{column} IN ({})", entries.join(","))
                }
                Eq | Ne | Gte | Lte => {
                    let parameter = bind(
                        &mut values,
                        value(
                            condition
                                .value
                                .as_ref()
                                .ok_or_else(|| Error::invalid("缺少条件值"))?,
                        )?,
                    );
                    let op = match condition.operator {
                        Eq => "=",
                        Ne => "!=",
                        Gte => ">=",
                        Lte => "<=",
                        _ => unreachable!(),
                    };
                    format!("{column}{op}{parameter}")
                }
            };
            if column.starts_with("o.") {
                storage.push(format!("({sql})"));
            } else {
                metadata.push(format!("({sql})"));
            }
        }
        Ok(Predicates {
            storage: if storage.is_empty() {
                "1".into()
            } else {
                storage.join(" AND ")
            },
            metadata: if metadata.is_empty() {
                "1".into()
            } else {
                metadata.join(" AND ")
            },
            values,
            positive: (!positives.is_empty()).then(|| positives.join(" AND ")),
            impossible,
        })
    }
    fn matching(
        &self,
        spec: &QuerySpec,
        predicate: &str,
        correlated: bool,
        seed: Option<&str>,
    ) -> String {
        let asset = if correlated {
            " AND a.sha256=o.sha256"
        } else {
            ""
        };
        let seed = seed
            .map(|v| format!(" AND p.row_id IN ({v})"))
            .unwrap_or_default();
        if self.pointer.schema_version == 3 {
            return match spec.observation_rule {
                ObservationRule::CurrentPost => format!(
                    "SELECT a.sha256 FROM current_works cw JOIN visible_works p ON p.observation_id=cw.observation_id JOIN visible_media m ON m.manifest_id=cw.manifest_id JOIN current_media_assets ma ON ma.media_id=m.media_id JOIN visible_assets a ON a.asset_id=ma.asset_id WHERE ({predicate}){asset}{seed}"
                ),
                ObservationRule::AnyObservation => format!(
                    "SELECT a.sha256 FROM visible_assets a JOIN visible_media m ON m.media_id=a.media_id JOIN visible_manifests mf ON mf.manifest_id=m.manifest_id JOIN visible_works p ON p.observation_id=mf.detail_observation_id WHERE ({predicate}){asset}{seed}"
                ),
            };
        }
        match spec.observation_rule {
            ObservationRule::CurrentPost => format!(
                "SELECT a.sha256 FROM current_posts cp JOIN visible_assets a ON a.asset_id=cp.asset_id JOIN visible_observations p ON p.row_id=cp.row_id WHERE ({predicate}){asset}{seed}"
            ),
            ObservationRule::AnyObservation => format!(
                "SELECT a.sha256 FROM visible_assets a JOIN visible_observations p ON p.post_id=a.post_id WHERE ({predicate}){asset}{seed} UNION ALL SELECT a.sha256 FROM visible_assets a JOIN visible_observations p ON p.observation_id=a.observation_id WHERE (a.post_id IS NULL OR p.post_id IS DISTINCT FROM a.post_id) AND ({predicate}){asset}{seed}"
            ),
        }
    }
    pub fn page_query(
        &self,
        source: &Source,
        spec: &QuerySpec,
        after: Option<&str>,
        limit: usize,
    ) -> Result<SourceQueryPage> {
        crate::query::fields::directory(source)?.validate(spec)?;
        let limit = limit.clamp(1, 129);
        let mut predicates = self.predicates(spec)?;
        if predicates.impossible {
            return Ok(SourceQueryPage {
                total_objects: self.count,
                hits: vec![],
                next: None,
                scanned: 0,
            });
        }
        let direction = if spec.order.descending() {
            "DESC"
        } else {
            "ASC"
        };
        let op = if spec.order.descending() { "<" } else { ">" };
        let mut after_clause = "1".to_string();
        if let Some(after) = after {
            if after.len() != 64 || hex::decode(after).is_err() {
                return Err(Error::invalid("在线查询游标无效"));
            }
            let key = bind(&mut predicates.values, Value::Text(after.into()));
            if spec.order.by_post() {
                let post: Option<Option<i64>> = self
                    .db
                    .query_row(
                        "SELECT post_id FROM object_order WHERE sha256=?1",
                        [after],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(sql_error)?;
                match post.ok_or_else(|| Error::invalid("游标不属于该视图"))? {
                    Some(post) => {
                        let p = bind(&mut predicates.values, Value::Integer(post));
                        after_clause =
                            format!("(v.post_id IS NULL OR (v.post_id,o.sha256){op}({p},{key}))");
                    }
                    None => after_clause = format!("v.post_id IS NULL AND o.sha256{op}{key}"),
                }
            } else {
                after_clause = format!("o.sha256{op}{key}");
            }
        }
        let order = if spec.order.by_post() {
            format!("v.post_id {direction} NULLS LAST,o.sha256 {direction}")
        } else {
            format!("o.sha256 {direction}")
        };
        // Small inverted results are sorted directly. Large results use ordered
        // object seeks plus exact per-observation membership probes.
        let mut candidates = None;
        if let Some(ids) = self.candidate_seed(spec, &predicates)? {
            let p = bind(
                &mut predicates.values,
                Value::Text(serde_json::to_string(&ids).map_err(Error::io)?),
            );
            candidates = Some(self.matching(
                spec,
                &predicates.metadata,
                false,
                Some(&format!("SELECT value FROM json_each({p})")),
            ));
        }
        if let Some(candidates) = candidates {
            let sql = format!(
                "WITH candidates AS MATERIALIZED(SELECT DISTINCT sha256 FROM ({candidates})) SELECT o.sha256,v.post_id FROM candidates c JOIN visible_objects o ON o.sha256=c.sha256 JOIN object_order v ON v.sha256=o.sha256 WHERE ({}) AND ({after_clause}) ORDER BY {order} LIMIT {}",
                predicates.storage,
                limit + 1
            );
            let mut stmt = self.db.prepare(&sql).map_err(sql_error)?;
            let mut hits = stmt
                .query_map(rusqlite::params_from_iter(predicates.values.iter()), |r| {
                    Ok(QueryHit {
                        key: AssetKey {
                            source_id: source.id.clone(),
                            asset_id: r.get(0)?,
                        },
                        post_id: r.get(1)?,
                    })
                })
                .map_err(sql_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sql_error)?;
            let more = hits.len() > limit;
            hits.truncate(limit);
            return Ok(SourceQueryPage {
                total_objects: self.count,
                next: if more {
                    hits.last().map(|h| h.key.asset_id.clone())
                } else {
                    None
                },
                scanned: hits.len() as u64,
                hits,
            });
        }
        let keep = if spec.uses_metadata() {
            format!(
                "EXISTS({})",
                self.matching(spec, &predicates.metadata, true, None)
            )
        } else {
            "1".into()
        };
        // The caller gets an explicit scan continuation when selective predicates
        // need more work; there is no implicit whole-lake result materialization.
        let scan_limit = if !spec.conditions.is_empty() {
            2048
        } else {
            limit + 1
        };
        let window = self.ordered_window(spec.order, after, scan_limit)?;
        let count = window.len();
        let input = bind(&mut predicates.values, Value::Null);
        let sql = format!(
            "SELECT (({}) AND ({keep})) FROM visible_objects o WHERE o.sha256={input}",
            predicates.storage
        );
        let mut stmt = self.db.prepare(&sql).map_err(sql_error)?;
        let mut hits = Vec::new();
        let mut scanned = 0;
        let mut last = None;
        // Evaluate in the already-indexed order, stopping after one lookahead.
        // Sorting an SQL expression table would evaluate every predicate in the
        // whole window before yielding the first row (costly for popular tags).
        for (id, post_id) in window {
            self.check()?;
            *predicates.values.last_mut().expect("object parameter") = Value::Text(id.clone());
            let matched = stmt
                .query_row(rusqlite::params_from_iter(predicates.values.iter()), |r| {
                    r.get::<_, Option<bool>>(0)
                })
                .optional()
                .map_err(sql_error)?
                .flatten()
                .unwrap_or(false);
            if matched {
                if hits.len() == limit {
                    return Ok(SourceQueryPage {
                        total_objects: self.count,
                        hits,
                        next: last,
                        scanned,
                    });
                }
                hits.push(QueryHit {
                    key: AssetKey {
                        source_id: source.id.clone(),
                        asset_id: id.clone(),
                    },
                    post_id,
                });
            }
            last = Some(id);
            scanned += 1;
        }
        Ok(SourceQueryPage {
            total_objects: self.count,
            hits,
            next: if count == scan_limit { last } else { None },
            scanned,
        })
    }
    fn ordered_window(
        &self,
        order: QueryOrder,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Option<i64>)>> {
        let direction = if order.descending() { "DESC" } else { "ASC" };
        let op = if order.descending() { "<" } else { ">" };
        if !order.by_post() {
            let condition = if after.is_some() {
                format!("o.sha256{op}?1")
            } else {
                "?1 IS NULL".into()
            };
            let mut statement = self.db.prepare(&format!("SELECT o.sha256,v.post_id FROM visible_objects o CROSS JOIN object_order v ON v.sha256=o.sha256 WHERE {condition} ORDER BY o.sha256 {direction} LIMIT ?2")).map_err(sql_error)?;
            return statement
                .query_map(params![after, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(sql_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sql_error);
        }
        let post: Option<i64> = after
            .map(|key| {
                self.db
                    .query_row(
                        "SELECT post_id FROM object_order WHERE sha256=?1",
                        [key],
                        |r| r.get(0),
                    )
                    .map_err(sql_error)
            })
            .transpose()?
            .flatten();
        let mut output = Vec::new();
        // Keep nullable posts in a separate indexed seek. An OR spanning null
        // and non-null branches makes SQLite sort the remaining whole lake.
        for missing in [false, true] {
            if (!missing && after.is_some() && post.is_none()) || output.len() == limit {
                continue;
            }
            let condition = if missing {
                if after.is_some() && post.is_none() {
                    format!("v.post_id IS NULL AND v.sha256{op}?2")
                } else {
                    "v.post_id IS NULL".into()
                }
            } else if post.is_some() {
                format!("v.post_id IS NOT NULL AND (v.post_id,v.sha256){op}(?1,?2)")
            } else {
                "v.post_id IS NOT NULL".into()
            };
            let mut statement=self.db.prepare(&format!("SELECT v.sha256,v.post_id FROM object_order v WHERE {condition} ORDER BY v.post_id {direction},v.sha256 {direction} LIMIT ?3")).map_err(sql_error)?;
            let remaining = limit - output.len();
            output.extend(
                statement
                    .query_map(params![post, after, remaining as i64], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .map_err(sql_error)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(sql_error)?,
            );
        }
        Ok(output)
    }
    pub fn stream_query(
        &self,
        source: &Source,
        spec: &QuerySpec,
        cancelled: ReadCancellation,
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<()> {
        let mut after = None;
        loop {
            studio_application::read_cancelled(&cancelled)?;
            let page = self.page_query(source, spec, after.as_deref(), 128)?;
            sink(
                &page.hits.into_iter().map(|h| h.key).collect::<Vec<_>>(),
                page.scanned,
            )?;
            after = page.next;
            if after.is_none() {
                return Ok(());
            }
        }
    }
}
