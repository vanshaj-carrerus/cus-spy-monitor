; CUS Monitor — uninstall authorization using in-app authentication UI
; Used by Tauri NSIS via bundle.windows.nsis.installerHooks

!include "LogicLib.nsh"

!macro NSIS_HOOK_PREUNINSTALL
  ; Store the installation directory
  StrCpy $R8 "$INSTDIR\CUS Monitor.exe"

  ; Check if the app executable exists
  ${If} ${FileExists} "$R8"
    DetailPrint "Launching CUS Monitor uninstall protection screen..."

    ; Launch the app with --request-uninstall-auth flag and wait for it to complete
    ; The app will show the password/OTP verification UI
    nsExec::ExecToStack '"$R8" --request-uninstall-auth'
    Pop $R0  ; Exit code
    Pop $R1  ; Output (if any)

    DetailPrint "Uninstall auth exit code: $R0"

    ; Check exit code:
    ; 0 = user verified credentials successfully, allow uninstall
    ; 1 = user cancelled or failed verification, abort uninstall
    ${If} $R0 != 0
      MessageBox MB_ICONEXCLAMATION|MB_OK "Uninstallation cancelled.$\r$\n$\r$\nPassword and OTP verification are required to uninstall CUS Monitor."
      Abort
    ${EndIf}

    DetailPrint "Uninstall credentials verified. Proceeding with uninstallation."

    ; Write uninstall lock file to prevent race conditions
    ReadEnvStr $R9 LOCALAPPDATA
    ${If} $R9 == ""
      StrCpy $R9 "$PROFILE\AppData\Local"
    ${EndIf}
    CreateDirectory "$R9\cus-monitor"
    FileOpen $R2 "$R9\cus-monitor\uninstall.lock" w
    FileWrite $R2 "uninstall"
    FileClose $R2
    Sleep 1000

    ; Kill any running instances
    nsExec::ExecToLog 'taskkill /F /IM "cus-monitor.exe" /T'
    Sleep 1500
  ${Else}
    ; App executable not found, proceed with basic uninstall
    DetailPrint "CUS Monitor executable not found. Skipping authentication."
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Clean up uninstall lock and cache
  ReadEnvStr $R8 LOCALAPPDATA
  ${If} $R8 == ""
    StrCpy $R8 "$PROFILE\AppData\Local"
  ${EndIf}
  RMDir /r "$R8\cus-monitor"
!macroend
