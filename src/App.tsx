import { useState, useEffect } from "react";
import Login from "./components/Login";
import UninstallAuth from "./components/UninstallAuth";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
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
  const [isUninstallAuthMode, setIsUninstallAuthMode] = useState<boolean>(false);
  const [isInitializing, setIsInitializing] = useState<boolean>(true);
  const [elapsedTime, setElapsedTime] = useState<number>(0);

  // Check for uninstall auth mode and initialize app on startup
  useEffect(() => {
    const initializeApp = async () => {
      try {
        console.log('[App] Starting initialization...');
        
        // Check if running in uninstall auth mode
        try {
          const isUninstallMode = await invoke<boolean>('is_uninstall_auth_mode');
          if (isUninstallMode) {
            console.log('[App] Uninstall auth mode detected');
            setIsUninstallAuthMode(true);
            setIsInitializing(false);
            return;
          }
        } catch (err) {
          console.warn('[App] Could not check uninstall mode (normal in dev):', err);
        }

        // Check for saved user session
        const savedUser = localStorage.getItem('user');
        const loggedIn = localStorage.getItem('isLoggedIn') === 'true';

        if (savedUser && loggedIn) {
          try {
            const parsedUser = JSON.parse(savedUser);
            setUser(parsedUser);
            setIsLoggedIn(true);
            console.log('[App] User session restored:', parsedUser.username);

            // Sync with Rust backend in background
            try {
              await invoke("set_active_user", {
                userId: parsedUser._id || parsedUser.id,
                username: parsedUser.username || null,
              });
            } catch (err) {
              console.error('[App] Failed to sync user with Rust:', err);
            }

            // Start tracking in background
            try {
              await fetch(`https://app.custech.co/api/track/start`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ 
                  userId: parsedUser._id || parsedUser.id,
                  sessionId: localStorage.getItem('sessionId') 
                })
              });
            } catch (err) {
              console.error('[App] Failed to start tracking:', err);
            }
          } catch (parseErr) {
            console.error('[App] Failed to parse saved user:', parseErr);
            localStorage.removeItem('user');
            localStorage.removeItem('isLoggedIn');
          }
        }

        setIsInitializing(false);
      } catch (err) {
        console.error('[App] Initialization error:', err);
        setIsInitializing(false);
      }
    };

    initializeApp();
  }, []);

  // Initialize session IDs in localStorage
  useEffect(() => {
    let sid = localStorage.getItem('sessionId');
    if (!sid) {
      sid = crypto.randomUUID();
      localStorage.setItem('sessionId', sid);
    }

    let mid = localStorage.getItem('machineId');
    if (!mid) {
      mid = crypto.randomUUID();
      localStorage.setItem('machineId', mid);
    }
  }, []);

  // Setup window close event handler
  useEffect(() => {
    let unlistenCloseRequested: (() => void) | undefined;

    const setupCloseHandler = async () => {
      try {
        // Listen for window close request from the close button
        unlistenCloseRequested = await listen<string>('tauri://close-requested', async () => {
          console.log('[App] Window close requested - gracefully closing...');
          try {
            // Close the webview window (Rust backend continues running)
            await getCurrentWindow().close();
          } catch (err) {
            console.error('[App] Error closing window:', err);
            // If close fails, let Rust handle the exit
          }
        });
        
        console.log('[App] Window close handler registered');
      } catch (err) {
        console.warn('[App] Could not setup close handler (normal in dev):', err);
      }
    };

    setupCloseHandler();

    return () => {
      if (unlistenCloseRequested) {
        unlistenCloseRequested();
      }
    };
  }, []);

  // Listen for heartbeat events and activity logs
  useEffect(() => {
    if (!isLoggedIn || isInitializing) return;

    let unlistenHeartbeat: (() => void) | undefined;
    let unlistenActivity: (() => void) | undefined;

    const setupListeners = async () => {
      try {
        unlistenHeartbeat = await listen('send-heartbeat', (event: any) => {
          console.log('[App] Heartbeat received:', event.payload);
          setElapsedTime(prev => prev + 60);
        });

        unlistenActivity = await listen('new-activity-log', (event: any) => {
          console.log('[App] Activity log received:', event.payload);
        });
      } catch (err) {
        console.error('[App] Failed to setup listeners:', err);
      }
    };

    setupListeners();

    // Disable F5 and Ctrl+R
    const handleKeydown = (e: KeyboardEvent) => {
      if (e.key === 'F5' || (e.ctrlKey && e.key === 'r')) {
        e.preventDefault();
      }
    };
    window.addEventListener('keydown', handleKeydown);

    return () => {
      if (unlistenHeartbeat) unlistenHeartbeat();
      if (unlistenActivity) unlistenActivity();
      window.removeEventListener('keydown', handleKeydown);
    };
  }, [isLoggedIn, isInitializing]);

  const handleLogin = (userData: User) => {
    console.log('[App] User logged in:', userData.username);
    setUser(userData);
    setIsLoggedIn(true);
    localStorage.setItem('user', JSON.stringify(userData));
    localStorage.setItem('isLoggedIn', 'true');

    // Sync with Rust backend
    invoke("set_active_user", {
      userId: userData._id || userData.id,
      username: userData.username || null,
    }).catch(err => console.error('[App] Failed to sync with Rust:', err));

    // Start tracking
    const sid = localStorage.getItem('sessionId');
    fetch(`https://app.custech.co/api/track/start`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ 
        userId: userData._id || userData.id,
        sessionId: sid 
      })
    }).catch(err => console.error('[App] Failed to start tracking:', err));
  };

  const formatTime = (totalSeconds: number) => {
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = totalSeconds % 60;
    return `${hours.toString().padStart(2, '0')}:${minutes.toString().padStart(2, '0')}:${seconds.toString().padStart(2, '0')}`;
  };

  // Loading screen during initialization
  if (isInitializing) {
    return (
      <div className="loading-container">
        <div className="loading-content">
          <div className="spinner"></div>
          <p>Initializing CUS Monitor...</p>
        </div>
      </div>
    );
  }

  // Uninstall authentication screen
  if (isUninstallAuthMode) {
    return (
      <UninstallAuth
        onAuthSuccess={async () => {
          console.log('[App] Uninstall authorized - signaling and exiting');
          // Signal Rust backend to allow uninstall (exits with code 0)
          await invoke('signal_uninstall_allowed').catch(err => {
            console.error('[App] Failed to signal uninstall allowed:', err);
          });
        }}
        onAuthCancel={async () => {
          console.log('[App] Uninstall cancelled - signaling and exiting');
          // Signal Rust backend to deny uninstall (exits with code 1)
          await invoke('signal_uninstall_denied').catch(err => {
            console.error('[App] Failed to signal uninstall denied:', err);
          });
        }}
      />
    );
  }

  // Login screen
  if (!isLoggedIn) {
    return <Login onLogin={handleLogin} />;
  }

  // Main dashboard with timing
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
