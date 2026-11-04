!include nsDialogs.nsh
!include LogicLib.nsh

!macro HiRouteDiagnostic SUMMARY STEP
  MessageBox MB_ICONSTOP "${SUMMARY} Press Ctrl+C to copy the diagnostic.$\r$\n$\r$\nHiRoute ${VERSION} | Setup step: ${STEP} | Exit: $0$\r$\n$1" /SD IDOK
!macroend
Var HiRouteLoginChoice
Var HiRouteTrayChoice
Var HiRouteLoginCheckbox
Var HiRouteTrayCheckbox
Var HiRouteDesktopCheckbox
Var HiRouteDesktopChoice
Var HiRouteOptionsToggle
Var HiRouteInstallPath
Var HiRouteInstalledVersion
Var HiRoutePrimary
Var HiRouteProcessStatus
Var HiRoutePageIcon
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

!macro HiRoutePhase TEXT
  SetDetailsPrint textonly
  DetailPrint "${TEXT}"
  SetDetailsPrint listonly
  DetailPrint "${TEXT}"
!macroend

!macro HiRouteRequireDesktopExit
  hiroute_check_desktop:
    nsis_tauri_utils::FindProcessCurrentUser "${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 != 1
      ${If} $PassiveMode = 1
        Abort "Exit HiRoute and try again."
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Exit HiRoute from its notification area icon, then select Retry. If the tray icon is disabled, close the HiRoute window." /SD IDCANCEL IDRETRY hiroute_check_desktop
      Abort
    ${EndIf}
!macroend

Function HiRouteSetOptionsVisibility
  Pop $0
  ${NSD_GetState} $HiRouteOptionsToggle $0
  ${If} $0 = ${BST_CHECKED}
    StrCpy $0 ${SW_SHOW}
  ${Else}
    StrCpy $0 ${SW_HIDE}
  ${EndIf}
  ShowWindow $HiRouteInstallPath $0
  ShowWindow $HiRouteLoginCheckbox $0
  ShowWindow $HiRouteTrayCheckbox $0
  ShowWindow $HiRouteDesktopCheckbox $0
FunctionEnd

Function HiRouteOptions
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
  IfSilent 0 +2
    Abort
  nsDialogs::Create 1018
  Pop $0
  ${IfThen} $0 == error ${|} Abort ${|}
  StrCpy $HiRoutePrimary "Install"
  StrCpy $1 "Set up HiRoute ${VERSION} for this Windows account."
  ${If} $HiRouteInstalledVersion != ""
    nsis_tauri_utils::SemverCompare "${VERSION}" $HiRouteInstalledVersion
    Pop $0
    ${If} $0 = 0
      StrCpy $HiRoutePrimary "Repair"
      StrCpy $1 "Repair HiRoute ${VERSION} and keep your settings."
    ${Else}
      StrCpy $HiRoutePrimary "Upgrade"
      StrCpy $1 "Upgrade HiRoute from $HiRouteInstalledVersion to ${VERSION}."
    ${EndIf}
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "HiRoute for Windows" "$HiRoutePrimary for this Windows account"
  ${NSD_CreateIcon} 0 3u 28u 28u ""
  Pop $0
  ${NSD_SetIconFromInstaller} $0 $HiRoutePageIcon
  ${NSD_CreateLabel} 34u 4u -34u 26u "$1"
  Pop $0
  StrCpy $1 "Your settings and data are kept during setup."
  nsis_tauri_utils::FindProcessCurrentUser "${MAINBINARYNAME}.exe"
  Pop $HiRouteProcessStatus
  ${If} $HiRouteProcessStatus = 0
    StrCpy $1 "HiRoute is running. Exit it from the notification area before continuing."
  ${EndIf}
  ${NSD_CreateLabel} 34u 30u -34u 23u "$1"
  Pop $0
  ${NSD_CreateCheckbox} 0 58u 100% 13u "Show installation options"
  Pop $HiRouteOptionsToggle
  ${NSD_OnClick} $HiRouteOptionsToggle HiRouteSetOptionsVisibility
  ${NSD_CreateText} 8u 75u -8u 13u "$INSTDIR"
  Pop $HiRouteInstallPath
  SendMessage $HiRouteInstallPath ${EM_SETREADONLY} 1 0
  ${NSD_CreateCheckbox} 8u 93u -8u 12u "Start HiRoute and its background service at login"
  Pop $HiRouteLoginCheckbox
  ${If} $HiRouteLoginChoice != 0
    ${NSD_Check} $HiRouteLoginCheckbox
  ${EndIf}
  ${NSD_CreateCheckbox} 8u 107u -8u 12u "Show HiRoute in the notification area"
  Pop $HiRouteTrayCheckbox
  ${If} $HiRouteTrayChoice != 0
    ${NSD_Check} $HiRouteTrayCheckbox
  ${EndIf}
  ${NSD_CreateCheckbox} 8u 121u -8u 12u "Add a desktop shortcut"
  Pop $HiRouteDesktopCheckbox
  ${If} $HiRouteDesktopChoice != 0
    ${NSD_Check} $HiRouteDesktopCheckbox
  ${EndIf}
  ${IfThen} $NoShortcutMode = 1 ${|} EnableWindow $HiRouteDesktopCheckbox 0 ${|}
  Push $HiRouteOptionsToggle
  Call HiRouteSetOptionsVisibility
  GetDlgItem $0 $HWNDPARENT 1
  ${NSD_SetText} $0 $HiRoutePrimary
  GetDlgItem $0 $HWNDPARENT 3
  ShowWindow $0 ${SW_HIDE}
  nsDialogs::Show
  ${NSD_FreeIcon} $HiRoutePageIcon
FunctionEnd
Function HiRouteOptionsLeave
  hiroute_verify_desktop:
    nsis_tauri_utils::FindProcessCurrentUser "${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 != 1
      MessageBox MB_RETRYCANCEL|MB_ICONINFORMATION "Exit HiRoute from its notification area icon, then select Retry. If the tray icon is disabled, close the HiRoute window." IDRETRY hiroute_verify_desktop
      Abort
    ${EndIf}
  ${NSD_GetState} $HiRouteLoginCheckbox $HiRouteLoginChoice
  ${NSD_GetState} $HiRouteTrayCheckbox $HiRouteTrayChoice
  ${NSD_GetState} $HiRouteDesktopCheckbox $HiRouteDesktopChoice
FunctionEnd

Function HiRouteReady
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
  nsDialogs::Create 1018
  Pop $0
  ${IfThen} $0 == error ${|} Abort ${|}
  !insertmacro MUI_HEADER_TEXT "HiRoute is ready" "${VERSION}"
  ${NSD_CreateIcon} 0 5u 28u 28u ""
  Pop $0
  ${NSD_SetIconFromInstaller} $0 $HiRoutePageIcon
  ${NSD_CreateLabel} 34u 9u -34u 25u "Installation is complete and the background service is ready."
  Pop $0
  ${NSD_CreateButton} 34u 45u 85u 17u "Open HiRoute"
  Pop $0
  ${NSD_OnClick} $0 HiRouteReadyOpen
  GetDlgItem $0 $HWNDPARENT 1
  ${NSD_SetText} $0 "Close"
  GetDlgItem $0 $HWNDPARENT 2
  EnableWindow $0 0
  nsDialogs::Show
  ${NSD_FreeIcon} $HiRoutePageIcon
FunctionEnd

Function HiRouteReadyOpen
  Pop $0
  Call RunMainBinary
  SendMessage $HWNDPARENT ${WM_COMMAND} 1 0
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  !insertmacro HiRouteRequireDesktopExit
  !insertmacro HiRoutePhase "Preparing installation and checking existing data..."
  StrCmp $INSTDIR "$LOCALAPPDATA\Programs\HiRoute" +3
    MessageBox MB_ICONSTOP "HiRoute install directory is invalid. No product files were written." /SD IDOK
    Abort
  nsExec::ExecToStack '"$PLUGINSDIR\hiroute-preflight.exe" --installer-prepare "$PLUGINSDIR\hiroute-package.json" "$PLUGINSDIR\hirouted-preflight.exe"'
  Pop $0
  Pop $1
  ${If} $0 != 0
  ${AndIf} $0 != 24
    DetailPrint "$1"
    !insertmacro HiRouteDiagnostic "HiRoute could not prepare the installation. Keep your existing data." "prepare-installation"
    Abort
  ${EndIf}
  StrCpy $HiRoutePrepared 1
  ${If} $0 = 24
    ${AndIf} $PassiveMode != 1
    IfSilent hiroute_history_acknowledged
    MessageBox MB_OKCANCEL|MB_ICONINFORMATION "This upgrade starts a new HiRoute history. Your settings, credentials and routes are kept. Previous HiRoute history requires recovery with the matching old program. Native Agent history is unchanged." IDOK hiroute_history_acknowledged
    Abort
  ${EndIf}
  hiroute_history_acknowledged:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro HiRoutePhase "Configuring HiRoute and waiting for its background service..."
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
    DetailPrint "$1"
    !insertmacro HiRouteDiagnostic "HiRoute installation did not complete. Keep your existing data." "install"
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode != 1
    !insertmacro HiRoutePhase "Checking Agent configuration and stopping the background service..."
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
      !insertmacro HiRouteDiagnostic "HiRoute was not removed. Resolve any Agent restore decisions or active work, then retry." "uninstall"
      Abort
    ${EndIf}
  ${EndIf}
!macroend
