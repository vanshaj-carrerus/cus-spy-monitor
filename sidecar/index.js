// sidecar/index.js

import express from "express";
import cors from "cors";
import dotenv from "dotenv";
import { mouse, Button, Point } from "@nut-tree-fork/nut-js";
import screenshot from "screenshot-desktop";

dotenv.config();

export const DASHBOARD_API_URL = "https://cus-spy-admin-dashboard.vercel.app";

const app = express();
const PORT = process.env.PORT || 3001;

// Middlewares
app.use(cors({
    origin: [
        "http://localhost:1420",   // Default Tauri dev port
        "tauri://localhost",       // Standard Tauri production protocol
        "http://tauri.localhost"    // Windows/Linux production protocol
    ],
    methods: ["GET", "POST", "PUT", "DELETE"],
    credentials: true
}));
app.use(express.json());

// --- API Endpoints Proxying ---

// Registration
app.post('/api/auth/register', async (req, res) => {
    try {
        const response = await fetch(`${DASHBOARD_API_URL}/api/auth/register`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(req.body)
        });
        const data = await response.json();
        res.status(response.status).json(data);
    } catch (err) {
        res.status(500).json({ success: false, error: err.message });
    }
});

// Login
app.post('/api/auth/login', async (req, res) => {
    try {
        const response = await fetch(`${DASHBOARD_API_URL}/api/auth/login`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(req.body)
        });
        const data = await response.json();
        res.status(response.status).json(data);
    } catch (err) {
        res.status(500).json({ success: false, error: err.message });
    }
});

// Invite System
app.post('/api/auth/invite', async (req, res) => {
    try {
        const response = await fetch(`${DASHBOARD_API_URL}/api/auth/invite`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(req.body)
        });
        const data = await response.json();
        res.status(response.status).json(data);
    } catch (err) {
        res.status(500).json({ success: false, error: err.message });
    }
});

// --- Tracking & Heartbeat Logic ---
// --- Tracking & Heartbeat Logic ---
let trackerInterval = null;
let activeUserId = null;

const HEARTBEAT_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes

const sendHeartbeat = async () => {
    if (!activeUserId) return;

    try {
        console.log(`Sending heartbeat for user: ${activeUserId}`);
        const response = await fetch(`${DASHBOARD_API_URL}/api/track/heartbeat`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                userId: activeUserId,
                timestamp: new Date().toISOString()
            })
        });
        const data = response.ok ? await response.json() : {};
        console.log("Heartbeat response:", data);
    } catch (err) {
        console.error("Heartbeat failed:", err.message);
    }
};

app.post('/api/track/start', async (req, res) => {
    const { userId } = req.body;
    if (!userId) return res.status(400).json({ error: "userId required" });

    try {
        // Fetch remote dashboard to check role and enable tracking
        const response = await fetch(`${DASHBOARD_API_URL}/api/track/start`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(req.body)
        });
        const data = await response.json();

        // If dashboard specifies tracking should be ignored (e.g. admin/manager)
        if (data.ignored) {
            return res.json({ success: true, message: data.message || "Tracking ignored for this role" });
        }

        if (!data.success) {
            return res.status(response.status || 400).json(data);
        }

        activeUserId = data.userId || userId;

        if (!trackerInterval) {
            trackerInterval = setInterval(sendHeartbeat, HEARTBEAT_INTERVAL_MS);
            console.log(`Tracker started for user: ${activeUserId}`);
            // Send initial heartbeat immediately
            sendHeartbeat();
        }

        res.json({ success: true, message: "Tracking initialized" });
    } catch (err) {
        res.status(500).json({ error: err.message });
    }
});

// --- Sidecar Internal Commands (Keep for compat) ---
process.stdin.on('data', async (data) => {
    try {
        const msg = JSON.parse(data.toString());
        if (msg.type === 'MOVE_MOUSE') {
            await mouse.setPosition(new Point(msg.x, msg.y));
        }
        if (msg.type === 'CLICK') {
            await mouse.click(Button.LEFT);
        }
        if (msg.type === 'CAPTURE') {
            const img = await screenshot();
            process.stdout.write("SCREENSHOT_DONE");
        }
    } catch (err) { }
});

app.listen(PORT, () => {
    console.log(`Sidecar API running on http://localhost:${PORT}`);
});

console.log("Sidecar Active");