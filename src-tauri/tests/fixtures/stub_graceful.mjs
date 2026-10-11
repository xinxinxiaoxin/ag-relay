// 桩 1：收到 SIGINT 立即优雅退出（模拟 supervisor/gateway 的 SIGINT→drain→exit 0 行为）
process.on('SIGINT', () => {
  process.exit(0);
});
// 保持进程存活等待信号
setInterval(() => {}, 1_000_000);
