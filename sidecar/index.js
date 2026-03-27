import express from "express";
import cors from "cors";
import dotenv from "dotenv";
import DBConnect from "./lib/mongoose.js";
import { createFullUser, loginUser } from "./lib/userService.js";
import { Invite } from "./models/index.js";
import { mouse, Button, Point } from "@nut-tree-fork/nut-js";
import screenshot from "screenshot-desktop";

dotenv.config();

const app = express();
const PORT = process.env.PORT || 3001;

// Middlewares
app.use(cors());
app.use(express.json());

// Init DB
DBConnect().catch(err => console.error("MongoDB connection failed:", err));

// --- API Endpoints ---

// Registration
app.post('/api/auth/register', async (req, res) => {
    try {
        const { userData, roleData } = req.body;
        const result = await createFullUser(userData, roleData);
        res.status(201).json({ success: true, user: result.user });
    } catch (err) {
        res.status(400).json({ success: false, error: err.message });
    }
});

// Login
app.post('/api/auth/login', async (req, res) => {
    try {
        const { username, password } = req.body;
        const user = await loginUser(username, password);
        res.status(200).json({ success: true, user });
    } catch (err) {
        res.status(401).json({ success: false, error: err.message });
    }
});

// Invite System
app.post('/api/auth/invite', async (req, res) => {
    try {
        const { email, role, invitedBy } = req.body;

        // Generate a random token
        const token = Math.random().toString(36).substring(2, 15) + Math.random().toString(36).substring(2, 15);

        const invite = new Invite({
            email,
            role,
            invitedBy,
            token
        });

        await invite.save();
        res.status(201).json({ success: true, invite });
    } catch (err) {
        res.status(400).json({ success: false, error: err.message });
    }
});

// --- Tracking & Heartbeat Logic ---
let trackerInterval = null;
let activeUserId = null;

const DASHBOARD_API_URL = process.env.DASHBOARD_API_URL || `http://localhost:${PORT}`;

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
        const data = await response.json();
        console.log("Heartbeat response:", data);
    } catch (err) {
        console.error("Heartbeat failed:", err.message);
    }
};

app.post('/api/track/start', (req, res) => {
    const { userId } = req.body;
    if (!userId) return res.status(400).json({ error: "userId required" });

    activeUserId = userId;
    
    if (!trackerInterval) {
        trackerInterval = setInterval(sendHeartbeat, 30000); // 30 seconds
        console.log(`Tracker started for user: ${userId}`);
        // Send initial heartbeat immediately
        sendHeartbeat();
    }

    res.json({ success: true, message: "Tracking initialized" });
});

// Dashboard API Heartbeat Endpoint (Internal/Centralized)
// If the sidecar is NOT the dashboard, this might be handled elsewhere.
// But as per requirements, we implement the communication.
app.post('/api/track/heartbeat', async (req, res) => {
    try {
        const { userId, timestamp } = req.body;
        // Here you would update MongoDB
        // Example: await User.findByIdAndUpdate(userId, { $inc: { activeSeconds: 30 }, lastActive: timestamp });
        
        console.log(`[DB UPDATE] Heartbeat received for ${userId} at ${timestamp}`);
        res.json({ success: true, message: "Heartbeat recorded" });
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