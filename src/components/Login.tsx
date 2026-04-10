// src/components/Login.tsx

import React, { useState } from 'react';
import './Login.css';
import { fetch } from '@tauri-apps/plugin-http';

interface LoginProps {
    onLogin: (user: { username: string; role: string }) => void;
}

const Login: React.FC<LoginProps> = ({ onLogin }) => {
    const [email, setEmail] = useState('');
    const [password, setPassword] = useState('');
    const [loading, setLoading] = useState(false);

    const handleLogin = async (e: React.FormEvent) => {
        e.preventDefault();
        setLoading(true);

        try {
            const response = await fetch('https://app.custech.co/api/auth/login', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ email, password })
            });

            const data = await response.json();

            if (data.success) {
                const userData = data.user;
                // Store in local storage to keep state across reloads
                localStorage.setItem('user', JSON.stringify(userData));
                localStorage.setItem('isLoggedIn', 'true');
                onLogin(userData);
            } else {
                alert(data.error || "Login failed");
            }
        } catch (err) {
            console.error("Auth error:", err);
            alert("Could not connect to the security server.");
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="login-container">
            <div className="login-card">
                <div className="login-header">
                    <img src="/app-logo.png" alt="App Logo" className="login-logo" />
                    <h1>CUS Monitor</h1>
                    <p>Secure login for agents and managers</p>
                </div>
                <form className="login-form" onSubmit={handleLogin}>
                    <div className="input-group">
                        <label htmlFor="email">Email Address</label>
                        <input
                            type="email"
                            id="email"
                            value={email}
                            onChange={(e) => setEmail(e.target.value)}
                            placeholder="Enter your email"
                            required
                        />
                    </div>
                    <div className="input-group">
                        <label htmlFor="password">Password</label>
                        <input
                            type="password"
                            id="password"
                            value={password}
                            onChange={(e) => setPassword(e.target.value)}
                            placeholder="Enter your password"
                            required
                        />
                    </div>
                    <button type="submit" className="login-button" disabled={loading}>
                        {loading ? 'Authenticating...' : 'Sign In'}
                    </button>
                </form>
                <div className="login-footer">
                    <p>Protected by CUS Security Protocol</p>
                </div>
            </div>
        </div>
    );
};

export default Login;
