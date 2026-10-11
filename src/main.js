// T2：启动/停止网关按钮 → Rust 命令层（start_gateway / stop_gateway）。
// T3：状态总览——每 3 秒轮询 get_status 刷新四张状态卡。
//   - 软件启动时不主动拉网关：8787 已有网关（含面板外启动的实例）直接接管显示；
//     「启动网关」点了若已在运行，给「已在运行」提示而非重复拉起。
//   - 网关不可达 → 「未运行」灰态；字段缺失/类型不对 → 该卡「未知」。不弹错、不闪退。
// 其余按钮（安全重启/打开日志/退出）T4/T5 接入。

const { invoke } = window.__TAURI__.core;

const btnStart = document.getElementById('btn-start');
const btnStop = document.getElementById('btn-stop');
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
