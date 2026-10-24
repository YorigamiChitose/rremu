//! RISC-V 舍入模式。
//!
//! 编码与指令中的 `rm` 字段一致：0–4 是静态模式，7 表示使用动态 `frm`。
//! 5 和 6 是保留值，由调用方按非法指令处理。

/// IEEE-754 / RISC-V 舍入模式。
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundingMode {
    /// Round to nearest, ties to even.
    Rne = 0,
    /// Round towards zero.
    Rtz = 1,
    /// Round down (towards negative infinity).
    Rdn = 2,
    /// Round up (towards positive infinity).
    Rup = 3,
    /// Round to nearest, ties to max magnitude.
    Rmm = 4,
}

impl RoundingMode {
    /// 所有静态舍入模式，按编码顺序排列。
    pub const ALL: [Self; 5] = [Self::Rne, Self::Rtz, Self::Rdn, Self::Rup, Self::Rmm];

    /// 从 3 位编码解析静态模式；5、6、7 返回 `None`。
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Rne),
            1 => Some(Self::Rtz),
            2 => Some(Self::Rdn),
            3 => Some(Self::Rup),
            4 => Some(Self::Rmm),
            _ => None,
        }
    }

    /// 返回 3 位编码。
    pub const fn bits(self) -> u8 {
        self as u8
    }

    /// 返回规范名称，便于测试失败信息与调试输出。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rne => "rne",
            Self::Rtz => "rtz",
            Self::Rdn => "rdn",
            Self::Rup => "rup",
            Self::Rmm => "rmm",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_modes_round_trip() {
        for (bits, expected) in [
            (0, RoundingMode::Rne),
            (1, RoundingMode::Rtz),
            (2, RoundingMode::Rdn),
            (3, RoundingMode::Rup),
            (4, RoundingMode::Rmm),
        ] {
            assert_eq!(RoundingMode::from_bits(bits), Some(expected));
            assert_eq!(expected.bits(), bits);
        }
    }

    #[test]
    fn reserved_modes_are_rejected() {
        for bits in 5..=7 {
            assert_eq!(RoundingMode::from_bits(bits), None);
        }
    }

    #[test]
    fn all_contains_each_mode_once() {
        assert_eq!(RoundingMode::ALL.len(), 5);
        for mode in RoundingMode::ALL {
            assert_eq!(RoundingMode::from_bits(mode.bits()), Some(mode));
            assert!(!mode.name().is_empty());
        }
    }
}
