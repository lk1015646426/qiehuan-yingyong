; Clean up historical desktop / start-menu shortcut names during manual installs.
; Tauri skips shortcut creation in /UPDATE mode, so updater runs must preserve
; every existing shortcut. The current product-name shortcut is managed by Tauri.

Var ShortcutRepairRequired

!macro NSIS_HOOK_PREINSTALL
  StrCpy $ShortcutRepairRequired 0

  ; 升级安装：旧版本可能仍在运行（托盘常驻），文件被锁会导致覆盖失败。
  ; 静默结束运行中的进程后继续安装：
  ;  - qiehuan-yingyong.exe：当前机器标识（同产品名原地升级时的自锁定）。
  ;  - cockpit-tools.exe / trae-work-cn-switcher.exe：历史产品名副本。
  ; 注意：不能用 taskkill /T（进程树终止）——由本应用启动的
  ; WorkBuddy / TRAE 客户端是本应用的子进程，/T 会把它们一并杀掉。
  ; 按镜像名精确结束本应用自身即可释放文件锁。
  nsExec::Exec 'taskkill /F /IM "${PRODUCTNAME}.exe"'
  Pop $R9
  nsExec::Exec 'taskkill /F /IM qiehuan-yingyong.exe'
  Pop $R9
  nsExec::Exec 'taskkill /F /IM qiehuan_yingyong.exe'
  Pop $R9
  nsExec::Exec 'taskkill /F /IM cockpit-tools.exe'
  Pop $R9
  nsExec::Exec 'taskkill /F /IM trae-work-cn-switcher.exe'
  Pop $R9
  Sleep 500

  ; v1.3.15 removed both shortcuts during NSIS updater runs. Repair only that
  ; exact upgrade state so users who intentionally removed one are unaffected.
  ${If} $UpdateMode = 1
    Push $R9
    ReadRegStr $R9 SHCTX "${UNINSTKEY}" "DisplayVersion"
    ${If} $R9 == "1.3.15"
      ${IfNot} ${FileExists} "$DESKTOP\${PRODUCTNAME}.lnk"
        ${IfNot} ${FileExists} "$SMPROGRAMS\${PRODUCTNAME}.lnk"
          StrCpy $ShortcutRepairRequired 1
        ${EndIf}
      ${EndIf}
    ${EndIf}
    Pop $R9
  ${EndIf}

  ${If} $UpdateMode != 1
    ; Historical / alternate display names that may have left shortcuts behind
    Delete "$DESKTOP\Antigravity Cockpit Tools.lnk"
    Delete "$DESKTOP\Antigravity Cockpit.lnk"
    Delete "$SMPROGRAMS\Antigravity Cockpit Tools.lnk"
    Delete "$SMPROGRAMS\Antigravity Cockpit.lnk"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $ShortcutRepairRequired = 1
    ; Reuse Tauri's shortcut creation functions to keep targets and AppUserModelId consistent.
    StrCpy $UpdateMode 0
    Call CreateOrUpdateStartMenuShortcut
    Call CreateOrUpdateDesktopShortcut
    StrCpy $UpdateMode 1
  ${EndIf}
!macroend
