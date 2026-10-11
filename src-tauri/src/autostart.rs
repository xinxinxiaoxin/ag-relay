//! 开机自启管理（T6）。

use std::fmt;
use std::path::Path;

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

/// 注册表操作失败（包装 io 错误信息）。
#[derive(Debug)]
pub enum AutostartError {
    Registry(String),
}

impl fmt::Display for AutostartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AutostartError::Registry(why) => write!(f, "注册表操作失败：{why}"),
        }
    }
}

impl std::error::Error for AutostartError {}

impl From<std::io::Error> for AutostartError {
    fn from(e: std::io::Error) -> Self {
        AutostartError::Registry(e.to_string())
    }
}

/// 判定一个快捷方式是否指向本网关（relay-retry-proxy 的 start.bat）。
/// 规则：目标路径与参数拼接后（大小写不敏感）同时包含两个特征
/// `relay-retry-proxy` 与 `start.bat`。
/// 必须看 TargetPath + Arguments 两处：真机旧 lnk 的 TargetPath 是 cmd.exe，
/// 特征出现在调用参数里（2026-10-11 侦查实证）。
/// 两个特征缺一不可：同名但指向别处的快捷方式一律保留。
pub fn is_our_gateway_shortcut(target: &str, arguments: &str) -> bool {
    let hay = format!("{target} {arguments}").to_lowercase();
    hay.contains("relay-retry-proxy") && hay.contains("start.bat")
}

// ---------------------------------------------------------------------------
// 旧自启快捷方式（启动文件夹 .lnk）的解析与处置
// ---------------------------------------------------------------------------

/// 启动文件夹里旧「直接启动网关」快捷方式的文件名（工单侦查实证）。
pub const LEGACY_LNK_NAME: &str = "Relay Retry Gateway.lnk";

/// 快捷方式解析结果（PowerShell WScript.Shell 只读读取）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LnkInfo {
    pub target: String,
    pub arguments: String,
}

/// 处置结果（进入提示区的素材）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyLnkOutcome {
    /// 启动目录没有旧 lnk（首次清理后的常态）
    NotFound,
    /// 确认指向本网关，已删除
    Removed { target: String },
    /// 同名但指向别处，绝不误删，保留并提示
    KeptForeign { target: String },
    /// 解析失败（无法确认归属时一律保留）
    Failed { error: String },
}

impl AutostartError {
    fn ps(why: String) -> Self {
        AutostartError::Registry(why) // PowerShell 环节与注册表同归「环境操作失败」
    }
}

/// base64（标准字母表，无换行）——给 PowerShell -EncodedCommand 用。
fn base64_utf16le(s: &str) -> String {
    let bytes: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// 无窗口跑一段 PowerShell 脚本，返回 stdout（带超时兜底，防 COM 卡死拖住启动）。
/// 输出强制 UTF-8：默认 redirected stdout 是 OEM 代码页（中文系统 GBK），
/// lnk 目标里的非 ASCII（如「程序」目录）会变乱码进入提示区。
fn run_powershell(script: &str) -> Result<String, AutostartError> {
    use std::os::windows::process::CommandExt;
    use std::time::{Duration, Instant};

    let full_script = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; {script}"
    );
    let mut child = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &base64_utf16le(&full_script)])
        .creation_flags(crate::gateway_process::CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| AutostartError::ps(format!("启动 PowerShell 失败：{e}")))?;

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().map_err(|e| {
                    AutostartError::ps(format!("读取 PowerShell 输出失败：{e}"))
                })?;
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                if status.success() {
                    return Ok(stdout);
                }
                return Err(AutostartError::ps(format!(
                    "PowerShell 退出码 {:?}：{}",
                    status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            Ok(None) => {}
            Err(e) => return Err(AutostartError::ps(format!("等待 PowerShell 失败：{e}"))),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AutostartError::ps("PowerShell 执行超时（20 秒）".to_string()));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// 只读解析 .lnk 的目标与参数（WScript.Shell COM；仅读，不 Save，不落任何盘）。
pub fn resolve_lnk(lnk: &Path) -> Result<LnkInfo, AutostartError> {
    let path = lnk.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; $s=(New-Object -ComObject WScript.Shell).CreateShortcut('{path}'); @{{t=[string]$s.TargetPath;a=[string]$s.Arguments}}|ConvertTo-Json -Compress"
    );
    let stdout = run_powershell(&script)?;
    #[derive(serde::Deserialize)]
    struct Raw {
        t: String,
        a: String,
    }
    let raw: Raw = serde_json::from_str(stdout.trim())
        .map_err(|e| AutostartError::ps(format!("解析 lnk 输出失败：{e}；原文：{stdout:.120}")))?;
    Ok(LnkInfo { target: raw.t, arguments: raw.a })
}

/// 直接删除文件（PowerShell Remove-Item；不走回收站）。
fn delete_lnk(lnk: &Path) -> Result<(), AutostartError> {
    let path = lnk.to_string_lossy().replace('\'', "''");
    let script = format!("$ErrorActionPreference='Stop'; Remove-Item -LiteralPath '{path}' -Force");
    run_powershell(&script).map(|_| ())
}

/// 当前用户启动文件夹：%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup。
pub fn user_startup_dir() -> Result<std::path::PathBuf, AutostartError> {
    let appdata = std::env::var("APPDATA")
        .map_err(|e| AutostartError::ps(format!("读取 APPDATA 环境变量失败：{e}")))?;
    Ok(std::path::PathBuf::from(appdata)
        .join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
}

/// 处置启动文件夹里的旧自启快捷方式（幂等，可每次启动调用）：
/// - 不存在 → NotFound（常态）；
/// - 解析目标，确认指向本网关（含 relay-retry-proxy + start.bat 两个特征）→ 删除；
/// - 同名但指向别处 / 解析失败 → 一律保留（宁可漏删，不可误删）。
/// startup_dir 为参数以便测试注入临时目录；生产传 user_startup_dir()。
pub fn handle_legacy_lnk(startup_dir: &Path, lnk_name: &str) -> LegacyLnkOutcome {
    let lnk = startup_dir.join(lnk_name);
    if !lnk.is_file() {
        return LegacyLnkOutcome::NotFound;
    }
    let info = match resolve_lnk(&lnk) {
        Ok(i) => i,
        Err(e) => {
            return LegacyLnkOutcome::Failed { error: e.to_string() };
        }
    };
    if !is_our_gateway_shortcut(&info.target, &info.arguments) {
        return LegacyLnkOutcome::KeptForeign { target: full_command(&info) };
    }
    match delete_lnk(&lnk) {
        Ok(()) => LegacyLnkOutcome::Removed { target: full_command(&info) },
        Err(e) => LegacyLnkOutcome::Failed { error: format!("删除失败：{e}") },
    }
}

/// 「目标 + 参数」拼成可读的完整命令（提示区展示用；TargetPath 是 cmd.exe 时参数才是关键特征）。
fn full_command(info: &LnkInfo) -> String {
    if info.arguments.is_empty() {
        info.target.clone()
    } else {
        format!("{} {}", info.target, info.arguments)
    }
}

// ---------------------------------------------------------------------------
// 应用启动编排与手动开关
// ---------------------------------------------------------------------------

/// 每次应用启动的编排结果：enabled = 当前自启是否生效；notes = 给提示区的中文消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSyncReport {
    pub enabled: bool,
    pub notes: Vec<String>,
}

/// 应用启动时调用（幂等）：
/// 1. 用户手动关闭过（opt-out 标记）→ 保持关闭，清掉可能的残留自启项；
/// 2. 否则幂等确保自启项存在且指向当前 exe（首次建立 / 路径变了改写 / 无变化不动）；
/// 3. 处置启动文件夹里的旧「直接启动网关」快捷方式（只删确认属于本网关的）。
/// 刻意不返回错误：自启是锦上添花的功能，任何失败只进提示区，绝不挡应用启动。
pub fn sync_on_launch(
    store: &AutostartStore,
    startup_dir: &Path,
    lnk_name: &str,
    exe: &Path,
) -> LaunchSyncReport {
    let mut notes = Vec::new();

    let opted_out = match store.opt_out() {
        Ok(v) => v,
        Err(e) => {
            notes.push(format!("开机自启状态读取失败：{e}"));
            return LaunchSyncReport { enabled: false, notes };
        }
    };

    if opted_out {
        match store.remove_run_entry() {
            Ok(true) => notes.push("开机自启：保持关闭（此前手动关闭，已清理残留自启项）".to_string()),
            Ok(false) => {}
            Err(e) => notes.push(format!("开机自启清理残留项失败：{e}")),
        }
        return LaunchSyncReport { enabled: false, notes };
    }

    match store.ensure(exe) {
        Ok(EnsureOutcome::Created) => {
            notes.push("已建立开机自启项（当前用户注册表，指向本程序）".to_string())
        }
        Ok(EnsureOutcome::PathUpdated { old_path }) => notes.push(format!(
            "开机自启路径已更新：{old_path} → 当前程序位置"
        )),
        Ok(EnsureOutcome::Unchanged) => {}
        Err(e) => notes.push(format!("建立开机自启项失败：{e}")),
    }

    match handle_legacy_lnk(startup_dir, lnk_name) {
        LegacyLnkOutcome::NotFound => {}
        LegacyLnkOutcome::Removed { target } => notes.push(format!(
            "已移除旧版开机自启快捷方式「{lnk_name}」（{target}）"
        )),
        LegacyLnkOutcome::KeptForeign { target } => notes.push(format!(
            "发现同名但非本网关的自启项「{lnk_name}」（{target}），未动"
        )),
        LegacyLnkOutcome::Failed { error } => notes.push(format!(
            "检查旧版开机自启快捷方式失败：{error}（未删除）"
        )),
    }

    let enabled = store
        .read_run_entry()
        .map(|o| o.is_some())
        .unwrap_or(false);
    LaunchSyncReport { enabled, notes }
}

/// 手动开关：开 = 清除关闭标记 + 幂等建立自启项；关 = 持久化关闭标记 + 删自启项。
/// 关闭标记必须持久化，否则下次应用启动又会被自动打开。
pub fn set_enabled(
    store: &AutostartStore,
    exe: &Path,
    on: bool,
) -> Result<bool, AutostartError> {
    if on {
        store.set_opt_out(false)?;
        store.ensure(exe)?;
    } else {
        store.set_opt_out(true)?;
        store.remove_run_entry()?;
    }
    Ok(on)
}

/// 当前自启是否生效（Run 键里有自启项即生效；opt-out 状态下恒无自启项）。
pub fn current_enabled(store: &AutostartStore) -> Result<bool, AutostartError> {
    Ok(store.read_run_entry()?.is_some())
}

/// 自启项的注册表存储位置。
/// 生产用真实 HKCU Run 键；测试用 `with_prefix` 注入 HKCU 下的临时键，防污染真键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutostartStore {
    /// HKCU\Software\Microsoft\Windows\CurrentVersion\Run
    pub run_key: String,
    /// 自启项值名（当前用户 Run 键内唯一，天然防重复项）
    pub run_value: String,
    /// 手动关闭的持久化标记所在键：HKCU\Software\ag-relay
    pub optout_key: String,
    pub optout_value: String,
}

impl AutostartStore {
    pub fn production() -> Self {
        Self {
            run_key: r"Software\Microsoft\Windows\CurrentVersion\Run".to_string(),
            run_value: "ag-relay".to_string(),
            optout_key: r"Software\ag-relay".to_string(),
            optout_value: "AutostartOptOut".to_string(),
        }
    }

    /// 测试注入：全部键挂在 HKCU\{prefix} 之下。
    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            run_key: format!(r"Software\{prefix}\Run"),
            run_value: "ag-relay".to_string(),
            optout_key: format!(r"Software\{prefix}"),
            optout_value: "AutostartOptOut".to_string(),
        }
    }

    /// 测试收尾：整树删除 prefix 下的临时键（生产 store 不可调用）。
    pub fn delete_all_test_keys(&self) {
        // run_key 形如 Software\{prefix}\Run，删除其父即整树
        if let Some(parent) = self.run_key.rsplit_once('\\').map(|(p, _)| p) {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let _ = hkcu.delete_subkey_all(parent);
        }
    }

    fn hkcu(&self) -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }
}

/// ensure 的执行结果（幂等语义）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnsureOutcome {
    /// 原本没有，新建了自启项
    Created,
    /// 原有自启项指向别的路径（如软件搬家/升级），已改写为当前路径
    PathUpdated { old_path: String },
    /// 自启项已存在且指向当前路径，未改动
    Unchanged,
}

impl AutostartStore {
    /// 幂等确保自启项存在且指向 exe：没有则建，路径变了则改写，重复执行不动。
    /// 写入当前 exe 绝对路径（current_exe 天然随安装位置更新）。
    pub fn ensure(&self, exe: &Path) -> Result<EnsureOutcome, AutostartError> {
        let data = exe.to_string_lossy().into_owned();
        let (key, _) = self.hkcu().create_subkey(&self.run_key)?;
        match key.get_value::<String, _>(&self.run_value) {
            Ok(old) if old == data => Ok(EnsureOutcome::Unchanged),
            Ok(old) => {
                key.set_value(&self.run_value, &data)?;
                Ok(EnsureOutcome::PathUpdated { old_path: old })
            }
            Err(_) => {
                key.set_value(&self.run_value, &data)?;
                Ok(EnsureOutcome::Created)
            }
        }
    }

    /// 读当前自启项指向的路径（None = 无自启项）。
    pub fn read_run_entry(&self) -> Result<Option<String>, AutostartError> {
        let key = match self.hkcu().open_subkey(&self.run_key) {
            Ok(k) => k,
            // 键不存在 = 没有自启项
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(key.get_value::<String, _>(&self.run_value).ok())
    }

    /// 删除自启项（手动关闭用）。返回是否真的删了（原本就不存在 → false）。
    pub fn remove_run_entry(&self) -> Result<bool, AutostartError> {
        let key = match self
            .hkcu()
            .open_subkey_with_flags(&self.run_key, KEY_READ | KEY_WRITE)
        {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        match key.delete_value(&self.run_value) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// 用户是否手动关闭过开机自启（持久化标记）。
    /// 作用：手动关闭后，下次应用启动不得再自动建立自启项。
    pub fn opt_out(&self) -> Result<bool, AutostartError> {
        let key = match self.hkcu().open_subkey(&self.optout_key) {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        Ok(key.get_value::<u32, _>(&self.optout_value).is_ok())
    }

    /// 写入/清除「手动关闭」标记。
    pub fn set_opt_out(&self, on: bool) -> Result<(), AutostartError> {
        if on {
            let (key, _) = self.hkcu().create_subkey(&self.optout_key)?;
            key.set_value(&self.optout_value, &1u32)?;
        } else {
            let key = match self
                .hkcu()
                .open_subkey_with_flags(&self.optout_key, KEY_READ | KEY_WRITE)
            {
                Ok(k) => k,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e.into()),
            };
            let _ = key.delete_value(&self.optout_value);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// HKCU 下的临时键前缀（每次测试进程唯一），测试完整树删除，不污染真键。
    fn temp_store(tag: &str) -> AutostartStore {
        AutostartStore::with_prefix(&format!(
            "ag-relay-selftest-{}-{}",
            std::process::id(),
            tag
        ))
    }

    fn cleanup(store: &AutostartStore) {
        store.delete_all_test_keys();
    }

    #[test]
    fn ensure_首次执行在Run键建立自启项_指向传入exe路径() {
        let store = temp_store("a");
        let exe = PathBuf::from(r"C:\安装 目录\ag-relay.exe");
        let outcome = store.ensure(&exe).expect("ensure 首次应成功");
        assert_eq!(outcome, EnsureOutcome::Created);
        // 独立读回（不经 ensure 的写入路径）验证注册表真实状态
        assert_eq!(
            store.read_run_entry().expect("read 应成功"),
            Some(exe.to_string_lossy().into_owned())
        );
        cleanup(&store);
    }

    #[test]
    fn ensure_重复执行不产生重复项且结果为Unchanged() {
        let store = temp_store("b");
        let exe = PathBuf::from(r"C:\Program Files\ag-relay\ag-relay.exe");
        assert_eq!(store.ensure(&exe).expect("第一次"), EnsureOutcome::Created);
        assert_eq!(store.ensure(&exe).expect("第二次"), EnsureOutcome::Unchanged);
        assert_eq!(store.ensure(&exe).expect("第三次"), EnsureOutcome::Unchanged);
        // 独立枚举 Run 键下的全部值：只有我们这一个自启项，无重复
        let key = store.hkcu().open_subkey(&store.run_key).expect("打开临时 Run 键");
        let values: Vec<String> = key
            .enum_values()
            .map(|r| r.expect("枚举项有效").0)
            .collect();
        assert_eq!(values, vec![store.run_value.clone()], "Run 键内不应出现重复自启项");
        cleanup(&store);
    }

    #[test]
    fn ensure_路径变更后自动改写为当前路径() {
        let store = temp_store("c");
        let old_exe = PathBuf::from(r"C:\旧位置\ag-relay.exe");
        let new_exe = PathBuf::from(r"D:\新位置\ag-relay.exe");
        assert_eq!(store.ensure(&old_exe).expect("第一次"), EnsureOutcome::Created);
        let outcome = store.ensure(&new_exe).expect("第二次");
        assert_eq!(outcome, EnsureOutcome::PathUpdated { old_path: old_exe.to_string_lossy().into_owned() });
        assert_eq!(
            store.read_run_entry().expect("read"),
            Some(new_exe.to_string_lossy().into_owned())
        );
        cleanup(&store);
    }

    #[test]
    fn remove_删除自启项_重复删除返回false且读取为空() {
        let store = temp_store("d");
        let exe = PathBuf::from(r"C:\x\ag-relay.exe");
        store.ensure(&exe).expect("先建立");
        assert!(store.remove_run_entry().expect("第一次删除应成功"));
        assert!(!store.remove_run_entry().expect("重复删除应安全返回 false"));
        assert_eq!(store.read_run_entry().expect("read"), None);
        // 键整体不存在时删除同样安全
        assert!(!store.remove_run_entry().expect("键不存在时删除"));
        cleanup(&store);
    }

    #[test]
    fn optout_手动关闭标记可读写_无标记视为未关闭() {
        let store = temp_store("e");
        assert!(!store.opt_out().expect("无标记应返回 false"));
        store.set_opt_out(true).expect("写入关闭标记");
        assert!(store.opt_out().expect("读回标记"));
        store.set_opt_out(false).expect("清除关闭标记");
        assert!(!store.opt_out().expect("清除后应返回 false"));
        cleanup(&store);
    }

    // ---- 旧自启快捷方式判定（纯函数） ----

    #[test]
    fn 判定_cmd调本网关startbat_判为本网关() {
        // 真机侦查到的旧 lnk 形态（2026-10-11）：TargetPath 是 cmd.exe，特征在参数里
        let target = r"C:\Windows\System32\cmd.exe";
        let args = r#"/d /c ""C:\Users\Administrator\Desktop\relay-retry-proxy\程序\start.bat" --startup""#;
        assert!(is_our_gateway_shortcut(target, args));
    }

    #[test]
    fn 判定_直接指向startbat_判为本网关() {
        assert!(is_our_gateway_shortcut(
            r"C:\tools\relay-retry-proxy\start.bat",
            "--startup"
        ));
    }

    #[test]
    fn 判定_大小写混排_仍判为本网关() {
        assert!(is_our_gateway_shortcut(
            r"C:\Tools\Relay-Retry-Proxy\Start.BAT",
            ""
        ));
    }

    #[test]
    fn 判定_指向无关程序_notepad_判为非本网关() {
        assert!(!is_our_gateway_shortcut(
            r"C:\Windows\System32\notepad.exe",
            ""
        ));
    }

    #[test]
    fn 判定_同名但指向别处含relay字样_判为非本网关() {
        // 只有 relay-retry-proxy、没有 start.bat → 不满足两个特征 → 保留
        assert!(!is_our_gateway_shortcut(
            r"C:\其他\relay-retry-proxy\readme.txt",
            ""
        ));
        // 只有 start.bat、没有 relay-retry-proxy → 保留
        assert!(!is_our_gateway_shortcut(
            r"C:\其他项目\start.bat",
            "--startup"
        ));
    }

    #[test]
    fn 判定_空目标_判为非本网关() {
        assert!(!is_our_gateway_shortcut("", ""));
    }

    // ---- 旧自启快捷方式的解析与处置（PowerShell 真实 lnk，临时目录） ----

    /// 用 PowerShell WScript.Shell 在临时目录造真实 .lnk（与生产同一解析对象）。
    /// CREATE_NO_WINDOW：不带窗口，也与测试进程的控制台状态完全隔离。
    fn create_test_lnk(dir: &Path, name: &str, target: &str, args: &str) -> PathBuf {
        use std::os::windows::process::CommandExt;
        let lnk = dir.join(name);
        let ps = format!(
            "$ErrorActionPreference='Stop'; $s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath='{}'; $s.Arguments='{}'; $s.Save()",
            lnk.to_string_lossy().replace('\'', "''"),
            target.replace('\'', "''"),
            args.replace('\'', "''"),
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
            .creation_flags(crate::gateway_process::CREATE_NO_WINDOW)
            .output()
            .expect("启动 powershell 造 lnk");
        assert!(
            out.status.success() && lnk.is_file(),
            "造测试 lnk 失败（退出码 {:?}）：stderr={} stdout={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
        );
        lnk
    }

    fn temp_startup_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ag-relay-t6-{}-{}", std::process::id(), tag));
        std::fs::create_dir_all(&dir).expect("建临时启动目录");
        dir
    }

    #[test]
    fn 旧lnk_指向本网关startbat_删除() {
        let dir = temp_startup_dir("ours");
        // 造一个路径里真的含 relay-retry-proxy 的 start.bat（让 lnk 目标带足特征）；
        // 目录名带中文：回归覆盖「PowerShell stdout 编码」——
        // 默认 OEM 代码页会把非 ASCII 变乱码（曾致提示区显示 ◆◆◆◆）
        let fake_gw_dir = dir.join("relay-retry-proxy-测试网关");
        std::fs::create_dir_all(&fake_gw_dir).expect("建假网关目录");
        let bat = fake_gw_dir.join("start.bat");
        std::fs::write(&bat, "@echo off\r\nrem 测试桩\r\n").expect("写假 start.bat");
        let lnk = create_test_lnk(
            &dir,
            LEGACY_LNK_NAME,
            r"C:\Windows\System32\cmd.exe",
            &format!("/d /c \"\"{}\" --startup\"", bat.to_string_lossy()),
        );
        assert!(lnk.is_file());

        let outcome = handle_legacy_lnk(&dir, LEGACY_LNK_NAME);
        match &outcome {
            LegacyLnkOutcome::Removed { target } => {
                assert!(target.to_lowercase().contains("relay-retry-proxy"));
                assert!(target.to_lowercase().contains("start.bat"));
                assert!(target.contains("测试网关"), "中文路径段必须原样保留（不能乱码），实际：{target}");
            }
            other => panic!("应删除本网关旧 lnk，实际 {:?}", other),
        }
        assert!(!lnk.exists(), "旧 lnk 应已被删除");
    }

    #[test]
    fn 旧lnk_同名但指向无关程序_保留不动() {
        let dir = temp_startup_dir("foreign");
        let lnk = create_test_lnk(&dir, LEGACY_LNK_NAME, r"C:\Windows\System32\notepad.exe", "");
        let outcome = handle_legacy_lnk(&dir, LEGACY_LNK_NAME);
        match &outcome {
            LegacyLnkOutcome::KeptForeign { target } => {
                assert!(target.to_lowercase().contains("notepad"));
            }
            other => panic!("同名非本网关应保留，实际 {:?}", other),
        }
        assert!(lnk.is_file(), "非本网关的同名 lnk 绝不能被删");
    }

    #[test]
    fn 旧lnk_不存在_报NotFound不做任何动作() {
        let dir = temp_startup_dir("absent");
        let outcome = handle_legacy_lnk(&dir, LEGACY_LNK_NAME);
        assert_eq!(outcome, LegacyLnkOutcome::NotFound);
    }

    // ---- 应用启动编排（sync_on_launch）与手动开关 ----

    #[test]
    fn sync_默认首启建立自启项_同名非本网关lnk保留并出提示() {
        let store = temp_store("s1");
        let dir = temp_startup_dir("s1");
        let lnk = create_test_lnk(&dir, LEGACY_LNK_NAME, r"C:\Windows\System32\notepad.exe", "");
        let exe = PathBuf::from(r"C:\成品\ag-relay.exe");

        let report = sync_on_launch(&store, &dir, LEGACY_LNK_NAME, &exe);

        assert!(report.enabled, "默认应开启自启");
        assert_eq!(
            store.read_run_entry().expect("read"),
            Some(exe.to_string_lossy().into_owned())
        );
        assert!(
            report.notes.iter().any(|n| n.contains("未动") && n.contains(LEGACY_LNK_NAME)),
            "提示区应记一条「未动」：{:?}",
            report.notes
        );
        assert!(lnk.is_file(), "非本网关的同名 lnk 必须还在");
        cleanup(&store);
    }

    #[test]
    fn sync_手动关闭过则保持关闭且清残留自启项() {
        let store = temp_store("s2");
        let dir = temp_startup_dir("s2");
        let exe = PathBuf::from(r"C:\成品\ag-relay.exe");
        store.set_opt_out(true).expect("预设手动关闭标记");
        store.ensure(&exe).expect("预设一个残留自启项");

        let report = sync_on_launch(&store, &dir, LEGACY_LNK_NAME, &exe);

        assert!(!report.enabled, "手动关闭后必须保持关闭");
        assert_eq!(store.read_run_entry().expect("read"), None, "残留自启项应被清掉");
        assert!(
            report.notes.iter().any(|n| n.contains("保持关闭")),
            "应提示保持关闭：{:?}",
            report.notes
        );
        cleanup(&store);
    }

    #[test]
    fn sync_重复启动无变化则不出提示() {
        let store = temp_store("s3");
        let dir = temp_startup_dir("s3");
        let exe = PathBuf::from(r"C:\成品\ag-relay.exe");
        let first = sync_on_launch(&store, &dir, LEGACY_LNK_NAME, &exe);
        assert!(first.enabled);
        let second = sync_on_launch(&store, &dir, LEGACY_LNK_NAME, &exe);
        assert!(
            second.notes.is_empty(),
            "无变化时不应刷提示，实际 {:?}",
            second.notes
        );
        cleanup(&store);
    }

    #[test]
    fn set_enabled_开关往返_注册表状态随之变化() {
        let store = temp_store("s4");
        let exe = PathBuf::from(r"C:\成品\ag-relay.exe");

        assert!(set_enabled(&store, &exe, true).expect("开"));
        assert!(current_enabled(&store).expect("读"));
        assert!(!store.opt_out().expect("开启时不应有关闭标记"));

        assert!(!set_enabled(&store, &exe, false).expect("关"));
        assert!(!current_enabled(&store).expect("读"));
        assert!(store.opt_out().expect("关闭标记应存在"), "关闭必须持久化，防下次启动又被自动打开");

        assert!(set_enabled(&store, &exe, true).expect("再开"));
        assert!(current_enabled(&store).expect("读"));
        assert!(!store.opt_out().expect("再开后关闭标记应清除"));
        cleanup(&store);
    }
}
