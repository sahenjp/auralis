@echo off
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-auralis.ps1"
set "AURALIS_EXIT=%ERRORLEVEL%"
if not "%AURALIS_EXIT%"=="0" (
  echo.
  echo Auralis could not start. See the error above.
  pause
)
exit /b %AURALIS_EXIT%
