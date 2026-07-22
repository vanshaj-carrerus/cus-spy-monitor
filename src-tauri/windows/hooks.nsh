; CUS Monitor — uninstall authorization (password + admin OTP)
; Hooks into Tauri's NSIS installer via installerHooks.

!include "LogicLib.nsh"

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "CUS Monitor: verifying uninstall authorization..."

  ; Write auth script to TEMP (avoids quoting hell inside nsExec)
  FileOpen $0 "$TEMP\cus-monitor-uninstall-auth.ps1" w
  FileWrite $0 "Add-Type -AssemblyName Microsoft.VisualBasic$\r$\n"
  FileWrite $0 "Add-Type -AssemblyName System.Windows.Forms$\r$\n"
  FileWrite $0 "$$pw = [Microsoft.VisualBasic.Interaction]::InputBox('Enter the uninstall password:','CUS Monitor - Uninstall')$\r$\n"
  FileWrite $0 "if ($$pw -cne 'cus@12345_54321#') { [System.Windows.Forms.MessageBox]::Show('Incorrect password. Uninstall denied.','CUS Monitor',[System.Windows.Forms.MessageBoxButtons]::OK,[System.Windows.Forms.MessageBoxIcon]::Error); exit 1 }$\r$\n"
  FileWrite $0 "$$otp = [Microsoft.VisualBasic.Interaction]::InputBox('Enter the admin OTP from app.custech.co:','CUS Monitor - Uninstall')$\r$\n"
  FileWrite $0 "if ([string]::IsNullOrWhiteSpace($$otp)) { exit 1 }$\r$\n"
  FileWrite $0 "try {$\r$\n"
  FileWrite $0 "  $$body = @{ otp = $$otp.Trim() } | ConvertTo-Json$\r$\n"
  FileWrite $0 "  $$resp = Invoke-RestMethod -Uri 'https://app.custech.co/api/uninstall/otp/verify' -Method POST -Body $$body -ContentType 'application/json; charset=utf-8'$\r$\n"
  FileWrite $0 "  if (-not $$resp.success) { [System.Windows.Forms.MessageBox]::Show('Invalid or expired OTP. Uninstall denied.','CUS Monitor',[System.Windows.Forms.MessageBoxButtons]::OK,[System.Windows.Forms.MessageBoxIcon]::Error); exit 1 }$\r$\n"
  FileWrite $0 "} catch {$\r$\n"
  FileWrite $0 "  [System.Windows.Forms.MessageBox]::Show('Could not verify OTP. Check internet connection and try again.','CUS Monitor',[System.Windows.Forms.MessageBoxButtons]::OK,[System.Windows.Forms.MessageBoxIcon]::Error); exit 1$\r$\n"
  FileWrite $0 "}$\r$\n"
  FileWrite $0 "exit 0$\r$\n"
  FileClose $0

  nsExec::ExecToStack 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$TEMP\cus-monitor-uninstall-auth.ps1"'
  Pop $R0
  Pop $R1
  Delete "$TEMP\cus-monitor-uninstall-auth.ps1"

  ${If} $R0 != 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "Uninstall cancelled or unauthorized.$\r$\n$\r$\nPassword + admin OTP are required to remove CUS Monitor."
    Abort
  ${EndIf}

  ; Signal watchdogs to stop respawning, then kill all siblings
  CreateDirectory "$LOCALAPPDATA\cus-monitor"
  FileOpen $0 "$LOCALAPPDATA\cus-monitor\uninstall.lock" w
  FileWrite $0 "uninstall"
  FileClose $0
  Sleep 1000

  nsExec::ExecToLog 'taskkill /F /IM "cus-monitor.exe" /T'
  Sleep 1500
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Clean local monitor state
  RMDir /r "$LOCALAPPDATA\cus-monitor"
!macroend
