@echo off
rem ============================================================
rem One-click launcher for the DEBUG build of ImageEdit.
rem
rem Usage (double-click or from a terminal):
rem   scripts\run_debug.cmd                 -> open an empty debug window
rem   scripts\run_debug.cmd <file.psd>      -> open that file (drag & drop works)
rem   scripts\run_debug.cmd build [file]    -> rebuild first, then open
rem
rem The Qt DLL path is added to PATH so the exe can start without the
rem installer. Everything here is ASCII on purpose: cmd.exe reads this
rem file with the active code page and would mangle non-ASCII text.
rem ============================================================
setlocal EnableExtensions
set "ROOT=%~dp0.."
set "EXE=%ROOT%\target\debug\imageedit.exe"
set "PATH=D:\Qt\6.8.3\msvc2022_64\bin;%PATH%"

rem --- optional rebuild ----------------------------------------
if /i "%~1"=="build" (
    shift
    echo [run_debug] rebuilding debug...
    pushd "%ROOT%"
    powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\build.ps1" build
    set "BUILDERR=%ERRORLEVEL%"
    popd
    if not "%BUILDERR%"=="0" (
        echo [run_debug] build failed, aborting.
        pause
        exit /b 1
    )
)

if not exist "%EXE%" (
    echo [run_debug] debug exe not found:
    echo   %EXE%
    echo [run_debug] build it first:  scripts\run_debug.cmd build
    pause
    exit /b 1
)

rem --- launch ---------------------------------------------------
rem The --debug flag is deliberately passed to EVERY launch. Any argument
rem starting with "--" opts the process out of the single-instance handoff
rem (see main.rs), so the debug window always opens on its own instead of
rem being swallowed by a running installed copy. No probe/selftest reacts
rem to "--debug": it only flips the automation branch (sync decode + no
rem handoff), both harmless for a manual session.
if "%~1"=="" (
    echo [run_debug] starting debug build...
    start "" "%EXE%" --debug
) else (
    echo [run_debug] starting debug build with: %~1
    start "" "%EXE%" --debug "%~1"
)
exit /b 0
