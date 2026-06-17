@echo off
rem ============================================================================
rem TypoFix - one-click INSTALLER build (Windows). Double-click to run.
rem Builds the PRODUCTION installers you can share with friends:
rem   - NSIS  .exe setup  (the one to hand out)
rem   - MSI   .msi        (corporate alternative)
rem Run this only when you actually want fresh installers - normal day-to-day
rem checking is build.bat (faster, no installers).
rem ASCII-only on purpose: cmd reads .bat in the OEM codepage, so Cyrillic in
rem echo/rem lines gets parsed as commands and breaks the script.
rem No TYPOFIX_DATA_DIR needed - the bundle embeds data/ via bundle.resources in
rem tauri.conf.json, and the app also resolves data/ via ancestor-walk.
rem
rem WHY the tauri CLI and NOT a raw cargo build: same reason as build.bat -
rem tauri build enables the custom-protocol feature (embedded, localhost-free
rem frontend) AND runs beforeBuildCommand (npm run build -> ui/dist). Unlike
rem build.bat we do NOT pass --no-bundle, so it ALSO produces the installers
rem under src-tauri\target\release\bundle\ (nsis\ and msi\).
rem
rem NOTE: error handling uses GOTO labels, NOT parenthesized if-blocks, and no
rem ')' appears inside any echo - a literal ')' inside an echo closes a paren
rem block early and makes cmd die with "was unexpected at this time".
rem ============================================================================

rem Work from the repo root (folder of this .bat). The tauri CLI must run from
rem the repo root - it looks for ./src-tauri/tauri.conf.json and does NOT walk up.
cd /d "%~dp0"

echo.
echo === TypoFix: installer build ===
echo.

rem 1) Stop the running app to unlock the exe (ignore error if not running).
rem    Without this the release link step fails with "Access is denied".
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
rem 3) Production installer build via the tauri CLI (from the repo root).
echo Building installers - this takes a while...
echo.
call "ui\node_modules\.bin\tauri.cmd" build
if errorlevel 1 goto build_failed

rem 4) Show where the installers landed and open the folder in Explorer.
echo.
echo === Done. Installers are here: ===
echo   src-tauri\target\release\bundle\nsis\   - the .exe setup to share
echo   src-tauri\target\release\bundle\msi\    - the .msi alternative
echo.
start "" "src-tauri\target\release\bundle"
echo Opened the bundle folder in Explorer.
echo.
pause
exit /b 0

:build_failed
echo.
echo *** BUILD FAILED - see the output above. ***
echo.
pause
exit /b 1
