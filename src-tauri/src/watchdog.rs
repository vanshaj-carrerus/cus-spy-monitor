//! Triple-sibling watchdog + leader election (Windows).
//!
//! Three processes (`--sibling 0|1|2`) keep each other alive. Only the elected
//! leader runs monitoring work so heartbeats/screenshots/streams stay single.

use log::{info, warn};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

#[cfg(target_os = "windows")]
use windows::core::HSTRING;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, GetLastError};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, MUTEX_MODIFY_STATE};

pub const SIBLING_COUNT: u8 = 3;
const UNINSTALL_LOCK_NAME: &str = "uninstall.lock";

static SIBLING_ID: AtomicU8 = AtomicU8::new(0);
static IS_LEADER: AtomicBool = AtomicBool::new(false);
static LEADER_HANDLE: OnceLock<MutexGuard> = OnceLock::new();
static SIBLING_HANDLE: OnceLock<MutexGuard> = OnceLock::new();

struct MutexGuard(#[cfg(target_os = "windows")] HANDLE);

#[cfg(target_os = "windows")]
unsafe impl Send for MutexGuard {}
#[cfg(target_os = "windows")]
unsafe impl Sync for MutexGuard {}

impl Drop for MutexGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join("cus-monitor");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn uninstall_lock_path() -> PathBuf {
    data_dir().join(UNINSTALL_LOCK_NAME)
}

pub fn uninstall_in_progress() -> bool {
    uninstall_lock_path().exists()
}

pub fn sibling_id() -> u8 {
    SIBLING_ID.load(Ordering::Relaxed)
}

pub fn is_leader() -> bool {
    IS_LEADER.load(Ordering::Relaxed)
}

fn mutex_prefix() -> &'static str {
    // Keep debug builds from colliding with the installed production app
    // (same mutex names would make `tauri dev` exit immediately).
    if cfg!(debug_assertions) {
        "Local\\CUSMonitorDev_"
    } else {
        "Local\\CUSMonitor_"
    }
}

fn sibling_mutex_name(id: u8) -> String {
    format!("{}Sibling_{}", mutex_prefix(), id)
}

fn leader_mutex_name() -> String {
    format!("{}Leader_v1", mutex_prefix())
}

#[cfg(target_os = "windows")]
fn try_create_mutex(name: &str) -> Option<HANDLE> {
    unsafe {
        match CreateMutexW(None, true, &HSTRING::from(name)) {
            Ok(handle) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    let _ = CloseHandle(handle);
                    None
                } else {
                    Some(handle)
                }
            }
            Err(_) => None,
        }
    }
}

#[cfg(target_os = "windows")]
fn mutex_exists(name: &str) -> bool {
    unsafe {
        match OpenMutexW(MUTEX_MODIFY_STATE, false, &HSTRING::from(name)) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn try_create_mutex(_name: &str) -> Option<()> {
    Some(())
}

#[cfg(not(target_os = "windows"))]
fn mutex_exists(_name: &str) -> bool {
    false
}

/// Parse `--sibling N` from argv. Returns None if not specified.
pub fn parse_sibling_arg(args: &[String]) -> Option<u8> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--sibling" {
            if let Some(v) = args.get(i + 1) {
                if let Ok(n) = v.parse::<u8>() {
                    if n < SIBLING_COUNT {
                        return Some(n);
                    }
                }
            }
        } else if let Some(rest) = a.strip_prefix("--sibling=") {
            if let Ok(n) = rest.parse::<u8>() {
                if n < SIBLING_COUNT {
                    return Some(n);
                }
            }
        }
        i += 1;
    }
    None
}

pub fn is_minimized_arg(args: &[String]) -> bool {
    args.iter().any(|a| a == "--minimized")
}

/// Claim a sibling slot. If `preferred` is set, only that slot is tried.
/// Returns sibling id or None if the slot(s) are taken.
pub fn claim_sibling(preferred: Option<u8>) -> Option<u8> {
    let candidates: Vec<u8> = match preferred {
        Some(n) => vec![n],
        None => (0..SIBLING_COUNT).collect(),
    };

    for id in candidates {
        #[cfg(target_os = "windows")]
        {
            if let Some(handle) = try_create_mutex(&sibling_mutex_name(id)) {
                SIBLING_ID.store(id, Ordering::Relaxed);
                let _ = SIBLING_HANDLE.set(MutexGuard(handle));
                info!("[watchdog] claimed sibling slot {}", id);
                return Some(id);
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            SIBLING_ID.store(id, Ordering::Relaxed);
            return Some(id);
        }
    }
    None
}

#[cfg(target_os = "windows")]
pub fn spawn_sibling(id: u8) {
    if uninstall_in_progress() {
        return;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            warn!("[watchdog] current_exe failed: {}", e);
            return;
        }
    };

    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    match std::process::Command::new(&exe)
        .args(["--sibling", &id.to_string(), "--minimized"])
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
    {
        Ok(_) => info!("[watchdog] spawned sibling {}", id),
        Err(e) => warn!("[watchdog] failed to spawn sibling {}: {}", id, e),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn spawn_sibling(_id: u8) {}

fn ensure_siblings_alive(my_id: u8) {
    if uninstall_in_progress() {
        return;
    }
    for id in 0..SIBLING_COUNT {
        if id == my_id {
            continue;
        }
        #[cfg(target_os = "windows")]
        {
            if !mutex_exists(&sibling_mutex_name(id)) {
                spawn_sibling(id);
            }
        }
    }
}

fn try_acquire_leadership() {
    #[cfg(target_os = "windows")]
    {
        if IS_LEADER.load(Ordering::Relaxed) {
            return;
        }
        if let Some(handle) = try_create_mutex(&leader_mutex_name()) {
            let _ = LEADER_HANDLE.set(MutexGuard(handle));
            IS_LEADER.store(true, Ordering::Relaxed);
            info!("[watchdog] sibling {} became LEADER", sibling_id());
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        IS_LEADER.store(true, Ordering::Relaxed);
    }
}

/// Spawn missing siblings once, then start the background watchdog loop.
pub fn start(my_id: u8) {
    ensure_siblings_alive(my_id);
    try_acquire_leadership();

    std::thread::spawn(move || {
        loop {
            if uninstall_in_progress() {
                info!("[watchdog] uninstall lock detected — exiting sibling {}", my_id);
                // Best-effort quit so NSIS can remove files.
                std::process::exit(0);
            }

            ensure_siblings_alive(my_id);
            try_acquire_leadership();

            std::thread::sleep(Duration::from_millis(800));
        }
    });
}
