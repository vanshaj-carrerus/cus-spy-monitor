; CUS Monitor — uninstall authorization (hardcoded password + admin OTP)
; Used by Tauri NSIS via bundle.windows.nsis.installerHooks

!include "LogicLib.nsh"

!macro NSIS_HOOK_PREUNINSTALL
  MessageBox MB_ICONINFORMATION|MB_OK "CUS Monitor uninstall requires:$\r$\n$\r$\n1) Company password$\r$\n2) Admin OTP from https://app.custech.co$\r$\n$\r$\nClick OK to continue."

  ClearErrors
  nsExec::ExecToStack '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -Command "Add-Type -AssemblyName Microsoft.VisualBasic; Add-Type -AssemblyName System.Windows.Forms; $expected = '"'"'CUS@878806'"'"'; $password = [Microsoft.VisualBasic.Interaction]::InputBox('"'"'Enter the company password:'"'"','"'"'CUS Monitor - Uninstall'"'"'); if ([string]::IsNullOrWhiteSpace($password)) { exit 1 }; if ($password -ne $expected) { [System.Windows.Forms.MessageBox]::Show('"'"'Incorrect password. Uninstall denied.'"'"','"'"'CUS Monitor'"'"',[System.Windows.Forms.MessageBoxButtons]::OK,[System.Windows.Forms.MessageBoxIcon]::Error); exit 1 }; $otp = [Microsoft.VisualBasic.Interaction]::InputBox('"'"'Enter the admin OTP from app.custech.co:'"'"','"'"'CUS Monitor - Uninstall'"'"'); if ([string]::IsNullOrWhiteSpace($otp)) { exit 1 }; try { $body = @{ password = $password; otp = $otp.Trim() } | ConvertTo-Json; $resp = Invoke-RestMethod -Uri '"'"'https://app.custech.co/api/uninstall/otp/verify'"'"' -Method POST -Body $body -ContentType '"'"'application/json; charset=utf-8'"'"'; if (-not $resp.success) { [System.Windows.Forms.MessageBox]::Show('"'"'Invalid or expired OTP. Uninstall denied.'"'"','"'"'CUS Monitor'"'"',[System.Windows.Forms.MessageBoxButtons]::OK,[System.Windows.Forms.MessageBoxIcon]::Error); exit 1 } } catch { [System.Windows.Forms.MessageBox]::Show(('"'"'Could not verify uninstall access: '"'"' + $_.Exception.Message),'"'"'CUS Monitor'"'"',[System.Windows.Forms.MessageBoxButtons]::OK,[System.Windows.Forms.MessageBoxIcon]::Error); exit 1 }; exit 0"'
  Pop $R0
  Pop $R1

  DetailPrint "Uninstall auth exit code: $R0"
  DetailPrint "Uninstall auth output: $R1"

  ${If} $R0 != 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "Uninstall cancelled or unauthorized.$\r$\n$\r$\nPassword + admin OTP are required to remove CUS Monitor."
    Abort
  ${EndIf}

  ReadEnvStr $R8 LOCALAPPDATA
  ${If} $R8 == ""
    StrCpy $R8 "$PROFILE\AppData\Local"
  ${EndIf}
  CreateDirectory "$R8\cus-monitor"
  FileOpen $0 "$R8\cus-monitor\uninstall.lock" w
  FileWrite $0 "uninstall"
  FileClose $0
  Sleep 1000

  nsExec::ExecToLog 'taskkill /F /IM "cus-monitor.exe" /T'
  Sleep 1500
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ReadEnvStr $R8 LOCALAPPDATA
  ${If} $R8 == ""
    StrCpy $R8 "$PROFILE\AppData\Local"
  ${EndIf}
  RMDir /r "$R8\cus-monitor"
!macroend
