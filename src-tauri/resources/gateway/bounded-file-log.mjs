// 自有普通文件日志。此模块没有网关启动、端口或默认日志目录等副作用。
import {
  appendFileSync, mkdirSync, statSync, renameSync, rmSync,
  openSync, readSync, writeSync, ftruncateSync, closeSync,
} from 'node:fs';
import { dirname } from 'node:path';

export function createFileLogger(env = process.env) {
  const path = env.PROXY_LOG_FILE;
  const configured = Number(env.PROXY_LOG_MAX_BYTES);
  const total = Number.isSafeInteger(configured) && configured >= 2 ? configured : 2 * 1024 * 1024;
  const limit = Math.floor(total / 2);
  const days = Number(env.PROXY_LOG_RETENTION_DAYS);
  const retentionMs = Number.isFinite(days) && days > 0 ? days * 86400000 : 0;
  const append = (line) => {
    if (!path) return;
    const record = line + '\n';
    const bytes = Buffer.byteLength(record);
    if (bytes > limit) return;
    mkdirSync(dirname(path), { recursive: true });
    for (const file of [path, path + '.1']) {
      let metadata;
      try { metadata = statSync(file); }
      catch (error) { if (error.code === 'ENOENT') continue; throw error; }
      if (retentionMs > 0 && Date.now() - metadata.mtimeMs > retentionMs) {
        rmSync(file);
        continue;
      }
      if (metadata.size > limit) {
        // 迁移旧版无界文件：仅读末尾预算 + 1 字节，保留完整 UTF-8 行。
        const fd = openSync(file, 'r+');
        try {
          const tail = Buffer.alloc(limit + 1);
          const count = readSync(fd, tail, 0, tail.length, metadata.size - limit - 1);
          const data = tail.subarray(0, count);
          const newline = data.indexOf(10);
          const kept = newline < 0 ? data.subarray(count) : data.subarray(newline + 1);
          let written = 0;
          while (written < kept.length) written += writeSync(fd, kept, written, kept.length - written, written);
          ftruncateSync(fd, kept.length);
        } finally { closeSync(fd); }
      }
    }
    let size = 0;
    try { size = statSync(path).size; }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (size + bytes > limit) {
      rmSync(path + '.1', { force: true });
      renameSync(path, path + '.1');
    }
    appendFileSync(path, record, { mode: 0o600 });
  };
  let warned = false;
  return (line) => {
    try { append(line); }
    catch {
      // 与诊断记录一样：不输出文件路径、异常正文或请求内容，绝不影响转发。
      if (!warned) console.error('普通文件日志暂不可用；请求继续处理');
      warned = true;
    }
  };
}
