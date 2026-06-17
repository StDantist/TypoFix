@echo off
rem ============================================================================
rem TypoFix - one-click build (Windows). Double-click to run.
rem Stops the running app (unlocks the exe), builds a PRODUCTION release, runs it.
rem ASCII-only on purpose: cmd reads .bat in the OEM codepage, so Cyrillic in
rem echo/rem lines gets parsed as commands and breaks the script.
rem No TYPOFIX_DATA_DIR needed - the app resolves data/ via ancestor-walk.
rem
rem WHY the tauri CLI and NOT a raw cargo build:
rem   Tauri picks dev-vs-prod from the custom-protocol cargo feature
rem   (tauri build.rs sets dev = NOT custom-protocol). A raw cargo build --release
rem   does NOT enable it, so the webview falls back to devUrl http://localhost:1420
rem   (ERR_CONNECTION_REFUSED in prod). tauri build enables custom-protocol AND
rem   runs beforeBuildCommand (npm run build -> ui/dist), giving an embedded,
rem   localhost-free frontend. --no-bundle = just the exe, skip installers.
rem
rem NOTE: error handling uses GOTO labels, NOT parenthesized if-blocks. A literal
rem ')' inside an echo (e.g. a parenthetical aside) closes a paren-block early and
rem makes cmd die with "was unexpected at this time" - so we avoid both.
rem ============================================================================

rem Work from the repo root (folder of this .bat). The tauri CLI must run from
rem the repo root - it looks for ./src-tauri/tauri.conf.json and does NOT walk up.
cd /d "%~dp0"

echo.
echo === TypoFix: build ===
echo.

rem 1) Stop the running app to unlock the exe (ignore error if not running).
echo Stopping old instance if running...
taskkill /IM typofix-app.exe /F >nul 2>nul

rem 2) Ensure frontend deps - the tauri CLI itself lives in ui/node_modules.
if not exist "ui\node_modules\.bin\tauri.cmd" goto install_deps
goto build

:install_deps
echo Installing frontend dependencies - first run, takes a while...
call npm --prefix ui install
if errorlevel 1 goto npm_failed
goto build

:npm_failed
echo.
echo *** npm install FAILED - see the output above. ***
echo.
pause
exit /b 1

:build
rem 3) Production build via the tauri CLI (from the repo root).
echo Building release - first run takes a while...
echo.
call "ui\node_modules\.bin\tauri.cmd" build --no-bundle
if errorlevel 1 goto build_failed

rem 4) Launch the fresh exe without blocking this window.
echo.
echo Done. Launching TypoFix...
start "" "src-tauri\target\release\typofix-app.exe"

echo.
echo TypoFix started - look for the tray icon. Left-click it to open Settings.
echo.
pause
exit /b 0

:build_failed
echo.
echo *** BUILD FAILED - see the output above. ***
echo.
pause
exit /b 1
