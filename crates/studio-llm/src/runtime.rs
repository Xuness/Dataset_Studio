use crate::{
    protocols,
    transport::{http, limits::Limits, sse::SseDecoder},
};
use futures::{FutureExt, StreamExt, future::BoxFuture, stream::BoxStream};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use studio_application::llm::*;
use studio_domain::{Result, llm::*};

pub struct RemoteLlm {
    pub(crate) credentials: Arc<dyn LlmCredentials>,
    pub(crate) limits: Limits,
    clients: Mutex<HashMap<(String, u64), reqwest::Client>>,
    pub(crate) recorded_clients: Mutex<HashMap<(String, u64, u32), reqwest::Client>>,
}
impl RemoteLlm {
    pub fn new(credentials: Arc<dyn LlmCredentials>) -> Self {
        Self {
            credentials,
            limits: Limits::default(),
            clients: Mutex::new(HashMap::new()),
            recorded_clients: Mutex::new(HashMap::new()),
        }
    }
    pub(crate) async fn client(&self, provider: &LlmProvider) -> LlmCallResult<reqwest::Client> {
        let key = (provider.id.clone(), provider.revision);
        if let Some(client) = self
            .clients
            .lock()
            .map_err(|_| LlmFailure::new("LLM_INTERNAL", "连接池不可用"))?
            .get(&key)
            .cloned()
        {
            return Ok(client);
        }
        let vault = self.credentials.clone();
        let reference = provider.credential_ref.clone();
        let secret =
            tokio::task::spawn_blocking(move || reference.map(|id| vault.get(&id)).transpose())
                .await
                .map_err(|_| LlmFailure::new("LLM_CREDENTIAL_UNAVAILABLE", "无法读取凭据"))?
                .map_err(|_| {
                    LlmFailure::new(
                        "LLM_CREDENTIAL_UNAVAILABLE",
                        "凭据不可用，请重新设置 API Key",
                    )
                })?;
        let client = http::client(provider, secret.as_ref())?;
        let mut clients = self
            .clients
            .lock()
            .map_err(|_| LlmFailure::new("LLM_INTERNAL", "连接池不可用"))?;
        clients.retain(|(id, revision), _| id != &provider.id || *revision == provider.revision);
        if clients.len() >= 256 {
            clients.clear();
        }
        clients.insert(key, client.clone());
        Ok(client)
    }
    async fn send(
        &self,
        client: &reqwest::Client,
        provider: &LlmProvider,
        url: reqwest::Url,
        body: &Value,
        attempted: &AtomicBool,
        permit: &crate::transport::limits::Permit,
    ) -> LlmCallResult<reqwest::Response> {
        for attempt in 0..=provider.config.network.rate_limit_retries {
            permit
                .before_send(provider.config.network.min_interval_ms)
                .await?;
            attempted.store(true, Ordering::Release);
            let response = client
                .post(url.clone())
                .json(body)
                .send()
                .await
                .map_err(http::network_error)?;
            if response.status().is_success() {
                return Ok(response);
            }
            let error = http::status_error(&response);
            if response.status().as_u16() != 429
                || attempt == provider.config.network.rate_limit_retries
            {
                return Err(error);
            }
            let delay = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1 << attempt);
            if delay > 60 {
                return Err(error);
            }
            drop(response);
            tokio::time::sleep(Duration::from_secs(delay.max(1))).await;
        }
        Err(LlmFailure::new("LLM_INTERNAL", "重试状态无效"))
    }
    fn stream_inner<'a>(
        &'a self,
        plan: LlmInvocationPlan,
        attempted: Arc<AtomicBool>,
    ) -> BoxStream<'a, LlmCallResult<LlmEvent>> {
        Box::pin(async_stream::try_stream! {
            let client = self.client(&plan.provider).await?;
            let permit = self.limits.acquire(&plan.provider).await?;
            let body = protocols::encode(&plan,true).map_err(|_| LlmFailure::new("LLM_CONFIGURATION", "无法编码调用请求"))?;
            let mut url = http::endpoint(&plan.provider.config.base_url,&protocols::suffix(&plan,true))?;
            if plan.snapshot.protocol == LlmProtocol::Gemini { url.query_pairs_mut().append_pair("alt","sse"); }
            let response = self.send(&client,&plan.provider,url,&body,&attempted,&permit).await?;
            if !response.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/event-stream")) {
                Err(LlmFailure::new("LLM_INVALID_STREAM", "供应商未返回 SSE 流"))?;
            }
            let request_id = http::request_id(&response);
            let mut bytes = response.bytes_stream();
            let idle = Duration::from_millis(plan.provider.config.network.idle_timeout_ms.into());
            let mut decoder = SseDecoder::default();
            let mut collector = protocols::streaming::Collector::new(plan.clone());
            let mut total = 0usize;
            while let Some(chunk) = tokio::time::timeout(idle, bytes.next()).await.map_err(|_| LlmFailure::new("LLM_TIMEOUT", "等待流式内容超时"))? {
                let chunk = chunk.map_err(http::network_error)?;
                total += chunk.len();
                if total > 32 * 1024 * 1024 { Err(LlmFailure::new("LLM_RESPONSE_LIMIT", "流式响应超过 32 MiB"))?; }
                for data in decoder.push(&chunk)? { for event in collector.push(&data)? { yield event; } }
                if collector.terminal && plan.snapshot.protocol != LlmProtocol::Gemini { break; }
            }
            decoder.finish()?;
            yield LlmEvent::Completed { response: Box::new(collector.finish(request_id)?) };
        })
    }
}
impl LlmBackend for RemoteLlm {
    fn preview_recorded(
        &self,
        plan: &LlmInvocationPlan,
        options: &LlmRecordedOptions,
    ) -> Result<Value> {
        crate::recorded::encode(plan, options)
    }
    fn generate_recorded_with_options(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
        sink: Arc<dyn LlmReceiptSink>,
        options: LlmRecordedOptions,
    ) -> BoxFuture<'_, LlmCallResult<LlmResponse>> {
        self.recorded_with_options(plan, cancel, sink, options)
    }
    fn generate_recorded(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
        sink: Arc<dyn LlmReceiptSink>,
    ) -> BoxFuture<'_, LlmCallResult<LlmResponse>> {
        self.recorded(plan, cancel, sink)
    }
    fn reparse(
        &self,
        plan: &LlmInvocationSnapshot,
        receipt: &LlmRawReceipt,
    ) -> LlmCallResult<LlmResponse> {
        crate::recorded::reparse(plan, receipt)
    }
    fn validate_connection(&self, config: &LlmConnectionConfig) -> Result<()> {
        http::validate(config)
    }
    fn preview(&self, plan: &LlmInvocationPlan) -> Result<Value> {
        protocols::encode(plan, false)
    }
    fn discover(
        &self,
        provider: LlmProvider,
        cancel: LlmCancellation,
    ) -> BoxFuture<'_, LlmCallResult<LlmCatalog>> {
        async move {
            let total_timeout = provider.config.network.request_timeout_ms;
            let operation = async {
                if !provider.config.enabled {
                    return Err(LlmFailure::new("LLM_DISABLED", "供应商连接已停用"));
                }
                let client = self.client(&provider).await?;
                let permit = self.limits.acquire(&provider).await?;
                let mut models = Vec::new();
                let mut seen = BTreeSet::new();
                let mut pages = BTreeSet::new();
                let mut next: Option<String> = None;
                for _ in 0..100 {
                    let mut url = http::endpoint(&provider.config.base_url, "models")?;
                    if provider.config.kind == LlmProviderKind::Gemini {
                        url.query_pairs_mut().append_pair("pageSize", "1000");
                        if let Some(next) = &next {
                            url.query_pairs_mut().append_pair("pageToken", next);
                        }
                    } else if let Some(next) = &next {
                        url.query_pairs_mut().append_pair("after", next);
                    }
                    permit
                        .before_send(provider.config.network.min_interval_ms)
                        .await?;
                    let response = client.get(url).send().await.map_err(http::network_error)?;
                    if !response.status().is_success() {
                        return Err(http::status_error(&response));
                    }
                    let value =
                        http::json(response, provider.config.network.idle_timeout_ms).await?;
                    let (items, cursor) = protocols::catalog::page(provider.config.kind, &value)?;
                    for model in items {
                        if seen.insert(model.id.clone()) {
                            models.push(model);
                        }
                    }
                    if models.len() > 10000 {
                        return Err(LlmFailure::new(
                            "LLM_RESPONSE_LIMIT",
                            "模型目录超过 10000 项",
                        ));
                    }
                    match cursor {
                        Some(cursor) if !cursor.is_empty() => {
                            if !pages.insert(cursor.clone()) {
                                return Err(LlmFailure::new(
                                    "LLM_INVALID_RESPONSE",
                                    "模型目录分页游标重复",
                                ));
                            }
                            next = Some(cursor);
                        }
                        _ => {
                            return Ok(LlmCatalog {
                                provider_id: provider.id.clone(),
                                provider_revision: provider.revision,
                                fetched_at: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis()
                                    .to_string(),
                                models,
                            });
                        }
                    }
                }
                Err(LlmFailure::new(
                    "LLM_RESPONSE_LIMIT",
                    "模型目录分页超过上限",
                ))
            };
            bounded(
                operation,
                cancel,
                total_timeout,
                Arc::new(AtomicBool::new(false)),
            )
            .await
        }
        .boxed()
    }
    fn generate(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
    ) -> BoxFuture<'_, LlmCallResult<LlmResponse>> {
        async move {
            let total_timeout = plan.provider.config.network.request_timeout_ms;
            let attempted = Arc::new(AtomicBool::new(false));
            let operation = async {
                let client = self.client(&plan.provider).await?;
                let permit = self.limits.acquire(&plan.provider).await?;
                let body = protocols::encode(&plan, false)
                    .map_err(|_| LlmFailure::new("LLM_CONFIGURATION", "无法编码调用请求"))?;
                let url = http::endpoint(
                    &plan.provider.config.base_url,
                    &protocols::suffix(&plan, false),
                )?;
                let response = self
                    .send(&client, &plan.provider, url, &body, &attempted, &permit)
                    .await?;
                let request_id = http::request_id(&response);
                let value = http::json(response, plan.provider.config.network.idle_timeout_ms)
                    .await
                    .map_err(|mut error| {
                        error.outcome_unknown = true;
                        error.provider_request_id = request_id.clone();
                        error
                    })?;
                protocols::decode(&plan, &value, request_id.clone()).map_err(|mut error| {
                    error.outcome_unknown = true;
                    error.provider_request_id = request_id;
                    error
                })
            };
            bounded(operation, cancel, total_timeout, attempted.clone()).await
        }
        .boxed()
    }
    fn stream(&self, plan: LlmInvocationPlan, cancel: LlmCancellation) -> BoxStream<'_, LlmEvent> {
        Box::pin(async_stream::stream! {
            let deadline = tokio::time::sleep(Duration::from_millis(plan.provider.config.network.request_timeout_ms.into()));
            tokio::pin!(deadline);
            let attempted = Arc::new(AtomicBool::new(false));
            yield LlmEvent::Started { invocation_id: plan.snapshot.invocation_id.clone() };
            let mut stream = self.stream_inner(plan,attempted.clone());
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => { yield LlmEvent::Failed { error: interrupted("LLM_CANCELLED","调用已取消",&attempted) }; break; },
                    _ = &mut deadline => { yield LlmEvent::Failed { error: interrupted("LLM_TIMEOUT","调用总时限已到",&attempted) }; break; },
                    value = stream.next() => match value {
                        Some(Ok(event)) => { let terminal = matches!(event,LlmEvent::Completed { .. }); yield event; if terminal { break; } },
                        Some(Err(mut error)) => { if attempted.load(Ordering::Acquire) && error.http_status.is_none() { error.outcome_unknown = true; } yield LlmEvent::Failed { error }; break; },
                        None => { yield LlmEvent::Failed { error: interrupted("LLM_STREAM_INTERRUPTED","流式响应未完成",&attempted) }; break; },
                    }
                }
            }
        })
    }
}
pub(crate) fn interrupted(code: &str, message: &str, attempted: &AtomicBool) -> LlmFailure {
    let mut error = LlmFailure::new(code, message);
    error.outcome_unknown = attempted.load(Ordering::Acquire);
    error
}
async fn bounded<T>(
    future: impl std::future::Future<Output = LlmCallResult<T>>,
    cancel: LlmCancellation,
    timeout: u32,
    attempted: Arc<AtomicBool>,
) -> LlmCallResult<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(interrupted("LLM_CANCELLED","调用已取消",&attempted)),
        result = tokio::time::timeout(Duration::from_millis(timeout.into()),future) => result.unwrap_or_else(|_| Err(interrupted("LLM_TIMEOUT","调用总时限已到",&attempted))),
    }
}
