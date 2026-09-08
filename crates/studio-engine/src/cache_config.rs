use serde::{Deserialize, Serialize};
use studio_domain::{Error, Result};
use studio_storage::QueryCachePolicy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    pub schema_version: u32,
    pub total_mib: u32,
    pub long_term_mib: u32,
    pub temporary_mib: u32,
    pub preview_mib: u32,
    pub long_term_idle_days: Option<u32>,
    pub temporary_idle_hours: u32,
    pub temporary_session_only: bool,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            schema_version: 2,
            total_mib: 65536,
            long_term_mib: 49152,
            temporary_mib: 8192,
            preview_mib: 8192,
            long_term_idle_days: None,
            temporary_idle_hours: 24,
            temporary_session_only: false,
        }
    }
}
impl CacheConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 2 {
            return Err(Error::new("FORMAT_UNSUPPORTED", "缓存设置版本不支持"));
        }
        if self.total_mib > 1048576
            || [self.long_term_mib, self.temporary_mib, self.preview_mib]
                .iter()
                .map(|v| u64::from(*v))
                .sum::<u64>()
                > u64::from(self.total_mib)
        {
            return Err(Error::invalid(
                "总缓存预算最多 1024 GiB，分类预算之和不能超过总预算",
            ));
        }
        if !(1..=2160).contains(&self.temporary_idle_hours)
            || self
                .long_term_idle_days
                .is_some_and(|d| !(1..=3650).contains(&d))
        {
            return Err(Error::invalid(
                "临时期限须为 1–2160 小时；长期期限须为 1–3650 天，或不自动过期",
            ));
        }
        Ok(())
    }
    pub fn query_mib(&self) -> u32 {
        self.long_term_mib.saturating_add(self.temporary_mib)
    }
    pub fn query_enabled(&self) -> bool {
        self.total_mib > 0 && self.query_mib() > 0
    }
    pub fn policy(&self) -> QueryCachePolicy {
        QueryCachePolicy {
            quota_bytes: u64::from(self.query_mib()) << 20,
            long_term_quota_bytes: u64::from(self.long_term_mib) << 20,
            temporary_quota_bytes: u64::from(self.temporary_mib) << 20,
            max_age_seconds: u64::from(self.temporary_idle_hours) * 3600,
            long_term_max_age_seconds: self.long_term_idle_days.map(|v| u64::from(v) * 86400),
            session_only: self.temporary_session_only,
            ..QueryCachePolicy::default()
        }
    }
    pub fn legacy(query_mib: u32, days: u32, preview_mib: u32) -> Result<Self> {
        if query_mib > 1048576 || !(1..=90).contains(&days) {
            return Err(Error::invalid("查询缓存容量或期限超出范围"));
        }
        let long_term_mib = query_mib / 2;
        let value = Self {
            total_mib: query_mib.saturating_add(preview_mib),
            long_term_mib,
            temporary_mib: query_mib - long_term_mib,
            preview_mib,
            long_term_idle_days: Some(days),
            temporary_idle_hours: days * 24,
            ..Self::default()
        };
        value.validate()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budgets_accept_one_hundred_gib_and_reject_hidden_category_overcommit() {
        let config = CacheConfig::default();
        config.validate().unwrap();
        assert_eq!(config.total_mib, 65536);
        assert_eq!(config.temporary_idle_hours, 24);
        assert_eq!(config.long_term_idle_days, None);
        CacheConfig {
            total_mib: 102400,
            long_term_mib: 86016,
            ..config.clone()
        }
        .validate()
        .unwrap();
        assert!(
            CacheConfig {
                total_mib: 1024,
                ..config
            }
            .validate()
            .is_err()
        );
        let migrated = CacheConfig::legacy(2048, 5, 4096).unwrap();
        assert_eq!(migrated.query_mib(), 2048);
        assert_eq!(migrated.preview_mib, 4096);
        assert_eq!(migrated.temporary_idle_hours, 120);
    }
}
