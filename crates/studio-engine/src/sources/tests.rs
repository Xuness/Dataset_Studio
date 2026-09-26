use super::*;
use std::sync::{Arc, atomic::AtomicBool};
use studio_application::*;
use studio_domain::*;

struct MemorySource;
impl SourceAdapter for MemorySource {
    fn probe(&self, s: &Source) -> Result<SourceProbe> {
        Ok(SourceProbe {
            id: s.id.clone(),
            revision: "memory-v1".into(),
            enumeration: "memory".into(),
            count: Some(1),
            index_version: 1,
        })
    }
    fn page(&self, s: &Source, _: Option<&str>, _: usize, _: Option<&str>) -> Result<AssetPage> {
        Ok(AssetPage {
            items: vec![asset(s)],
            next: None,
            revision: "memory-v1".into(),
        })
    }
    fn freeze(&self, s: &Source, _: &[AssetKey]) -> Result<Vec<FrozenInput>> {
        Ok(vec![FrozenInput {
            asset: asset(s),
            source_revision: "memory-v1".into(),
            fields: vec![],
        }])
    }
    fn read(&self, _: &Source, _: &str) -> Result<Media> {
        Ok(Media {
            bytes: b"test".to_vec(),
            content_type: "fixture".into(),
        })
    }
}
fn asset(s: &Source) -> Asset {
    Asset {
        key: AssetKey {
            source_id: s.id.clone(),
            asset_id: "not-a-sha".into(),
        },
        name: "fixture".into(),
        bytes: 4,
        extension: "fixture".into(),
        source_name: s.name.clone(),
    }
}
impl MediaSource for MemorySource {
    fn content_version(&self, _: &Source, _: &str) -> Result<String> {
        Ok("fixture-v1".into())
    }
    fn verify_media_identity(&self, _: &Source, _: &str) -> Result<MediaIdentity> {
        Ok(MediaIdentity {
            content_version: "fixture-v1".into(),
            source_revision: "memory-v1".into(),
            bytes: 4,
        })
    }
    fn read_many(&self, s: &Source, inputs: &[MediaInput]) -> Result<MediaBatch> {
        Ok(MediaBatch {
            items: inputs
                .iter()
                .map(|i| {
                    i.check()?;
                    self.read(s, &i.asset_id)
                })
                .collect(),
            stats: PhysicalReadStats::default(),
        })
    }
}
fn service() -> (Arc<SourceService>, Source) {
    let mut registry = SourceRegistry::default();
    registry
        .register(
            "memory",
            SourceProvider {
                require_metadata_on_attach: false,
                descriptor: SourceDescriptor {
                    version: 1,
                    backend_id: "memory".into(),
                    display_name: "Memory".into(),
                    site_id: None,
                    semantics_version: "fixture-v1".into(),
                    capabilities: SourceCapabilities {
                        browse: true,
                        media: true,
                        ..Default::default()
                    },
                    projections: vec![],
                },
                browser: Arc::new(MemorySource),
                media: Some(Arc::new(MemorySource)),
                metadata: None,
                query: None,
                projection: None,
            },
        )
        .unwrap();
    (
        SourceService::new(
            registry,
            Arc::new(studio_resources::ReadCoordinator::default()),
        ),
        Source {
            id: new_id(),
            kind: "memory".into(),
            name: "Memory".into(),
            index_root: None,
            media_root: None,
        },
    )
}
#[test]
fn media_only_provider_requires_no_website_or_sha_branch() {
    let (s, source) = service();
    let read = s.inspect().unwrap();
    assert_eq!(
        read.page(&source, None, 1, None).unwrap().items[0]
            .key
            .asset_id,
        "not-a-sha"
    );
    assert_eq!(
        read.query(32 << 20, false)
            .fields(&source)
            .unwrap_err()
            .code,
        "QUERY_UNSUPPORTED"
    );
    assert_eq!(
        read.metadata(&source, "not-a-sha", Default::default())
            .unwrap_err()
            .code,
        "METADATA_UNSUPPORTED"
    );
    drop(read);
    let read = s
        .background(ReadClass::Media, 4, Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert_eq!(read.read(&source, "not-a-sha").unwrap().bytes, b"test");
}
#[test]
fn admission_deadline_and_payload_limit_are_enforced() {
    let (s, source) = service();
    let mut ctx =
        SourceReadContext::new(Arc::new(AtomicBool::new(false)), ReadPriority::Interactive);
    ctx.deadline = Some(std::time::Instant::now());
    assert_eq!(
        s.admit(ReadClass::Index, 1, ctx).err().unwrap().code,
        "SOURCE_TIMEOUT"
    );
    let read = s
        .background(ReadClass::Media, 3, Arc::new(AtomicBool::new(false)))
        .unwrap();
    let inputs = [MediaInput {
        asset_id: "not-a-sha".into(),
        byte_limit: 4,
        cancelled: Arc::new(AtomicBool::new(false)),
        deadline: None,
    }];
    assert_eq!(
        read.read_many(&source, &inputs).err().unwrap().code,
        "READ_BUDGET_EXCEEDED"
    );
}
