use super::*;

const PLANNING_KIB: u32 = 128 * 1024;
struct Budget {
    runner: Arc<Runner>,
    _memory: tokio::sync::OwnedSemaphorePermit,
    _compute: tokio::sync::OwnedSemaphorePermit,
}
impl Drop for Budget {
    fn drop(&mut self) {
        self.runner
            .reserved
            .fetch_sub(u64::from(PLANNING_KIB) * 1024, Ordering::Relaxed);
    }
}

impl Runner {
    pub(super) async fn plan_round(
        self: &Arc<Self>,
        db: Arc<EvaluationDb>,
        id: String,
        control: Control,
        memory: Arc<Semaphore>,
    ) -> Result<bool> {
        let admit = async {
            let compute = self
                .planners
                .clone()
                .acquire_owned()
                .await
                .map_err(Error::io)?;
            let memory = memory
                .acquire_many_owned(PLANNING_KIB)
                .await
                .map_err(Error::io)?;
            Ok::<_, Error>((compute, memory))
        };
        tokio::pin!(admit);
        let (compute, memory) = loop {
            tokio::select! {
                result=&mut admit => break result?,
                _=control.cancel.cancelled()=>return Err(Error::new("CANCELLED","采样计算已取消")),
                _=tokio::time::sleep(Duration::from_millis(200))=>{
                    let copy=db.clone();let sid=id.clone();
                    if work(move||copy.stage(&sid)).await?.state!="running" {return Err(Error::new("CANCELLED","采样计算已暂停"));}
                }
            }
        };
        let reserved = self
            .reserved
            .fetch_add(u64::from(PLANNING_KIB) * 1024, Ordering::Relaxed)
            + u64::from(PLANNING_KIB) * 1024;
        self.peak.fetch_max(reserved, Ordering::Relaxed);
        let _budget = Budget {
            runner: self.clone(),
            _memory: memory,
            _compute: compute,
        };
        work(move || {
            let checked = std::cell::Cell::new(Instant::now() - Duration::from_secs(1));
            db.plan_sampling(&id, &|| {
                if control.reads.load(Ordering::Acquire) {
                    return Err(Error::new("CANCELLED", "采样计算已取消"));
                }
                if checked.get().elapsed() >= Duration::from_millis(200) {
                    if db.stage(&id)?.state != "running" {
                        return Err(Error::new("CANCELLED", "采样计算已暂停"));
                    }
                    checked.set(Instant::now());
                }
                Ok(())
            })
        })
        .await
    }
}
