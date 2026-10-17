param(
    [switch]$SkipBuild,
    [switch]$NoPrune,
    [switch]$SkipVerify
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'env.ps1')

$repoRoot = Split-Path -Parent $PSScriptRoot
$distDir = Join-Path $repoRoot 'dist\imageedit'
$zipPath = Join-Path $repoRoot 'dist\imageedit-0.1-win64.zip'
$benchDir = Join-Path $repoRoot 'bench-data'
$sample = Join-Path $benchDir 'small-1600x1200.jpg'

if (-not $SkipBuild) {
    $oldEA = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $buildOut = & cargo build --release --workspace 2>&1 | Out-String
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = $oldEA
    if ($buildExit -ne 0) { throw "cargo build failed`n$buildOut" }
}

if (Test-Path -LiteralPath $distDir) { Remove-Item -LiteralPath $distDir -Recurse -Force }
New-Item -ItemType Directory -Path $distDir -Force | Out-Null

Copy-Item (Join-Path $repoRoot 'target\release\imageedit.exe') $distDir
Copy-Item (Join-Path $repoRoot 'target\release\imageedit-bench.exe') $distDir

$oldEA = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$depOut = & (Join-Path $env:IMAGEEDIT_QT_BIN 'windeployqt.exe') --release --no-translations `
    --no-system-d3d-compiler --compiler-runtime `
    --qmldir (Join-Path $repoRoot 'crates\ui\qml') (Join-Path $distDir 'imageedit.exe') 2>&1 |
    Out-String
$depExit = $LASTEXITCODE
$ErrorActionPreference = $oldEA
if ($depExit -ne 0) { throw "windeployqt failed`n$depOut" }

# QML modules and DLLs that this app never imports. QtQuick.Dialogs.quickimpl
# only needs QtQuick + QtQuick.Templates + QtQuick.Layouts, so the whole
# QtQuick.Controls stack (and its style DLLs) can go.
$pruneDirs = @(
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

if (-not $NoPrune) {
    foreach ($rel in $pruneDirs) {
        $target = Join-Path $distDir $rel
        if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Recurse -Force }
    }
    foreach ($name in $pruneFiles) {
        $target = Join-Path $distDir $name
        if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Force }
    }
    foreach ($name in $keepFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $distDir $name))) {
            throw "pruning removed a required library: $name"
        }
    }
}

if (-not $SkipVerify) {
    $env:QT_ASSUME_STDERR_HAS_CONSOLE = '1'
    $savedPath = $env:Path
    $savedEA = $ErrorActionPreference
    $env:Path = 'C:\Windows\System32;C:\Windows'
    $ErrorActionPreference = 'Continue'
    try {
        if (Test-Path -LiteralPath $sample) {
            $log = & (Join-Path $distDir 'imageedit.exe') $sample '--script=crop:100,100,800,600;extract:100,100,300,200;resize:400,300;undo;redo' '--selftest' *>&1 | Out-String
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

Compress-Archive -Path (Join-Path $distDir '*') -DestinationPath $zipPath -Force
Write-Host ("zip: {0:N1} MB" -f ((Get-Item -LiteralPath $zipPath).Length / 1MB))
