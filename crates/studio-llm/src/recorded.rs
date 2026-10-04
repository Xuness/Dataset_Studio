use crate::{
    protocols,
    runtime::RemoteLlm,
    transport::{http, sse::SseDecoder},
};
use futures::{FutureExt, StreamExt, future::BoxFuture};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use studio_application::llm::*;
use studio_domain::llm::*;
use tokio::time::Instant;

fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn validate(options: &LlmRecordedOptions) -> studio_domain::Result<()> {
    if !(100..=120_000).contains(&options.connect_timeout_ms)
        || !(100..=600_000).contains(&options.first_response_timeout_ms)
        || !(100..=600_000).contains(&options.idle_timeout_ms)
        || !(100..=3_600_000).contains(&options.request_timeout_ms)
        || options.first_response_timeout_ms > options.request_timeout_ms
    {
        return Err(studio_domain::Error::invalid(
            "连接、首包、空闲及请求总时限无效",
        ));
    }
    Ok(())
}
pub(crate) fn encode(
    plan: &LlmInvocationPlan,
    options: &LlmRecordedOptions,
) -> studio_domain::Result<serde_json::Value> {
    validate(options)?;
    let mut body = protocols::encode(plan, options.stream)?;
    if options.stream && plan.snapshot.protocol == LlmProtocol::OpenaiChat {
        body["stream_options"] = serde_json::json!({"include_usage":true});
    }
    Ok(body)
}
fn decode_sse(
    plan: &LlmInvocationPlan,
    body: &[u8],
    request_id: Option<String>,
) -> LlmCallResult<LlmResponse> {
    let mut decoder = SseDecoder::default();
    let mut collector = protocols::streaming::Collector::new(plan.clone());
    // Use the same bounded framing on live streams and offline replay.
    for bytes in body.chunks(16 * 1024) {
        for event in decoder.push(bytes)? {
            collector.push(&event)?;
            if collector.terminal && plan.snapshot.protocol != LlmProtocol::Gemini {
                return collector.finish(request_id);
            }
        }
    }
    decoder.finish()?;
    collector.finish(request_id)
}
pub(crate) fn decode(
    plan: &LlmInvocationPlan,
    receipt: &LlmRawReceipt,
) -> LlmCallResult<LlmResponse> {
    let mut result = if !receipt.complete {
        Err(receipt
            .failure
            .clone()
            .unwrap_or_else(|| LlmFailure::new("LLM_INVALID_RESPONSE", "原始回执不完整")))
    } else if let Some(error) = &receipt.failure {
        Err(error.clone())
    } else if receipt.protocol != plan.snapshot.protocol {
        Err(LlmFailure::new(
            "LLM_CONFIGURATION",
            "原始回执协议与冻结配置不一致",
        ))
    } else if receipt.adapter_version == "native_sse_v1" {
        decode_sse(plan, &receipt.body, receipt.provider_request_id.clone())
    } else if receipt.adapter_version == "native_json_v1" {
        serde_json::from_slice(&receipt.body)
            .map_err(|_| LlmFailure::new("LLM_INVALID_RESPONSE", "供应商未返回有效 JSON"))
            .and_then(|v| protocols::decode(plan, &v, receipt.provider_request_id.clone()))
    } else {
        Err(LlmFailure::new(
            "LLM_CONFIGURATION",
            "此回执的解析版本不受支持",
        ))
    };
    if let Err(error) = &mut result {
        error.provider_request_id = receipt.provider_request_id.clone();
        error.http_status = Some(receipt.http_status);
        if (200..300).contains(&receipt.http_status) {
            error.outcome_unknown = true;
        }
    }
    result
}
fn interrupted(code: &str, message: &str) -> LlmFailure {
    let mut failure = LlmFailure::new(code, message);
    failure.outcome_unknown = true;
    failure
}
impl RemoteLlm {
    async fn recorded_client(
        &self,
        provider: &LlmProvider,
        connect_timeout_ms: u32,
    ) -> LlmCallResult<reqwest::Client> {
        let key = (provider.id.clone(), provider.revision, connect_timeout_ms);
        if let Some(client) = self
            .recorded_clients
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
        let mut provider = provider.clone();
        provider.config.network.connect_timeout_ms = connect_timeout_ms;
        let client = http::recorded_client(&provider, secret.as_ref())?;
        let mut clients = self
            .recorded_clients
            .lock()
            .map_err(|_| LlmFailure::new("LLM_INTERNAL", "连接池不可用"))?;
        clients.retain(|(id, revision, _), _| id != &provider.id || *revision == provider.revision);
        if clients.len() >= 256 {
            clients.clear();
        }
        clients.insert(key, client.clone());
        Ok(client)
    }
    pub(crate) fn recorded(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
        sink: Arc<dyn LlmReceiptSink>,
    ) -> BoxFuture<'_, LlmCallResult<LlmResponse>> {
        let n = &plan.provider.config.network;
        let options = LlmRecordedOptions {
            stream: false,
            connect_timeout_ms: n.connect_timeout_ms,
            first_response_timeout_ms: n.idle_timeout_ms.min(n.request_timeout_ms),
            idle_timeout_ms: n.idle_timeout_ms,
            request_timeout_ms: n.request_timeout_ms,
        };
        self.recorded_with_options(plan, cancel, sink, options)
    }
    pub(crate) fn recorded_with_options(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
        sink: Arc<dyn LlmReceiptSink>,
        options: LlmRecordedOptions,
    ) -> BoxFuture<'_, LlmCallResult<LlmResponse>> {
        async move {
            let body=encode(&plan,&options).map_err(|e|LlmFailure::new(e.code,&e.message))?;
            let mut url=http::endpoint(&plan.provider.config.base_url,&protocols::suffix(&plan,options.stream))?;
            if options.stream && plan.snapshot.protocol==LlmProtocol::Gemini {url.query_pairs_mut().append_pair("alt","sse");}
            sink.progress(LlmTransferProgress{phase:"queueing",started_at_ms:None,last_data_at_ms:None,received_bytes:0});
            // Local concurrency/rate-limit waits have no network deadline and are cancellable.
            let admission=async {
                let client=self.recorded_client(&plan.provider,options.connect_timeout_ms).await?;
                let permit=self.limits.acquire(&plan.provider).await?;
                permit.before_send(plan.provider.config.network.min_interval_ms).await?;
                Ok::<_,LlmFailure>((client,permit))
            };
            tokio::pin!(admission);
            let (client,_permit)=loop {
                if sink.dispatch_cancelled() {return Err(LlmFailure::new("LLM_NOT_DISPATCHED","阶段暂停派发，尚未发送"));}
                tokio::select! {
                    _=cancel.cancelled()=>return Err(LlmFailure::new("LLM_NOT_DISPATCHED","本地排队已取消，尚未发送")),
                    result=&mut admission=>break result?,
                    _=tokio::time::sleep(Duration::from_millis(100))=>{},
                }
            };
            let decision=sink.before_send().await.map_err(|e|LlmFailure::new(e.code,&e.message))?;
            if decision.defer {return Err(LlmFailure::new("LLM_NOT_DISPATCHED","阶段已停靠或批次恢复预算已结束，尚未发送"));}
            let started_at=millis();
            let allowed=u64::from(options.request_timeout_ms).min(decision.recovery_deadline_ms.map_or(u64::MAX,|d|d.saturating_sub(started_at)));
            // An exhausted recovery budget never purchases a new request.
            if allowed==0 {return Err(LlmFailure::new("LLM_NOT_DISPATCHED","批次恢复时限已到，尚未发送"));}
            let started=Instant::now();
            let deadline=started+Duration::from_millis(allowed);
            let first_deadline=deadline.min(started+Duration::from_millis(options.first_response_timeout_ms.into()));
            sink.progress(LlmTransferProgress{phase:"waiting_response",started_at_ms:Some(started_at),last_data_at_ms:None,received_bytes:0});
            let response=tokio::select! {
                biased;
                _=cancel.cancelled()=>return Err(interrupted("LLM_CANCELLED","调用已取消")),
                _=tokio::time::sleep_until(first_deadline)=>return Err(interrupted("LLM_TIMEOUT",if first_deadline==deadline {"调用总时限已到"}else{"等待首个响应超时"})),
                response=client.post(url).json(&body).send()=>response.map_err(http::network_error)?,
            };
            let content_type=response.headers().get("content-type").and_then(|v|v.to_str().ok()).unwrap_or("").to_owned();
            let mut receipt=LlmRawReceipt {
                http_status:response.status().as_u16(),
                headers:["content-type","x-request-id","request-id","x-goog-request-id","x-generation-id","retry-after"].into_iter()
                    .filter_map(|k|response.headers().get(k).and_then(|v|v.to_str().ok()).filter(|v|v.len()<=2048).map(|v|(k.into(),v.into()))).collect(),
                provider_request_id:http::request_id(&response),protocol:plan.snapshot.protocol,
                adapter_version:if options.stream {"native_sse_v1"}else{"native_json_v1"}.into(),
                complete:false,failure:(!response.status().is_success()).then(||http::status_error(&response)),body:Vec::new(),
            };
            let use_sse=options.stream && receipt.failure.is_none();
            if use_sse && !content_type.starts_with("text/event-stream") {
                receipt.failure=Some(LlmFailure::new("LLM_INVALID_STREAM","供应商未返回 SSE 流；已保留回执，未自动重发"));
            }
            let mut bytes=response.bytes_stream();let mut first=true;
            let mut decoder=SseDecoder::default();let mut collector=protocols::streaming::Collector::new(plan.clone());
            let mut phase=if use_sse {"waiting_result"}else{"receiving"};
            loop {
                let read_deadline=if first {first_deadline}else{deadline.min(Instant::now()+Duration::from_millis(options.idle_timeout_ms.into()))};
                let next=tokio::select! {
                    biased;
                    _=cancel.cancelled()=>Err(interrupted("LLM_CANCELLED","响应接收被取消")),
                    _=tokio::time::sleep_until(read_deadline)=>Err(interrupted("LLM_TIMEOUT",if read_deadline==deadline {"调用总时限已到"}else if first {"等待首个响应数据超时"}else{"等待后续响应数据超时"})),
                    chunk=bytes.next()=>match chunk {Some(Ok(v))=>Ok(Some(v)),Some(Err(e))=>Err(http::network_error(e)),None=>Ok(None)},
                };
                match next {
                    Ok(None)=>{receipt.complete=true;break;},
                    Ok(Some(chunk))=>{
                        if chunk.is_empty(){continue;}
                        first=false;
                        let remaining=LLM_RECEIPT_LIMIT.saturating_sub(receipt.body.len());
                        receipt.body.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
                        if chunk.len()>remaining {receipt.failure=Some(interrupted("LLM_RESPONSE_LIMIT","响应超过 16 MiB，已保留前缀"));break;}
                        if use_sse && receipt.failure.is_none() {
                            let events=(|| {
                                for data in decoder.push(&chunk)? {
                                    for event in collector.push(&data)? {
                                        if let LlmEvent::Delta{kind,..}=event {phase=if kind=="reasoning" {"reasoning"}else{"generating"};}
                                    }
                                    if collector.terminal && plan.snapshot.protocol!=LlmProtocol::Gemini {break;}
                                }
                                Ok::<_,LlmFailure>(())
                            })();
                            if let Err(error)=events {receipt.failure=Some(error);break;}
                        }
                        sink.progress(LlmTransferProgress{phase,started_at_ms:Some(started_at),last_data_at_ms:Some(millis()),received_bytes:receipt.body.len() as u64});
                        if use_sse && collector.terminal && plan.snapshot.protocol!=LlmProtocol::Gemini {receipt.complete=true;break;}
                    },
                    Err(error)=>{receipt.failure=Some(error);break;},
                }
            }
            // Cancellation ends network reads, never the durable commit of received evidence.
            sink.progress(LlmTransferProgress{phase:"saving",started_at_ms:Some(started_at),last_data_at_ms:None,received_bytes:receipt.body.len() as u64});
            sink.persist(receipt.clone()).await.map_err(|e| {
                let mut f=LlmFailure::new(e.code,"原始回执尚未持久保存");f.outcome_unknown=true;f.provider_request_id=receipt.provider_request_id.clone();f
            })?;
            decode(&plan,&receipt)
        }.boxed()
    }
}
pub(crate) fn reparse(
    snapshot: &LlmInvocationSnapshot,
    receipt: &LlmRawReceipt,
) -> LlmCallResult<LlmResponse> {
    let plan = LlmInvocationPlan {
        snapshot: snapshot.clone(),
        provider: LlmProvider {
            id: snapshot.provider_id.clone(),
            revision: snapshot.provider_revision,
            credential_ref: None,
            config: LlmConnectionConfig {
                name: "offline receipt parser".into(),
                kind: snapshot.provider_kind,
                base_url: snapshot.base_url.clone(),
                enabled: false,
                headers: Default::default(),
                network: Default::default(),
            },
        },
    };
    decode(&plan, receipt)
}
