use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use studio_application::QueryAdapter;
use studio_domain::*;
use studio_sources::QueryReader;
use studio_storage::SqliteStore;

#[derive(Default)]
pub struct QueryRunner {
    pub reader: QueryReader,
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    stopping: AtomicBool,
}
impl QueryRunner {
    pub fn cancel(&self, id: &str) {
        if let Ok(running) = self.running.lock()
            && let Some(cancel) = running.get(id)
        {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        if let Ok(running) = self.running.lock() {
            for cancelled in running.values() {
                cancelled.store(true, Ordering::Release);
            }
        }
    }
    pub fn versions(
        &self,
        store: &SqliteStore,
        pid: &str,
        spec: &QuerySpec,
    ) -> Result<Vec<QuerySourceVersion>> {
        spec.source_ids
            .iter()
            .map(|id| self.reader.query_version(&store.source(pid, id)?, spec))
            .collect()
    }
    pub fn validate_result(&self, store: &SqliteStore, result: &QueryResult) -> Result<()> {
        if result.state != ResultState::Ready {
            return Err(Error::new("RESULT_NOT_READY", "结果尚未完整构建或已释放"));
        }
        if self.versions(store, &result.project_id, &result.spec)? != result.source_versions {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "来源版本已变化；已有结果保留固定成员，请重新计算后使用查询范围",
            ));
        }
        Ok(())
    }
    fn build(
        &self,
        store: &SqliteStore,
        result: &QueryResult,
        cancelled: Arc<AtomicBool>,
    ) -> Result<()> {
        let _lease = store.operation_lease(&result.project_id)?;
        for expected in &result.source_versions {
            let source = store.source(&result.project_id, &expected.source_id)?;
            self.reader.execute_query(
                &source,
                &result.spec,
                expected,
                cancelled.clone(),
                &mut |keys, processed| {
                    store.append_result(&result.project_id, &result.id, keys, processed)
                },
            )?;
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(Error::new("CANCELLED", "构建已取消"));
        }
        // Multi-source builds have per-source transactions; check every source again
        // before publication. This fence is explicitly not a historical snapshot.
        if self.versions(store, &result.project_id, &result.spec)? != result.source_versions {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "构建期间来源已更新，请重新计算",
            ));
        }
        Ok(())
    }
}
pub async fn scheduler(store: Arc<SqliteStore>, runner: Arc<QueryRunner>) {
    while !runner.stopping.load(Ordering::Acquire) {
        let s = store.clone();
        let next = tokio::task::spawn_blocking(move || -> Result<Option<QueryResult>> {
            s.reap_closed()?;
            for id in s.owned_projects()? {
                match s.next_result(&id) {
                    Ok(Some(result)) => return Ok(Some(result)),
                    Err(error) => tracing::warn!(project_id=%id,%error,"query scheduling failed"),
                    _ => {}
                }
            }
            Ok(None)
        })
        .await;
        match next {
            Ok(Ok(Some(result))) => {
                let cancelled = Arc::new(AtomicBool::new(false));
                if let Ok(mut running) = runner.running.lock() {
                    running.insert(result.id.clone(), cancelled.clone());
                }
                let s = store.clone();
                let r = runner.clone();
                let query = result.clone();
                let completed=tokio::task::spawn_blocking(move||->Result<()> {
                    // Pin through status publication, including user cancellation.
                    let _lease=s.operation_lease(&query.project_id)?;
                    if !s.start_result(&query.project_id,&query.id)? { return Ok(()); }
                    let built=r.build(&s,&query,cancelled);
                    let error=if r.stopping.load(Ordering::Acquire) { Some(Error::new("INTERRUPTED","引擎已停止，结果需要重新计算")) } else { built.err() };
                    if let Some(e)=&error { tracing::warn!(result_id=%query.id,code=e.code,message=%e.message,"query build stopped"); }
                    s.finish_result(&query.project_id,&query.id,error.as_ref())?;
                    Ok(())
                }).await;
                if let Ok(mut running) = runner.running.lock() {
                    running.remove(&result.id);
                }
                if let Err(error) = completed.map_err(Error::io).and_then(|r| r) {
                    let _ = store.finish_result(&result.project_id, &result.id, Some(&error));
                    tracing::warn!(%error,"query worker failed");
                }
            }
            Ok(Err(error)) => tracing::warn!(%error,"query scheduler unavailable"),
            Err(error) => tracing::error!(%error,"query scheduler panic"),
            _ => {}
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}
