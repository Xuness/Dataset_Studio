//! Fixed rating predicates for project queries. Opens each material once per run.
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
use studio_domain::*;
use studio_storage::{
    SqliteStore, ranking_field_id,
    ranking_tables::{RankingInputTable, RankingResultTable},
};

struct Predicate {
    input: RankingInputTable,
    scores: RankingResultTable,
    ratings: Vec<String>,
}
pub struct RankingQuery {
    predicates: Vec<Predicate>,
}
impl RankingQuery {
    pub fn open(
        store: &SqliteStore,
        pid: &str,
        spec: &QuerySpec,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        let mut conditions = BTreeMap::<String, Vec<String>>::new();
        for c in &spec.conditions {
            let Some(id) = ranking_field_id(&c.field)? else {
                continue;
            };
            let ratings = match (&c.operator, &c.value) {
                (QueryOperator::Eq, Some(QueryValue::Text(v))) => vec![v.clone()],
                (QueryOperator::In, Some(QueryValue::TextList(v))) => v.clone(),
                _ => return Err(Error::invalid("不支持的评分分级条件")),
            };
            conditions
                .entry(id.into())
                .and_modify(|old| old.retain(|r| ratings.contains(r)))
                .or_insert(ratings);
        }
        let mut predicates = Vec::new();
        for (id, ratings) in conditions {
            let (_, scores, input) = crate::ranking::paths(store, pid, &id)?;
            let input = RankingInputTable::open(&input)?;
            let scores = RankingResultTable::open(&scores)?;
            scores.cancel_reads(cancelled.clone())?;
            predicates.push(Predicate {
                input,
                scores,
                ratings,
            });
        }
        Ok(Self { predicates })
    }
    pub fn active(&self) -> bool {
        !self.predicates.is_empty()
    }
    pub fn filter(&self, keys: &[AssetKey], first_matched: bool) -> Result<Vec<AssetKey>> {
        let mut kept = Vec::new();
        for key in keys {
            let mut keep = true;
            for p in self.predicates.iter().skip(usize::from(first_matched)) {
                if let Some(n) = p.input.ordinal_for_key(key)? {
                    if !p.scores.rating(n)?.is_some_and(|r| p.ratings.contains(&r)) {
                        keep = false;
                        break;
                    }
                } else {
                    keep = false;
                    break;
                }
            }
            if keep {
                kept.push(key.clone());
            }
        }
        Ok(kept)
    }
    /// The first rating index supplies candidates; further conditions are checked
    /// by the common bounded sink, including the actual workset membership.
    pub fn stream(
        &self,
        source: &str,
        cancelled: &AtomicBool,
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<u64> {
        let first = &self.predicates[0];
        let mut processed = 0;
        for rating in &first.ratings {
            let mut after = None;
            loop {
                studio_application::read_cancelled(cancelled)?;
                let ordinals = first.scores.rating_ordinals(rating, after)?;
                if ordinals.is_empty() {
                    break;
                }
                after = ordinals.last().copied();
                let keys = ordinals
                    .into_iter()
                    .map(|n| first.input.key(n))
                    .collect::<Result<Vec<_>>>()?;
                let batch_processed = keys.len() as u64;
                processed += batch_processed;
                let keys = keys
                    .into_iter()
                    .filter(|k| k.source_id == source)
                    .collect::<Vec<_>>();
                sink(&keys, batch_processed)?;
            }
        }
        Ok(processed)
    }
}
