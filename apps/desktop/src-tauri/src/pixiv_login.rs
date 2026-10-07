//! Owned, in-private login window. Secrets move from the system WebView to the loopback
//! engine, never through the renderer. Only the main application may use IPC.
use serde::de::DeserializeOwned;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use studio_protocol::{
    CollectionAccountMode, CollectionAccountProbe, CollectionCookie, CollectionLoginError,
    CollectionLoginPhase as Phase, CollectionLoginSession, CollectionLoginStatus, EngineConnection,
    SaveCollectionAccount, StartCollectionLogin,
};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::{Mutex, Semaphore};
use url::Url;
use uuid::Uuid;

const LOGIN_URL: &str = "https://accounts.pixiv.net/login?return_to=https%3A%2F%2Fwww.pixiv.net%2F";
const WINDOW_PREFIX: &str = "pixiv-login-";
const MAX_AGE: Duration = Duration::from_secs(30 * 60);
type Result<T> = std::result::Result<T, CollectionLoginError>;

fn error(code: &str, message: &str) -> CollectionLoginError {
    CollectionLoginError {
        code: code.into(),
        message: message.into(),
    }
}
pub(crate) fn require_main(window: &WebviewWindow) -> Result<()> {
    let url = window
        .url()
        .map_err(|_| error("LOGIN_FORBIDDEN", "无法确认设置窗口"))?;
    let local = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost"));
    let development = window
        .app_handle()
        .config()
        .build
        .dev_url
        .as_ref()
        .is_some_and(|allowed| allowed.origin() == url.origin());
    if window.label() != "main" || !(local || development) {
        return Err(error(
            "LOGIN_FORBIDDEN",
            "请从 Studio 的 Pixiv 设置使用登录助手",
        ));
    }
    Ok(())
}
fn terminal(phase: &Phase) -> bool {
    !matches!(
        phase,
        Phase::Waiting | Phase::Verifying | Phase::Unconfirmed
    )
}
fn window_label(id: &str) -> String {
    format!("{WINDOW_PREFIX}{id}")
}
fn close_windows(app: &AppHandle, id: &str) {
    let prefix = window_label(id);
    for (label, window) in app.webview_windows() {
        if label == prefix || label.starts_with(&format!("{prefix}-")) {
            let _ = window.destroy();
        }
    }
}
pub(crate) fn close_all(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(WINDOW_PREFIX) {
            let _ = window.destroy();
        }
    }
}
pub(crate) fn close_children(app: &AppHandle, parent: &str) {
    if !parent.starts_with(WINDOW_PREFIX) {
        return;
    }
    for (label, window) in app.webview_windows() {
        if label.starts_with(&format!("{parent}-")) {
            let _ = window.destroy();
        }
    }
}

struct Flow {
    start: StartCollectionLogin,
    public: CollectionLoginSession,
    target_root: String,
    born: Instant,
    pending: Option<SaveCollectionAccount>,
}
pub(crate) struct LoginHost {
    profile: PathBuf,
    login_url: Url,
    visible: bool,
    popup_slots: Arc<Semaphore>,
    flow: Mutex<Option<Flow>>,
}
impl LoginHost {
    pub(crate) fn new(data_dir: PathBuf) -> Self {
        Self::with_window(
            data_dir,
            LOGIN_URL.parse().expect("fixed Pixiv login URL"),
            true,
        )
    }
    // The URL is selected by the trusted native host, never an IPC argument.
    // The native acceptance host supplies its own isolated loopback login page.
    pub(crate) fn with_window(data_dir: PathBuf, login_url: Url, visible: bool) -> Self {
        Self {
            profile: data_dir.join("pixiv-login-webview"),
            login_url,
            visible,
            popup_slots: Arc::new(Semaphore::new(3)),
            flow: Mutex::new(None),
        }
    }
}
fn refresh(app: &AppHandle, flow: &mut Flow) {
    flow.public.window_open = app
        .get_webview_window(&window_label(&flow.public.id))
        .is_some();
    if flow.public.phase == Phase::Waiting
        && (!flow.public.window_open || flow.born.elapsed() > MAX_AGE)
    {
        flow.public.phase = if flow.born.elapsed() > MAX_AGE {
            Phase::Expired
        } else {
            Phase::Cancelled
        };
        flow.pending = None;
        close_windows(app, &flow.public.id);
        flow.public.window_open = false;
    }
}
fn allowed_navigation(url: &Url, start: &Url) -> bool {
    if url.as_str() == "about:blank" {
        return true;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if url.origin() == start.origin() {
        return true;
    }
    if url.scheme() != "https" || url.port_or_known_default() != Some(443) {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    host == "pixiv.net"
        || host.ends_with(".pixiv.net")
        || matches!(
            host,
            "accounts.google.com"
                | "accounts.google.co.jp"
                | "accounts.youtube.com"
                | "appleid.apple.com"
                | "account.apple.com"
                | "www.facebook.com"
                | "m.facebook.com"
                | "twitter.com"
                | "api.twitter.com"
                | "x.com"
                | "api.x.com"
                | "access.line.me"
        )
}
fn login_window(
    app: &AppHandle,
    label: String,
    url: Url,
    profile: PathBuf,
    start: Url,
    visible: bool,
    features: Option<tauri::webview::NewWindowFeatures>,
) -> tauri::Result<WebviewWindow> {
    let initially_visible = visible && url.as_str() != "about:blank";
    let popup_app = app.clone();
    let popup_profile = profile.clone();
    let popup_start = start.clone();
    let popup_parent = label.clone();
    let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url));
    let builder = if let Some(features) = features {
        // On Linux a related view shares the opener's ephemeral cookie store.
        // A second independent incognito WebView would lose the login session.
        builder.window_features(features)
    } else {
        builder
    };
    builder
        .title("Pixiv 登录 · 完成后返回 Studio 验证并保存")
        .inner_size(1080.0, 800.0)
        .min_inner_size(640.0, 500.0)
        .decorations(true)
        .resizable(true)
        .center()
        .data_directory(profile)
        .incognito(true)
        .visible(initially_visible)
        .devtools(false)
        .on_navigation(move |url| allowed_navigation(url, &start))
        .on_download(|_, _| false)
        .on_new_window(move |url, _features| {
            if !allowed_navigation(&url, &popup_start)
                || popup_app
                    .webview_windows()
                    .keys()
                    .filter(|k| k.starts_with(WINDOW_PREFIX))
                    .count()
                    >= 4
            {
                return tauri::webview::NewWindowResponse::Deny;
            }
            let Ok(permit) = popup_app
                .state::<LoginHost>()
                .popup_slots
                .clone()
                .try_acquire_owned()
            else {
                return tauri::webview::NewWindowResponse::Deny;
            };
            #[cfg(target_os = "linux")]
            {
                // WebKitGTK supplies a GTK object in the request. Create the
                // related window on this UI thread, without moving it to Tokio.
                let state = popup_app.state::<LoginHost>();
                let Ok(mut guard) = state.flow.try_lock() else {
                    return tauri::webview::NewWindowResponse::Deny;
                };
                let Some(flow) = guard.as_mut() else {
                    return tauri::webview::NewWindowResponse::Deny;
                };
                if flow.public.phase != Phase::Waiting
                    || !popup_parent.starts_with(&window_label(&flow.public.id))
                    || popup_app.get_webview_window(&popup_parent).is_none()
                {
                    return tauri::webview::NewWindowResponse::Deny;
                }
                match login_window(
                    &popup_app,
                    format!("{popup_parent}-{}", Uuid::new_v4()),
                    url,
                    popup_profile.clone(),
                    popup_start.clone(),
                    visible,
                    Some(_features),
                ) {
                    Ok(view) => {
                        let permit = StdMutex::new(Some(permit));
                        view.on_window_event(move |event| {
                            if matches!(event, tauri::WindowEvent::Destroyed)
                                && let Ok(mut held) = permit.lock()
                            {
                                held.take();
                            }
                        });
                        tauri::webview::NewWindowResponse::Create { window: view }
                    }
                    Err(_) => {
                        flow.public.error = Some(error(
                            "LOGIN_WINDOW_FAILED",
                            "登录链接未能打开，请重试或使用 Cookie 导入",
                        ));
                        tauri::webview::NewWindowResponse::Deny
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                // Creating a WebView synchronously in this WebView2 callback deadlocks
                // its UI thread. Open an owned child asynchronously, with the same
                // private environment. This does not promise window.opener semantics.
                let handle = popup_app.clone();
                let parent = popup_parent.clone();
                let profile = popup_profile.clone();
                let start = popup_start.clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<LoginHost>();
                    let mut guard = state.flow.lock().await;
                    let Some(flow) = guard.as_mut() else {
                        return;
                    };
                    if flow.public.phase != Phase::Waiting
                        || !parent.starts_with(&window_label(&flow.public.id))
                        || handle.get_webview_window(&parent).is_none()
                        || handle
                            .webview_windows()
                            .keys()
                            .filter(|k| k.starts_with(WINDOW_PREFIX))
                            .count()
                            >= 4
                    {
                        return;
                    }
                    let app = handle.clone();
                    let label = format!("{parent}-{}", Uuid::new_v4());
                    match tokio::task::spawn_blocking(move || {
                        login_window(&app, label, url, profile, start, visible, None)
                    })
                    .await
                    {
                        Ok(Ok(view)) if handle.get_webview_window(&parent).is_some() => {
                            let permit = StdMutex::new(Some(permit));
                            view.on_window_event(move |event| {
                                if matches!(event, tauri::WindowEvent::Destroyed)
                                    && let Ok(mut held) = permit.lock()
                                {
                                    held.take();
                                }
                            });
                        }
                        Ok(Ok(view)) => {
                            let _ = view.destroy();
                        }
                        _ => {
                            flow.public.error = Some(error(
                                "LOGIN_WINDOW_FAILED",
                                "登录链接未能打开，请使用邮箱和密码登录，或改用 Cookie 导入",
                            ));
                        }
                    }
                });
                tauri::webview::NewWindowResponse::Deny
            }
        })
        .build()
}

async fn backend(app: &AppHandle) -> Result<(reqwest::Client, EngineConnection)> {
    let connection = crate::owned_engine_connection(app).await.map_err(|_| {
        error(
            "LOGIN_ENGINE_UNAVAILABLE",
            "本机引擎暂不可用，请恢复连接后重试",
        )
    })?;
    let url = Url::parse(&connection.endpoint)
        .map_err(|_| error("LOGIN_ENGINE_UNAVAILABLE", "本机引擎地址无效"))?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(error("LOGIN_ENGINE_UNAVAILABLE", "本机引擎地址无效"));
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|_| error("LOGIN_ENGINE_UNAVAILABLE", "无法建立本机验证连接"))?;
    Ok((client, connection))
}
fn backend_error(value: &serde_json::Value) -> CollectionLoginError {
    let code = value["code"]
        .as_str()
        .filter(|s| s.len() <= 80 && s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
        .unwrap_or("LOGIN_VERIFY_FAILED");
    let message = match code {
        "COLLECTION_CREDENTIAL_REQUIRED" => "网站尚未确认登录，请在登录窗口完成登录或验证码后重试",
        "COLLECTION_SCOPE_CHANGED" => {
            "浏览器登录的是另一个账号；请取消此次登录，选择“新增登录会话”后重试"
        }
        "REVISION_CONFLICT" => "该会话已被其他操作修改，请取消此次登录、刷新账号后重试",
        "COLLECTION_REMOTE_UNAVAILABLE" => {
            "Pixiv 暂时无法访问或正在限流，请稍后重试，已有凭据保持不变"
        }
        "COLLECTION_LIMIT" | "INVALID_INPUT" => {
            "浏览器会话材料不符合导入要求，请重新登录或使用 Cookie 导入"
        }
        _ => "本次会话验证未完成，请稍后重试或使用 Cookie 导入",
    };
    error(code, message)
}
async fn decode<T: DeserializeOwned>(
    mut response: reqwest::Response,
    byte_limit: usize,
) -> Result<T> {
    let ok = response.status().is_success();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        error(
            "LOGIN_CONNECTION_INTERRUPTED",
            "验证连接中断，请重试以确认保存结果",
        )
    })? {
        if bytes.len() + chunk.len() > byte_limit {
            return Err(error("LOGIN_VERIFY_FAILED", "验证响应超出限制"));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !ok {
        return Err(backend_error(
            &serde_json::from_slice(&bytes).unwrap_or_default(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        error(
            "LOGIN_VERIFY_FAILED",
            "验证响应不兼容，请更新 Studio 后重试",
        )
    })
}
async fn target_root(client: &reqwest::Client, connection: &EngineConnection) -> Result<String> {
    let response = client
        .get(format!(
            "{}/v1/source-collections/status",
            connection.endpoint
        ))
        .bearer_auth(&connection.token)
        .send()
        .await
        .map_err(|_| error("LOGIN_ENGINE_UNAVAILABLE", "无法连接本机采集服务"))?;
    // Status may include twenty active definitions with 1,000 seeds each.
    // Use the bounded worker-frame budget, not the smaller authentication reply budget.
    let value: serde_json::Value = decode(response, 2 * 1024 * 1024).await?;
    if value["configured"] != true {
        return Err(error("LOGIN_NOT_CONFIGURED", "请先配置数据湖更新服务"));
    }
    value["runtime"]["state_root"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| error("LOGIN_NOT_CONFIGURED", "采集服务尚未就绪，请恢复连接后重试"))
}
fn normalize_cookies(
    values: Vec<tauri::webview::Cookie<'static>>,
) -> Result<Vec<CollectionCookie>> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let mut cookies = BTreeMap::new();
    for c in values {
        let domain = c.domain().unwrap_or_default();
        let plain = domain.trim_start_matches('.');
        if !matches!(plain, "pixiv.net" | "www.pixiv.net" | "accounts.pixiv.net") {
            continue;
        }
        let expiry = c.expires_datetime().map(|t| t.unix_timestamp());
        if c.value().is_empty() || expiry.is_some_and(|t| t <= now) {
            continue;
        }
        // cookie::Cookie normalizes a leading dot; the server cookie jar accepts
        // this canonical domain form and still scopes requests to Pixiv HTTPS.
        let domain = plain;
        let value = CollectionCookie {
            name: c.name().into(),
            value: c.value().into(),
            domain: domain.into(),
            path: c.path().unwrap_or("/").into(),
            secure: true,
            http_only: c.http_only().unwrap_or(false),
            expires_unix: expiry.map(|t| t as u64),
        };
        cookies.insert(
            (value.domain.clone(), value.path.clone(), value.name.clone()),
            value,
        );
    }
    let values: Vec<_> = cookies.into_values().collect();
    if !values
        .iter()
        .any(|c| c.name == "PHPSESSID" && c.path == "/" && c.domain != "accounts.pixiv.net")
    {
        return Err(error(
            "COLLECTION_CREDENTIAL_REQUIRED",
            "尚未取得 Pixiv 登录会话，请先在登录窗口完成登录",
        ));
    }
    if values.len() > 128 || serde_json::to_vec(&values).map_or(true, |b| b.len() > 60 * 1024) {
        return Err(error(
            "COLLECTION_LIMIT",
            "Pixiv 会话材料超过导入上限，请重新登录",
        ));
    }
    Ok(values)
}

#[tauri::command]
pub(crate) async fn pixiv_login_status(
    window: WebviewWindow,
    app: AppHandle,
    host: tauri::State<'_, LoginHost>,
) -> Result<CollectionLoginStatus> {
    require_main(&window)?;
    let mut guard = host.flow.lock().await;
    if let Some(flow) = guard.as_mut() {
        refresh(&app, flow);
    }
    Ok(CollectionLoginStatus {
        available: cfg!(any(windows, target_os = "linux")),
        session: guard.as_ref().map(|f| f.public.clone()),
    })
}
#[tauri::command]
pub(crate) async fn pixiv_login_start(
    window: WebviewWindow,
    app: AppHandle,
    host: tauri::State<'_, LoginHost>,
    mut input: StartCollectionLogin,
) -> Result<CollectionLoginSession> {
    require_main(&window)?;
    if !cfg!(any(windows, target_os = "linux")) {
        return Err(error("LOGIN_UNSUPPORTED", "当前平台请使用 Cookie 导入"));
    }
    input.label = input.label.trim().into();
    if Uuid::parse_str(&input.account_id).is_err()
        || Uuid::parse_str(&input.request_key).is_err()
        || input.label.is_empty()
        || input.label.chars().count() > 128
    {
        return Err(error("INVALID_INPUT", "请填写有效的登录会话名称"));
    }
    let mut guard = host.flow.lock().await;
    if let Some(flow) = guard.as_mut() {
        refresh(&app, flow);
        if !terminal(&flow.public.phase) {
            if flow.start.request_key == input.request_key
                && flow.start.account_id == input.account_id
                && flow.start.label == input.label
                && flow.start.expected_revision == input.expected_revision
            {
                return Ok(flow.public.clone());
            }
            return Err(error("LOGIN_BUSY", "请先完成或取消当前登录会话"));
        }
    }
    let (client, connection) = backend(&app).await?;
    let root = target_root(&client, &connection).await?;
    std::fs::create_dir_all(&host.profile)
        .map_err(|_| error("LOGIN_WINDOW_FAILED", "无法建立独立登录窗口的数据目录"))?;
    let id = Uuid::new_v4().to_string();
    let handle = app.clone();
    let label = window_label(&id);
    let profile = host.profile.clone();
    let start = host.login_url.clone();
    let visible = host.visible;
    tokio::task::spawn_blocking(move || {
        let view = login_window(
            &handle,
            label,
            "about:blank".parse().expect("fixed blank URL"),
            profile,
            start.clone(),
            visible,
            None,
        )?;
        // Clear only this owned InPrivate profile before the first network navigation.
        // Both native WebViews clear asynchronously; wait before opening Pixiv.
        let initialized = (|| -> tauri::Result<()> {
            let marker = tauri::webview::Cookie::build(("studio_login_reset", "1"))
                .domain("accounts.pixiv.net")
                .path("/")
                .secure(true)
                .http_only(true)
                .build();
            view.set_cookie(marker)?;
            let mut marker_visible = false;
            for _ in 0..30 {
                if view
                    .cookies()?
                    .iter()
                    .any(|c| c.name() == "studio_login_reset")
                {
                    marker_visible = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if !marker_visible {
                return Err(tauri::Error::Io(std::io::Error::other(
                    "private cookie store unavailable",
                )));
            }
            view.clear_all_browsing_data()?;
            for _ in 0..30 {
                if view.cookies()?.is_empty() {
                    view.navigate(start)?;
                    if visible {
                        view.show()?;
                        view.set_focus()?;
                    }
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(tauri::Error::Io(std::io::Error::other(
                "private login profile could not be cleared",
            )))
        })();
        if initialized.is_err() {
            let _ = view.destroy();
        }
        initialized
    })
    .await
    .map_err(|_| error("LOGIN_WINDOW_FAILED", "登录窗口创建失败，请重试"))?
    .map_err(|_| {
        error(
            "LOGIN_WINDOW_FAILED",
            "无法打开登录窗口，请检查系统浏览器组件或使用 Cookie 导入",
        )
    })?;
    let public = CollectionLoginSession {
        id,
        account_id: input.account_id.clone(),
        label: input.label.clone(),
        phase: Phase::Waiting,
        window_open: true,
        result: None,
        error: None,
    };
    *guard = Some(Flow {
        start: input,
        public: public.clone(),
        target_root: root,
        born: Instant::now(),
        pending: None,
    });
    Ok(public)
}
#[tauri::command]
pub(crate) async fn pixiv_login_show(
    window: WebviewWindow,
    app: AppHandle,
    host: tauri::State<'_, LoginHost>,
    id: String,
) -> Result<()> {
    require_main(&window)?;
    let mut guard = host.flow.lock().await;
    let flow = guard
        .as_mut()
        .filter(|f| f.public.id == id)
        .ok_or_else(|| error("LOGIN_CLOSED", "登录会话已结束"))?;
    refresh(&app, flow);
    let view = app
        .get_webview_window(&window_label(&id))
        .ok_or_else(|| error("LOGIN_CLOSED", "登录窗口已关闭，请重新开始"))?;
    if !host.visible {
        return Ok(());
    }
    view.show()
        .and_then(|_| view.unminimize())
        .and_then(|_| view.set_focus())
        .map_err(|_| error("LOGIN_WINDOW_FAILED", "无法显示登录窗口"))
}
#[tauri::command]
pub(crate) async fn pixiv_login_cancel(
    window: WebviewWindow,
    app: AppHandle,
    host: tauri::State<'_, LoginHost>,
    id: String,
) -> Result<()> {
    require_main(&window)?;
    let mut guard = host.flow.lock().await;
    if let Some(flow) = guard.as_mut().filter(|f| f.public.id == id) {
        if matches!(flow.public.phase, Phase::Verifying | Phase::Unconfirmed) {
            return Err(error("LOGIN_BUSY", "请先确认本次保存结果，再结束登录会话"));
        }
        if !terminal(&flow.public.phase) {
            flow.public.phase = Phase::Cancelled;
        }
        flow.pending = None;
        flow.public.window_open = false;
        close_windows(&app, &id);
    }
    Ok(())
}
#[tauri::command]
pub(crate) async fn pixiv_login_finish(
    window: WebviewWindow,
    app: AppHandle,
    host: tauri::State<'_, LoginHost>,
    id: String,
) -> Result<CollectionAccountProbe> {
    require_main(&window)?;
    let (view, replay) = {
        let mut guard = host.flow.lock().await;
        let flow = guard
            .as_mut()
            .filter(|f| f.public.id == id)
            .ok_or_else(|| error("LOGIN_CLOSED", "登录会话已结束"))?;
        refresh(&app, flow);
        if let Some(result) = &flow.public.result {
            return Ok(result.clone());
        }
        if flow.public.phase == Phase::Verifying {
            return Err(error("LOGIN_BUSY", "正在验证，请等待结果"));
        }
        if !matches!(flow.public.phase, Phase::Waiting | Phase::Unconfirmed) {
            return Err(error("LOGIN_CLOSED", "登录窗口已关闭或超时，请重新开始"));
        }
        let replay = if flow.public.phase == Phase::Unconfirmed {
            flow.pending.clone()
        } else {
            None
        };
        let view = app.get_webview_window(&window_label(&id));
        if replay.is_none() && view.is_none() {
            return Err(error("LOGIN_CLOSED", "登录窗口已关闭"));
        }
        flow.public.phase = Phase::Verifying;
        flow.public.error = None;
        (view, replay)
    };
    let replaying = replay.is_some();
    let result: Result<CollectionAccountProbe> = async {
        let body = if let Some(body) = replay {
            body
        } else {
            let view = view.ok_or_else(|| error("LOGIN_CLOSED", "登录窗口已关闭"))?;
            let cookies = tokio::task::spawn_blocking(move || {
                let mut values = Vec::new();
                for url in ["https://www.pixiv.net/", "https://accounts.pixiv.net/"] {
                    values.extend(
                        view.cookies_for_url(url.parse().expect("fixed Pixiv cookie URL"))
                            .map_err(|_| {
                                error(
                                    "LOGIN_COOKIE_READ_FAILED",
                                    "无法读取此次登录会话，请重新打开登录窗口",
                                )
                            })?,
                    );
                }
                normalize_cookies(values)
            })
            .await
            .map_err(|_| error("LOGIN_COOKIE_READ_FAILED", "无法读取此次登录会话"))??;
            {
                let mut guard = host.flow.lock().await;
                let flow = guard
                    .as_mut()
                    .filter(|f| f.public.id == id)
                    .ok_or_else(|| error("LOGIN_CLOSED", "登录会话已结束"))?;
                if flow
                    .pending
                    .as_ref()
                    .is_none_or(|p| p.cookies.as_ref() != Some(&cookies))
                {
                    flow.pending = Some(SaveCollectionAccount {
                        request_key: Uuid::new_v4().to_string(),
                        account_id: flow.start.account_id.clone(),
                        expected_revision: flow.start.expected_revision,
                        label: flow.start.label.clone(),
                        mode: CollectionAccountMode::Session,
                        cookies: Some(cookies),
                    });
                }
                flow.pending.clone().expect("candidate prepared")
            }
        };
        let expected_root = host
            .flow
            .lock()
            .await
            .as_ref()
            .filter(|f| f.public.id == id)
            .map(|f| f.target_root.clone())
            .ok_or_else(|| error("LOGIN_CLOSED", "登录会话已结束"))?;
        let (client, connection) = backend(&app).await?;
        if target_root(&client, &connection).await? != expected_root {
            return Err(error(
                "LOGIN_TARGET_CHANGED",
                "采集服务位置已改变，请取消此次登录后重新开始",
            ));
        }
        let response = client
            .post(format!(
                "{}/v1/source-collections/accounts/{}/authenticate",
                connection.endpoint, body.account_id
            ))
            .bearer_auth(&connection.token)
            .json(&body)
            .send()
            .await
            .map_err(|_| {
                error(
                    "LOGIN_CONNECTION_INTERRUPTED",
                    "验证连接中断，请重试以确认保存结果",
                )
            })?;
        let result: CollectionAccountProbe = decode(response, 65536).await?;
        if result.account.id != body.account_id
            || result.account.state != "valid"
            || !result.account.credential_set
        {
            return Err(error(
                "REVISION_CONFLICT",
                "保存结果已被后续操作修改，请刷新账号状态后重试",
            ));
        }
        Ok(result)
    }
    .await;
    let mut guard = host.flow.lock().await;
    if let Some(flow) = guard.as_mut().filter(|f| f.public.id == id) {
        match &result {
            Ok(value) => {
                flow.public.phase = Phase::Succeeded;
                flow.public.result = Some(value.clone());
                flow.pending = None;
                close_windows(&app, &id);
                flow.public.window_open = false;
            }
            Err(value) => {
                flow.public.phase = if (value.code == "LOGIN_CONNECTION_INTERRUPTED"
                    || (replaying && value.code.starts_with("LOGIN_")))
                    && flow.pending.is_some()
                {
                    Phase::Unconfirmed
                } else {
                    Phase::Waiting
                };
                flow.public.error = Some(value.clone());
                refresh(&app, flow);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_navigation_rejects_local_and_confusable_destinations() {
        let start: Url = LOGIN_URL.parse().unwrap();
        for valid in [
            "https://accounts.pixiv.net/login",
            "https://www.pixiv.net/",
            "https://accounts.google.com/",
            "about:blank",
        ] {
            assert!(allowed_navigation(&valid.parse().unwrap(), &start));
        }
        for invalid in [
            "http://www.pixiv.net/",
            "https://pixiv.net.evil.invalid/",
            "https://evilpixiv.net/",
            "https://pixiv.net@evil.invalid/",
            "http://127.0.0.1:1420/",
            "file:///C:/Windows/",
            "javascript:alert(1)",
            "https://www.pixiv.net:8443/",
        ] {
            assert!(
                !allowed_navigation(&invalid.parse().unwrap(), &start),
                "{invalid}"
            );
        }
    }

    #[test]
    fn only_current_pixiv_cookies_cross_the_bridge() {
        let parse =
            |value: &'static str| tauri::webview::Cookie::parse(value).unwrap().into_owned();
        let session =
            parse("PHPSESSID=NATIVE_SYNTHETIC; Domain=.pixiv.net; Path=/; Secure; HttpOnly");
        let result = normalize_cookies(vec![
            session.clone(),
            session,
            parse("FOREIGN=private; Domain=accounts.google.com; Path=/; Secure; HttpOnly"),
            parse("EXPIRED=old; Domain=.pixiv.net; Path=/; Expires=Sat, 01 Jan 2000 00:00:00 GMT"),
        ])
        .unwrap_or_else(|e| panic!("{}", e.code));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].domain, "pixiv.net");
        assert_eq!(result[0].expires_unix, None);
        assert!(result[0].http_only && result[0].secure);
        assert_eq!(
            normalize_cookies(Vec::new()).err().unwrap().code,
            "COLLECTION_CREDENTIAL_REQUIRED"
        );
    }

    #[test]
    fn oversized_cookie_candidates_fail_before_transport() {
        let value = "a".repeat(64 * 1024);
        let candidate = tauri::webview::Cookie::build(("PHPSESSID", value))
            .domain("www.pixiv.net")
            .path("/")
            .build();
        assert_eq!(
            normalize_cookies(vec![candidate]).err().unwrap().code,
            "COLLECTION_LIMIT"
        );
    }

    #[test]
    fn arbitrary_backend_error_text_never_returns_credentials() {
        for code in [
            "COLLECTION_CREDENTIAL_REQUIRED",
            "UNEXPECTED_ERROR",
            "secret-value-123",
        ] {
            let response = backend_error(
                &serde_json::json!({"code":code,"message":"PHPSESSID=private-secret"}),
            );
            let encoded = serde_json::to_string(&response).unwrap();
            assert!(!encoded.contains("private-secret") && !encoded.contains("secret-value-123"));
        }
    }
}
