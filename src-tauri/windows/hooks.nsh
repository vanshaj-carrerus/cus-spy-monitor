; CUS Monitor — uninstall authorization (hardcoded password + admin OTP)
; Used by Tauri NSIS via bundle.windows.nsis.installerHooks

!include "LogicLib.nsh"

!macro NSIS_HOOK_PREUNINSTALL
  ; Make it obvious the custom uninstall gate is running
  MessageBox MB_ICONINFORMATION|MB_OK "CUS Monitor uninstall requires:$\r$\n$\r$\n1) Company password$\r$\n2) Admin OTP from https://app.custech.co$\r$\n$\r$\nClick OK to continue."

  ; Write auth script (avoid NSIS '#' comment pitfalls by building password in PS)
  FileOpen $0 "$TEMP\cus-monitor-uninstall-auth.ps1" w
  FileWrite $0 "Add-Type -AssemblyName Microsoft.VisualBasic$\r$\n"
  FileWrite $0 "Add-Type -AssemblyName System.Windows.Forms$\r$\n"
  FileWrite $0 "$$expected = 'cus@12345_54321' + [char]35$\r$\n"
  FileWrite $0 "$$pw = [Microsoft.VisualBasic.Interaction]::InputBox('Enter the uninstall password:','CUS Monitor - Uninstall')$\r$\n"
  FileWrite $0 "if ($$pw -cne $$expected) { [System.Windows.Forms.MessageBox]::Show('Incorrect password. Uninstall denied.','CUS Monitor',[System.Windows.Forms.MessageBoxButtons]::OK,[System.Windows.Forms.MessageBoxIcon]::Error); exit 1 }$\r$\n"
  FileWrite $0 "$$otp = [Microsoft.VisualBasic.Interaction]::InputBox('Enter the admin OTP from app.custech.co:','CUS Monitor - Uninstall')$\r$\n"
  FileWrite $0 "if ([string]::IsNullOrWhiteSpace($$otp)) { exit 1 }$\r$\n"
  FileWrite $0 "try {$\r$\n"
  FileWrite $0 "  $$body = @{ otp = $$otp.Trim() } | ConvertTo-Json$\r$\n"
  FileWrite $0 "  $$resp = Invoke-RestMethod -Uri 'https://app.custech.co/api/uninstall/otp/verify' -Method POST -Body $$body -ContentType 'application/json; charset=utf-8'$\r$\n"
  FileWrite $0 "  if (-not $$resp.success) { [System.Windows.Forms.MessageBox]::Show('Invalid or expired OTP. Uninstall denied.','CUS Monitor',[System.Windows.Forms.MessageBoxButtons]::OK,[System.Windows.Forms.MessageBoxIcon]::Error); exit 1 }$\r$\n"
  FileWrite $0 "} catch {$\r$\n"
  FileWrite $0 "  [System.Windows.Forms.MessageBox]::Show(('Could not verify OTP: ' + $$_.Exception.Message),'CUS Monitor',[System.Windows.Forms.MessageBoxButtons]::OK,[System.Windows.Forms.MessageBoxIcon]::Error); exit 1$\r$\n"
  FileWrite $0 "}$\r$\n"
  FileWrite $0 "exit 0$\r$\n"
  FileClose $0

  ClearErrors
  nsExec::ExecToStack '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$TEMP\cus-monitor-uninstall-auth.ps1"'
  Pop $R0
  Pop $R1
  Delete "$TEMP\cus-monitor-uninstall-auth.ps1"

  DetailPrint "Uninstall auth exit code: $R0"
  DetailPrint "Uninstall auth output: $R1"

  ${If} $R0 != 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "Uninstall cancelled or unauthorized.$\r$\n$\r$\nPassword + admin OTP are required to remove CUS Monitor.$\r$\n$\r$\nExit code: $R0"
    Abort
  ${EndIf}

  ; Stop sibling watchdogs from respawning, then kill all instances
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
