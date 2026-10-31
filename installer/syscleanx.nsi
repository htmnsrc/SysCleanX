; SysCleanX installer (NSIS 3, Modern UI 2).
;
; Build from macOS/Linux:  installer/build.sh
; Build on Windows (NSIS installed, after `cargo build --release`):
;   makensis /DVERSION=2.0.0 /DEXE=../target/release/syscleanx.exe installer/syscleanx.nsi
;
; Installs to Program Files, adds Start menu (+ optional desktop) shortcuts, registers in
; Settings > Apps / Control Panel > Programs, and ships an uninstaller.

Unicode True
ManifestDPIAware true
RequestExecutionLevel admin
SetCompressor /SOLID lzma

!ifndef VERSION
  !error "Pass the version: makensis /DVERSION=x.y.z syscleanx.nsi"
!endif
!ifndef EXE
  !define EXE "build/SysCleanX.exe"
!endif
!ifndef LICENSE_TXT
  !define LICENSE_TXT "build/LICENSE.txt"
!endif
!ifndef OUTFILE
  !define OUTFILE "../dist/SysCleanX-Setup-${VERSION}.exe"
!endif

!define APP "SysCleanX"
!define PUBLISHER "Hitman"
!define EXE_NAME "SysCleanX.exe"
!define WEBSITE "https://discord.gg/Ayn7uzBgbC"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define APPPATHS_KEY "Software\Microsoft\Windows\CurrentVersion\App Paths\${EXE_NAME}"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "WinVer.nsh"
!include "FileFunc.nsh"

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APP}"
BrandingText "${APP} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "FileDescription" "${APP} Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2025 ${PUBLISHER}"

; ---------------------------------------------------------------- Look & pages

!define MUI_ICON "../crates/gui/assets/icon.ico"
!define MUI_UNICON "../crates/gui/assets/icon.ico"
!define MUI_WELCOMEFINISHPAGE_BITMAP "assets/welcome.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "assets/welcome.bmp"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "assets/header.bmp"
!define MUI_HEADERIMAGE_UNBITMAP "assets/header.bmp"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_NODESC

!define MUI_WELCOMEPAGE_TEXT "This will install ${APP} ${VERSION}, a fast and safe cleaner for Windows.$\r$\n$\r$\nIt removes junk files, uninstalls programs completely and lets you control what starts with Windows.$\r$\n$\r$\nClick Next to continue."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${LICENSE_TXT}"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE_NAME}"
!define MUI_FINISHPAGE_RUN_TEXT "Launch ${APP}"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ---------------------------------------------------------------- Helpers

; Deleting a running .exe fails on Windows, so a leftover file means the app is open.
; (Renaming would succeed even while it runs, so it can't be used as the test.)
!macro ENSURE_CLOSED UN
Function ${UN}EnsureAppClosed
  retry:
    ClearErrors
    Delete "$INSTDIR\${EXE_NAME}"
    ${If} ${FileExists} "$INSTDIR\${EXE_NAME}"
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${APP} is running.$\r$\nClose it, then click Retry." /SD IDCANCEL IDRETRY retry
      Abort
    ${EndIf}
FunctionEnd
!macroend
!insertmacro ENSURE_CLOSED ""
!insertmacro ENSURE_CLOSED "un."

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "${APP} needs 64-bit Windows 10 or 11." /SD IDOK
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_ICONSTOP "${APP} needs Windows 10 or newer." /SD IDOK
    Abort
  ${EndIf}
  ; Our keys live in the native 64-bit registry view (the installer itself is 32-bit).
  SetRegView 64
  ; Upgrades go to wherever the previous version was installed.
  ReadRegStr $0 HKLM "${UNINST_KEY}" "InstallLocation"
  ${If} $0 != ""
    StrCpy $INSTDIR $0
  ${EndIf}
FunctionEnd

Function un.onInit
  SetRegView 64
FunctionEnd

; ---------------------------------------------------------------- Install

Section "${APP} (required)" SecMain
  SectionIn RO
  SetRegView 64
  SetShellVarContext all
  SetOutPath "$INSTDIR"
  Call EnsureAppClosed

  File "/oname=${EXE_NAME}" "${EXE}"
  File "/oname=LICENSE.txt" "${LICENSE_TXT}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE_NAME}" "" "$INSTDIR\${EXE_NAME}" 0
  ; Re-created below only if the desktop shortcut is still selected.
  Delete "$DESKTOP\${APP}.lnk"

  ; Settings > Apps and Control Panel > Programs
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE_NAME},0"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegStr HKLM "${UNINST_KEY}" "URLInfoAbout" "${WEBSITE}"
  WriteRegStr HKLM "${UNINST_KEY}" "HelpLink" "${WEBSITE}"
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
  ${GetTime} "" "L" $0 $1 $2 $3 $4 $5 $6
  WriteRegStr HKLM "${UNINST_KEY}" "InstallDate" "$2$1$0"
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINST_KEY}" "EstimatedSize" $0

  ; Lets "SysCleanX" be started from the Run dialog (Win+R).
  WriteRegStr HKLM "${APPPATHS_KEY}" "" "$INSTDIR\${EXE_NAME}"
  WriteRegStr HKLM "${APPPATHS_KEY}" "Path" "$INSTDIR"
SectionEnd

Section "Desktop shortcut" SecDesktop
  SetShellVarContext all
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${EXE_NAME}" "" "$INSTDIR\${EXE_NAME}" 0
SectionEnd

; ---------------------------------------------------------------- Uninstall

Section "Uninstall"
  SetRegView 64
  SetShellVarContext all
  Call un.EnsureAppClosed

  Delete "$INSTDIR\${EXE_NAME}"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  ; Only removed if empty: never recursively delete a folder the user picked.
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  DeleteRegKey HKLM "${UNINST_KEY}"
  DeleteRegKey HKLM "${APPPATHS_KEY}"

  ; Per-user data (settings, cleaning logs, registry backups) is kept unless asked.
  SetShellVarContext current
  ${If} ${FileExists} "$APPDATA\${APP}\*.*"
    MessageBox MB_YESNO|MB_ICONQUESTION "Also delete your ${APP} settings, cleaning logs and registry backups?$\r$\n$\r$\n$APPDATA\${APP}" /SD IDNO IDNO keep_data
    RMDir /r "$APPDATA\${APP}"
    keep_data:
  ${EndIf}
SectionEnd
