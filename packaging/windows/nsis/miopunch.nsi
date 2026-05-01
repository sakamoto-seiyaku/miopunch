; miopunch Windows installer (NSIS)
;
; v0 contract:
; - Install to %ProgramFiles%\miopunch\
; - Copy miopunch.exe + miopunch-desktop.exe
; - Call: miopunch install-system-daemon (fail-fast)
; - Append installer logs: %ProgramData%\miopunch\install.log
; - Provide "Export log" UI to copy installer log to a user-selected path

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "nsDialogs.nsh"
!include "FileFunc.nsh"

Unicode true
Name "miopunch"
OutFile "miopunch-setup.exe"
InstallDir "$PROGRAMFILES\\miopunch"
RequestExecutionLevel admin

Var LogFile
Var LogDir
Var LogExportPath
Var LogExportPathCtl

!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
Page custom LogExportPage
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "English"

Function .onInit
  Call SetLogPath
  CreateDirectory "$LogDir"
  FileOpen $0 $LogFile a
  FileWrite $0 "=== miopunch installer start ===$\r$\n"
  FileClose $0
FunctionEnd

Function SetLogPath
  ReadEnvStr $LogDir "ProgramData"
  ${If} $LogDir == ""
    StrCpy $LogDir "$APPDATA"
  ${EndIf}
  StrCpy $LogDir "$LogDir\\miopunch"
  StrCpy $LogFile "$LogDir\\install.log"
FunctionEnd

Function LogLine
  Exch $0
  Push $1
  FileOpen $1 $LogFile a
  FileWrite $1 "$0$\r$\n"
  FileClose $1
  Pop $1
  Pop $0
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"

  Push "install_dir=$INSTDIR"
  Call LogLine

  ; These files are expected to be present next to the .nsi at compile time.
  File "miopunch.exe"
  File "miopunch-desktop.exe"

  WriteRegStr HKLM "Software\\miopunch" "InstallDir" "$INSTDIR"

  CreateDirectory "$SMPROGRAMS\\miopunch"
  CreateShortcut "$SMPROGRAMS\\miopunch\\miopunch.lnk" "$INSTDIR\\miopunch-desktop.exe"
  CreateShortcut "$DESKTOP\\miopunch.lnk" "$INSTDIR\\miopunch-desktop.exe"

  WriteUninstaller "$INSTDIR\\uninstall.exe"

  Push "calling: miopunch install-system-daemon"
  Call LogLine

  ; Fail-fast on service install error; write command output to installer log.
  ExecWait '"$SYSDIR\\cmd.exe" /c ""$INSTDIR\\miopunch.exe" install-system-daemon >> "$LogFile" 2>&1""' $0
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "miopunch install failed (install-system-daemon rc=$0). See: $LogFile"
    Abort
  ${EndIf}

  Push "install ok"
  Call LogLine
SectionEnd

Section "Uninstall"
  Call un.SetLogPath
  CreateDirectory "$LogDir"

  Push "=== miopunch uninstall start ==="
  Call un.LogLine

  ; Best-effort daemon uninstall; continue even on failures.
  ExecWait '"$SYSDIR\\cmd.exe" /c ""$INSTDIR\\miopunch.exe" uninstall-system-daemon >> "$LogFile" 2>&1""' $0
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "Warning: miopunch uninstall-system-daemon failed (rc=$0). State is preserved. See: $LogFile"
  ${EndIf}

  Delete "$DESKTOP\\miopunch.lnk"
  Delete "$SMPROGRAMS\\miopunch\\miopunch.lnk"
  RMDir "$SMPROGRAMS\\miopunch"

  Delete "$INSTDIR\\miopunch.exe"
  Delete "$INSTDIR\\miopunch-desktop.exe"
  Delete "$INSTDIR\\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKLM "Software\\miopunch"

  Push "=== miopunch uninstall done ==="
  Call un.LogLine
SectionEnd

Function un.SetLogPath
  ReadEnvStr $LogDir "ProgramData"
  ${If} $LogDir == ""
    StrCpy $LogDir "$APPDATA"
  ${EndIf}
  StrCpy $LogDir "$LogDir\\miopunch"
  StrCpy $LogFile "$LogDir\\install.log"
FunctionEnd

Function un.LogLine
  Exch $0
  Push $1
  FileOpen $1 $LogFile a
  FileWrite $1 "$0$\r$\n"
  FileClose $1
  Pop $1
  Pop $0
FunctionEnd

Function LogExportPage
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}

  ${NSD_CreateLabel} 0 0 100% 24u "Installer log: $LogFile"
  Pop $0

  ${NSD_CreateText} 0 28u 75% 12u "$DESKTOP\\miopunch-install.log"
  Pop $LogExportPathCtl

  ${NSD_CreateButton} 78% 28u 22% 12u "Export log"
  Pop $1
  ${NSD_OnClick} $1 LogExportClicked

  nsDialogs::Show
FunctionEnd

Function LogExportClicked
  ${NSD_GetText} $LogExportPathCtl $LogExportPath
  ${If} $LogExportPath == ""
    MessageBox MB_ICONEXCLAMATION "Please enter an output path."
    Return
  ${EndIf}

  ${GetParent} $LogExportPath $0
  CreateDirectory "$0"
  CopyFiles /SILENT "$LogFile" "$0"
  Rename "$0\\install.log" "$LogExportPath"
  MessageBox MB_ICONINFORMATION "Exported: $LogExportPath"
FunctionEnd
