@echo off
REM ============================================================
REM  ReverieAgent - one-click launcher
REM
REM  Usage:
REM    start.bat            build (debug) and launch
REM    start.bat release    build (release) and launch
REM
REM  Builds with the `custom-protocol` feature so the frontend is
REM  embedded in the executable. Without it Tauri treats every build
REM  as a dev build, loads http://127.0.0.1:1420 and fails with
REM  ERR_CONNECTION_REFUSED when no Vite server is running.
REM
REM  This file is intentionally ASCII-only: cmd.exe decodes batch
REM  files with the active OEM code page, and non-ASCII text breaks
REM  parsing on machines with a different code page.
REM ============================================================
setlocal enabledelayedexpansion
cd /d "%~dp0"
title ReverieAgent Launcher

set "PROFILE=debug"
set "CARGO_ARGS=--features custom-protocol"
if /i "%~1"=="release" (
    set "PROFILE=release"
    set "CARGO_ARGS=--release --features custom-protocol"
)

echo ============================================================
echo    ReverieAgent - one-click launch
echo ============================================================
echo.
echo Mode: %PROFILE% ^(pass "release" for an optimised build^)
echo.

REM ---------- 1. Rust toolchain ----------
if exist "%USERPROFILE%\.cargo\bin" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where cargo >nul 2>nul
if errorlevel 1 (
    echo [ERROR] cargo not found - the Rust toolchain is not installed.
    echo         Install it from https://rustup.rs and reopen this window.
    echo.
    pause
    exit /b 1
)

REM ---------- 2. Node.js ----------
where npm >nul 2>nul
if errorlevel 1 (
    echo [ERROR] npm not found - Node.js is not installed.
    echo         Install it from https://nodejs.org
    echo.
    pause
    exit /b 1
)

REM ---------- 3. frontend dependencies ----------
if exist "apps\desktop\node_modules" (
    echo [1/4] Frontend dependencies already installed
) else (
    echo [1/4] Installing frontend dependencies ^(~1 minute^)...
    call npm --prefix apps\desktop install --no-audit --no-fund
    if errorlevel 1 goto :fail
)

REM ---------- 4. frontend bundle ----------
echo [2/4] Building the frontend...
call npm --prefix apps\desktop run build
if errorlevel 1 goto :fail

REM ---------- 5. desktop build ----------
if /i "%PROFILE%"=="release" (
    echo [3/4] Building the desktop shell ^(release, first run may take 5-10 min^)...
) else (
    echo [3/4] Building the desktop shell ^(debug, first run may take 2-4 min^)...
)
echo       First build downloads and compiles many Rust crates. Please wait.
echo.
cargo build -p reverie-desktop %CARGO_ARGS%
if errorlevel 1 goto :fail

REM ---------- 6. launch ----------
set "EXE=target\%PROFILE%\reverie-desktop.exe"
if not exist "%EXE%" (
    echo [ERROR] executable not found: %EXE%
    goto :fail
)

echo [4/4] Launching ReverieAgent...
start "" "%EXE%"
echo.
echo ============================================================
echo    Started. You can close this window.
echo    Database: %APPDATA%\com.reverieagent.desktop\reverie_agent.db
echo ============================================================
ping -n 3 127.0.0.1 >nul
exit /b 0

:fail
echo.
echo ============================================================
echo    [FAILED] Launch aborted - see the errors above.
echo ============================================================
pause
exit /b 1
