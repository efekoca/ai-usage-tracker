; Uninstall reverts the opt-in edits to Claude Code's settings.json before files go. App data
; lives in %LOCALAPPDATA%\AIUsageTracker, not the bundle-id folder NSIS knows, so remove it here.

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
