use tauri::{Manager, Emitter, RunEvent};
use std::time::{Duration, Instant};
use tauri_plugin_updater::UpdaterExt;
use serde::Serialize;
use chrono::Utc;
use std::sync::{Arc, Mutex, OnceLock};
use device_query::{DeviceQuery, DeviceState};

// ─── Payload sent to the frontend ────────────────────────────────────────────
#[derive(Serialize, Clone, Debug)]
struct ActivityLog {
    user_id: String,
    title: String,
    app_name: String,
    start_time: String,
    duration_seconds: u64,
}

// ─── Shared mutable tracking state (for window hook) ─────────────────────────
struct TrackingState {
    last_title: String,
    last_app: String,
    start_time: chrono::DateTime<Utc>,
    user_id: String,
}

// Static globals – the only way to reach them from a raw C callback
static TRACKING_STATE: OnceLock<Arc<Mutex<TrackingState>>> = OnceLock::new();
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

// ─── Windows-specific event hook module ──────────────────────────────────────
#[cfg(target_os = "windows")]
mod win_tracker {
    use super::*;

    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, HWND},
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW,
                PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            },
            UI::{
                Accessibility::{SetWinEventHook, HWINEVENTHOOK},
                WindowsAndMessaging::{
                    DispatchMessageW, GetMessageW, GetWindowTextW,
                    GetWindowThreadProcessId, TranslateMessage,
                    EVENT_SYSTEM_FOREGROUND, MSG, WINEVENT_OUTOFCONTEXT,
                },
            },
        },
    };

    /// Resolve the window title and executable filename from an HWND.
    fn get_window_info(hwnd: HWND) -> (String, String) {
        // --- title ---
        let mut title_buf = [0u16; 512];
        let title_len = unsafe { GetWindowTextW(hwnd, &mut title_buf) };
        let title = String::from_utf16_lossy(&title_buf[..title_len as usize]);

        // --- app name (exe filename) ---
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };

        let app_name = if pid != 0 {
            unsafe {
                match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    Ok(handle) => {
                        let mut name_buf = [0u16; 512];
                        let mut size = name_buf.len() as u32;
                        let _ = QueryFullProcessImageNameW(
                            handle,
                            PROCESS_NAME_WIN32,
                            PWSTR(name_buf.as_mut_ptr()),
                            &mut size,
                        );
                        let _ = CloseHandle(handle);
                        let full_path =
                            String::from_utf16_lossy(&name_buf[..size as usize]);
                        // Keep only the filename, e.g. "chrome.exe"
                        full_path
                            .split('\\')
                            .last()
                            .unwrap_or("Unknown")
                            .to_string()
                    }
                    Err(_) => "Unknown".to_string(),
                }
            }
        } else {
            "Unknown".to_string()
        };

        (title, app_name)
    }

    /// The raw callback fired by Windows on every EVENT_SYSTEM_FOREGROUND.
    /// Must be `unsafe extern "system"` – no closure captures allowed.
    pub unsafe extern "system" fn hook_callback(
        _hook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        _id_object: i32,
        _id_child: i32,
        _id_event_thread: u32,
        _event_time: u32,
    ) {
        if hwnd.is_invalid() {
            return;
        }

        let (title, app_name) = get_window_info(hwnd);
        if title.is_empty() {
            return;
        }

        let state_arc = match TRACKING_STATE.get() {
            Some(s) => s,
            None => return,
        };
        let handle = match APP_HANDLE.get() {
            Some(h) => h,
            None => return,
        };

        let now = Utc::now();
        let mut state = state_arc.lock().unwrap();

        // No change – skip
        if title == state.last_title && app_name == state.last_app {
            return;
        }

        // Emit log for the window that just lost focus
        if !state.last_title.is_empty() {
            let secs = now.signed_duration_since(state.start_time).num_seconds();
            if secs > 0 {
                let _ = handle.emit(
                    "new-activity-log",
                    ActivityLog {
                        user_id: state.user_id.clone(),
                        title: state.last_title.clone(),
                        app_name: state.last_app.clone(),
                        start_time: state.start_time.to_rfc3339(),
                        duration_seconds: secs as u64,
                    },
                );
            }
        }

        // Update state to the new foreground window
        state.last_title = title;
        state.last_app = app_name;
        state.start_time = now;
    }

    /// Spawn a dedicated OS thread, install the hook, then run the message pump.
    /// Windows requires both the hook and the pump to live on the same thread.
    pub fn start_hook_thread() {
        std::thread::spawn(|| unsafe {
            let _hook = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(hook_callback),
                0, // all processes
                0, // all threads
                WINEVENT_OUTOFCONTEXT,
            );

            // Message pump – keeps the thread alive and delivers hook callbacks
            let mut msg = MSG::default();
            loop {
                let result = GetMessageW(&mut msg, None, 0, 0);
                // 0 = WM_QUIT, -1 = error
                if result.0 == 0 || result.0 == -1 {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }
}

// ─── Tauri command (kept for compatibility) ───────────────────────────────────
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// ─── App entry point ──────────────────────────────────────────────────────────
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Windows username becomes the user_id – no login required on the Rust side
    let user_id = std::env::var("USERNAME").unwrap_or_else(|_| "unknown".to_string());

    let tracking_state = Arc::new(Mutex::new(TrackingState {
        last_title: String::new(),
        last_app: String::new(),
        start_time: Utc::now(),
        user_id,
    }));

    // Store in global so the static hook callback can reach it
    TRACKING_STATE.set(tracking_state.clone()).ok();

    let state_for_exit = tracking_state.clone();

    tauri::Builder::default()
        // Single-instance guard
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.unminimize();
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
            // Store AppHandle globally for the static hook callback
            APP_HANDLE.set(app.handle().clone()).ok();

            // Silent auto-updater
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(Some(update)) = handle
                    .updater()
                    .expect("failed to get updater")
                    .check()
                    .await
                {
                    let _ = update.download_and_install(|_cl, _tl| {}, || {}).await;
                    let _ = handle.restart();
                }
            });

            // ─── Idle-aware heartbeat loop ─────────────────────────────────────────
            // Runs every 1 s, accumulates productive / unproductive seconds,
            // then emits "send-heartbeat" to the frontend every 60 s.
            let heartbeat_handle = app.handle().clone();
            let hb_user_id = whoami::username();
            let session_id = format!("session-{}", Utc::now().timestamp());

            tauri::async_runtime::spawn(async move {
                let device_state = DeviceState::new();

                let mut productive_acc: u64 = 0;
                let mut unproductive_acc: u64 = 0;
                let mut last_input    = Instant::now();
                let mut last_heartbeat = Instant::now();
                let mut last_mouse_coords = (0i32, 0i32);

                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;

                    // Check system-wide input (keyboard + mouse movement/clicks)
                    let keys  = device_state.get_keys();
                    let mouse = device_state.get_mouse();
                    let coords = mouse.coords;
                    let has_activity = !keys.is_empty()
                        || coords != last_mouse_coords
                        || mouse.button_pressed.iter().any(|&p| p);

                    if has_activity {
                        last_input = Instant::now();
                        last_mouse_coords = coords;
                    }

                    // >60 s without input → unproductive
                    if last_input.elapsed().as_secs() > 60 {
                        unproductive_acc += 1;
                    } else {
                        productive_acc += 1;
                    }

                    // Emit heartbeat payload every 60 s
                    if last_heartbeat.elapsed().as_secs() >= 60 {
                        let payload = serde_json::json!({
                            "userId": hb_user_id,
                            "sessionId": session_id,
                            "productiveSeconds": productive_acc,
                            "unproductiveSeconds": unproductive_acc,
                            "timestamp": Utc::now().to_rfc3339()
                        });
                        let _ = heartbeat_handle.emit("send-heartbeat", payload);

                        productive_acc   = 0;
                        unproductive_acc = 0;
                        last_heartbeat   = Instant::now();
                    }
                }
            });

            // Start the Windows event hook on its own message-pump thread
            #[cfg(target_os = "windows")]
            win_tracker::start_hook_thread();

            Ok(())
        })
        .on_window_event(|window, event| {
            // Minimise to tray instead of closing
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |handle, event| {
            // Flush the last open window on clean exit
            if let RunEvent::Exit = event {
                let s = state_for_exit.lock().unwrap();
                if !s.last_title.is_empty() {
                    let secs =
                        Utc::now().signed_duration_since(s.start_time).num_seconds();
                    if secs > 0 {
                        let _ = handle.emit(
                            "new-activity-log",
                            ActivityLog {
                                user_id: s.user_id.clone(),
                                title: s.last_title.clone(),
                                app_name: s.last_app.clone(),
                                start_time: s.start_time.to_rfc3339(),
                                duration_seconds: secs as u64,
                            },
                        );
                    }
                }
            }
        });
}