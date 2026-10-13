; Register the browser host for this user even when "Launch SDM" is unchecked.
!macro NSIS_HOOK_POSTINSTALL
  ExecWait '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --register-browser-connection'
!macroend

; The standard Tauri NSIS uninstaller removes the Run entry. Remove its
; matching Task Manager approval entry too, preserving both during updates.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Supo Download Manager"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Supo Download Manager"
    DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\com.supodownloadmanager.bridge"
    DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\com.supodownloadmanager.bridge"
    DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\com.supodownloadmanager.bridge"
    DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.supodownloadmanager.bridge"
    DeleteRegKey HKCU "Software\CocCoc\Browser\NativeMessagingHosts\com.supodownloadmanager.bridge"
    DeleteRegKey HKCU "Software\Opera Software\NativeMessagingHosts\com.supodownloadmanager.bridge"
    DeleteRegKey HKCU "Software\Vivaldi\NativeMessagingHosts\com.supodownloadmanager.bridge"
    DeleteRegKey HKCU "Software\Mozilla\NativeMessagingHosts\com.supodownloadmanager.bridge"
    Delete "$APPDATA\com.supodownloadmanager.app\native-messaging\chromium\com.supodownloadmanager.bridge.json"
    Delete "$APPDATA\com.supodownloadmanager.app\native-messaging\chromium\com.supodownloadmanager.bridge.json.bak"
    Delete "$APPDATA\com.supodownloadmanager.app\native-messaging\firefox\com.supodownloadmanager.bridge.json"
    Delete "$APPDATA\com.supodownloadmanager.app\native-messaging\firefox\com.supodownloadmanager.bridge.json.bak"
    RMDir "$APPDATA\com.supodownloadmanager.app\native-messaging\chromium"
    RMDir "$APPDATA\com.supodownloadmanager.app\native-messaging\firefox"
    RMDir "$APPDATA\com.supodownloadmanager.app\native-messaging"
  ${EndIf}
!macroend
