import { useState, useEffect } from "react";
import Login from "./components/Login";
import { listen } from "@tauri-apps/api/event";
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

      // Notify sidecar to start tracking on launch if already logged in
      startTracking(parsedUser._id || parsedUser.id || parsedUser.username);
    }
  }, []);

  // 1. Listen for the Rust heartbeat and update the timer/trigger logic
  useEffect(() => {
    if (!isLoggedIn) return;

    const unlistenHeartbeat = listen('heartbeat-tick', (event) => {
      console.log("Rust triggered heartbeat");
      setElapsedTime(prev => prev + 60);
      sendHeartbeatAPI();
      takeScreenshotLogic();
    });

    const unlistenActivity = listen('new-activity-log', async (event: any) => {
      const payload = event.payload;
      console.log("New activity log received:", payload);

      if (!user) return;
      const uid = user._id || user.id || user.username;

      try {
        await fetch('https://cus-spy-admin-dashboard.vercel.app/api/activity-log', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            userId: uid,
            machine_id: machineId,
            title: payload.title,
            app_name: payload.app_name,
            start_time: payload.start_time,
            duration_seconds: payload.duration_seconds
          })
        });
        console.log("Activity log sent successfully");
      } catch (err) {
        console.error("Failed to send activity log:", err);
      }
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

  const sendHeartbeatAPI = async () => {
    if (!user || !sessionId) return;
    const uid = user._id || user.id || user.username;
    try {
      await fetch('https://cus-spy-admin-dashboard.vercel.app/api/track/heartbeat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          userId: uid,
          sessionId,
          timestamp: new Date().toISOString()
        })
      });
    } catch (err) {
      console.error("Manual heartbeat failed:", err);
    }
  };

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
