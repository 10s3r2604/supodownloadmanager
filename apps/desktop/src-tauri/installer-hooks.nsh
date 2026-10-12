; The standard Tauri NSIS uninstaller removes the Run entry. Remove its
; matching Task Manager approval entry too, preserving both during updates.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Supo Download Manager"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Supo Download Manager"
  ${EndIf}
!macroend
