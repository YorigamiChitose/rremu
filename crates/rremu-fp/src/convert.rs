//! RISC-V `fcvt.*` 转换原语。
//!
//! 浮点到整数先把有限值按目标舍入模式舍入为整数，再检查 32 位有符号/无符号
//! 范围；越界、NaN 和 Inf 返回规范的饱和值并置 `INVALID`。整数到浮点复用
//! [`round_pack32`] / [`round_pack64`]，浮点到浮点也复用同一套舍入路径。
//!
//! 所有运算都基于整数和位模式，不依赖宿主浮点环境。

use crate::flags::Flags;
use crate::format::{Fp32, Fp64};
use crate::round::RoundingMode;
use crate::softfloat::{Kind, round_pack32, round_pack64, unpack32, unpack64};

const I32_MAX_BITS: u32 = 0x7fff_ffff;
const I32_MIN_BITS: u32 = 0x8000_0000;
const U32_MAX_BITS: u32 = 0xffff_ffff;

/// `fcvt.w.s`：单精度浮点到有符号 32 位整数。
///
/// 返回值是 x 寄存器中的原始 32 位补码。NaN、±Inf 和越界值返回饱和值并置
/// `INVALID`。
pub fn fcvt_w_s(a: Fp32, mode: RoundingMode) -> (u32, Flags) {
    let value = unpack32(a.to_bits());
    to_int(
        value.sign,
        value.kind,
        value.exponent,
        value.significand as u128,
        true,
        mode,
    )
}

/// `fcvt.wu.s`：单精度浮点到无符号 32 位整数。
pub fn fcvt_wu_s(a: Fp32, mode: RoundingMode) -> (u32, Flags) {
    let value = unpack32(a.to_bits());
    to_int(
        value.sign,
        value.kind,
        value.exponent,
        value.significand as u128,
        false,
        mode,
    )
}

/// `fcvt.w.d`：双精度浮点到有符号 32 位整数。
pub fn fcvt_w_d(a: Fp64, mode: RoundingMode) -> (u32, Flags) {
    let value = unpack64(a.to_bits());
    to_int(
        value.sign,
        value.kind,
        value.exponent,
        value.significand as u128,
        true,
        mode,
    )
}

/// `fcvt.wu.d`：双精度浮点到无符号 32 位整数。
pub fn fcvt_wu_d(a: Fp64, mode: RoundingMode) -> (u32, Flags) {
    let value = unpack64(a.to_bits());
    to_int(
        value.sign,
        value.kind,
        value.exponent,
        value.significand as u128,
        false,
        mode,
    )
}

/// `fcvt.s.w`：有符号 32 位整数到单精度浮点。
pub fn fcvt_s_w(value: i32, mode: RoundingMode) -> (Fp32, Flags) {
    round_pack32(value < 0, 0, u128::from(value.unsigned_abs()), mode)
}

/// `fcvt.s.wu`：无符号 32 位整数到单精度浮点。
pub fn fcvt_s_wu(value: u32, mode: RoundingMode) -> (Fp32, Flags) {
    round_pack32(false, 0, u128::from(value), mode)
}

/// `fcvt.d.w`：有符号 32 位整数到双精度浮点。
pub fn fcvt_d_w(value: i32, mode: RoundingMode) -> (Fp64, Flags) {
    round_pack64(value < 0, 0, u128::from(value.unsigned_abs()), mode)
}

/// `fcvt.d.wu`：无符号 32 位整数到双精度浮点。
pub fn fcvt_d_wu(value: u32, mode: RoundingMode) -> (Fp64, Flags) {
    round_pack64(false, 0, u128::from(value), mode)
}

/// `fcvt.s.d`：双精度浮点到单精度浮点，按 `mode` 单次舍入。
pub fn fcvt_s_d(a: Fp64, mode: RoundingMode) -> (Fp32, Flags) {
    if a.is_nan() {
        return (Fp32::canonical_nan(), nan_flags(a.is_signaling_nan()));
    }
    if a.is_inf() {
        return (inf32(a.sign()), Flags::empty());
    }
    if a.is_zero() {
        return (zero32(a.sign()), Flags::empty());
    }
    let value = unpack64(a.to_bits());
    round_pack32(value.sign, value.exponent, value.significand as u128, mode)
}

/// `fcvt.d.s`：单精度浮点到双精度浮点，转换精确。
pub fn fcvt_d_s(a: Fp32) -> (Fp64, Flags) {
    if a.is_nan() {
        return (Fp64::canonical_nan(), nan_flags(a.is_signaling_nan()));
    }
    if a.is_inf() {
        return (inf64(a.sign()), Flags::empty());
    }
    if a.is_zero() {
        return (zero64(a.sign()), Flags::empty());
    }
    let value = unpack32(a.to_bits());
    round_pack64(
        value.sign,
        value.exponent,
        value.significand as u128,
        RoundingMode::Rne,
    )
}

fn to_int(
    sign: bool,
    kind: Kind,
    exponent: i32,
    significand: u128,
    signed: bool,
    mode: RoundingMode,
) -> (u32, Flags) {
    if matches!(kind, Kind::SignalingNan | Kind::QuietNan) {
        return (nan_saturation(signed), Flags::INVALID);
    }
    if matches!(kind, Kind::Infinity) {
        return (inf_saturation(sign, signed), Flags::INVALID);
    }
    if matches!(kind, Kind::Zero) {
        return (0, Flags::empty());
    }

    let (magnitude, inexact) = rounded_integer_magnitude(sign, significand, exponent, mode);
    let Some(magnitude) = magnitude else {
        return (inf_saturation(sign, signed), Flags::INVALID);
    };
    let in_range = if signed {
        if sign {
            magnitude <= (1u128 << 31)
        } else {
            magnitude <= u128::from(I32_MAX_BITS)
        }
    } else if sign {
        magnitude == 0
    } else {
        magnitude <= u128::from(U32_MAX_BITS)
    };
    if !in_range {
        return (inf_saturation(sign, signed), Flags::INVALID);
    }

    let bits = if signed && sign {
        (magnitude as u32).wrapping_neg()
    } else {
        magnitude as u32
    };
    let flags = if inexact {
        Flags::INEXACT
    } else {
        Flags::empty()
    };
    (bits, flags)
}

/// 把 `significand * 2^exponent` 舍入为整数绝对值。
///
/// 返回 `None` 表示结果精确且大到无法用 `u128` 表示；对 32 位目标来说这
/// 一定越界。`inexact` 只在结果可表示时才有意义。
fn rounded_integer_magnitude(
    sign: bool,
    significand: u128,
    exponent: i32,
    mode: RoundingMode,
) -> (Option<u128>, bool) {
    if exponent >= 0 {
        let shift = exponent as u32;
        let bits = 128 - significand.leading_zeros();
        if u64::from(shift) + u64::from(bits) > 128 {
            return (None, false);
        }
        return (Some(significand << shift), false);
    }

    let shift = (-exponent) as u32;
    if shift >= 128 {
        let increment = match mode {
            RoundingMode::Rup => !sign,
            RoundingMode::Rdn => sign,
            _ => false,
        };
        return (Some(u128::from(increment)), true);
    }

    let kept = significand >> shift;
    let discarded = significand & ((1u128 << shift) - 1);
    let guard = (discarded >> (shift - 1)) & 1 != 0;
    let sticky = discarded & ((1u128 << (shift - 1)) - 1) != 0;
    let inexact = guard || sticky;
    let increment = if inexact {
        match mode {
            RoundingMode::Rne => guard && (sticky || kept & 1 != 0),
            RoundingMode::Rmm => guard,
            RoundingMode::Rtz => false,
            RoundingMode::Rdn => sign,
            RoundingMode::Rup => !sign,
        }
    } else {
        false
    };
    (Some(kept + u128::from(increment)), inexact)
}

const fn nan_saturation(signed: bool) -> u32 {
    if signed { I32_MAX_BITS } else { U32_MAX_BITS }
}

const fn inf_saturation(sign: bool, signed: bool) -> u32 {
    if signed {
        if sign { I32_MIN_BITS } else { I32_MAX_BITS }
    } else if sign {
        0
    } else {
        U32_MAX_BITS
    }
}

const fn nan_flags(signaling: bool) -> Flags {
    if signaling {
        Flags::INVALID
    } else {
        Flags::empty()
    }
}

const fn zero32(sign: bool) -> Fp32 {
    Fp32::from_bits((sign as u32) << 31)
}

const fn zero64(sign: bool) -> Fp64 {
    Fp64::from_bits((sign as u64) << 63)
}

const fn inf32(sign: bool) -> Fp32 {
    Fp32::from_bits(((sign as u32) << 31) | 0x7f80_0000)
}

const fn inf64(sign: bool) -> Fp64 {
    Fp64::from_bits(((sign as u64) << 63) | 0x7ff0_0000_0000_0000)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: u32 = 0x3f80_0000;
    const ONE_POINT_FIVE: u32 = 0x3fc0_0000;
    const TWO_POINT_FIVE: u32 = 0x4020_0000;
    const THREE_POINT_FIVE: u32 = 0x4060_0000;
    const HALF: u32 = 0x3f00_0000;
    const NEG_HALF: u32 = 0xbf00_0000;
    const NEG_ONE_POINT_FIVE: u32 = 0xbfc0_0000;
    const TINY: u32 = 0x0d80_0000; // 2^-100
    const NEG_TINY: u32 = 0x8d80_0000;
    const TWO_POW_31: u32 = 0x4f00_0000;
    const NEG_TWO_POW_31: u32 = 0xcf00_0000;
    const BELOW_NEG_TWO_POW_31: u32 = 0xcf00_0001;
    const TWO_POW_32: u32 = 0x4f80_0000;
    const BELOW_TWO_POW_32: u32 = 0x4f7f_ffff;
    const MIN_SUBNORMAL: u32 = 0x0000_0001;
    const INF: u32 = 0x7f80_0000;
    const NEG_INF: u32 = 0xff80_0000;
    const QUIET_NAN: u32 = 0x7fc0_0000;
    const SIGNALING_NAN: u32 = 0x7f80_0001;
    const NEG_ZERO: u32 = 0x8000_0000;

    fn f32(bits: u32) -> Fp32 {
        Fp32::from_bits(bits)
    }

    fn to_i32(bits: u32, mode: RoundingMode) -> (u32, Flags) {
        fcvt_w_s(f32(bits), mode)
    }

    fn to_u32(bits: u32, mode: RoundingMode) -> (u32, Flags) {
        fcvt_wu_s(f32(bits), mode)
    }

    #[test]
    fn float_to_signed_integer_rounding_modes() {
        assert_eq!(to_i32(ONE, RoundingMode::Rne), (1, Flags::empty()));
        assert_eq!(
            to_i32(ONE_POINT_FIVE, RoundingMode::Rne),
            (2, Flags::INEXACT)
        );
        assert_eq!(
            to_i32(TWO_POINT_FIVE, RoundingMode::Rne),
            (2, Flags::INEXACT)
        );
        assert_eq!(
            to_i32(THREE_POINT_FIVE, RoundingMode::Rne),
            (4, Flags::INEXACT)
        );
        assert_eq!(
            to_i32(NEG_ONE_POINT_FIVE, RoundingMode::Rne),
            (0xffff_fffe, Flags::INEXACT)
        );

        for (mode, expected) in [
            (RoundingMode::Rne, 0),
            (RoundingMode::Rtz, 0),
            (RoundingMode::Rdn, 0),
            (RoundingMode::Rup, 1),
            (RoundingMode::Rmm, 1),
        ] {
            assert_eq!(to_i32(HALF, mode), (expected, Flags::INEXACT));
        }
        for (mode, expected, flags) in [
            (RoundingMode::Rne, 0, Flags::INEXACT),
            (RoundingMode::Rtz, 0, Flags::INEXACT),
            (RoundingMode::Rdn, 0xffff_ffff, Flags::INEXACT),
            (RoundingMode::Rup, 0, Flags::INEXACT),
            (RoundingMode::Rmm, 0xffff_ffff, Flags::INEXACT),
        ] {
            assert_eq!(to_i32(NEG_HALF, mode), (expected, flags));
        }

        assert_eq!(to_i32(TINY, RoundingMode::Rup), (1, Flags::INEXACT));
        assert_eq!(
            to_i32(NEG_TINY, RoundingMode::Rdn),
            (0xffff_ffff, Flags::INEXACT)
        );
    }

    #[test]
    fn float_to_integer_boundaries_and_specials() {
        assert_eq!(
            to_i32(TWO_POW_31, RoundingMode::Rtz),
            (I32_MAX_BITS, Flags::INVALID)
        );
        assert_eq!(
            to_i32(NEG_TWO_POW_31, RoundingMode::Rne),
            (I32_MIN_BITS, Flags::empty())
        );
        assert_eq!(
            to_i32(BELOW_NEG_TWO_POW_31, RoundingMode::Rne),
            (I32_MIN_BITS, Flags::INVALID)
        );
        assert_eq!(
            to_i32(INF, RoundingMode::Rne),
            (I32_MAX_BITS, Flags::INVALID)
        );
        assert_eq!(
            to_i32(NEG_INF, RoundingMode::Rne),
            (I32_MIN_BITS, Flags::INVALID)
        );
        assert_eq!(
            to_i32(QUIET_NAN, RoundingMode::Rne),
            (I32_MAX_BITS, Flags::INVALID)
        );
        assert_eq!(
            to_i32(SIGNALING_NAN, RoundingMode::Rne),
            (I32_MAX_BITS, Flags::INVALID)
        );
        assert_eq!(to_i32(NEG_ZERO, RoundingMode::Rne), (0, Flags::empty()));
    }

    #[test]
    fn float_to_unsigned_integer_boundaries_and_specials() {
        assert_eq!(
            to_u32(BELOW_TWO_POW_32, RoundingMode::Rtz),
            (0xffff_ff00, Flags::empty())
        );
        assert_eq!(
            to_u32(TWO_POW_32, RoundingMode::Rtz),
            (U32_MAX_BITS, Flags::INVALID)
        );
        assert_eq!(
            to_u32(NEG_ONE_POINT_FIVE, RoundingMode::Rne),
            (0, Flags::INVALID)
        );
        assert_eq!(to_u32(NEG_HALF, RoundingMode::Rne), (0, Flags::INEXACT));
        assert_eq!(to_u32(NEG_HALF, RoundingMode::Rdn), (0, Flags::INVALID));
        assert_eq!(to_u32(HALF, RoundingMode::Rmm), (1, Flags::INEXACT));
        assert_eq!(
            to_u32(INF, RoundingMode::Rne),
            (U32_MAX_BITS, Flags::INVALID)
        );
        assert_eq!(to_u32(NEG_INF, RoundingMode::Rne), (0, Flags::INVALID));
        assert_eq!(
            to_u32(QUIET_NAN, RoundingMode::Rne),
            (U32_MAX_BITS, Flags::INVALID)
        );
        assert_eq!(to_u32(NEG_ZERO, RoundingMode::Rne), (0, Flags::empty()));
    }

    #[test]
    fn integer_to_single_precision_rounds_by_mode() {
        let value = 0x0100_0001; // 2^24 + 1
        for (mode, expected) in [
            (RoundingMode::Rne, 0x4b80_0000),
            (RoundingMode::Rtz, 0x4b80_0000),
            (RoundingMode::Rdn, 0x4b80_0000),
            (RoundingMode::Rup, 0x4b80_0001),
            (RoundingMode::Rmm, 0x4b80_0001),
        ] {
            let (result, flags) = fcvt_s_w(value, mode);
            assert_eq!(result.to_bits(), expected);
            assert_eq!(flags, Flags::INEXACT);
        }

        let (result, flags) = fcvt_s_w(i32::MIN, RoundingMode::Rne);
        assert_eq!(result.to_bits(), NEG_TWO_POW_31);
        assert_eq!(flags, Flags::empty());

        let (result, flags) = fcvt_s_wu(u32::MAX, RoundingMode::Rne);
        assert_eq!(result.to_bits(), TWO_POW_32);
        assert_eq!(flags, Flags::INEXACT);

        let (result, flags) = fcvt_s_wu(u32::MAX, RoundingMode::Rtz);
        assert_eq!(result.to_bits(), BELOW_TWO_POW_32);
        assert_eq!(flags, Flags::INEXACT);
    }

    #[test]
    fn integer_to_double_is_exact() {
        let (result, flags) = fcvt_d_w(i32::MIN, RoundingMode::Rne);
        assert_eq!(result.to_bits(), 0xc1e0_0000_0000_0000);
        assert_eq!(flags, Flags::empty());

        let (result, flags) = fcvt_d_wu(u32::MAX, RoundingMode::Rne);
        assert_eq!(flags, Flags::empty());
        let (roundtrip, flags) = fcvt_wu_d(result, RoundingMode::Rtz);
        assert_eq!(roundtrip, u32::MAX);
        assert_eq!(flags, Flags::empty());
    }

    #[test]
    fn single_to_double_is_exact_and_preserves_specials() {
        for bits in [ONE, MIN_SUBNORMAL, NEG_ZERO, INF, NEG_INF] {
            let (wide, flags) = fcvt_d_s(f32(bits));
            assert_eq!(flags, Flags::empty());
            let (narrow, flags) = fcvt_s_d(wide, RoundingMode::Rne);
            assert_eq!(narrow.to_bits(), bits);
            assert_eq!(flags, Flags::empty());
        }

        let (nan, flags) = fcvt_d_s(f32(QUIET_NAN));
        assert_eq!(nan, Fp64::canonical_nan());
        assert_eq!(flags, Flags::empty());
        let (nan, flags) = fcvt_d_s(f32(SIGNALING_NAN));
        assert_eq!(nan, Fp64::canonical_nan());
        assert_eq!(flags, Flags::INVALID);
    }

    #[test]
    fn double_to_single_rounds_and_handles_extremes() {
        let one_plus_half_ulp = Fp64::from_bits(0x3ff0_0000_1000_0000); // 1+2^-24
        for (mode, expected) in [
            (RoundingMode::Rne, ONE),
            (RoundingMode::Rtz, ONE),
            (RoundingMode::Rdn, ONE),
            (RoundingMode::Rup, 0x3f80_0001),
            (RoundingMode::Rmm, 0x3f80_0001),
        ] {
            let (result, flags) = fcvt_s_d(one_plus_half_ulp, mode);
            assert_eq!(result.to_bits(), expected);
            assert_eq!(flags, Flags::INEXACT);
        }

        let max = Fp64::from_bits(0x7fef_ffff_ffff_ffff);
        let (result, flags) = fcvt_s_d(max, RoundingMode::Rne);
        assert_eq!(result.to_bits(), 0x7f80_0000);
        assert_eq!(flags, Flags::OVERFLOW | Flags::INEXACT);

        let tiny = Fp64::from_bits(0x3370_0000_0000_0000); // 2^-200
        let (result, flags) = fcvt_s_d(tiny, RoundingMode::Rne);
        assert_eq!(result.to_bits(), 0);
        assert_eq!(flags, Flags::UNDERFLOW | Flags::INEXACT);
        let (result, flags) = fcvt_s_d(tiny, RoundingMode::Rup);
        assert_eq!(result.to_bits(), MIN_SUBNORMAL);
        assert_eq!(flags, Flags::UNDERFLOW | Flags::INEXACT);
    }
}
