use tauri::{Manager, Emitter, RunEvent};
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicI64, Ordering};
use tauri_plugin_updater::UpdaterExt;
use serde::{Serialize, Deserialize};
use chrono::Utc;
use std::sync::{Arc, Mutex, OnceLock};
use device_query::{DeviceQuery, DeviceState};
use reqwest::Client;
use tauri_plugin_log::{Target, TargetKind};
use tauri_plugin_autostart::ManagerExt;
use screenshots::Screen;
use base64::{engine::general_purpose, Engine as _};
use image::{codecs::jpeg::JpegEncoder, ColorType};

// LiveKit imports
use livekit::prelude::*;
use livekit::webrtc::video_frame::{I420Buffer, VideoFrame, VideoRotation};
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::webrtc::video_source::{RtcVideoSource, VideoResolution};
use log::{info, error, warn, debug};

static LAST_INPUT_TS: AtomicI64 = AtomicI64::new(0);

// ─── Shared HTTP Client ──────────────────────────────────────────────────────
static CLIENT: OnceLock<Client> = OnceLock::new();

// ─── Payload sent to the frontend and backend ────────────────────────────────
#[derive(Serialize, Deserialize, Clone, Debug)]
struct ActivityLog {
    #[serde(rename = "userId")]
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
    username: String,  // The human-readable username (used by dashboard for stream)
}

// Static globals – the only way to reach them from a raw C callback
static TRACKING_STATE: OnceLock<Arc<Mutex<TrackingState>>> = OnceLock::new();
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

#[inline]
fn put_rgba_pixel(rgba: &mut [u8], width: u32, height: u32, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let idx = ((y as u32 * width + x as u32) * 4) as usize;
    if idx + 3 >= rgba.len() {
        return;
    }
    rgba[idx] = color[0];
    rgba[idx + 1] = color[1];
    rgba[idx + 2] = color[2];
    rgba[idx + 3] = color[3];
}

#[inline]
fn draw_cursor_on_frame(rgba: &mut [u8], width: u32, height: u32, x: i32, y: i32) {
    // Realistic arrow-like pointer (shadow + black stroke + white fill).
    // This is intentionally simple/fast because it runs every frame.
    let cursor_h = 24i32;
    let cursor_w = 16i32;

    // Shadow
    for dy in 0..cursor_h {
        let row_w = ((dy as f32 / cursor_h as f32) * cursor_w as f32).max(1.0) as i32;
        for dx in 0..=row_w {
            put_rgba_pixel(
                rgba,
                width,
                height,
                x + dx + 2,
                y + dy + 2,
                [0, 0, 0, 110],
            );
        }
    }

    // Black outline
    for dy in 0..cursor_h {
        let row_w = ((dy as f32 / cursor_h as f32) * cursor_w as f32).max(1.0) as i32;
        put_rgba_pixel(rgba, width, height, x, y + dy, [0, 0, 0, 255]);
        put_rgba_pixel(rgba, width, height, x + row_w, y + dy, [0, 0, 0, 255]);
    }
    for dx in 0..cursor_w {
        put_rgba_pixel(rgba, width, height, x + dx, y, [0, 0, 0, 255]);
    }
    put_rgba_pixel(rgba, width, height, x + 3, y + 18, [0, 0, 0, 255]);
    put_rgba_pixel(rgba, width, height, x + 4, y + 19, [0, 0, 0, 255]);
    put_rgba_pixel(rgba, width, height, x + 5, y + 20, [0, 0, 0, 255]);

    // White fill
    for dy in 1..cursor_h - 1 {
        let row_w = ((dy as f32 / cursor_h as f32) * cursor_w as f32).max(1.0) as i32;
        for dx in 1..row_w {
            put_rgba_pixel(rgba, width, height, x + dx, y + dy, [245, 245, 245, 255]);
        }
    }

    // Small inner gray detail near the tail for realism
    put_rgba_pixel(rgba, width, height, x + 4, y + 17, [170, 170, 170, 255]);
    put_rgba_pixel(rgba, width, height, x + 5, y + 18, [170, 170, 170, 255]);
}

#[derive(Deserialize, Debug)]
#[serde(tag = "t")]
enum RemoteControlCommand {
    #[serde(rename = "mouseMove")]
    MouseMove { x: f32, y: f32 },
    #[serde(rename = "mouseDown")]
    MouseDown { button: i32, x: f32, y: f32 },
    #[serde(rename = "mouseUp")]
    MouseUp { button: i32, x: f32, y: f32 },
    #[serde(rename = "mouseWheel")]
    MouseWheel { #[serde(rename = "deltaY")] delta_y: f32, x: f32, y: f32 },
    #[serde(rename = "keyDown")]
    KeyDown { key: String, code: String },
    #[serde(rename = "keyUp")]
    KeyUp { key: String, code: String },
}

#[cfg(target_os = "windows")]
mod remote_input {
    use super::RemoteControlCommand;
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, INPUT_KEYBOARD, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_MOVE, MOUSEEVENTF_ABSOLUTE, MOUSE_EVENT_FLAGS,
        MOUSEINPUT, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_MENU, VK_SHIFT, BlockInput,
        VIRTUAL_KEY, MapVirtualKeyW, MAPVK_VK_TO_VSC,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    #[derive(Default)]
    struct InputState {
        pressed_keys: HashSet<u16>,
        pressed_buttons: HashSet<i32>,
        control_active: bool,
    }

    static INPUT_STATE: OnceLock<Mutex<InputState>> = OnceLock::new();

    fn state() -> &'static Mutex<InputState> {
        INPUT_STATE.get_or_init(|| Mutex::new(InputState::default()))
    }

    fn clamp01(v: f32) -> f32 {
        if v < 0.0 { 0.0 } else if v > 1.0 { 1.0 } else { v }
    }

    fn to_abs_coords(x: f32, y: f32) -> (i32, i32) {
        let width = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(1);
        let height = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(1);
        let nx = (clamp01(x) * 65535.0) as i32;
        let ny = (clamp01(y) * 65535.0) as i32;
        let _ = (width, height);
        (nx, ny)
    }

    fn send_mouse(flags: MOUSE_EVENT_FLAGS, x: i32, y: i32, data: i32) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: x,
                    dy: y,
                    mouseData: data as u32,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe { let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32); }
    }

    fn send_key(vk: u16, key_up: bool) {
        let flags = if key_up { KEYEVENTF_KEYUP } else { Default::default() };
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16 },
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe { let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32); }
    }

    fn release_all_locked() {
        // Release any keys/buttons we previously pressed so the user's machine isn't left "stuck".
        let mut st = state().lock().unwrap();

        for vk in st.pressed_keys.drain() {
            send_key(vk, true);
        }

        // Release mouse buttons (best-effort).
        for b in st.pressed_buttons.drain() {
            let flag = match b { 2 => MOUSEEVENTF_RIGHTUP, 1 => MOUSEEVENTF_MIDDLEUP, _ => MOUSEEVENTF_LEFTUP };
            send_mouse(flag, 0, 0, 0);
        }
    }

    pub fn set_control_active(active: bool) {
        {
            let mut st = state().lock().unwrap();
            if st.control_active == active {
                return;
            }
            st.control_active = active;
        }

        // BlockInput blocks physical mouse/keyboard from the local user (but still allows SendInput).
        unsafe {
            let _ = BlockInput(active);
        }

        if !active {
            release_all_locked();
            unsafe { let _ = BlockInput(false); }
        }
    }

    fn key_to_vk(code: &str, key: &str) -> Option<u16> {
        let c = code.to_uppercase();
        match c.as_str() {
            "SHIFTLEFT" | "SHIFTRIGHT" => Some(VK_SHIFT.0),
            "CONTROLLEFT" | "CONTROLRIGHT" => Some(VK_CONTROL.0),
            "ALTLEFT" | "ALTRIGHT" => Some(VK_MENU.0),
            _ => {
                let k = if key.len() == 1 { key.chars().next()?.to_ascii_uppercase() as u16 } else { 0 };
                if k >= 0x30 && k <= 0x5A { Some(k) } else { None }
            }
        }
    }

    pub fn execute(cmd: RemoteControlCommand) {
        match cmd {
            RemoteControlCommand::MouseMove { x, y } => {
                let (ax, ay) = to_abs_coords(x, y);
                send_mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, ax, ay, 0);
            }
            RemoteControlCommand::MouseDown { button, x, y } => {
                let (ax, ay) = to_abs_coords(x, y);
                send_mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, ax, ay, 0);
                let flag = match button { 2 => MOUSEEVENTF_RIGHTDOWN, 1 => MOUSEEVENTF_MIDDLEDOWN, _ => MOUSEEVENTF_LEFTDOWN };
                send_mouse(flag, 0, 0, 0);
                let mut st = state().lock().unwrap();
                st.pressed_buttons.insert(button);
            }
            RemoteControlCommand::MouseUp { button, x, y } => {
                let (ax, ay) = to_abs_coords(x, y);
                send_mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, ax, ay, 0);
                let flag = match button { 2 => MOUSEEVENTF_RIGHTUP, 1 => MOUSEEVENTF_MIDDLEUP, _ => MOUSEEVENTF_LEFTUP };
                send_mouse(flag, 0, 0, 0);
                let mut st = state().lock().unwrap();
                st.pressed_buttons.remove(&button);
            }
            RemoteControlCommand::MouseWheel { delta_y, x, y } => {
                let (ax, ay) = to_abs_coords(x, y);
                send_mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, ax, ay, 0);
                send_mouse(MOUSEEVENTF_WHEEL, 0, 0, (-delta_y as i32).clamp(-240, 240));
            }
            RemoteControlCommand::KeyDown { key, code } => {
                if let Some(vk) = key_to_vk(&code, &key) {
                    send_key(vk, false);
                    let mut st = state().lock().unwrap();
                    st.pressed_keys.insert(vk);
                }
            }
            RemoteControlCommand::KeyUp { key, code } => {
                if let Some(vk) = key_to_vk(&code, &key) {
                    send_key(vk, true);
                    let mut st = state().lock().unwrap();
                    st.pressed_keys.remove(&vk);
                }
            }
        }
    }
}

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
    /// Resolve the window title and executable filename from an HWND.
    fn get_window_info(hwnd: HWND) -> (String, String) {
        // --- title ---
        let mut title_buf = [0u16; 512];
        let title_len = unsafe { GetWindowTextW(hwnd, &mut title_buf) };
        let mut title = String::from_utf16_lossy(&title_buf[..title_len as usize]);

        // --- app name (exe filename) ---
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };

        let mut app_name = if pid != 0 {
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

        // Clean up app_name: remove .exe extension and capitalize
        if app_name.to_lowercase().ends_with(".exe") {
            app_name = app_name[..app_name.len() - 4].to_string();
        }
        if !app_name.is_empty() {
             let mut chars = app_name.chars();
             app_name = match chars.next() {
                 None => String::new(),
                 Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
             };
        }

        // Special case: if title is empty or just whitespace, use app_name
        if title.trim().is_empty() {
            title = app_name.clone();
        }

        (title, app_name)
    }

    /// List of system titles to ignore to avoid noise in logs
    const IGNORE_TITLES: &[&str] = &[
        "Task Switching",
        "Task View",
        "Task Manager",
        "Windows Input Experience",
        "Program Manager",
        "Start",
        "Search",
    ];

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

        // 1. Emit log for the window that just lost focus (if it's not empty/noise)
        if !state.last_title.is_empty() {
            let secs = now.signed_duration_since(state.start_time).num_seconds();
            if secs > 0 {
                let log = ActivityLog {
                    user_id: state.user_id.clone(),
                    title: state.last_title.clone(),
                    app_name: state.last_app.clone(),
                    start_time: state.start_time.to_rfc3339(),
                    duration_seconds: secs as u64,
                };

                // Notify UI if open
                let _ = handle.emit("new-activity-log", log.clone());

                // Report to DB (from background)
                post_activity_log(log);
            }
        }

        // 2. Update state to the new foreground window
        // If it's a noise window or empty, we set it to empty so it doesn't get logged next time
        let is_noise = title.is_empty() || IGNORE_TITLES.iter().any(|&s| title == s);
        if is_noise {
            state.last_title = String::new();
            state.last_app = String::new();
        } else {
            state.last_title = title;
            state.last_app = app_name;
        }
        state.start_time = now;
    }

    /// POST an activity log to the dashboard.
    fn post_activity_log(log: ActivityLog) {
        let client = match CLIENT.get() {
            Some(c) => c.clone(),
            None => return,
        };
        tauri::async_runtime::spawn(async move {
            let url = "https://cus-monitor-dashboard.vercel.app/api/activity-log";
            let _ = client.post(url).json(&log).send().await;
        });
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

#[tauri::command]
fn set_active_user(user_id: String, username: Option<String>) {
    if let Some(state_arc) = TRACKING_STATE.get() {
        let mut state = state_arc.lock().unwrap();
        state.user_id = user_id.clone();
        if let Some(uname) = username {
            state.username = uname.clone();
            info!("Rust State Updated: User ID={}, Username={}", user_id, uname);
        } else {
            info!("Rust State Updated: User ID={}", user_id);
        }
    }
}

// ─── App entry point ──────────────────────────────────────────────────────────
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Use whoami for more reliable username detection in production/admin environments
    let user_id = whoami::username();
    LAST_INPUT_TS.store(Utc::now().timestamp(), Ordering::Relaxed);

    let tracking_state = Arc::new(Mutex::new(TrackingState {
        last_title: String::new(),
        last_app: String::new(),
        start_time: Utc::now(),
        user_id: user_id.clone(),
        username: user_id, // Default to OS username; overridden by set_active_user after login
    }));

    // Store in global so the static hook callback can reach it
    TRACKING_STATE.set(tracking_state.clone()).ok();

    let state_for_exit = tracking_state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new()
            .targets([
                Target::new(TargetKind::Stdout),
                Target::new(TargetKind::LogDir { file_name: Some("cus-monitor.log".to_string()) }), // Corrected type to Option<String>
                Target::new(TargetKind::Webview),
            ])
            .build())
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
        .invoke_handler(tauri::generate_handler![greet, set_active_user])
        .setup(|app| {
            // Taskkill sidecar is removed as we no longer use it

            // Store AppHandle and Client globally (with User-Agent for WAF compatibility)
            APP_HANDLE.set(app.handle().clone()).ok();
            CLIENT.set(Client::builder().user_agent("CUS-Monitor-App").build().unwrap()).ok();

            // Force autostart to be enabled every time the app starts
            let _ = app.handle().autolaunch().enable();

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
                    // Don't restart in the user's face. Wait until the PC is idle,
                    // then relaunch into the updated version.
                    loop {
                        let last_input = LAST_INPUT_TS.load(Ordering::Relaxed);
                        let idle_seconds = (Utc::now().timestamp() - last_input).max(0);
                        if idle_seconds >= 300 {
                            let _ = handle.restart();
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            });

            // ─── Activity-aware heartbeat loop ─────────────────────────────────────
            // Runs every 1 s, classifies each second as productive/unproductive
            // based on keyboard/mouse activity, then emits heartbeat every 120 s.
            let heartbeat_handle = app.handle().clone();
            let hb_user_id = whoami::username();
            let session_id = format!("session-{}", Utc::now().timestamp());

            tauri::async_runtime::spawn(async move {
                let device_state = DeviceState::new();
                const HEARTBEAT_INTERVAL_SECS: u64 = 120; // 2 minutes
                const SCREENSHOT_INTERVAL_SECS: u64 = 15 * 60; // 15 minutes

                let mut productive_acc: u64 = 0;
                let mut unproductive_acc: u64 = 0;
                let mut last_heartbeat = Instant::now();
                let mut last_mouse_coords = (0i32, 0i32);
                let mut screenshot_elapsed_secs: u64 = 0;

                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;

                    // Check system-wide input (keyboard + mouse movement/clicks)
                    let keys  = device_state.get_keys();
                    let mouse = device_state.get_mouse();
                    let coords = mouse.coords;
                    let has_activity = !keys.is_empty()
                        || coords != last_mouse_coords
                        || mouse.button_pressed.iter().any(|&p| p);

                    // Productive if any keyboard/mouse activity occurred in this second.
                    if has_activity {
                        productive_acc += 1;
                        LAST_INPUT_TS.store(Utc::now().timestamp(), Ordering::Relaxed);
                    } else {
                        unproductive_acc += 1;
                    }
                    last_mouse_coords = coords;

                    // Report heartbeat directly from background every 2 minutes
                    if last_heartbeat.elapsed().as_secs() >= HEARTBEAT_INTERVAL_SECS {
                        // Get latest user_id from state
                        let current_user_id = if let Some(s) = TRACKING_STATE.get() {
                            s.lock().unwrap().user_id.clone()
                        } else {
                            hb_user_id.clone()
                        };

                        let payload = serde_json::json!({
                            "userId": current_user_id,
                            "sessionId": &session_id,
                            "productiveSeconds": productive_acc,
                            "unproductiveSeconds": unproductive_acc,
                            "timestamp": Utc::now().to_rfc3339()
                        });

                        // 1. Notify UI
                        let _ = heartbeat_handle.emit("send-heartbeat", payload.clone());

                        // 2. Report heartbeat to DB
                        let client = CLIENT.get().unwrap().clone();
                        let hb_payload = payload.clone();
                        tauri::async_runtime::spawn(async move {
                            let url = "https://cus-monitor-dashboard.vercel.app/api/track/heartbeat";
                            let _ = client.post(url).json(&hb_payload).send().await;
                        });

                        // 3. Trigger screenshot every 15 minutes total runtime
                        screenshot_elapsed_secs += HEARTBEAT_INTERVAL_SECS;
                        if screenshot_elapsed_secs >= SCREENSHOT_INTERVAL_SECS {
                            let screenshot_client = CLIENT.get().unwrap().clone();
                            let capture_user_id = current_user_id.clone();
                            let capture_session_id = session_id.clone();
                            tauri::async_runtime::spawn(async move {
                                info!("[Screenshot] Attempting to capture screen for user {}...", capture_user_id);
                                match Screen::all() {
                                    Ok(screens) => {
                                        if let Some(screen) = screens.first() {
                                            match screen.capture() {
                                                Ok(image) => {
                                                    info!("[Screenshot] Captured successfully ({}x{}). Encoding...", image.width(), image.height());
                                                    let width = image.width();
                                                    let height = image.height();
                                                    let rgba_buffer = image.into_raw();
                                                    
                                                    // Convert RGBA to RGB for JPEG encoding
                                                    let mut rgb_buffer = Vec::with_capacity((width * height * 3) as usize);
                                                    for chunk in rgba_buffer.chunks_exact(4) {
                                                        rgb_buffer.push(chunk[0]); // R
                                                        rgb_buffer.push(chunk[1]); // G
                                                        rgb_buffer.push(chunk[2]); // B
                                                    }
                                                    
                                                    let mut jpeg_buffer = Vec::new();
                                                    let mut encoder = JpegEncoder::new(&mut jpeg_buffer);
                                                    
                                                    match encoder.encode(&rgb_buffer, width, height, ColorType::Rgb8.into()) {
                                                        Ok(_) => {
                                                            let base64_image = general_purpose::STANDARD.encode(&jpeg_buffer);
                                                            info!("[Screenshot] Encoded to base64. Size: {} bytes. Uploading...", base64_image.len());
                                                            
                                                            let payload = serde_json::json!({
                                                                "userId": capture_user_id,
                                                                "sessionId": capture_session_id,
                                                                "image": format!("data:image/jpeg;base64,{}", base64_image),
                                                                "timestamp": Utc::now().to_rfc3339()
                                                            });
                                                            
                                                            let url = "https://cus-monitor-dashboard.vercel.app/api/track/upload-screenshot";
                                                            match screenshot_client.post(url).json(&payload).send().await {
                                                                Ok(resp) => {
                                                                    if resp.status().is_success() {
                                                                        info!("[Screenshot] Uploaded natively! Status: {}", resp.status());
                                                                    } else {
                                                                        error!("[Screenshot] Upload failed with status: {}", resp.status());
                                                                    }
                                                                },
                                                                Err(e) => error!("[Screenshot] Network error during upload: {}", e),
                                                            }
                                                        },
                                                        Err(e) => error!("[Screenshot] Failed to encode JPEG: {:?}", e),
                                                    }
                                                },
                                                Err(e) => error!("[Screenshot] Failed to capture screen: {:?}", e),
                                            }
                                        } else {
                                            error!("[Screenshot] No active screens found to capture.");
                                        }
                                    },
                                    Err(e) => error!("[Screenshot] Failed to get screens: {:?}", e),
                                }
                            });
                            screenshot_elapsed_secs = 0;
                        }

                        productive_acc   = 0;
                        unproductive_acc = 0;
                        last_heartbeat   = Instant::now();
                    }
                }
            });

            // ─── Live Stream Room Logic (LiveKit) ──────────────────────────────
            tauri::async_runtime::spawn(async move {
                #[derive(Clone, Debug)]
                struct StreamStatus {
                    active: bool,
                    control_enabled: bool,
                    controller_id: String,
                }

                let (status_tx, mut status_rx) = tokio::sync::watch::channel(StreamStatus {
                    active: false,
                    control_enabled: false,
                    controller_id: String::new(),
                });
                let poll_tx = status_tx.clone();

                // 1. Status Polling Task (fast, 1s interval so control release feels instant)
                tauri::async_runtime::spawn(async move {
                    info!("[LiveKit] Polling task started (1s interval)");
                    let mut last_control_enabled = false;
                    loop {
                        let (_uid, uname) = if let Some(s) = TRACKING_STATE.get() {
                            let state = s.lock().unwrap();
                            (state.user_id.clone(), state.username.clone())
                        } else {
                            let u = whoami::username();
                            (u.clone(), u)
                        };

                        let stream_client = match CLIENT.get() {
                            Some(c) => c.clone(),
                            None => {
                                tokio::time::sleep(Duration::from_secs(5)).await;
                                continue;
                            }
                        };

                        let status_url = format!(
                            "https://cus-monitor-dashboard.vercel.app/api/stream?action=status&userId={}",
                            uname
                        );

                        match stream_client
                            .get(&status_url)
                            .header("x-client-type", "agent")
                            .send()
                            .await
                        {
                            Ok(res) => {
                                if let Ok(json) = res.json::<serde_json::Value>().await {
                                    // Livestream must not be blocked by productivity/inactivity heuristics.
                                    // Server decides whether stream is toggled on; "PC active" is reported separately.
                                    let active = json.get("isActive").and_then(|v| v.as_bool()).unwrap_or(false);
                                    let control_enabled = json.get("controlEnabled").and_then(|v| v.as_bool()).unwrap_or(false);
                                    let controller_id = json.get("controllerId").and_then(|v| v.as_str()).unwrap_or("").to_string();

                                    // Apply local input lock/unlock immediately on poll changes.
                                    #[cfg(target_os = "windows")]
                                    {
                                        if control_enabled != last_control_enabled {
                                            remote_input::set_control_active(control_enabled);
                                            last_control_enabled = control_enabled;
                                        }
                                    }

                                    let _ = poll_tx.send(StreamStatus { active, control_enabled, controller_id });
                                }
                            }
                            Err(e) => error!("[LiveKit] Status poll error: {}", e),
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                });

                // 2. Stream Controller Task
                loop {
                    // Wait for isActive to become true
                    if !status_rx.borrow().active {
                        if let Err(_) = status_rx.changed().await { break; }
                        if !status_rx.borrow().active { continue; }
                    }

                    // START STREAMING
                    let uname = if let Some(s) = TRACKING_STATE.get() {
                        s.lock().unwrap().username.clone()
                    } else {
                        whoami::username()
                    };

                    info!("[LiveKit] 🚀 Activating stream for: {}", uname);

                    let token_url = format!(
                        "https://cus-monitor-dashboard.vercel.app/api/stream/token?room=room_{}&identity=agent_{}&clientType=agent",
                        uname, uname
                    );

                    let stream_client = CLIENT.get().unwrap().clone();
                    if let Ok(res) = stream_client.get(&token_url).send().await {
                        if let Ok(data) = res.json::<serde_json::Value>().await {
                            let token = data["token"].as_str().unwrap_or_default();
                            let url = data["url"].as_str().unwrap_or_default();

                            if !token.is_empty() && !url.is_empty() {
                                match Room::connect(url, token, RoomOptions::default()).await {
                                    Ok((room, mut events)) => {
                                        info!("[LiveKit] ✅ Connected to room_v{}", uname);
                                        
                                        // 1. START EVENT LOOP IMMEDIATELY (Crucial for negotiation)
                                        let evt_status_rx = status_tx.subscribe();
                                        tauri::async_runtime::spawn(async move {
                                            info!("[LiveKit] Room event task starting...");
                                            info!("[LiveKit] Event task started");
                                            while evt_status_rx.borrow().active {
                                                match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
                                                    Ok(Some(event)) => {
                                                        match event {
                                                            RoomEvent::DataReceived { payload, participant, topic, .. } => {
                                                                // Only accept remote-control packets when server says control is enabled,
                                                                // and only from the current controller (admin has priority).
                                                                let st = evt_status_rx.borrow().clone();
                                                                if !st.control_enabled {
                                                                    continue;
                                                                }
                                                                if topic.as_deref().unwrap_or("") != "remote-control" {
                                                                    continue;
                                                                }
                                                                let sender_identity = participant
                                                                    .as_ref()
                                                                    .map(|p| p.identity().to_string())
                                                                    .unwrap_or_default();
                                                                if !st.controller_id.is_empty() && sender_identity != st.controller_id {
                                                                    continue;
                                                                }

                                                                let payload_str = String::from_utf8_lossy(&payload).to_string();
                                                                match serde_json::from_str::<RemoteControlCommand>(&payload_str) {
                                                                    Ok(cmd) => {
                                                                        #[cfg(target_os = "windows")]
                                                                        {
                                                                            remote_input::execute(cmd);
                                                                        }
                                                                    }
                                                                    Err(e) => warn!("[LiveKit] Invalid control command: {}", e),
                                                                }
                                                            }
                                                            RoomEvent::TrackPublished { publication, participant } => {
                                                                info!("[LiveKit] Event: Track Published! SID: {}, Participant: {}", publication.sid(), participant.identity());
                                                            },
                                                            RoomEvent::ParticipantConnected(participant) => {
                                                                info!("[LiveKit] Event: Participant Connected: {}", participant.identity());
                                                            },
                                                            RoomEvent::Disconnected { reason } => {
                                                                warn!("[LiveKit] Event: Disconnected from room: {:?}", reason);
                                                            },
                                                            _ => debug!("[LiveKit] Other Room Event: {:?}", event),
                                                        }
                                                    }
                                                    Ok(None) => {
                                                        info!("[LiveKit] Room event stream closed.");
                                                        break;
                                                    }
                                                    Err(_) => {} // Timeout – check borrow() and loop
                                                }
                                            }
                                            // Ensure we never leave the user input blocked if we exit this task.
                                            #[cfg(target_os = "windows")]
                                            {
                                                remote_input::set_control_active(false);
                                            }
                                            info!("[LiveKit] Event task stopped");
                                        });

                                        // 2. Create source and publish track
                                        // Prefer full-HD source for sharper remote readability.
                                        let native_source = NativeVideoSource::new(VideoResolution { width: 1920, height: 1080 }, true);
                                        let track = LocalVideoTrack::create_video_track("screen", RtcVideoSource::Native(native_source.clone()));
                                        
                                        info!("[LiveKit] Publishing video track...");
                                        match room.local_participant().publish_track(LocalTrack::Video(track), Default::default()).await {
                                            Ok(_) => {
                                                info!("[LiveKit] 📡 Video track published!");

                                                // 3. START CAPTURE LOOP
                                                let cap_status_rx = status_tx.subscribe();
                                                let cap_source = native_source.clone();
                                                tauri::async_runtime::spawn(async move {
                                                    info!("[LiveKit] Capture task started");
                                                    let device_state = DeviceState::new();
                                                    const TARGET_FRAME_MS: u64 = 33; // ~30 FPS
                                                    while cap_status_rx.borrow().active {
                                                        let start = Instant::now();

                                                        if let Ok(screens) = Screen::all() {
                                                            if let Some(screen) = screens.first() {
                                                                if let Ok(img) = screen.capture() {
                                                                    let orig_w = img.width();
                                                                    let mut w = orig_w;
                                                                    let mut h = img.height();
                                                                    
                                                                    // Ensure even dimensions for I420 consistency
                                                                    if w % 2 != 0 { w -= 1; }
                                                                    if h % 2 != 0 { h -= 1; }

                                                                    let mut rgba = img.into_raw();
                                                                    let mouse = device_state.get_mouse();
                                                                    // For now, we assume the primary monitor starts at (0,0).
                                                                    // Coordinates are safely clamped by put_rgba_pixel.
                                                                    draw_cursor_on_frame(
                                                                        &mut rgba,
                                                                        w,
                                                                        h,
                                                                        mouse.coords.0,
                                                                        mouse.coords.1,
                                                                    );
                                                                    let mut i420 = I420Buffer::new(w, h);
                                                                    let (stride_y, stride_u, stride_v) = i420.strides();
                                                                    let (data_y, data_u, data_v) = i420.data_mut();

                                                                    let src_w = orig_w as usize;

                                                                    for y in 0..h as usize {
                                                                        for x in 0..w as usize {
                                                                            let i = (y * src_w + x) * 4;
                                                                            let r = rgba[i] as i32;
                                                                            let g = rgba[i+1] as i32;
                                                                            let b = rgba[i+2] as i32;

                                                                            // Y = 0.299R + 0.587G + 0.114B (scaled by 256)
                                                                            data_y[y * stride_y as usize + x] = ((77 * r + 150 * g + 29 * b) >> 8) as u8;

                                                                            if y % 2 == 0 && x % 2 == 0 {
                                                                                // U = -0.169R - 0.331G + 0.500B + 128
                                                                                data_u[(y/2) * stride_u as usize + (x/2)] = ((-43 * r - 84 * g + 127 * b + 32768) >> 8) as u8;
                                                                                // V = 0.500R - 0.419G - 0.081B + 128
                                                                                data_v[(y/2) * stride_v as usize + (x/2)] = ((127 * r - 106 * g - 21 * b + 32768) >> 8) as u8;
                                                                            }
                                                                        }
                                                                    }
                                                                    cap_source.capture_frame(&VideoFrame {
                                                                        timestamp_us: Utc::now().timestamp_micros(),
                                                                        rotation: VideoRotation::VideoRotation0,
                                                                        buffer: i420,
                                                                    });
                                                                }
                                                            }
                                                        }
                                                        let elapsed = start.elapsed();
                                                        if elapsed < Duration::from_millis(TARGET_FRAME_MS) {
                                                            tokio::time::sleep(Duration::from_millis(TARGET_FRAME_MS) - elapsed).await;
                                                        }
                                                    }
                                                    info!("[LiveKit] Capture task stopped");
                                                });

                                                // 4. WAIT FOR DEACTIVATION (checked every 10s)
                                                while status_rx.borrow().active {
                                                    tokio::time::sleep(Duration::from_secs(10)).await;
                                                }

                                                info!("[LiveKit] ⏹ Deactivating. Closing room...");
                                                #[cfg(target_os = "windows")]
                                                {
                                                    remote_input::set_control_active(false);
                                                }
                                                let _ = room.close().await;
                                            }
                                            Err(e) => error!("[LiveKit] ❌ Failed to publish track: {:?}", e),
                                        }
                                    }
                                    Err(e) => error!("[LiveKit] Connection failed: {}", e),
                                }
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
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
                        let log = ActivityLog {
                            user_id: s.user_id.clone(),
                            title: s.last_title.clone(),
                            app_name: s.last_app.clone(),
                            start_time: s.start_time.to_rfc3339(),
                            duration_seconds: secs as u64,
                        };

                        // UI emit
                        let _ = handle.emit("new-activity-log", log.clone());

                        // Critical flush to DB (might be slightly jittery on exit,
                        // but reqwest will try to fire it off)
                        if let Some(client) = CLIENT.get() {
                            let client = client.clone();
                            tauri::async_runtime::spawn(async move {
                                let url = "https://cus-monitor-dashboard.vercel.app/api/activity-log";
                                let _ = client.post(url).json(&log).send().await;
                            });
                        }
                    }
                }
            }
        });
}