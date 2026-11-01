$repoRoot = Split-Path -Parent $PSScriptRoot

$qtRoot = if ($env:IMAGEEDIT_QT_ROOT) { $env:IMAGEEDIT_QT_ROOT } else { 'D:\Qt\6.8.3\msvc2022_64' }
if (-not (Test-Path -LiteralPath (Join-Path $qtRoot 'bin\qmake.exe'))) {
    throw "Qt not found at '$qtRoot'. Set IMAGEEDIT_QT_ROOT to override."
}

# The machine-wide CC/CXX point at LLVM clang, which cannot compile against MSVC
# headers with the C++11 standard cxx requests. Force MSVC detection instead.
# Remove through the .NET API: Remove-Item is intercepted by the local
# safe-delete hook, which fails closed when the variable does not exist.
[Environment]::SetEnvironmentVariable('CC', $null, 'Process')
[Environment]::SetEnvironmentVariable('CXX', $null, 'Process')
[Environment]::SetEnvironmentVariable('CC_x86_64_pc_windows_msvc', $null, 'Process')
[Environment]::SetEnvironmentVariable('CXX_x86_64_pc_windows_msvc', $null, 'Process')

$env:QMAKE = Join-Path $qtRoot 'bin\qmake.exe'
$env:QT_VERSION_MAJOR = '6'
$env:IMAGEEDIT_QT_ROOT = $qtRoot
$env:IMAGEEDIT_QT_BIN = Join-Path $qtRoot 'bin'

# qmlcachegen emits UTF-8 Chinese string literals into .cpp; without /utf-8
# MSVC reads them under the system code page (GBK) and breaks string literals.
$env:CC = $null
$env:CXX = $null
$env:CFLAGS = '/utf-8'
$env:CXXFLAGS = '/utf-8'

$tools = Join-Path $repoRoot '.tools'
$paths = @(
    (Join-Path $qtRoot 'bin'),
    (Join-Path $tools 'cmake-3.30.5-windows-x86_64\bin'),
    (Join-Path $tools 'ninja'),
    (Join-Path $env:USERPROFILE '.cargo\bin')
)
$env:Path = (($paths + $env:Path) -join ';')

# qmlcachegen's QSaveFile commit (atomic replace) gets "Access denied" from
# security minifilters (e.g. 360) when OVERWRITING an existing
# qmlcache_loader.cpp; creating it fresh always works. Same for
# qmltyperegistrar's qmldir / plugin.qmltypes / .qrc outputs. Delete the
# cached outputs before every build so the build script never needs to
# overwrite. (.NET delete on purpose: Remove-Item goes through the local
# safe-delete hook, which fails closed when the path is absent.)
foreach ($cfg in @('debug', 'release')) {
    $buildRoot = Join-Path $repoRoot "target\$cfg\build"
    if (-not (Test-Path -LiteralPath $buildRoot)) { continue }
    Get-ChildItem -LiteralPath $buildRoot -Directory -Filter 'imageedit-ui-*' -ErrorAction SilentlyContinue |
        ForEach-Object {
            foreach ($sub in @('qmlcachegen', 'qt-build-utils')) {
                $p = Join-Path $_.FullName "out\$sub"
                if (Test-Path -LiteralPath $p) {
                    [System.IO.Directory]::Delete($p, $true)
                }
            }
        }
}
