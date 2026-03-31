import { useState, useEffect } from "react";
import Login from "./components/Login";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

interface User {
  id?: string;
  _id?: string;
  username: string;
  role: string;
}

function App() {
  const [user, setUser] = useState<User | null>(null);
  const [isLoggedIn, setIsLoggedIn] = useState<boolean>(false);
  const [elapsedTime, setElapsedTime] = useState<number>(0);
  const [sessionId, setSessionId] = useState<string>("");
  const [machineId, setMachineId] = useState<string>("");

  // Initialize session ID once per app run or retrieve from storage
  useEffect(() => {
    let sid = localStorage.getItem('sessionId');
    if (!sid) {
      sid = crypto.randomUUID();
      localStorage.setItem('sessionId', sid);
    }
    setSessionId(sid);

    let mid = localStorage.getItem('machineId');
    if (!mid) {
      mid = crypto.randomUUID();
      localStorage.setItem('machineId', mid);
    }
    setMachineId(mid);
  }, []);

  // Initialize tracking and user state
  useEffect(() => {
    const savedUser = localStorage.getItem('user');
    const loggedIn = localStorage.getItem('isLoggedIn') === 'true';

    if (savedUser && loggedIn) {
      const parsedUser = JSON.parse(savedUser);
      setUser(parsedUser);
      setIsLoggedIn(true);

      // Sync with Rust backend
      syncUserWithRust(parsedUser);

      // Notify sidecar to start tracking on launch if already logged in
      startTracking(parsedUser._id || parsedUser.id || parsedUser.username);
    }
  }, []);

  // 1. Listen for the Rust heartbeat and update the timer/trigger logic
  useEffect(() => {
    if (!isLoggedIn) return;

    // 1. Listen for the Rust send-heartbeat event (UI timer only)
    const unlistenHeartbeat = listen('send-heartbeat', async (event: any) => {
      console.log("Rust heartbeat received:", event.payload);
      setElapsedTime(prev => prev + 60);
      takeScreenshotLogic();
    });

    const unlistenActivity = listen('new-activity-log', (event: any) => {
      console.log("New activity log received (Reported by Rust):", event.payload);
    });

    // 2. Disable F5 and Ctrl+R manually in JS for extra safety
    const handleKeydown = (e: KeyboardEvent) => {
      if (e.key === 'F5' || (e.ctrlKey && e.key === 'r')) {
        e.preventDefault();
      }
    };
    window.addEventListener('keydown', handleKeydown);

    return () => {
      unlistenHeartbeat.then(f => f());
      unlistenActivity.then(f => f());
      window.removeEventListener('keydown', handleKeydown);
    };
  }, [isLoggedIn, machineId, user]);

  const SIDECAR_URL = "https://tauri.localhost";

  const takeScreenshotLogic = async () => {
    if (!user || !sessionId) return;
    const uid = user._id || user.id || user.username;
    try {
      // 1. Get image from local sidecar
      const captureResponse = await fetch(`${SIDECAR_URL}/api/track/capture-image`);
      const captureData = await captureResponse.json();

      if (!captureData.success || !captureData.image) {
        throw new Error("Failed to capture image from sidecar");
      }

      // 2. Upload to Vercel dashboard
      await fetch('https://cus-spy-admin-dashboard.vercel.app/api/track/upload-screenshot', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          userId: uid,
          sessionId,
          image: captureData.image, // The base64 data
          timestamp: new Date().toISOString()
        })
      });
      console.log("Screenshot captured and uploaded successfully");
    } catch (err) {
      console.error("Manual screenshot failed:", err);
    }
  };

  const syncUserWithRust = async (userData: User) => {
    const uid = userData._id || userData.id;
    if (uid) {
      try {
        // This sends the MongoDB _id string directly to your Rust TrackingState
        await invoke("set_active_user", { userId: uid });
        console.log("User synced with Rust backend");
      } catch (err) {
        console.error("Failed to sync user with Rust:", err);
      }
    }
  };

  const startTracking = async (userId: string) => {
    try {
      // Initialize sidecar with session info
      await fetch(`https://cus-spy-admin-dashboard.vercel.app/api/track/start`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ userId, sessionId })
      });
      console.log("Tracking initialized in sidecar");
    } catch (err) {
      console.error("Failed to initialize sidecar tracking:", err);
    }
  };

  const handleLogin = (userData: User) => {
    setUser(userData);
    setIsLoggedIn(true);
    localStorage.setItem('user', JSON.stringify(userData));
    localStorage.setItem('isLoggedIn', 'true');

    // Sync with Rust backend
    syncUserWithRust(userData);

    // Trigger tracking in sidecar immediately after login
    startTracking(userData._id || userData.id || userData.username);
  };

  const formatTime = (totalSeconds: number) => {
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = totalSeconds % 60;
    return `${hours.toString().padStart(2, '0')}:${minutes.toString().padStart(2, '0')}:${seconds.toString().padStart(2, '0')}`;
  };

  if (!isLoggedIn) {
    return <Login onLogin={handleLogin} />;
  }

  return (
    <div className="tracker-overlay">
      <div className="tracker-card">
        <div className="tracker-header">
          <div className="brand">
            <img src="/app-logo.png" alt="Logo" className="mini-logo" />
            <span>CUS</span>
          </div>
          <div className="status-indicator">
            <span className="pulse"></span>
            ACTIVE
          </div>
        </div>

        <div className="tracker-body">
          <p className="user-welcome">Monitoring: <strong>{user?.username}</strong></p>
          <div className="time-display">
            <span className="label">TIME ELAPSED TODAY</span>
            <span className="timer">{formatTime(elapsedTime)}</span>
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
