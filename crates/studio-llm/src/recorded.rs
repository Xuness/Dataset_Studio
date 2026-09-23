use crate::{protocols, runtime::RemoteLlm, transport::http};
use futures::{FutureExt, StreamExt, future::BoxFuture};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use studio_application::llm::*;
use studio_domain::llm::*;

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
    } else {
        serde_json::from_slice(&receipt.body)
            .map_err(|_| LlmFailure::new("LLM_INVALID_RESPONSE", "供应商未返回有效 JSON"))
            .and_then(|v| protocols::decode(plan, &v, receipt.provider_request_id.clone()))
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

impl RemoteLlm {
    pub(crate) fn recorded(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
        sink: Arc<dyn LlmReceiptSink>,
    ) -> BoxFuture<'_, LlmCallResult<LlmResponse>> {
        async move {
            // A ledger attempt is exactly one HTTP attempt. Never hide a 429 retry here.
            let attempted = Arc::new(AtomicBool::new(false));
            let deadline = tokio::time::Instant::now() + Duration::from_millis(plan.provider.config.network.request_timeout_ms.into());
            let response = {
                let operation = async {
                    let client = self.client(&plan.provider).await?;
                    let permit = self.limits.acquire(&plan.provider).await?;
                    let body = protocols::encode(&plan, false).map_err(|_| LlmFailure::new("LLM_CONFIGURATION", "无法编码调用请求"))?;
                    let url = http::endpoint(&plan.provider.config.base_url, &protocols::suffix(&plan, false))?;
                    permit.before_send(plan.provider.config.network.min_interval_ms).await?;
                    attempted.store(true, Ordering::Release);
                    let response = client.post(url).json(&body).send().await.map_err(http::network_error)?;
                    Ok((response, permit))
                };
                tokio::select! {
                    _ = cancel.cancelled() => return Err(crate::runtime::interrupted("LLM_CANCELLED", "调用已取消", &attempted)),
                    _ = tokio::time::sleep_until(deadline) => return Err(crate::runtime::interrupted("LLM_TIMEOUT", "调用总时限已到", &attempted)),
                    result = operation => result?,
                }
            };
            let (response, _permit) = response;
            let mut receipt = LlmRawReceipt {
                http_status: response.status().as_u16(),
                headers: ["content-type", "x-request-id", "request-id", "x-goog-request-id", "retry-after"]
                    .into_iter().filter_map(|k| response.headers().get(k).and_then(|v|v.to_str().ok()).filter(|v|v.len()<=2048).map(|v|(k.into(),v.into()))).collect(),
                provider_request_id: http::request_id(&response),
                protocol: plan.snapshot.protocol,
                adapter_version: "native_json_v1".into(), complete: false,
                failure: (!response.status().is_success()).then(||http::status_error(&response)), body: Vec::new(),
            };
            let mut stream = response.bytes_stream();
            loop {
                let next = tokio::select! {
                    _ = cancel.cancelled() => Err(crate::runtime::interrupted("LLM_CANCELLED", "响应接收被取消", &attempted)),
                    _ = tokio::time::sleep_until(deadline) => Err(crate::runtime::interrupted("LLM_TIMEOUT", "响应接收超时", &attempted)),
                    next = tokio::time::timeout(Duration::from_millis(plan.provider.config.network.idle_timeout_ms.into()), stream.next()) => match next {
                        Ok(Some(Ok(bytes))) => Ok(Some(bytes)),
                        Ok(Some(Err(error))) => Err(http::network_error(error)),
                        Ok(None) => Ok(None),
                        Err(_) => Err(crate::runtime::interrupted("LLM_TIMEOUT", "等待响应内容超时", &attempted)),
                    }
                };
                match next {
                    Ok(None) => { receipt.complete = true; break; },
                    Ok(Some(chunk)) => {
                        let remaining = LLM_RECEIPT_LIMIT - receipt.body.len();
                        receipt.body.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
                        if chunk.len() > remaining {
                            receipt.failure = Some(LlmFailure::new("LLM_RESPONSE_LIMIT", "供应商响应超过 16 MiB，已保留前缀")); break;
                        }
                    },
                    Err(error) => { receipt.failure = Some(error); break; },
                }
            }
            // Cancellation/timeouts stop network reads, never cancel an already received outcome's commit.
            sink.persist(receipt.clone()).await.map_err(|e| {
                let mut failure = LlmFailure::new(e.code, "原始回执尚未持久保存");
                failure.outcome_unknown = true; failure.provider_request_id = receipt.provider_request_id.clone(); failure
            })?;
            decode(&plan, &receipt)
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
