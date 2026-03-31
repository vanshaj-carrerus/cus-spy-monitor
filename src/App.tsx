import { useState, useEffect } from "react";
import Login from "./components/Login";
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

  // Timer for display (local only, real tracking is in sidecar)
  useEffect(() => {
    let interval: number;
    if (isLoggedIn) {
      interval = window.setInterval(() => {
        setElapsedTime(prev => prev + 1);
      }, 1000);
    }
    return () => clearInterval(interval);
  }, [isLoggedIn]);

  const startTracking = async (userId: string) => {
    try {
      await fetch('http://localhost:3001/api/track/start', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ userId })
      });
      console.log("Tracking started in sidecar");
    } catch (err) {
      console.error("Failed to start tracking in sidecar:", err);
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
            <span>CUS Spy</span>
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

        <div className="tracker-footer">
          <p>System is running in background. Closing this window will not stop tracking.</p>
        </div>
      </div>
    </div>
  );
}

export default App;
