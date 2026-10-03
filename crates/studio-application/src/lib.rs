use std::path::PathBuf;
use studio_domain::*;
pub mod aesthetic;
pub mod aesthetic_analysis;
pub mod lake_updates;
pub mod llm;
pub mod source_collections;
mod sources;
pub use sources::*;
mod tools;
pub use tools::*;
mod resources;
pub use resources::*;
mod management;
pub use management::*;

/// Application ports contain no window, HTTP, or database types.
pub trait SourceAdapter: Send + Sync {
    fn probe(&self, source: &Source) -> Result<SourceProbe>;
    fn page(
        &self,
        source: &Source,
        after: Option<&str>,
        limit: usize,
        revision: Option<&str>,
    ) -> Result<AssetPage>;
    fn freeze(&self, source: &Source, keys: &[AssetKey]) -> Result<Vec<FrozenInput>>;
    fn freeze_at(
        &self,
        source: &Source,
        keys: &[AssetKey],
        revision: Option<&str>,
    ) -> Result<Vec<FrozenInput>> {
        let rows = self.freeze(source, keys)?;
        if revision.is_some_and(|v| rows.iter().any(|r| r.source_revision != v)) {
            return Err(Error::new("SOURCE_CHANGED", "来源版本已变化"));
        }
        Ok(rows)
    }
    fn read(&self, source: &Source, asset_id: &str) -> Result<Media>;
    fn page_ordered(
        &self,
        source: &Source,
        after: Option<&str>,
        limit: usize,
        revision: Option<&str>,
        descending: bool,
    ) -> Result<AssetPage> {
        if descending {
            return Err(Error::new("QUERY_UNSUPPORTED", "来源不支持降序浏览"));
        }
        self.page(source, after, limit, revision)
    }
}

#[derive(Debug, Clone)]
pub struct SourceProbe {
    pub id: String,
    pub revision: String,
    pub enumeration: String,
    pub count: Option<u64>,
    pub index_version: u32,
}

#[derive(Debug, Clone)]
pub struct Media {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

/// Metadata inspection is independent of project selection and task inputs.
pub trait MetadataAdapter: Send + Sync {
    fn source_relation(
        &self,
        _source: &Source,
        _request: SourceRelationRequest,
        _context: &SourceReadContext,
    ) -> Result<serde_json::Value> {
        Err(Error::new(
            "METADATA_UNSUPPORTED",
            "来源不支持作品或作者关联读取",
        ))
    }
    fn metadata_context(
        &self,
        source: &Source,
        asset: &str,
        request: MetadataRequest,
        context: &SourceReadContext,
    ) -> Result<MetadataOverview> {
        context.check()?;
        self.metadata_cancelled(source, asset, request, context.cancelled.clone())
    }
    fn observations_context(
        &self,
        source: &Source,
        asset: &str,
        record: &str,
        request: MetadataRequest,
        context: &SourceReadContext,
    ) -> Result<ObservationPage> {
        context.check()?;
        self.observations_cancelled(source, asset, record, request, context.cancelled.clone())
    }
    fn raw_context(
        &self,
        source: &Source,
        asset: &str,
        record: &str,
        observation: &str,
        version: &str,
        context: &SourceReadContext,
    ) -> Result<RawMetadata> {
        context.check()?;
        self.raw_metadata_cancelled(
            source,
            asset,
            record,
            observation,
            version,
            context.cancelled.clone(),
        )
    }

    fn summaries(
        &self,
        source: &Source,
        asset_ids: &[String],
        cancelled: ReadCancellation,
    ) -> Result<Vec<AssetSummary>>;
    fn summaries_at(
        &self,
        source: &Source,
        asset_ids: &[String],
        revision: Option<&str>,
        cancelled: ReadCancellation,
    ) -> Result<Vec<AssetSummary>> {
        if revision.is_some() {
            return Err(Error::new("METADATA_UNSUPPORTED", "来源不支持指定版本摘要"));
        }
        self.summaries(source, asset_ids, cancelled)
    }
    fn metadata(
        &self,
        source: &Source,
        asset_id: &str,
        request: MetadataRequest,
    ) -> Result<MetadataOverview>;
    fn observations(
        &self,
        source: &Source,
        asset_id: &str,
        record_id: &str,
        request: MetadataRequest,
    ) -> Result<ObservationPage>;
    fn raw_metadata(
        &self,
        source: &Source,
        asset_id: &str,
        record_id: &str,
        observation_id: &str,
        version: &str,
    ) -> Result<RawMetadata>;

    fn metadata_cancelled(
        &self,
        source: &Source,
        asset_id: &str,
        request: MetadataRequest,
        cancelled: ReadCancellation,
    ) -> Result<MetadataOverview> {
        read_cancelled(&cancelled)?;
        self.metadata(source, asset_id, request)
    }
    fn observations_cancelled(
        &self,
        source: &Source,
        asset_id: &str,
        record_id: &str,
        request: MetadataRequest,
        cancelled: ReadCancellation,
    ) -> Result<ObservationPage> {
        read_cancelled(&cancelled)?;
        self.observations(source, asset_id, record_id, request)
    }
    fn raw_metadata_cancelled(
        &self,
        source: &Source,
        asset_id: &str,
        record_id: &str,
        observation_id: &str,
        version: &str,
        cancelled: ReadCancellation,
    ) -> Result<RawMetadata> {
        read_cancelled(&cancelled)?;
        self.raw_metadata(source, asset_id, record_id, observation_id, version)
    }
}

/// Adapters stream bounded identity batches. The receiver owns deduplication and publication.
pub trait QueryAdapter: Send + Sync {
    /// Exact published membership count at a retained source version. None
    /// preserves the materialized-capture path for other source formats.
    fn snapshot_count(
        &self,
        _source: &Source,
        _expected: &QuerySourceVersion,
    ) -> Result<Option<u64>> {
        Ok(None)
    }
    fn read_version_at(
        &self,
        source: &Source,
        revision: Option<&str>,
        metadata: bool,
    ) -> Result<QuerySourceVersion> {
        let version = self.read_version(source, metadata)?;
        if revision.is_some_and(|v| v != version.catalog_revision) {
            return Err(Error::new("SOURCE_CHANGED", "来源版本已变化"));
        }
        Ok(version)
    }
    fn validate_version(&self, source: &Source, expected: &QuerySourceVersion) -> Result<()> {
        if self.read_version(source, expected.analysis_sequence.is_some())? != *expected {
            return Err(Error::new("SOURCE_CHANGED", "来源版本已变化"));
        }
        Ok(())
    }
    fn query_page(
        &self,
        _source: &Source,
        _spec: &QuerySpec,
        _expected: &QuerySourceVersion,
        _after: Option<&str>,
        _limit: usize,
        _cancelled: ReadCancellation,
    ) -> Result<SourceQueryPage> {
        Err(Error::new(
            "QUERY_VIEW_UNSUPPORTED",
            "该来源需要先生成固定查询结果",
        ))
    }
    fn retain_version(
        &self,
        _source: &Source,
        _expected: &QuerySourceVersion,
        _id: &str,
        _owner: &str,
        _permanent: bool,
    ) -> Result<()> {
        Err(Error::new(
            "QUERY_VIEW_UNSUPPORTED",
            "来源不支持保留读取视图",
        ))
    }
    fn release_version(&self, _source: &Source, _id: &str) -> Result<()> {
        Ok(())
    }
    fn read_version(&self, _source: &Source, _metadata: bool) -> Result<QuerySourceVersion> {
        Err(Error::new("QUERY_UNSUPPORTED", "来源未提供独立版本探测"))
    }
    fn fields(&self, source: &Source) -> Result<FieldDirectory>;
    fn query_version(&self, source: &Source, spec: &QuerySpec) -> Result<QuerySourceVersion>;
    fn execute_query(
        &self,
        source: &Source,
        spec: &QuerySpec,
        expected: &QuerySourceVersion,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<()>;
    /// Optional complete capture with ordering metadata from the same source
    /// snapshot. False means unsupported and must not emit any rows.
    fn execute_query_hits(
        &self,
        _source: &Source,
        _spec: &QuerySpec,
        _expected: &QuerySourceVersion,
        _cancelled: ReadCancellation,
        _sink: &mut dyn FnMut(&[QueryHit], u64) -> Result<()>,
    ) -> Result<bool> {
        Ok(false)
    }
    fn execute_query_keys(
        &self,
        _source: &Source,
        _spec: &QuerySpec,
        _expected: &QuerySourceVersion,
        _cancelled: ReadCancellation,
        _keys: &[AssetKey],
        _sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<()> {
        Err(Error::new("QUERY_UNSUPPORTED", "来源不支持指定成员查询"))
    }
    #[allow(clippy::too_many_arguments)]
    fn execute_delta(
        &self,
        _source: &Source,
        _spec: &QuerySpec,
        _expected: &QuerySourceVersion,
        _previous: &ChangeAnchor,
        _cancelled: ReadCancellation,
        _affected: &mut dyn FnMut(&[AssetKey]) -> Result<()>,
        _sink: &mut dyn FnMut(&[AssetKey], u64) -> Result<()>,
    ) -> Result<bool> {
        Ok(false)
    }
    fn explain(&self, _source: &Source, _spec: QuerySpec) -> Result<serde_json::Value> {
        Err(Error::new("QUERY_UNSUPPORTED", "来源不提供查询计划诊断"))
    }
    fn rating_usage(&self) -> Result<(Vec<String>, u64)> {
        Ok((vec![], 0))
    }
}

pub trait QueryRepository: Send + Sync {
    fn save_query(
        &self,
        project_id: &str,
        name: &str,
        spec: QuerySpec,
        previous: Option<(&str, u64)>,
    ) -> Result<QueryDefinition>;
    fn query_definition(&self, project_id: &str, id: &str) -> Result<QueryDefinition>;
    fn query_definitions(
        &self,
        project_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueryDefinition>>;
    fn create_result(
        &self,
        project_id: &str,
        definition: Option<(&str, u64)>,
        spec: QuerySpec,
        versions: Vec<QuerySourceVersion>,
    ) -> Result<QueryResult>;
    fn query_result(&self, project_id: &str, id: &str) -> Result<QueryResult>;
    fn query_results(
        &self,
        project_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueryResult>>;
    fn result_page(
        &self,
        project_id: &str,
        id: &str,
        after: Option<&AssetKey>,
        limit: usize,
    ) -> Result<ResultPage>;
    fn cancel_result(&self, project_id: &str, id: &str) -> Result<QueryResult>;
    fn release_result(&self, project_id: &str, id: &str) -> Result<QueryResult>;
}

pub trait ScopeRepository: Send + Sync {
    fn change_selection_scope(
        &self,
        project_id: &str,
        expected_revision: u64,
        scope: &ScopeRef,
        operation: ScopeOperation,
    ) -> Result<Selection>;
    fn save_scope_collection(
        &self,
        project_id: &str,
        name: &str,
        scope: &ScopeRef,
    ) -> Result<Collection>;
}

pub trait ProjectRepository: Send + Sync {
    fn create(&self, name: &str, parent: Option<PathBuf>) -> Result<Project>;
    fn open(&self, directory: PathBuf) -> Result<Project>;
    fn list(&self) -> Result<Vec<ProjectSummary>>;
    fn open_recent(&self, id: &str) -> Result<Project>;
    fn close(&self, id: &str) -> Result<ProjectClose>;
    fn project(&self, id: &str) -> Result<Project>;
    fn attach(&self, project_id: &str, source: Source) -> Result<()>;
    fn sources(&self, project_id: &str) -> Result<Vec<Source>>;
    fn selection(&self, project_id: &str) -> Result<Selection>;
    fn selection_keys(
        &self,
        project_id: &str,
        after: Option<&AssetKey>,
        limit: usize,
    ) -> Result<Vec<AssetKey>>;
    fn change_selection(
        &self,
        project_id: &str,
        expected_revision: u64,
        add: &[AssetKey],
        remove: &[AssetKey],
        clear: bool,
    ) -> Result<Selection>;
    fn collections(&self, project_id: &str) -> Result<Vec<Collection>>;
    fn save_collection(&self, project_id: &str, name: &str) -> Result<Collection>;
    fn collection_keys(
        &self,
        project_id: &str,
        collection_id: &str,
        after: Option<&AssetKey>,
        limit: usize,
    ) -> Result<Vec<AssetKey>>;
    fn events(&self, project_id: &str, after: u64) -> Result<Vec<ProjectEvent>>;
}
