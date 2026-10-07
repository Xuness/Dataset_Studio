//! Optional native acceptance host. Real production login bridge and system WebView,
//! synthetic cookies, a caller-owned loopback server, and an isolated profile.
use std::path::PathBuf;
use studio_protocol::EngineConnection;
use tauri::Manager;

#[path = "../src/pixiv_login.rs"]
#[allow(dead_code)] // Production constructor is replaced by this host's fixed test page.
mod pixiv_login;

struct Probe {
    connection: EngineConnection,
    run: PathBuf,
}
pub(crate) async fn owned_engine_connection(
    app: &tauri::AppHandle,
) -> Result<EngineConnection, String> {
    Ok(app.state::<Probe>().connection.clone())
}
#[tauri::command]
async fn engine_connection(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<EngineConnection, String> {
    pixiv_login::require_main(&window).map_err(|e| e.message)?;
    owned_engine_connection(&app).await
}
#[tauri::command]
async fn probe_seed(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    id: Option<String>,
) -> Result<(), String> {
    pixiv_login::require_main(&window).map_err(|e| e.message)?;
    let label = id
        .map(|id| format!("pixiv-login-{id}"))
        .unwrap_or_else(|| "main".into());
    let view = app
        .get_webview_window(&label)
        .ok_or("probe window missing")?;
    tokio::task::spawn_blocking(move || {
        let values = if label == "main" {
            vec![(
                "regular_scope_guard",
                "KEEP_REGULAR_PROFILE",
                "www.pixiv.net",
            )]
        } else {
            vec![
                ("PHPSESSID", "4242_NATIVE_LOGIN_FIXTURE", "www.pixiv.net"),
                ("NON_PIXIV_SECRET", "MUST_STAY_IN_BROWSER", ".example.com"),
            ]
        };
        for (name, value, domain) in values {
            view.set_cookie(
                tauri::webview::Cookie::build((name, value))
                    .domain(domain)
                    .path("/")
                    .secure(true)
                    .http_only(true)
                    .build(),
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn probe_regular_intact(window: tauri::WebviewWindow) -> Result<bool, String> {
    pixiv_login::require_main(&window).map_err(|e| e.message)?;
    tokio::task::spawn_blocking(move || {
        Ok(window
            .cookies_for_url("https://www.pixiv.net/".parse().unwrap())
            .map_err(|e| e.to_string())?
            .iter()
            .any(|c| c.name() == "regular_scope_guard" && c.value() == "KEEP_REGULAR_PROFILE"))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn probe_close_login(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    pixiv_login::require_main(&window).map_err(|e| e.message)?;
    if let Some(view) = app.get_webview_window(&format!("pixiv-login-{id}")) {
        view.destroy().map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
async fn probe_shutdown(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    pixiv_login::require_main(&window).map_err(|e| e.message)?;
    pixiv_login::close_all(&app);
    app.exit(0);
    Ok(())
}
#[tauri::command]
async fn probe_open_child(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    pixiv_login::require_main(&window).map_err(|e| e.message)?;
    app.get_webview_window(&format!("pixiv-login-{id}"))
        .ok_or("login missing")?
        .eval("window.open('http://127.0.0.1:1458/login-child', '_blank')")
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn probe_child_shared(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    id: String,
) -> Result<bool, String> {
    pixiv_login::require_main(&window).map_err(|e| e.message)?;
    let prefix = format!("pixiv-login-{id}-");
    let child = app
        .webview_windows()
        .into_iter()
        .find(|(label, _)| label.starts_with(&prefix));
    let Some((_, view)) = child else {
        return Ok(false);
    };
    tokio::task::spawn_blocking(move || {
        Ok(view
            .cookies_for_url("https://www.pixiv.net/".parse().unwrap())
            .map_err(|e| e.to_string())?
            .iter()
            .any(|c| c.name() == "PHPSESSID" && c.value() == "4242_NATIVE_LOGIN_FIXTURE"))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn probe_native_desktop(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<bool, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    pixiv_login::require_main(&window).map_err(|e| e.message)?;
    window.show().map_err(|e| e.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(1100.0, 850.0))
        .map_err(|e| e.to_string())?;
    window.maximize().map_err(|e| e.to_string())?;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let maximized = window.is_maximized().map_err(|e| e.to_string())?;
    window.unmaximize().map_err(|e| e.to_string())?;
    app.clipboard()
        .write_text("Studio 原生剪贴板 · Linux")
        .map_err(|e| e.to_string())?;
    let text = app.clipboard().read_text().map_err(|e| e.to_string())?;
    Ok(maximized && text == "Studio 原生剪贴板 · Linux")
}
#[tauri::command]
async fn probe_folder_dialog(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    pixiv_login::require_main(&window).map_err(|e| e.message)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Studio Linux folder probe")
        .set_directory(app.state::<Probe>().run.clone())
        .pick_folder(move |value| {
            let _ = tx.send(value.map(|v| v.to_string()));
        });
    rx.await.map_err(|e| e.to_string())
}
fn main() {
    let run = PathBuf::from(
        std::env::var_os("STUDIO_PIXIV_LOGIN_PROBE").expect("isolated probe directory"),
    );
    let connection: EngineConnection =
        serde_json::from_slice(&std::fs::read(run.join("connection.json")).unwrap()).unwrap();
    let mut context = tauri::generate_context!();
    let config = context.config_mut();
    config.identifier = "com.xuness.datasetstudio.pixiv-login-probe".into();
    config.build.dev_url = Some("http://127.0.0.1:1458".parse().unwrap());
    let window = &mut config.app.windows[0];
    window.title = "Pixiv Login Verification".into();
    window.width = 1100.0;
    window.height = 950.0;
    window.visible = false;
    window.decorations = true;
    // Same environment, distinct normal/InPrivate profiles: the test proves that
    // clearing the login profile does not erase the main profile's guard cookie.
    window.data_directory = Some(run.join("host/pixiv-login-webview"));
    let login_url = if std::env::var("STUDIO_PIXIV_LOGIN_LIVE_PAGE").as_deref() == Ok("1") {
        "https://accounts.pixiv.net/login?return_to=https%3A%2F%2Fwww.pixiv.net%2F"
    } else {
        "http://127.0.0.1:1458/login"
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(move |app| {
            app.manage(Probe {
                connection,
                run: run.clone(),
            });
            app.manage(pixiv_login::LoginHost::with_window(
                run.join("host"),
                login_url.parse().unwrap(),
                std::env::var("STUDIO_NATIVE_VISIBLE").as_deref() == Ok("1"),
            ));
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if window.label() == "main" {
                    pixiv_login::close_all(window.app_handle());
                } else {
                    pixiv_login::close_children(window.app_handle(), window.label());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            engine_connection,
            probe_seed,
            probe_regular_intact,
            probe_close_login,
            probe_shutdown,
            probe_open_child,
            probe_child_shared,
            probe_native_desktop,
            probe_folder_dialog,
            pixiv_login::pixiv_login_start,
            pixiv_login::pixiv_login_status,
            pixiv_login::pixiv_login_show,
            pixiv_login::pixiv_login_finish,
            pixiv_login::pixiv_login_cancel
        ])
        .run(context)
        .expect("native login acceptance host");
}
