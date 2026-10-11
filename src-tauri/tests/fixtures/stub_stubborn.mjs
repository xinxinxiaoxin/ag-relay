// 桩 2：注册 SIGINT 处理器但不退出（模拟优雅关停不可达时的钉子户），持续 sleep
process.on('SIGINT', () => {
  // 故意什么都不做：优雅关停路径失效，只能靠 10 秒强杀兜底
});
// 就绪信号：监听 STUB_READY_PORT（测试等到端口可连再广播 CTRL_C，见桩 1 注释）
import net from 'node:net';
const readyPort = Number(process.env.STUB_READY_PORT || 0);
if (readyPort) net.createServer(() => {}).listen(readyPort, '127.0.0.1');
// 保持进程存活等待信号
setInterval(() => {}, 1_000_000);
