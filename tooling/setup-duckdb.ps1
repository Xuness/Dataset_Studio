$ErrorActionPreference = 'Stop'
& node (Join-Path $PSScriptRoot 'setup-duckdb.mjs')
if ($LASTEXITCODE -ne 0) { throw 'DuckDB setup failed.' }
