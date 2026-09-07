use std::path::PathBuf;
use studio_domain::*;

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

pub struct Media {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

pub trait ProjectRepository: Send + Sync {
    fn create(&self, name: &str, parent: Option<PathBuf>) -> Result<Project>;
    fn open(&self, directory: PathBuf) -> Result<Project>;
    fn list(&self) -> Result<Vec<Project>>;
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
