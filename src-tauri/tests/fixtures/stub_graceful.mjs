// 桩 1：收到 SIGINT 立即优雅退出（模拟 supervisor/gateway 的 SIGINT→drain→exit 0 行为）
process.on('SIGINT', () => {
  process.exit(0);
});
// 就绪信号：监听 STUB_READY_PORT（测试等到端口可连再广播 CTRL_C——
// 保证 node 的 SIGINT 控制台处理器已注册，消除「CTRL_C 早于 node 引导完成」的竞态）
import net from 'node:net';
const readyPort = Number(process.env.STUB_READY_PORT || 0);
if (readyPort) net.createServer(() => {}).listen(readyPort, '127.0.0.1');
// 保持进程存活等待信号
setInterval(() => {}, 1_000_000);
