use crate::*;
use std::{path::PathBuf, sync::Arc};

struct QueryFactory {
    directory: PathBuf,
    candidates: Option<Arc<RatingCache>>,
}
impl SourceQueryFactory for QueryFactory {
    fn create(&self, options: &SourceQueryOptions) -> Box<dyn QueryAdapter> {
        let mut reader = QueryReader::with_query_directory(self.directory.clone())
            .with_query_memory(options.memory_bytes)
            .with_deadline(options.deadline)
            .with_cancellation(options.cancelled.clone());
        if options.use_candidates
            && let Some(cache) = &self.candidates
        {
            reader = reader.with_rating_cache(cache.clone());
        }
        Box::new(reader)
    }
}
struct Projections {
    metadata: Arc<MetadataReader>,
    directory: PathBuf,
}
impl SourceProjection for Projections {
    fn ranking_source(
        &self,
        source: &Source,
        expected: &QuerySourceVersion,
        parameters: &RankingParameters,
        memory: u64,
        cancelled: ReadCancellation,
        sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>,
    ) -> Result<u64> {
        RankingReader::configured(self.directory.clone(), memory)
            .project_source(source, expected, parameters, cancelled, sink)
    }
    fn origin_width(
        &self,
        source: &Source,
        asset: &str,
        expected: &QuerySourceVersion,
    ) -> Result<FrozenField> {
        self.metadata.freeze_origin_width(source, asset, expected)
    }
    fn origin_groups(
        &self,
        source: &Source,
        keys: &[AssetKey],
        expected: &QuerySourceVersion,
        cancelled: ReadCancellation,
    ) -> Result<Vec<(String, Option<i32>, String)>> {
        self.metadata
            .aesthetic_groups(source, keys, expected, cancelled)
    }
    fn ranking(
        &self,
        source: &Source,
        expected: &QuerySourceVersion,
        bases: &[RankingBasis],
        parameters: &RankingParameters,
        memory: u64,
        cancelled: ReadCancellation,
        produce: &mut RankingMemberProducer<'_>,
        sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>,
    ) -> Result<()> {
        profiles::descriptor(&source.kind)?.require_projection(if parameters.v2.is_some() {
            "danbooru_ranking_v2"
        } else {
            "danbooru_ranking_v1"
        })?;
        RankingReader::configured(self.directory.clone(), memory).project(
            source, expected, bases, parameters, cancelled, produce, sink,
        )
    }
}

pub fn registry(
    directory: PathBuf,
    identities: Option<Arc<IdentityIndex>>,
    candidates: Option<Arc<RatingCache>>,
) -> Result<SourceRegistry> {
    let mut registry = SourceRegistry::default();
    let browser = Arc::new(SourceRouter);
    let mut reader = MetadataReader::default();
    if let Some(identities) = identities {
        reader = reader.with_identity_index(identities);
    }
    let metadata = Arc::new(reader);
    let factory = Arc::new(QueryFactory {
        directory: directory.clone(),
        candidates,
    });
    let projections = Arc::new(Projections {
        metadata: metadata.clone(),
        directory,
    });
    for kind in std::iter::once("demo").chain(profiles::SITES.iter().map(|s| s.kind)) {
        registry.register(
            kind,
            SourceProvider {
                descriptor: profiles::descriptor(kind)?,
                require_metadata_on_attach: profiles::site(kind)
                    .is_some_and(|p| p.normalizer.is_some()),
                browser: browser.clone(),
                media: Some(browser.clone()),
                metadata: Some(metadata.clone()),
                query: Some(factory.clone()),
                projection: Some(projections.clone()),
            },
        )?;
    }
    Ok(registry)
}
