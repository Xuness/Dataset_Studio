use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};
use studio_application::{ReadLease, ReadResources, read_cancelled};
use studio_domain::*;

struct Waiting {
    id: u64,
    request: ReadRequest,
}
struct ClassState {
    metrics: ReadMetrics,
    queue: VecDeque<Waiting>,
    interactive_streak: usize,
}
struct State {
    classes: Vec<ClassState>,
    next: u64,
}
struct Inner {
    state: Mutex<State>,
    changed: Condvar,
}
#[derive(Clone)]
pub struct ReadCoordinator {
    inner: Arc<Inner>,
}
impl Default for ReadCoordinator {
    fn default() -> Self {
        Self::new(vec![
            ReadBudget {
                class: ReadClass::Index,
                concurrency: 4,
                queue_limit: 64,
                bytes: 128 << 20,
            },
            ReadBudget {
                class: ReadClass::Media,
                concurrency: 1,
                queue_limit: 32,
                bytes: 64 << 20,
            },
            ReadBudget {
                class: ReadClass::Decode,
                concurrency: 2,
                queue_limit: 32,
                bytes: 512 << 20,
            },
            ReadBudget {
                class: ReadClass::NativeQuery,
                concurrency: 2,
                queue_limit: 32,
                bytes: QUERY_MEMORY_BYTES + METADATA_MEMORY_BYTES,
            },
        ])
    }
}
impl ReadCoordinator {
    /// Called between range queries. In-flight metadata reservations remain valid
    /// because the minimum range-query budget is larger than two metadata reads.
    pub fn set_query_memory(&self, bytes: u64) -> Result<()> {
        if !(1 << 30..=64 << 30).contains(&bytes) {
            return Err(Error::invalid("范围查询内存须为 1 至 64 GiB"));
        }
        let mut state = self.inner.state.lock().map_err(|_| lock_error())?;
        let class = state
            .classes
            .iter_mut()
            .find(|c| c.metrics.budget.class == ReadClass::NativeQuery)
            .ok_or_else(|| Error::invalid("未配置原生查询资源"))?;
        class.metrics.budget.bytes = bytes + METADATA_MEMORY_BYTES;
        self.inner.changed.notify_all();
        Ok(())
    }

    pub fn new(budgets: Vec<ReadBudget>) -> Self {
        assert!(
            budgets
                .iter()
                .all(|b| b.concurrency > 0 && b.queue_limit > 0 && b.bytes > 0)
        );
        Self {
            inner: Arc::new(Inner {
                changed: Condvar::new(),
                state: Mutex::new(State {
                    next: 0,
                    classes: budgets
                        .into_iter()
                        .map(|budget| ClassState {
                            metrics: ReadMetrics {
                                budget,
                                active: 0,
                                queued: 0,
                                reserved_bytes: 0,
                                peak_reserved_bytes: 0,
                                started: 0,
                                completed: 0,
                                cancelled_waiting: 0,
                                rejected: 0,
                                wait_ms: 0,
                                max_wait_ms: 0,
                                work_ms: 0,
                            },
                            queue: VecDeque::new(),
                            interactive_streak: 0,
                        })
                        .collect(),
                }),
            }),
        }
    }
}
fn lock_error() -> Error {
    Error::new("INTERNAL_ERROR", "读取调度状态不可用")
}
fn millis(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}
impl ReadResources for ReadCoordinator {
    fn acquire(&self, request: ReadRequest, cancelled: &AtomicBool) -> Result<Box<dyn ReadLease>> {
        self.acquire_until(request, cancelled, None)
    }
    fn acquire_until(
        &self,
        request: ReadRequest,
        cancelled: &AtomicBool,
        deadline: Option<Instant>,
    ) -> Result<Box<dyn ReadLease>> {
        read_cancelled(cancelled)?;
        let start = Instant::now();
        let mut state = self.inner.state.lock().map_err(|_| lock_error())?;
        let class_index = state
            .classes
            .iter()
            .position(|c| c.metrics.budget.class == request.class)
            .ok_or_else(|| Error::invalid("未配置该读取资源"))?;
        let id = state.next;
        state.next += 1;
        let class = &mut state.classes[class_index];
        if request.bytes > class.metrics.budget.bytes
            || class.queue.len() >= class.metrics.budget.queue_limit
        {
            class.metrics.rejected += 1;
            return Err(Error::new(
                "READ_BUDGET_EXCEEDED",
                "读取预算或等待队列已满，请稍后重试",
            ));
        }
        class.queue.push_back(Waiting { id, request });
        loop {
            let class = &mut state.classes[class_index];
            class.metrics.queued = class.queue.len();
            let check = read_cancelled(cancelled).and_then(|()| {
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    Err(Error::new("SOURCE_TIMEOUT", "等待来源资源超过截止时间"))
                } else {
                    Ok(())
                }
            });
            if let Err(error) = check {
                class.queue.retain(|w| w.id != id);
                class.metrics.queued = class.queue.len();
                class.metrics.cancelled_waiting += 1;
                self.inner.changed.notify_all();
                return Err(error);
            }
            // Four interactive admissions allow the oldest noninteractive request.
            // Do not skip a chosen large reservation: it must also eventually run.
            let chosen = if class.interactive_streak >= 4 {
                (!class.queue.is_empty()).then_some(0)
            } else {
                None
            }
            .or_else(|| {
                class
                    .queue
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, w)| (w.request.priority, w.id))
                    .map(|(i, _)| i)
            });
            if let Some(index) = chosen
                && class.queue[index].id == id
                && class.metrics.active < class.metrics.budget.concurrency
                && class.metrics.reserved_bytes + request.bytes <= class.metrics.budget.bytes
            {
                class.queue.remove(index);
                class.metrics.queued = class.queue.len();
                class.metrics.active += 1;
                class.metrics.reserved_bytes += request.bytes;
                class.metrics.peak_reserved_bytes = class
                    .metrics
                    .peak_reserved_bytes
                    .max(class.metrics.reserved_bytes);
                class.metrics.started += 1;
                let wait = millis(start);
                class.metrics.wait_ms += wait;
                class.metrics.max_wait_ms = class.metrics.max_wait_ms.max(wait);
                class.interactive_streak = if class.interactive_streak >= 4 {
                    0
                } else {
                    class.interactive_streak + 1
                };
                self.inner.changed.notify_all();
                return Ok(Box::new(Permit {
                    inner: self.inner.clone(),
                    class_index,
                    bytes: request.bytes,
                    start: Instant::now(),
                }));
            }
            state = self
                .inner
                .changed
                .wait_timeout(state, Duration::from_millis(20))
                .map_err(|_| lock_error())?
                .0;
        }
    }
    fn metrics(&self) -> Vec<ReadMetrics> {
        self.inner
            .state
            .lock()
            .map(|s| s.classes.iter().map(|c| c.metrics.clone()).collect())
            .unwrap_or_default()
    }
}
struct Permit {
    inner: Arc<Inner>,
    class_index: usize,
    bytes: u64,
    start: Instant,
}
impl ReadLease for Permit {}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.inner.state.lock() {
            let m = &mut state.classes[self.class_index].metrics;
            m.active -= 1;
            m.reserved_bytes -= self.bytes;
            m.completed += 1;
            m.work_ms += millis(self.start);
            self.inner.changed.notify_all();
        }
    }
}
