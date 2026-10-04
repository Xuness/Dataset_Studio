use futures::StreamExt;
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use std::time::Duration;
use studio_application::llm::{LlmCallResult, LlmSecret};
use studio_domain::{Error, Result, llm::*};

pub fn validate(config: &LlmConnectionConfig) -> Result<()> {
    base_url(&config.base_url)?;
    let n = &config.network;
    if !(100..=120_000).contains(&n.connect_timeout_ms)
        || !(100..=3_600_000).contains(&n.request_timeout_ms)
        || !(100..=600_000).contains(&n.idle_timeout_ms)
        || !(1..=32).contains(&n.max_concurrency)
        || n.min_interval_ms > 60_000
        || n.rate_limit_retries > 3
    {
        return Err(Error::invalid("连接超时、调用超时、并发或重试设置超出范围"));
    }
    if let Some(proxy) = &n.proxy_url
        && !proxy.is_empty()
    {
        base_url(proxy)?;
    }
    if config.headers.len() > 16 {
        return Err(Error::invalid("自定义请求头最多 16 项"));
    }
    for (key, value) in &config.headers {
        let lower = key.to_ascii_lowercase();
        if lower.contains("key")
            || lower.contains("token")
            || lower.contains("authorization")
            || [
                "cookie",
                "host",
                "content-type",
                "content-length",
                "connection",
                "transfer-encoding",
                "proxy-connection",
                "set-cookie",
            ]
            .contains(&lower.as_str())
            || value.len() > 2048
        {
            return Err(Error::invalid("认证及传输控制请求头不能通过普通配置设置"));
        }
        HeaderName::from_bytes(key.as_bytes()).map_err(|_| Error::invalid("请求头名称无效"))?;
        HeaderValue::from_str(value).map_err(|_| Error::invalid("请求头值无效"))?;
    }
    Ok(())
}
pub fn base_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| Error::invalid("API 地址无效"))?;
    if !["https", "http"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::invalid(
            "地址须为 HTTP(S)，不得包含凭据、查询参数或片段",
        ));
    }
    Ok(url)
}
pub fn endpoint(base: &str, suffix: &str) -> LlmCallResult<Url> {
    let mut url =
        base_url(base).map_err(|_| LlmFailure::new("LLM_CONFIGURATION", "API 地址无效"))?;
    let path = format!(
        "{}/{}",
        url.path().trim_end_matches('/'),
        suffix.trim_start_matches('/')
    );
    url.set_path(&path);
    Ok(url)
}
pub fn client(provider: &LlmProvider, secret: Option<&LlmSecret>) -> LlmCallResult<Client> {
    build_client(provider, secret, true)
}
/// Recorded requests enforce separate first-response/idle deadlines in their read loop.
pub fn recorded_client(
    provider: &LlmProvider,
    secret: Option<&LlmSecret>,
) -> LlmCallResult<Client> {
    build_client(provider, secret, false)
}
fn build_client(
    provider: &LlmProvider,
    secret: Option<&LlmSecret>,
    read_timeout: bool,
) -> LlmCallResult<Client> {
    validate(&provider.config).map_err(|_| LlmFailure::new("LLM_CONFIGURATION", "连接配置无效"))?;
    let mut headers = HeaderMap::new();
    for (key, value) in &provider.config.headers {
        headers.insert(
            HeaderName::from_bytes(key.as_bytes())
                .map_err(|_| LlmFailure::new("LLM_CONFIGURATION", "请求头无效"))?,
            HeaderValue::from_str(value)
                .map_err(|_| LlmFailure::new("LLM_CONFIGURATION", "请求头无效"))?,
        );
    }
    if let Some(secret) = secret {
        let (name, value) = if provider.config.kind == LlmProviderKind::Gemini {
            ("x-goog-api-key", secret.expose().to_owned())
        } else {
            ("authorization", format!("Bearer {}", secret.expose()))
        };
        let mut value = HeaderValue::from_str(&value)
            .map_err(|_| LlmFailure::new("LLM_CREDENTIAL_UNAVAILABLE", "API Key 无法用于请求头"))?;
        value.set_sensitive(true);
        headers.insert(HeaderName::from_static(name), value);
    }
    let n = &provider.config.network;
    let mut builder = Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_millis(n.connect_timeout_ms.into()))
        .pool_max_idle_per_host(n.max_concurrency as usize)
        .redirect(reqwest::redirect::Policy::none());
    if read_timeout {
        builder = builder.read_timeout(Duration::from_millis(n.idle_timeout_ms.into()));
    }
    if let Some(proxy) = &n.proxy_url {
        builder = if proxy.is_empty() {
            builder.no_proxy()
        } else {
            builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|_| LlmFailure::new("LLM_CONFIGURATION", "代理地址无效"))?,
            )
        };
    }
    builder
        .build()
        .map_err(|_| LlmFailure::new("LLM_CONFIGURATION", "无法创建 HTTP 客户端"))
}
pub fn network_error(error: reqwest::Error) -> LlmFailure {
    let mut result = LlmFailure::new(
        if error.is_timeout() {
            "LLM_TIMEOUT"
        } else {
            "LLM_NETWORK"
        },
        "供应商连接失败或中断",
    );
    result.outcome_unknown = !error.is_connect();
    result.retryable = error.is_connect();
    result
}
pub fn request_id(response: &reqwest::Response) -> Option<String> {
    [
        "x-request-id",
        "request-id",
        "x-goog-request-id",
        "x-generation-id",
    ]
    .iter()
    .find_map(|name| {
        response
            .headers()
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .filter(|s| s.len() <= 256)
            .map(str::to_owned)
    })
}
pub fn status_error(response: &reqwest::Response) -> LlmFailure {
    let status = response.status().as_u16();
    let (code, message) = match status {
        401 | 403 => ("LLM_AUTHENTICATION", "供应商拒绝认证或账号无权限"),
        413 => (
            "LLM_REQUEST_TOO_LARGE",
            "供应商拒绝请求体大小，请降低此端点的阶段预算",
        ),
        404 => ("LLM_NOT_FOUND", "供应商模型或接口不存在"),
        429 => ("LLM_RATE_LIMITED", "供应商限流或额度不足"),
        400 | 422 => (
            "LLM_INVALID_REQUEST",
            "供应商拒绝请求，请检查模型与参数支持情况",
        ),
        300..=399 => ("LLM_REDIRECT", "供应商返回重定向，请直接配置最终 API 地址"),
        _ => ("LLM_UPSTREAM", "供应商返回服务错误"),
    };
    LlmFailure {
        code: code.into(),
        message: message.into(),
        http_status: Some(status),
        provider_request_id: request_id(response),
        retryable: status == 429,
        outcome_unknown: status >= 500,
    }
}
pub async fn json(response: reqwest::Response, idle_ms: u32) -> LlmCallResult<serde_json::Value> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::time::timeout(Duration::from_millis(idle_ms.into()), stream.next())
            .await
            .map_err(|_| LlmFailure::new("LLM_TIMEOUT", "等待响应内容超时"))?;
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(network_error)?;
        if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
            return Err(LlmFailure::new(
                "LLM_RESPONSE_LIMIT",
                "供应商响应超过 16 MiB",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| LlmFailure::new("LLM_INVALID_RESPONSE", "供应商未返回有效 JSON"))
}
