$ErrorActionPreference = 'Stop'
$studioRoot = Split-Path -Parent $PSScriptRoot
$vendorDirectory = Join-Path $studioRoot 'vendor\duckdb'
New-Item -ItemType Directory -Path $vendorDirectory -Force | Out-Null
$archivePath = Join-Path $vendorDirectory 'duckdb.zip'
$expectedHash = '74E73AFD3B010C6F310E14A961FFF679F876952BC196F82584B0E2E76D11A91F'
if (-not (Test-Path -LiteralPath $archivePath)) {
    Invoke-WebRequest -Uri 'https://github.com/duckdb/duckdb/releases/download/v1.5.4/libduckdb-windows-amd64.zip' -OutFile $archivePath
}
if ((Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash -ne $expectedHash) {
    throw 'DuckDB archive checksum mismatch.'
}
Expand-Archive -LiteralPath $archivePath -DestinationPath $vendorDirectory -Force
Write-Output 'DuckDB 1.5.4 compatibility probe runtime is ready.'
