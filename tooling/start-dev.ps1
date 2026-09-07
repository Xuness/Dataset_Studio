$ErrorActionPreference = 'Stop'
$studioRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $studioRoot
foreach ($toolName in @('node','pnpm','cargo')) {
    if (-not (Get-Command $toolName -ErrorAction SilentlyContinue)) {
        throw "缺少 $toolName，请先安装开发环境后重试。"
    }
}
New-Item -ItemType Directory -Path (Join-Path $studioRoot '.local') -Force | Out-Null
Write-Host 'Dataset Studio · 开发模式'
Write-Host '检查依赖与锁文件…'
& pnpm install --frozen-lockfile *> (Join-Path $studioRoot '.local\startup-install.log')
if ($LASTEXITCODE -ne 0) {
    Get-Content -LiteralPath (Join-Path $studioRoot '.local\startup-install.log') -Tail 25
    throw '依赖安装失败。'
}
& node (Join-Path $studioRoot 'tooling\dev.mjs')
exit $LASTEXITCODE
