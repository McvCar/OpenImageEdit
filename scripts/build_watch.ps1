# Guarded build runner for this machine. Dot-source from package.ps1.
#
# Two known failure modes on this box:
#   * cl.exe sometimes sits at 0% CPU forever during a release build;
#   * `& cargo ... | Out-String` buffers ALL output until the process exits,
#     so a stalled build looks identical to a healthy quiet one (no output
#     for 10+ minutes and nobody can tell whether the compiler is alive).
#
# This helper starts the compiler through a tiny .cmd wrapper that redirects
# stdout/stderr to files, and emits a heartbeat every IntervalSec (60s by
# default): alive flag, accumulated
# CPU of the build process plus rustc/cl/link/lld children, log bytes, helper
# PID list, the last stderr line, and the idle counter. A build is considered
# stalled only when NONE of those signals changed for StallMinutes consecutive
# heartbeats; then it kills the whole process tree and throws instead of
# hanging silently.
#
# ASCII-only on purpose: powershell.exe 5.1 reads .ps1 as ANSI (GBK here), so
# UTF-8 Chinese comments corrupt the parser.

function Get-LogText([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return '' }
    try {
        $text = Get-Content -LiteralPath $Path -Raw -ErrorAction Stop
        if ($null -eq $text) { return '' }
        return [string]$text
    } catch {
        return ''
    }
}

function Get-BuildTail([string]$OutLog, [string]$ErrLog, [int]$Lines) {
    $all = (Get-LogText -Path $OutLog) + (Get-LogText -Path $ErrLog)
    return (($all -split "`n" | Select-Object -Last $Lines) -join "`n")
}

function Invoke-GuardedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$Arguments = @(),
        [int]$IntervalSec = 60,
        [int]$StallMinutes = 5
    )
    if ($IntervalSec -lt 1) { $IntervalSec = 1 }
    if ($StallMinutes -lt 1) { $StallMinutes = 1 }

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $safe = $Label -replace '[^A-Za-z0-9]', '_'
    $base = Join-Path $env:TEMP ("imageedit-build-{0}-{1}" -f $safe, $stamp)
    $outLog = $base + '.out.log'
    $errLog = $base + '.err.log'

    Write-Host ("==> build [{0}]: {1} {2}" -f $Label, $FilePath, ($Arguments -join ' ')) -ForegroundColor Cyan
    Write-Host ("    logs: {0}.out.log / .err.log" -f $base) -ForegroundColor DarkGray

    # Do NOT use Start-Process here: PS 5.1 builds a case-insensitive
    # environment dictionary and this box has BOTH HTTPS_PROXY and
    # https_proxy, so it throws "An item with the same key has already been
    # added" before the compiler even starts. The .NET Process API inherits
    # the environment block directly and has no such dictionary.
    #
    # A .cmd wrapper does the file redirection itself, so we do not need
    # managed async stream pumping (which would otherwise deadlock when the
    # pipe fills). taskkill /T on the cmd PID still kills cargo + rustc + cl.
    $quotedArgs = ($Arguments | ForEach-Object {
            $a = [string]$_
            if ($a -match '\s') { '"' + $a + '"' } else { $a }
        }) -join ' '
    $wrapper = $base + '.cmd'
    $lines = @(
        '@echo off',
        ('"{0}" {1} 1> "{2}" 2> "{3}"' -f $FilePath, $quotedArgs, $outLog, $errLog),
        'exit /b %ERRORLEVEL%'
    )
    [System.IO.File]::WriteAllLines($wrapper, $lines, [System.Text.Encoding]::ASCII)

    $cmdExe = if ($env:ComSpec) { $env:ComSpec } else { Join-Path $env:WINDIR 'System32\cmd.exe' }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $cmdExe
    $psi.Arguments = '/d /c "' + $wrapper + '"'
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    if (-not $proc.Start()) { throw "failed to start build process for $Label" }

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $lastSig = ''
    $idle = 0
    $done = $false

    while (-not $done) {
        # 1-second slices: notice an exit immediately, heartbeat on the minute.
        for ($i = 0; $i -lt $IntervalSec; $i++) {
            Start-Sleep -Seconds 1
            if ($proc.HasExited) { $done = $true; break }
        }
        if ($done) { break }

        $proc.Refresh()
        $cpu = 0.0
        try { $cpu = $proc.TotalProcessorTime.TotalSeconds } catch { }
        # Real compilation happens in children. A quiet cargo with a busy
        # cl.exe is healthy, so sample the whole MSVC/Rust toolchain.
        $helpers = @(Get-Process -Name 'rustc', 'cl', 'link', 'lld', 'ld', 'cargo' -ErrorAction SilentlyContinue)
        foreach ($h in $helpers) {
            if ($h.Id -eq $proc.Id) { continue }
            try { $cpu += $h.TotalProcessorTime.TotalSeconds } catch { }
        }
        $cpu = [math]::Round($cpu, 1)

        $bytes = 0
        foreach ($f in @($outLog, $errLog)) {
            if (Test-Path -LiteralPath $f) {
                try { $bytes += (Get-Item -LiteralPath $f).Length } catch { }
            }
        }
        $tail = ''
        try {
            $line = @(Get-Content -LiteralPath $errLog -Tail 1 -ErrorAction Stop)
            if ($line.Count -gt 0) { $tail = [string]$line[0] }
        } catch { }
        if ($tail.Length -gt 140) { $tail = $tail.Substring(0, 140) }

        Write-Host ("    [{0}] {1} alive=yes cpu={2}s log={3}B helpers={4} idle={5}/{6} | {7}" -f `
                $Label, $sw.Elapsed.ToString('mm\:ss'), $cpu, $bytes, $helpers.Count, `
                $idle, $StallMinutes, $tail)

        # PID list matters: rustc processes come and go as crates finish even
        # when rounded CPU time and log size happen to look unchanged.
        $sig = '{0}|{1}|{2}' -f $cpu, $bytes, ($helpers.Id -join ',')
        if ($sig -eq $lastSig) { $idle++ } else { $idle = 0 }
        $lastSig = $sig

        if ($idle -ge $StallMinutes) {
            Write-Warning ("[{0}] STALLED: {1} heartbeats with no CPU, no helper change and no log growth. Killing process tree." -f $Label, $idle)
            Write-Host (Get-BuildTail -OutLog $outLog -ErrLog $errLog -Lines 20)
            try { & taskkill /PID $proc.Id /T /F 2>&1 | Out-Null } catch { }
            throw ("[{0}] build stalled after {1}. Logs at {2}.*.log ; re-run .\scripts\package.ps1 -NoBump (or without -NoBump for a fresh version)." -f `
                    $Label, $sw.Elapsed.ToString('mm\:ss'), $base)
        }
    }

    $proc.WaitForExit()
    $code = $proc.ExitCode
    $tailText = Get-BuildTail -OutLog $outLog -ErrLog $errLog -Lines 25
    Write-Host ("    [{0}] finished in {1}, exit={2}" -f $Label, $sw.Elapsed.ToString('mm\:ss'), $code)
    if ($code -ne 0) { Write-Host $tailText }
    return [pscustomobject]@{
        ExitCode = $code
        Output   = (Get-LogText -Path $outLog) + (Get-LogText -Path $errLog)
        Tail     = $tailText
    }
}

function Invoke-GuardedCargo {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string[]]$CargoArgs,
        [int]$IntervalSec = 60,
        [int]$StallMinutes = 5
    )
    $cargo = (Get-Command cargo -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
    return Invoke-GuardedProcess -Label $Label -FilePath $cargo -Arguments $CargoArgs `
        -IntervalSec $IntervalSec -StallMinutes $StallMinutes
}
