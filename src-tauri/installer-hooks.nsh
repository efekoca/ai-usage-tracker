; Clean uninstall for AI Usage Tracker.
; The app keeps its archive in %LOCALAPPDATA%\AIUsageTracker (not the default bundle-id
; folder), so remove it when the user ticks "Delete the application data", and always
; remove the "start with Windows" entry the app may have created.

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "AI Usage Tracker"
  ${If} $DeleteAppDataCheckboxState = 1
    RMDir /r "$LOCALAPPDATA\AIUsageTracker"
  ${EndIf}
!macroend
