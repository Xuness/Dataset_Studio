use futures::{
    FutureExt,
    channel::oneshot,
    future::{BoxFuture, Shared},
    stream::BoxStream,
};
use std::sync::{Arc, Mutex};
use studio_domain::{Result, llm::*};

pub struct LlmSecret(zeroize::Zeroizing<String>);
impl LlmSecret {
    pub fn new(value: String) -> Self {
        Self(zeroize::Zeroizing::new(value))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

pub trait LlmCredentials: Send + Sync {
    fn put(&self, secret: LlmSecret) -> Result<String>;
    fn get(&self, reference: &str) -> Result<LlmSecret>;
    fn remove(&self, reference: &str) -> Result<()>;
}

pub trait LlmRepository: Send + Sync {
    fn providers(&self) -> Result<Vec<LlmProvider>>;
    fn provider(&self, id: &str) -> Result<LlmProvider>;
    fn save_provider(&self, value: LlmProvider, expected_revision: u64) -> Result<LlmProvider>;
    fn remove_provider(&self, id: &str, expected_revision: u64) -> Result<()>;
    fn models(&self, provider_id: &str) -> Result<Vec<LlmModel>>;
    fn model(&self, id: &str) -> Result<LlmModel>;
    fn save_model(&self, value: LlmModel, expected_revision: u64) -> Result<LlmModel>;
    fn remove_model(&self, id: &str, expected_revision: u64) -> Result<()>;
    fn presets(&self) -> Result<Vec<LlmPreset>>;
    fn preset(&self, id: &str) -> Result<LlmPreset>;
    fn save_preset(&self, value: LlmPreset, expected_revision: u64) -> Result<LlmPreset>;
    fn remove_preset(&self, id: &str, expected_revision: u64) -> Result<()>;
    fn system_prompts(&self) -> Result<Vec<LlmSystemPrompt>>;
    fn system_prompt(&self, id: &str) -> Result<LlmSystemPrompt>;
    fn save_system_prompt(
        &self,
        value: LlmSystemPrompt,
        expected_revision: u64,
    ) -> Result<LlmSystemPrompt>;
    fn remove_system_prompt(&self, id: &str, expected_revision: u64) -> Result<()>;
    fn catalog(&self, provider_id: &str) -> Result<Option<LlmCatalog>>;
    fn save_catalog(&self, catalog: &LlmCatalog) -> Result<()>;
}

/// Multiple waiters are supported, and cancellation is retained for late subscribers.
#[derive(Clone)]
pub struct LlmCancellation {
    sender: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    notified: Shared<BoxFuture<'static, ()>>,
}
impl Default for LlmCancellation {
    fn default() -> Self {
        let (sender, receiver) = oneshot::channel();
        Self {
            sender: Arc::new(Mutex::new(Some(sender))),
            notified: async move {
                let _ = receiver.await;
            }
            .boxed()
            .shared(),
        }
    }
}
impl LlmCancellation {
    pub fn cancel(&self) {
        if let Ok(mut sender) = self.sender.lock()
            && let Some(sender) = sender.take()
        {
            let _ = sender.send(());
        }
    }
    pub async fn cancelled(&self) {
        self.notified.clone().await;
    }
}

pub type LlmCallResult<T> = std::result::Result<T, LlmFailure>;
pub trait LlmReceiptSink: Send + Sync {
    /// Admission and the durable network-attempt record happen after local queueing.
    fn before_send(&self) -> BoxFuture<'_, Result<LlmDispatchDecision>> {
        async { Ok(LlmDispatchDecision::default()) }.boxed()
    }
    /// Ephemeral UI telemetry. Never acknowledges evidence or writes every token.
    fn progress(&self, _progress: LlmTransferProgress) {}
    /// Pause only unsent admission; paid responses continue draining and are persisted.
    fn dispatch_cancelled(&self) -> bool {
        false
    }
    /// Must acknowledge durable storage before provider parsing starts.
    fn persist(&self, receipt: LlmRawReceipt) -> BoxFuture<'_, Result<()>>;
}
pub trait LlmBackend: Send + Sync {
    fn validate_connection(&self, config: &LlmConnectionConfig) -> Result<()>;
    /// Validates native representation without authentication or network activity.
    fn preview(&self, plan: &LlmInvocationPlan) -> Result<serde_json::Value>;
    fn discover(
        &self,
        provider: LlmProvider,
        cancel: LlmCancellation,
    ) -> BoxFuture<'_, LlmCallResult<LlmCatalog>>;
    fn generate(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
    ) -> BoxFuture<'_, LlmCallResult<LlmResponse>>;
    fn stream(&self, plan: LlmInvocationPlan, cancel: LlmCancellation) -> BoxStream<'_, LlmEvent>;
    fn generate_recorded(
        &self,
        _plan: LlmInvocationPlan,
        _cancel: LlmCancellation,
        _sink: Arc<dyn LlmReceiptSink>,
    ) -> BoxFuture<'_, LlmCallResult<LlmResponse>> {
        async {
            Err(LlmFailure::new(
                "LLM_CONFIGURATION",
                "此后端不支持原始回执保全",
            ))
        }
        .boxed()
    }
    fn preview_recorded(
        &self,
        plan: &LlmInvocationPlan,
        options: &LlmRecordedOptions,
    ) -> Result<serde_json::Value> {
        if options.stream {
            return Err(studio_domain::Error::invalid("此后端不支持流式回执保全"));
        }
        self.preview(plan)
    }
    fn generate_recorded_with_options(
        &self,
        _plan: LlmInvocationPlan,
        _cancel: LlmCancellation,
        _sink: Arc<dyn LlmReceiptSink>,
        _options: LlmRecordedOptions,
    ) -> BoxFuture<'_, LlmCallResult<LlmResponse>> {
        async {
            Err(LlmFailure::new(
                "LLM_CONFIGURATION",
                "此后端不支持阶段传输设置",
            ))
        }
        .boxed()
    }
    fn reparse(
        &self,
        _plan: &LlmInvocationSnapshot,
        _receipt: &LlmRawReceipt,
    ) -> LlmCallResult<LlmResponse> {
        Err(LlmFailure::new(
            "LLM_CONFIGURATION",
            "此后端不支持本地重解析",
        ))
    }
}
