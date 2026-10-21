//! 网关环境变量装配。
//!
//! 17 项值的独立真值：上游 `gateway-config.bat`（已与三个 .mjs 同目录入库到
//! `src-tauri/resources/gateway/`，该文件只含环境变量默认值、不含任何密钥）。
//! 测试逐项断言环境变量表与 bat 解析结果一致，bat 改了这里就会红。
//! 历史来源：共享笔记《env-vars-and-gateway-facts.md》第 1 节（2026-10-11 抄录）。
//! 另有壳必须补设的两项：PROXY_DIAGNOSTICS_FILE（诊断日志指向应用数据目录）与
//! PROXY_DIAGNOSTICS_MAX_BYTES（诊断每份容量；1 MiB 仅约 2–4 小时，2026-10-21 上调为 8 MiB，
//! 沿用核心内置默认，不改变 17 项重试契约）。
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

/// 诊断每份容量：8 MiB。与核心 `diagnosticsMaxBytes` 内置默认一致（shell 显式注入便于面板展示）。
pub const DIAGNOSTICS_MAX_BYTES: &str = "8388608";

/// 只读视图，供测试与上层校验。
pub fn gateway_env_table() -> &'static [(&'static str, &'static str); 17] {
    &GATEWAY_ENV_TABLE
}

/// 在继承环境之上的追加项：17 项网关配置 + 诊断日志路径与容量。
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
    pairs.push((
        "PROXY_DIAGNOSTICS_MAX_BYTES".to_string(),
        DIAGNOSTICS_MAX_BYTES.to_string(),
    ));
    pairs
}

/// 用户连接设置在默认配方之上覆盖；密钥不属于设置表。
pub fn gateway_env_pairs_with_settings(
    diagnostics_file: &Path,
    settings: &crate::connection_settings::ConnectionSettings,
) -> Vec<(String, String)> {
    let mut pairs = gateway_env_pairs(diagnostics_file);
    for (key, value) in &mut pairs {
        match key.as_str() {
            "PROXY_UPSTREAM" => *value = settings.upstream().into(),
            "PROXY_HOST" => *value = settings.host().into(),
            "PROXY_PORT" => *value = settings.port().to_string(),
            _ => {}
        }
    }
    pairs
}

/// 受管启动显式注入完整策略；旧 17 项环境契约及无策略入口保持不变。
/// spawn_managed 在 env_clear 后逐项添加此列表，supervisor 再自然传给核心。
pub fn gateway_env_pairs_with_policy(
    diagnostics_file: &Path,
    settings: &crate::connection_settings::ConnectionSettings,
    policy: &crate::policy_settings::PolicySettings,
) -> Result<Vec<(String, String)>, String> {
    policy.validate()?;
    let mut pairs = gateway_env_pairs_with_settings(diagnostics_file, settings);
    pairs.push((
        "PROXY_POLICY_JSON".into(),
        serde_json::to_string(policy).map_err(|_| "无法编码策略启动配置")?,
    ));
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// 手抄兜底真值（仅在 bat fixture 缺失时使用）：
    /// 共享笔记 env-vars-and-gateway-facts.md 第 1 节原文逐项抄录。
    /// 独立真值是 resources/gateway/gateway-config.bat（F7），优先用它。
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

    /// 独立真值 fixture：上游 gateway-config.bat（与三个 .mjs 同目录入库）。
    fn bat_fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("gateway")
            .join("gateway-config.bat")
    }

    /// 解析 bat 里的环境变量赋值：仅取 `set "K=V"` 行
    /// （@echo off / REM 注释 / exit /b 一律忽略；键值两侧 trim，值原样保留）。
    fn parse_bat_env(text: &str) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("set \"") else {
                continue;
            };
            let Some(assign) = rest.strip_suffix('"') else {
                continue;
            };
            let Some((key, value)) = assign.split_once('=') else {
                continue;
            };
            // 诊断容量不属于 17 项重试契约，单独断言；这里只取重试表覆盖的键。
            if !GATEWAY_ENV_TABLE.iter().any(|(name, _)| *name == key.trim()) {
                continue;
            }
            pairs.push((key.trim().to_string(), value.to_string()));
        }
        pairs
    }

    #[test]
    fn 配置表与上游gateway_config_bat逐项一致() {
        let path = bat_fixture_path();
        if !path.is_file() {
            // fixture 缺失时的兜底：退回手抄真值（笔记第 1 节），仍保有一层契约保护。
            // 正常情况下 bat 已入库，本分支不应触发（README 重打包步骤第 1 步负责补齐）。
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
            return;
        }
        let text = std::fs::read_to_string(&path).expect("读取 bat fixture 失败");
        let bat = parse_bat_env(&text);

        assert_eq!(bat.len(), 17, "bat 应恰好 set 17 项，实际：{bat:?}");
        // 键不重复且顺序与表一致（bat 就是真值排序）
        let bat_keys: Vec<&str> = bat.iter().map(|(k, _)| k.as_str()).collect();
        let mut deduped = bat_keys.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), 17, "bat 不得有重复键：{bat_keys:?}");

        let table = gateway_env_table();
        assert_eq!(table.len(), 17, "环境变量表必须恰好 17 项");
        for (index, (name, value)) in table.iter().enumerate() {
            assert_eq!(
                bat_keys.get(index),
                Some(name),
                "第 {} 项键与 bat 不一致（bat 键序：{bat_keys:?}）",
                index + 1
            );
            let bat_value = &bat[index].1;
            assert_eq!(bat_value, value, "环境变量 {name} 的值与 bat 不一致");
        }
        // 诊断容量与 shell 注入同源：bat 与 gateway_env 的常量必须一致。
        assert!(
            text.contains(&format!("set \"PROXY_DIAGNOSTICS_MAX_BYTES={DIAGNOSTICS_MAX_BYTES}\"")),
            "bat 的诊断容量应与 gateway_env 的常量一致（{DIAGNOSTICS_MAX_BYTES}）"
        );
    }

    #[test]
    fn 装配的环境对包含全部17项与诊断路径() {
        let diag =
            PathBuf::from(r"C:\Users\x\AppData\Roaming\ag-relay\logs\relay-diagnostics.jsonl");
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
        let diagnostics_limit = pairs
            .iter()
            .find(|(k, _)| k == "PROXY_DIAGNOSTICS_MAX_BYTES")
            .map(|(_, v)| v.as_str())
            .expect("缺少 PROXY_DIAGNOSTICS_MAX_BYTES");
        assert_eq!(diagnostics_limit, "8388608", "诊断每份容量应为 8 MiB");
        assert_eq!(pairs.len(), 19, "应为 17 项配置 + 诊断路径 + 诊断容量");
    }
}
