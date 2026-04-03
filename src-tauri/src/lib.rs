use tauri::{Manager, Emitter, RunEvent};
use std::time::{Duration, Instant};
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
            println!("Rust State Updated: User ID={}, Username={}", user_id, uname);
        } else {
            println!("Rust State Updated: User ID={}", user_id);
        }
    }
}

// ─── App entry point ──────────────────────────────────────────────────────────
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Use whoami for more reliable username detection in production/admin environments
    let user_id = whoami::username();

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
                let mut screenshot_counter: u32 = 0; // New counter for screenshots

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

                    // Report heartbeat directly from background every 1 min
                    if last_heartbeat.elapsed().as_secs() >= 60 {
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

                        // 3. Trigger Screenshot natively every 15 minutes (every 15 × 60s heartbeats)
                        screenshot_counter += 1;
                        if screenshot_counter >= 15 {
                            let screenshot_client = CLIENT.get().unwrap().clone();
                            let capture_user_id = current_user_id.clone();
                            let capture_session_id = session_id.clone();
                            tauri::async_runtime::spawn(async move {
                                println!("[Screenshot] Attempting to capture screen for user {}...", capture_user_id);
                                match Screen::all() {
                                    Ok(screens) => {
                                        if let Some(screen) = screens.first() {
                                            match screen.capture() {
                                                Ok(image) => {
                                                    println!("[Screenshot] Captured successfully ({}x{}). Encoding...", image.width(), image.height());
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
                                                            println!("[Screenshot] Encoded to base64. Size: {} bytes. Uploading...", base64_image.len());
                                                            
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
                                                                        println!("[Screenshot] Uploaded natively! Status: {}", resp.status());
                                                                    } else {
                                                                        eprintln!("[Screenshot] Upload failed with status: {}", resp.status());
                                                                    }
                                                                },
                                                                Err(e) => eprintln!("[Screenshot] Network error during upload: {}", e),
                                                            }
                                                        },
                                                        Err(e) => eprintln!("[Screenshot] Failed to encode JPEG: {:?}", e),
                                                    }
                                                },
                                                Err(e) => eprintln!("[Screenshot] Failed to capture screen: {:?}", e),
                                            }
                                        } else {
                                            eprintln!("[Screenshot] No active screens found to capture.");
                                        }
                                    },
                                    Err(e) => eprintln!("[Screenshot] Failed to get screens: {:?}", e),
                                }
                            });
                            screenshot_counter = 0; // Reset counter
                        }

                        productive_acc   = 0;
                        unproductive_acc = 0;
                        last_heartbeat   = Instant::now();
                    }
                }
            });

            // ─── Live Stream Room Logic (LiveKit) ──────────────────────────────
            tauri::async_runtime::spawn(async move {
                let mut current_room: Option<Room> = Option::None;
                let mut video_source: Option<NativeVideoSource> = Option::None;
                let mut is_active = false;

                println!("[LiveKit] Stream controller started — idle polling every 30 s");

                loop {
                    // Timing: 30s poll when idle, 100ms when publishing (high FPS)
                    let sleep_ms = if is_active { 100 } else { 30_000 };
                    tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

                    // 1. Resolve identifiers
                    let (uid, uname) = if let Some(s) = TRACKING_STATE.get() {
                        let state = s.lock().unwrap();
                        (state.user_id.clone(), state.username.clone())
                    } else {
                        let u = whoami::username();
                        (u.clone(), u)
                    };

                    let stream_client = match CLIENT.get() {
                        Some(c) => c.clone(),
                        None => continue,
                    };

                    // 2. Check if admin requested stream
                    let status_url = format!(
                        "https://cus-monitor-dashboard.vercel.app/api/stream?action=status&userId={}",
                        uname
                    );
                    
                    let mut requested = false;
                    if let Ok(res) = stream_client.get(&status_url).send().await {
                        if let Ok(json) = res.json::<serde_json::Value>().await {
                            requested = json.get("isActive").and_then(|v| v.as_bool()).unwrap_or(false);
                        }
                    }

                    // 3. Handle connection state changes
                    if requested && !is_active {
                        // START STEAMING
                        println!("[LiveKit] 🚀 Request received. Fetching token...");
                        let token_url = format!(
                            "https://cus-monitor-dashboard.vercel.app/api/stream/token?room=room_{}&identity={}",
                            uname, uname
                        );

                        if let Ok(res) = stream_client.get(&token_url).send().await {
                            if let Ok(data) = res.json::<serde_json::Value>().await {
                                let token = data["token"].as_str().unwrap_or_default();
                                let url = data["url"].as_str().unwrap_or_default();

                                if !token.is_empty() && !url.is_empty() {
                                    match Room::connect(url, token, RoomOptions::default()).await {
                                        Ok((room, mut events)) => {
                                            println!("[LiveKit] ✅ Connected to room: room_{}", uname);
                                            
                                            // Create video source and track
                                            let native_source = NativeVideoSource::new(VideoResolution { width: 1280, height: 720 }, true);
                                            let track = LocalVideoTrack::create_video_track("screen", RtcVideoSource::Native(native_source.clone()));
                                            
                                            // Publish track
                                            match room.local_participant().publish_track(
                                                LocalTrack::Video(track),
                                                Default::default()
                                            ).await {
                                                Ok(_) => {
                                                    println!("[LiveKit] 📡 Video track published!");
                                                    current_room = Some(room);
                                                    video_source = Some(native_source);
                                                    is_active = true;
                                                }
                                                Err(e) => eprintln!("[LiveKit] ❌ Failed to publish track: {:?}", e),
                                            }
                                        }
                                        Err(e) => eprintln!("[LiveKit] ❌ Room connect error: {:?}", e),
                                    }
                                }
                            }
                        }
                    } else if !requested && is_active {
                        // STOP STREAMING
                        println!("[LiveKit] ⏹ Admin requested stop. Disconnecting...");
                        if let Some(room) = current_room.take() {
                            let _ = room.close().await;
                        }
                        video_source = None;
                        is_active = false;
                    }

                    // 4. Capture and Push Frame (if active)
                    if is_active {
                        if let (Some(source), Some(room)) = (&video_source, &current_room) {
                            if let Ok(screens) = Screen::all() {
                                if let Some(screen) = screens.first() {
                                    if let Ok(img) = screen.capture() {
                                        let w = img.width();
                                        let h = img.height();
                                        let rgba = img.into_raw();

                                        // Convert RGBA to I420
                                        let mut i420 = I420Buffer::new(w, h);
                                        let (stride_y, stride_u, stride_v) = i420.strides();
                                        let (data_y, data_u, data_v) = i420.data_mut();

                                        for y in 0..h as usize {
                                            for x in 0..w as usize {
                                                let i = (y * w as usize + x) * 4;
                                                let r = rgba[i] as f32;
                                                let g = rgba[i+1] as f32;
                                                let b = rgba[i+2] as f32;

                                                data_y[y * stride_y as usize + x] = (0.299 * r + 0.587 * g + 0.114 * b) as u8;

                                                if y % 2 == 0 && x % 2 == 0 {
                                                    let u = -0.169 * r - 0.331 * g + 0.5 * b + 128.0;
                                                    let v = 0.5 * r - 0.419 * g - 0.081 * b + 128.0;
                                                    data_u[(y/2) * stride_u as usize + (x/2)] = u as u8;
                                                    data_v[(y/2) * stride_v as usize + (x/2)] = v as u8;
                                                }
                                            }
                                        }

                                        let frame = VideoFrame {
                                            timestamp_us: Instant::now().elapsed().as_micros() as i64,
                                            rotation: VideoRotation::VideoRotation0,
                                            buffer: i420,
                                        };

                                        source.capture_frame(&frame);
                                    }
                                }
                            }
                        }
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