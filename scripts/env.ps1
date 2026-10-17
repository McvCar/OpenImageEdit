$repoRoot = Split-Path -Parent $PSScriptRoot

$qtRoot = if ($env:IMAGEEDIT_QT_ROOT) { $env:IMAGEEDIT_QT_ROOT } else { 'D:\Qt\6.8.3\msvc2022_64' }
if (-not (Test-Path -LiteralPath (Join-Path $qtRoot 'bin\qmake.exe'))) {
    throw "Qt not found at '$qtRoot'. Set IMAGEEDIT_QT_ROOT to override."
}

# The machine-wide CC/CXX point at LLVM clang, which cannot compile against MSVC
# headers with the C++11 standard cxx requests. Force MSVC detection instead.
Remove-Item Env:CC -ErrorAction SilentlyContinue
Remove-Item Env:CXX -ErrorAction SilentlyContinue
Remove-Item Env:CC_x86_64_pc_windows_msvc -ErrorAction SilentlyContinue
Remove-Item Env:CXX_x86_64_pc_windows_msvc -ErrorAction SilentlyContinue

$env:QMAKE = Join-Path $qtRoot 'bin\qmake.exe'
$env:QT_VERSION_MAJOR = '6'
$env:IMAGEEDIT_QT_ROOT = $qtRoot
$env:IMAGEEDIT_QT_BIN = Join-Path $qtRoot 'bin'

$tools = Join-Path $repoRoot '.tools'
$paths = @(
    (Join-Path $qtRoot 'bin'),
    (Join-Path $tools 'cmake-3.30.5-windows-x86_64\bin'),
    (Join-Path $tools 'ninja'),
    (Join-Path $env:USERPROFILE '.cargo\bin')
)
$env:Path = (($paths + $env:Path) -join ';')
