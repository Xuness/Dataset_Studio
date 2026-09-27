param([string]$Python = 'python', [switch]$Dev)
$ErrorActionPreference = 'Stop'
$studioRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$runtimeRoot = Join-Path $studioRoot '.local\runtime\lake-worker'
& $Python -m venv $runtimeRoot
if ($LASTEXITCODE -ne 0) { throw 'Python 3.11–3.13 is required to create the worker runtime.' }
$runtimePython = Join-Path $runtimeRoot 'Scripts\python.exe'
$packagePath = Join-Path $studioRoot 'services\lake-worker'
if ($Dev) { $packagePath += '[dev]' }
& $runtimePython -m pip install $packagePath
if ($LASTEXITCODE -ne 0) { throw 'Lake worker dependency installation failed.' }
Write-Output "数据湖运行环境：$runtimePython"
