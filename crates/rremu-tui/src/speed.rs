//! 速度档位：1/2/4/8/16/32/64/全速（←/→ 切换，M4.3）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedTier {
    Steps(u64),
    Full,
}

impl SpeedTier {
    /// 8 档，索引 0..6 为步进，7 为全速。
    pub const ALL: [SpeedTier; 8] = [
        SpeedTier::Steps(1),
        SpeedTier::Steps(2),
        SpeedTier::Steps(4),
        SpeedTier::Steps(8),
        SpeedTier::Steps(16),
        SpeedTier::Steps(32),
        SpeedTier::Steps(64),
        SpeedTier::Full,
    ];

    pub fn batch(self) -> u64 {
        match self {
            SpeedTier::Steps(n) => n,
            SpeedTier::Full => 4096,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SpeedTier::Steps(1) => "1",
            SpeedTier::Steps(2) => "2",
            SpeedTier::Steps(4) => "4",
            SpeedTier::Steps(8) => "8",
            SpeedTier::Steps(16) => "16",
            SpeedTier::Steps(32) => "32",
            SpeedTier::Steps(64) => "64",
            SpeedTier::Steps(_) => "?",
            SpeedTier::Full => "full",
        }
    }

    pub fn next(self) -> SpeedTier {
        let idx = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(idx + 1).min(Self::ALL.len() - 1)]
    }

    pub fn prev(self) -> SpeedTier {
        let idx = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[idx.saturating_sub(1)]
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn next_cycles_through_all_tiers() {
        let mut s = SpeedTier::Steps(1);
        for _ in 0..7 {
            s = s.next();
        }
        assert_eq!(s, SpeedTier::Full);
        // Full stays Full
        assert_eq!(s.next(), SpeedTier::Full);
    }

    #[test]
    fn prev_cycles_backwards() {
        let s = SpeedTier::Full.prev();
        assert_eq!(s, SpeedTier::Steps(64));
        let s = SpeedTier::Steps(1).prev();
        assert_eq!(s, SpeedTier::Steps(1)); // clamped
    }

    #[test]
    fn batch_returns_step_count_or_large_for_full() {
        assert_eq!(SpeedTier::Steps(8).batch(), 8);
        assert_eq!(SpeedTier::Full.batch(), 4096);
    }
}
