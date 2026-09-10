// Optional native clipboard smoke host. It uses the production dependencies and
// permissions, with an isolated WebView profile and no engine or project access.
fn main() {
    let mut context = tauri::generate_context!();
    let config = context.config_mut();
    config.identifier = "com.xuness.datasetstudio.clipboard-probe".into();
    config.build.dev_url = Some("http://127.0.0.1:1435".parse().unwrap());
    let window = &mut config.app.windows[0];
    window.title = "Studio Clipboard Verification".into();
    window.width = 850.0;
    window.height = 620.0;
    window.min_width = None;
    window.min_height = None;
    window.decorations = true;
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .run(context)
        .expect("clipboard verification host failed");
}
