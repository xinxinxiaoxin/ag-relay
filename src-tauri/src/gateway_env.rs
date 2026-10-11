//! 网关环境变量装配。
//!
//! 17 项值来自共享笔记《env-vars-and-gateway-facts.md》第 1 节（2026-10-11 逐项读取
//! gateway-config.bat 抄录），一个不能少、值不能改；漏设或错值都会改变网关行为。
//! 另有壳必须补设的一项：PROXY_DIAGNOSTICS_FILE（诊断日志指向应用数据目录）。
//! PROXY_SUPERVISED=1 由 supervisor 自己设置，壳不设。

use std::path::Path;

/// 网关环境变量表（17 项，顺序与笔记第 1 节一致）。
pub const GATEWAY_ENV_TABLE: [(&str, &str); 17] = [
    ("PROXY_UPSTREAM", "https://agentrouter.org"),
    ("PROXY_HOST", "127.0.0.1"),
    ("PROXY_PORT", "8787"),
    ("PROXY_MAX_ATTEMPTS", "110"),
    ("PROXY_BASE_DELAY_MS", "500"),
    ("PROXY_MAX_DELAY_MS", "1000"),
    ("PROXY_DEPLOY_MAX_ATTEMPTS", "8"),
    ("PROXY_DEPLOY_DELAY_MS", "800"),
    ("PROXY_CONNECT_TIMEOUT_MS", "150000"),
    ("PROXY_STREAM_IDLE_MS", "90000"),
    ("PROXY_STREAM_PROGRESS_MS", "180000"),
    ("PROXY_TOTAL_BUDGET_MS", "300000"),
    ("PROXY_SESSION_COMPAT", "1"),
    ("PROXY_SESSION_COMPAT_TTL_MS", "86400000"),
    ("PROXY_SESSION_COMPAT_MAX_KEYS", "4096"),
    ("PROXY_STATUS_INTERVAL_MS", "20000"),
    ("PROXY_VERBOSE", "1"),
];

/// 只读视图，供测试与上层校验。
pub fn gateway_env_table() -> &'static [(&'static str, &'static str); 17] {
    &GATEWAY_ENV_TABLE
}

/// 在继承环境之上的追加项：17 项网关配置 + 诊断日志路径（PROXY_DIAGNOSTICS_FILE）。
/// 注意：不含 PROXY_SUPERVISED（supervisor fork 网关时自己注入）。
pub fn gateway_env_pairs(diagnostics_file: &Path) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = GATEWAY_ENV_TABLE
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    pairs.push((
        "PROXY_DIAGNOSTICS_FILE".to_string(),
        diagnostics_file.to_string_lossy().into_owned(),
    ));
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 独立于实现的真值来源：共享笔记 env-vars-and-gateway-facts.md 第 1 节原文逐项抄录。
    /// 任何一项遗漏或错值，本测试必须失败。
    const EXPECTED_17: [(&str, &str); 17] = [
        ("PROXY_UPSTREAM", "https://agentrouter.org"),
        ("PROXY_HOST", "127.0.0.1"),
        ("PROXY_PORT", "8787"),
        ("PROXY_MAX_ATTEMPTS", "110"),
        ("PROXY_BASE_DELAY_MS", "500"),
        ("PROXY_MAX_DELAY_MS", "1000"),
        ("PROXY_DEPLOY_MAX_ATTEMPTS", "8"),
        ("PROXY_DEPLOY_DELAY_MS", "800"),
        ("PROXY_CONNECT_TIMEOUT_MS", "150000"),
        ("PROXY_STREAM_IDLE_MS", "90000"),
        ("PROXY_STREAM_PROGRESS_MS", "180000"),
        ("PROXY_TOTAL_BUDGET_MS", "300000"),
        ("PROXY_SESSION_COMPAT", "1"),
        ("PROXY_SESSION_COMPAT_TTL_MS", "86400000"),
        ("PROXY_SESSION_COMPAT_MAX_KEYS", "4096"),
        ("PROXY_STATUS_INTERVAL_MS", "20000"),
        ("PROXY_VERBOSE", "1"),
    ];

    #[test]
    fn 配置表恰好是笔记里的17项且名值逐项一致() {
        let table = gateway_env_table();
        assert_eq!(table.len(), 17, "必须恰好 17 项（漏一项或多一项都不合格）");
        for (i, (name, value)) in EXPECTED_17.iter().enumerate() {
            assert_eq!(
                table[i],
                (*name, *value),
                "第 {} 项与笔记第 1 节不一致",
                i + 1
            );
        }
    }

    #[test]
    fn 装配的环境对包含全部17项与诊断路径() {
        let diag = PathBuf::from(r"C:\Users\x\AppData\Roaming\ag-relay\logs\relay-diagnostics.jsonl");
        let pairs = gateway_env_pairs(&diag);

        for (name, value) in EXPECTED_17.iter() {
            let actual = pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
                .unwrap_or_else(|| panic!("缺少环境变量 {}", name));
            assert_eq!(actual, *value, "环境变量 {} 的值与笔记不一致", name);
        }

        let diag_actual = pairs
            .iter()
            .find(|(k, _)| k == "PROXY_DIAGNOSTICS_FILE")
            .map(|(_, v)| PathBuf::from(v))
            .expect("缺少 PROXY_DIAGNOSTICS_FILE");
        assert_eq!(diag_actual, diag);

        // 壳不设 PROXY_SUPERVISED（supervisor fork 网关时自己注入）
        assert!(
            !pairs.iter().any(|(k, _)| k == "PROXY_SUPERVISED"),
            "壳不应设置 PROXY_SUPERVISED（由 supervisor 自己设置）"
        );
        assert_eq!(pairs.len(), 18, "应为 17 项配置 + 1 项诊断路径");
    }
}
