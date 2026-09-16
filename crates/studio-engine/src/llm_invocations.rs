use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use studio_application::llm::LlmCancellation;
use studio_domain::{Error, Result, validate_id};

#[derive(Default)]
struct State {
    active: HashMap<String, LlmCancellation>,
    cancelled: HashMap<String, Instant>,
}
#[derive(Clone, Default)]
pub struct Invocations(Arc<Mutex<State>>);
pub struct InvocationGuard {
    owner: Invocations,
    id: String,
    pub cancel: LlmCancellation,
}
impl Drop for InvocationGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Ok(mut state) = self.owner.0.lock() {
            state.active.remove(&self.id);
        }
    }
}
impl Invocations {
    pub fn register(&self, id: &str) -> Result<InvocationGuard> {
        validate_id(id)?;
        let mut state = self
            .0
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "调用状态不可用"))?;
        state
            .cancelled
            .retain(|_, time| time.elapsed() < Duration::from_secs(120));
        if state.cancelled.contains_key(id) {
            return Err(Error::new("CANCELLED", "此调用 ID 已被取消"));
        }
        if state.active.contains_key(id) {
            return Err(Error::new("IDEMPOTENCY_CONFLICT", "此调用 ID 正在执行"));
        }
        if state.active.len() >= 128 {
            return Err(Error::new("LLM_QUEUE_FULL", "LLM 调用队列已满"));
        }
        let cancel = LlmCancellation::default();
        state.active.insert(id.into(), cancel.clone());
        Ok(InvocationGuard {
            owner: self.clone(),
            id: id.into(),
            cancel,
        })
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        let mut state = self
            .0
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "调用状态不可用"))?;
        state
            .cancelled
            .retain(|_, time| time.elapsed() < Duration::from_secs(120));
        if let Some(cancel) = state.active.get(id) {
            cancel.cancel();
        }
        if state.cancelled.len() >= 256
            && let Some(oldest) = state
                .cancelled
                .iter()
                .min_by_key(|(_, t)| **t)
                .map(|(id, _)| id.clone())
        {
            state.cancelled.remove(&oldest);
        }
        state.cancelled.insert(id.into(), Instant::now());
        Ok(())
    }
    pub fn cancel_all(&self) {
        if let Ok(state) = self.0.lock() {
            for cancel in state.active.values() {
                cancel.cancel();
            }
        }
    }
}
