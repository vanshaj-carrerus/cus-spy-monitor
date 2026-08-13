# Password and OTP Protected Uninstallation Flow - Implementation Guide

This document provides a complete overview of the password and OTP protected uninstallation flow implementation for CUS Monitor.

---

## Overview

The uninstallation process is now protected by a two-factor authentication mechanism:
1. **Password Verification** - Company-level password protection
2. **OTP Verification** - Admin One-Time Password sent to registered email

If either step fails or is cancelled, the uninstallation process is immediately aborted with all application files intact.

---

## Architecture

### Component Flow

```
User initiates uninstall via Windows Control Panel
              ↓
      NSIS Uninstaller
              ↓
    Launches app with --request-uninstall-auth flag
              ↓
      React Frontend (UninstallAuth.tsx)
              ↓
    User enters Password → Verification API
              ↓
    User enters OTP → Verification API
              ↓
         Success? → Exit Code 0 (allow uninstall)
         Failure? → Exit Code 1 (abort uninstall)
              ↓
      NSIS checks exit code and proceeds/aborts
```

---

## Implementation Details

### 1. Frontend Components

#### File: `src/components/UninstallAuth.tsx`
A dedicated React component for uninstall authentication with:
- **Two-step form UI**: Password input, then OTP input
- **Error handling**: Inline error messages for failed attempts
- **OTP delivery**: "Send OTP" button to trigger OTP delivery
- **Exit mechanisms**: Cancel button and window close handler
- **API integration**: Communicates with backend verification endpoints
- **Tauri IPC**: Signals Rust backend when auth succeeds/fails

**Key features:**
- Responsive design with gradient purple theme
- Dark mode support
- Disabled inputs during loading
- Field hints for user guidance
- Success and error messages

#### File: `src/components/UninstallAuth.css`
Professional styling with:
- Gradient backgrounds
- Smooth transitions and animations
- Mobile responsive design
- Dark mode support
- Accessibility-friendly colors and spacing

#### File: `src/App.tsx` (Updated)
- Detects `--request-uninstall-auth` flag via `is_uninstall_auth_mode()` Tauri command
- Renders `UninstallAuth` component when in uninstall mode
- Handles exit codes via Tauri's `appWindow.exit()`

### 2. Rust Backend Changes

#### File: `src-tauri/src/lib.rs` (Updated)

**New static variable:**
```rust
static UNINSTALL_AUTH_MODE: OnceLock<bool> = OnceLock::new();
```

**New Tauri commands:**
```rust
#[tauri::command]
fn is_uninstall_auth_mode() -> bool
// Returns whether app is running in uninstall auth mode

#[tauri::command]
fn signal_uninstall_allowed()
// Called when credentials are verified - exits with code 0

#[tauri::command]
fn signal_uninstall_denied()
// Called when user cancels or auth fails - exits with code 1
```

**New function:**
```rust
fn run_uninstall_auth_mode()
// Lightweight Tauri app with only uninstall auth UI
// Skips normal initialization (sibling claiming, watchdog, etc.)
```

**Modifications to `pub fn run()`:**
- Checks for `--request-uninstall-auth` CLI argument
- Sets `UNINSTALL_AUTH_MODE` flag
- Skips sibling claiming and normal initialization when in uninstall mode
- Calls `run_uninstall_auth_mode()` instead of full app initialization

### 3. NSIS Installer Integration

#### File: `src-tauri/windows/hooks.nsh` (Updated)

**Pre-uninstall hook:**
- Locates CUS Monitor executable in installation directory
- Launches app with `--request-uninstall-auth` CLI flag
- Waits for app to complete (blocking call)
- Checks exit code:
  - **Exit code 0**: Uninstallation allowed - continues with removal
  - **Exit code ≠ 0**: Uninstallation denied - shows message and aborts
- Kills remaining app processes before uninstall
- Writes uninstall lock file to prevent race conditions

**Post-uninstall hook:**
- Cleans up CUS Monitor cache directory (`%LOCALAPPDATA%\cus-monitor`)

---

## Backend API Endpoints Required

The frontend expects three backend API endpoints. Implement these in your backend:

### 1. Verify Password
**Endpoint:** `POST /api/uninstall/verify-password`

**Request:**
```json
{
  "password": "company_password"
}
```

**Response (Success):**
```json
{
  "success": true,
  "otpSent": true,
  "message": "Password verified. OTP has been sent."
}
```

**Response (Failure):**
```json
{
  "success": false,
  "error": "Invalid password. Please try again."
}
```

**Security considerations:**
- Implement rate limiting (e.g., 5 attempts per 15 minutes)
- Hash passwords server-side
- Log all uninstall attempts for audit trail
- Validate against your company password policy

---

### 2. Send OTP
**Endpoint:** `POST /api/uninstall/send-otp`

**Request:**
```json
{}
```

**Response (Success):**
```json
{
  "success": true,
  "message": "OTP sent to registered admin email"
}
```

**Response (Failure):**
```json
{
  "success": false,
  "error": "No admin email configured. Contact administrator."
}
```

**Security considerations:**
- Verify password was already validated before sending OTP
- Send OTP only to registered admin/authorized emails
- Use time-limited codes (e.g., 5 minutes)
- Implement rate limiting on OTP generation
- Log all OTP requests

---

### 3. Verify OTP
**Endpoint:** `POST /api/uninstall/otp/verify`

**Request:**
```json
{
  "otp": "123456"
}
```

**Response (Success):**
```json
{
  "success": true,
  "message": "OTP verified. Uninstallation will proceed."
}
```

**Response (Failure):**
```json
{
  "success": false,
  "error": "Invalid or expired OTP. Please try again."
}
```

**Security considerations:**
- Validate OTP format (6 digits or alphanumeric)
- Check expiration timestamp
- Implement lockout after 3 failed attempts
- Remove OTP after successful verification
- Log verification attempts for security audit
- Ensure one-time use only

---

## Flow Diagrams

### User Cancels Uninstallation
```
User clicks "Cancel Uninstallation"
              ↓
Frontend calls signal_uninstall_denied()
              ↓
Rust exits with code 1
              ↓
NSIS receives exit code 1
              ↓
NSIS shows cancellation message
              ↓
NSIS calls Abort
              ↓
Application remains installed
```

### Wrong Password Entered
```
User enters wrong password
              ↓
Frontend sends to /api/uninstall/verify-password
              ↓
Backend returns success=false
              ↓
Frontend displays error message
              ↓
User stays on password screen
              ↓
Can retry or cancel
```

### Expired OTP
```
User enters OTP (already expired or wrong)
              ↓
Frontend sends to /api/uninstall/otp/verify
              ↓
Backend returns success=false
              ↓
Frontend displays error: "Invalid or expired OTP"
              ↓
Can retry or cancel uninstall
```

### Successful Uninstallation
```
User enters valid password
              ↓
Frontend moves to OTP step
              ↓
User requests OTP (backend sends to email)
              ↓
User enters valid OTP
              ↓
Frontend calls signal_uninstall_allowed()
              ↓
Rust exits with code 0
              ↓
NSIS receives exit code 0
              ↓
NSIS proceeds with file removal
              ↓
Application is uninstalled
```

---

## Security Considerations

### 1. Network Security
- All API calls use HTTPS
- Validate SSL certificates
- Handle network timeouts gracefully (fail-safe: abort uninstall)

### 2. Authentication
- Password should be company-specific (configured in backend)
- OTP should be time-limited and one-time use only
- Implement account lockout after failed attempts
- Log all uninstall attempts for audit trail

### 3. Process Security
- App runs with minimal privileges when in uninstall mode
- Skips normal watchdog and sibling claiming
- Cleans up cache/logs after uninstallation
- Lock file prevents race conditions

### 4. User Experience
- Clear error messages without exposing sensitive info
- Ability to retry or cancel at any step
- Timeout handling for long network delays
- Graceful degradation if backend unavailable

---

## Testing Checklist

- [ ] User cancels at password step → uninstall aborts
- [ ] User enters wrong password → error shown, can retry
- [ ] User cancels at OTP step → uninstall aborts
- [ ] User clicks "Send OTP" → OTP sent to email
- [ ] User enters wrong OTP → error shown, can retry
- [ ] User enters valid credentials → uninstall proceeds
- [ ] Window close button triggers cancellation → uninstall aborts
- [ ] Network timeout is handled gracefully
- [ ] Exit codes are correct (0 for success, 1 for failure)
- [ ] NSIS receives and processes exit codes correctly
- [ ] App processes are killed before uninstall
- [ ] Cache directory is cleaned up after uninstall
- [ ] Uninstall lock file prevents race conditions
- [ ] UI is responsive and user-friendly
- [ ] Dark mode displays correctly

---

## API Example Implementation (Node.js/Express)

Here's a reference implementation for the backend endpoints:

```typescript
// routes/uninstall.ts
import express, { Router, Request, Response } from 'express';
import crypto from 'crypto';
import nodemailer from 'nodemailer';

const router = Router();

// In-memory OTP store (use Redis in production)
const otpStore: Map<string, { code: string; expiresAt: Date }> = new Map();

// Company password (should be from environment variable)
const COMPANY_PASSWORD = process.env.COMPANY_PASSWORD || 'CUS@MonitorSecure';

// Email configuration
const transporter = nodemailer.createTransport({
  host: process.env.EMAIL_HOST,
  port: parseInt(process.env.EMAIL_PORT || '587'),
  secure: process.env.EMAIL_SECURE === 'true',
  auth: {
    user: process.env.EMAIL_USER,
    pass: process.env.EMAIL_PASS,
  },
});

// Verify password endpoint
router.post('/verify-password', async (req: Request, res: Response) => {
  try {
    const { password } = req.body;

    if (!password || password !== COMPANY_PASSWORD) {
      return res.json({
        success: false,
        error: 'Invalid password. Please try again.',
      });
    }

    // Password is valid, send OTP
    const otp = crypto.randomBytes(3).toString('hex').toUpperCase().slice(0, 6);
    const expiresAt = new Date(Date.now() + 5 * 60 * 1000); // 5 minutes

    // Store OTP (in production, associate with user/machine ID)
    otpStore.set('current_uninstall', { code: otp, expiresAt });

    // Send OTP to admin email
    const adminEmail = process.env.ADMIN_EMAIL || 'admin@custech.co';
    await transporter.sendMail({
      from: process.env.EMAIL_FROM,
      to: adminEmail,
      subject: 'CUS Monitor Uninstall OTP',
      html: `
        <h2>Uninstall Verification Required</h2>
        <p>An uninstall request for CUS Monitor has been initiated.</p>
        <p><strong>OTP Code: ${otp}</strong></p>
        <p>This code will expire in 5 minutes.</p>
        <p>If you did not initiate this request, the uninstall will be aborted.</p>
      `,
    });

    res.json({
      success: true,
      otpSent: true,
      message: 'Password verified. OTP has been sent to the admin email.',
    });
  } catch (error) {
    console.error('Password verification error:', error);
    res.status(500).json({
      success: false,
      error: 'Server error. Please try again later.',
    });
  }
});

// Send OTP endpoint
router.post('/send-otp', async (req: Request, res: Response) => {
  try {
    const otp = crypto.randomBytes(3).toString('hex').toUpperCase().slice(0, 6);
    const expiresAt = new Date(Date.now() + 5 * 60 * 1000);

    otpStore.set('current_uninstall', { code: otp, expiresAt });

    const adminEmail = process.env.ADMIN_EMAIL || 'admin@custech.co';
    await transporter.sendMail({
      from: process.env.EMAIL_FROM,
      to: adminEmail,
      subject: 'CUS Monitor Uninstall OTP',
      html: `
        <h2>Uninstall Verification Required</h2>
        <p>OTP Code: <strong>${otp}</strong></p>
        <p>This code will expire in 5 minutes.</p>
      `,
    });

    res.json({
      success: true,
      message: 'OTP sent to registered admin email.',
    });
  } catch (error) {
    console.error('Send OTP error:', error);
    res.status(500).json({
      success: false,
      error: 'Failed to send OTP. Please try again.',
    });
  }
});

// Verify OTP endpoint
router.post('/otp/verify', (req: Request, res: Response) => {
  try {
    const { otp } = req.body;

    const stored = otpStore.get('current_uninstall');

    if (!stored) {
      return res.json({
        success: false,
        error: 'No OTP request found. Please restart uninstallation.',
      });
    }

    if (new Date() > stored.expiresAt) {
      otpStore.delete('current_uninstall');
      return res.json({
        success: false,
        error: 'OTP expired. Please request a new one.',
      });
    }

    if (otp !== stored.code) {
      return res.json({
        success: false,
        error: 'Invalid OTP. Please try again.',
      });
    }

    // OTP verified successfully - clean up
    otpStore.delete('current_uninstall');

    // Log uninstall approval
    console.log('[AUDIT] Uninstall approved via valid credentials');

    res.json({
      success: true,
      message: 'OTP verified. Uninstallation will proceed.',
    });
  } catch (error) {
    console.error('OTP verification error:', error);
    res.status(500).json({
      success: false,
      error: 'Server error. Please try again later.',
    });
  }
});

export default router;
```

---

## Deployment Notes

1. **Environment Variables Required:**
   - `COMPANY_PASSWORD`: The password required for uninstall
   - `ADMIN_EMAIL`: Email to send OTP codes
   - `EMAIL_HOST`, `EMAIL_PORT`, `EMAIL_USER`, `EMAIL_PASS`: Email configuration
   - `EMAIL_FROM`: Sender email address

2. **Database Considerations:**
   - Store OTP codes with timestamps for audit trail
   - Log all uninstall attempts for security review
   - Track which machines/users attempted uninstall

3. **Rate Limiting:**
   - Implement rate limiting on API endpoints
   - Lock accounts after 3-5 failed attempts
   - Temporary lockout (5-15 minutes) to prevent brute force

4. **Monitoring:**
   - Alert admins on uninstall attempts
   - Monitor for abuse patterns
   - Track successful uninstalls for licensing purposes

---

## Rollback Instructions

If you need to revert this feature:

1. **In NSIS hooks (`hooks.nsh`):**
   - Remove `--request-uninstall-auth` flag launch
   - Restore simple uninstall process or hardcoded password check

2. **In React App (`App.tsx`):**
   - Remove `UninstallAuth` import and condition
   - Remove `isUninstallAuthMode` state

3. **In Rust (`lib.rs`):**
   - Remove `UNINSTALL_AUTH_MODE` static
   - Remove uninstall auth commands
   - Remove `run_uninstall_auth_mode()` function
   - Revert `run()` function to skip uninstall mode check

---

## Future Enhancements

1. **Machine ID tracking:** Store which machines attempted uninstall
2. **Email notifications:** Alert on all uninstall attempts
3. **Biometric auth:** Add fingerprint/facial recognition for additional security
4. **Approval workflow:** Require manager approval before uninstall
5. **Uninstall recovery:** Keep backup of settings/data for 30 days
6. **Custom branding:** Allow company logo/colors in uninstall UI
7. **Multi-language support:** Translate UI messages to multiple languages

---

## Support & Troubleshooting

### Issue: App doesn't launch with --request-uninstall-auth
**Solution:** Verify the executable is built with the latest code. Check logs in `%APPDATA%\cus-monitor\logs`.

### Issue: OTP not being sent
**Solution:** Verify email configuration. Check backend logs for email send errors. Ensure admin email is configured.

### Issue: Uninstall still proceeds on failed auth
**Solution:** Verify NSIS is checking exit code correctly. Check that `signal_uninstall_denied()` is being called.

### Issue: App window is invisible
**Solution:** Check that the window configuration in `tauri.conf.json` is correct. Ensure frontend is loading properly.

---

## License & Security Policy

This implementation follows security best practices:
- Principle of least privilege
- Defense in depth (password + OTP)
- Fail-safe defaults (abort on auth failure)
- Audit logging for compliance
- Rate limiting to prevent brute force

---

**Document Version:** 1.0  
**Last Updated:** 2026-08-13  
**Maintainer:** CUS Monitor Security Team
