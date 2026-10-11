// T2：启动/停止网关按钮 → Rust 命令层（start_gateway / stop_gateway）。
// T6：开机自启开关 → get_autostart / set_autostart_enabled。
// T4：安全重启 / 打开日志文件夹已接入；T5：退出按钮接入 quit_app（与点 × 行为一致）。

const { invoke } = window.__TAURI__.core;

const btnStart = document.getElementById('btn-start');
const btnStop = document.getElementById('btn-stop');
const btnRestart = document.getElementById('btn-restart');
const btnLogs = document.getElementById('btn-logs');
const btnQuit = document.getElementById('btn-quit');
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

function setBusy(isBusy) {
  btnStart.disabled = isBusy;
  btnStop.disabled = isBusy;
  btnRestart.disabled = isBusy;
}

// 安全重启不改变本壳的启停状态（supervisor 排空后自行重新拉起），故只在提示区反馈结果
btnRestart.addEventListener('click', async () => {
  setBusy(true);
  setHint('正在安全重启网关（优雅排空在途请求后重新拉起，最长等 60 秒）…');
  try {
    const res = await invoke('restart_gateway');
    if (res.kind === 'restarted') {
      setHint('安全重启已受理：网关正在排空并重新启动，稍后即可继续使用。');
    } else if (res.kind === 'not_running') {
      setHint('网关未运行——请先「启动网关」，再执行安全重启。', true);
    } else if (res.kind === 'timeout') {
      setHint('重启等待超时：网关可能仍在排空在途请求，未收到完成确认。', true);
    } else {
      setHint(`安全重启失败：${res.detail}`, true);
    }
  } catch (err) {
    setHint(`失败：调用命令层出错 ${err}`, true);
  } finally {
    setBusy(false);
  }
});

btnLogs.addEventListener('click', async () => {
  setHint('正在打开日志文件夹…');
  try {
    const res = await invoke('open_logs_dir');
    if (res.kind === 'opened') {
      setHint(`已打开日志文件夹：${res.path}`);
    } else {
      setHint(`打开日志文件夹失败：${res.detail}`, true);
    }
  } catch (err) {
    setHint(`失败：调用命令层出错 ${err}`, true);
  }
});

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

// ---- T5：退出按钮（后端 quit_app：记退出来源 panel_button 后 app.exit(0)，
// 与点 × / 托盘「退出」殊途同归，都汇入 RunEvent::Exit 钩子优雅停网关） ----
btnQuit.addEventListener('click', async () => {
  setHint('正在退出软件（网关将先优雅停止，最多等 10 秒）…');
  try {
    await invoke('quit_app');
  } catch (err) {
    setHint(`退出失败：调用命令层出错 ${err}`, true);
  }
});

// ---- T6：开机自启开关（读写注册表 Run 键，即时生效） ----

const autostartToggle = document.getElementById('autostart-toggle');
const autostartNote = document.getElementById('autostart-note');

function setAutostartNote(text, isError) {
  autostartNote.textContent = text;
  autostartNote.classList.toggle('error', Boolean(isError));
}

async function loadAutostart() {
  try {
    const status = await invoke('get_autostart');
    autostartToggle.checked = status.enabled;
    if (Array.isArray(status.notes) && status.notes.length > 0) {
      setAutostartNote(status.notes.join('；'));
    }
  } catch (err) {
    setAutostartNote(`开机自启状态获取失败：${err}`, true);
  }
}

autostartToggle.addEventListener('change', async () => {
  const want = autostartToggle.checked;
  autostartToggle.disabled = true;
  try {
    const res = await invoke('set_autostart_enabled', { enabled: want });
    if (res.kind === 'ok') {
      autostartToggle.checked = res.status.enabled;
      setAutostartNote(
        res.status.enabled
          ? '开机自启：已开启（指向当前程序，登录后自动启动）'
          : '开机自启：已关闭（重启电脑后不再自动启动）'
      );
    } else {
      autostartToggle.checked = !want; // 失败回弹到设置前的状态
      setAutostartNote(`开机自启设置失败：${res.reason}`, true);
    }
  } catch (err) {
    autostartToggle.checked = !want;
    setAutostartNote(`开机自启设置失败：调用命令层出错 ${err}`, true);
  } finally {
    autostartToggle.disabled = false;
  }
});

loadAutostart();
