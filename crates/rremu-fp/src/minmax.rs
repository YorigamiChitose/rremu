//! RISC-V `fmin` / `fmax` 语义。
//!
//! 只有一个操作数是 NaN 时返回另一个操作数；两个都是 NaN 时返回
//! canonical NaN。两个零比较时，只要有一个是 `-0`，最小值就是 `-0`；
//! 只要有一个是 `+0`，最大值就是 `+0`；同号零保持原符号。
//! Signaling NaN 输入会置 `INVALID`，即使结果不是 NaN。

use core::cmp::Ordering;

use crate::flags::Flags;
use crate::format::{Fp32, Fp64};

/// 单精度最小值。
pub fn min32(a: Fp32, b: Fp32) -> (Fp32, Flags) {
    let flags = if a.is_signaling_nan() || b.is_signaling_nan() {
        Flags::INVALID
    } else {
        Flags::empty()
    };
    if a.is_nan() && b.is_nan() {
        return (Fp32::canonical_nan(), flags);
    }
    if a.is_nan() {
        return (b, flags);
    }
    if b.is_nan() {
        return (a, flags);
    }
    if a.is_zero() && b.is_zero() {
        let negative = a.sign() || b.sign();
        return (
            Fp32::from_bits(if negative { Fp32::SIGN_MASK } else { 0 }),
            flags,
        );
    }
    if a.partial_cmp(b) == Some(Ordering::Greater) {
        (b, flags)
    } else {
        (a, flags)
    }
}

/// 单精度最大值。
pub fn max32(a: Fp32, b: Fp32) -> (Fp32, Flags) {
    let flags = if a.is_signaling_nan() || b.is_signaling_nan() {
        Flags::INVALID
    } else {
        Flags::empty()
    };
    if a.is_nan() && b.is_nan() {
        return (Fp32::canonical_nan(), flags);
    }
    if a.is_nan() {
        return (b, flags);
    }
    if b.is_nan() {
        return (a, flags);
    }
    if a.is_zero() && b.is_zero() {
        let positive = !a.sign() || !b.sign();
        return (
            Fp32::from_bits(if positive { 0 } else { Fp32::SIGN_MASK }),
            flags,
        );
    }
    if a.partial_cmp(b) == Some(Ordering::Less) {
        (b, flags)
    } else {
        (a, flags)
    }
}

/// 双精度最小值。
pub fn min64(a: Fp64, b: Fp64) -> (Fp64, Flags) {
    let flags = if a.is_signaling_nan() || b.is_signaling_nan() {
        Flags::INVALID
    } else {
        Flags::empty()
    };
    if a.is_nan() && b.is_nan() {
        return (Fp64::canonical_nan(), flags);
    }
    if a.is_nan() {
        return (b, flags);
    }
    if b.is_nan() {
        return (a, flags);
    }
    if a.is_zero() && b.is_zero() {
        let negative = a.sign() || b.sign();
        return (
            Fp64::from_bits(if negative { Fp64::SIGN_MASK } else { 0 }),
            flags,
        );
    }
    if a.partial_cmp(b) == Some(Ordering::Greater) {
        (b, flags)
    } else {
        (a, flags)
    }
}

/// 双精度最大值。
pub fn max64(a: Fp64, b: Fp64) -> (Fp64, Flags) {
    let flags = if a.is_signaling_nan() || b.is_signaling_nan() {
        Flags::INVALID
    } else {
        Flags::empty()
    };
    if a.is_nan() && b.is_nan() {
        return (Fp64::canonical_nan(), flags);
    }
    if a.is_nan() {
        return (b, flags);
    }
    if b.is_nan() {
        return (a, flags);
    }
    if a.is_zero() && b.is_zero() {
        let positive = !a.sign() || !b.sign();
        return (
            Fp64::from_bits(if positive { 0 } else { Fp64::SIGN_MASK }),
            flags,
        );
    }
    if a.partial_cmp(b) == Some(Ordering::Less) {
        (b, flags)
    } else {
        (a, flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO: u32 = 0x0000_0000;
    const NEG_ZERO: u32 = 0x8000_0000;
    const ONE: u32 = 0x3f80_0000;
    const TWO: u32 = 0x4000_0000;
    const NEG_ONE: u32 = 0xbf80_0000;
    const MIN_SUBNORMAL: u32 = 0x0000_0001;
    const NEG_MIN_SUBNORMAL: u32 = 0x8000_0001;
    const MAX_FINITE: u32 = 0x7f7f_ffff;
    const NEG_MAX_FINITE: u32 = 0xff7f_ffff;
    const INF: u32 = 0x7f80_0000;
    const NEG_INF: u32 = 0xff80_0000;
    const QUIET_NAN: u32 = 0x7fc0_0000;
    const SIGNALING_NAN: u32 = 0x7f80_0001;

    fn f32(bits: u32) -> Fp32 {
        Fp32::from_bits(bits)
    }

    fn min(a: u32, b: u32) -> (u32, Flags) {
        let (value, flags) = min32(f32(a), f32(b));
        (value.to_bits(), flags)
    }

    fn max(a: u32, b: u32) -> (u32, Flags) {
        let (value, flags) = max32(f32(a), f32(b));
        (value.to_bits(), flags)
    }

    #[test]
    fn ordered_values() {
        assert_eq!(min(ONE, TWO), (ONE, Flags::empty()));
        assert_eq!(max(ONE, TWO), (TWO, Flags::empty()));
        assert_eq!(min(NEG_ONE, ONE), (NEG_ONE, Flags::empty()));
        assert_eq!(max(NEG_ONE, ONE), (ONE, Flags::empty()));
        assert_eq!(min(INF, MAX_FINITE), (MAX_FINITE, Flags::empty()));
        assert_eq!(max(INF, MAX_FINITE), (INF, Flags::empty()));
        assert_eq!(min(NEG_INF, NEG_MAX_FINITE), (NEG_INF, Flags::empty()));
        assert_eq!(
            max(NEG_INF, NEG_MAX_FINITE),
            (NEG_MAX_FINITE, Flags::empty())
        );
        assert_eq!(min(ONE, ONE), (ONE, Flags::empty()));
        assert_eq!(max(ONE, ONE), (ONE, Flags::empty()));
    }

    #[test]
    fn signed_zero_rules() {
        assert_eq!(min(ZERO, NEG_ZERO), (NEG_ZERO, Flags::empty()));
        assert_eq!(min(NEG_ZERO, ZERO), (NEG_ZERO, Flags::empty()));
        assert_eq!(min(ZERO, ZERO), (ZERO, Flags::empty()));
        assert_eq!(min(NEG_ZERO, NEG_ZERO), (NEG_ZERO, Flags::empty()));

        assert_eq!(max(ZERO, NEG_ZERO), (ZERO, Flags::empty()));
        assert_eq!(max(NEG_ZERO, ZERO), (ZERO, Flags::empty()));
        assert_eq!(max(ZERO, ZERO), (ZERO, Flags::empty()));
        assert_eq!(max(NEG_ZERO, NEG_ZERO), (NEG_ZERO, Flags::empty()));
    }

    #[test]
    fn subnormal_values() {
        assert_eq!(min(MIN_SUBNORMAL, ZERO), (ZERO, Flags::empty()));
        assert_eq!(max(MIN_SUBNORMAL, ZERO), (MIN_SUBNORMAL, Flags::empty()));
        assert_eq!(
            min(NEG_MIN_SUBNORMAL, NEG_ZERO),
            (NEG_MIN_SUBNORMAL, Flags::empty())
        );
        assert_eq!(max(NEG_MIN_SUBNORMAL, NEG_ZERO), (NEG_ZERO, Flags::empty()));
    }

    #[test]
    fn nan_rules_and_signaling_nan_flags() {
        assert_eq!(min(QUIET_NAN, ONE), (ONE, Flags::empty()));
        assert_eq!(min(ONE, QUIET_NAN), (ONE, Flags::empty()));
        assert_eq!(max(QUIET_NAN, ONE), (ONE, Flags::empty()));
        assert_eq!(max(ONE, QUIET_NAN), (ONE, Flags::empty()));

        assert_eq!(min(QUIET_NAN, QUIET_NAN), (QUIET_NAN, Flags::empty()));
        assert_eq!(max(QUIET_NAN, QUIET_NAN), (QUIET_NAN, Flags::empty()));
        assert_eq!(min(SIGNALING_NAN, QUIET_NAN), (QUIET_NAN, Flags::INVALID));
        assert_eq!(max(QUIET_NAN, SIGNALING_NAN), (QUIET_NAN, Flags::INVALID));
        assert_eq!(min(SIGNALING_NAN, ONE), (ONE, Flags::INVALID));
        assert_eq!(max(SIGNALING_NAN, ONE), (ONE, Flags::INVALID));
        assert_eq!(
            min(SIGNALING_NAN, SIGNALING_NAN),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            max(SIGNALING_NAN, SIGNALING_NAN),
            (QUIET_NAN, Flags::INVALID)
        );
    }

    const ZERO64: u64 = 0x0000_0000_0000_0000;
    const NEG_ZERO64: u64 = 0x8000_0000_0000_0000;
    const ONE64: u64 = 0x3ff0_0000_0000_0000;
    const TWO64: u64 = 0x4000_0000_0000_0000;
    const NEG_ONE64: u64 = 0xbff0_0000_0000_0000;
    const MIN_SUBNORMAL64: u64 = 0x0000_0000_0000_0001;
    const NEG_MIN_SUBNORMAL64: u64 = 0x8000_0000_0000_0001;
    const MAX_FINITE64: u64 = 0x7fef_ffff_ffff_ffff;
    const NEG_MAX_FINITE64: u64 = 0xffef_ffff_ffff_ffff;
    const INF64: u64 = 0x7ff0_0000_0000_0000;
    const NEG_INF64: u64 = 0xfff0_0000_0000_0000;
    const QUIET_NAN64: u64 = 0x7ff8_0000_0000_0000;
    const SIGNALING_NAN64: u64 = 0x7ff0_0000_0000_0001;

    fn f64(bits: u64) -> Fp64 {
        Fp64::from_bits(bits)
    }

    fn min64_bits(a: u64, b: u64) -> (u64, Flags) {
        let (value, flags) = min64(f64(a), f64(b));
        (value.to_bits(), flags)
    }

    fn max64_bits(a: u64, b: u64) -> (u64, Flags) {
        let (value, flags) = max64(f64(a), f64(b));
        (value.to_bits(), flags)
    }

    #[test]
    fn double_ordered_values() {
        assert_eq!(min64_bits(ONE64, TWO64), (ONE64, Flags::empty()));
        assert_eq!(max64_bits(ONE64, TWO64), (TWO64, Flags::empty()));
        assert_eq!(min64_bits(NEG_ONE64, ONE64), (NEG_ONE64, Flags::empty()));
        assert_eq!(max64_bits(NEG_ONE64, ONE64), (ONE64, Flags::empty()));
        assert_eq!(
            min64_bits(INF64, MAX_FINITE64),
            (MAX_FINITE64, Flags::empty())
        );
        assert_eq!(max64_bits(INF64, MAX_FINITE64), (INF64, Flags::empty()));
        assert_eq!(
            min64_bits(NEG_INF64, NEG_MAX_FINITE64),
            (NEG_INF64, Flags::empty())
        );
        assert_eq!(
            max64_bits(NEG_INF64, NEG_MAX_FINITE64),
            (NEG_MAX_FINITE64, Flags::empty())
        );
    }

    #[test]
    fn double_signed_zero_rules() {
        assert_eq!(min64_bits(ZERO64, NEG_ZERO64), (NEG_ZERO64, Flags::empty()));
        assert_eq!(min64_bits(NEG_ZERO64, ZERO64), (NEG_ZERO64, Flags::empty()));
        assert_eq!(min64_bits(ZERO64, ZERO64), (ZERO64, Flags::empty()));
        assert_eq!(
            min64_bits(NEG_ZERO64, NEG_ZERO64),
            (NEG_ZERO64, Flags::empty())
        );
        assert_eq!(max64_bits(ZERO64, NEG_ZERO64), (ZERO64, Flags::empty()));
        assert_eq!(max64_bits(NEG_ZERO64, ZERO64), (ZERO64, Flags::empty()));
        assert_eq!(max64_bits(ZERO64, ZERO64), (ZERO64, Flags::empty()));
        assert_eq!(
            max64_bits(NEG_ZERO64, NEG_ZERO64),
            (NEG_ZERO64, Flags::empty())
        );
    }

    #[test]
    fn double_subnormal_values() {
        assert_eq!(
            min64_bits(MIN_SUBNORMAL64, ZERO64),
            (ZERO64, Flags::empty())
        );
        assert_eq!(
            max64_bits(MIN_SUBNORMAL64, ZERO64),
            (MIN_SUBNORMAL64, Flags::empty())
        );
        assert_eq!(
            min64_bits(NEG_MIN_SUBNORMAL64, NEG_ZERO64),
            (NEG_MIN_SUBNORMAL64, Flags::empty())
        );
        assert_eq!(
            max64_bits(NEG_MIN_SUBNORMAL64, NEG_ZERO64),
            (NEG_ZERO64, Flags::empty())
        );
    }

    #[test]
    fn double_nan_rules_and_signaling_nan_flags() {
        assert_eq!(min64_bits(QUIET_NAN64, ONE64), (ONE64, Flags::empty()));
        assert_eq!(max64_bits(ONE64, QUIET_NAN64), (ONE64, Flags::empty()));
        assert_eq!(
            min64_bits(QUIET_NAN64, QUIET_NAN64),
            (QUIET_NAN64, Flags::empty())
        );
        assert_eq!(
            max64_bits(QUIET_NAN64, QUIET_NAN64),
            (QUIET_NAN64, Flags::empty())
        );
        assert_eq!(
            min64_bits(QUIET_NAN64, SIGNALING_NAN64),
            (QUIET_NAN64, Flags::INVALID)
        );
        assert_eq!(
            max64_bits(SIGNALING_NAN64, QUIET_NAN64),
            (QUIET_NAN64, Flags::INVALID)
        );
        assert_eq!(min64_bits(SIGNALING_NAN64, ONE64), (ONE64, Flags::INVALID));
        assert_eq!(max64_bits(SIGNALING_NAN64, ONE64), (ONE64, Flags::INVALID));
        assert_eq!(
            min64_bits(SIGNALING_NAN64, SIGNALING_NAN64),
            (QUIET_NAN64, Flags::INVALID)
        );
        assert_eq!(
            max64_bits(SIGNALING_NAN64, SIGNALING_NAN64),
            (QUIET_NAN64, Flags::INVALID)
        );
    }
}
