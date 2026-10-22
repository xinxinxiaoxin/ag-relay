//! 安全停机：受保护的核心排空协议 + 双进程句柄确认。绝不发送信号或强杀。
use crate::{
    gateway_connection::ConnectionError,
    gateway_process::{
        port_in_use, resolve_external_stop_target, ExternalStopTarget, GATEWAY_HOST,
    },
    gateway_status::{fetch_status_body, parse_status_pid},
    shutdown_wait::ShutdownWait,
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

const WAIT_CANCELLED: &str = "已取消等待，网关已恢复接单；未强制结束请求。";
const WAIT_CLEANUP_FAILED: &str =
    "撤销等待失败，未确认网关恢复接单；未强杀，等待租约会在10秒内自动到期，请稍后核实。";
const WAIT_RESPONSE_LIMIT: usize = 128 * 1024;

// 系统级密码学随机源；不使用时间、PID 或 UI 的 operation_id 充当网络凭证。
// 直接链接 Windows 自带 bcrypt，避免引入额外 Cargo feature / 依赖。
#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(
        algorithm: *mut std::ffi::c_void,
        buffer: *mut u8,
        len: u32,
        flags: u32,
    ) -> i32;
}
fn wait_token() -> Result<String, ConnectionError> {
    let mut bytes = [0_u8; 16];
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(failed("无法生成安全等待凭证，未执行停机"));
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(32);
    for byte in bytes {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(token)
}

struct WaitResponse {
    code: u16,
    value: serde_json::Value,
}

// 状态和每次控制独立短连接。读循环另设挂钟上限，慢速分片也不能将2秒拖成10分钟。
fn wait_http(port: u16, request: &str) -> Result<WaitResponse, ConnectionError> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .map_err(|_| failed("无法连接等待控制接口，未确认停机"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| failed("无法设置等待控制超时，未确认停机"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|_| failed("发送等待控制请求失败，未确认停机"))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(failed("等待控制响应中断，未确认停机；不会强杀"));
        }
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| failed("无法设置等待控制超时，未确认停机"))?;
        let mut chunk = [0_u8; 4096];
        let count = stream
            .read(&mut chunk)
            .map_err(|_| failed("等待控制响应中断，未确认停机；不会强杀"))?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > WAIT_RESPONSE_LIMIT {
            return Err(failed("等待控制响应过大，未确认停机"));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let raw = std::str::from_utf8(&bytes).map_err(|_| failed("等待控制响应无效，未确认停机"))?;
    let (headers, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| failed("等待控制响应不完整，未确认停机"))?;
    let mut status = headers.lines().next().unwrap_or("").split_whitespace();
    if !matches!(status.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
        return Err(failed("等待控制响应无效，未确认停机"));
    }
    let code = status
        .next()
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|v| (100..=599).contains(v))
        .ok_or_else(|| failed("等待控制响应无效，未确认停机"))?;
    let mut length = None;
    for line in headers.lines().skip(1) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| failed("等待控制响应无效，未确认停机"))?;
        if name.eq_ignore_ascii_case("transfer-encoding") {
            // HTTP/1.0 要求连接结束定界，不接收未解码的 chunked 响应。
            return Err(failed("等待控制响应无效，未确认停机"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| failed("等待控制响应无效，未确认停机"))?;
            if length.replace(parsed).is_some() {
                return Err(failed("等待控制响应无效，未确认停机"));
            }
        }
    }
    if length.is_some_and(|length| length != body.len()) {
        return Err(failed("等待控制响应不完整，未确认停机"));
    }
    let value = serde_json::from_str(body).map_err(|_| failed("等待控制响应无效，未确认停机"))?;
    Ok(WaitResponse { code, value })
}

fn wait_action(
    port: u16,
    pid: u32,
    operation: &str,
    action: &str,
    ms: u128,
) -> Result<WaitResponse, ConnectionError> {
    // token 只进入本机控制头；错误/日志/UI 不输出该值。
    wait_http(port, &format!(
        "POST /_relay/drain HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nx-relay-control: 1\r\nx-relay-pid: {pid}\r\nx-relay-operation: {operation}\r\nx-relay-action: {action}\r\nx-relay-timeout-ms: {ms}\r\nx-relay-lease-ms: 10000\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    ))
}
fn wait_count(
    response: &WaitResponse,
    operation: &str,
    code: u16,
) -> Result<usize, ConnectionError> {
    if response.code != code
        || response.value.get("operation").and_then(|v| v.as_str()) != Some(operation)
        || response.value.get("draining").and_then(|v| v.as_bool()) != Some(true)
    {
        return Err(failed("网关拒绝等待控制或返回状态不符，未确认停机"));
    }
    response
        .value
        .get("activeRequests")
        .and_then(|v| v.as_u64())
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| failed("等待控制缺少有效在途请求数，未确认停机"))
}
fn cancel_wait(port: u16, pid: u32, operation: &str) -> Result<(), ConnectionError> {
    let response =
        wait_action(port, pid, operation, "cancel", 0).map_err(|_| failed(WAIT_CLEANUP_FAILED))?;
    if response.code != 200
        || response.value.get("operation").and_then(|v| v.as_str()) != Some(operation)
        || response.value.get("cancelled").and_then(|v| v.as_bool()) != Some(true)
    {
        return Err(failed(WAIT_CLEANUP_FAILED));
    }
    Ok(())
}
fn waiting_failed(port: u16, pid: u32, operation: &str, error: ConnectionError) -> ConnectionError {
    match cancel_wait(port, pid, operation) {
        Ok(()) => error,
        Err(cleanup) => failed(&format!("{error}；{cleanup}")),
    }
}
fn waiting_cancelled(port: u16, pid: u32, operation: &str, cancelled: bool) -> ConnectionError {
    match cancel_wait(port, pid, operation) {
        Ok(()) if cancelled => failed(WAIT_CANCELLED),
        Ok(()) => ConnectionError::Timeout,
        Err(error) => error,
    }
}

/// 可取消两阶段等待；只支持 drainWaitVersion=1，绝不退回旧10秒协议。
/// claim_commit 与取消同锁裁决；赢得 commit 后不再发送 cancel。
/// 成功必须收到完整同实例 commit 确认，并由真实句柄确认核心与 supervisor 均退出。
/// 调用者持有生命周期独占锁，失败保留受管 Job，也不恢复/启动实例。
pub fn safe_shutdown_wait_at(
    port: u16,
    managed_supervisor: Option<u32>,
    wait: &ShutdownWait,
) -> Result<bool, ConnectionError> {
    let response = match wait_http(
        port,
        &format!(
            "GET /_relay/status HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    ) {
        Ok(response) if response.code == 200 => response,
        Err(_) if managed_supervisor.is_none() && !port_in_use(GATEWAY_HOST, port) => {
            wait.claim_commit().map_err(|detail| failed(&detail))?;
            return Ok(false);
        }
        _ => return Err(failed("无法核实当前网关状态，未执行停机")),
    };
    let status = response.value;
    // 先验证能力，旧核心/任意本机假服务的PID绝不进入父链查询。
    if status.get("drainWaitVersion").and_then(|v| v.as_u64()) != Some(1) {
        return Err(failed(
            "此网关不支持可取消等待，请先手动处理旧实例；不会强制停止",
        ));
    }
    let pid = status
        .get("pid")
        .and_then(|v| v.as_u64())
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0)
        .ok_or_else(|| failed("网关状态缺少实例身份"))?;
    let ExternalStopTarget::Supervisor {
        pid: supervisor_pid,
    } = resolve_external_stop_target(pid)
    else {
        return Err(failed("无法核实 supervisor 父链，未执行停机"));
    };
    if managed_supervisor.is_some_and(|expected| expected != supervisor_pid) {
        return Err(failed("端口实例已变化，未执行停机"));
    }
    let core = Process::open(pid)?;
    let supervisor = Process::open(supervisor_pid)?;
    let ms = wait.timeout().as_millis();
    if !(1..=600_000).contains(&ms) {
        return Err(failed("等待预算须为 1 到 600000 毫秒"));
    }
    let operation = wait_token()?;
    // start之后所有错误都尝试明确撤销；网络中断不能假定服务端未受理。
    let response = match wait_action(port, pid, &operation, "start", ms) {
        Ok(response) if response.code != 202 => {
            return Err(failed("网关拒绝开始等待，实例或运行状态可能已变化"));
        }
        Ok(response) => response,
        Err(error) => return Err(waiting_failed(port, pid, &operation, error)),
    };
    let mut count = match wait_count(&response, &operation, 202) {
        Ok(count) => count,
        Err(error) => return Err(waiting_failed(port, pid, &operation, error)),
    };
    loop {
        wait.update_remaining(count);
        if wait.cancelled() || wait.remaining().is_zero() {
            return Err(waiting_cancelled(port, pid, &operation, wait.cancelled()));
        }
        if count == 0 {
            if let Err(error) = wait.claim_commit() {
                if wait.cancelled() || wait.remaining().is_zero() {
                    return Err(waiting_cancelled(port, pid, &operation, wait.cancelled()));
                }
                return Err(waiting_failed(port, pid, &operation, failed(&error)));
            }
            // 取消已经输给 commit；即使网络结果未知也不可改发 cancel。
            let response = wait_action(port, pid, &operation, "commit", 0)?;
            if response.code != 200
                || response.value.get("operation").and_then(|v| v.as_str())
                    != Some(operation.as_str())
                || response.value.get("drained").and_then(|v| v.as_bool()) != Some(true)
                || response.value.get("pid").and_then(|v| v.as_u64()) != Some(pid.into())
            {
                return Err(failed(
                    "网关拒绝确认停机或返回实例不符，未确认停机；不会强杀",
                ));
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            core.wait(deadline)?;
            supervisor.wait(deadline)?;
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(200).min(wait.remaining()));
        // 不在已取消/耗尽预算后 refresh 续约。
        if wait.cancelled() || wait.remaining().is_zero() {
            continue;
        }
        let response = match wait_action(port, pid, &operation, "refresh", 0) {
            Ok(response) => response,
            Err(error) => return Err(waiting_failed(port, pid, &operation, error)),
        };
        count = match wait_count(&response, &operation, 200) {
            Ok(count) => count,
            Err(error) => return Err(waiting_failed(port, pid, &operation, error)),
        };
    }
}
