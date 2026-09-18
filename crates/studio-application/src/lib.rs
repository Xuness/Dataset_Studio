use std::path::PathBuf;
use studio_domain::*;
pub mod aesthetic;
pub mod llm;
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
    fn read(&self, source: &Source, asset_id: &str) -> Result<Media>;
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
    fn summaries(
        &self,
        source: &Source,
        asset_ids: &[String],
        cancelled: ReadCancellation,
    ) -> Result<Vec<AssetSummary>>;
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
