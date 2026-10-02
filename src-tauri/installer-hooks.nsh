; Clean uninstall for AI Usage Tracker.
; 1. Before files are removed, undo every opt-in live-capture change made outside the app's
;    folder (Claude Code statusLine / telemetry env in settings.json).
; 2. Remove the "start with Windows" entry.
; 3. The archive lives in %LOCALAPPDATA%\AIUsageTracker (not the default bundle-id folder), so
;    remove it when the user ticks "Delete the application data".

!macro NSIS_HOOK_PREUNINSTALL
  ; an update also runs the old uninstaller (with /UPDATE); keep capture settings then
  ${If} $UpdateMode <> 1
  ${AndIf} ${FileExists} "$INSTDIR\ai-usage-tracker.exe"
    ExecWait '"$INSTDIR\ai-usage-tracker.exe" --revert-capture'
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "AI Usage Tracker"
  ${If} $DeleteAppDataCheckboxState = 1
    RMDir /r "$LOCALAPPDATA\AIUsageTracker"
  ${EndIf}
!macroend
