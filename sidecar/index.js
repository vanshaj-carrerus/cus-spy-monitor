// sidecar/index.js

import express from "express";
import cors from "cors";
import dotenv from "dotenv";
import { mouse, Button, Point } from "@nut-tree-fork/nut-js";
import screenshot from "screenshot-desktop";
import crypto from "crypto";

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
let trackerInterval = null;
let screenshotInterval = null;
let activeUserId = null;
const sessionId = crypto.randomUUID();
let isFetchingHeartbeat = false;

const HEARTBEAT_INTERVAL_MS = 60 * 1000; // 60 seconds
const SCREENSHOT_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes

const captureAndUploadScreenshot = async () => {
    if (!activeUserId) return;
    try {
        console.log(`Capturing screenshot for user: ${activeUserId}`);
        const imgBuffer = await screenshot({ format: 'jpg' });
        const base64Image = imgBuffer.toString('base64');

        const response = await fetch(`${DASHBOARD_API_URL}/api/track/upload-screenshot`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                userId: activeUserId,
                sessionId,
                image: `data:image/jpeg;base64,${base64Image}`,
                timestamp: new Date().toISOString()
            })
        });

        if (response.ok) {
            console.log("Screenshot uploaded successfully");
        } else {
            console.error("Screenshot upload failed:", response.status);
        }
    } catch (err) {
        console.error("Screenshot capture/upload failed:", err.message);
    }
};

const sendHeartbeat = async () => {
    if (!activeUserId || isFetchingHeartbeat) return;

    isFetchingHeartbeat = true;
    try {
        console.log(`Sending heartbeat for user: ${activeUserId} in session: ${sessionId}`);
        const response = await fetch(`${DASHBOARD_API_URL}/api/track/heartbeat`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                userId: activeUserId,
                sessionId,
                timestamp: new Date().toISOString()
            })
        });
        const data = response.ok ? await response.json() : {};
        console.log("Heartbeat response:", data);
    } catch (err) {
        console.error("Heartbeat failed:", err.message);
    } finally {
        isFetchingHeartbeat = false;
    }
};

// --- Manual Triggers for Rust/Frontend ---
app.post('/api/track/heartbeat', async (req, res) => {
    const { userId, sessionId: reqSessionId, timestamp } = req.body;
    if (userId) {
        activeUserId = userId;
        // Optionally update the local sessionId if provided
        // or just use the one from the request for this specific call
        const sid = reqSessionId || sessionId;

        try {
            console.log(`Manual heartbeat for user: ${userId}`);
            const response = await fetch(`${DASHBOARD_API_URL}/api/track/heartbeat`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    userId,
                    sessionId: sid,
                    timestamp: timestamp || new Date().toISOString()
                })
            });
            const data = await response.json();
            res.json({ success: true, data });
        } catch (err) {
            res.status(500).json({ success: false, error: err.message });
        }
    } else {
        res.status(400).json({ success: false, message: "userId required" });
    }
});

app.post('/api/track/screenshot', async (req, res) => {
    const { userId, sessionId: reqSessionId, timestamp } = req.body;
    if (userId) {
        activeUserId = userId;
        const sid = reqSessionId || sessionId;

        try {
            console.log(`Manual screenshot for user: ${userId}`);
            const imgBuffer = await screenshot({ format: 'jpg' });
            const base64Image = imgBuffer.toString('base64');

            const response = await fetch(`${DASHBOARD_API_URL}/api/track/upload-screenshot`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    userId,
                    sessionId: sid,
                    image: `data:image/jpeg;base64,${base64Image}`,
                    timestamp: timestamp || new Date().toISOString()
                })
            });
            res.json({ success: response.ok });
        } catch (err) {
            res.status(500).json({ success: false, error: err.message });
        }
    } else {
        res.status(400).json({ success: false, message: "userId required" });
    }
});

app.get('/api/track/capture-image', async (req, res) => {
    try {
        console.log("Capturing image for frontend...");
        const imgBuffer = await screenshot({ format: 'jpg' });
        const base64Image = imgBuffer.toString('base64');
        res.json({ success: true, image: `data:image/jpeg;base64,${base64Image}` });
    } catch (err) {
        res.status(500).json({ success: false, error: err.message });
    }
});

app.post('/api/track/start', async (req, res) => {
    const { userId, sessionId: incomingSessionId } = req.body;
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

        if (!activeUserId) {
            activeUserId = data.userId || userId;
            console.log(`Sidecar tracking initialized for user: ${activeUserId}`);
            // Send initial heartbeat and screenshot immediately
            sendHeartbeat();
            setTimeout(captureAndUploadScreenshot, 3000);
        }

        // Intervals are now driven by Rust/Frontend events to avoid throttling
        // if (!trackerInterval) {
        //     trackerInterval = setInterval(sendHeartbeat, HEARTBEAT_INTERVAL_MS);
        // }
        // if (!screenshotInterval) {
        //     screenshotInterval = setInterval(captureAndUploadScreenshot, SCREENSHOT_INTERVAL_MS);
        // }

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

// graceful task termination handle
const handleExit = async () => {
    if (activeUserId) {
        console.log("Process terminating, sending stop-session ping...");
        try {
            await fetch(`${DASHBOARD_API_URL}/api/track/stop`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    userId: activeUserId,
                    sessionId,
                    timestamp: new Date().toISOString()
                })
            });
        } catch (err) {
            console.error("Failed to send stop-session ping:", err.message);
        }
    }
    process.exit(0);
};

process.on('SIGTERM', handleExit);
process.on('SIGINT', handleExit);
process.on('exit', handleExit);