param([string]$Python = 'python', [switch]$Dev)
$ErrorActionPreference = 'Stop'
$studioArgs = @("--python=$Python")
if ($Dev) { $studioArgs += '--dev' }
& node (Join-Path $PSScriptRoot 'setup-lake-worker.mjs') @studioArgs
if ($LASTEXITCODE -ne 0) { throw 'Lake worker setup failed.' }
