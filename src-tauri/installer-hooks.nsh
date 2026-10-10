; Uninstall reverts the opt-in edits to Claude Code's settings.json before files go; a manual
; upgrade runs this too, and the new version turns them back on at its first start. App data
; lives in %LOCALAPPDATA%\AIUsageTracker, not the bundle-id folder NSIS knows, so remove it here.

!macro NSIS_HOOK_POSTINSTALL
  ; logs and WebView cache left from before the bundle id became io.aiusagetracker.desktop (0.2.8)
  RMDir /r "$LOCALAPPDATA\io.aiusagetracker.app"
  RMDir /r "$APPDATA\io.aiusagetracker.app"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; close the app first: a running copy would write its capture settings back, and cancelling
  ; here must leave them untouched
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  ; an update also runs the old uninstaller (with /UPDATE); keep capture settings then
  ${If} $UpdateMode <> 1
  ${AndIf} ${FileExists} "$INSTDIR\ai-usage-tracker.exe"
    ExecWait '"$INSTDIR\ai-usage-tracker.exe" --revert-capture' $0
    ${If} $0 <> 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "AI Usage Tracker could not undo all of its changes to Claude Code's settings.json (the status line or the telemetry export to 127.0.0.1). Please remove them yourself (usually in %USERPROFILE%\.claude\settings.json)." /SD IDOK
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; the app writes its start-with-Windows entry for the current user only; an update keeps it and
  ; the "Disabled" mark Task Manager keeps under StartupApproved
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "AI Usage Tracker"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "AI Usage Tracker"
  ${EndIf}
  ${If} $DeleteAppDataCheckboxState = 1
    RMDir /r "$LOCALAPPDATA\AIUsageTracker"
    ; the private copy left when the app was started from the Claude desktop app (MSIX)
    FindFirst $0 $1 "$LOCALAPPDATA\Packages\Claude_*"
    ${DoWhile} $1 != ""
      RMDir /r "$LOCALAPPDATA\Packages\$1\LocalCache\Local\AIUsageTracker"
      FindNext $0 $1
    ${Loop}
    FindClose $0
  ${EndIf}
!macroend
