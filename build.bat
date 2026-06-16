@echo off
rem ============================================================================
rem TypoFix - one-click build (Windows). Double-click to run.
rem Stops the running app (unlocks the exe), builds release, launches it.
rem ASCII-only on purpose: cmd reads .bat in the OEM codepage, so Cyrillic in
rem echo/rem lines gets parsed as commands and breaks the script.
rem No TYPOFIX_DATA_DIR needed - the app resolves data/ via ancestor-walk.
rem ============================================================================

rem Work from the repo root (folder of this .bat), regardless of launch dir.
cd /d "%~dp0"

echo.
echo === TypoFix: build ===
echo.

rem 1) Stop the running app to unlock the exe (ignore error if not running).
echo Stopping old instance (if running)...
taskkill /IM typofix-app.exe /F >nul 2>nul

rem 2) Build release. typofix-app is in an isolated workspace (src-tauri/),
rem    so build via --manifest-path; cwd stays at the repo root.
echo Building release (first run takes a while)...
echo.
cargo build --release --manifest-path src-tauri\Cargo.toml

rem 3) Check the build result.
if errorlevel 1 (
    echo.
    echo *** BUILD FAILED - see the output above. ***
    echo.
    pause
    exit /b 1
)

rem 4) Launch the fresh exe without blocking this window.
echo.
echo Done. Launching TypoFix...
start "" "src-tauri\target\release\typofix-app.exe"

echo.
echo TypoFix started - look for the tray icon.
echo.
pause
