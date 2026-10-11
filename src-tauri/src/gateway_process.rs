//! 网关进程生命周期管理（Windows 专用，本项目自用平台）。
//!
//! 启动链（配方见共享笔记 tauri-2-recipes.md §4，含 2026-10-11 实测更正）：
//!   spawn 自带 node.exe relay-supervisor.mjs（cwd=网关资源目录，CREATE_NO_WINDOW）。
//!   ⚠ 绝不用 CREATE_NEW_PROCESS_GROUP——它会隐式禁用 Ctrl+C，毁掉优雅关停。
//!   → 子进程挂进 KILL_ON_JOB_CLOSE 的 Job Object（壳崩溃则网关整树必死）。
//! 停止链：AttachConsole(supervisor_pid) → 忽略自身 CTRL_C → 广播 CTRL_C_EVENT
//!   → 等待优雅退出（≤10s）→ 超时 TerminateJobObject 强杀整树。全程幂等。
//! ⚠ 实测更正（配方笔记未覆盖）：AttachConsole 必须在 FreeConsole 之后的数百毫秒内
//!   重试才能成功（free 后立即 attach 稳定报 err=6 INVALID_HANDLE），故此处带重试。

use std::fmt;
use std::net::TcpListener;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Console::{
    AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler, CTRL_C_EVENT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use std::os::windows::io::AsRawHandle;

use crate::gateway_env::gateway_env_pairs;

/// 无窗口创建标志（MSDN CREATE_NO_WINDOW）。
/// 实测：子进程拥有一个隐藏控制台（可 AttachConsole），CTRL_C 广播可达。
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 网关监听端口（与 gateway_env 表中 PROXY_PORT=8787 一致，测试断言两者同步）。
pub const GATEWAY_PORT: u16 = 8787;
pub const GATEWAY_HOST: &str = "127.0.0.1";

/// 优雅关停默认预算（工单：最多等 10 秒后强杀）。
pub const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// 广播 CTRL_C 前的启动缓冲。node 需要数百毫秒引导后才注册 SIGINT 处理器，
/// 过早广播会命中引导期的默认处置：进程以 STATUS_CONTROL_C_EXIT（0xC000013A）裸死，
/// 完全不优雅（高负载下 node 引导变慢，T5 基线实测桩测试间歇复现）。
/// 缓冲期间子进程若自行退出则按优雅提前返回。对常驻网关只是把广播推迟约一瞬。
const BOOT_GRACE: Duration = Duration::from_millis(1_500);

/// 串行化整个进程生命周期序列（spawn 与 stop）。原因：
/// 1) 「忽略 CTRL_C」属性是进程级且会被子进程继承，spawn 前要清、广播时要设、
///    广播后不能立刻清（CTRL_C 派发有毫秒级延迟，并行交叠会自伤或让子进程忽略信号）；
/// 2) 广播的 CTRL_C 会到达本进程自己。
/// 生产中同一时刻至多一个网关，串行无副作用（stop 最长 10 秒）。
static LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

/// 网关相关路径（全部运行期解析，不硬编码安装位置）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayPaths {
    /// 自带 node.exe（externalBin 剥 triple 后落在主程序 exe 旁）
    pub node_exe: PathBuf,
    /// 网关脚本目录（resources 落盘，exe 旁 resources\gateway\，三个 .mjs 必须同处一目录）
    pub gateway_dir: PathBuf,
    /// 诊断日志：%APPDATA%\ag-relay\logs\relay-diagnostics.jsonl
    pub diagnostics_file: PathBuf,
}

/// 由 exe 所在目录与应用数据目录解析网关路径。
pub fn resolve_paths(exe_dir: &Path, app_data_dir: &Path) -> GatewayPaths {
    GatewayPaths {
        node_exe: exe_dir.join("node.exe"),
        gateway_dir: exe_dir.join("resources").join("gateway"),
        diagnostics_file: app_data_dir.join("logs").join("relay-diagnostics.jsonl"),
    }
}

#[derive(Debug)]
pub enum GatewayError {
    NodeNotFound(PathBuf),
    SupervisorScriptNotFound(PathBuf),
    PortAlreadyInUse { host: String, port: u16 },
    SpawnFailed(String),
    JobSetupFailed(&'static str),
}

impl fmt::Display for GatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GatewayError::NodeNotFound(p) => write!(f, "自带 node 运行时缺失：{}", p.display()),
            GatewayError::SupervisorScriptNotFound(p) => {
                write!(f, "网关脚本缺失：{}", p.display())
            }
            GatewayError::PortAlreadyInUse { host, port } => {
                write!(f, "端口 {host}:{port} 已被占用，网关可能已在运行，为避免重复拉起未执行启动")
            }
            GatewayError::SpawnFailed(why) => write!(f, "拉起网关进程失败：{why}"),
            GatewayError::JobSetupFailed(why) => {
                write!(f, "进程树兜底（Job Object）建立失败：{why}")
            }
        }
    }
}

impl std::error::Error for GatewayError {}

/// Job Object 句柄 newtype：裸指针本身非 Send/Sync，但句柄只是整数，
/// 所有 Win32 调用都发生在 LIFECYCLE_LOCK 串行区内，跨线程移动/共享是安全的。
pub struct JobHandle(pub(crate) HANDLE);
unsafe impl Send for JobHandle {}
unsafe impl Sync for JobHandle {}

/// 受管子进程：壳持有 Child 与 Job Object 句柄。
/// 刻意不实现「Drop 杀进程」（std Child 默认不 kill on drop）——
/// 生命周期只由 stop_managed / KILL_ON_JOB_CLOSE 决定。
pub struct ManagedChild {
    pub(crate) child: Child,
    /// supervisor 进程 PID（CTRL_C 广播 attach 的目标）
    pub supervisor_pid: u32,
    /// KILL_ON_JOB_CLOSE 的 Job Object 句柄；存活至本进程退出（内核兜底杀整树）
    pub(crate) job: JobHandle,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StopOutcome {
    /// 优雅退出（supervisor 排空后退出；75=热重启排空，0=关停排空，均属优雅）
    Graceful { exit_code: Option<i32> },
    /// 优雅窗口超时，TerminateJobObject 强杀整树
    ForceKilled,
}

fn create_kill_on_close_job() -> Result<HANDLE, &'static str> {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err("CreateJobObjectW 失败");
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            return Err("SetInformationJobObject 失败");
        }
        Ok(job)
    }
}

/// 通用受管 spawn：node 运行脚本，CREATE_NO_WINDOW，挂 KILL_ON_JOB_CLOSE Job。
/// extra_env 在继承环境之上追加（网关用 17 项配置 + 诊断路径）。
pub fn spawn_managed(
    node_exe: &Path,
    script: &Path,
    cwd: &Path,
    extra_env: &[(String, String)],
) -> Result<ManagedChild, GatewayError> {
    if !node_exe.is_file() {
        return Err(GatewayError::NodeNotFound(node_exe.to_path_buf()));
    }
    if !script.is_file() {
        return Err(GatewayError::SupervisorScriptNotFound(script.to_path_buf()));
    }

    let _lifecycle = LIFECYCLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut cmd = Command::new(node_exe);
    cmd.arg(script).current_dir(cwd);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    // 只用 CREATE_NO_WINDOW；绝不用 CREATE_NEW_PROCESS_GROUP（隐式禁 Ctrl+C）
    cmd.creation_flags(CREATE_NO_WINDOW);

    // ⚠ 实测关键：「忽略 CTRL_C」属性会沿进程树继承。若壳本身运行在设置了该属性的
    // 环境（自动化工具/部分终端链路），子进程会继承它导致 CTRL_C 广播被静默忽略。
    // spawn 前先清除本进程的忽略属性，保证子进程继承到可响应 CTRL_C 的干净状态。
    unsafe {
        SetConsoleCtrlHandler(None, 0);
    }

    let mut child = cmd.spawn().map_err(|e| GatewayError::SpawnFailed(e.to_string()))?;
    let supervisor_pid = child.id();

    let job = create_kill_on_close_job().map_err(|why| {
        let _ = child.kill();
        let _ = child.wait();
        GatewayError::JobSetupFailed(why)
    })?;
    unsafe {
        if AssignProcessToJobObject(job, child.as_raw_handle()) == 0 {
            // 杀不掉就不留孤儿：TerminateJobObject + 关 job 句柄（KILL_ON_JOB_CLOSE 双保险）
            TerminateJobObject(job, 1);
            let _ = child.wait();
            return Err(GatewayError::JobSetupFailed("AssignProcessToJobObject 失败"));
        }
    }

    Ok(ManagedChild {
        child,
        supervisor_pid,
        job: JobHandle(job),
    })
}

/// 把 CTRL_C 广播到目标子进程的（隐藏）控制台：
/// supervisor 与网关都注册了 SIGINT → 优雅 drain。壳自己注册空处理器忽略该信号。
///
/// 实测坑：FreeConsole 之后立即 AttachConsole 会偶发失败（err=6 INVALID_HANDLE），
/// 需要间隔重试（conhost 侧收尾有延迟）——以下用 50ms 间隔、上限 2 秒的重试。
/// 调用方必须已持有 LIFECYCLE_LOCK。
fn broadcast_ctrl_c_to_child_console(child_pid: u32) {
    unsafe {
        let mut attached = AttachConsole(child_pid) != 0;
        if !attached {
            // 已有控制台（GUI 壳平时没有；dev/test 从终端启动时有）先脱离再挂子进程的
            FreeConsole();
            for _ in 0..40 {
                if AttachConsole(child_pid) != 0 {
                    attached = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        if attached {
            // HandlerRoutine=NULL + Add=TRUE ⇒ 本进程忽略 CTRL_C，避免被一并终止
            SetConsoleCtrlHandler(None, 1);
            // CTRL_C 不能定向进程组，只能 0=同控制台全体进程（我们已挂到子进程控制台）
            GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
            FreeConsole();
            // 注意：忽略属性保持到本次 stop 全程结束再解除（在 stop_managed 收尾），
            // 因为 CTRL_C 的派发有延迟，立即解除可能被迟到的派发命中。
        }
    }
}

/// 停止受管子进程：先 CTRL_C 优雅（等 ≤timeout），超时 TerminateJobObject 强杀整树。
/// 对已退出的进程调用是安全的（返回其退出码或直接强杀兜底）。
pub fn stop_managed(proc: &mut ManagedChild, timeout: Duration) -> StopOutcome {
    let _lifecycle = LIFECYCLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let outcome = stop_managed_locked(proc, timeout);
    // 广播期间设置的「忽略 CTRL_C」到此解除（等待期已覆盖派发延迟）
    unsafe {
        SetConsoleCtrlHandler(None, 0);
    }
    outcome
}

fn stop_managed_locked(proc: &mut ManagedChild, timeout: Duration) -> StopOutcome {
    // 幂等：已退出直接返回
    if let Ok(Some(status)) = proc.child.try_wait() {
        return StopOutcome::Graceful { exit_code: status.code() };
    }

    // 启动缓冲：等子进程完成引导、注册好 SIGINT 处理器再广播（见 BOOT_GRACE 注释）。
    // 期间自行退出（含刚被外部杀掉）都按优雅处理，提前返回。
    let grace_deadline = Instant::now() + BOOT_GRACE;
    while Instant::now() < grace_deadline {
        match proc.child.try_wait() {
            Ok(Some(status)) => return StopOutcome::Graceful { exit_code: status.code() },
            Ok(None) => {}
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    broadcast_ctrl_c_to_child_console(proc.supervisor_pid);

    let deadline = Instant::now() + timeout;
    loop {
        match proc.child.try_wait() {
            Ok(Some(status)) => {
                return StopOutcome::Graceful { exit_code: status.code() }
            }
            Ok(None) => {}
            Err(_) => break, // 句柄异常时交给强杀兜底
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    unsafe {
        TerminateJobObject(proc.job.0, 1);
    }
    let _ = proc.child.wait();
    StopOutcome::ForceKilled
}

/// 8787 端口探测：被占用（含已有网关实例）返回 true。
pub fn port_in_use(host: &str, port: u16) -> bool {
    TcpListener::bind((host, port)).is_err()
}

/// 启动网关：端口被占则明确报错（不重复拉起）；否则拉起 supervisor。
pub fn start_gateway_process(paths: &GatewayPaths) -> Result<ManagedChild, GatewayError> {
    if port_in_use(GATEWAY_HOST, GATEWAY_PORT) {
        return Err(GatewayError::PortAlreadyInUse {
            host: GATEWAY_HOST.to_string(),
            port: GATEWAY_PORT,
        });
    }
    let supervisor_script = paths.gateway_dir.join("relay-supervisor.mjs");
    let extra_env = gateway_env_pairs(&paths.diagnostics_file);
    spawn_managed(&paths.node_exe, &supervisor_script, &paths.gateway_dir, &extra_env)
}

// ---------------------------------------------------------------------------
// /_relay/status 只读客户端（本地控制端点，壳复用不新增）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayConfig {
    pub status_interval_ms: u64,
    pub stream_idle_ms: u64,
    pub stream_progress_ms: u64,
    pub connect_timeout_ms: u64,
    pub total_budget_ms: u64,
    pub max_attempts: u64,
    /// status 输出里是布尔（实测：未设 PROXY_SESSION_COMPAT 时默认也为 true）
    pub session_compat: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayStatus {
    pub loaded_version: String,
    pub pid: u32,
    pub supervisor_enabled: bool,
    pub restarting: bool,
    pub draining: bool,
    pub active_requests: u64,
    pub config: RelayConfig,
}

/// GET http://127.0.0.1:{port}/_relay/status（网关本地控制端点要求 Host 头精确匹配端口）。
/// HTTP 细节（HTTP/1.0 + 仅 Host 头、超时）统一在 gateway_status::fetch_status_body。
pub fn get_relay_status(port: u16) -> Result<RelayStatus, String> {
    let body = crate::gateway_status::fetch_status_body(GATEWAY_HOST, port)?;
    serde_json::from_str(body.as_str()).map_err(|e| format!("解析状态 JSON 失败：{e}"))
}

// ---------------------------------------------------------------------------
// 测试：桩进程生命周期 + 路径解析 + 端口占用
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway_env::gateway_env_table;
    use std::net::TcpStream;
    use std::sync::Mutex as TestMutex;

    /// 触碰 8787 端口与全局控制台的测试串行执行
    static PORT_TESTS_LOCK: TestMutex<()> = TestMutex::new(());

    /// 等 8787 空闲再动它。PORT_TESTS_LOCK 只串行本测试进程——同一台机器上
    /// 并行工单的测试（各自 worktree）也会占用 8787，瞬时争用属正常排队，
    /// 最多等 120 秒；超时仍被占才按「环境不干净（残留网关）」失败。
    fn wait_port_free() {
        let deadline = Instant::now() + Duration::from_secs(120);
        while port_in_use(GATEWAY_HOST, GATEWAY_PORT) {
            assert!(
                Instant::now() < deadline,
                "8787 持续被占用超过 120 秒，测试环境不干净（可能有残留网关）"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// 占住 8787 给「端口被占」测试用：等待与绑定之间仍可能被并行测试抢先，
    /// 故 bind 失败就重试（同一 120 秒预算），把瞬时争用变成排队。
    fn occupy_port_for_test() -> TcpListener {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            match TcpListener::bind((GATEWAY_HOST, GATEWAY_PORT)) {
                Ok(listener) => return listener,
                Err(_) => {
                    assert!(
                        Instant::now() < deadline,
                        "8787 持续被占用超过 120 秒，测试环境不干净（可能有残留网关）"
                    );
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }

    /// 持 LIFECYCLE_LOCK 后执行 stop 并返回（耗时不含与并行测试排队等锁的时间）。
    /// 等价于 stop_managed，只是把「持锁」与「计时」的责任留给调用方——
    /// 直接用 stop_managed 计时会把锁排队时间算进优雅关停耗时（桩2 的强杀窗口
    /// 持锁约 10 秒，桩1 满载下排队 + 0.7 秒优雅退出 > 10 秒断言，误报）。
    fn timed_stop_locked(proc: &mut ManagedChild, timeout: Duration) -> (StopOutcome, Duration) {
        let lifecycle = LIFECYCLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let started = Instant::now();
        let outcome = stop_managed_locked(proc, timeout);
        let elapsed = started.elapsed();
        drop(lifecycle);
        // 与 stop_managed 收尾一致：解除广播期间设置的「忽略 CTRL_C」属性
        unsafe {
            SetConsoleCtrlHandler(None, 0);
        }
        (outcome, elapsed)
    }

    fn manifest_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn sidecar_node() -> PathBuf {
        manifest_dir().join("binaries").join("node-x86_64-pc-windows-msvc.exe")
    }

    fn fixtures_dir() -> PathBuf {
        manifest_dir().join("tests").join("fixtures")
    }

    /// 领一个空闲端口给桩当就绪信号用（先占后放，桩会立刻绑定）。
    fn free_port() -> u16 {
        let listener = TcpListener::bind((GATEWAY_HOST, 0)).expect("bind 随机端口");
        listener.local_addr().expect("local_addr").port()
    }

    /// 等桩就绪：以「桩监听的 STUB_READY_PORT 可连接」为信号。
    /// 端口可连 = node 引导完成、用户 JS 已运行、SIGINT 控制台处理器已注册。
    /// 不等它就广播 CTRL_C 会在高负载下命中 node 引导窗口，桩以
    /// STATUS_CONTROL_C_EXIT 默认终止（实测：全量并行测试时稳定复现的竞态）。
    fn wait_stub_ready(port: u16) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if TcpStream::connect((GATEWAY_HOST, port)).is_ok() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "桩 10 秒内未就绪（STUB_READY_PORT={port}）"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// spawn 一个带就绪信号的桩并等到它真正可用。
    fn spawn_ready_stub(script: &Path) -> ManagedChild {
        let ready_port = free_port();
        let env = vec![("STUB_READY_PORT".to_string(), ready_port.to_string())];
        let proc = spawn_managed(&sidecar_node(), script, &fixtures_dir(), &env)
            .expect("spawn 桩");
        wait_stub_ready(ready_port);
        proc
    }

    #[test]
    fn 路径解析_node在exe旁_网关脚本在resources下_诊断日志在应用数据logs() {
        let paths = resolve_paths(Path::new("C:\\安装目录"), Path::new("C:\\Users\\x\\AppData\\Roaming\\ag-relay"));
        assert_eq!(paths.node_exe, PathBuf::from("C:\\安装目录\\node.exe"));
        assert_eq!(paths.gateway_dir, PathBuf::from("C:\\安装目录\\resources\\gateway"));
        assert_eq!(
            paths.diagnostics_file,
            PathBuf::from("C:\\Users\\x\\AppData\\Roaming\\ag-relay\\logs\\relay-diagnostics.jsonl")
        );
    }

    #[test]
    fn 端口常量与环境变量表保持一致() {
        let table = gateway_env_table();
        let (_, port_value) = table.iter().find(|(k, _)| *k == "PROXY_PORT").expect("表里有 PROXY_PORT");
        assert_eq!(port_value.parse::<u16>().unwrap(), GATEWAY_PORT);
        let (_, host_value) = table.iter().find(|(k, _)| *k == "PROXY_HOST").expect("表里有 PROXY_HOST");
        assert_eq!(*host_value, GATEWAY_HOST);
    }

    #[test]
    fn 桩1_收到CTRL_C应优雅退出_耗时远小于10秒上限() {
        // CTRL_C 广播会送达同控制台的所有子进程，桩测试必须串行（本锁是 T2 既定设计）
        let _guard = PORT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let script = fixtures_dir().join("stub_graceful.mjs");
        let mut proc = spawn_ready_stub(&script);
        let (outcome, elapsed) = timed_stop_locked(&mut proc, STOP_TIMEOUT);

        match outcome {
            StopOutcome::Graceful { exit_code } => {
                assert_eq!(exit_code, Some(0), "桩1 应以退出码 0 优雅退出");
            }
            other => panic!("桩1 应走优雅路径，实际 {:?}", other),
        }
        assert!(
            elapsed < Duration::from_secs(10),
            "优雅关停必须在 10 秒上限内完成，实际 {:?}",
            elapsed
        );
    }

    #[test]
    fn 桩2_忽略CTRL_C_须在10秒上限后被强杀且进程消失() {
        // 同桩1：广播影响同控制台全部子进程，必须串行
        let _guard = PORT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let script = fixtures_dir().join("stub_stubborn.mjs");
        let mut proc = spawn_ready_stub(&script);
        let (outcome, elapsed) = timed_stop_locked(&mut proc, STOP_TIMEOUT);

        assert!(matches!(outcome, StopOutcome::ForceKilled), "桩2 忽略 CTRL_C，应被强杀，实际 {:?}", outcome);
        assert!(
            elapsed >= Duration::from_secs(9),
            "强杀前应给满优雅窗口（≥9s），实际 {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_secs(20),
            "强杀兜底应在超时后立刻发生，实际 {:?}",
            elapsed
        );
        assert!(
            proc.child.try_wait().expect("try_wait").is_some(),
            "强杀后桩2 必须已退出（进程消失）"
        );
    }

    #[test]
    fn 启动_端口已被占用时返回明确错误且不拉起() {
        let _guard = PORT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let manifest = manifest_dir();
        let paths = GatewayPaths {
            node_exe: sidecar_node(),
            gateway_dir: manifest.join("resources").join("gateway"),
            diagnostics_file: std::env::temp_dir().join("ag-relay-port-test").join("diag.jsonl"),
        };
        // 占住 8787（含等并行测试放口 + bind 重试）
        let listener = occupy_port_for_test();
        let result = start_gateway_process(&paths);
        drop(listener);
        match result {
            Err(GatewayError::PortAlreadyInUse { host, port }) => {
                assert_eq!((host.as_str(), port), (GATEWAY_HOST, GATEWAY_PORT));
            }
            other => panic!("端口被占时应返回 PortAlreadyInUse，实际 {:?}", other.map(|_| ())),
        }
    }

    /// 工单给定的独立真值：28b764d 版 relay-retry-proxy.mjs 的 sha256
    /// （网关 /_relay/status 的 loadedVersion 就是该脚本自身的 sha256 全文）。
    const EXPECTED_GATEWAY_SHA256: &str =
        "e8395e0f90fe46bf1059bfcbac77e65d887d09d4ed5057490f85847e5cde85dc";

    #[test]
    fn 真网关冒烟_起停_版本与配置断言_端口释放_诊断日志落位() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let _guard = PORT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let manifest = manifest_dir();
        let diag = std::env::temp_dir()
            .join(format!("ag-relay-smoke-{}", std::process::id()))
            .join("logs")
            .join("relay-diagnostics.jsonl");
        let paths = GatewayPaths {
            node_exe: sidecar_node(),
            gateway_dir: manifest.join("resources").join("gateway"),
            diagnostics_file: diag.clone(),
        };

        wait_port_free();
        // 启动真网关：wait 与 start 之间仍可能被并行测试抢走 8787，PortAlreadyInUse 就重试
        let smoke_deadline = Instant::now() + Duration::from_secs(120);
        let mut gw = loop {
            match start_gateway_process(&paths) {
                Ok(gw) => break gw,
                Err(GatewayError::PortAlreadyInUse { .. }) => {
                    assert!(
                        Instant::now() < smoke_deadline,
                        "8787 被并行测试反复抢占，冒烟无法开始"
                    );
                    std::thread::sleep(Duration::from_millis(500));
                    wait_port_free();
                }
                Err(e) => panic!("启动真网关失败：{e:?}"),
            }
        };

        // 断言失败也要把网关停掉，避免污染后续测试
        let result = catch_unwind(AssertUnwindSafe(|| {
            // 轮询 /_relay/status 直到就绪（60 秒上限；status 有 20s 缓存期，不设短超时）
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut status = None;
            while Instant::now() < deadline {
                if let Ok(s) = get_relay_status(GATEWAY_PORT) {
                    status = Some(s);
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            let status = status.expect("60 秒内 /_relay/status 未就绪");

            // loadedVersion = 打包脚本自身的 sha256 → 证明拉起的是自带脚本（28b764d 版）
            assert_eq!(status.loaded_version, EXPECTED_GATEWAY_SHA256);
            // supervisor 链路成立（gateway 由 supervisor fork，PROXY_SUPERVISED 由它自己注入）
            assert!(status.supervisor_enabled, "网关应处于 supervisor 托管模式");
            // status.config 暴露的配置项与环境变量表逐项对上（端到端证明环境变量生效；
            // max_attempts 默认值是 5，110 只能来自 PROXY_MAX_ATTEMPTS）
            assert_eq!(status.config.max_attempts, 110, "PROXY_MAX_ATTEMPTS 未生效");
            assert_eq!(status.config.total_budget_ms, 300_000, "PROXY_TOTAL_BUDGET_MS 未生效");
            assert_eq!(status.config.connect_timeout_ms, 150_000, "PROXY_CONNECT_TIMEOUT_MS 未生效");
            assert_eq!(status.config.status_interval_ms, 20_000, "PROXY_STATUS_INTERVAL_MS 未生效");
            assert_eq!(status.config.stream_idle_ms, 90_000, "PROXY_STREAM_IDLE_MS 未生效");
            assert_eq!(status.config.stream_progress_ms, 180_000, "PROXY_STREAM_PROGRESS_MS 未生效");
            assert_eq!(status.config.session_compat, true, "PROXY_SESSION_COMPAT 未生效");
        }));

        // 停止（命令层同一路径）
        let outcome = stop_managed(&mut gw, STOP_TIMEOUT);
        match outcome {
            StopOutcome::Graceful { exit_code } => {
                assert_eq!(exit_code, Some(0), "真网关应优雅退出（退出码 0）");
            }
            other => panic!("真网关应走优雅路径，实际 {:?}", other),
        }

        // 端口释放（gateway 进程已死的最直接证据）
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !port_in_use(GATEWAY_HOST, GATEWAY_PORT),
            "停止后 8787 端口应释放（无残留网关进程）"
        );
        // 诊断日志写入指定路径（PROXY_DIAGNOSTICS_FILE 生效，而非脚本目录默认值）
        assert!(diag.is_file(), "诊断日志应写入 PROXY_DIAGNOSTICS_FILE 指定路径：{}", diag.display());

        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }
}
