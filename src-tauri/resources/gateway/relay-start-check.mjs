import net from 'node:net';

const portText = process.env.PROXY_PORT || '8787';
const port = Number(portText);
const configuredHost = process.env.PROXY_HOST || '127.0.0.1';
const host = configuredHost === '0.0.0.0' ? '127.0.0.1'
  : configuredHost === '::' ? '::1' : configuredHost;

if (!/^\d+$/.test(portText) || !Number.isInteger(port) || port < 1 || port > 65535) {
  console.error('[startup] Invalid PROXY_PORT; gateway was not started.');
  process.exitCode = 20;
} else {
  try {
    const occupied = await new Promise((resolve, reject) => {
      const socket = net.createConnection({ host, port });
      const finish = (error, value) => {
        socket.removeAllListeners();
        socket.on('error', () => {});
        socket.destroy();
        if (error) reject(error);
        else resolve(value);
      };
      socket.setTimeout(2000, () => finish(new Error('probe_timeout')));
      socket.once('connect', () => finish(null, true));
      socket.once('error', (error) => {
        if (error.code === 'ECONNREFUSED') finish(null, false);
        else finish(error);
      });
    });
    if (occupied) {
      console.log(`[startup] Port ${port} is already in use; no second gateway was started.`);
      process.exitCode = 10;
    }
  } catch {
    console.error('[startup] Could not check the listening port; gateway was not started.');
    process.exitCode = 20;
  }
}
