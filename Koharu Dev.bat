@echo off
setlocal
title Koharu V2 Development
cd /d "%~dp0"

if /i "%~1"=="1" goto launch
if /i "%~1"=="2" goto rebuild
if /i "%~1"=="Q" exit /b 0

:menu
echo.
echo   Koharu V2 development
echo.
echo   1  Launch saved debug build (no Rust rebuild)
echo   2  Compile changed code and launch
echo   Q  Exit
echo.
choice /c 12Q /n /m "Choose 1, 2, or Q: "
if errorlevel 3 exit /b 0
if errorlevel 2 goto rebuild

:launch
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\launch-dev.ps1"
goto finished

:rebuild
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\launch-dev.ps1" -Rebuild

:finished
set "LAUNCH_EXIT=%ERRORLEVEL%"
if not "%~1"=="" exit /b %LAUNCH_EXIT%
if not "%LAUNCH_EXIT%"=="0" pause
goto menu
