//! 双精度平方根。
//!
//! 先把输入缩放成偶指数，再用整数平方根得到足够多的结果位，
//! 非零余数合并为 sticky 位，最后由 [`round_pack64`] 做唯一一次舍入。

use crate::flags::Flags;
use crate::format::Fp64;
use crate::round::RoundingMode;
use crate::softfloat::{Kind, round_pack64, unpack64};
use crate::sqrt::isqrt_u128;

/// 双精度平方根。
///
/// `-0` 保留符号，负的有限非零数和 `-Inf` 返回 canonical NaN 并置
/// `INVALID`。输入输出都是原始位模式。
pub fn sqrt64(a: Fp64, mode: RoundingMode) -> (Fp64, Flags) {
    if a.is_nan() {
        let flags = if a.is_signaling_nan() {
            Flags::INVALID
        } else {
            Flags::empty()
        };
        return (Fp64::canonical_nan(), flags);
    }

    if a.is_zero() {
        return (a, Flags::empty());
    }

    if a.is_inf() {
        if a.sign() {
            return (Fp64::canonical_nan(), Flags::INVALID);
        }
        return (a, Flags::empty());
    }

    if a.sign() {
        return (Fp64::canonical_nan(), Flags::INVALID);
    }

    let ua = unpack64(a.to_bits());
    debug_assert!(matches!(ua.kind, Kind::Finite));

    let mut significand = ua.significand;
    let mut exponent = ua.exponent;
    if exponent & 1 != 0 {
        significand <<= 1;
        exponent -= 1;
    }

    let bits = 64 - significand.leading_zeros();
    let shift = 126 - bits;
    let scale = shift / 2;
    let radicand = (significand as u128) << (2 * scale);
    let root = isqrt_u128(radicand) as u128;
    let remainder = radicand - root * root;
    let rounded = if remainder == 0 { root } else { root | 1 };

    round_pack64(false, exponent / 2 - scale as i32, rounded, mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO: u64 = 0x0000_0000_0000_0000;
    const ONE: u64 = 0x3ff0_0000_0000_0000;
    const TWO: u64 = 0x4000_0000_0000_0000;
    const THREE: u64 = 0x4008_0000_0000_0000;
    const FOUR: u64 = 0x4010_0000_0000_0000;
    const NINE: u64 = 0x4022_0000_0000_0000;
    const HALF: u64 = 0x3fe0_0000_0000_0000;
    const QUARTER: u64 = 0x3fd0_0000_0000_0000;
    const ONE_AND_HALF: u64 = 0x3ff8_0000_0000_0000;
    const TWO_AND_QUARTER: u64 = 0x4002_0000_0000_0000;
    const MIN_SUBNORMAL: u64 = 0x0000_0000_0000_0001;
    const TWO_MIN_SUBNORMAL: u64 = 0x0000_0000_0000_0002;
    const FOUR_MIN_SUBNORMAL: u64 = 0x0000_0000_0000_0004;
    const MIN_NORMAL: u64 = 0x0010_0000_0000_0000;
    const MAX_FINITE: u64 = 0x7fef_ffff_ffff_ffff;
    const NEG_INF: u64 = 0xfff0_0000_0000_0000;
    const INF: u64 = 0x7ff0_0000_0000_0000;
    const QUIET_NAN: u64 = 0x7ff8_0000_0000_0000;
    const SIGNALING_NAN: u64 = 0x7ff0_0000_0000_0001;
    const NEG_ZERO: u64 = 0x8000_0000_0000_0000;

    fn f64(bits: u64) -> Fp64 {
        Fp64::from_bits(bits)
    }

    fn sqrt(a: u64, mode: RoundingMode) -> (u64, Flags) {
        let (value, flags) = sqrt64(f64(a), mode);
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
            (0x2000_0000_0000_0000, Flags::empty())
        );
        assert_eq!(
            sqrt(MIN_SUBNORMAL, RoundingMode::Rne),
            (0x1e60_0000_0000_0000, Flags::empty())
        );
        assert_eq!(
            sqrt(FOUR_MIN_SUBNORMAL, RoundingMode::Rne),
            (0x1e70_0000_0000_0000, Flags::empty())
        );
    }

    #[test]
    fn irrational_square_roots_round_by_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, 0x3ff6_a09e_667f_3bcd),
            (RoundingMode::Rtz, 0x3ff6_a09e_667f_3bcc),
            (RoundingMode::Rdn, 0x3ff6_a09e_667f_3bcc),
            (RoundingMode::Rup, 0x3ff6_a09e_667f_3bcd),
            (RoundingMode::Rmm, 0x3ff6_a09e_667f_3bcd),
        ] {
            assert_eq!(sqrt(TWO, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn subnormal_input_is_supported() {
        for (mode, expected) in [
            (RoundingMode::Rne, 0x1e66_a09e_667f_3bcd),
            (RoundingMode::Rtz, 0x1e66_a09e_667f_3bcc),
            (RoundingMode::Rdn, 0x1e66_a09e_667f_3bcc),
            (RoundingMode::Rup, 0x1e66_a09e_667f_3bcd),
            (RoundingMode::Rmm, 0x1e66_a09e_667f_3bcd),
        ] {
            assert_eq!(sqrt(TWO_MIN_SUBNORMAL, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn largest_finite_input_does_not_overflow() {
        for (mode, expected) in [
            (RoundingMode::Rne, 0x5fef_ffff_ffff_ffff),
            (RoundingMode::Rtz, 0x5fef_ffff_ffff_ffff),
            (RoundingMode::Rdn, 0x5fef_ffff_ffff_ffff),
            (RoundingMode::Rup, 0x5ff0_0000_0000_0000),
            (RoundingMode::Rmm, 0x5fef_ffff_ffff_ffff),
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
}
