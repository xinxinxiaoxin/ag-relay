import { fork, spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const mainScript = fileURLToPath(new URL('./relay-retry-proxy.mjs', import.meta.url));
const mainDirectory = fileURLToPath(new URL('.', import.meta.url));
let current = null;
let stopping = false;
let finished = false;

function finish(code) {
  if (finished) return;
  finished = true;
  process.exitCode = Number.isInteger(code) && code >= 0 ? code : 1;
}

function send(state, message, callback = () => {}) {
  if (state.exited || !state.child.connected) {
    callback(new Error('ipc_unavailable'));
    return;
  }
  try {
    state.child.send(message, (error) => callback(error || null));
  } catch (error) {
    callback(error);
  }
}

function requestShutdown(state) {
  if (state.exited || state.shutdownSent) return;
  // fork establishes IPC before the application's ready message; messages may
  // safely queue while the child initializes its shutdown handler.
  if (!state.child.connected) return;
  state.shutdownSent = true;
  send(state, { type: 'shutdown' }, (error) => {
    if (error && !state.exited) {
      state.shutdownSent = false;
      console.error('[supervisor] 无法发送排空请求，等待子进程退出；不会强杀。');
    }
  });
}

function rejectRestart(state) {
  if (stopping || state.exited || current !== state) return;
  state.phase = 'running';
  send(state, { type: 'restart_rejected', reason: 'syntax_check_failed' });
}

function checkRestart(state) {
  if (stopping || state.exited || current !== state || state.phase !== 'running') return;
  state.phase = 'checking';
  let checker;
  try {
    checker = spawn(process.execPath, ['--check', mainScript], {
      cwd: mainDirectory,
      shell: false,
      stdio: ['ignore', 'inherit', 'inherit'],
      windowsHide: true,
    });
  } catch {
    rejectRestart(state);
    return;
  }
  state.checker = checker;
  let settled = false;
  const complete = (valid) => {
    if (settled) return;
    settled = true;
    state.checker = null;
    if (stopping || state.exited || current !== state) return;
    if (!valid) {
      rejectRestart(state);
      return;
    }
    // Authorization belongs only to this child and only to this validated
    // request. Never drain the running child before syntax validation passes.
    state.phase = 'draining';
    state.restartAuthorized = true;
    send(state, { type: 'drain' }, (error) => {
      if (!error || state.exited) return;
      state.restartAuthorized = false;
      state.phase = 'running';
      send(state, { type: 'restart_rejected', reason: 'supervisor_unavailable' });
      console.error('[supervisor] 无法发送重启排空请求，已取消重启。');
    });
  };
  checker.once('error', () => complete(false));
  checker.once('close', (code, signal) => complete(code === 0 && signal === null));
}

function launch() {
  if (stopping || finished) return;
  let child;
  try {
    child = fork(mainScript, [], {
      cwd: mainDirectory,
      env: { ...process.env, PROXY_SUPERVISED: '1' },
      stdio: ['inherit', 'inherit', 'inherit', 'ipc'],
    });
  } catch {
    console.error('[supervisor] 子进程启动失败。');
    finish(1);
    return;
  }
  const state = {
    child,
    pid: child.pid ?? null,
    phase: 'running',
    restartAuthorized: false,
    shutdownSent: false,
    checker: null,
    exited: false,
    failed: false,
  };
  current = state;
  child.once('spawn', () => {
    state.pid = child.pid;
    console.log(`[supervisor] child PID=${state.pid}`);
    if (stopping) requestShutdown(state);
  });
  child.on('message', (message) => {
    if (state.exited || current !== state || !message || typeof message !== 'object') return;
    if (stopping) {
      // Also handles a child whose application becomes ready after the signal.
      if (message.type === 'ready') {
        state.shutdownSent = false;
        requestShutdown(state);
      }
      return;
    }
    if (message.type === 'restart_request') checkRestart(state);
  });
  child.on('error', () => {
    state.failed = true;
    state.restartAuthorized = false;
    console.error('[supervisor] 子进程发生错误；不会自动重启。');
  });
  const onExit = (code, signal) => {
    if (state.exited) return;
    state.exited = true;
    const restart = !stopping && !state.failed && state.restartAuthorized
      && state.phase === 'draining' && code === 75 && signal === null;
    state.restartAuthorized = false;
    if (restart) {
      launch();
    } else {
      // Signal exits have no numeric code; use the conventional shell code.
      finish(code ?? (signal === 'SIGINT' ? 130 : signal === 'SIGTERM' ? 143 : 1));
    }
  };
  child.once('exit', onExit);
  // A spawn failure may emit error + close without ever emitting exit.
  child.once('close', onExit);
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    stopping = true;
    if (!current || current.exited) {
      finish(0);
      return;
    }
    current.restartAuthorized = false;
    requestShutdown(current);
  });
}

launch();
