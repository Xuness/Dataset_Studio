use studio_domain::*;

/// Project-owned labels, relationships and lifecycle; no UI or filesystem DTOs.
pub trait ManagementRepository: Send + Sync {
    fn managed_objects(&self, pid: &str, listing: ObjectListing) -> Result<ObjectPage>;
    fn object_details(&self, pid: &str, kind: ObjectKind, id: &str) -> Result<ObjectDetails>;
    fn object_links(
        &self,
        pid: &str,
        kind: ObjectKind,
        id: &str,
        incoming: bool,
        after: Option<&str>,
        limit: usize,
    ) -> Result<ObjectLinkPage>;
    fn edit_object(
        &self,
        pid: &str,
        kind: ObjectKind,
        id: &str,
        edit: EditObject,
    ) -> Result<ManagedObject>;
    fn remove_object(&self, pid: &str, kind: ObjectKind, id: &str, expected: u64) -> Result<()>;
    fn archive_job(
        &self,
        pid: &str,
        id: &str,
        archived: bool,
        expected: u64,
    ) -> Result<ManagedObject>;
    fn restore_source(&self, pid: &str, id: &str, expected: u64) -> Result<ManagedObject>;
}
