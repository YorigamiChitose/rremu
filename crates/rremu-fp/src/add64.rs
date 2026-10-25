//! 双精度加法和减法。
//!
//! 运算先把两个有限值对齐到同一个最低有效位指数，低位用 sticky 位合并，
//! 再由 [`round_pack64`] 做唯一一次舍入。整个过程只使用整数和位操作。

use crate::flags::Flags;
use crate::format::Fp64;
use crate::round::RoundingMode;
use crate::softfloat::{Kind, round_pack64, unpack64};

/// 对齐时在目标最低有效位以下保留的位数。
const GUARD_BITS: i32 = 7;

/// 双精度加法。
///
/// 输入输出都是原始位模式；NaN 结果统一为 RISC-V canonical NaN，
/// signaling NaN 同时置 `INVALID`。
pub fn add64(a: Fp64, b: Fp64, mode: RoundingMode) -> (Fp64, Flags) {
    add_or_sub64(a, b, false, mode)
}

/// 双精度减法。
///
/// 等价于把 `b` 的符号翻转后做加法；零、Inf 和 NaN 的符号规则与加法一致。
pub fn sub64(a: Fp64, b: Fp64, mode: RoundingMode) -> (Fp64, Flags) {
    add_or_sub64(a, b, true, mode)
}

fn add_or_sub64(a: Fp64, b: Fp64, negate_b: bool, mode: RoundingMode) -> (Fp64, Flags) {
    let sign_b = b.sign() ^ negate_b;

    if a.is_nan() || b.is_nan() {
        let flags = if a.is_signaling_nan() || b.is_signaling_nan() {
            Flags::INVALID
        } else {
            Flags::empty()
        };
        return (Fp64::canonical_nan(), flags);
    }

    if a.is_inf() || b.is_inf() {
        if a.is_inf() && b.is_inf() {
            if a.sign() != sign_b {
                return (Fp64::canonical_nan(), Flags::INVALID);
            }
            return (a, Flags::empty());
        }
        if a.is_inf() {
            return (a, Flags::empty());
        }
        return (b.with_sign(sign_b), Flags::empty());
    }

    if a.is_zero() && b.is_zero() {
        let sign = if a.sign() == sign_b {
            a.sign()
        } else {
            mode == RoundingMode::Rdn
        };
        return (zero64(sign), Flags::empty());
    }

    if a.is_zero() {
        return (b.with_sign(sign_b), Flags::empty());
    }
    if b.is_zero() {
        return (a, Flags::empty());
    }

    let ua = unpack64(a.to_bits());
    let ub = unpack64(b.to_bits());
    debug_assert!(matches!(ua.kind, Kind::Finite));
    debug_assert!(matches!(ub.kind, Kind::Finite));

    let lead_a = ua.exponent + (63 - ua.significand.leading_zeros()) as i32;
    let lead_b = ub.exponent + (63 - ub.significand.leading_zeros()) as i32;
    let max_lead = lead_a.max(lead_b);
    let work_exponent = max_lead - (Fp64::FRAC_MASK.count_ones() as i32) - GUARD_BITS;

    let aligned_a = align64(ua.significand, work_exponent - ua.exponent);
    let aligned_b = align64(ub.significand, work_exponent - ub.exponent);
    let signed_a = if ua.sign {
        -(aligned_a as i128)
    } else {
        aligned_a as i128
    };
    let signed_b = if sign_b {
        -(aligned_b as i128)
    } else {
        aligned_b as i128
    };

    let sum = signed_a + signed_b;
    if sum == 0 {
        return (zero64(mode == RoundingMode::Rdn), Flags::empty());
    }

    let sign = sum < 0;
    round_pack64(sign, work_exponent, sum.unsigned_abs(), mode)
}

fn align64(significand: u64, shift: i32) -> u128 {
    if shift >= 0 {
        shift_right_jam64(significand as u128, shift as u32)
    } else {
        let left = (-shift) as u32;
        debug_assert!(left < 128, "left shift out of range");
        (significand as u128) << left
    }
}

/// 右移并把所有被移出的非零位合并到最低位。
fn shift_right_jam64(value: u128, shift: u32) -> u128 {
    if shift == 0 || value == 0 {
        return value;
    }
    if shift >= 128 {
        return 1;
    }
    let kept = value >> shift;
    let discarded = value & ((1u128 << shift) - 1);
    if discarded == 0 { kept } else { kept | 1 }
}

const fn zero64(sign: bool) -> Fp64 {
    Fp64::from_bits((sign as u64) << 63)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO: u64 = 0x0000_0000_0000_0000;
    const ONE: u64 = 0x3ff0_0000_0000_0000;
    const TWO: u64 = 0x4000_0000_0000_0000;
    const THREE: u64 = 0x4008_0000_0000_0000;
    const HALF: u64 = 0x3fe0_0000_0000_0000;
    const NEG_HALF: u64 = 0xbfe0_0000_0000_0000;
    const ONE_AND_HALF: u64 = 0x3ff8_0000_0000_0000;
    const ONE_PLUS_ULP: u64 = 0x3ff0_0000_0000_0001;
    const ONE_MINUS_ULP: u64 = 0x3fef_ffff_ffff_ffff;
    const TWO_POW_MINUS_52: u64 = 0x3cb0_0000_0000_0000;
    const TWO_POW_MINUS_53: u64 = 0x3ca0_0000_0000_0000;
    const TWO_POW_MINUS_54: u64 = 0x3c90_0000_0000_0000;
    const TWO_POW_MINUS_55: u64 = 0x3c80_0000_0000_0000;
    const MIN_SUBNORMAL: u64 = 0x0000_0000_0000_0001;
    const MAX_SUBNORMAL: u64 = 0x000f_ffff_ffff_ffff;
    const MIN_NORMAL: u64 = 0x0010_0000_0000_0000;
    const MAX_FINITE: u64 = 0x7fef_ffff_ffff_ffff;
    const NEG_MAX_FINITE: u64 = 0xffef_ffff_ffff_ffff;
    const INF: u64 = 0x7ff0_0000_0000_0000;
    const NEG_INF: u64 = 0xfff0_0000_0000_0000;
    const QUIET_NAN: u64 = 0x7ff8_0000_0000_0000;
    const SIGNALING_NAN: u64 = 0x7ff0_0000_0000_0001;
    const NEG_ZERO: u64 = 0x8000_0000_0000_0000;

    fn f64(bits: u64) -> Fp64 {
        Fp64::from_bits(bits)
    }

    fn add(a: u64, b: u64, mode: RoundingMode) -> (u64, Flags) {
        let (value, flags) = add64(f64(a), f64(b), mode);
        (value.to_bits(), flags)
    }

    fn sub(a: u64, b: u64, mode: RoundingMode) -> (u64, Flags) {
        let (value, flags) = sub64(f64(a), f64(b), mode);
        (value.to_bits(), flags)
    }

    #[test]
    fn exact_addition_and_subtraction() {
        assert_eq!(add(ONE, TWO, RoundingMode::Rne), (THREE, Flags::empty()));
        assert_eq!(
            add(ONE, HALF, RoundingMode::Rne),
            (ONE_AND_HALF, Flags::empty())
        );
        assert_eq!(
            sub(HALF, ONE, RoundingMode::Rne),
            (NEG_HALF, Flags::empty())
        );
        assert_eq!(add(HALF, HALF, RoundingMode::Rne), (ONE, Flags::empty()));
        assert_eq!(
            add(ONE, TWO_POW_MINUS_52, RoundingMode::Rne),
            (ONE_PLUS_ULP, Flags::empty())
        );
        assert_eq!(
            sub(ONE, TWO_POW_MINUS_53, RoundingMode::Rne),
            (ONE_MINUS_ULP, Flags::empty())
        );
    }

    #[test]
    fn addition_tie_rounds_by_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, ONE),
            (RoundingMode::Rtz, ONE),
            (RoundingMode::Rdn, ONE),
            (RoundingMode::Rup, ONE_PLUS_ULP),
            (RoundingMode::Rmm, ONE_PLUS_ULP),
        ] {
            assert_eq!(add(ONE, TWO_POW_MINUS_53, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn subtraction_tie_rounds_by_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, ONE),
            (RoundingMode::Rtz, ONE_MINUS_ULP),
            (RoundingMode::Rdn, ONE_MINUS_ULP),
            (RoundingMode::Rup, ONE),
            (RoundingMode::Rmm, ONE),
        ] {
            assert_eq!(sub(ONE, TWO_POW_MINUS_54, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn addition_below_tie_rounds_by_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, ONE),
            (RoundingMode::Rtz, ONE),
            (RoundingMode::Rdn, ONE),
            (RoundingMode::Rup, ONE_PLUS_ULP),
            (RoundingMode::Rmm, ONE),
        ] {
            assert_eq!(add(ONE, TWO_POW_MINUS_54, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn subtraction_below_tie_rounds_by_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, ONE),
            (RoundingMode::Rtz, ONE_MINUS_ULP),
            (RoundingMode::Rdn, ONE_MINUS_ULP),
            (RoundingMode::Rup, ONE),
            (RoundingMode::Rmm, ONE),
        ] {
            assert_eq!(sub(ONE, TWO_POW_MINUS_55, mode), (expected, Flags::INEXACT));
        }
    }

    #[test]
    fn signed_zero_results() {
        assert_eq!(
            add(NEG_ZERO, NEG_ZERO, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(add(0, NEG_ZERO, RoundingMode::Rne), (0, Flags::empty()));
        assert_eq!(
            add(0, NEG_ZERO, RoundingMode::Rdn),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            sub(NEG_ZERO, ZERO, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(sub(ZERO, ZERO, RoundingMode::Rne), (0, Flags::empty()));
        assert_eq!(
            sub(ZERO, ZERO, RoundingMode::Rdn),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            sub(NEG_ZERO, NEG_ZERO, RoundingMode::Rne),
            (0, Flags::empty())
        );
        assert_eq!(
            sub(NEG_ZERO, NEG_ZERO, RoundingMode::Rdn),
            (NEG_ZERO, Flags::empty())
        );
    }

    #[test]
    fn infinities_and_nans() {
        assert_eq!(add(INF, INF, RoundingMode::Rne), (INF, Flags::empty()));
        assert_eq!(
            add(NEG_INF, NEG_INF, RoundingMode::Rne),
            (NEG_INF, Flags::empty())
        );
        assert_eq!(add(INF, ONE, RoundingMode::Rne), (INF, Flags::empty()));
        assert_eq!(
            add(ONE, NEG_INF, RoundingMode::Rne),
            (NEG_INF, Flags::empty())
        );
        assert_eq!(
            add(INF, NEG_INF, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            sub(INF, INF, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(sub(INF, NEG_INF, RoundingMode::Rne), (INF, Flags::empty()));
        assert_eq!(
            sub(NEG_INF, INF, RoundingMode::Rne),
            (NEG_INF, Flags::empty())
        );
        assert_eq!(
            add(QUIET_NAN, ONE, RoundingMode::Rne),
            (QUIET_NAN, Flags::empty())
        );
        assert_eq!(
            add(SIGNALING_NAN, ONE, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            add(ONE, SIGNALING_NAN, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
    }

    #[test]
    fn subnormal_addition_is_exact() {
        assert_eq!(
            add(MIN_SUBNORMAL, MIN_SUBNORMAL, RoundingMode::Rne),
            (MIN_SUBNORMAL << 1, Flags::empty())
        );
        assert_eq!(
            add(MAX_SUBNORMAL, MIN_SUBNORMAL, RoundingMode::Rne),
            (MIN_NORMAL, Flags::empty())
        );
    }

    #[test]
    fn overflow_respects_rounding_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, INF),
            (RoundingMode::Rtz, MAX_FINITE),
            (RoundingMode::Rdn, MAX_FINITE),
            (RoundingMode::Rup, INF),
            (RoundingMode::Rmm, INF),
        ] {
            assert_eq!(
                add(MAX_FINITE, MAX_FINITE, mode),
                (expected, Flags::OVERFLOW | Flags::INEXACT)
            );
        }

        for (mode, expected) in [
            (RoundingMode::Rne, NEG_INF),
            (RoundingMode::Rtz, NEG_MAX_FINITE),
            (RoundingMode::Rdn, NEG_INF),
            (RoundingMode::Rup, NEG_MAX_FINITE),
            (RoundingMode::Rmm, NEG_INF),
        ] {
            assert_eq!(
                add(NEG_MAX_FINITE, NEG_MAX_FINITE, mode),
                (expected, Flags::OVERFLOW | Flags::INEXACT)
            );
        }
    }
}
