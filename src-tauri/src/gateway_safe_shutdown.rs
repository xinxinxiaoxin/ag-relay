//! 安全停机：受保护的核心排空协议 + 双进程句柄确认。绝不发送信号或强杀。
use crate::{
    gateway_connection::ConnectionError,
    gateway_process::{
        port_in_use, resolve_external_stop_target, ExternalStopTarget, GATEWAY_HOST,
    },
    gateway_status::{fetch_status_body, parse_status_pid},
};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    },
};

struct Process(HANDLE);
impl Process {
    fn open(pid: u32) -> Result<Self, ConnectionError> {
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            return Err(failed("无法取得网关进程句柄，未执行停机"));
        }
        Ok(Self(handle))
    }
    fn wait(&self, deadline: Instant) -> Result<(), ConnectionError> {
        loop {
            match unsafe { WaitForSingleObject(self.0, 25) } {
                WAIT_OBJECT_0 => return Ok(()),
                WAIT_TIMEOUT if Instant::now() < deadline => {}
                _ => {
                    return Err(failed(
                        "排空已完成，但未能确认整个网关进程链退出；未强杀，请稍后核实",
                    ))
                }
            }
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn failed(detail: &str) -> ConnectionError {
    ConnectionError::Failed(detail.into())
}

/// 成功 true = core 与 supervisor 都已退出；false = 端口空闲且无受管句柄。
/// Timeout 只指核心明确取消排空并恢复接单。网络/协议失败一律 Failed，不能当停机成功。
/// 调用者必须持有生命周期独占锁，并在任何错误时保留受管句柄。
pub fn safe_shutdown_at(
    port: u16,
    managed_supervisor: Option<u32>,
    timeout: Duration,
) -> Result<bool, ConnectionError> {
    let body = match fetch_status_body(GATEWAY_HOST, port) {
        Ok(body) => body,
        Err(_) if managed_supervisor.is_none() && !port_in_use(GATEWAY_HOST, port) => {
            return Ok(false)
        }
        Err(_) => return Err(failed("无法核实当前网关状态，未执行停机")),
    };
    let pid = parse_status_pid(&body).ok_or_else(|| failed("网关状态缺少实例身份"))?;
    let status: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| failed("网关状态无效"))?;
    if status.get("safeShutdownVersion").and_then(|v| v.as_u64()) != Some(1) {
        return Err(failed(
            "此网关不支持有界安全停机，请先手动处理旧实例；不会强制停止",
        ));
    }
    let ExternalStopTarget::Supervisor {
        pid: supervisor_pid,
    } = resolve_external_stop_target(pid)
    else {
        return Err(failed("无法核实 supervisor 父链，未执行停机"));
    };
    if managed_supervisor.is_some_and(|expected| expected != supervisor_pid) {
        return Err(failed("端口实例已变化，未执行停机"));
    }
    // 控制前打开真实句柄，避免仅凭端口消失或 PID 复用就报告整链退出。
    let core = Process::open(pid)?;
    let supervisor = Process::open(supervisor_pid)?;
    let ms = timeout.as_millis();
    if !(1..=60_000).contains(&ms) {
        return Err(failed("排空预算须为 1 到 60000 毫秒"));
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .map_err(|_| failed("无法连接网关，未确认停机"))?;
    stream
        .set_read_timeout(Some(timeout + Duration::from_secs(2)))
        .map_err(|_| failed("无法设置停机超时"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| failed("无法设置停机超时"))?;
    write!(stream, "POST /_relay/shutdown HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nx-relay-control: 1\r\nx-relay-pid: {pid}\r\nx-relay-timeout-ms: {ms}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").map_err(|_| failed("发送安全停机请求失败"))?;
    let mut raw = String::new();
    stream
        .take(131072)
        .read_to_string(&mut raw)
        .map_err(|_| failed("安全停机响应中断，未确认停机；不会强杀"))?;
    let (headers, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| failed("安全停机响应不完整"))?;
    let code = headers.split_whitespace().nth(1).unwrap_or("");
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| failed("安全停机响应无效"))?;
    if code == "408" && value.get("error").and_then(|v| v.as_str()) == Some("drain_timeout") {
        return Err(ConnectionError::Timeout);
    }
    if code != "200"
        || value.get("drained").and_then(|v| v.as_bool()) != Some(true)
        || value.get("pid").and_then(|v| v.as_u64()) != Some(pid.into())
    {
        return Err(failed("网关拒绝安全停机，实例或运行状态可能已变化"));
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    core.wait(deadline)?;
    supervisor.wait(deadline)?;
    Ok(true)
}
