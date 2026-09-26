//! The only production entry to source readers. A SourceRead owns exactly one admission.
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
use studio_application::*;
use studio_domain::*;

pub struct SourceService {
    registry: Arc<SourceRegistry>,
    dispatcher: super::dispatch::SourceDispatcher,
}
pub struct SourceRead {
    registry: Arc<SourceRegistry>,
    pub context: SourceReadContext,
    pub memory_bytes: u64,
    class: ReadClass,
    _lease: Box<dyn ReadLease>,
}
impl SourceService {
    pub fn new(registry: SourceRegistry, resources: Arc<dyn ReadResources>) -> Arc<Self> {
        Arc::new(Self {
            registry: Arc::new(registry),
            dispatcher: super::dispatch::SourceDispatcher::new(resources),
        })
    }
    pub fn descriptor(&self, source: &Source) -> Result<SourceDescriptor> {
        Ok(self.registry.resolve(source)?.descriptor.clone())
    }
    pub fn has(
        &self,
        source: &Source,
        capability: impl FnOnce(&SourceCapabilities) -> bool,
    ) -> bool {
        self.descriptor(source)
            .is_ok_and(|d| capability(&d.capabilities))
    }
    pub fn registrations(&self) -> Vec<(String, SourceDescriptor)> {
        self.registry.descriptors()
    }
    pub fn read(
        &self,
        class: ReadClass,
        priority: ReadPriority,
        bytes: u64,
        cancelled: ReadCancellation,
    ) -> Result<SourceRead> {
        self.admit(class, bytes, SourceReadContext::new(cancelled, priority))
    }
    pub fn admit(
        &self,
        class: ReadClass,
        bytes: u64,
        context: SourceReadContext,
    ) -> Result<SourceRead> {
        let lease = self.dispatcher.admit(class, bytes, &context)?;
        Ok(SourceRead {
            registry: self.registry.clone(),
            context,
            memory_bytes: bytes,
            class,
            _lease: lease,
        })
    }
    pub fn background(
        &self,
        class: ReadClass,
        bytes: u64,
        cancelled: ReadCancellation,
    ) -> Result<SourceRead> {
        self.read(class, ReadPriority::Background, bytes, cancelled)
    }
    pub fn validate_attachment(
        &self,
        source: &mut Source,
        context: SourceReadContext,
    ) -> Result<SourceProbe> {
        let read = self.admit(ReadClass::Index, 32 << 20, context.clone())?;
        read.infer_kind(source)?;
        let probe = read.probe(source)?;
        validate_id(&probe.id)?;
        source.id = probe.id.clone();
        let required = self.registry.resolve(source)?.require_metadata_on_attach;
        drop(read);
        if required {
            let read = self.admit(ReadClass::NativeQuery, METADATA_MEMORY_BYTES, context)?;
            let version = read
                .query(METADATA_MEMORY_BYTES, false)
                .read_version(source, true)?;
            if version.catalog_revision != probe.revision {
                return Err(Error::new(
                    "SOURCE_CHANGED",
                    "登记期间来源版本发生变化，请重试",
                ));
            }
        }
        Ok(probe)
    }
    pub fn inspect(&self) -> Result<SourceRead> {
        self.read(
            ReadClass::Index,
            ReadPriority::Interactive,
            32 << 20,
            Arc::new(AtomicBool::new(false)),
        )
    }
}
impl SourceRead {
    fn require_class(&self, class: ReadClass) -> Result<()> {
        self.context.check()?;
        if self.class != class {
            return Err(Error::new(
                "READ_BUDGET_EXCEEDED",
                "来源操作与已准入的资源类别不匹配",
            ));
        }
        Ok(())
    }
    fn provider(&self, source: &Source) -> Result<&SourceProvider> {
        self.context.check()?;
        self.registry.resolve(source)
    }
    pub fn query(&self, memory: u64, use_candidates: bool) -> SourceQuery<'_> {
        let options = SourceQueryOptions {
            memory_bytes: memory.min(self.memory_bytes),
            use_candidates,
            deadline: self.context.deadline,
        };
        let readers = self
            .registry
            .descriptors()
            .into_iter()
            .filter_map(|(kind, _)| {
                let source = Source {
                    id: String::new(),
                    name: String::new(),
                    kind: kind.clone(),
                    index_root: None,
                    media_root: None,
                };
                self.registry
                    .resolve(&source)
                    .ok()?
                    .query
                    .as_ref()
                    .map(|q| (kind, q.create(&options)))
            })
            .collect();
        SourceQuery {
            read: self,
            readers,
        }
    }
    pub fn infer_kind(&self, source: &mut Source) -> Result<()> {
        self.context.check()?;
        if source.kind == "auto" {
            if let Some(index) = &source.index_root {
                let path = index.join("ONLINE.json");
                if path.is_file() {
                    if std::fs::metadata(&path).map_err(Error::io)?.len() > 16384 {
                        return Err(Error::invalid("在线指针过大"));
                    }
                    let pointer: studio_sources::online::Pointer =
                        serde_json::from_slice(&std::fs::read(&path).map_err(Error::io)?)
                            .map_err(Error::io)?;
                    source.kind = pointer.site;
                    studio_sources::online::pointer(source)?;
                    return Ok(());
                }
            }
            let root = source
                .media_root
                .as_ref()
                .ok_or_else(|| Error::invalid("缺少图片湖目录"))?;
            source.kind = studio_sources::profiles::detected_site(root)?.ok_or_else(|| {
                Error::new(
                    "SOURCE_SITE_UNKNOWN",
                    "无法自动识别站点；旧 Danbooru 湖请显式选择 Danbooru",
                )
            })?;
        }
        Ok(())
    }
    pub fn origin_width(
        &self,
        source: &Source,
        asset: &str,
        expected: &QuerySourceVersion,
    ) -> Result<FrozenField> {
        self.require_class(ReadClass::NativeQuery)?;
        let p = self.provider(source)?;
        p.descriptor.require_projection("origin_width_v1")?;
        p.projection
            .as_ref()
            .ok_or_else(unsupported)?
            .origin_width(source, asset, expected)
    }
    pub fn origin_groups(
        &self,
        source: &Source,
        keys: &[AssetKey],
        expected: &QuerySourceVersion,
    ) -> Result<Vec<(String, Option<i32>, String)>> {
        self.require_class(ReadClass::NativeQuery)?;
        let p = self.provider(source)?;
        p.descriptor.require_projection("origin_groups_v1")?;
        p.projection
            .as_ref()
            .ok_or_else(unsupported)?
            .origin_groups(source, keys, expected, self.context.cancelled.clone())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn ranking(
        &self,
        source: &Source,
        expected: &QuerySourceVersion,
        bases: &[RankingBasis],
        parameters: &RankingParameters,
        memory: u64,
        produce: &mut RankingMemberProducer<'_>,
        sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>,
    ) -> Result<()> {
        self.require_class(ReadClass::NativeQuery)?;
        let p = self.provider(source)?;
        p.descriptor
            .require_projection(if parameters.v2.is_some() {
                "danbooru_ranking_v2"
            } else {
                "danbooru_ranking_v1"
            })?;
        p.projection.as_ref().ok_or_else(unsupported)?.ranking(
            source,
            expected,
            bases,
            parameters,
            memory.min(self.memory_bytes),
            self.context.cancelled.clone(),
            produce,
            sink,
        )
    }
}
fn unsupported() -> Error {
    Error::new("METADATA_UNSUPPORTED", "来源不支持请求的能力")
}
impl SourceAdapter for SourceRead {
    fn probe(&self, s: &Source) -> Result<SourceProbe> {
        self.provider(s)?.browser.probe(s)
    }
    fn page(&self, s: &Source, a: Option<&str>, n: usize, v: Option<&str>) -> Result<AssetPage> {
        self.provider(s)?.browser.page(s, a, n, v)
    }
    fn page_ordered(
        &self,
        s: &Source,
        a: Option<&str>,
        n: usize,
        v: Option<&str>,
        desc: bool,
    ) -> Result<AssetPage> {
        self.provider(s)?.browser.page_ordered(s, a, n, v, desc)
    }
    fn freeze(&self, s: &Source, k: &[AssetKey]) -> Result<Vec<FrozenInput>> {
        self.provider(s)?.browser.freeze(s, k)
    }
    fn freeze_at(&self, s: &Source, k: &[AssetKey], v: Option<&str>) -> Result<Vec<FrozenInput>> {
        self.provider(s)?.browser.freeze_at(s, k, v)
    }
    fn read(&self, s: &Source, id: &str) -> Result<Media> {
        let identity = self.verify_media_identity(s, id)?;
        let mut batch = self.read_many(
            s,
            &[MediaInput {
                deadline: self.context.deadline,
                asset_id: id.into(),
                cancelled: self.context.cancelled.clone(),
                byte_limit: identity.bytes.min(self.memory_bytes),
            }],
        )?;
        batch
            .items
            .pop()
            .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "来源未返回媒体"))?
    }
}
impl MediaSource for SourceRead {
    fn content_version(&self, s: &Source, id: &str) -> Result<String> {
        self.provider(s)?
            .media
            .as_ref()
            .ok_or_else(unsupported)?
            .content_version(s, id)
    }
    fn verify_media_identity(&self, s: &Source, id: &str) -> Result<MediaIdentity> {
        self.provider(s)?
            .media
            .as_ref()
            .ok_or_else(unsupported)?
            .verify_media_identity(s, id)
    }
    fn read_many(&self, s: &Source, inputs: &[MediaInput]) -> Result<MediaBatch> {
        self.require_class(ReadClass::Media)?;
        if inputs
            .iter()
            .try_fold(0u64, |n, i| n.checked_add(i.byte_limit))
            .is_none_or(|n| n > self.memory_bytes)
        {
            return Err(Error::new(
                "READ_BUDGET_EXCEEDED",
                "媒体批次超过本次读取准入额度",
            ));
        }
        let inputs = inputs
            .iter()
            .map(|i| MediaInput {
                asset_id: i.asset_id.clone(),
                cancelled: i.cancelled.clone(),
                byte_limit: i.byte_limit,
                deadline: match (i.deadline, self.context.deadline) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                },
            })
            .collect::<Vec<_>>();
        self.provider(s)?
            .media
            .as_ref()
            .ok_or_else(unsupported)?
            .read_many(s, &inputs)
    }
}
impl MetadataAdapter for SourceRead {
    fn summaries_at(
        &self,
        s: &Source,
        ids: &[String],
        revision: Option<&str>,
        c: ReadCancellation,
    ) -> Result<Vec<AssetSummary>> {
        self.provider(s)?
            .metadata
            .as_ref()
            .ok_or_else(unsupported)?
            .summaries_at(s, ids, revision, c)
    }
    fn summaries(
        &self,
        s: &Source,
        ids: &[String],
        c: ReadCancellation,
    ) -> Result<Vec<AssetSummary>> {
        self.provider(s)?
            .metadata
            .as_ref()
            .ok_or_else(unsupported)?
            .summaries(s, ids, c)
    }
    fn metadata(&self, s: &Source, id: &str, q: MetadataRequest) -> Result<MetadataOverview> {
        self.provider(s)?
            .metadata
            .as_ref()
            .ok_or_else(unsupported)?
            .metadata_context(s, id, q, &self.context)
    }
    fn observations(
        &self,
        s: &Source,
        id: &str,
        record: &str,
        q: MetadataRequest,
    ) -> Result<ObservationPage> {
        self.provider(s)?
            .metadata
            .as_ref()
            .ok_or_else(unsupported)?
            .observations_context(s, id, record, q, &self.context)
    }
    fn raw_metadata(
        &self,
        s: &Source,
        id: &str,
        record: &str,
        obs: &str,
        v: &str,
    ) -> Result<RawMetadata> {
        self.provider(s)?
            .metadata
            .as_ref()
            .ok_or_else(unsupported)?
            .raw_context(s, id, record, obs, v, &self.context)
    }
}
pub struct SourceQuery<'a> {
    read: &'a SourceRead,
    readers: BTreeMap<String, Box<dyn QueryAdapter>>,
}
impl SourceQuery<'_> {
    fn reader(&self, s: &Source) -> Result<&dyn QueryAdapter> {
        self.read.context.check()?;
        self.readers
            .get(&s.kind)
            .map(|v| v.as_ref())
            .ok_or_else(|| Error::new("QUERY_UNSUPPORTED", "来源不支持查询"))
    }
}
impl QueryAdapter for SourceQuery<'_> {
    fn read_version_at(
        &self,
        s: &Source,
        v: Option<&str>,
        metadata: bool,
    ) -> Result<QuerySourceVersion> {
        self.reader(s)?.read_version_at(s, v, metadata)
    }
    fn validate_version(&self, s: &Source, v: &QuerySourceVersion) -> Result<()> {
        self.reader(s)?.validate_version(s, v)
    }
    fn query_page(
        &self,
        s: &Source,
        q: &QuerySpec,
        v: &QuerySourceVersion,
        after: Option<&str>,
        limit: usize,
        c: ReadCancellation,
    ) -> Result<SourceQueryPage> {
        self.reader(s)?.query_page(s, q, v, after, limit, c)
    }
    fn retain_version(
        &self,
        s: &Source,
        v: &QuerySourceVersion,
        id: &str,
        owner: &str,
        permanent: bool,
    ) -> Result<()> {
        self.reader(s)?.retain_version(s, v, id, owner, permanent)
    }
    fn release_version(&self, s: &Source, id: &str) -> Result<()> {
        self.reader(s)?.release_version(s, id)
    }
    fn read_version(&self, s: &Source, metadata: bool) -> Result<QuerySourceVersion> {
        self.reader(s)?.read_version(s, metadata)
    }
    fn fields(&self, s: &Source) -> Result<FieldDirectory> {
        self.reader(s)?.fields(s)
    }
    fn query_version(&self, s: &Source, q: &QuerySpec) -> Result<QuerySourceVersion> {
        self.reader(s)?.query_version(s, q)
    }
    fn execute_query(
        &self,
        s: &Source,
        q: &QuerySpec,
        v: &QuerySourceVersion,
        c: ReadCancellation,
        out: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<()> {
        self.reader(s)?.execute_query(s, q, v, c, out)
    }
    fn execute_query_keys(
        &self,
        s: &Source,
        q: &QuerySpec,
        v: &QuerySourceVersion,
        c: ReadCancellation,
        keys: &[AssetKey],
        out: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<()> {
        self.reader(s)?.execute_query_keys(s, q, v, c, keys, out)
    }
    fn execute_delta(
        &self,
        s: &Source,
        q: &QuerySpec,
        v: &QuerySourceVersion,
        previous: &ChangeAnchor,
        c: ReadCancellation,
        affected: &mut dyn FnMut(&[AssetKey]) -> Result<()>,
        out: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<bool> {
        self.reader(s)?
            .execute_delta(s, q, v, previous, c, affected, out)
    }
    fn explain(&self, s: &Source, q: QuerySpec) -> Result<serde_json::Value> {
        self.reader(s)?.explain(s, q)
    }
    fn rating_usage(&self) -> Result<(Vec<String>, u64)> {
        let mut ratings = vec![];
        let mut rows = 0;
        for r in self.readers.values() {
            let (v, n) = r.rating_usage()?;
            ratings.extend(v);
            rows += n;
        }
        ratings.sort();
        ratings.dedup();
        Ok((ratings, rows))
    }
}
