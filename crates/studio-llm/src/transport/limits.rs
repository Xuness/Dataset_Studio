use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use studio_application::llm::LlmCallResult;
use studio_domain::llm::*;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

#[derive(Default)]
struct State {
    active: u32,
    next: Option<Instant>,
}
#[derive(Default)]
struct Gate {
    state: Mutex<State>,
    notify: Notify,
}
pub struct Limits {
    gates: Mutex<HashMap<String, Arc<Gate>>>,
    pending: Arc<Semaphore>,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            gates: Mutex::new(HashMap::new()),
            pending: Arc::new(Semaphore::new(128)),
        }
    }
}
pub struct Permit {
    gate: Arc<Gate>,
    _pending: OwnedSemaphorePermit,
}
impl Permit {
    pub async fn before_send(&self, interval_ms: u32) -> LlmCallResult<()> {
        loop {
            let delay = {
                let mut state = self
                    .gate
                    .state
                    .lock()
                    .map_err(|_| LlmFailure::new("LLM_INTERNAL", "调用限流状态不可用"))?;
                let now = Instant::now();
                let delay = state.next.unwrap_or(now).saturating_duration_since(now);
                if delay.is_zero() {
                    state.next = Some(now + Duration::from_millis(interval_ms.into()));
                    return Ok(());
                }
                delay
            };
            tokio::time::sleep(delay).await;
        }
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut s) = self.gate.state.lock() {
            s.active = s.active.saturating_sub(1);
        }
        self.gate.notify.notify_waiters();
    }
}
impl Limits {
    pub async fn acquire(&self, provider: &LlmProvider) -> LlmCallResult<Permit> {
        let pending = self
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| LlmFailure::new("LLM_QUEUE_FULL", "LLM 调用队列已满"))?;
        let gate = {
            let mut gates = self
                .gates
                .lock()
                .map_err(|_| LlmFailure::new("LLM_INTERNAL", "调用限流状态不可用"))?;
            // Retire inactive gates; retained callers keep their own Arc.
            if gates.len() >= 256 {
                gates.retain(|_, g| Arc::strong_count(g) > 1);
            }
            gates.entry(provider.id.clone()).or_default().clone()
        };
        loop {
            let notified = gate.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = gate
                    .state
                    .lock()
                    .map_err(|_| LlmFailure::new("LLM_INTERNAL", "调用限流状态不可用"))?;
                if state.active < provider.config.network.max_concurrency {
                    state.active += 1;
                    return Ok(Permit {
                        gate: gate.clone(),
                        _pending: pending,
                    });
                }
            }
            notified.await;
        }
    }
}
