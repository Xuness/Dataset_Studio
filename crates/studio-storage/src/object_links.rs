//! Reference traversal reads project metadata and indexed membership existence.
//! It never enumerates all members of a lake or workset to construct a graph.
use crate::*;

fn owners(table: &str, column: &str) -> String {
    format!(
        "SELECT CASE owner_kind WHEN 'collection' THEN 'workset' WHEN 'query_definition' THEN 'query' WHEN 'query_definition_input' THEN 'query' WHEN 'query_input' THEN 'query_result' WHEN 'result' THEN 'query_result' WHEN 'job_input' THEN 'job' WHEN 'job_scope' THEN 'job' ELSE owner_kind END AS kind,owner_id AS id,'使用此依据' AS relation,1 AS blocking FROM {table} WHERE {column}=?1"
    )
}
fn outgoing_refs(owner: &str) -> Vec<String> {
    vec![
        format!(
            "SELECT 'artifact' AS kind,artifact_id AS id,'计算依据' AS relation,0 AS blocking FROM artifact_references WHERE owner_kind IN ({owner}) AND owner_id=?1"
        ),
        format!(
            "SELECT 'query_result' AS kind,result_id AS id,'成员与查询依据' AS relation,0 AS blocking FROM result_references WHERE owner_kind IN ({owner}) AND owner_id=?1"
        ),
    ]
}
fn queries_using_scope(scope_kind: &str, key: &str) -> Vec<String> {
    vec![
        format!(
            "SELECT 'query' AS kind,q.id,'保存的查询输入' AS relation,1 AS blocking FROM query_definitions q WHERE json_extract(q.spec_json,'$.input_scope.target.kind')='{scope_kind}' AND json_extract(q.spec_json,'$.input_scope.target.{key}')=?1 AND NOT EXISTS(SELECT 1 FROM object_metadata m WHERE m.kind='query' AND m.id=q.id AND m.deleted=1)"
        ),
        format!(
            "SELECT 'query_result' AS kind,q.id,'查询使用的输入范围' AS relation,q.status IN ('queued','running') AS blocking FROM query_results q WHERE json_extract(q.spec_json,'$.input_scope.target.kind')='{scope_kind}' AND json_extract(q.spec_json,'$.input_scope.target.{key}')=?1 AND q.status!='released'"
        ),
    ]
}
fn raw(kind: ObjectKind, incoming: bool) -> String {
    let mut queries:Vec<String>=match (kind,incoming) {
        (ObjectKind::Artifact,true)=>vec![owners("artifact_references","artifact_id")],
        (ObjectKind::QueryResult,true)=>vec![owners("result_references","result_id")],
        (ObjectKind::Source,true)=>vec![
            "SELECT 'workset' AS kind,c.id,'包含此来源的图片' AS relation,0 AS blocking FROM collections c WHERE NOT EXISTS(SELECT 1 FROM evaluation_workset_builds b WHERE b.collection_id=c.id AND b.state!='ready') AND EXISTS(SELECT 1 FROM collection_members m WHERE m.collection_id=c.id AND m.source_id=?1)".into(),
            "SELECT 'job' AS kind,j.id,'任务输入来源' AS relation,j.status IN ('queued','waiting_input','preparing','running') AS blocking FROM jobs j LEFT JOIN job_scopes s ON s.job_id=j.id WHERE NOT EXISTS(SELECT 1 FROM object_metadata o WHERE o.kind='job' AND o.id=j.id AND o.deleted=1) AND (EXISTS(SELECT 1 FROM job_inputs i WHERE i.job_id=j.id AND i.source_id=?1) OR json_extract(s.scope_json,'$.target.source_id')=?1 OR EXISTS(SELECT 1 FROM query_results r,json_each(r.spec_json,'$.source_ids') x WHERE r.id=s.result_id AND x.value=?1))".into(),
            "SELECT 'query' AS kind,q.id,'查询来源' AS relation,0 AS blocking FROM query_definitions q WHERE EXISTS(SELECT 1 FROM json_each(q.spec_json,'$.source_ids') WHERE value=?1) AND NOT EXISTS(SELECT 1 FROM object_metadata m WHERE m.kind='query' AND m.id=q.id AND m.deleted=1)".into(),
            "SELECT 'query_result' AS kind,q.id,'查询结果来源' AS relation,q.status IN ('queued','running') AS blocking FROM query_results q WHERE q.status!='released' AND EXISTS(SELECT 1 FROM json_each(q.spec_json,'$.source_ids') WHERE value=?1)".into(),
            "SELECT 'artifact' AS kind,a.id,'已固定的计算来源' AS relation,0 AS blocking FROM artifacts a WHERE a.status!='released' AND (EXISTS(SELECT 1 FROM job_inputs i WHERE i.job_id=a.job_id AND i.source_id=?1) OR EXISTS(SELECT 1 FROM job_runs r,json_each(r.versions_json) v WHERE r.job_id=a.job_id AND json_extract(v.value,'$.source_id')=?1))".into(),
            "SELECT 'selection' AS kind,'selection' AS id,'当前选择包含此来源' AS relation,0 AS blocking WHERE EXISTS(SELECT 1 FROM selection WHERE source_id=?1) OR EXISTS(SELECT 1 FROM result_members r WHERE r.result_id=(SELECT result_id FROM selection_base WHERE singleton=1) AND r.source_id=?1 AND NOT EXISTS(SELECT 1 FROM selection_exclusions e WHERE e.source_id=r.source_id AND e.asset_id=r.asset_id))".into(),
        ],
        (ObjectKind::Workset,true)=>{
            let mut q=queries_using_scope("workset","collection_id");
            q.push("SELECT 'job' AS kind,j.id,'任务输入范围' AS relation,(j.total=0 AND j.status IN ('queued','waiting_input','preparing','running')) AS blocking FROM jobs j JOIN job_scopes s ON s.job_id=j.id WHERE json_extract(s.scope_json,'$.target.collection_id')=?1 AND NOT EXISTS(SELECT 1 FROM object_metadata m WHERE m.kind='job' AND m.id=j.id AND m.deleted=1)".into());
            q.push("SELECT 'workset' AS kind,s.collection_id AS id,'保存自此工作集' AS relation,0 AS blocking FROM collection_scopes s WHERE json_extract(s.scope_json,'$.target.collection_id')=?1".into());
            q
        },
        (ObjectKind::Query,true)=>vec!["SELECT 'query_result' AS kind,id,'保留生成时的查询快照' AS relation,0 AS blocking FROM query_results WHERE definition_id=?1 AND status!='released'".into()],
        (ObjectKind::Job,true)=>vec!["SELECT 'artifact' AS kind,id,'由此任务生成' AS relation,status!='released' AS blocking FROM artifacts WHERE job_id=?1".into()],
        (ObjectKind::Artifact,false)=>{
            let mut q=outgoing_refs("'artifact'");
            q.push("SELECT 'job' AS kind,job_id AS id,'生成任务' AS relation,0 AS blocking FROM artifacts WHERE id=?1".into());
            q
        },
        (ObjectKind::Workset,false)=>{
            let mut q=outgoing_refs("'collection'");
            q.push("SELECT 'source' AS kind,s.id,'图片来源' AS relation,0 AS blocking FROM sources s WHERE EXISTS(SELECT 1 FROM collection_members m WHERE m.collection_id=?1 AND m.source_id=s.id)".into());
            q.push("SELECT CASE json_extract(scope_json,'$.target.kind') WHEN 'workset' THEN 'workset' WHEN 'source' THEN 'source' WHEN 'query_result' THEN 'query_result' ELSE 'selection' END AS kind,COALESCE(json_extract(scope_json,'$.target.collection_id'),json_extract(scope_json,'$.target.source_id'),json_extract(scope_json,'$.target.result_id'),'selection') AS id,'保存时的输入范围' AS relation,0 AS blocking FROM collection_scopes WHERE collection_id=?1".into());
            q
        },
        (ObjectKind::Query,false)=>{
            let mut q=outgoing_refs("'query_definition','query_definition_input'");
            q.push("SELECT 'source' AS kind,x.value AS id,'查询来源' AS relation,0 AS blocking FROM query_definitions q,json_each(q.spec_json,'$.source_ids') x WHERE q.id=?1".into());
            q.push("SELECT 'workset' AS kind,json_extract(spec_json,'$.input_scope.target.collection_id') AS id,'查询输入范围' AS relation,0 AS blocking FROM query_definitions WHERE id=?1 AND json_extract(spec_json,'$.input_scope.target.kind')='workset'".into());
            q
        },
        (ObjectKind::QueryResult,false)=>{
            let mut q=outgoing_refs("'query_result','query_input','result'");
            q.push("SELECT 'query' AS kind,definition_id AS id,'生成时的查询定义' AS relation,0 AS blocking FROM query_results WHERE id=?1 AND definition_id IS NOT NULL".into());
            q.push("SELECT 'source' AS kind,x.value AS id,'查询来源' AS relation,0 AS blocking FROM query_results q,json_each(q.spec_json,'$.source_ids') x WHERE q.id=?1".into());
            q
        },
        (ObjectKind::Job,false)=>{
            let mut q=outgoing_refs("'job','job_input','job_scope'");
            q.push("SELECT 'source' AS kind,s.id,'已固定的输入来源' AS relation,0 AS blocking FROM sources s WHERE EXISTS(SELECT 1 FROM job_inputs i WHERE i.job_id=?1 AND i.source_id=s.id) OR EXISTS(SELECT 1 FROM job_runs r,json_each(r.versions_json) v WHERE r.job_id=?1 AND json_extract(v.value,'$.source_id')=s.id)".into());
            q.push("SELECT 'workset' AS kind,json_extract(scope_json,'$.target.collection_id') AS id,'提交时的工作集' AS relation,0 AS blocking FROM job_scopes WHERE job_id=?1 AND json_extract(scope_json,'$.target.kind')='workset'".into());
            q
        },
        (ObjectKind::Selection,false)=>outgoing_refs("'selection'"),
        (ObjectKind::SelectionHistory,false)=>vec![
            "SELECT 'artifact' AS kind,artifact_id AS id,'撤销历史保护' AS relation,0 AS blocking FROM artifact_references WHERE owner_kind='selection_history' AND (?1='selection' OR owner_id=?1)".into(),
            "SELECT 'query_result' AS kind,result_id AS id,'撤销历史保护' AS relation,0 AS blocking FROM result_references WHERE owner_kind='selection_history' AND (?1='selection' OR owner_id=?1)".into(),
        ],
        (ObjectKind::Project,false)=>vec![
            "SELECT 'source' AS kind,id,'项目数据湖' AS relation,0 AS blocking FROM sources WHERE ?1 IS NOT NULL".into(),
            "SELECT 'workset' AS kind,id,'项目工作集' AS relation,0 AS blocking FROM collections WHERE ?1 IS NOT NULL AND NOT EXISTS(SELECT 1 FROM evaluation_workset_builds b WHERE b.collection_id=collections.id AND b.state!='ready')".into(),
            "SELECT 'artifact' AS kind,id,'项目成果' AS relation,0 AS blocking FROM artifacts WHERE status!='released' AND ?1 IS NOT NULL".into(),
        ],
        _=>Vec::new(),
    };
    if queries.is_empty() {
        queries.push(
            "SELECT 'project' AS kind,'' AS id,'' AS relation,0 AS blocking WHERE ?1 IS NULL AND 0"
                .into(),
        );
    }
    format!(
        "SELECT DISTINCT kind,id,relation,blocking FROM ({}) WHERE id IS NOT NULL",
        queries.join(" UNION ALL ")
    )
}
const NAME: &str = "COALESCE(m.name,CASE q.kind WHEN 'source' THEN (SELECT json_extract(json,'$.name') FROM sources WHERE id=q.id) WHEN 'workset' THEN (SELECT name FROM collections WHERE id=q.id) WHEN 'artifact' THEN (SELECT name FROM artifacts WHERE id=q.id) WHEN 'query' THEN (SELECT name FROM query_definitions WHERE id=q.id) WHEN 'job' THEN (SELECT CASE operator WHEN 'danbooru.metarecall' THEN 'Danbooru 元数据排名' WHEN 'core.manifest' THEN '数据清单' WHEN 'core.scalar' THEN '标量计算' ELSE operator END FROM jobs WHERE id=q.id) WHEN 'query_result' THEN '查询结果 · '||substr(q.id,1,8) WHEN 'selection' THEN '当前选择' WHEN 'selection_history' THEN '撤销记录 · '||(SELECT label FROM selection_history WHERE CAST(id AS TEXT)=q.id) ELSE NULL END,q.kind||' · '||substr(q.id,1,8))";
#[derive(Serialize, Deserialize)]
struct Cursor {
    signature: String,
    key: String,
    blocking: bool,
}
pub(super) fn page(
    db: &Connection,
    pid: &str,
    kind: ObjectKind,
    id: &str,
    incoming: bool,
    after: Option<&str>,
    limit: usize,
) -> Result<ObjectLinkPage> {
    let signature = serde_json::to_string(&(pid, kind, id, incoming)).map_err(Error::io)?;
    let cursor = after
        .map(|text| -> Result<Cursor> {
            if text.len() > 4096 {
                return Err(Error::invalid("引用列表游标无效"));
            }
            let value: Cursor =
                serde_json::from_str(text).map_err(|_| Error::invalid("引用列表游标无效"))?;
            if value.signature != signature {
                return Err(Error::invalid("引用对象已变化，请刷新列表"));
            }
            Ok(value)
        })
        .transpose()?;
    let query = raw(kind, incoming);
    let total = db
        .query_row(&format!("SELECT COUNT(*) FROM ({query})"), [id], |r| {
            unsigned(r, 0)
        })
        .map_err(db_error)?;
    let sql = format!(
        "WITH q AS ({query}),links AS (SELECT q.kind,q.id,{NAME} AS name,q.relation,q.blocking,q.kind||':'||q.id||':'||q.relation AS link_key FROM q LEFT JOIN object_metadata m ON m.kind=q.kind AND m.id=q.id) SELECT kind,id,name,relation,blocking,link_key FROM links WHERE (?2 IS NULL OR blocking<?2 OR (blocking=?2 AND link_key>?3)) ORDER BY blocking DESC,link_key LIMIT ?4"
    );
    let limit = limit.clamp(1, 128);
    let mut statement = db.prepare(&sql).map_err(db_error)?;
    let rows = statement
        .query_map(
            params![
                id,
                cursor.as_ref().map(|c| c.blocking),
                cursor.as_ref().map(|c| &c.key),
                limit as i64 + 1
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, bool>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)?;
    let more = rows.len() > limit;
    let mut next_cursor = None;
    let mut items = Vec::new();
    for (kind, id, name, relation, blocking, key) in rows.into_iter().take(limit) {
        if more {
            next_cursor = Some(
                serde_json::to_string(&Cursor {
                    signature: signature.clone(),
                    key,
                    blocking,
                })
                .map_err(Error::io)?,
            );
        }
        items.push(ObjectLink {
            kind: ObjectKind::parse(&kind)?,
            id,
            name,
            relation,
            blocking,
        });
    }
    Ok(ObjectLinkPage {
        items,
        next_cursor,
        total,
    })
}
