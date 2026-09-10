use super::*;
use std::{cmp::Ordering, collections::HashSet};
use studio_application::read_cancelled;
use studio_storage::ranking_tables::{RankingInputTable, RankingPosition, RankingResultTable};

const SMALL_SCOPE: u64 = 4096;
const PAGE_SCAN_BUDGET: usize = 4096;
const POST_SCAN_ROWS: u64 = 262_144;

#[derive(Deserialize)]
pub(super) struct InfoQuery {
    collection_id: Option<String>,
    result_id: Option<String>,
}

#[utoipa::path(get,path="/v1/projects/{project_id}/ranking-browse",operation_id="ranking_browse_info",params(("project_id"=String,Path),("collection_id"=Option<String>,Query),("result_id"=Option<String>,Query)),responses((status=200,body=RankingBrowseInfo)))]
pub(super) async fn info(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(query): Query<InfoQuery>,
) -> ApiResult<RankingBrowseInfo> {
    Ok(Json(
        blocking(move || {
            let target = match (query.collection_id, query.result_id) {
                (Some(collection_id), None) => domain::ScopeTarget::Workset { collection_id },
                (None, Some(result_id)) => domain::ScopeTarget::QueryResult { result_id },
                _ => return Err(domain::Error::invalid("请选择一个工作集或查询结果")),
            };
            let _lease = s.store.operation_lease(&pid)?;
            let scope = domain::ScopeRef {
                project_id: pid.clone(),
                target,
            };
            Ok(RankingBrowseInfo {
                ranking: s.store.ranked_scope(&pid, &scope)?.map(Into::into),
            })
        })
        .await?,
    ))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum Phase {
    Seek {
        next_ordinal: u64,
        best: Option<u64>,
    },
    Browse {
        after: Option<RankingPosition>,
        pending: Vec<u64>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    signature: String,
    state: Phase,
    start: Option<u64>,
    examined: u64,
}
fn encode(cursor: &Cursor) -> domain::Result<String> {
    let raw = serde_json::to_vec(cursor).map_err(domain::Error::io)?;
    if raw.len() > 12_288 {
        return Err(domain::Error::invalid("排名分页状态超出范围"));
    }
    Ok(URL_SAFE_NO_PAD.encode(raw))
}
fn parse_post(value: Option<String>) -> domain::Result<Option<i64>> {
    value
        .map(|value| {
            if value.is_empty() || value.len() > 19 || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(domain::Error::invalid("Danbooru ID 需要填写正整数"));
            }
            value
                .parse::<i64>()
                .ok()
                .filter(|v| *v > 0)
                .ok_or_else(|| domain::Error::invalid("Danbooru ID 超出有效范围"))
        })
        .transpose()
}
fn missing_anchor() -> domain::Error {
    domain::Error::new(
        "RANK_ANCHOR_NOT_FOUND",
        "当前范围中未找到这个 Danbooru ID，请检查 ID 或筛选范围",
    )
}

struct Browse<'a> {
    state: &'a AppState,
    pid: &'a str,
    read: &'a RequestReadContext,
    scope: domain::ScopeRef,
    basis: domain::RankedScope,
    input: RankingInputTable,
    table: RankingResultTable,
    order: domain::RankingOrder,
    descending: bool,
    post: Option<i64>,
    total: u64,
    limit: usize,
    signature: String,
}

impl Browse<'_> {
    fn position(&self, ordinal: u64) -> domain::Result<RankingPosition> {
        if ordinal >= self.total {
            return Err(domain::Error::invalid("排名游标位置超出范围"));
        }
        Ok(RankingPosition::for_scores(
            &self.table.row(ordinal)?,
            self.order,
        ))
    }
    fn keep(&self, ordinals: &[u64]) -> domain::Result<Vec<bool>> {
        let keys = ordinals
            .iter()
            .map(|v| self.input.key(*v))
            .collect::<domain::Result<Vec<_>>>()?;
        self.state
            .store
            .filter_browse_scope(self.pid, &self.scope, &keys)
    }
    fn validate_member(&self, ordinal: u64) -> domain::Result<()> {
        self.position(ordinal)?;
        if !self.keep(&[ordinal])?[0]
            || !self
                .table
                .matches_filter(ordinal, &self.basis.saved_filter)?
        {
            return Err(domain::Error::invalid("排名游标包含当前范围之外的图片"));
        }
        Ok(())
    }
    fn validate(&self, cursor: &Cursor) -> domain::Result<()> {
        if cursor.signature != self.signature || cursor.examined > self.total.saturating_mul(2) {
            return Err(domain::Error::invalid("排名游标不属于当前范围、排序或起点"));
        }
        if let Some(start) = cursor.start {
            self.validate_member(start)?;
            if self.post.is_none() || self.input.row(start)?.post_id != self.post {
                return Err(domain::Error::invalid("排名起点与 Danbooru ID 不一致"));
            }
        }
        match &cursor.state {
            Phase::Seek { next_ordinal, best } => {
                if self.post.is_none() || *next_ordinal > self.total || cursor.start.is_some() {
                    return Err(domain::Error::invalid("无效的排名定位游标"));
                }
                if let Some(best) = best {
                    self.validate_member(*best)?;
                    if self.input.row(*best)?.post_id != self.post {
                        return Err(domain::Error::invalid("定位游标的帖子 ID 不一致"));
                    }
                }
            }
            Phase::Browse { after, pending } => {
                if pending.len() > 129
                    || pending.iter().copied().collect::<HashSet<_>>().len() != pending.len()
                {
                    return Err(domain::Error::invalid("排名分页缓冲无效"));
                }
                if let Some(after) = after {
                    let expected = self.position(after.ordinal)?;
                    if expected.compare(after, false) != Ordering::Equal {
                        return Err(domain::Error::invalid("排名游标次序不一致"));
                    }
                }
                let mut previous: Option<RankingPosition> = None;
                for ordinal in pending {
                    self.validate_member(*ordinal)?;
                    let position = self.position(*ordinal)?;
                    if previous
                        .as_ref()
                        .is_some_and(|p| p.compare(&position, self.descending) != Ordering::Less)
                        || after.as_ref().is_none_or(|p| {
                            position.compare(p, self.descending) == Ordering::Greater
                        })
                    {
                        return Err(domain::Error::invalid("排名缓冲次序不一致"));
                    }
                    previous = Some(position);
                }
            }
        }
        Ok(())
    }
    fn start_cursor(&self, cursor: &Cursor) -> domain::Result<Option<String>> {
        cursor
            .start
            .map(|start| {
                encode(&Cursor {
                    signature: self.signature.clone(),
                    start: Some(start),
                    examined: 0,
                    state: Phase::Browse {
                        after: Some(self.position(start)?),
                        pending: vec![start],
                    },
                })
            })
            .transpose()
    }
    fn preparing(&self, cursor: &Cursor, seeking: bool) -> domain::Result<AssetPage> {
        let scanned = match &cursor.state {
            Phase::Seek { next_ordinal, .. } => *next_ordinal,
            _ => cursor.examined,
        };
        Ok(AssetPage {
            items: Vec::new(),
            next_cursor: Some(encode(cursor)?),
            revision: self.signature.clone(),
            preparing: Some(if seeking {
                format!("正在定位 Danbooru #{}", self.post.unwrap_or(0))
            } else {
                "正在读取这一页的排名成员".into()
            }),
            result_id: None,
            scan: Some(BrowseScan {
                scanned,
                total: self.total,
            }),
            start_cursor: self.start_cursor(cursor)?,
        })
    }
    fn finish(&self, mut cursor: Cursor, picked: Vec<u64>) -> domain::Result<AssetPage> {
        let more = picked.len() > self.limit;
        let shown = &picked[..picked.len().min(self.limit)];
        let next_cursor = if more {
            let extra = picked[self.limit];
            cursor.state = Phase::Browse {
                after: Some(self.position(extra)?),
                pending: vec![extra],
            };
            cursor.examined = 0;
            Some(encode(&cursor)?)
        } else {
            None
        };
        let sources = self
            .state
            .store
            .sources(self.pid)?
            .into_iter()
            .map(|s| (s.id, s.name))
            .collect::<BTreeMap<_, _>>();
        let keys = shown
            .iter()
            .map(|ordinal| self.input.key(*ordinal))
            .collect::<domain::Result<Vec<_>>>()?;
        let selected = self.state.store.contains(self.pid, &keys)?;
        let mut items = Vec::new();
        for ((ordinal, key), selected) in shown.iter().zip(keys).zip(selected) {
            read_cancelled(self.read.cancelled.as_ref())?;
            let input = self.input.row(*ordinal)?;
            let scores = self.table.row(*ordinal)?;
            let source_name = sources
                .get(&key.source_id)
                .cloned()
                .unwrap_or_else(|| "已固定的来源".into());
            let name = input
                .post_id
                .map(|id| format!("Danbooru #{id}"))
                .unwrap_or_else(|| key.asset_id.clone());
            let mut asset = Asset::from_domain(
                domain::Asset {
                    key,
                    name,
                    bytes: input.stored_bytes,
                    extension: input.stored_extension.clone(),
                    source_name,
                },
                selected,
            );
            asset.ranking = Some(annotation(&self.basis.artifact_id, &input, scores));
            items.push(asset);
        }
        Ok(AssetPage {
            items,
            next_cursor,
            revision: self.signature.clone(),
            preparing: None,
            result_id: None,
            scan: None,
            start_cursor: self.start_cursor(&cursor)?,
        })
    }
    fn small(&self, mut cursor: Cursor) -> domain::Result<AssetPage> {
        let keys = self
            .state
            .store
            .browse_scope_keys(self.pid, &self.scope, None, 4097, false)?;
        if keys.len() as u64 != self.basis.count || keys.len() > SMALL_SCOPE as usize {
            return Err(domain::Error::new(
                "SOURCE_CHANGED",
                "范围成员数量已变化，请重新打开",
            ));
        }
        let mut rows = Vec::new();
        for key in keys {
            read_cancelled(self.read.cancelled.as_ref())?;
            let (ordinal, post) = self.input.post_for_key(&key)?.ok_or_else(|| {
                domain::Error::new("ARTIFACT_INVALID", "工作集图片不在原排名输入中")
            })?;
            if !self
                .table
                .matches_filter(ordinal, &self.basis.saved_filter)?
            {
                return Err(domain::Error::new(
                    "ARTIFACT_INVALID",
                    "工作集成员与保存的排名条件不一致",
                ));
            }
            rows.push((self.position(ordinal)?, post));
        }
        rows.sort_by(|(a, _), (b, _)| a.compare(b, self.descending));
        if matches!(cursor.state, Phase::Seek { .. }) {
            let (start, _) = rows
                .iter()
                .find(|(_, post)| *post == self.post)
                .ok_or_else(missing_anchor)?;
            cursor.start = Some(start.ordinal);
            cursor.state = Phase::Browse {
                after: Some(start.clone()),
                pending: vec![start.ordinal],
            };
        }
        let Phase::Browse { after, mut pending } = cursor.state.clone() else {
            unreachable!()
        };
        for (position, _) in rows {
            if pending.len() > self.limit {
                break;
            }
            if after
                .as_ref()
                .is_none_or(|after| position.compare(after, self.descending) == Ordering::Greater)
            {
                pending.push(position.ordinal);
            }
        }
        self.finish(cursor, pending)
    }
    fn run(&self, mut cursor: Cursor) -> domain::Result<AssetPage> {
        self.validate(&cursor)?;
        if self.basis.count <= SMALL_SCOPE {
            return self.small(cursor);
        }
        if let Phase::Seek {
            next_ordinal,
            mut best,
        } = cursor.state.clone()
        {
            let scan = self.input.post_id_scan(
                self.post.ok_or_else(missing_anchor)?,
                next_ordinal,
                self.total,
                POST_SCAN_ROWS,
            )?;
            let kept = self.keep(&scan.ordinals)?;
            for (ordinal, keep) in scan.ordinals.into_iter().zip(kept) {
                read_cancelled(self.read.cancelled.as_ref())?;
                if keep
                    && self
                        .table
                        .matches_filter(ordinal, &self.basis.saved_filter)?
                    && match best {
                        Some(old) => {
                            self.position(ordinal)?
                                .compare(&self.position(old)?, self.descending)
                                == Ordering::Less
                        }
                        None => true,
                    }
                {
                    best = Some(ordinal);
                }
            }
            if scan.next_ordinal < self.total {
                cursor.state = Phase::Seek {
                    next_ordinal: scan.next_ordinal,
                    best,
                };
                return self.preparing(&cursor, true);
            }
            let start = best.ok_or_else(missing_anchor)?;
            cursor.start = Some(start);
            cursor.state = Phase::Browse {
                after: Some(self.position(start)?),
                pending: vec![start],
            };
        }
        let Phase::Browse {
            mut after,
            mut pending,
        } = cursor.state.clone()
        else {
            unreachable!()
        };
        let mut scanned = 0;
        loop {
            read_cancelled(self.read.cancelled.as_ref())?;
            if pending.len() > self.limit {
                return self.finish(cursor, pending);
            }
            if scanned >= PAGE_SCAN_BUDGET {
                cursor.state = Phase::Browse { after, pending };
                return self.preparing(&cursor, false);
            }
            let batch = (PAGE_SCAN_BUDGET - scanned).min(512);
            let page = self.table.browse_scan(
                &self.basis.saved_filter,
                self.order,
                self.descending,
                after.as_ref(),
                batch,
            )?;
            let candidates = page
                .rows
                .iter()
                .filter(|(_, yes)| *yes)
                .map(|(row, _)| row.ordinal)
                .collect::<Vec<_>>();
            let mut kept = self.keep(&candidates)?.into_iter();
            for (scores, matches) in page.rows {
                scanned += 1;
                cursor.examined += 1;
                after = Some(RankingPosition::for_scores(&scores, self.order));
                if matches && kept.next().unwrap_or(false) {
                    pending.push(scores.ordinal);
                }
                if pending.len() > self.limit {
                    return self.finish(cursor, pending);
                }
            }
            if !page.more {
                return self.finish(cursor, pending);
            }
        }
    }
}

fn annotation(
    aid: &str,
    input: &domain::RankingInput,
    scores: domain::RankingScores,
) -> AssetRanking {
    AssetRanking {
        artifact_id: aid.into(),
        ordinal: scores.ordinal,
        post_id: input.post_id.map(|id| id.to_string()),
        rating: scores.rating,
        eligibility: scores.eligibility.into(),
        main_score: scores.main_score,
        rescue_score: scores.rescue_score,
        main_rank: scores.main_rank,
        rescue_rank: scores.rescue_rank,
    }
}

#[utoipa::path(post,path="/v1/projects/{project_id}/ranking-browse/assets",operation_id="ranking_browse_assets",params(("project_id"=String,Path)),request_body=RankingBrowseRequest,responses((status=200,body=AssetPage)))]
pub(super) async fn assets(
    State(s): State<AppState>,
    Extension(read): Extension<RequestReadContext>,
    Path(pid): Path<String>,
    Body(body): Body<RankingBrowseRequest>,
) -> ApiResult<AssetPage> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read)?;
            let _lease = s.store.operation_lease(&pid)?;
            let scope: domain::ScopeRef = body.scope.into();
            query::validate_scope(&s, &pid, &scope)?;
            let basis = s.store.ranked_scope(&pid, &scope)?.ok_or_else(|| {
                domain::Error::new("RANKING_SCOPE_UNSUPPORTED", "当前范围没有可用的排名来源")
            })?;
            let order = basis.order(body.order.map(Into::into));
            let post = parse_post(body.start_post_id)?;
            let (artifact, table_path, input_path) =
                crate::ranking::paths(&s.store, &pid, &basis.artifact_id)?;
            let total = artifact
                .count
                .ok_or_else(|| domain::Error::new("ARTIFACT_INVALID", "排名输入数量缺失"))?;
            let signature = hex::encode(Sha256::digest(
                serde_json::to_vec(&(
                    1,
                    &pid,
                    &scope,
                    &basis.workset_id,
                    &basis.artifact_id,
                    &basis.saved_filter,
                    artifact
                        .files
                        .iter()
                        .map(|f| (&f.path, &f.sha256))
                        .collect::<Vec<_>>(),
                    order,
                    body.descending,
                    post,
                ))
                .map_err(domain::Error::io)?,
            ));
            let cursor = if let Some(raw) = body.cursor {
                if raw.len() > 16_384 {
                    return Err(domain::Error::invalid("排名游标过长"));
                }
                URL_SAFE_NO_PAD
                    .decode(raw)
                    .ok()
                    .and_then(|raw| serde_json::from_slice::<Cursor>(&raw).ok())
                    .ok_or_else(|| domain::Error::invalid("无效的排名浏览游标"))?
            } else {
                Cursor {
                    signature: signature.clone(),
                    start: None,
                    examined: 0,
                    state: if post.is_some() {
                        Phase::Seek {
                            next_ordinal: 0,
                            best: None,
                        }
                    } else {
                        Phase::Browse {
                            after: None,
                            pending: Vec::new(),
                        }
                    },
                }
            };
            Browse {
                state: &s,
                pid: &pid,
                read: &read,
                scope,
                basis,
                input: RankingInputTable::open(&input_path)?,
                table: RankingResultTable::open(&table_path)?,
                order,
                descending: body.descending,
                post,
                total,
                limit: body.limit.unwrap_or(48).clamp(1, 128),
                signature,
            }
            .run(cursor)
        })
        .await?,
    ))
}

pub(super) fn annotate(
    s: &AppState,
    pid: &str,
    scope: &domain::ScopeRef,
    read: &RequestReadContext,
    items: &mut [Asset],
) -> domain::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let Some(basis) = s.store.ranked_scope(pid, scope)? else {
        return Ok(());
    };
    let (_, table_path, input_path) = crate::ranking::paths(&s.store, pid, &basis.artifact_id)?;
    let input = RankingInputTable::open(&input_path)?;
    let table = RankingResultTable::open(&table_path)?;
    for asset in items {
        read_cancelled(read.cancelled.as_ref())?;
        let key: domain::AssetKey = asset.key.clone().into();
        let ordinal = input
            .ordinal_for_key(&key)?
            .ok_or_else(|| domain::Error::new("ARTIFACT_INVALID", "范围图片不在原排名输入中"))?;
        asset.ranking = Some(annotation(
            &basis.artifact_id,
            &input.row(ordinal)?,
            table.row(ordinal)?,
        ));
    }
    Ok(())
}
