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

use log::{info, error};

mod p2p_webrtc;
mod supabase_signaling;
mod user_state;
mod watchdog;

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

#[derive(Deserialize, Debug)]
#[serde(tag = "t")]
pub(crate) enum RemoteControlCommand {
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
pub(crate) mod remote_input {
    use super::RemoteControlCommand;
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    use log::{info, warn};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, INPUT_KEYBOARD, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_MOVE, MOUSEEVENTF_ABSOLUTE, MOUSE_EVENT_FLAGS,
        MOUSEINPUT, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_MENU, VK_SHIFT, BlockInput,
        VIRTUAL_KEY, MapVirtualKeyW, MAPVK_VK_TO_VSC, VK_BACK, VK_TAB, VK_RETURN, VK_ESCAPE,
        VK_SPACE, VK_PRIOR, VK_NEXT, VK_END, VK_HOME, VK_LEFT, VK_UP, VK_RIGHT, VK_DOWN,
        VK_DELETE, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10,
        VK_F11, VK_F12, VK_OEM_PERIOD, VK_OEM_COMMA, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4,
        VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_MINUS, VK_OEM_PLUS, VK_CAPITAL, VK_INSERT,
        VK_NUMLOCK, VK_SCROLL, VK_PAUSE,
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

    pub fn is_control_active() -> bool {
        state().lock().unwrap().control_active
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
            let result = BlockInput(active);
            if result.is_ok() {
                info!("[remote_input] BlockInput({}) SUCCESS", active);
            } else {
                warn!("[remote_input] BlockInput({}) FAILED (likely lack of admin privileges)", active);
            }
        }

        if !active {
            release_all_locked();
        }
    }

    fn key_to_vk(code: &str, key: &str) -> Option<u16> {
        let c = code.to_uppercase();
        match c.as_str() {
            "SHIFTLEFT" | "SHIFTRIGHT" => Some(VK_SHIFT.0),
            "CONTROLLEFT" | "CONTROLRIGHT" => Some(VK_CONTROL.0),
            "ALTLEFT" | "ALTRIGHT" => Some(VK_MENU.0),
            "BACKSPACE" => Some(VK_BACK.0),
            "TAB" => Some(VK_TAB.0),
            "ENTER" => Some(VK_RETURN.0),
            "ESCAPE" => Some(VK_ESCAPE.0),
            "SPACE" => Some(VK_SPACE.0),
            "PAGEUP" => Some(VK_PRIOR.0),
            "PAGEDOWN" => Some(VK_NEXT.0),
            "END" => Some(VK_END.0),
            "HOME" => Some(VK_HOME.0),
            "ARROWLEFT" => Some(VK_LEFT.0),
            "ARROWUP" => Some(VK_UP.0),
            "ARROWRIGHT" => Some(VK_RIGHT.0),
            "ARROWDOWN" => Some(VK_DOWN.0),
            "DELETE" => Some(VK_DELETE.0),
            "F1" => Some(VK_F1.0),
            "F2" => Some(VK_F2.0),
            "F3" => Some(VK_F3.0),
            "F4" => Some(VK_F4.0),
            "F5" => Some(VK_F5.0),
            "F6" => Some(VK_F6.0),
            "F7" => Some(VK_F7.0),
            "F8" => Some(VK_F8.0),
            "F9" => Some(VK_F9.0),
            "F10" => Some(VK_F10.0),
            "F11" => Some(VK_F11.0),
            "F12" => Some(VK_F12.0),
            "CAPSLOCK" => Some(VK_CAPITAL.0),
            "INSERT" => Some(VK_INSERT.0),
            "NUMLOCK" => Some(VK_NUMLOCK.0),
            "SCROLLLOCK" => Some(VK_SCROLL.0),
            "PAUSE" => Some(VK_PAUSE.0),
            _ => {
                let k = if key.len() == 1 {
                    key.chars().next()?.to_ascii_uppercase() as u16
                } else {
                    0
                };
                if (k >= 0x30 && k <= 0x39) || (k >= 0x41 && k <= 0x5A) {
                    Some(k)
                } else if k != 0 {
                    // Map common symbols
                    match k as u8 {
                        b' ' => Some(VK_SPACE.0),
                        b'.' => Some(VK_OEM_PERIOD.0),
                        b',' => Some(VK_OEM_COMMA.0),
                        b';' => Some(VK_OEM_1.0),
                        b'/' => Some(VK_OEM_2.0),
                        b'`' => Some(VK_OEM_3.0),
                        b'[' => Some(VK_OEM_4.0),
                        b'\\' => Some(VK_OEM_5.0),
                        b']' => Some(VK_OEM_6.0),
                        b'\'' => Some(VK_OEM_7.0),
                        b'-' => Some(VK_OEM_MINUS.0),
                        b'=' => Some(VK_OEM_PLUS.0),
                        _ => None,
                    }
                } else {
                    None
                }
            }
        }
    }

    pub fn execute(cmd: RemoteControlCommand) {
        if !is_control_active() {
            return;
        }
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
        if watchdog::is_leader() && !state.last_title.is_empty() {
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
            let url = "https://app.custech.co/api/activity-log";
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
    let uname = username.clone().unwrap_or_default();
    user_state::save(&user_id, &uname);

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
    let args: Vec<String> = std::env::args().collect();
    let preferred_sibling = watchdog::parse_sibling_arg(&args);
    let start_minimized = watchdog::is_minimized_arg(&args);

    // Fresh user/autostart launch: clear a stale uninstall lock from a failed uninstall.
    if preferred_sibling.is_none() {
        let _ = std::fs::remove_file(watchdog::uninstall_lock_path());
    }

    // Triple-sibling claim: exit if the requested/available slot is taken.
    let Some(my_sibling) = watchdog::claim_sibling(preferred_sibling) else {
        info!("[watchdog] no free sibling slot — exiting");
        return;
    };
    watchdog::start(my_sibling);

    // Use whoami for more reliable username detection in production/admin environments
    let mut user_id = whoami::username();
    let mut username = user_id.clone();
    if let Some(persisted) = user_state::load() {
        if !persisted.user_id.is_empty() {
            user_id = persisted.user_id;
            username = if persisted.username.is_empty() {
                user_id.clone()
            } else {
                persisted.username
            };
            info!(
                "[user_state] restored user_id={} username={}",
                user_id, username
            );
        }
    }
    LAST_INPUT_TS.store(Utc::now().timestamp(), Ordering::Relaxed);

    let tracking_state = Arc::new(Mutex::new(TrackingState {
        last_title: String::new(),
        last_app: String::new(),
        start_time: Utc::now(),
        user_id: user_id.clone(),
        username,
    }));

    // Store in global so the static hook callback can reach it
    TRACKING_STATE.set(tracking_state.clone()).ok();

    let state_for_exit = tracking_state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new()
            .level(log::LevelFilter::Info)
            .targets([
                Target::new(TargetKind::Stdout),
                Target::new(TargetKind::LogDir { file_name: Some("cus-monitor.log".to_string()) }), // Corrected type to Option<String>
                Target::new(TargetKind::Webview),
            ])
            .build())
        // Note: single-instance plugin intentionally omitted so 3 siblings can run.
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![greet, set_active_user])
        .setup(move |app| {
            // Store AppHandle and Client globally (with User-Agent for WAF compatibility)
            APP_HANDLE.set(app.handle().clone()).ok();
            CLIENT.set(Client::builder().user_agent("CUS-Monitor-App").build().unwrap()).ok();

            // Hide UI for watchdog siblings / minimized launches
            if my_sibling != 0 || start_minimized {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            // Only sibling 0 manages autostart registration
            if my_sibling == 0 {
                let _ = app.handle().autolaunch().enable();
            }

            // Silent auto-updater (leader only)
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    if !watchdog::is_leader() {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        continue;
                    }
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
                            }
                            tokio::time::sleep(Duration::from_secs(30)).await;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                }
            });

            // ─── Activity-aware heartbeat loop ─────────────────────────────────────
            // Runs every 1 s, classifies each second as productive/unproductive
            // based on keyboard/mouse activity, then emits heartbeat every 120 s.
            let heartbeat_handle = app.handle().clone();
            let hb_user_id = user_id.clone();
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

                    if !watchdog::is_leader() {
                        // Reset accumulators while following so a takeover starts clean.
                        productive_acc = 0;
                        unproductive_acc = 0;
                        last_heartbeat = Instant::now();
                        continue;
                    }

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
                            let url = "https://app.custech.co/api/track/heartbeat";
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
                                                            
                                                            let url = "https://app.custech.co/api/track/upload-screenshot";
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

            // ─── P2P WebRTC Stream Logic (Supabase Signaling) ──────────────────
            tauri::async_runtime::spawn(async move {
                let (status_tx, mut status_rx) = tokio::sync::watch::channel(
                    p2p_webrtc::StreamStatus {
                        active: false,
                        control_enabled: false,
                        controller_id: String::new(),
                    },
                );
                let poll_tx = status_tx.clone();

                // 1. Status Polling Task (fast, 1s interval so control release feels instant)
                tauri::async_runtime::spawn(async move {
                    info!("[P2P] Polling task started (1s interval)");
                    let mut last_control_enabled = false;
                    loop {
                        if !watchdog::is_leader() {
                            #[cfg(target_os = "windows")]
                            {
                                if last_control_enabled {
                                    remote_input::set_control_active(false);
                                    last_control_enabled = false;
                                }
                            }
                            let _ = poll_tx.send(p2p_webrtc::StreamStatus {
                                active: false,
                                control_enabled: false,
                                controller_id: String::new(),
                            });
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }

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
                            "https://app.custech.co/api/stream?action=status&userId={}",
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
                                    let active = json.get("isActive").and_then(|v| v.as_bool()).unwrap_or(false);
                                    let control_enabled = json.get("controlEnabled").and_then(|v| v.as_bool()).unwrap_or(false);
                                    let controller_id = json.get("controllerId").and_then(|v| v.as_str()).unwrap_or("").to_string();

                                    #[cfg(target_os = "windows")]
                                    {
                                        if control_enabled != last_control_enabled {
                                            remote_input::set_control_active(control_enabled);
                                            last_control_enabled = control_enabled;
                                        }
                                    }

                                    let _ = poll_tx.send(p2p_webrtc::StreamStatus {
                                        active,
                                        control_enabled,
                                        controller_id,
                                    });
                                }
                            }
                            Err(e) => error!("[P2P] Status poll error: {}", e),
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                });

                // 2. Stream Controller Task
                loop {
                    if !status_rx.borrow().active {
                        if status_rx.changed().await.is_err() {
                            break;
                        }
                        if !status_rx.borrow().active {
                            continue;
                        }
                    }

                    if !watchdog::is_leader() {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue;
                    }

                    let uname = if let Some(s) = TRACKING_STATE.get() {
                        s.lock().unwrap().username.clone()
                    } else {
                        whoami::username()
                    };

                    info!("[P2P] Activating stream for: {}", uname);

                    let stream_client = CLIENT.get().unwrap().clone();
                    let config_url = "https://app.custech.co/api/stream/signaling-config";

                    match stream_client.get(config_url).send().await {
                        Ok(res) => {
                            if let Ok(config) = res.json::<p2p_webrtc::SignalingConfig>().await {
                                let stream_status_rx = status_tx.subscribe();
                                p2p_webrtc::run_p2p_streaming(
                                    uname.clone(),
                                    config.supabase_url,
                                    config.supabase_anon_key,
                                    stream_status_rx,
                                )
                                .await;
                            } else {
                                error!("[P2P] Failed to parse signaling config");
                            }
                        }
                        Err(e) => error!("[P2P] Failed to fetch signaling config: {}", e),
                    }

                    #[cfg(target_os = "windows")]
                    {
                        remote_input::set_control_active(false);
                    }

                    info!("[P2P] Stream session ended for {}", uname);
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
                if !watchdog::is_leader() {
                    return;
                }
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
                                let url = "https://app.custech.co/api/activity-log";
                                let _ = client.post(url).json(&log).send().await;
                            });
                        }
                    }
                }
            }
        });
}