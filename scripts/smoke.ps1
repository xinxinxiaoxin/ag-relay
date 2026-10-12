# 只读发布物冒烟：不启动应用、不安装、不修改注册表、不探测/停止网关。
# 取舍：宿主机只做只读验签（现用网关正被使用，真实安装/更新有系统级副作用），
# 真实成品验收在独立 VM/Windows Sandbox 内人工执行，逐条步骤见
# docs/windows-acceptance-checklist.md；本脚本不替代该验收。
# 用法：npm run smoke -- -Artifacts <发布物目录> [-Config <该构建的 tauri 配置>]
# 测试构建产物（目录含 TEST-ONLY-NOT-FOR-RELEASE）自带 tauri.test.json，默认用它验签；
# 正式产物仍用仓库已审核配置，绝不能拿测试公钥冒充正式验签。
param(
    [string]$Artifacts,
    [string]$Config
)
$ErrorActionPreference = "Stop"
if (-not $Artifacts) {
    Write-Error "必须显式提供 -Artifacts。此脚本仅验签，不再自动启动或强杀 AG Relay。"
    exit 1
}
$isTestArtifact = Test-Path (Join-Path $Artifacts "TEST-ONLY-NOT-FOR-RELEASE")
if (-not $Config) {
    if ($isTestArtifact) {
        $Config = Join-Path $Artifacts "tauri.test.json"
        if (-not (Test-Path $Config)) {
            Write-Error "这是测试产物但缺少 tauri.test.json，无法用测试公钥验签。"
            exit 1
        }
        Write-Output "注意：这是标记为 TEST-ONLY 的测试产物，使用其中自带的测试公钥验签，不能当作正式发布物。"
    } else {
        $Config = Join-Path $PSScriptRoot "../src-tauri/tauri.conf.json"
    }
}
& node (Join-Path $PSScriptRoot "release.mjs") verify --config $Config --artifacts $Artifacts
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
if ($isTestArtifact) {
    Write-Output "SMOKE-PASS（测试产物）：清单与测试签名通过；未安装、未启动，不能据此声称正式发布或干净机器运行验收通过。"
} else {
    Write-Output "SMOKE-PASS: 清单与签名通过；未安装、未启动，不能据此声称干净机器运行验收通过。"
}
