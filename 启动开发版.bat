@echo off
setlocal
chcp 65001 >nul
cd /d "%~dp0"
pwsh -NoLogo -NoProfile -File "%~dp0tooling\start-dev.ps1" %*
set "studioExit=%errorlevel%"
if not "%studioExit%"=="0" pause
exit /b %studioExit%
