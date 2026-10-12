# 只读发布物冒烟：不启动应用、不安装、不修改注册表、不探测/停止网关。
# 真实 Windows 安装/升级须在独立 VM 或 Windows Sandbox 内人工验收。
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
