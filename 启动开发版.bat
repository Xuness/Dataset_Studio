@echo off
chcp 65001 >nul
cd /d "%~dp0"
pwsh -NoLogo -NoProfile -File "%~dp0tooling\start-dev.ps1"
if errorlevel 1 pause
