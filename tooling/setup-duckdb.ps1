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
$dllPath = Join-Path $vendorDirectory 'duckdb.dll'
# Avoid replacing a DLL already loaded by an active engine. Verify the extracted
# binary against the checked archive before deciding whether extraction is needed.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$duckArchive = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
try {
    $duckEntry = $duckArchive.GetEntry('duckdb.dll')
    if ($null -eq $duckEntry) { throw 'DuckDB archive has no runtime library.' }
    $duckStream = $duckEntry.Open()
    try { $duckExpected = [Convert]::ToHexString([System.Security.Cryptography.SHA256]::HashData($duckStream)) }
    finally { $duckStream.Dispose() }
} finally { $duckArchive.Dispose() }
if (-not (Test-Path -LiteralPath $dllPath) -or (Get-FileHash -LiteralPath $dllPath -Algorithm SHA256).Hash -ne $duckExpected) {
    Expand-Archive -LiteralPath $archivePath -DestinationPath $vendorDirectory -Force
}
Write-Output 'DuckDB 1.5.4 metadata runtime is ready.'
