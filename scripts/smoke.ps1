# T1 冒烟检查：产物存在 + 窗口标题正确 + 进程可退出
# 用法（PowerShell，构建后运行）：
#   powershell -ExecutionPolicy Bypass -File scripts\smoke.ps1 [-ExePath <路径>] [-KeepRunning]
# 默认 exe 路径按共享约定 CARGO_TARGET_DIR=D:\zcode项目\ag-relay-build\target 解析。

param(
    [string]$ExePath = "D:\zcode项目\ag-relay-build\target\release\ag-relay.exe",
    [switch]$KeepRunning
)

$ErrorActionPreference = "Stop"

# 1. 产物存在
if (-not (Test-Path $ExePath)) {
    Write-Output "SMOKE-FAIL: 未找到构建产物 $ExePath（先完成打包再冒烟）"
    exit 1
}
Write-Output "SMOKE-OK-1: 产物存在 $ExePath"

# 2. 启动并检查主窗口标题
$proc = Start-Process -FilePath $ExePath -PassThru
try {
    $deadline = (Get-Date).AddSeconds(20)
    $title = ""
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 300
        $proc.Refresh()
        if ($proc.HasExited) {
            Write-Output "SMOKE-FAIL: 进程启动后立即退出，退出码 $($proc.ExitCode)"
            exit 1
        }
        if ($proc.MainWindowHandle -ne 0) {
            $title = $proc.MainWindowTitle
            if ($title -like "*AG Relay*") { break }
        }
    }
    if ($title -notlike "*AG Relay*") {
        Write-Output "SMOKE-FAIL: 未取到含 'AG Relay' 的主窗口标题（实际: '$title'）"
        exit 1
    }
    Write-Output "SMOKE-OK-2: 窗口标题 '$title'，PID $($proc.Id)"

    if (-not $KeepRunning) {
        Stop-Process -Id $proc.Id -Force
        Write-Output "SMOKE-OK-3: 进程已关闭"
    } else {
        Write-Output "SMOKE-KEEP: 进程保持运行，PID $($proc.Id)（需手动 Stop-Process）"
    }
    Write-Output "SMOKE-PASS"
    exit 0
} finally {
    if (-not $KeepRunning -and $proc -and -not $proc.HasExited) {
        try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch {}
    }
}
