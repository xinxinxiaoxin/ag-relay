// T2：启动/停止网关按钮 → Rust 命令层（start_gateway / stop_gateway）。
// T3：状态总览——每 3 秒轮询 get_status 刷新四张状态卡。
//   - 8787 已有网关（含面板外启动的实例）直接接管显示；
//     「启动网关」点了若已在运行，给「已在运行」提示而非重复拉起。
//   - 网关不可达 → 「未运行」灰态；字段缺失/类型不对 → 该卡「未知」。不弹错、不闪退。
// T4：安全重启/打开日志按钮；T6：开机自启开关 + 启动即拉网关（Rust 侧 setup 编排）。

const { invoke } = window.__TAURI__.core;

const btnStart = document.getElementById('btn-start');
const btnStop = document.getElementById('btn-stop');
const btnRestart = document.getElementById('btn-restart');
const btnLogs = document.getElementById('btn-logs');
const hint = document.getElementById('action-hint');

const cardEls = {
  state: document.querySelector('[data-key="state"]'),
  version: document.querySelector('[data-key="version"]'),
  activeRequests: document.querySelector('[data-key="activeRequests"]'),
  uptime: document.querySelector('[data-key="uptime"]'),
};

// 网关 status 有约 20 秒内部缓存节奏（PROXY_STATUS_INTERVAL_MS），数值不秒变属正常
const STATUS_POLL_MS = 3000;

// 命令层万一异常时的兜底快照：整体按未运行显示
const OFFLINE_SNAPSHOT = { available: false, version: null, activeRequests: null, uptimeText: null };

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

// mode：'ok' 高亮 | 'off' 未运行灰态 | 'unknown' 未知
function setCard(key, text, mode) {
  const el = cardEls[key];
  el.textContent = text;
  el.classList.toggle('off', mode === 'off');
  el.classList.toggle('unknown', mode === 'unknown');
}

function renderSnapshot(snap) {
  if (snap && snap.available) {
    setCard('state', '运行中', 'ok');
    setCard('version', snap.version ?? '未知', snap.version == null ? 'unknown' : 'ok');
    setCard('activeRequests', snap.activeRequests ?? '未知', snap.activeRequests == null ? 'unknown' : 'ok');
    setCard('uptime', snap.uptimeText ?? '未知', snap.uptimeText == null ? 'unknown' : 'ok');
  } else {
    setCard('state', '未运行', 'off');
    setCard('version', '—', 'off');
    setCard('activeRequests', '—', 'off');
    setCard('uptime', '—', 'off');
  }
}

async function pollStatus() {
  let snap;
  try {
    snap = await invoke('get_status');
  } catch {
    snap = OFFLINE_SNAPSHOT;
  }
  renderSnapshot(snap);
}

function setBusy(isBusy) {
  btnStart.disabled = isBusy;
  btnStop.disabled = isBusy;
  btnRestart.disabled = isBusy;
}

// 「端口被占」的接管语义：先探测一次状态——真有网关在跑就按「已在运行」提示（非错误）；
// 探测也不可达才维持原有的占用失败提示。
async function hintForPortInUse() {
  let snap = OFFLINE_SNAPSHOT;
  try {
    snap = await invoke('get_status');
  } catch {
    // 保持离线快照
  }
  if (snap.available) {
    setHint('网关已在运行（面板已接管显示其状态），无需重复启动。');
  } else {
    setHint(FAILURE_TEXT.port_already_in_use, true);
  }
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
      setHint(`网关已启动（supervisor PID ${res.supervisor_pid}），监听 127.0.0.1:8787。`);
    } else if (res.kind === 'failed') {
      if (res.reason === 'port_already_in_use') {
        await hintForPortInUse();
      } else {
        setHint(FAILURE_TEXT[res.reason] || `失败：${res.reason}`, true);
      }
    } else {
      setHint(`意外结果：${res.kind}`, true);
    }
  } catch (err) {
    setHint(`失败：调用命令层出错 ${err}`, true);
  } finally {
    setBusy(false);
    pollStatus();
  }
});

btnStop.addEventListener('click', async () => {
  setBusy(true);
  setHint('正在停止网关（优雅排空，最多等 10 秒）…');
  try {
    const res = await invoke('stop_gateway');
    if (res.kind === 'stopped') {
      if (res.graceful) {
        setHint(`网关已优雅停止（排空完成，退出码 ${res.exit_code ?? '未知'}）。`);
      } else {
        setHint('优雅排空超时，已强制结束网关进程树。', true);
      }
    } else if (res.kind === 'not_running') {
      setHint('网关当前未由面板启动（无需停止，可放心重复点击）。');
    } else if (res.kind === 'failed') {
      setHint(FAILURE_TEXT[res.reason] || `失败：${res.reason}`, true);
    } else {
      setHint(`意外结果：${res.kind}`, true);
    }
  } catch (err) {
    setHint(`失败：调用命令层出错 ${err}`, true);
  } finally {
    setBusy(false);
    pollStatus();
  }
});

// 启动即轮询，此后每 3 秒刷新一次（接管显示全靠它，无需任何「连接」动作）
pollStatus();
setInterval(pollStatus, STATUS_POLL_MS);

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
