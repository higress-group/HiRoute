!include nsDialogs.nsh
!include LogicLib.nsh
Var HiRouteLoginChoice
Var HiRouteTrayChoice
Var HiRouteLoginCheckbox
Var HiRouteTrayCheckbox
Var HiRouteArguments
Var HiRouteRestoreConsent
Var HiRoutePrepared

!macro HiRouteRestoreConfirmation PREFIX
Function ${PREFIX}HiRouteConfirmUnavailableRestore
  StrCpy $HiRouteRestoreConsent 0
  IfSilent hiroute_restore_declined
  ${If} $PassiveMode = 1
    Return
  ${EndIf}
  MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "HiRoute cannot read Agent settings because its background service is unavailable. Continuing stops only this account's HiRoute Task and may interrupt active work. Affected Agents cannot be listed; any HiRoute-configured Agent may need manual restoration. All HiRoute data will be kept. Continue without verified Agent restoration?" /SD IDNO IDNO hiroute_restore_declined
  StrCpy $HiRouteRestoreConsent 1
  hiroute_restore_declined:
FunctionEnd
!macroend
!insertmacro HiRouteRestoreConfirmation ""
!insertmacro HiRouteRestoreConfirmation "un."

Function HiRouteHistoryNotice
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
  IfSilent 0 +2
    Abort
  !insertmacro MUI_HEADER_TEXT "Upgrade and history / 升级与历史" "Keep the complete recovery material / 请保留完整恢复材料"
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateLabel} 0 0 100% 100% "When upgrading old HiRoute storage, configuration, credentials and routing continue. Old HiRoute observations are sealed with the matching old program; current history and statistics begin at cutover. Old-period continuation/history references require complete-group recovery with the old program. Native Agent history is unchanged. Service restarts can interrupt availability. Keep all sealed history and recovery packages.$\r$\n$\r$\n升级旧存储时，配置、凭据和路由会保留。旧 HiRoute 历史与配套旧程序一起封存，新历史与统计从切换时开始；旧时期的继续会话与历史引用需要整组恢复到配套旧版本。原生 Agent 历史不变。服务重启会短暂影响使用，请保留全部封存历史与恢复包。"
  Pop $0
  nsDialogs::Show
FunctionEnd

Function HiRouteOptions
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
  IfSilent 0 +2
    Abort
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateLabel} 0 0 100% 32u "HiRoute installs for this account in LocalAppData\Programs\HiRoute. Background service data is kept separately."
  Pop $0
  ${NSD_CreateCheckbox} 0 45u 100% 14u "Start HiRoute and its background service at login"
  Pop $HiRouteLoginCheckbox
  ${If} $HiRouteLoginChoice != 0
    ${NSD_Check} $HiRouteLoginCheckbox
  ${EndIf}
  ${NSD_CreateCheckbox} 0 70u 100% 14u "Show tray icon and keep the window available in the tray"
  Pop $HiRouteTrayCheckbox
  ${If} $HiRouteTrayChoice != 0
    ${NSD_Check} $HiRouteTrayCheckbox
  ${EndIf}
  nsDialogs::Show
FunctionEnd
Function HiRouteOptionsLeave
  ${NSD_GetState} $HiRouteLoginCheckbox $HiRouteLoginChoice
  ${NSD_GetState} $HiRouteTrayCheckbox $HiRouteTrayChoice
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  StrCmp $INSTDIR "$LOCALAPPDATA\Programs\HiRoute" +3
    MessageBox MB_ICONSTOP "HiRoute install directory is invalid. No product files were written." /SD IDOK
    Abort
  nsExec::ExecToStack '"$PLUGINSDIR\hiroute-preflight.exe" --installer-prepare "$PLUGINSDIR\hiroute-package.json" "$PLUGINSDIR\hirouted-preflight.exe"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "HiRoute could not prepare the upgrade. Installed programs were not replaced and existing data is preserved. $1" /SD IDOK
    Abort
  ${EndIf}
  StrCpy $HiRoutePrepared 1
!macroend

!macro NSIS_HOOK_POSTINSTALL
  StrCpy $HiRouteArguments ""
  ${If} $HiRouteLoginChoice = 0
    StrCpy $HiRouteArguments "$HiRouteArguments --no-login-start"
  ${EndIf}
  ${If} $HiRouteTrayChoice = 0
    StrCpy $HiRouteArguments "$HiRouteArguments --no-tray"
  ${EndIf}
  nsExec::ExecToStack '"$INSTDIR\hiroute-shell.exe" --installer-install $HiRouteArguments'
  Pop $0
  Pop $1
  ${If} $0 = 20
    ${IfThen} $PassiveMode = 1 ${|} Goto hiroute_install_failed ${|}
    IfSilent hiroute_install_failed
    MessageBox MB_YESNO|MB_ICONEXCLAMATION "The older background service cannot drain work. Stopping it may interrupt active requests and Worker tasks. Stop it and continue?" IDNO hiroute_install_failed
    nsExec::ExecToStack '"$INSTDIR\hiroute-shell.exe" --installer-install --allow-legacy-stop $HiRouteArguments'
    Pop $0
    Pop $1
  ${EndIf}
  ${If} $0 != 0
    hiroute_install_failed:
    MessageBox MB_ICONSTOP "HiRoute installation did not complete. Existing data is preserved. $1" /SD IDOK
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode != 1
    StrCpy $HiRouteArguments ""
    ${If} $DeleteAppDataCheckboxState = 1
      StrCpy $HiRouteArguments "--remove-data"
    ${EndIf}
    nsExec::ExecToStack '"$PLUGINSDIR\hiroute-uninstall-helper.exe" --installer-uninstall $HiRouteArguments'
    Pop $0
    Pop $1
    ${If} $0 = 23
      Call un.HiRouteConfirmUnavailableRestore
      ${If} $HiRouteRestoreConsent = 1
        ; Unverified restoration always retains the complete business data.
        StrCpy $DeleteAppDataCheckboxState 0
        StrCpy $HiRouteArguments "--allow-unavailable-agent-restore"
        nsExec::ExecToStack '"$PLUGINSDIR\hiroute-uninstall-helper.exe" --installer-uninstall $HiRouteArguments'
        Pop $0
        Pop $1
      ${EndIf}
    ${EndIf}
    ${If} $0 = 20
      ${IfThen} $PassiveMode = 1 ${|} Goto hiroute_uninstall_failed ${|}
      IfSilent hiroute_uninstall_failed
      MessageBox MB_YESNO|MB_ICONEXCLAMATION "The older background service cannot drain work. Stopping it may interrupt active requests and Worker tasks. Stop it and continue uninstalling?" IDNO hiroute_uninstall_failed
      nsExec::ExecToStack '"$PLUGINSDIR\hiroute-uninstall-helper.exe" --installer-uninstall --allow-legacy-stop $HiRouteArguments'
      Pop $0
      Pop $1
    ${EndIf}
    ${If} $0 != 0
      hiroute_uninstall_failed:
      MessageBox MB_ICONSTOP "HiRoute was not removed. Resolve any Agent restore decisions or active work, then retry. $1" /SD IDOK
      Abort
    ${EndIf}
  ${EndIf}
!macroend
