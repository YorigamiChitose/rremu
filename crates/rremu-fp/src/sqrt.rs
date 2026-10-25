//! 单精度平方根。
//!
//! 先把输入缩放成偶指数，再用整数平方根得到足够多的结果位，
//! 非零余数合并为 sticky 位，最后由 [`round_pack32`] 做唯一一次舍入。

use crate::flags::Flags;
use crate::format::Fp32;
use crate::round::RoundingMode;
use crate::softfloat::{Kind, round_pack32, unpack32};

/// 单精度平方根。
///
/// `-0` 保留符号，负的有限非零数和 `-Inf` 返回 canonical NaN 并置
/// `INVALID`。输入输出都是原始位模式。
pub fn sqrt32(a: Fp32, mode: RoundingMode) -> (Fp32, Flags) {
    if a.is_nan() {
        let flags = if a.is_signaling_nan() {
            Flags::INVALID
        } else {
            Flags::empty()
        };
        return (Fp32::canonical_nan(), flags);
    }

    if a.is_zero() {
        return (a, Flags::empty());
    }

    if a.is_inf() {
        if a.sign() {
            return (Fp32::canonical_nan(), Flags::INVALID);
        }
        return (a, Flags::empty());
    }

    if a.sign() {
        return (Fp32::canonical_nan(), Flags::INVALID);
    }

    let ua = unpack32(a.to_bits());
    debug_assert!(matches!(ua.kind, Kind::Finite));

    let mut significand = ua.significand;
    let mut exponent = ua.exponent;
    if exponent & 1 != 0 {
        significand <<= 1;
        exponent -= 1;
    }

    let bits = 32 - significand.leading_zeros();
    let shift = 126 - bits;
    let scale = shift / 2;
    let radicand = (significand as u128) << (2 * scale);
    let root = isqrt_u128(radicand) as u128;
    let remainder = radicand - root * root;
    let rounded = if remainder == 0 { root } else { root | 1 };

    round_pack32(false, exponent / 2 - scale as i32, rounded, mode)
}

/// 返回 `value` 的整数平方根，只使用整数运算。
pub(crate) fn isqrt_u128(value: u128) -> u64 {
    let mut low = 0u64;
    let mut high = 1u64 << 63;

    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if (mid as u128) * (mid as u128) <= value {
            low = mid;
        } else {
            high = mid;
        }
    }

    low
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO: u32 = 0x0000_0000;
    const NEG_ZERO: u32 = 0x8000_0000;
    const ONE: u32 = 0x3f80_0000;
    const TWO: u32 = 0x4000_0000;
    const THREE: u32 = 0x4040_0000;
    const FOUR: u32 = 0x4080_0000;
    const NINE: u32 = 0x4110_0000;
    const HALF: u32 = 0x3f00_0000;
    const QUARTER: u32 = 0x3e80_0000;
    const ONE_AND_HALF: u32 = 0x3fc0_0000;
    const TWO_AND_QUARTER: u32 = 0x4010_0000;
    const MIN_SUBNORMAL: u32 = 0x0000_0001;
    const TWO_MIN_SUBNORMAL: u32 = 0x0000_0002;
    const MIN_NORMAL: u32 = 0x0080_0000;
    const MAX_FINITE: u32 = 0x7f7f_ffff;
    const INF: u32 = 0x7f80_0000;
    const NEG_INF: u32 = 0xff80_0000;
    const QUIET_NAN: u32 = 0x7fc0_0000;
    const SIGNALING_NAN: u32 = 0x7f80_0001;

    fn f32(bits: u32) -> Fp32 {
        Fp32::from_bits(bits)
    }

    fn sqrt(a: u32, mode: RoundingMode) -> (u32, Flags) {
        let (value, flags) = sqrt32(f32(a), mode);
        (value.to_bits(), flags)
    }

    #[test]
    fn exact_square_roots() {
        assert_eq!(sqrt(ONE, RoundingMode::Rne), (ONE, Flags::empty()));
        assert_eq!(sqrt(FOUR, RoundingMode::Rne), (TWO, Flags::empty()));
        assert_eq!(sqrt(NINE, RoundingMode::Rne), (THREE, Flags::empty()));
        assert_eq!(sqrt(QUARTER, RoundingMode::Rne), (HALF, Flags::empty()));
        assert_eq!(
            sqrt(TWO_AND_QUARTER, RoundingMode::Rne),
            (ONE_AND_HALF, Flags::empty())
        );
        assert_eq!(
            sqrt(MIN_NORMAL, RoundingMode::Rne),
            (0x2000_0000, Flags::empty())
        );
        assert_eq!(
            sqrt(TWO_MIN_SUBNORMAL, RoundingMode::Rne),
            (0x1a80_0000, Flags::empty())
        );
    }

    #[test]
    fn irrational_square_roots_round_by_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, 0x3fb5_04f3),
            (RoundingMode::Rtz, 0x3fb5_04f3),
            (RoundingMode::Rdn, 0x3fb5_04f3),
            (RoundingMode::Rup, 0x3fb5_04f4),
            (RoundingMode::Rmm, 0x3fb5_04f3),
        ] {
            assert_eq!(sqrt(TWO, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn subnormal_input_is_supported() {
        for (mode, expected) in [
            (RoundingMode::Rne, 0x1a35_04f3),
            (RoundingMode::Rtz, 0x1a35_04f3),
            (RoundingMode::Rdn, 0x1a35_04f3),
            (RoundingMode::Rup, 0x1a35_04f4),
            (RoundingMode::Rmm, 0x1a35_04f3),
        ] {
            assert_eq!(sqrt(MIN_SUBNORMAL, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn largest_finite_input_does_not_overflow() {
        for (mode, expected) in [
            (RoundingMode::Rne, 0x5f7f_ffff),
            (RoundingMode::Rtz, 0x5f7f_ffff),
            (RoundingMode::Rdn, 0x5f7f_ffff),
            (RoundingMode::Rup, 0x5f80_0000),
            (RoundingMode::Rmm, 0x5f7f_ffff),
        ] {
            assert_eq!(sqrt(MAX_FINITE, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn zeros_preserve_sign() {
        assert_eq!(sqrt(ZERO, RoundingMode::Rne), (ZERO, Flags::empty()));
        assert_eq!(
            sqrt(NEG_ZERO, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );
    }

    #[test]
    fn negative_inputs_are_invalid() {
        assert_eq!(
            sqrt(ONE | NEG_ZERO, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            sqrt(MIN_SUBNORMAL | NEG_ZERO, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            sqrt(NEG_INF, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
    }

    #[test]
    fn infinities_and_nans() {
        assert_eq!(sqrt(INF, RoundingMode::Rne), (INF, Flags::empty()));
        assert_eq!(
            sqrt(QUIET_NAN, RoundingMode::Rne),
            (QUIET_NAN, Flags::empty())
        );
        assert_eq!(
            sqrt(SIGNALING_NAN, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
    }

    #[test]
    fn integer_square_root_is_exact() {
        for (value, expected) in [(0, 0), (1, 1), (2, 1), (3, 1), (4, 2), (15, 3), (16, 4)] {
            assert_eq!(isqrt_u128(value), expected);
        }
        assert_eq!(isqrt_u128(1u128 << 124), 1u64 << 62);
        assert_eq!(isqrt_u128((1u128 << 124) - 1), (1u64 << 62) - 1);
    }
}
