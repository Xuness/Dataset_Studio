$ErrorActionPreference = 'Stop'
$studioArguments = @($args)
$studioRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $studioRoot
foreach ($toolName in @('node','pnpm','cargo')) {
    if (-not (Get-Command $toolName -ErrorAction SilentlyContinue)) {
        throw "缺少 $toolName，请先安装开发环境后重试。"
    }
}
$studioLogDirectory = Join-Path $studioRoot '.local\logs'
New-Item -ItemType Directory -Path $studioLogDirectory -Force | Out-Null
$studioLaunchKey = '{0}-{1}' -f (Get-Date -Format 'yyyyMMdd-HHmmss-fff'), $PID
$studioStartupLog = Join-Path $studioLogDirectory "startup-install-$studioLaunchKey.log"
$studioDuckDbLog = Join-Path $studioLogDirectory "startup-duckdb-$studioLaunchKey.log"
$studioDevLog = Join-Path $studioLogDirectory "startup-dev-$studioLaunchKey.log"
Write-Host 'Dataset Studio · 开发模式'
Write-Host '检查依赖与锁文件…'
& pnpm install --frozen-lockfile *> $studioStartupLog
if ($LASTEXITCODE -ne 0) {
    Get-Content -LiteralPath $studioStartupLog -Tail 25
    throw '依赖安装失败。'
}
Write-Host '检查元数据运行库…'
& (Join-Path $studioRoot 'tooling\setup-duckdb.ps1') *> $studioDuckDbLog
Write-Host "启动日志：$studioDevLog"
& node (Join-Path $studioRoot 'tooling\dev.mjs') @studioArguments 2>&1 | Tee-Object -FilePath $studioDevLog
exit $LASTEXITCODE
