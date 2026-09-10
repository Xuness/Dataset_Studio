#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
};
use studio_protocol::{API_VERSION, EngineConnection, Health};
use tauri::Manager;
struct Host {
    data_dir: PathBuf,
    lock: Arc<tokio::sync::Mutex<()>>,
}
async fn live(path: &std::path::Path) -> Option<EngineConnection> {
    let bytes = fs::read(path).ok()?;
    let connection: EngineConnection = serde_json::from_slice(&bytes).ok()?;
    if connection.api_version != API_VERSION
        || !connection.endpoint.starts_with("http://127.0.0.1:")
    {
        return None;
    }
    let response = reqwest::Client::new()
        .get(format!("{}/v1/health", connection.endpoint))
        .bearer_auth(&connection.token)
        .timeout(std::time::Duration::from_secs(1))
        .send()
        .await
        .ok()?;
    let health: Health = response.json().await.ok()?;
    (health.api_version == API_VERSION && health.instance_id == connection.instance_id)
        .then_some(connection)
}
#[tauri::command]
async fn engine_connection(host: tauri::State<'_, Host>) -> Result<EngineConnection, String> {
    let _guard = host.lock.lock().await;
    let discovery = host.data_dir.join("engine.json");
    if let Some(connection) = live(&discovery).await {
        return Ok(connection);
    }
    // The development coordinator owns rebuild/restart ordering. A reconnect must
    // not race it by launching an earlier binary inherited in this host's environment.
    if std::env::var_os("STUDIO_DEVELOPMENT").is_some()
        || std::env::var_os("STUDIO_ENGINE_PATH").is_some()
    {
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if let Some(connection) = live(&discovery).await {
                return Ok(connection);
            }
        }
        return Err("开发引擎正在重启，请查看开发控制台后重试连接。".into());
    }
    fs::create_dir_all(&host.data_dir).map_err(|e| e.to_string())?;
    let engine = if let Some(path) = std::env::var_os("STUDIO_ENGINE_PATH") {
        PathBuf::from(path)
    } else {
        std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .ok_or("应用位置无效")?
            .join("studio-engine-sidecar.exe")
    };
    if !engine.is_file() {
        return Err("未找到本机引擎，请从仓库运行 pnpm dev 或重新安装应用。".into());
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(host.data_dir.join("engine.log"))
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(&engine);
    command
        .arg("serve")
        .arg("--data-dir")
        .arg(&host.data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if let Some(connection) = live(&discovery).await {
            return Ok(connection);
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!(
                "本机引擎未能启动（{status}），请查看应用目录中的 engine.log。"
            ));
        }
    }
    Err("本机引擎连接超时。已有任务与项目状态保存在磁盘中，可重试连接。".into())
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = std::env::var_os("STUDIO_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(app.path().app_local_data_dir()?);
            app.manage(Host {
                data_dir,
                lock: Arc::new(tokio::sync::Mutex::new(())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![engine_connection])
        .run(tauri::generate_context!())
        .expect("桌面宿主启动失败");
}
