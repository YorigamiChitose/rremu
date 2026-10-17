//! 速度档位：1/4/16/64/256（←/→ 切换）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedTier {
    Steps(u64),
}

impl SpeedTier {
    /// 5 档步进。
    pub const ALL: [SpeedTier; 5] = [
        SpeedTier::Steps(1),
        SpeedTier::Steps(4),
        SpeedTier::Steps(16),
        SpeedTier::Steps(64),
        SpeedTier::Steps(256),
    ];

    /// 最快档，供 `f` 快捷键使用。
    pub const FASTEST: SpeedTier = SpeedTier::Steps(256);

    /// 每帧执行的指令批大小。
    pub fn steps(self) -> u64 {
        match self {
            SpeedTier::Steps(n) => n,
        }
    }

    pub fn label(self) -> &'static str {
        match self.steps() {
            1 => "1",
            4 => "4",
            16 => "16",
            64 => "64",
            256 => "256",
            _ => "?",
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
        for _ in 0..4 {
            s = s.next();
        }
        assert_eq!(s, SpeedTier::FASTEST);
        // 最快档保持最快档
        assert_eq!(s.next(), SpeedTier::FASTEST);
    }

    #[test]
    fn prev_cycles_backwards() {
        let s = SpeedTier::FASTEST.prev();
        assert_eq!(s, SpeedTier::Steps(64));
        let s = SpeedTier::Steps(1).prev();
        assert_eq!(s, SpeedTier::Steps(1)); // clamped
    }

    #[test]
    fn steps_and_labels() {
        assert_eq!(SpeedTier::Steps(1).steps(), 1);
        assert_eq!(SpeedTier::Steps(256).steps(), 256);
        assert_eq!(SpeedTier::Steps(1).label(), "1");
        assert_eq!(SpeedTier::Steps(4).label(), "4");
        assert_eq!(SpeedTier::Steps(16).label(), "16");
        assert_eq!(SpeedTier::Steps(64).label(), "64");
        assert_eq!(SpeedTier::FASTEST.label(), "256");
    }
}
