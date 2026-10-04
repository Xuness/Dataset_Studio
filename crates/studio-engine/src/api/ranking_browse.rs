use super::*;
use std::{cmp::Ordering, collections::HashSet};
use studio_application::read_cancelled;
use studio_storage::ranked_index::{
    DISPLAY_ORDER_VERSION, RankedIndex, RankedIndexMeta, RankedIndexPlan,
};
use studio_storage::ranking_projection::RankingProjectionReader;
use studio_storage::ranking_tables::{RankingInputTable, RankingPosition, RankingResultTable};

trait RankingOrderReader {
    fn page(
        &self,
        order: domain::RankingOrder,
        descending: bool,
        after: Option<&RankingPosition>,
        limit: usize,
    ) -> domain::Result<Vec<RankingPosition>>;
    fn locate(
        &self,
        post: i64,
        order: domain::RankingOrder,
        descending: bool,
    ) -> domain::Result<Option<RankingPosition>>;
    fn locate_rank(
        &self,
        rank: u64,
        rating: &str,
        order: domain::RankingOrder,
        descending: bool,
    ) -> domain::Result<Option<RankingPosition>>;
    fn locate_position(
        &self,
        position: u64,
        order: domain::RankingOrder,
        descending: bool,
    ) -> domain::Result<Option<RankingPosition>>;
}
macro_rules! ranking_order_reader {
    ($reader:ty) => {
        impl RankingOrderReader for $reader {
            fn page(
                &self,
                order: domain::RankingOrder,
                descending: bool,
                after: Option<&RankingPosition>,
                limit: usize,
            ) -> domain::Result<Vec<RankingPosition>> {
                <$reader>::page(self, order, descending, after, limit)
            }
            fn locate(
                &self,
                post: i64,
                order: domain::RankingOrder,
                descending: bool,
            ) -> domain::Result<Option<RankingPosition>> {
                <$reader>::locate(self, post, order, descending)
            }
            fn locate_rank(
                &self,
                rank: u64,
                rating: &str,
                order: domain::RankingOrder,
                descending: bool,
            ) -> domain::Result<Option<RankingPosition>> {
                <$reader>::locate_rank(self, rank, rating, order, descending)
            }
            fn locate_position(
                &self,
                position: u64,
                order: domain::RankingOrder,
                descending: bool,
            ) -> domain::Result<Option<RankingPosition>> {
                <$reader>::locate_position(self, position, order, descending)
            }
        }
    };
}
ranking_order_reader!(RankedIndex);
ranking_order_reader!(RankingProjectionReader);

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
    SeekRank,
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
    #[serde(default)]
    first_page: bool,
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

#[derive(Serialize)]
struct RankAnchor {
    rank: u64,
    rating: Option<String>,
}
fn parse_rank(
    rank: Option<String>,
    rating: Option<String>,
    post: Option<i64>,
    order: domain::RankingOrder,
) -> domain::Result<Option<RankAnchor>> {
    let Some(rank) = rank else {
        if rating.is_some() {
            return Err(domain::Error::invalid("按 Rating 定位时需要填写排名"));
        }
        return Ok(None);
    };
    if post.is_some() {
        return Err(domain::Error::invalid("Danbooru ID 与排名起点只能选择一种"));
    }
    if rank.is_empty() || rank.len() > 19 || !rank.bytes().all(|b| b.is_ascii_digit()) {
        return Err(domain::Error::invalid("排名需要填写正整数"));
    }
    let rank = rank
        .parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| domain::Error::invalid("排名超出有效范围"))? as u64;
    if let Some(rating) = &rating {
        if !matches!(rating.as_str(), "g" | "s" | "q" | "e") {
            return Err(domain::Error::invalid("Rating 需要选择 G、S、Q 或 E"));
        }
        if matches!(order, domain::RankingOrder::Input) {
            return Err(domain::Error::invalid(
                "输入顺序没有分级名次，请选择排名顺序或总榜位置",
            ));
        }
    }
    Ok(Some(RankAnchor { rank, rating }))
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
    rank: Option<RankAnchor>,
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
            if self.rank.is_none()
                && (self.post.is_none() || self.input.row(start)?.post_id != self.post)
            {
                return Err(domain::Error::invalid("排名起点与 Danbooru ID 不一致"));
            }
        }
        match &cursor.state {
            Phase::SeekRank => {
                if self.rank.is_none() || cursor.start.is_some() {
                    return Err(domain::Error::invalid("无效的排名定位游标"));
                }
            }
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
                if self.rank.is_some() && cursor.start.is_none() {
                    return Err(domain::Error::invalid("排名定位游标缺少起点"));
                }
                if pending.len() > 129
                    || pending.iter().copied().collect::<HashSet<_>>().len() != pending.len()
                {
                    return Err(domain::Error::invalid("排名分页缓冲无效"));
                }
                if let Some(after) = after {
                    let expected = self.position(after.ordinal)?;
                    if expected.compare_ranked(after, false) != Ordering::Equal {
                        return Err(domain::Error::invalid("排名游标次序不一致"));
                    }
                }
                let mut previous: Option<RankingPosition> = None;
                for ordinal in pending {
                    self.validate_member(*ordinal)?;
                    let position = self.position(*ordinal)?;
                    if previous.as_ref().is_some_and(|p| {
                        p.compare_ranked(&position, self.descending) != Ordering::Less
                    }) || after.as_ref().is_none_or(|p| {
                        position.compare_ranked(p, self.descending) == Ordering::Greater
                    }) {
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
                    first_page: false,
                    state: Phase::Browse {
                        after: Some(self.position(start)?),
                        pending: vec![start],
                    },
                })
            })
            .transpose()
    }
    fn finish(&self, mut cursor: Cursor, picked: Vec<u64>) -> domain::Result<AssetPage> {
        let start_cursor = self.start_cursor(&cursor)?;
        cursor.first_page = false;
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
            start_cursor,
        })
    }
    fn run(
        &self,
        mut cursor: Cursor,
        index: &impl RankingOrderReader,
    ) -> domain::Result<AssetPage> {
        read_cancelled(&self.read.cancelled)?;
        if let Some(anchor) = &self.rank {
            let position =
                if let Some(position) = self.state.ranking_reads.anchors.get(&self.signature) {
                    position
                } else {
                    let position = if let Some(rating) = &anchor.rating {
                        index.locate_rank(anchor.rank, rating, self.order, self.descending)?
                    } else {
                        index.locate_position(anchor.rank, self.order, self.descending)?
                    };
                    read_cancelled(&self.read.cancelled)?;
                    self.state
                        .ranking_reads
                        .anchors
                        .insert(self.signature.clone(), position.clone());
                    position
                }
                .ok_or_else(|| {
                    domain::Error::new(
                        "RANK_POSITION_NOT_FOUND",
                        if let Some(rating) = &anchor.rating {
                            format!(
                                "当前范围中没有 {} 分级的第 {} 名，该原始名次可能已被筛选排除",
                                rating.to_uppercase(),
                                anchor.rank
                            )
                        } else {
                            format!("总榜位置超出当前范围，请输入 1 至 {}", self.basis.count)
                        },
                    )
                })?;
            if matches!(cursor.state, Phase::SeekRank) {
                cursor.start = Some(position.ordinal);
                cursor.state = Phase::Browse {
                    after: Some(position.clone()),
                    pending: vec![position.ordinal],
                };
            } else if cursor.start != Some(position.ordinal) {
                return Err(domain::Error::invalid("排名起点与指定排名不一致"));
            }
        }
        if matches!(cursor.state, Phase::Seek { .. }) {
            let position = index
                .locate(
                    self.post.ok_or_else(missing_anchor)?,
                    self.order,
                    self.descending,
                )?
                .ok_or_else(missing_anchor)?;
            cursor.start = Some(position.ordinal);
            cursor.state = Phase::Browse {
                after: Some(position.clone()),
                pending: vec![position.ordinal],
            };
        }
        let Phase::Browse { after, mut pending } = cursor.state.clone() else {
            unreachable!()
        };
        if pending.len() <= self.limit {
            pending.extend(
                index
                    .page(
                        self.order,
                        self.descending,
                        after.as_ref(),
                        self.limit + 1 - pending.len(),
                    )?
                    .into_iter()
                    .map(|p| p.ordinal),
            );
        }
        self.finish(cursor, pending)
    }
}

fn annotation(
    aid: &str,
    input: &domain::RankingInput,
    scores: domain::RankingScores,
) -> AssetRanking {
    AssetRanking {
        record_id: input.record_id.clone(),
        observation_id: input.observation_id.clone(),
        v2: scores.v2.map(Into::into),
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
            // Reading immutable ranking material does not depend on the current
            // source watermark. ranked_scope still checks ready fixed members.
            scope.validate_project(&pid)?;
            let basis = s.store.ranked_scope(&pid, &scope)?.ok_or_else(|| {
                domain::Error::new("RANKING_SCOPE_UNSUPPORTED", "当前范围没有可用的排名来源")
            })?;
            let order = basis.order(body.order.map(Into::into));
            let post = parse_post(body.start_post_id)?;
            let rank = parse_rank(body.start_rank, body.start_rating, post, order)?;
            let (artifact, table_path, input_path) =
                crate::ranking::paths(&s.store, &pid, &basis.artifact_id)?;
            let total = artifact
                .count
                .ok_or_else(|| domain::Error::new("ARTIFACT_INVALID", "排名输入数量缺失"))?;
            let mut signature = hex::encode(Sha256::digest(
                serde_json::to_vec(&(
                    DISPLAY_ORDER_VERSION,
                    &pid,
                    &basis.index_scope,
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
            // Bind numeric anchors to the same display-order revision as pages.
            if let Some(rank) = &rank {
                signature = hex::encode(Sha256::digest(
                    serde_json::to_vec(&(&signature, rank)).map_err(domain::Error::io)?,
                ));
            }
            let key_for = |scope: &domain::ScopeRef| -> domain::Result<String> {
                Ok(hex::encode(Sha256::digest(
                    serde_json::to_vec(&(
                        DISPLAY_ORDER_VERSION,
                        artifact.schema_version,
                        scope,
                        &basis.workset_id,
                        &basis.artifact_id,
                        &basis.saved_filter,
                        basis.count,
                        &artifact.files,
                    ))
                    .map_err(domain::Error::io)?,
                )))
            };
            let index_key = key_for(&basis.index_scope)?;
            let plan = RankedIndexPlan {
                requested_scope: scope.clone(),
                meta: RankedIndexMeta {
                    version: artifact.schema_version,
                    key: index_key,
                    scope: basis.index_scope.clone(),
                    count: basis.count,
                },
                project: s.store.directory(&pid)?.join("project.sqlite"),
                input: input_path.clone(),
                scores: table_path.clone(),
            };
            let cursor = if let Some(raw) = body.cursor {
                if raw.len() > 16384 {
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
                    first_page: false,
                    state: if rank.is_some() {
                        Phase::SeekRank
                    } else if post.is_some() {
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
            read_cancelled(&read.cancelled)?;
            let projection = s.store.ranking_projection(&pid, &scope)?;
            if projection.is_none() {
                s.queries.ranked_indexes.adopt(&plan, |old| {
                    Ok(old.version == plan.meta.version
                        && old.count == plan.meta.count
                        && s.store.canonical_ranked_scope(&pid, &old.scope)? == plan.meta.scope
                        && key_for(&old.scope)? == old.key)
                })?;
            }
            let browse = Browse {
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
                rank,
                total,
                limit: body.limit.unwrap_or(48).clamp(1, 128),
                signature: signature.clone(),
            };
            browse.table.cancel_reads(read.cancelled.clone())?;
            browse.validate(&cursor)?;
            read_cancelled(&read.cancelled)?;
            if let Some(projection) = projection {
                let reader = RankingProjectionReader::open(
                    &s.store.directory(&pid)?,
                    &projection,
                    read.cancelled.clone(),
                )?;
                let result = browse.run(cursor, &reader);
                read_cancelled(&read.cancelled)?;
                return result;
            }
            if let Some(index) = s
                .queries
                .ranked_indexes
                .open(&plan, read.cancelled.clone())?
            {
                let result = browse.run(cursor, &index.index);
                drop(index);
                read_cancelled(&read.cancelled)?;
                if result
                    .as_ref()
                    .is_err_and(|e| e.code == "RANKING_INDEX_INVALID")
                {
                    s.queries.ranked_indexes.invalidate(&plan.meta.key)?;
                }
                return result;
            }
            let (message, completed) = s.queries.ranked_indexes.prepare(
                s.store.clone(),
                s.queries.clone(),
                s.resources.clone(),
                s.previews.cache.clone(),
                plan.clone(),
            )?;
            Ok(AssetPage {
                items: Vec::new(),
                next_cursor: None,
                revision: signature,
                preparing: Some(message),
                result_id: None,
                scan: Some(BrowseScan {
                    scanned: completed,
                    total: plan.meta.count,
                }),
                start_cursor: None,
            })
        })
        .await?,
    ))
}

#[utoipa::path(post,path="/v1/projects/{project_id}/ranking-browse/lease",operation_id="ranking_scope_lease",params(("project_id"=String,Path)),request_body=RankingBrowseLease,responses((status=200,body=OkResponse)))]
pub(super) async fn lease(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(body): Body<RankingBrowseLease>,
) -> ApiResult<OkResponse> {
    let scope: domain::ScopeRef = body.scope.into();
    scope.validate_project(&pid)?;
    blocking(move || {
        let canonical = s.store.canonical_ranked_scope(&pid, &scope)?;
        s.queries
            .ranked_indexes
            .lease(&canonical, &body.lease_id, body.release)
    })
    .await?;
    Ok(Json(OkResponse { ok: true }))
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
