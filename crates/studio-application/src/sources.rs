//! Registry and small ports; no storage implementation or runtime dependency.
use crate::*;
use std::{collections::BTreeMap, sync::Arc, time::Instant};

#[derive(Clone)]
pub struct SourceReadContext {
    pub request_id: String,
    pub cancelled: ReadCancellation,
    pub priority: ReadPriority,
    pub deadline: Option<Instant>,
}
impl SourceReadContext {
    pub fn new(cancelled: ReadCancellation, priority: ReadPriority) -> Self {
        Self {
            request_id: new_id(),
            cancelled,
            priority,
            deadline: None,
        }
    }
    pub fn check(&self) -> Result<()> {
        read_cancelled(&self.cancelled)?;
        if self.deadline.is_some_and(|v| Instant::now() >= v) {
            return Err(Error::new("SOURCE_TIMEOUT", "来源读取超过截止时间"));
        }
        Ok(())
    }
}

pub struct SourceQueryOptions {
    pub memory_bytes: u64,
    pub use_candidates: bool,
    pub deadline: Option<Instant>,
    pub cancelled: ReadCancellation,
}
pub trait SourceQueryFactory: Send + Sync {
    fn create(&self, options: &SourceQueryOptions) -> Box<dyn QueryAdapter>;
}
pub trait SourceProjection: Send + Sync {
    /// Project every identity at a retained version in canonical identity order.
    /// The receiving engine seals the complete typed input as the member proof.
    fn ranking_source(
        &self,
        _source: &Source,
        _expected: &QuerySourceVersion,
        _parameters: &RankingParameters,
        _memory: u64,
        _cancelled: ReadCancellation,
        _sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>,
    ) -> Result<u64> {
        Err(Error::new(
            "RANKING_SOURCE_UNSUPPORTED",
            "该来源不支持固定版本的全湖排名",
        ))
    }
    fn origin_width(
        &self,
        source: &Source,
        asset: &str,
        expected: &QuerySourceVersion,
    ) -> Result<FrozenField>;
    fn origin_groups(
        &self,
        source: &Source,
        keys: &[AssetKey],
        expected: &QuerySourceVersion,
        cancelled: ReadCancellation,
    ) -> Result<Vec<(String, Option<i32>, String)>>;
    #[allow(clippy::too_many_arguments)]
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
    ) -> Result<()>;
}

pub struct SourceProvider {
    pub descriptor: SourceDescriptor,
    /// New canonical releases require matched metadata watermarks before attachment.
    /// Legacy image-only registrations retain their original compatibility contract.
    pub require_metadata_on_attach: bool,
    pub browser: Arc<dyn SourceAdapter>,
    pub media: Option<Arc<dyn MediaSource>>,
    pub metadata: Option<Arc<dyn MetadataAdapter>>,
    pub query: Option<Arc<dyn SourceQueryFactory>>,
    pub projection: Option<Arc<dyn SourceProjection>>,
}
#[derive(Default)]
pub struct SourceRegistry {
    providers: BTreeMap<String, SourceProvider>,
}
impl SourceRegistry {
    pub fn register(&mut self, kind: &str, provider: SourceProvider) -> Result<()> {
        let c = &provider.descriptor.capabilities;
        if kind.is_empty()
            || self.providers.contains_key(kind)
            || provider.descriptor.version != 1
            || c.media != provider.media.is_some()
            || c.metadata != provider.metadata.is_some()
            || c.query != provider.query.is_some()
            || (provider.require_metadata_on_attach && (!c.query || !c.metadata))
            || (c.raw_metadata && !c.metadata)
            || (!provider.descriptor.projections.is_empty() && provider.projection.is_none())
        {
            return Err(Error::invalid("来源注册的身份、能力与实现不一致"));
        }
        self.providers.insert(kind.into(), provider);
        Ok(())
    }
    pub fn resolve(&self, source: &Source) -> Result<&SourceProvider> {
        self.providers
            .get(&source.kind)
            .ok_or_else(|| Error::new("SOURCE_FORMAT_UNSUPPORTED", "来源类型尚未注册"))
    }
    pub fn descriptors(&self) -> Vec<(String, SourceDescriptor)> {
        self.providers
            .iter()
            .map(|(kind, p)| (kind.clone(), p.descriptor.clone()))
            .collect()
    }
}
