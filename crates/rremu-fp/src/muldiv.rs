//! 单精度乘法和除法。
//!
//! 乘法直接使用整数 significand 的精确乘积；除法用整数除法得到足够的
//! 商位，并把非零余数合并为 sticky 位。两种情况都只调用
//! [`round_pack32`] 做一次舍入。

use crate::flags::Flags;
use crate::format::Fp32;
use crate::round::RoundingMode;
use crate::softfloat::{Kind, round_pack32, unpack32};

/// 单精度乘法。
///
/// 输入输出都是原始位模式；NaN 结果统一为 RISC-V canonical NaN，
/// signaling NaN 同时置 `INVALID`。`0 * Inf` 也置 `INVALID`。
pub fn mul32(a: Fp32, b: Fp32, mode: RoundingMode) -> (Fp32, Flags) {
    let sign = a.sign() ^ b.sign();

    if a.is_nan() || b.is_nan() {
        return nan_result32(a, b);
    }

    if a.is_inf() || b.is_inf() {
        if a.is_zero() || b.is_zero() {
            return (Fp32::canonical_nan(), Flags::INVALID);
        }
        return (inf32(sign), Flags::empty());
    }

    if a.is_zero() || b.is_zero() {
        return (zero32(sign), Flags::empty());
    }

    let ua = unpack32(a.to_bits());
    let ub = unpack32(b.to_bits());
    debug_assert!(matches!(ua.kind, Kind::Finite));
    debug_assert!(matches!(ub.kind, Kind::Finite));

    let significand = (ua.significand as u128) * (ub.significand as u128);
    round_pack32(sign, ua.exponent + ub.exponent, significand, mode)
}

/// 单精度除法。
///
/// 有限非零数除以零时返回带符号的 Inf 并置 `DIVIDE_BY_ZERO`；
/// `0 / 0` 和 `Inf / Inf` 返回 canonical NaN 并置 `INVALID`。
pub fn div32(a: Fp32, b: Fp32, mode: RoundingMode) -> (Fp32, Flags) {
    let sign = a.sign() ^ b.sign();

    if a.is_nan() || b.is_nan() {
        return nan_result32(a, b);
    }

    if a.is_inf() || b.is_inf() {
        if a.is_inf() && b.is_inf() {
            return (Fp32::canonical_nan(), Flags::INVALID);
        }
        if a.is_inf() {
            return (inf32(sign), Flags::empty());
        }
        return (zero32(sign), Flags::empty());
    }

    if b.is_zero() {
        if a.is_zero() {
            return (Fp32::canonical_nan(), Flags::INVALID);
        }
        return (inf32(sign), Flags::DIVIDE_BY_ZERO);
    }

    if a.is_zero() {
        return (zero32(sign), Flags::empty());
    }

    let ua = unpack32(a.to_bits());
    let ub = unpack32(b.to_bits());
    debug_assert!(matches!(ua.kind, Kind::Finite));
    debug_assert!(matches!(ub.kind, Kind::Finite));

    let ma = 31 - ua.significand.leading_zeros();
    let mb = 31 - ub.significand.leading_zeros();
    let shift = 64 - ma + mb;
    let numerator = (ua.significand as u128) << shift;
    let divisor = ub.significand as u128;
    let quotient = numerator / divisor;
    let remainder = numerator % divisor;
    let significand = if remainder == 0 {
        quotient
    } else {
        quotient | 1
    };

    round_pack32(
        sign,
        ua.exponent - ub.exponent - shift as i32,
        significand,
        mode,
    )
}

fn nan_result32(a: Fp32, b: Fp32) -> (Fp32, Flags) {
    let flags = if a.is_signaling_nan() || b.is_signaling_nan() {
        Flags::INVALID
    } else {
        Flags::empty()
    };
    (Fp32::canonical_nan(), flags)
}

const fn zero32(sign: bool) -> Fp32 {
    Fp32::from_bits((sign as u32) << 31)
}

const fn inf32(sign: bool) -> Fp32 {
    Fp32::from_bits(((sign as u32) << 31) | 0x7f80_0000)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: u32 = 0x3f80_0000;
    const TWO: u32 = 0x4000_0000;
    const THREE: u32 = 0x4040_0000;
    const SIX: u32 = 0x40c0_0000;
    const HALF: u32 = 0x3f00_0000;
    const QUARTER: u32 = 0x3e80_0000;
    const ONE_AND_HALF: u32 = 0x3fc0_0000;
    const NEG_ONE: u32 = 0xbf80_0000;
    const NEG_TWO: u32 = 0xc000_0000;
    const NEG_THREE: u32 = 0xc040_0000;
    const NEG_SIX: u32 = 0xc0c0_0000;
    const MIN_SUBNORMAL: u32 = 0x0000_0001;
    const MAX_SUBNORMAL: u32 = 0x007f_ffff;
    const MIN_NORMAL: u32 = 0x0080_0000;
    const MAX_FINITE: u32 = 0x7f7f_ffff;
    const INF: u32 = 0x7f80_0000;
    const NEG_INF: u32 = 0xff80_0000;
    const QUIET_NAN: u32 = 0x7fc0_0000;
    const SIGNALING_NAN: u32 = 0x7f80_0001;
    const NEG_ZERO: u32 = 0x8000_0000;

    fn f32(bits: u32) -> Fp32 {
        Fp32::from_bits(bits)
    }

    fn mul(a: u32, b: u32, mode: RoundingMode) -> (u32, Flags) {
        let (value, flags) = mul32(f32(a), f32(b), mode);
        (value.to_bits(), flags)
    }

    fn div(a: u32, b: u32, mode: RoundingMode) -> (u32, Flags) {
        let (value, flags) = div32(f32(a), f32(b), mode);
        (value.to_bits(), flags)
    }

    #[test]
    fn exact_multiplication() {
        assert_eq!(mul(ONE, TWO, RoundingMode::Rne), (TWO, Flags::empty()));
        assert_eq!(
            mul(ONE_AND_HALF, TWO, RoundingMode::Rne),
            (THREE, Flags::empty())
        );
        assert_eq!(
            mul(HALF, HALF, RoundingMode::Rne),
            (QUARTER, Flags::empty())
        );
        assert_eq!(mul(TWO, THREE, RoundingMode::Rne), (SIX, Flags::empty()));
        assert_eq!(
            mul(NEG_TWO, THREE, RoundingMode::Rne),
            (NEG_SIX, Flags::empty())
        );
        assert_eq!(
            mul(NEG_TWO, NEG_THREE, RoundingMode::Rne),
            (SIX, Flags::empty())
        );
        assert_eq!(
            mul(MIN_SUBNORMAL, TWO, RoundingMode::Rne),
            (2, Flags::empty())
        );
        assert_eq!(
            mul(MAX_SUBNORMAL, TWO, RoundingMode::Rne),
            (0x00ff_fffe, Flags::empty())
        );
    }

    #[test]
    fn exact_division() {
        assert_eq!(div(SIX, TWO, RoundingMode::Rne), (THREE, Flags::empty()));
        assert_eq!(div(ONE, TWO, RoundingMode::Rne), (HALF, Flags::empty()));
        assert_eq!(
            div(THREE, TWO, RoundingMode::Rne),
            (ONE_AND_HALF, Flags::empty())
        );
        assert_eq!(
            div(NEG_SIX, TWO, RoundingMode::Rne),
            (NEG_THREE, Flags::empty())
        );
        assert_eq!(
            div(MIN_NORMAL, TWO, RoundingMode::Rne),
            (0x0040_0000, Flags::empty())
        );
    }

    #[test]
    fn multiplication_rounds_by_mode() {
        let a = 0x3f80_0001;
        for (mode, expected) in [
            (RoundingMode::Rne, 0x3f80_0002),
            (RoundingMode::Rtz, 0x3f80_0002),
            (RoundingMode::Rdn, 0x3f80_0002),
            (RoundingMode::Rup, 0x3f80_0003),
            (RoundingMode::Rmm, 0x3f80_0002),
        ] {
            assert_eq!(mul(a, a, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn division_rounds_by_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, 0x3eaa_aaab),
            (RoundingMode::Rtz, 0x3eaa_aaaa),
            (RoundingMode::Rdn, 0x3eaa_aaaa),
            (RoundingMode::Rup, 0x3eaa_aaab),
            (RoundingMode::Rmm, 0x3eaa_aaab),
        ] {
            assert_eq!(div(ONE, THREE, mode), (expected, Flags::INEXACT));
        }

        for (mode, expected) in [
            (RoundingMode::Rne, 0xbeaa_aaab),
            (RoundingMode::Rtz, 0xbeaa_aaaa),
            (RoundingMode::Rdn, 0xbeaa_aaab),
            (RoundingMode::Rup, 0xbeaa_aaaa),
            (RoundingMode::Rmm, 0xbeaa_aaab),
        ] {
            assert_eq!(div(NEG_ONE, THREE, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn multiplication_underflow_respects_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, 0),
            (RoundingMode::Rtz, 0),
            (RoundingMode::Rdn, 0),
            (RoundingMode::Rup, MIN_SUBNORMAL),
            (RoundingMode::Rmm, MIN_SUBNORMAL),
        ] {
            assert_eq!(
                mul(MIN_SUBNORMAL, HALF, mode),
                (expected, Flags::UNDERFLOW | Flags::INEXACT)
            );
        }
    }

    #[test]
    fn division_underflow_respects_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, 0),
            (RoundingMode::Rtz, 0),
            (RoundingMode::Rdn, 0),
            (RoundingMode::Rup, MIN_SUBNORMAL),
            (RoundingMode::Rmm, MIN_SUBNORMAL),
        ] {
            assert_eq!(
                div(MIN_SUBNORMAL, TWO, mode),
                (expected, Flags::UNDERFLOW | Flags::INEXACT)
            );
        }
    }

    #[test]
    fn multiplication_and_division_overflow() {
        assert_eq!(
            mul(MAX_FINITE, TWO, RoundingMode::Rne),
            (INF, Flags::OVERFLOW | Flags::INEXACT)
        );
        assert_eq!(
            mul(MAX_FINITE, TWO, RoundingMode::Rtz),
            (MAX_FINITE, Flags::OVERFLOW | Flags::INEXACT)
        );
        assert_eq!(
            div(MAX_FINITE, HALF, RoundingMode::Rne),
            (INF, Flags::OVERFLOW | Flags::INEXACT)
        );
        assert_eq!(
            div(MAX_FINITE, HALF, RoundingMode::Rtz),
            (MAX_FINITE, Flags::OVERFLOW | Flags::INEXACT)
        );
    }

    #[test]
    fn divide_by_zero_and_invalid_cases() {
        assert_eq!(div(ONE, 0, RoundingMode::Rne), (INF, Flags::DIVIDE_BY_ZERO));
        assert_eq!(
            div(NEG_ONE, 0, RoundingMode::Rne),
            (NEG_INF, Flags::DIVIDE_BY_ZERO)
        );
        assert_eq!(
            div(ONE, NEG_ZERO, RoundingMode::Rne),
            (NEG_INF, Flags::DIVIDE_BY_ZERO)
        );
        assert_eq!(div(0, 0, RoundingMode::Rne), (QUIET_NAN, Flags::INVALID));
        assert_eq!(
            div(NEG_ZERO, 0, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(div(INF, 0, RoundingMode::Rne), (INF, Flags::empty()));
        assert_eq!(
            div(NEG_INF, NEG_ZERO, RoundingMode::Rne),
            (INF, Flags::empty())
        );
        assert_eq!(mul(INF, 0, RoundingMode::Rne), (QUIET_NAN, Flags::INVALID));
        assert_eq!(
            mul(0, NEG_INF, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
    }

    #[test]
    fn infinities_and_signed_zeros() {
        assert_eq!(
            mul(INF, NEG_TWO, RoundingMode::Rne),
            (NEG_INF, Flags::empty())
        );
        assert_eq!(mul(INF, INF, RoundingMode::Rne), (INF, Flags::empty()));
        assert_eq!(
            mul(INF, NEG_INF, RoundingMode::Rne),
            (NEG_INF, Flags::empty())
        );
        assert_eq!(
            div(NEG_INF, TWO, RoundingMode::Rne),
            (NEG_INF, Flags::empty())
        );
        assert_eq!(
            div(INF, INF, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(div(ONE, INF, RoundingMode::Rne), (0, Flags::empty()));
        assert_eq!(
            div(NEG_ONE, INF, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            div(ONE, NEG_INF, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );

        assert_eq!(
            mul(NEG_TWO, 0, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            mul(NEG_TWO, NEG_ZERO, RoundingMode::Rne),
            (0, Flags::empty())
        );
        assert_eq!(
            mul(0, NEG_ZERO, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            div(0, NEG_TWO, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            div(NEG_ZERO, TWO, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            div(NEG_ZERO, NEG_TWO, RoundingMode::Rne),
            (0, Flags::empty())
        );
    }

    #[test]
    fn nan_propagation_and_priority() {
        assert_eq!(
            mul(QUIET_NAN, ONE, RoundingMode::Rne),
            (QUIET_NAN, Flags::empty())
        );
        assert_eq!(
            mul(ONE, QUIET_NAN, RoundingMode::Rne),
            (QUIET_NAN, Flags::empty())
        );
        assert_eq!(
            mul(SIGNALING_NAN, ONE, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            mul(ONE, SIGNALING_NAN, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            div(QUIET_NAN, ONE, RoundingMode::Rne),
            (QUIET_NAN, Flags::empty())
        );
        assert_eq!(
            div(ONE, SIGNALING_NAN, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            div(SIGNALING_NAN, 0, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
    }
}
