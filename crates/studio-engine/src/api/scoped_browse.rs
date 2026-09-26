use super::*;
use std::{cmp::Ordering, collections::VecDeque};
use studio_domain::{AssetKey, QueryOrder, ScopeRef, Source};
use studio_sources::BrowseIndexReader;

const SCAN_BUDGET: usize = 8192;
const SMALL_SCOPE: u64 = 4096;
const BATCH: usize = 129;

pub(super) struct Page {
    pub keys: Vec<AssetKey>,
    pub more: bool,
    pub preparing: Option<String>,
    pub scan: Option<studio_protocol::BrowseScan>,
}
impl Page {
    fn ready(keys: Vec<AssetKey>, more: bool) -> Self {
        Self {
            keys,
            more,
            preparing: None,
            scan: None,
        }
    }
}
pub(super) fn compare(
    order: QueryOrder,
    a: &AssetKey,
    ap: Option<i64>,
    b: &AssetKey,
    bp: Option<i64>,
) -> Ordering {
    let posts = match (ap, bp) {
        (Some(a), Some(b)) => {
            if order.descending() {
                b.cmp(&a)
            } else {
                a.cmp(&b)
            }
        }
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        _ => Ordering::Equal,
    };
    posts.then_with(|| {
        if order.descending() {
            (&b.source_id, &b.asset_id).cmp(&(&a.source_id, &a.asset_id))
        } else {
            (&a.source_id, &a.asset_id).cmp(&(&b.source_id, &b.asset_id))
        }
    })
}

struct SourceStream {
    source: Source,
    index: BrowseIndexReader,
    after: Option<String>,
    exhausted: bool,
    rows: VecDeque<(AssetKey, Option<i64>, bool)>,
}
impl SourceStream {
    fn fill(
        &mut self,
        store: &studio_storage::SqliteStore,
        pid: &str,
        scope: &ScopeRef,
        order: QueryOrder,
    ) -> domain::Result<()> {
        if !self.rows.is_empty() || self.exhausted {
            return Ok(());
        }
        let rows = self
            .index
            .page(&self.source.id, order, self.after.as_deref(), BATCH)?;
        self.exhausted = rows.len() < BATCH;
        if let Some((key, _)) = rows.last() {
            self.after = Some(key.asset_id.clone());
        }
        let keys = rows.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>();
        let kept = store.filter_browse_scope(pid, scope, &keys)?;
        self.rows.extend(
            rows.into_iter()
                .zip(kept)
                .map(|((key, post), keep)| (key, post, keep)),
        );
        Ok(())
    }
}

pub(super) fn page(
    s: &AppState,
    reader: &SourceRead,
    pid: &str,
    scope: &ScopeRef,
    cursor: &mut Cursor,
    order: QueryOrder,
    limit: usize,
) -> domain::Result<Option<Page>> {
    if !order.by_post() {
        let mut keys = s.store.browse_scope_keys(
            pid,
            scope,
            cursor.last_key.as_ref(),
            limit + 1,
            order.descending(),
        )?;
        let more = keys.len() > limit;
        keys.truncate(limit);
        cursor.last_key = keys.last().cloned();
        return Ok(Some(Page::ready(keys, more)));
    }
    let count = s.store.browse_scope_count(pid, scope)?;
    if count == 0 {
        return Ok(Some(Page::ready(Vec::new(), false)));
    }
    let ids = s.store.scope_source_ids(pid, scope)?;
    let sources = ids
        .iter()
        .map(|id| s.store.source(pid, id))
        .collect::<domain::Result<Vec<_>>>()?;
    if sources.is_empty() {
        return Err(domain::Error::new(
            "SOURCE_UNAVAILABLE",
            "范围中的来源不可用",
        ));
    }
    if sources.len() > 8
        || sources
            .iter()
            .any(|source| !s.sources.has(source, |c| c.post_order))
    {
        return Ok(None);
    }
    for source in &sources {
        if let Some(revision) = cursor.revisions.get(&source.id) {
            studio_sources::BrowseIndex::verify_revision(source, revision)?;
        }
        if !s
            .queries
            .source_indexes
            .prepare_browse_index(source, reader)?
        {
            return Ok(Some(Page {
                keys: Vec::new(),
                more: false,
                preparing: Some("正在更新帖子排序索引".into()),
                scan: None,
            }));
        }
    }
    let mut streams = sources
        .into_iter()
        .map(|source| {
            let index = s.queries.source_indexes.browse_index.reader(&source)?;
            let revision = format!(
                "catalog-v1:{}:{}",
                index.stamp.generation, index.stamp.sequence
            );
            if cursor
                .revisions
                .get(&source.id)
                .is_some_and(|old| old != &revision)
            {
                return Err(domain::Error::new(
                    "SOURCE_CHANGED",
                    "来源已更新，请从第一页重新读取",
                ));
            }
            cursor.revisions.insert(source.id.clone(), revision);
            Ok(SourceStream {
                after: cursor.source_afters.get(&source.id).cloned(),
                source,
                index,
                exhausted: false,
                rows: VecDeque::new(),
            })
        })
        .collect::<domain::Result<Vec<_>>>()?;
    let result = if count <= SMALL_SCOPE {
        let keys = s
            .store
            .browse_scope_keys(pid, scope, None, SMALL_SCOPE as usize + 1, false)?;
        if keys.len() as u64 != count {
            return Err(domain::Error::new(
                "SOURCE_CHANGED",
                "范围成员已变化，请重新读取",
            ));
        }
        let mut ordered = Vec::with_capacity(keys.len());
        for stream in &streams {
            let source_keys = keys
                .iter()
                .filter(|key| key.source_id == stream.source.id)
                .cloned()
                .collect::<Vec<_>>();
            let posts = stream.index.post_ids(&source_keys)?;
            ordered.extend(source_keys.into_iter().zip(posts));
        }
        ordered.sort_by(|(a, ap), (b, bp)| compare(order, a, *ap, b, *bp));
        let start = if let Some(last) = &cursor.last_key {
            ordered
                .iter()
                .position(|(key, _)| key == last)
                .ok_or_else(|| domain::Error::invalid("排序游标不属于当前范围"))?
                + 1
        } else {
            0
        };
        let more = ordered.len().saturating_sub(start) > limit;
        let keys = ordered
            .into_iter()
            .skip(start)
            .take(limit)
            .map(|(key, _)| key)
            .collect::<Vec<_>>();
        cursor.last_key = keys.last().cloned();
        Page::ready(keys, more)
    } else {
        let total = streams
            .iter()
            .map(|stream| stream.index.stamp.count)
            .sum::<u64>();
        let mut keys = Vec::new();
        let mut consumed = 0usize;
        let mut ended = false;
        while consumed < SCAN_BUDGET {
            for stream in &mut streams {
                stream.fill(&s.store, pid, scope, order)?;
            }
            let next = streams
                .iter()
                .enumerate()
                .filter(|(_, stream)| !stream.rows.is_empty())
                .min_by(|(_, a), (_, b)| {
                    let (ak, ap, _) = a.rows.front().expect("nonempty");
                    let (bk, bp, _) = b.rows.front().expect("nonempty");
                    compare(order, ak, *ap, bk, *bp)
                })
                .map(|(index, _)| index);
            let Some(index) = next else {
                ended = true;
                break;
            };
            let stream = &mut streams[index];
            if stream.rows.front().expect("nonempty").2 && keys.len() == limit {
                break;
            }
            let (key, _, keep) = stream.rows.pop_front().expect("nonempty");
            cursor
                .source_afters
                .insert(key.source_id.clone(), key.asset_id.clone());
            cursor.scope_scanned = cursor.scope_scanned.saturating_add(1);
            consumed += 1;
            if keep {
                keys.push(key);
            }
        }
        cursor.scope_scan = true;
        cursor.last_key = keys.last().cloned().or_else(|| cursor.last_key.clone());
        let preparing = keys.is_empty() && !ended;
        Page {
            keys,
            more: !ended,
            preparing: preparing.then(|| "正在定位范围中的图像".into()),
            scan: preparing.then_some(studio_protocol::BrowseScan {
                scanned: cursor.scope_scanned.min(total),
                total,
            }),
        }
    };
    for stream in &streams {
        studio_sources::BrowseIndex::verify_revision(
            &stream.source,
            &cursor.revisions[&stream.source.id],
        )?;
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_posts_merge_sources_and_missing_posts_stay_last() {
        let a = AssetKey {
            source_id: "a".into(),
            asset_id: "2".into(),
        };
        let b = AssetKey {
            source_id: "b".into(),
            asset_id: "3".into(),
        };
        let c = AssetKey {
            source_id: "a".into(),
            asset_id: "1".into(),
        };
        let d = AssetKey {
            source_id: "a".into(),
            asset_id: "4".into(),
        };
        let mut rows = [(a, Some(100)), (b, Some(100)), (c, None), (d, Some(200))];
        rows.sort_by(|(a, ap), (b, bp)| compare(QueryOrder::PostIdDesc, a, *ap, b, *bp));
        assert_eq!(
            rows.iter()
                .map(|(key, _)| key.asset_id.as_str())
                .collect::<Vec<_>>(),
            vec!["4", "3", "2", "1"]
        );
        rows.sort_by(|(a, ap), (b, bp)| compare(QueryOrder::PostIdAsc, a, *ap, b, *bp));
        assert_eq!(
            rows.iter()
                .map(|(key, _)| key.asset_id.as_str())
                .collect::<Vec<_>>(),
            vec!["2", "3", "4", "1"]
        );
    }
}
