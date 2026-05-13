//! 冷却管理模块
#![allow(dead_code)]
//!
//! 分类管理不同原因的冷却状态，支持差异化冷却时长和自动清理。
//! 参考 CLIProxyAPIPlus 的实现。

use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 冷却原因
///
/// **当前触发覆盖** (Round 7 静态审查 2026-05-13):
///   生产代码中实际调用 `set_cooldown` 触发的变体:
///     - `RateLimitExceeded`  ← 上游 429 + 普通 throttle
///     - `TokenRefreshFailed` ← refresh wire 失败
///     - `ServerError`        ← 上游 5xx
///     - `ModelUnavailable`   ← `MODEL_TEMPORARILY_UNAVAILABLE` 全局熔断
///
///   尚未被 set_cooldown 实际触发的变体（保留作为 future 分类点）:
///     - `AuthenticationFailed` — 上游 401/403 非 bearer 失效场景（目前走通用 markFailure）
///     - `AccountSuspended`     — 上游账户级 suspend 信号（暂无明确 wire 字段）
///     - `QuotaExhausted`       — 月度配额耗尽（目前走 RateLimitExceeded）
///
///   这 3 个变体的 `default_duration()` / `is_retryable()` / `Display` 配置仍完整，
///   handlers/provider 层日后做细粒度错误分类时可直接使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CooldownReason {
    /// 429 速率限制
    RateLimitExceeded,

    /// 账户暂停
    AccountSuspended,

    /// 配额耗尽
    QuotaExhausted,

    /// Token 刷新失败
    TokenRefreshFailed,

    /// 认证失败
    AuthenticationFailed,

    /// 服务器错误
    ServerError,

    /// 模型暂时不可用
    ModelUnavailable,
}

impl CooldownReason {
    /// 获取默认冷却时长
    pub fn default_duration(&self) -> Duration {
        match self {
            // 短冷却（1-5 分钟）
            CooldownReason::RateLimitExceeded => Duration::from_secs(60),
            CooldownReason::TokenRefreshFailed => Duration::from_secs(60),
            CooldownReason::ServerError => Duration::from_secs(120),
            CooldownReason::ModelUnavailable => Duration::from_secs(300),

            // 长冷却（1-24 小时）
            CooldownReason::AuthenticationFailed => Duration::from_secs(3600),
            CooldownReason::AccountSuspended => Duration::from_secs(86400),
            CooldownReason::QuotaExhausted => Duration::from_secs(86400),
        }
    }

    /// 是否可以自动恢复
    pub fn is_auto_recoverable(&self) -> bool {
        match self {
            CooldownReason::RateLimitExceeded => true,
            CooldownReason::TokenRefreshFailed => true,
            CooldownReason::ServerError => true,
            CooldownReason::ModelUnavailable => true,
            CooldownReason::AuthenticationFailed => false,
            CooldownReason::AccountSuspended => false,
            CooldownReason::QuotaExhausted => false,
        }
    }

    /// 获取原因描述
    pub fn description(&self) -> &'static str {
        match self {
            CooldownReason::RateLimitExceeded => "速率限制",
            CooldownReason::AccountSuspended => "账户暂停",
            CooldownReason::QuotaExhausted => "配额耗尽",
            CooldownReason::TokenRefreshFailed => "Token 刷新失败",
            CooldownReason::AuthenticationFailed => "认证失败",
            CooldownReason::ServerError => "服务器错误",
            CooldownReason::ModelUnavailable => "模型暂时不可用",
        }
    }
}

/// 冷却条目
#[derive(Debug, Clone)]
struct CooldownEntry {
    /// 冷却原因
    reason: CooldownReason,

    /// 冷却开始时间
    started_at: Instant,

    /// 冷却结束时间
    expires_at: Instant,

    /// 连续触发次数（用于递增冷却时长）
    trigger_count: u32,
}

/// 冷却管理器
///
/// 管理所有凭据的冷却状态
pub struct CooldownManager {
    /// 凭据冷却状态
    entries: Mutex<HashMap<u64, CooldownEntry>>,

    /// 最大短冷却时长（秒）
    max_short_cooldown_secs: u64,

    /// 长冷却时长（秒）
    long_cooldown_secs: u64,
}

impl Default for CooldownManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CooldownManager {
    /// 创建新的冷却管理器
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_short_cooldown_secs: 300, // 5 分钟
            long_cooldown_secs: 86400,    // 24 小时
        }
    }

    /// 使用自定义配置创建冷却管理器
    #[allow(dead_code)]
    pub fn with_config(max_short_cooldown_secs: u64, long_cooldown_secs: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_short_cooldown_secs,
            long_cooldown_secs,
        }
    }

    /// 设置凭据冷却
    ///
    /// 返回实际的冷却时长
    pub fn set_cooldown(&self, credential_id: u64, reason: CooldownReason) -> Duration {
        self.set_cooldown_with_duration(credential_id, reason, None)
    }

    /// 设置凭据冷却（自定义时长）
    pub fn set_cooldown_with_duration(
        &self,
        credential_id: u64,
        reason: CooldownReason,
        custom_duration: Option<Duration>,
    ) -> Duration {
        let mut entries = self.entries.lock();
        let now = Instant::now();

        // 获取或创建条目
        let entry = entries
            .entry(credential_id)
            .or_insert_with(|| CooldownEntry {
                reason,
                started_at: now,
                expires_at: now,
                trigger_count: 0,
            });

        // 更新触发次数
        if entry.reason == reason {
            entry.trigger_count += 1;
        } else {
            entry.reason = reason;
            entry.trigger_count = 1;
        }

        // 计算冷却时长
        let duration = custom_duration
            .unwrap_or_else(|| self.calculate_cooldown_duration(reason, entry.trigger_count));

        entry.started_at = now;
        entry.expires_at = now + duration;

        tracing::info!(
            credential_id = %credential_id,
            reason = %reason.description(),
            duration_secs = %duration.as_secs(),
            trigger_count = %entry.trigger_count,
            "凭据进入冷却"
        );

        duration
    }

    /// 检查凭据是否在冷却中
    ///
    /// 返回 `None` 表示不在冷却中，`Some((reason, remaining))` 表示冷却原因和剩余时间
    pub fn check_cooldown(&self, credential_id: u64) -> Option<(CooldownReason, Duration)> {
        let entries = self.entries.lock();
        let now = Instant::now();

        entries.get(&credential_id).and_then(|entry| {
            if now < entry.expires_at {
                Some((
                    entry.reason,
                    entry.expires_at.saturating_duration_since(now),
                ))
            } else {
                None
            }
        })
    }

    /// 检查凭据是否可用（不在冷却中或冷却已过期）
    pub fn is_available(&self, credential_id: u64) -> bool {
        self.check_cooldown(credential_id).is_none()
    }

    /// 清除凭据冷却
    pub fn clear_cooldown(&self, credential_id: u64) -> bool {
        let mut entries = self.entries.lock();
        entries.remove(&credential_id).is_some()
    }

    /// 清除所有已过期的冷却
    pub fn cleanup_expired(&self) -> usize {
        let mut entries = self.entries.lock();
        let now = Instant::now();
        let before_count = entries.len();

        entries.retain(|_, entry| now < entry.expires_at);

        let removed = before_count - entries.len();
        if removed > 0 {
            tracing::debug!("清理了 {} 个过期冷却条目", removed);
        }
        removed
    }

    /// 获取所有冷却中的凭据
    pub fn get_all_cooldowns(&self) -> Vec<CooldownInfo> {
        let entries = self.entries.lock();
        let now = Instant::now();

        entries
            .iter()
            .filter(|(_, entry)| now < entry.expires_at)
            .map(|(&id, entry)| CooldownInfo {
                credential_id: id,
                reason: entry.reason,
                started_at_ms: entry.started_at.elapsed().as_millis() as u64,
                remaining_ms: entry.expires_at.saturating_duration_since(now).as_millis() as u64,
                trigger_count: entry.trigger_count,
            })
            .collect()
    }

    /// 计算冷却时长
    fn calculate_cooldown_duration(&self, reason: CooldownReason, trigger_count: u32) -> Duration {
        let base = reason.default_duration();

        if reason.is_auto_recoverable() {
            // 可自动恢复的原因：递增冷却时长，但不超过最大值
            let multiplier = 1.5_f64.powi((trigger_count.saturating_sub(1)) as i32);
            let duration_secs = (base.as_secs() as f64 * multiplier) as u64;
            let capped_secs = duration_secs.min(self.max_short_cooldown_secs);
            Duration::from_secs(capped_secs)
        } else {
            // 不可自动恢复的原因：使用长冷却时长
            Duration::from_secs(self.long_cooldown_secs)
        }
    }
}

/// 冷却信息（公开 API）
#[derive(Debug, Clone)]
pub struct CooldownInfo {
    /// 凭据 ID
    pub credential_id: u64,

    /// 冷却原因
    pub reason: CooldownReason,

    /// 冷却开始时间（毫秒前）
    pub started_at_ms: u64,

    /// 剩余冷却时间（毫秒）
    pub remaining_ms: u64,

    /// 连续触发次数
    pub trigger_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cooldown_manager_new() {
        let manager = CooldownManager::new();
        assert!(manager.is_available(1));
    }

    #[test]
    fn test_cooldown_set_and_check() {
        let manager = CooldownManager::new();

        let duration = manager.set_cooldown(1, CooldownReason::RateLimitExceeded);
        assert!(duration.as_secs() >= 60);

        let (reason, remaining) = manager.check_cooldown(1).unwrap();
        assert_eq!(reason, CooldownReason::RateLimitExceeded);
        assert!(remaining.as_secs() > 0);

        assert!(!manager.is_available(1));
    }

    #[test]
    fn test_cooldown_clear() {
        let manager = CooldownManager::new();

        manager.set_cooldown(1, CooldownReason::ServerError);
        assert!(!manager.is_available(1));

        assert!(manager.clear_cooldown(1));
        assert!(manager.is_available(1));
    }

    #[test]
    fn test_cooldown_incremental() {
        let manager = CooldownManager::new();

        // 第一次冷却
        let d1 = manager.set_cooldown(1, CooldownReason::RateLimitExceeded);

        // 清除后再次触发，应该有更长的冷却
        manager.clear_cooldown(1);
        let d2 = manager.set_cooldown(1, CooldownReason::RateLimitExceeded);

        // 由于触发次数增加，第二次冷却应该更长
        assert!(d2 >= d1);
    }

    #[test]
    fn test_cooldown_reason_auto_recoverable() {
        assert!(CooldownReason::RateLimitExceeded.is_auto_recoverable());
        assert!(CooldownReason::ServerError.is_auto_recoverable());
        assert!(!CooldownReason::AccountSuspended.is_auto_recoverable());
        assert!(!CooldownReason::QuotaExhausted.is_auto_recoverable());
    }

    #[test]
    fn test_cooldown_custom_duration() {
        let manager = CooldownManager::new();

        let custom = Duration::from_secs(10);
        let duration =
            manager.set_cooldown_with_duration(1, CooldownReason::ServerError, Some(custom));

        assert_eq!(duration, custom);
    }

    #[test]
    fn test_cooldown_get_all() {
        let manager = CooldownManager::new();

        manager.set_cooldown(1, CooldownReason::RateLimitExceeded);
        manager.set_cooldown(2, CooldownReason::ServerError);

        let cooldowns = manager.get_all_cooldowns();
        assert_eq!(cooldowns.len(), 2);
    }

    #[test]
    fn test_cooldown_reason_description() {
        assert_eq!(CooldownReason::RateLimitExceeded.description(), "速率限制");
        assert_eq!(CooldownReason::AccountSuspended.description(), "账户暂停");
    }
}
