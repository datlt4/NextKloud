//! Quota enforcement — must stay pure + fully unit-tested
//! (user can't test manually, so `cargo test -p nextkloud-backend` is the gate).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quota {
    /// bytes allowed, 0 = unlimited (admin)
    pub limit_bytes: u64,
    /// bytes currently used
    pub used_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaError {
    Exceeded { limit: u64, used: u64, requested: u64 },
    Overflow,
}

impl std::fmt::Display for QuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exceeded { limit, used, requested } => write!(
                f,
                "quota exceeded: limit={limit} used={used} requested={requested}"
            ),
            Self::Overflow => write!(f, "quota arithmetic overflow"),
        }
    }
}

impl Quota {
    pub fn new(limit_bytes: u64, used_bytes: u64) -> Self {
        Self { limit_bytes, used_bytes }
    }

    /// unlimited when limit == 0
    pub fn is_unlimited(&self) -> bool {
        self.limit_bytes == 0
    }

    /// Can `requested` bytes be added?
    pub fn can_add(&self, requested: u64) -> Result<(), QuotaError> {
        if self.is_unlimited() {
            return Ok(());
        }
        let next = self.used_bytes.checked_add(requested).ok_or(QuotaError::Overflow)?;
        if next > self.limit_bytes {
            return Err(QuotaError::Exceeded {
                limit: self.limit_bytes,
                used: self.used_bytes,
                requested,
            });
        }
        Ok(())
    }

    /// Apply an upload (mutates used). Returns new used.
    pub fn add(&mut self, requested: u64) -> Result<u64, QuotaError> {
        self.can_add(requested)?;
        self.used_bytes = self.used_bytes.saturating_add(requested);
        Ok(self.used_bytes)
    }

    /// Apply a delete / free-up.
    pub fn release(&mut self, freed: u64) -> u64 {
        self.used_bytes = self.used_bytes.saturating_sub(freed);
        self.used_bytes
    }

    /// Admin resize (expand or shrink). Shrinking below current usage is
    /// allowed but the user becomes over-quota (read-only until freed).
    /// Returns true if currently over quota.
    pub fn resize(&mut self, new_limit: u64) -> bool {
        self.limit_bytes = new_limit;
        self.over_quota()
    }

    pub fn over_quota(&self) -> bool {
        !self.is_unlimited() && self.used_bytes > self.limit_bytes
    }

    pub fn remaining(&self) -> Option<u64> {
        if self.is_unlimited() {
            return None;
        }
        Some(self.limit_bytes.saturating_sub(self.used_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;

    #[test]
    fn unlimited_never_blocks() {
        let mut q = Quota::new(0, 999 * GB);
        assert!(q.can_add(u64::MAX / 2).is_ok());
        assert!(q.add(10 * GB).is_ok());
        assert!(!q.over_quota());
        assert_eq!(q.remaining(), None);
    }

    #[test]
    fn exact_fit_allowed_one_more_byte_blocked() {
        let mut q = Quota::new(100 * MB, 0);
        assert!(q.add(100 * MB).is_ok());
        assert_eq!(q.remaining(), Some(0));
        let err = q.can_add(1).unwrap_err();
        assert_eq!(
            err,
            QuotaError::Exceeded { limit: 100 * MB, used: 100 * MB, requested: 1 }
        );
    }

    #[test]
    fn upload_then_delete_frees_space() {
        let mut q = Quota::new(10 * GB, 0);
        q.add(6 * GB).unwrap();
        q.add(3 * GB).unwrap();
        assert!(q.add(2 * GB).is_err()); // 9+2 > 10
        q.release(4 * GB);
        assert_eq!(q.used_bytes, 5 * GB);
        assert!(q.add(2 * GB).is_ok());
    }

    #[test]
    fn shrink_below_usage_flags_over_quota() {
        let mut q = Quota::new(10 * GB, 8 * GB);
        let over = q.resize(5 * GB); // admin thu hẹp
        assert!(over);
        assert!(q.over_quota());
        assert!(q.can_add(1).is_err());
        // user xóa 4GB -> hết over-quota
        q.release(4 * GB);
        assert!(!q.over_quota());
        assert!(q.add(1 * GB).is_ok()); // 4+1 = 5 == limit
    }

    #[test]
    fn expand_allows_uploads_again() {
        let mut q = Quota::new(1 * GB, 1 * GB);
        assert!(q.add(1).is_err());
        q.resize(5 * GB);
        assert!(q.add(3 * GB).is_ok());
    }

    #[test]
    fn overflow_is_error_not_wrap() {
        let q = Quota::new(u64::MAX, u64::MAX);
        assert_eq!(q.can_add(1).unwrap_err(), QuotaError::Overflow);
    }

    #[test]
    fn release_saturates_at_zero() {
        let mut q = Quota::new(1 * GB, 100);
        q.release(999 * GB);
        assert_eq!(q.used_bytes, 0);
    }
}
