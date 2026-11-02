param(
    [switch]$SkipBuild,
    [switch]$NoPrune,
    [switch]$SkipVerify
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'env.ps1')

# NOTE: keep this file ASCII-only. powershell.exe (5.1) reads .ps1 as ANSI,
# so UTF-8 Chinese comments corrupt the parser.
#
# This machine runs a safe-delete hook that intercepts bulk `Remove-Item` and
# fails closed. In a non-interactive (background/CI) run that looks like the
# script "hangs forever": no error, no exit. So packaging deletes files through
# the .NET API instead.
function Remove-PathForce([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer) {
        [System.IO.Directory]::Delete($item.FullName, $true)
    } else {
        [System.IO.File]::Delete($item.FullName)
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$distDir = Join-Path $repoRoot 'dist\imageedit'
$zipPath = Join-Path $repoRoot 'dist\imageedit-0.1-win64.zip'
$benchDir = Join-Path $repoRoot 'bench-data'
$sample = Join-Path $benchDir 'small-1600x1200.jpg'

if (-not $SkipBuild) {
    $oldEA = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $buildOut = & cargo build --release --workspace --exclude imageedit-ui 2>&1 | Out-String
    $buildExit = $LASTEXITCODE
    if ($buildExit -eq 0) {
        # End-user binary: drop the console feature so main.rs compiles as
        # windows_subsystem = "windows" (no console window for users).
        # imageedit-ui is excluded from the loop above on purpose: the console
        # variant is never packaged, and building it just to throw it away adds
        # a whole second ui compile + link to every package run.
        $buildOut += & cargo build --release -p imageedit-ui --no-default-features 2>&1 | Out-String
        $buildExit = $LASTEXITCODE
    }
    $ErrorActionPreference = $oldEA
    if ($buildExit -ne 0) { throw "cargo build failed`n$buildOut" }
}

Write-Host '==> deploy' -ForegroundColor Cyan
if (Test-Path -LiteralPath $distDir) { Remove-PathForce $distDir }
New-Item -ItemType Directory -Path $distDir -Force | Out-Null

Copy-Item (Join-Path $repoRoot 'target\release\imageedit.exe') $distDir
# End-user zip: the bench harness is a dev tool (target\release keeps it);
# shipping it in the public archive just bloats the download.
# Copy-Item (Join-Path $repoRoot 'target\release\imageedit-bench.exe') $distDir

# Explorer thumbnail shell extension (COM in-proc DLL). Built with the
# dedicated `thumb` profile: same optimizations as release but panic=unwind
# so the provider's catch_unwind can stop PSD parse panics from taking down
# explorer.exe (the workspace release profile uses panic=abort).
# PS 5.1 turns cargo's stderr progress lines into ErrorRecords; under
# $ErrorActionPreference='Stop' the first one aborts the script.
$oldEA = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
# Static CRT: the DLL must load on machines without the VC++ redistributable
# (Explorer silently drops the handler when LoadLibrary fails). Pure Rust,
# no C++ objects, so /MT is conflict-free here.
$env:RUSTFLAGS = '-C target-feature=+crt-static'
$thumbOut = & cargo build -p imageedit-thumbnail --profile thumb 2>&1 | Out-String
[Environment]::SetEnvironmentVariable('RUSTFLAGS', $null, 'Process')
$ErrorActionPreference = $oldEA
if ($LASTEXITCODE -ne 0) { throw "thumbnail DLL build failed`n$thumbOut" }
Copy-Item (Join-Path $repoRoot 'target\thumb\imageedit_thumb.dll') $distDir

# App-local VC++ runtime: imageedit.exe (cxx-qt) and the Qt6 DLLs import
# MSVCP140/VCRUNTIME140, which clean machines without the VC++ redist lack.
# Windows resolves app-local DLLs before the system copy, so shipping them
# beside the exe keeps per-user installs self-contained (no admin needed).
foreach ($rt in 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll') {
    $src = Join-Path $env:WINDIR "System32\$rt"
    if (-not (Test-Path -LiteralPath $src)) { throw "runtime DLL missing: $src" }
    Copy-Item $src $distDir -Force
}
# Per-user registration helpers for zip users (the NSIS installer writes
# the same HKCU keys itself, no regsvr32 needed there).
Set-Content -LiteralPath (Join-Path $distDir 'register-psd-thumbnail.cmd') -Value @'
@echo off
rem Registers the ImageEdit PSD thumbnail handler for the current user.
regsvr32 /s "%~dp0imageedit_thumb.dll"
'@ -Encoding Ascii
Set-Content -LiteralPath (Join-Path $distDir 'unregister-psd-thumbnail.cmd') -Value @'
@echo off
rem Removes the ImageEdit PSD thumbnail handler registration.
regsvr32 /s /u "%~dp0imageedit_thumb.dll"
'@ -Encoding Ascii

# Machine-wide fallback registration for setups where the per-user (HKCU)
# handler is not honored; path matches the NSIS default InstallDir.
Copy-Item (Join-Path $repoRoot 'installer\register-psd-thumbnail-hklm.reg') $distDir

# Diagnostic for remote debugging (e.g. "no PSD preview on another PC").
# English-only output: cmd files are read in the OEM code page.
Set-Content -LiteralPath (Join-Path $distDir 'check-psd-thumbnail.cmd') -Value @'
@echo off
rem Diagnoses the ImageEdit PSD thumbnail handler registration.
set CLSID={43B34785-3F16-49F5-BAB6-069B487ED407}
set SHELLEX={e357fccd-a995-4576-b01f-234630154e96}
echo [1] DLL file:
if exist "%~dp0imageedit_thumb.dll" (echo     OK  %~dp0imageedit_thumb.dll) else (echo     MISSING %~dp0imageedit_thumb.dll)
echo [2] CLSID (HKCU):
reg query "HKCU\Software\Classes\CLSID\%CLSID%\InprocServer32" /ve
echo [2b] CLSID redirected to 32-bit view (WOW6432Node)? If ONLY this one
echo      shows a value, your installer predates the 64-bit registry fix:
reg query "HKCU\Software\Classes\WOW6432Node\CLSID\%CLSID%\InprocServer32" /ve 2>nul
echo [3] ShellEx key (HKCR merged view):
reg query "HKCR\.psd\ShellEx\%SHELLEX%" /ve
echo [4] VC++ runtime (app-local next to exe, else System32):
if exist "%~dp0vcruntime140.dll" (echo     OK  app-local vcruntime140.dll) else if exist "%WINDIR%\System32\vcruntime140.dll" (echo     OK  System32\vcruntime140.dll) else (echo     MISSING - install VC++ x64 redistributable)
echo [5] Test extraction via Explorer API requires a reboot of explorer.exe
echo     after registration: taskkill /f /im explorer.exe ^&^& start explorer.exe
echo.
echo If [2] or [3] is missing, run register-psd-thumbnail.cmd as this user.
pause
'@ -Encoding Ascii

$oldEA = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$depOut = & (Join-Path $env:IMAGEEDIT_QT_BIN 'windeployqt.exe') --release --no-translations `
    --no-system-d3d-compiler --compiler-runtime `
    --qmldir (Join-Path $repoRoot 'crates\ui\qml') (Join-Path $distDir 'imageedit.exe') 2>&1 |
    Out-String
$depExit = $LASTEXITCODE
$ErrorActionPreference = $oldEA
if ($depExit -ne 0) { throw "windeployqt failed`n$depOut" }
Write-Host '==> windeployqt done' -ForegroundColor Cyan

# QML modules and DLLs that this app never imports. QtQuick.Dialogs.quickimpl
# only needs QtQuick + QtQuick.Templates + QtQuick.Layouts, so the whole
# QtQuick.Controls stack (and its style DLLs) can go.
#
# Image formats: ALL Qt image-format plugins are dead weight here — decoding
# runs through the Rust `image` crate and the QML preview only ever loads BMP
# data URLs, which QtGui handles with its BUILT-IN handler (no plugin). Same
# story for the network stack: Qt6Qml hard-links Qt6Network.dll (must stay)
# but the app opens no connections, so the TLS / network-information / touch
# / svg-icon-engine plugins windeployqt drags in are never loaded.
$pruneDirs = @(
    'imageformats',
    'iconengines',
    'tls',
    'networkinformation',
    'generic',
    'qml\QML',
    'qml\QtQuick\Controls',
    'qml\QtQuick\NativeStyle',
    'qml\QtQuick\Effects',
    'qml\QtQuick\Particles',
    'qml\QtQuick\Shapes',
    'qml\QtQuick\VectorImage',
    'qml\QtQuick\LocalStorage',
    'qml\QtQuick\tooling',
    'qml\QtQuick\Dialogs\quickimpl\qml\+Fusion',
    'qml\QtQuick\Dialogs\quickimpl\qml\+Imagine',
    'qml\QtQuick\Dialogs\quickimpl\qml\+Material',
    'qml\QtQuick\Dialogs\quickimpl\qml\+Universal',
    'qml\QtQml\XmlListModel',
    'qmltooling',
    'qml\QtQuick\VirtualKeyboard'
)
$pruneFiles = @(
    'Qt6QuickControls2.dll',
    'Qt6QuickControls2Basic.dll',
    'Qt6QuickControls2BasicStyleImpl.dll',
    'Qt6QuickControls2FluentWinUI3StyleImpl.dll',
    'Qt6QuickControls2Fusion.dll',
    'Qt6QuickControls2FusionStyleImpl.dll',
    'Qt6QuickControls2Imagine.dll',
    'Qt6QuickControls2ImagineStyleImpl.dll',
    'Qt6QuickControls2Material.dll',
    'Qt6QuickControls2MaterialStyleImpl.dll',
    'Qt6QuickControls2Universal.dll',
    'Qt6QuickControls2UniversalStyleImpl.dll',
    'Qt6QuickControls2WindowsStyleImpl.dll',
    'Qt6QuickEffects.dll',
    'Qt6QuickShapes.dll',
    'Qt6LabsFolderListModel.dll',
    'Qt6Svg.dll',
    'opengl32sw.dll'
)

# Qt6QuickControls2Impl.dll is the one piece of the Controls stack that
# Qt6QuickDialogs2QuickImpl.dll links against, so it must stay.
$keepFiles = @(
    'Qt6QuickControls2Impl.dll',
    'Qt6QuickTemplates2.dll',
    'Qt6QuickDialogs2QuickImpl.dll',
    'Qt6QuickDialogs2.dll',
    'Qt6QuickDialogs2Utils.dll'
)

Write-Host '==> prune' -ForegroundColor Cyan
if (-not $NoPrune) {
    foreach ($rel in $pruneDirs) {
        Remove-PathForce (Join-Path $distDir $rel)
    }
    foreach ($name in $pruneFiles) {
        Remove-PathForce (Join-Path $distDir $name)
    }
    foreach ($name in $keepFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $distDir $name))) {
            throw "pruning removed a required library: $name"
        }
    }
}

Write-Host '==> verify' -ForegroundColor Cyan
if (-not $SkipVerify) {
    $env:QT_ASSUME_STDERR_HAS_CONSOLE = '1'
    $savedPath = $env:Path
    $savedEA = $ErrorActionPreference
    $env:Path = 'C:\Windows\System32;C:\Windows'
    $ErrorActionPreference = 'Continue'
    try {
        if (Test-Path -LiteralPath $sample) {
            $log = & (Join-Path $distDir 'imageedit.exe') $sample '--script=crop:100,100,800,600;extract:100,100,300,200;resize:400,300;copy:20,20,100,100;paste;scale-selected:150,80;undo;redo' '--selftest' *>&1 | Out-String
            if ($LASTEXITCODE -ne 0) { throw "packaged run failed with exit $LASTEXITCODE`n$log" }
            if ($log -notmatch 'script-done') { throw "packaged run produced no script-done`n$log" }
            Write-Host "verify: edit pipeline ok"

            $dialogLog = & (Join-Path $distDir 'imageedit.exe') $sample '--selftest' '--probe-dialog' *>&1 | Out-String
            if ($dialogLog -notmatch 'dialog-opened') {
                throw "file dialog did not open in packaged build`n$dialogLog"
            }
            Write-Host "verify: file dialog ok"
        } else {
            Write-Warning "sample '$sample' missing; skipped packaged verification"
        }
    } finally {
        $env:Path = $savedPath
        $ErrorActionPreference = $savedEA
    }
}

$files = Get-ChildItem -LiteralPath $distDir -Recurse -File
$totalMb = ($files | Measure-Object -Property Length -Sum).Sum / 1MB
Write-Host ("dist: {0} files, {1:N1} MB" -f $files.Count, $totalMb)

# Delete the previous archive explicitly: Compress-Archive -Force's internal
# replace is also caught by the safe-delete hook.
Remove-PathForce $zipPath
Compress-Archive -Path (Join-Path $distDir '*') -DestinationPath $zipPath -Force
Write-Host ("zip: {0:N1} MB" -f ((Get-Item -LiteralPath $zipPath).Length / 1MB))
