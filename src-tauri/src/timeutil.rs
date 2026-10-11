//! 共享的时间/历法换算（代码审查 F8 去重）：
//! - 公历 ↔ 天数（Howard Hinnant 算法的一对互逆实现）；
//! - 「当前 Unix 时刻」的秒/毫秒助手（时钟异常时按 0 处理，调用方只做兜底显示/落盘）。
//!
//! gateway_status.rs（startedAt 解析）与 app_exit.rs（退出日志时间戳）共用，
//! 两处不再各持一份手写历法。

/// 天数（自 1970-01-01，可为负）→（年, 月, 日）。Howard Hinnant 的 civil_from_days 算法。
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// （年, 月, 日）→ 天数（自 1970-01-01，可为负）。Howard Hinnant 的 days_from_civil 算法。
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // 3 月=0
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// 当前 Unix 秒（时钟异常时按 0 处理，日志行仍然成形）。
pub fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 当前 Unix 毫秒（时钟异常时按 0 处理）。
pub fn system_time_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 锚点独立真值：node -e "new Date(...).getTime()" / Date.UTC 换算。
    #[test]
    fn 历法双向换算_纪元闰日与远期锚点() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 闰日：2000-02-29 → node Date.UTC(2000,1,29)/86400000 = 11016
        let leap = days_from_civil(2000, 2, 29);
        assert_eq!(civil_from_days(leap), (2000, 2, 29));
        // 2099-12-31（远期锚点，format_utc_timestamp 单测同源）
        assert_eq!(civil_from_days(days_from_civil(2099, 12, 31)), (2099, 12, 31));
        // 跨闰年边界：1900 不是闰年（%100）
        assert_eq!(days_from_civil(1900, 3, 1) - days_from_civil(1900, 2, 28), 1);
        // 2000 是闰年（%400）
        assert_eq!(days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28), 2);
    }

    #[test]
    fn 当前时刻助手_毫秒与秒一致量级() {
        let secs = now_unix_secs() as i64;
        let ms = system_time_ms();
        // 两个助手读的是同一时钟：毫秒值换算成秒后与秒值相差不超过 2 秒
        assert!((ms.div_euclid(1000) - secs).abs() <= 2, "secs={secs} ms={ms}");
        // 与外部真值同量级：2025-10-11 前后的锚点
        assert!(secs > 1_700_000_000 && secs < 2_100_000_000);
    }
}
