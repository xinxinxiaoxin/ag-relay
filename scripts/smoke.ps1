# 只读发布物冒烟：不启动应用、不安装、不修改注册表、不探测/停止网关。
# 取舍：宿主机只做只读验签（现用网关正被使用，真实安装/更新有系统级副作用），
# 真实成品验收在独立 VM/Windows Sandbox 内人工执行，逐条步骤见
# docs/windows-acceptance-checklist.md；本脚本不替代该验收。
# 用法：npm run smoke -- -Artifacts <发布物目录> [-Config <该构建的 tauri 配置>]
param(
    [string]$Artifacts,
    [string]$Config = (Join-Path $PSScriptRoot "../src-tauri/tauri.conf.json")
)
$ErrorActionPreference = "Stop"
if (-not $Artifacts) {
    Write-Error "必须显式提供 -Artifacts。此脚本仅验签，不再自动启动或强杀 AG Relay。"
    exit 1
}
& node (Join-Path $PSScriptRoot "release.mjs") verify --config $Config --artifacts $Artifacts
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Output "SMOKE-PASS: 清单与签名通过；未安装、未启动，不能据此声称干净机器运行验收通过。"
