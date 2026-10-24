//! IEEE-754 原始位模式与分类。
//!
//! 本模块只解释位模式，不进行算术运算。所有访问器都直接操作位，
//! 因此不会触发宿主的浮点异常或舍入行为。

use core::fmt;

/// RISC-V `fclass` 的 10 个类别，编码顺序与 `fclass` 结果位一致。
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FpClass {
    /// Negative infinity.
    NegativeInfinity = 0,
    /// Negative normal.
    NegativeNormal = 1,
    /// Negative subnormal.
    NegativeSubnormal = 2,
    /// Negative zero.
    NegativeZero = 3,
    /// Positive zero.
    PositiveZero = 4,
    /// Positive subnormal.
    PositiveSubnormal = 5,
    /// Positive normal.
    PositiveNormal = 6,
    /// Positive infinity.
    PositiveInfinity = 7,
    /// Signaling NaN.
    SignalingNan = 8,
    /// Quiet NaN.
    QuietNan = 9,
}

impl FpClass {
    /// 将类别转换为 `fclass` 返回的 10 位掩码。
    pub const fn mask(self) -> u16 {
        1u16 << self as u8
    }
}

/// 单精度浮点的原始 32 位模式。
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Fp32(u32);

impl Fp32 {
    /// Sign bit.
    pub const SIGN_MASK: u32 = 0x8000_0000;
    /// Exponent field mask.
    pub const EXP_MASK: u32 = 0x7f80_0000;
    /// Fraction field mask.
    pub const FRAC_MASK: u32 = 0x007f_ffff;
    /// Quiet-NaN bit.
    pub const QUIET_BIT: u32 = 0x0040_0000;
    /// RISC-V canonical NaN.
    pub const CANONICAL_NAN_BITS: u32 = 0x7fc0_0000;

    /// 从原始位模式构造。
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// 返回原始位模式。
    pub const fn to_bits(self) -> u32 {
        self.0
    }

    /// 符号位是否为 1。
    pub const fn sign(self) -> bool {
        self.0 & Self::SIGN_MASK != 0
    }

    /// 无符号指数域。
    pub const fn exponent(self) -> u32 {
        (self.0 & Self::EXP_MASK) >> 23
    }

    /// 尾数域（不含隐藏位）。
    pub const fn fraction(self) -> u32 {
        self.0 & Self::FRAC_MASK
    }

    /// 是否为 ±0。
    pub const fn is_zero(self) -> bool {
        self.0 & !Self::SIGN_MASK == 0
    }

    /// 是否为次正规数。
    pub const fn is_subnormal(self) -> bool {
        self.exponent() == 0 && self.fraction() != 0
    }

    /// 是否为正规数。
    pub const fn is_normal(self) -> bool {
        let exponent = self.exponent();
        exponent != 0 && exponent != 0xff
    }

    /// 是否为 ±Inf。
    pub const fn is_inf(self) -> bool {
        self.0 & !Self::SIGN_MASK == Self::EXP_MASK
    }

    /// 是否为 NaN。
    pub const fn is_nan(self) -> bool {
        self.exponent() == 0xff && self.fraction() != 0
    }

    /// 是否为 signaling NaN。
    pub const fn is_signaling_nan(self) -> bool {
        self.is_nan() && self.0 & Self::QUIET_BIT == 0
    }

    /// 是否为 quiet NaN。
    pub const fn is_quiet_nan(self) -> bool {
        self.is_nan() && self.0 & Self::QUIET_BIT != 0
    }

    /// 返回 RISC-V canonical NaN。
    pub const fn canonical_nan() -> Self {
        Self(Self::CANONICAL_NAN_BITS)
    }

    /// 返回指定符号位的同一数值。
    pub const fn with_sign(self, negative: bool) -> Self {
        if negative {
            Self(self.0 | Self::SIGN_MASK)
        } else {
            Self(self.0 & !Self::SIGN_MASK)
        }
    }

    /// 按 `fclass` 语义分类。
    pub const fn classify(self) -> FpClass {
        if self.is_signaling_nan() {
            FpClass::SignalingNan
        } else if self.is_quiet_nan() {
            FpClass::QuietNan
        } else if self.is_inf() {
            if self.sign() {
                FpClass::NegativeInfinity
            } else {
                FpClass::PositiveInfinity
            }
        } else if self.is_zero() {
            if self.sign() {
                FpClass::NegativeZero
            } else {
                FpClass::PositiveZero
            }
        } else if self.is_subnormal() {
            if self.sign() {
                FpClass::NegativeSubnormal
            } else {
                FpClass::PositiveSubnormal
            }
        } else if self.sign() {
            FpClass::NegativeNormal
        } else {
            FpClass::PositiveNormal
        }
    }
}

impl From<u32> for Fp32 {
    fn from(bits: u32) -> Self {
        Self::from_bits(bits)
    }
}

impl fmt::Debug for Fp32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fp32(0x{:08x})", self.0)
    }
}

/// 双精度浮点的原始 64 位模式。
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Fp64(u64);

impl Fp64 {
    /// Sign bit.
    pub const SIGN_MASK: u64 = 0x8000_0000_0000_0000;
    /// Exponent field mask.
    pub const EXP_MASK: u64 = 0x7ff0_0000_0000_0000;
    /// Fraction field mask.
    pub const FRAC_MASK: u64 = 0x000f_ffff_ffff_ffff;
    /// Quiet-NaN bit.
    pub const QUIET_BIT: u64 = 0x0008_0000_0000_0000;
    /// RISC-V canonical NaN.
    pub const CANONICAL_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

    /// 从原始位模式构造。
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// 返回原始位模式。
    pub const fn to_bits(self) -> u64 {
        self.0
    }

    /// 符号位是否为 1。
    pub const fn sign(self) -> bool {
        self.0 & Self::SIGN_MASK != 0
    }

    /// 无符号指数域。
    pub const fn exponent(self) -> u32 {
        ((self.0 & Self::EXP_MASK) >> 52) as u32
    }

    /// 尾数域（不含隐藏位）。
    pub const fn fraction(self) -> u64 {
        self.0 & Self::FRAC_MASK
    }

    /// 是否为 ±0。
    pub const fn is_zero(self) -> bool {
        self.0 & !Self::SIGN_MASK == 0
    }

    /// 是否为次正规数。
    pub const fn is_subnormal(self) -> bool {
        self.exponent() == 0 && self.fraction() != 0
    }

    /// 是否为正规数。
    pub const fn is_normal(self) -> bool {
        let exponent = self.exponent();
        exponent != 0 && exponent != 0x7ff
    }

    /// 是否为 ±Inf。
    pub const fn is_inf(self) -> bool {
        self.0 & !Self::SIGN_MASK == Self::EXP_MASK
    }

    /// 是否为 NaN。
    pub const fn is_nan(self) -> bool {
        self.exponent() == 0x7ff && self.fraction() != 0
    }

    /// 是否为 signaling NaN。
    pub const fn is_signaling_nan(self) -> bool {
        self.is_nan() && self.0 & Self::QUIET_BIT == 0
    }

    /// 是否为 quiet NaN。
    pub const fn is_quiet_nan(self) -> bool {
        self.is_nan() && self.0 & Self::QUIET_BIT != 0
    }

    /// 返回 RISC-V canonical NaN。
    pub const fn canonical_nan() -> Self {
        Self(Self::CANONICAL_NAN_BITS)
    }

    /// 返回指定符号位的同一数值。
    pub const fn with_sign(self, negative: bool) -> Self {
        if negative {
            Self(self.0 | Self::SIGN_MASK)
        } else {
            Self(self.0 & !Self::SIGN_MASK)
        }
    }

    /// 按 `fclass` 语义分类。
    pub const fn classify(self) -> FpClass {
        if self.is_signaling_nan() {
            FpClass::SignalingNan
        } else if self.is_quiet_nan() {
            FpClass::QuietNan
        } else if self.is_inf() {
            if self.sign() {
                FpClass::NegativeInfinity
            } else {
                FpClass::PositiveInfinity
            }
        } else if self.is_zero() {
            if self.sign() {
                FpClass::NegativeZero
            } else {
                FpClass::PositiveZero
            }
        } else if self.is_subnormal() {
            if self.sign() {
                FpClass::NegativeSubnormal
            } else {
                FpClass::PositiveSubnormal
            }
        } else if self.sign() {
            FpClass::NegativeNormal
        } else {
            FpClass::PositiveNormal
        }
    }
}

impl From<u64> for Fp64 {
    fn from(bits: u64) -> Self {
        Self::from_bits(bits)
    }
}

impl fmt::Debug for Fp64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fp64(0x{:016x})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_masks_match_fclass_layout() {
        assert_eq!(FpClass::NegativeInfinity.mask(), 1 << 0);
        assert_eq!(FpClass::NegativeNormal.mask(), 1 << 1);
        assert_eq!(FpClass::NegativeSubnormal.mask(), 1 << 2);
        assert_eq!(FpClass::NegativeZero.mask(), 1 << 3);
        assert_eq!(FpClass::PositiveZero.mask(), 1 << 4);
        assert_eq!(FpClass::PositiveSubnormal.mask(), 1 << 5);
        assert_eq!(FpClass::PositiveNormal.mask(), 1 << 6);
        assert_eq!(FpClass::PositiveInfinity.mask(), 1 << 7);
        assert_eq!(FpClass::SignalingNan.mask(), 1 << 8);
        assert_eq!(FpClass::QuietNan.mask(), 1 << 9);
    }

    #[test]
    fn f32_special_values() {
        let pos_zero = Fp32::from_bits(0x0000_0000);
        let neg_zero = Fp32::from_bits(0x8000_0000);
        let pos_inf = Fp32::from_bits(0x7f80_0000);
        let neg_inf = Fp32::from_bits(0xff80_0000);
        let smallest_subnormal = Fp32::from_bits(0x0000_0001);
        let smallest_normal = Fp32::from_bits(0x0080_0000);
        let qnan = Fp32::canonical_nan();
        let snan = Fp32::from_bits(0x7f80_0001);

        assert!(pos_zero.is_zero());
        assert!(neg_zero.is_zero());
        assert!(pos_zero.sign() != neg_zero.sign());
        assert!(pos_inf.is_inf() && !pos_inf.is_nan());
        assert!(neg_inf.is_inf() && neg_inf.sign());
        assert!(smallest_subnormal.is_subnormal());
        assert!(!smallest_subnormal.is_normal());
        assert!(smallest_normal.is_normal());
        assert!(qnan.is_nan() && qnan.is_quiet_nan() && !qnan.is_signaling_nan());
        assert!(snan.is_nan() && snan.is_signaling_nan() && !snan.is_quiet_nan());

        assert_eq!(pos_zero.classify(), FpClass::PositiveZero);
        assert_eq!(neg_zero.classify(), FpClass::NegativeZero);
        assert_eq!(pos_inf.classify(), FpClass::PositiveInfinity);
        assert_eq!(neg_inf.classify(), FpClass::NegativeInfinity);
        assert_eq!(smallest_subnormal.classify(), FpClass::PositiveSubnormal);
        assert_eq!(smallest_normal.classify(), FpClass::PositiveNormal);
        assert_eq!(qnan.classify(), FpClass::QuietNan);
        assert_eq!(snan.classify(), FpClass::SignalingNan);
    }

    #[test]
    fn f64_special_values() {
        let pos_zero = Fp64::from_bits(0x0000_0000_0000_0000);
        let neg_zero = Fp64::from_bits(0x8000_0000_0000_0000);
        let pos_inf = Fp64::from_bits(0x7ff0_0000_0000_0000);
        let neg_inf = Fp64::from_bits(0xfff0_0000_0000_0000);
        let smallest_subnormal = Fp64::from_bits(0x0000_0000_0000_0001);
        let smallest_normal = Fp64::from_bits(0x0010_0000_0000_0000);
        let qnan = Fp64::canonical_nan();
        let snan = Fp64::from_bits(0x7ff0_0000_0000_0001);

        assert!(pos_zero.is_zero());
        assert!(neg_zero.is_zero());
        assert!(pos_zero.sign() != neg_zero.sign());
        assert!(pos_inf.is_inf() && !pos_inf.is_nan());
        assert!(neg_inf.is_inf() && neg_inf.sign());
        assert!(smallest_subnormal.is_subnormal());
        assert!(!smallest_subnormal.is_normal());
        assert!(smallest_normal.is_normal());
        assert!(qnan.is_nan() && qnan.is_quiet_nan() && !qnan.is_signaling_nan());
        assert!(snan.is_nan() && snan.is_signaling_nan() && !snan.is_quiet_nan());

        assert_eq!(pos_zero.classify(), FpClass::PositiveZero);
        assert_eq!(neg_zero.classify(), FpClass::NegativeZero);
        assert_eq!(pos_inf.classify(), FpClass::PositiveInfinity);
        assert_eq!(neg_inf.classify(), FpClass::NegativeInfinity);
        assert_eq!(smallest_subnormal.classify(), FpClass::PositiveSubnormal);
        assert_eq!(smallest_normal.classify(), FpClass::PositiveNormal);
        assert_eq!(qnan.classify(), FpClass::QuietNan);
        assert_eq!(snan.classify(), FpClass::SignalingNan);
    }

    #[test]
    fn sign_injection_keeps_payload() {
        let value = Fp32::from_bits(0x4049_0fdb); // 3.1415927
        assert_eq!(value.with_sign(true).to_bits(), 0xc049_0fdb);
        assert_eq!(value.with_sign(true).with_sign(false), value);
        assert_eq!(Fp32::canonical_nan().with_sign(true).to_bits(), 0xffc0_0000);
    }
}
