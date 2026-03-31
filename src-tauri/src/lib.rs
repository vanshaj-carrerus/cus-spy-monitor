use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 1. IMPROVED SINGLE INSTANCE LOGIC
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // When a user tries to start the app again, find the 'main' window
            if let Some(window) = app.get_webview_window("main") {
                // Show it, focus it, and bring it to the front
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.unminimize(); // In case it was minimized to taskbar
            }
        }))
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![greet])
        .setup(|app| {
            // SILENT UPDATER LOGIC
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(Some(update)) = handle
                    .updater()
                    .expect("failed to get updater")
                    .check()
                    .await
                {
                    let _ = update.download_and_install(|_cl, _tl| {}, || {}).await;
                    handle.restart();
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // 2. PREVENT CLOSE (Keep running in background)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}