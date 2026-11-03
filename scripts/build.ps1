# Single entry point for every build. Do NOT run `cargo build` directly.
#
# Two failure modes look exactly like "the build is stuck" and both are caused
# by a second, differently-configured cargo:
#
# 1) The machine-wide CC/CXX point at LLVM clang. cxx/cxx-qt must compile
#    against the MSVC headers, so clang fails (and leaves a polluted cache).
#    env.ps1 clears CC/CXX so cc-rs locates cl.exe via vswhere. That only works
#    in PowerShell: in Git Bash, CFLAGS=/utf-8 is rewritten by MSYS into
#    C:/.../PortableGit/utf-8 and cl.exe then treats it as a source file.
#
# 2) cxx-qt-lib's build script emits rerun-if-env-changed for CC/CXX/CFLAGS/
#    CXXFLAGS. Any build with a different environment re-runs it, which means
#    all 264 Qt C++ sources get recompiled (several minutes on one core, which
#    reads as "CPU is idle, nothing is happening").
#
# 3) Two concurrent cargo processes contend on the target-directory lock and
#    both sit at 0% CPU. The lock below makes the second one fail fast instead
#    of hanging silently.
#
# Usage:
#   pwsh -File scripts/build.ps1              # debug build (for manual testing)
#   pwsh -File scripts/build.ps1 -- test      # cargo test
#   pwsh -File scripts/build.ps1 -- build --release

param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CargoArgs
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $CargoArgs -or $CargoArgs.Count -eq 0) { $CargoArgs = @('build') }

$lockPath = Join-Path $repo 'target\.build.lock'
$null = New-Item -ItemType Directory -Force -Path (Join-Path $repo 'target')

try {
    $lock = [System.IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None')
} catch {
    $others = Get-Process cargo, rustc, cl, link -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty ProcessName -Unique
    throw "Another build is already running ($lockPath is locked; processes: $($others -join ', ')). Wait for it instead of starting a second cargo."
}

try {
    . (Join-Path $PSScriptRoot 'env.ps1')
    $sw = [Diagnostics.Stopwatch]::StartNew()
    Write-Host "==> cargo $($CargoArgs -join ' ')" -ForegroundColor Cyan
    # cargo writes warnings to stderr. Under $ErrorActionPreference = 'Stop'
    # PowerShell 5.1 turns the first of those into a *terminating*
    # NativeCommandError, so the command appears to fail while cargo is in fact
    # still compiling. Relax the preference for the child only and judge the
    # result by the exit code.
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & cargo @CargoArgs
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    $sw.Stop()
    $msg = "==> cargo exit {0}, {1:n1}s" -f $code, $sw.Elapsed.TotalSeconds
    $color = if ($code -eq 0) { 'Green' } else { 'Red' }
    Write-Host $msg -ForegroundColor $color
    exit $code
} finally {
    $lock.Dispose()
}
