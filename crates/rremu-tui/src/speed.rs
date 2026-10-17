//! 速度档位：single/slow/middle/fast/full（←/→ 切换）。
//!
//! single 每秒只执行一条指令；其余档位按帧预算的时间占比节流：
//! slow/middle/fast/full = 峰值的 25%/50%/75%/100%。

/// 全速运行时的单批指令数。
pub const FULL_BATCH: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedTier {
    Single,
    Slow,
    Middle,
    Fast,
    Full,
}

impl SpeedTier {
    /// 5 档，按速度从低到高排列。
    pub const ALL: [SpeedTier; 5] = [
        SpeedTier::Single,
        SpeedTier::Slow,
        SpeedTier::Middle,
        SpeedTier::Fast,
        SpeedTier::Full,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SpeedTier::Single => "single",
            SpeedTier::Slow => "slow",
            SpeedTier::Middle => "middle",
            SpeedTier::Fast => "fast",
            SpeedTier::Full => "full",
        }
    }

    /// 相对峰值的占空比；`single` 不使用占空比调度。
    pub fn duty(self) -> Option<f64> {
        match self {
            SpeedTier::Single => None,
            SpeedTier::Slow => Some(0.25),
            SpeedTier::Middle => Some(0.5),
            SpeedTier::Fast => Some(0.75),
            SpeedTier::Full => Some(1.0),
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
        let mut s = SpeedTier::Single;
        for _ in 0..4 {
            s = s.next();
        }
        assert_eq!(s, SpeedTier::Full);
        // Full stays Full
        assert_eq!(s.next(), SpeedTier::Full);
    }

    #[test]
    fn prev_cycles_backwards() {
        assert_eq!(SpeedTier::Full.prev(), SpeedTier::Fast);
        assert_eq!(SpeedTier::Single.prev(), SpeedTier::Single); // clamped
    }

    #[test]
    fn labels_and_duty_ratios() {
        assert_eq!(SpeedTier::Single.label(), "single");
        assert_eq!(SpeedTier::Slow.label(), "slow");
        assert_eq!(SpeedTier::Middle.label(), "middle");
        assert_eq!(SpeedTier::Fast.label(), "fast");
        assert_eq!(SpeedTier::Full.label(), "full");
        assert_eq!(SpeedTier::Single.duty(), None);
        assert_eq!(SpeedTier::Slow.duty(), Some(0.25));
        assert_eq!(SpeedTier::Middle.duty(), Some(0.5));
        assert_eq!(SpeedTier::Fast.duty(), Some(0.75));
        assert_eq!(SpeedTier::Full.duty(), Some(1.0));
    }
}
