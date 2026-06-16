@echo off
chcp 65001 >nul
rem ============================================================================
rem TypoFix — збірка одним кліком (Windows).
rem Подвійний клік: зупиняє стару версію (розблоковує exe), збирає реліз,
rem запускає свіжий застосунок. Env TYPOFIX_DATA_DIR НЕ потрібен — застосунок
rem сам резолвить data/ через ancestor-walk поряд з exe (коміт a897154).
rem ============================================================================

rem Робоча директорія = корінь репо (тека цього .bat), незалежно звідки запущено.
cd /d "%~dp0"

echo.
echo === TypoFix: збірка ===
echo.

rem 1) Зупинити запущений застосунок, щоб розблокувати exe.
rem    Якщо не запущений — taskkill поверне помилку; глушимо й НЕ падаємо.
echo Зупиняю стару версію (якщо запущена)...
taskkill /IM typofix-app.exe /F >nul 2>nul

rem 2) Зібрати реліз. typofix-app у відокремленому workspace (src-tauri/),
rem    тому збираємо через --manifest-path; cwd лишається коренем.
echo Збираю реліз (перший раз — довго)...
echo.
cargo build --release --manifest-path src-tauri\Cargo.toml -p typofix-app

rem 3) Перевірити результат збірки.
if errorlevel 1 (
    echo.
    echo *** Помилка збірки — дивись вивід вище. ***
    echo.
    pause
    exit /b 1
)

rem 4) Запустити свіжий exe (не блокуючи це вікно).
echo.
echo Готово, запускаю TypoFix...
start "" "src-tauri\target\release\typofix-app.exe"

echo.
echo TypoFix запущено — шукай іконку в треї.
echo.
pause
