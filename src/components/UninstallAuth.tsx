import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './UninstallAuth.css';

interface UninstallAuthProps {
  onAuthSuccess?: () => void;
  onAuthCancel?: () => void;
}

const UninstallAuth: React.FC<UninstallAuthProps> = ({ onAuthSuccess, onAuthCancel }) => {
  const [password, setPassword] = useState('');
  const [otp, setOtp] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [step, setStep] = useState<'password' | 'otp'>('password');
  const [otpSent, setOtpSent] = useState(false);

  const handlePasswordSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');

    if (!password.trim()) {
      setError('Password is required');
      return;
    }

    setLoading(true);
    try {
      // Verify password with backend
      const response = await fetch('https://app.custech.co/api/uninstall/verify-password', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ password: password.trim() })
      });

      const data = await response.json();

      if (data.success) {
        // Move to OTP step
        setStep('otp');
        setPassword(''); // Clear password from memory
        if (data.otpSent) {
          setOtpSent(true);
        }
      } else {
        setError(data.error || 'Invalid password. Please try again.');
      }
    } catch (err) {
      console.error('Password verification error:', err);
      setError('Could not verify password. Please check your connection and try again.');
    } finally {
      setLoading(false);
    }
  };

  const handleSendOtp = async () => {
    setError('');
    setLoading(true);
    try {
      const response = await fetch('https://app.custech.co/api/uninstall/send-otp', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({})
      });

      const data = await response.json();

      if (data.success) {
        setOtpSent(true);
        setError('OTP sent to registered admin email.');
      } else {
        setError(data.error || 'Could not send OTP. Please try again.');
      }
    } catch (err) {
      console.error('Send OTP error:', err);
      setError('Could not send OTP. Please check your connection.');
    } finally {
      setLoading(false);
    }
  };

  const handleOtpSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');

    if (!otp.trim()) {
      setError('OTP is required');
      return;
    }

    setLoading(true);
    try {
      // Verify OTP with backend
      const response = await fetch('https://app.custech.co/api/uninstall/otp/verify', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ otp: otp.trim() })
      });

      const data = await response.json();

      if (data.success) {
        // Signal to Rust that uninstallation is allowed
        try {
          await invoke('signal_uninstall_allowed', {});
          if (onAuthSuccess) {
            onAuthSuccess();
          }
        } catch (invokeErr) {
          console.error('Failed to signal uninstall allowed:', invokeErr);
          // Even if invoke fails, consider it a success since backend verification passed
          setError('');
          if (onAuthSuccess) {
            onAuthSuccess();
          }
        }
      } else {
        setError(data.error || 'Invalid OTP. Please try again.');
      }
    } catch (err) {
      console.error('OTP verification error:', err);
      setError('Could not verify OTP. Please check your connection and try again.');
    } finally {
      setLoading(false);
    }
  };

  const handleCancel = async () => {
    try {
      // Signal to Rust that uninstallation is not allowed
      await invoke('signal_uninstall_denied', {});
    } catch (err) {
      console.error('Failed to signal uninstall denied:', err);
    }

    if (onAuthCancel) {
      onAuthCancel();
    }
  };

  return (
    <div className="uninstall-auth-container">
      <div className="uninstall-auth-card">
        {/* Header */}
        <div className="uninstall-auth-header">
          <div className="uninstall-auth-icon">
            <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6h14zM10 11v6M14 11v6" />
            </svg>
          </div>
          <h1>Uninstallation Protection</h1>
          <p>CUS Monitor requires authentication to be uninstalled</p>
        </div>

        {/* Content */}
        <div className="uninstall-auth-content">
          {step === 'password' ? (
            <form onSubmit={handlePasswordSubmit}>
              <div className="form-group">
                <label htmlFor="password">Company Password</label>
                <input
                  id="password"
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="Enter company password"
                  disabled={loading}
                  autoFocus
                />
                <p className="field-hint">Required to prevent unauthorized removal</p>
              </div>

              {error && <div className="error-message">{error}</div>}

              <button
                type="submit"
                className="btn-primary"
                disabled={loading || !password.trim()}
              >
                {loading ? 'Verifying...' : 'Continue to OTP'}
              </button>
            </form>
          ) : (
            <form onSubmit={handleOtpSubmit}>
              <div className="form-group">
                <label htmlFor="otp">Admin OTP</label>
                <input
                  id="otp"
                  type="text"
                  value={otp}
                  onChange={(e) => setOtp(e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, ''))}
                  placeholder="Enter OTP"
                  disabled={loading}
                  autoFocus
                  maxLength={6}
                />
                <p className="field-hint">
                  {otpSent
                    ? 'Check your registered admin email for the OTP'
                    : 'Click "Send OTP" to receive the code'}
                </p>
              </div>

              {error && (
                <div className={error.includes('sent') ? 'success-message' : 'error-message'}>
                  {error}
                </div>
              )}

              <div className="button-group">
                <button
                  type="button"
                  className="btn-secondary"
                  onClick={handleSendOtp}
                  disabled={loading || otpSent}
                >
                  {loading ? 'Sending...' : otpSent ? 'OTP Sent' : 'Send OTP'}
                </button>
                <button
                  type="submit"
                  className="btn-primary"
                  disabled={loading || !otp.trim()}
                >
                  {loading ? 'Verifying...' : 'Verify & Uninstall'}
                </button>
              </div>
            </form>
          )}
        </div>

        {/* Footer */}
        <div className="uninstall-auth-footer">
          <button
            type="button"
            className="btn-cancel"
            onClick={handleCancel}
            disabled={loading}
          >
            Cancel Uninstallation
          </button>
          <p className="footer-text">
            If you don't have the password or OTP, contact your administrator.
          </p>
        </div>
      </div>
    </div>
  );
};

export default UninstallAuth;
