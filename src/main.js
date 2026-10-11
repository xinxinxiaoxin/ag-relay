// T2：启动/停止网关按钮 → Rust 命令层（start_gateway / stop_gateway）。
// 其余按钮（安全重启/打开日志/退出）T3/T4/T5 接入。

const { invoke } = window.__TAURI__.core;

const btnStart = document.getElementById('btn-start');
const btnStop = document.getElementById('btn-stop');
const hint = document.getElementById('action-hint');
const stateValue = document.querySelector('[data-key="state"]');

const FAILURE_TEXT = {
  node_missing: '失败：自带 node 运行时缺失（安装不完整）',
  gateway_script_missing: '失败：网关脚本缺失（安装不完整）',
  port_already_in_use: '失败：端口 8787 已被占用——网关可能已在运行（含面板外启动的实例），未重复拉起',
  spawn_failed: '失败：拉起网关进程失败',
  job_setup_failed: '失败：进程树兜底（Job Object）建立失败，为防孤儿已回滚',
};

function setHint(text, isError) {
  hint.textContent = text;
  hint.classList.toggle('error', Boolean(isError));
}

function setRunning(isRunning) {
  stateValue.textContent = isRunning ? '运行中' : '已停止';
}

function setBusy(isBusy) {
  btnStart.disabled = isBusy;
  btnStop.disabled = isBusy;
}

btnStart.addEventListener('click', async () => {
  setBusy(true);
  setHint('正在启动网关…');
  try {
    const res = await invoke('start_gateway');
    if (res.kind === 'started') {
      setRunning(true);
      setHint(`网关已启动（supervisor PID ${res.supervisor_pid}），监听 127.0.0.1:8787。`);
    } else if (res.kind === 'failed') {
      setRunning(false);
      setHint(FAILURE_TEXT[res.reason] || `失败：${res.reason}`, true);
    } else {
      setHint(`意外结果：${res.kind}`, true);
    }
  } catch (err) {
    setHint(`失败：调用命令层出错 ${err}`, true);
  } finally {
    setBusy(false);
  }
});

btnStop.addEventListener('click', async () => {
  setBusy(true);
  setHint('正在停止网关（优雅排空，最多等 10 秒）…');
  try {
    const res = await invoke('stop_gateway');
    if (res.kind === 'stopped') {
      setRunning(false);
      if (res.graceful) {
        setHint(`网关已优雅停止（排空完成，退出码 ${res.exit_code ?? '未知'}）。`);
      } else {
        setHint('优雅排空超时，已强制结束网关进程树。', true);
      }
    } else if (res.kind === 'not_running') {
      setHint('网关当前未由面板启动（无需停止，可放心重复点击）。');
      setRunning(false);
    } else if (res.kind === 'failed') {
      setHint(FAILURE_TEXT[res.reason] || `失败：${res.reason}`, true);
    } else {
      setHint(`意外结果：${res.kind}`, true);
    }
  } catch (err) {
    setHint(`失败：调用命令层出错 ${err}`, true);
  } finally {
    setBusy(false);
  }
});
