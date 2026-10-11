// F1 外部停止链的桩「supervisor 替身」：spawn 一个真子进程（模拟网关子进程），
// 把子进程 pid 写入 STUB_CHILD_PID_FILE，并监听 STUB_READY_PORT 作为就绪信号。
// 收到 CTRL_C（SIGINT）：先杀子进程再退出（退出码 0）——模拟 supervisor 的排空语义。
// 注意：本桩的命令行含 stub_parent.mjs、不含 relay-supervisor.mjs，
// 正好用来验证外部停止链的「父进程非 supervisor → 放弃（宁可不杀不可误杀）」判定。
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { createServer } from 'node:net';

const readyPort = Number(process.env.STUB_READY_PORT);
const pidFile = process.env.STUB_CHILD_PID_FILE;

const child = spawn(process.execPath, ['-e', 'setInterval(()=>{},200);'], {
  stdio: 'ignore',
});
writeFileSync(pidFile, String(child.pid));

const killChild = () => {
  try {
    child.kill('SIGKILL');
  } catch {}
};

process.on('SIGINT', () => {
  killChild();
  process.exit(0);
});
process.on('exit', killChild);

createServer(() => {}).listen(readyPort, '127.0.0.1');
// 常驻事件循环，等待被 CTRL_C 停止
setInterval(() => {}, 1000);
