; ImageEdit per-user installer (NSIS 3).
; Component page lets the user tick which image formats to associate with
; ImageEdit. File associations are written under HKCU\Software\Classes so no
; admin rights are needed.

Unicode True
!include "MUI2.nsh"
!include "FileFunc.nsh"

!define APP_NAME "ImageEdit"
!define APP_VERSION "0.1.0"
!define PROG_ID "ImageEdit.Document"
!define REG_CLASSES "Software\Classes"

!define MUI_ABORTWARNING

; Welcome / finish pages use the app icon on the dialogs.
!define MUI_ICON "..\crates\ui\assets\app.ico"
!define MUI_UNICON "..\crates\ui\assets\app.ico"

Name "${APP_NAME}"
OutFile "..\dist\imageedit-${APP_VERSION}-win64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\ImageEdit"
; Per-user install: no UAC prompt, no HKLM writes.
RequestExecutionLevel user
InstallDirRegKey HKCU "Software\ImageEdit" "InstallDir"

; Installer exe icon comes from the manifest resource compiled below.
VIProductVersion "0.1.0.0"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "FileDescription" "${APP_NAME} 安装程序"
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"

!define MUI_WELCOMEPAGE_TITLE "欢迎使用 ${APP_NAME} 安装向导"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\imageedit.exe"
!define MUI_FINISHPAGE_RUN_TEXT "立即运行 ${APP_NAME}"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"

; ---------------------------------------------------------------------------
; Sections. The main section is mandatory; format associations are optional
; and appear as checkboxes on the component page.
; ---------------------------------------------------------------------------

Section "${APP_NAME}（必需）" SecMain
    SectionIn RO
    SetOutPath "$INSTDIR"
    File /r "..\dist\imageedit\*.*"
    WriteUninstaller "$INSTDIR\uninstall.exe"

    CreateDirectory "$SMPROGRAMS\ImageEdit"
    CreateShortcut "$SMPROGRAMS\ImageEdit\${APP_NAME}.lnk" "$INSTDIR\imageedit.exe"
    CreateShortcut "$SMPROGRAMS\ImageEdit\卸载 ${APP_NAME}.lnk" "$INSTDIR\uninstall.exe"

    ; Remember the install dir so reinstall/upgrade reuses it.
    WriteRegStr HKCU "Software\ImageEdit" "InstallDir" "$INSTDIR"

    ; Uninstall metadata for "应用和功能".
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "DisplayName" "${APP_NAME}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "DisplayVersion" "${APP_VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "DisplayIcon" "$INSTDIR\imageedit.exe,0"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "Publisher" "${APP_NAME}"
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "NoModify" 1
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "NoRepair" 1
SectionEnd

Section "桌面快捷方式" SecDesktop
    ; Silent installs (CI smoke tests) never touch the user's desktop.
    IfSilent skip_desktop
    CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\imageedit.exe"
    skip_desktop:
SectionEnd

; --- File-format associations --------------------------------------------
; Each section registers the ProgId and (when ticked) points the extension's
; user-level default at it. HKCU\Software\Classes\.ext shadows HKLM, so this
; works without admin rights. Silent installs skip associations so CI smoke
; tests never rewrite the user's real file associations.

Section "关联 PNG 图片 (.png)" SecPng
    IfSilent skip_png
    Call RegisterProgId
    WriteRegStr HKCU "${REG_CLASSES}\.png" "" "${PROG_ID}"
    Call NotifyShell
    skip_png:
SectionEnd

Section "关联 JPG 图片 (.jpg .jpeg)" SecJpg
    IfSilent skip_jpg
    Call RegisterProgId
    WriteRegStr HKCU "${REG_CLASSES}\.jpg" "" "${PROG_ID}"
    WriteRegStr HKCU "${REG_CLASSES}\.jpeg" "" "${PROG_ID}"
    Call NotifyShell
    skip_jpg:
SectionEnd

Section "关联 BMP 图片 (.bmp)" SecBmp
    IfSilent skip_bmp
    Call RegisterProgId
    WriteRegStr HKCU "${REG_CLASSES}\.bmp" "" "${PROG_ID}"
    Call NotifyShell
    skip_bmp:
SectionEnd

Section "关联 GIF 图片 (.gif)" SecGif
    IfSilent skip_gif
    Call RegisterProgId
    WriteRegStr HKCU "${REG_CLASSES}\.gif" "" "${PROG_ID}"
    Call NotifyShell
    skip_gif:
SectionEnd

Section "关联 TIFF 图片 (.tif .tiff)" SecTiff
    IfSilent skip_tiff
    Call RegisterProgId
    WriteRegStr HKCU "${REG_CLASSES}\.tif" "" "${PROG_ID}"
    WriteRegStr HKCU "${REG_CLASSES}\.tiff" "" "${PROG_ID}"
    Call NotifyShell
    skip_tiff:
SectionEnd

Section "关联 WebP 图片 (.webp)" SecWebp
    IfSilent skip_webp
    Call RegisterProgId
    WriteRegStr HKCU "${REG_CLASSES}\.webp" "" "${PROG_ID}"
    Call NotifyShell
    skip_webp:
SectionEnd

; --- Helpers --------------------------------------------------------------

; Register (or refresh) the ImageEdit.Document ProgId.
Function RegisterProgId
    WriteRegStr HKCU "${REG_CLASSES}\${PROG_ID}" "" "ImageEdit 图片文档"
    WriteRegStr HKCU "${REG_CLASSES}\${PROG_ID}\DefaultIcon" "" "$INSTDIR\imageedit.exe,0"
    WriteRegStr HKCU "${REG_CLASSES}\${PROG_ID}\shell\open\command" "" '"$INSTDIR\imageedit.exe" "%1"'
FunctionEnd

; Tell Explorer that file associations changed.
Function NotifyShell
    System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
FunctionEnd

; --- Descriptions shown under the component list ---------------------------

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
    !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "安装 ${APP_NAME} 主程序及其运行库（必需）。"
    !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "在桌面创建 ${APP_NAME} 快捷方式。"
    !insertmacro MUI_DESCRIPTION_TEXT ${SecPng} "双击 .png 文件时使用 ${APP_NAME} 打开。"
    !insertmacro MUI_DESCRIPTION_TEXT ${SecJpg} "双击 .jpg / .jpeg 文件时使用 ${APP_NAME} 打开。"
    !insertmacro MUI_DESCRIPTION_TEXT ${SecBmp} "双击 .bmp 文件时使用 ${APP_NAME} 打开。"
    !insertmacro MUI_DESCRIPTION_TEXT ${SecGif} "双击 .gif 文件时使用 ${APP_NAME} 打开。"
    !insertmacro MUI_DESCRIPTION_TEXT ${SecTiff} "双击 .tif / .tiff 文件时使用 ${APP_NAME} 打开。"
    !insertmacro MUI_DESCRIPTION_TEXT ${SecWebp} "双击 .webp 文件时使用 ${APP_NAME} 打开。"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; --- Uninstaller -----------------------------------------------------------

Function un.DissociateExtension
    ; Pop: extension (e.g. ".png"). Clears the association if it still
    ; points at our ProgId, and removes us from the OpenWith list.
    Exch $R0
    ReadRegStr $R1 HKCU "${REG_CLASSES}\$R0" ""
    StrCmp $R1 "${PROG_ID}" 0 +2
    DeleteRegValue HKCU "${REG_CLASSES}\$R0" ""
    DeleteRegValue HKCU "${REG_CLASSES}\$R0\OpenWithProgids" "${PROG_ID}"
    Pop $R0
FunctionEnd

Section "Uninstall"
    ; Remove shortcuts.
    Delete "$DESKTOP\${APP_NAME}.lnk"
    Delete "$SMPROGRAMS\ImageEdit\${APP_NAME}.lnk"
    Delete "$SMPROGRAMS\ImageEdit\卸载 ${APP_NAME}.lnk"
    RMDir "$SMPROGRAMS\ImageEdit"

    ; Drop associations that still point at us.
    Push ".png"
    Call un.DissociateExtension
    Push ".jpg"
    Call un.DissociateExtension
    Push ".jpeg"
    Call un.DissociateExtension
    Push ".bmp"
    Call un.DissociateExtension
    Push ".gif"
    Call un.DissociateExtension
    Push ".tif"
    Call un.DissociateExtension
    Push ".tiff"
    Call un.DissociateExtension
    Push ".webp"
    Call un.DissociateExtension
    DeleteRegKey /ifempty HKCU "${REG_CLASSES}\.png\OpenWithProgids"
    DeleteRegKey /ifempty HKCU "${REG_CLASSES}\.jpg\OpenWithProgids"
    DeleteRegKey /ifempty HKCU "${REG_CLASSES}\.jpeg\OpenWithProgids"
    DeleteRegKey /ifempty HKCU "${REG_CLASSES}\.bmp\OpenWithProgids"
    DeleteRegKey /ifempty HKCU "${REG_CLASSES}\.gif\OpenWithProgids"
    DeleteRegKey /ifempty HKCU "${REG_CLASSES}\.tif\OpenWithProgids"
    DeleteRegKey /ifempty HKCU "${REG_CLASSES}\.tiff\OpenWithProgids"
    DeleteRegKey /ifempty HKCU "${REG_CLASSES}\.webp\OpenWithProgids"
    DeleteRegKey HKCU "${REG_CLASSES}\${PROG_ID}"
    System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'

    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"
    DeleteRegValue HKCU "Software\ImageEdit" "InstallDir"
    DeleteRegKey /ifempty HKCU "Software\ImageEdit"

    ; Remove the installed files.
    RMDir /r "$INSTDIR"
SectionEnd
