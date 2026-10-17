//! 速度档位：1/4/16/64/256/全速（←/→ 切换）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedTier {
    Steps(u64),
    Full,
}

impl SpeedTier {
    /// 6 档：5 个固定步进档 + 全速。
    pub const ALL: [SpeedTier; 6] = [
        SpeedTier::Steps(1),
        SpeedTier::Steps(4),
        SpeedTier::Steps(16),
        SpeedTier::Steps(64),
        SpeedTier::Steps(256),
        SpeedTier::Full,
    ];

    /// 每帧执行的指令批大小；全速档在帧预算内循环使用该批大小。
    pub fn batch(self) -> u64 {
        match self {
            SpeedTier::Steps(n) => n,
            SpeedTier::Full => 4096,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SpeedTier::Steps(1) => "1",
            SpeedTier::Steps(4) => "4",
            SpeedTier::Steps(16) => "16",
            SpeedTier::Steps(64) => "64",
            SpeedTier::Steps(256) => "256",
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
        for _ in 0..5 {
            s = s.next();
        }
        assert_eq!(s, SpeedTier::Full);
        // Full stays Full
        assert_eq!(s.next(), SpeedTier::Full);
    }

    #[test]
    fn prev_cycles_backwards() {
        let s = SpeedTier::Full.prev();
        assert_eq!(s, SpeedTier::Steps(256));
        let s = SpeedTier::Steps(1).prev();
        assert_eq!(s, SpeedTier::Steps(1)); // clamped
    }

    #[test]
    fn batch_and_labels() {
        assert_eq!(SpeedTier::Steps(1).batch(), 1);
        assert_eq!(SpeedTier::Steps(256).batch(), 256);
        assert_eq!(SpeedTier::Full.batch(), 4096);
        assert_eq!(SpeedTier::Steps(1).label(), "1");
        assert_eq!(SpeedTier::Steps(4).label(), "4");
        assert_eq!(SpeedTier::Steps(16).label(), "16");
        assert_eq!(SpeedTier::Steps(64).label(), "64");
        assert_eq!(SpeedTier::Steps(256).label(), "256");
        assert_eq!(SpeedTier::Full.label(), "full");
    }
}
