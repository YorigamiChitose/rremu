//! IEEE-754 解码、规范化、舍入和编码原语。
//!
//! 有限值的内部表示统一为 `significand * 2^exponent`：
//! `significand` 是整数，`exponent` 是最低有效位的指数。
//! 正规数的 `significand` 含隐藏位；次正规数不含隐藏位。
//! 所有运算只使用整数和位操作，不依赖宿主浮点环境。

use crate::flags::Flags;
use crate::format::{Fp32, Fp64};
use crate::round::RoundingMode;

/// 解码后的浮点类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// ±0。
    Zero,
    /// 有限非零值，包括正规数和次正规数。
    Finite,
    /// ±Inf。
    Infinity,
    /// signaling NaN。
    SignalingNan,
    /// quiet NaN。
    QuietNan,
}

/// 单精度浮点的解码结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unpacked32 {
    /// 符号位。
    pub sign: bool,
    /// 类别。
    pub kind: Kind,
    /// 最低有效位的指数；`value = significand * 2^exponent`。
    pub exponent: i32,
    /// 整数 significand。
    pub significand: u32,
}

impl Unpacked32 {
    /// 是否为 ±0。
    pub const fn is_zero(self) -> bool {
        matches!(self.kind, Kind::Zero)
    }

    /// 是否为有限值（含 ±0）。
    pub const fn is_finite(self) -> bool {
        matches!(self.kind, Kind::Zero | Kind::Finite)
    }

    /// 是否为 ±Inf。
    pub const fn is_inf(self) -> bool {
        matches!(self.kind, Kind::Infinity)
    }

    /// 是否为 NaN。
    pub const fn is_nan(self) -> bool {
        matches!(self.kind, Kind::SignalingNan | Kind::QuietNan)
    }

    /// 是否为 signaling NaN。
    pub const fn is_signaling_nan(self) -> bool {
        matches!(self.kind, Kind::SignalingNan)
    }

    /// 是否为 quiet NaN。
    pub const fn is_quiet_nan(self) -> bool {
        matches!(self.kind, Kind::QuietNan)
    }
}

/// 双精度浮点的解码结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unpacked64 {
    /// 符号位。
    pub sign: bool,
    /// 类别。
    pub kind: Kind,
    /// 最低有效位的指数；`value = significand * 2^exponent`。
    pub exponent: i32,
    /// 整数 significand。
    pub significand: u64,
}

impl Unpacked64 {
    /// 是否为 ±0。
    pub const fn is_zero(self) -> bool {
        matches!(self.kind, Kind::Zero)
    }

    /// 是否为有限值（含 ±0）。
    pub const fn is_finite(self) -> bool {
        matches!(self.kind, Kind::Zero | Kind::Finite)
    }

    /// 是否为 ±Inf。
    pub const fn is_inf(self) -> bool {
        matches!(self.kind, Kind::Infinity)
    }

    /// 是否为 NaN。
    pub const fn is_nan(self) -> bool {
        matches!(self.kind, Kind::SignalingNan | Kind::QuietNan)
    }

    /// 是否为 signaling NaN。
    pub const fn is_signaling_nan(self) -> bool {
        matches!(self.kind, Kind::SignalingNan)
    }

    /// 是否为 quiet NaN。
    pub const fn is_quiet_nan(self) -> bool {
        matches!(self.kind, Kind::QuietNan)
    }
}

/// 解码单精度位模式。
pub fn unpack32(bits: u32) -> Unpacked32 {
    let sign = bits & Fp32::SIGN_MASK != 0;
    let exponent_field = (bits & Fp32::EXP_MASK) >> 23;
    let fraction = bits & Fp32::FRAC_MASK;

    if exponent_field == 0 {
        if fraction == 0 {
            Unpacked32 {
                sign,
                kind: Kind::Zero,
                exponent: 0,
                significand: 0,
            }
        } else {
            Unpacked32 {
                sign,
                kind: Kind::Finite,
                exponent: 1 - 127 - 23,
                significand: fraction,
            }
        }
    } else if exponent_field == 0xff {
        let kind = if fraction == 0 {
            Kind::Infinity
        } else if fraction & Fp32::QUIET_BIT != 0 {
            Kind::QuietNan
        } else {
            Kind::SignalingNan
        };
        Unpacked32 {
            sign,
            kind,
            exponent: 0,
            significand: fraction,
        }
    } else {
        Unpacked32 {
            sign,
            kind: Kind::Finite,
            exponent: exponent_field as i32 - 127 - 23,
            significand: (1 << 23) | fraction,
        }
    }
}

/// 解码双精度位模式。
pub fn unpack64(bits: u64) -> Unpacked64 {
    let sign = bits & Fp64::SIGN_MASK != 0;
    let exponent_field = ((bits & Fp64::EXP_MASK) >> 52) as u32;
    let fraction = bits & Fp64::FRAC_MASK;

    if exponent_field == 0 {
        if fraction == 0 {
            Unpacked64 {
                sign,
                kind: Kind::Zero,
                exponent: 0,
                significand: 0,
            }
        } else {
            Unpacked64 {
                sign,
                kind: Kind::Finite,
                exponent: 1 - 1023 - 52,
                significand: fraction,
            }
        }
    } else if exponent_field == 0x7ff {
        let kind = if fraction == 0 {
            Kind::Infinity
        } else if fraction & Fp64::QUIET_BIT != 0 {
            Kind::QuietNan
        } else {
            Kind::SignalingNan
        };
        Unpacked64 {
            sign,
            kind,
            exponent: 0,
            significand: fraction,
        }
    } else {
        Unpacked64 {
            sign,
            kind: Kind::Finite,
            exponent: exponent_field as i32 - 1023 - 52,
            significand: (1 << 52) | fraction,
        }
    }
}

/// 将单精度有限值的 significand 左移到隐藏位位置。
///
/// 只调整指数，不丢弃任何位。已经是 24 位或更宽的 significand 保持不变。
pub fn normalize32(value: &mut Unpacked32) {
    if !matches!(value.kind, Kind::Finite) {
        return;
    }
    if value.significand == 0 {
        value.kind = Kind::Zero;
        value.exponent = 0;
        return;
    }

    let msb = 31 - value.significand.leading_zeros();
    if msb < 23 {
        let shift = 23 - msb;
        value.significand <<= shift;
        value.exponent -= shift as i32;
    }
}

/// 将双精度有限值的 significand 左移到隐藏位位置。
///
/// 只调整指数，不丢弃任何位。已经是 53 位或更宽的 significand 保持不变。
pub fn normalize64(value: &mut Unpacked64) {
    if !matches!(value.kind, Kind::Finite) {
        return;
    }
    if value.significand == 0 {
        value.kind = Kind::Zero;
        value.exponent = 0;
        return;
    }

    let msb = 63 - value.significand.leading_zeros();
    if msb < 52 {
        let shift = 52 - msb;
        value.significand <<= shift;
        value.exponent -= shift as i32;
    }
}

/// 将整数 significand 右移 `shift` 位并按指定模式舍入。
///
/// 返回舍入后的整数和是否不精确。移出的最高位是 guard，
/// 其余位合并为 sticky。
fn shift_right_round(
    significand: u128,
    shift: u32,
    sign: bool,
    mode: RoundingMode,
) -> (u128, bool) {
    if shift == 0 || significand == 0 {
        return (significand, false);
    }

    let (mut kept, guard, sticky) = if shift >= 128 {
        if shift == 128 {
            let guard = (significand >> 127) & 1 != 0;
            let sticky = significand & ((1u128 << 127) - 1) != 0;
            (0, guard, sticky)
        } else {
            (0, false, true)
        }
    } else {
        let discarded = significand & ((1u128 << shift) - 1);
        let guard = (discarded >> (shift - 1)) & 1 != 0;
        let sticky = discarded & ((1u128 << (shift - 1)) - 1) != 0;
        (significand >> shift, guard, sticky)
    };

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

    if increment {
        kept += 1;
    }
    (kept, inexact)
}

fn signed_zero32(sign: bool) -> Fp32 {
    Fp32::from_bits((sign as u32) << 31)
}

fn signed_zero64(sign: bool) -> Fp64 {
    Fp64::from_bits((sign as u64) << 63)
}

fn overflow32(sign: bool, mode: RoundingMode) -> Fp32 {
    let to_inf = match mode {
        RoundingMode::Rne | RoundingMode::Rmm => true,
        RoundingMode::Rtz => false,
        RoundingMode::Rdn => sign,
        RoundingMode::Rup => !sign,
    };
    let magnitude = if to_inf { 0x7f80_0000 } else { 0x7f7f_ffff };
    Fp32::from_bits(((sign as u32) << 31) | magnitude)
}

fn overflow64(sign: bool, mode: RoundingMode) -> Fp64 {
    let to_inf = match mode {
        RoundingMode::Rne | RoundingMode::Rmm => true,
        RoundingMode::Rtz => false,
        RoundingMode::Rdn => sign,
        RoundingMode::Rup => !sign,
    };
    let magnitude = if to_inf {
        0x7ff0_0000_0000_0000
    } else {
        0x7fef_ffff_ffff_ffff
    };
    Fp64::from_bits(((sign as u64) << 63) | magnitude)
}

fn pack_normal32(sign: bool, lsb_exponent: i64, significand: u128) -> Fp32 {
    let exponent_field = (lsb_exponent + 127 + 23) as u32;
    let fraction = significand as u32 & Fp32::FRAC_MASK;
    Fp32::from_bits(((sign as u32) << 31) | (exponent_field << 23) | fraction)
}

fn pack_normal64(sign: bool, lsb_exponent: i64, significand: u128) -> Fp64 {
    let exponent_field = (lsb_exponent + 1023 + 52) as u64;
    let fraction = significand as u64 & Fp64::FRAC_MASK;
    Fp64::from_bits(((sign as u64) << 63) | (exponent_field << 52) | fraction)
}

const F32_PRECISION: i64 = 24;
const F32_MIN_LSB_EXP: i64 = 1 - 127 - (F32_PRECISION - 1);
const F32_MAX_LEAD_EXP: i64 = 127;
const F32_MAX_LSB_EXP: i64 = F32_MAX_LEAD_EXP - (F32_PRECISION - 1);

/// 将 `significand * 2^exponent` 舍入并编码为单精度。
///
/// `significand` 可以是任意精度整数；函数只做一次舍入。
/// 溢出按 IEEE 754 的“无界指数舍入结果”判定；下溢采用 after-rounding 检测。
/// 溢出、下溢和不精确标志会随结果一起返回。
pub fn round_pack32(
    sign: bool,
    exponent: i32,
    significand: u128,
    mode: RoundingMode,
) -> (Fp32, Flags) {
    if significand == 0 {
        return (signed_zero32(sign), Flags::empty());
    }

    let msb = 127 - significand.leading_zeros() as i64;
    let leading_exponent = exponent as i64 + msb;
    if leading_exponent > F32_MAX_LEAD_EXP {
        return (overflow32(sign, mode), Flags::OVERFLOW | Flags::INEXACT);
    }

    let normal_lsb = leading_exponent - (F32_PRECISION - 1);
    let mut target_lsb = normal_lsb.max(F32_MIN_LSB_EXP);
    let shift = target_lsb - exponent as i64;
    let mut flags = Flags::empty();
    let mut sig = significand;

    if shift > 0 {
        let (rounded, inexact) = shift_right_round(sig, shift as u32, sign, mode);
        sig = rounded;
        if inexact {
            flags |= Flags::INEXACT;
        }
    } else if shift < 0 {
        let left = -shift as u32;
        debug_assert!(left < 128, "left shift out of range");
        sig <<= left;
    }

    if sig == 0 {
        if flags.contains(Flags::INEXACT) {
            flags |= Flags::UNDERFLOW;
        }
        return (signed_zero32(sign), flags);
    }

    if sig >= 1u128 << F32_PRECISION {
        sig >>= 1;
        target_lsb += 1;
        if target_lsb > F32_MAX_LSB_EXP {
            return (
                overflow32(sign, mode),
                flags | Flags::OVERFLOW | Flags::INEXACT,
            );
        }
        return (pack_normal32(sign, target_lsb, sig), flags);
    }

    let hidden = 1u128 << (F32_PRECISION - 1);
    if sig >= hidden {
        return (pack_normal32(sign, target_lsb, sig), flags);
    }

    debug_assert_eq!(target_lsb, F32_MIN_LSB_EXP);
    if flags.contains(Flags::INEXACT) {
        flags |= Flags::UNDERFLOW;
    }
    (Fp32::from_bits(((sign as u32) << 31) | sig as u32), flags)
}

const F64_PRECISION: i64 = 53;
const F64_MIN_LSB_EXP: i64 = 1 - 1023 - (F64_PRECISION - 1);
const F64_MAX_LEAD_EXP: i64 = 1023;
const F64_MAX_LSB_EXP: i64 = F64_MAX_LEAD_EXP - (F64_PRECISION - 1);

/// 将 `significand * 2^exponent` 舍入并编码为双精度。
///
/// `significand` 可以是任意精度整数；函数只做一次舍入。
/// 溢出按 IEEE 754 的“无界指数舍入结果”判定；下溢采用 after-rounding 检测。
/// 溢出、下溢和不精确标志会随结果一起返回。
pub fn round_pack64(
    sign: bool,
    exponent: i32,
    significand: u128,
    mode: RoundingMode,
) -> (Fp64, Flags) {
    if significand == 0 {
        return (signed_zero64(sign), Flags::empty());
    }

    let msb = 127 - significand.leading_zeros() as i64;
    let leading_exponent = exponent as i64 + msb;
    if leading_exponent > F64_MAX_LEAD_EXP {
        return (overflow64(sign, mode), Flags::OVERFLOW | Flags::INEXACT);
    }

    let normal_lsb = leading_exponent - (F64_PRECISION - 1);
    let mut target_lsb = normal_lsb.max(F64_MIN_LSB_EXP);
    let shift = target_lsb - exponent as i64;
    let mut flags = Flags::empty();
    let mut sig = significand;

    if shift > 0 {
        let (rounded, inexact) = shift_right_round(sig, shift as u32, sign, mode);
        sig = rounded;
        if inexact {
            flags |= Flags::INEXACT;
        }
    } else if shift < 0 {
        let left = -shift as u32;
        debug_assert!(left < 128, "left shift out of range");
        sig <<= left;
    }

    if sig == 0 {
        if flags.contains(Flags::INEXACT) {
            flags |= Flags::UNDERFLOW;
        }
        return (signed_zero64(sign), flags);
    }

    if sig >= 1u128 << F64_PRECISION {
        sig >>= 1;
        target_lsb += 1;
        if target_lsb > F64_MAX_LSB_EXP {
            return (
                overflow64(sign, mode),
                flags | Flags::OVERFLOW | Flags::INEXACT,
            );
        }
        return (pack_normal64(sign, target_lsb, sig), flags);
    }

    let hidden = 1u128 << (F64_PRECISION - 1);
    if sig >= hidden {
        return (pack_normal64(sign, target_lsb, sig), flags);
    }

    debug_assert_eq!(target_lsb, F64_MIN_LSB_EXP);
    if flags.contains(Flags::INEXACT) {
        flags |= Flags::UNDERFLOW;
    }
    (Fp64::from_bits(((sign as u64) << 63) | sig as u64), flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_FLAGS: Flags = Flags::empty();

    fn round32(sign: bool, exponent: i32, significand: u128, mode: RoundingMode) -> (u32, Flags) {
        let (value, flags) = round_pack32(sign, exponent, significand, mode);
        (value.to_bits(), flags)
    }

    fn round64(sign: bool, exponent: i32, significand: u128, mode: RoundingMode) -> (u64, Flags) {
        let (value, flags) = round_pack64(sign, exponent, significand, mode);
        (value.to_bits(), flags)
    }

    #[test]
    fn unpack_f32_normal_subnormal_and_specials() {
        let one = unpack32(0x3f80_0000);
        assert!(!one.sign);
        assert_eq!(one.kind, Kind::Finite);
        assert_eq!(one.exponent, -23);
        assert_eq!(one.significand, 1 << 23);

        let min_subnormal = unpack32(0x0000_0001);
        assert_eq!(min_subnormal.kind, Kind::Finite);
        assert_eq!(min_subnormal.exponent, -149);
        assert_eq!(min_subnormal.significand, 1);

        let min_normal = unpack32(0x0080_0000);
        assert_eq!(min_normal.exponent, -149);
        assert_eq!(min_normal.significand, 1 << 23);

        let max_normal = unpack32(0x7f7f_ffff);
        assert_eq!(max_normal.exponent, 104);
        assert_eq!(max_normal.significand, 0x00ff_ffff);

        let neg_zero = unpack32(0x8000_0000);
        assert_eq!(neg_zero.kind, Kind::Zero);
        assert!(neg_zero.sign);

        let inf = unpack32(0x7f80_0000);
        assert_eq!(inf.kind, Kind::Infinity);

        let qnan = unpack32(0x7fc0_1234);
        assert_eq!(qnan.kind, Kind::QuietNan);
        assert_eq!(qnan.significand, 0x0040_1234);

        let snan = unpack32(0x7f80_0001);
        assert_eq!(snan.kind, Kind::SignalingNan);
        assert!(snan.is_nan());
        assert!(snan.is_signaling_nan());
    }

    #[test]
    fn unpack_f64_normal_subnormal_and_specials() {
        let one = unpack64(0x3ff0_0000_0000_0000);
        assert_eq!(one.kind, Kind::Finite);
        assert_eq!(one.exponent, -52);
        assert_eq!(one.significand, 1 << 52);

        let min_subnormal = unpack64(0x0000_0000_0000_0001);
        assert_eq!(min_subnormal.exponent, -1074);
        assert_eq!(min_subnormal.significand, 1);

        let min_normal = unpack64(0x0010_0000_0000_0000);
        assert_eq!(min_normal.exponent, -1074);
        assert_eq!(min_normal.significand, 1 << 52);

        let max_normal = unpack64(0x7fef_ffff_ffff_ffff);
        assert_eq!(max_normal.exponent, 971);
        assert_eq!(max_normal.significand, 0x001f_ffff_ffff_ffff);

        let neg_zero = unpack64(0x8000_0000_0000_0000);
        assert_eq!(neg_zero.kind, Kind::Zero);
        assert!(neg_zero.sign);

        let inf = unpack64(0x7ff0_0000_0000_0000);
        assert_eq!(inf.kind, Kind::Infinity);

        let qnan = unpack64(0x7ff8_0000_0000_1234);
        assert_eq!(qnan.kind, Kind::QuietNan);

        let snan = unpack64(0x7ff0_0000_0000_0001);
        assert_eq!(snan.kind, Kind::SignalingNan);
    }

    #[test]
    fn normalize_f32_moves_subnormal_to_hidden_bit() {
        let mut value = unpack32(0x0000_0001);
        normalize32(&mut value);
        assert_eq!(value.kind, Kind::Finite);
        assert_eq!(value.exponent, -172);
        assert_eq!(value.significand, 1 << 23);

        let mut one = unpack32(0x3f80_0000);
        normalize32(&mut one);
        assert_eq!(one.exponent, -23);
        assert_eq!(one.significand, 1 << 23);

        let mut zero = unpack32(0x8000_0000);
        normalize32(&mut zero);
        assert_eq!(zero.kind, Kind::Zero);
        assert_eq!(zero.exponent, 0);

        let mut inf = unpack32(0x7f80_0000);
        normalize32(&mut inf);
        assert_eq!(inf.kind, Kind::Infinity);
    }

    #[test]
    fn normalize_f64_moves_subnormal_to_hidden_bit() {
        let mut value = unpack64(0x0000_0000_0000_0001);
        normalize64(&mut value);
        assert_eq!(value.kind, Kind::Finite);
        assert_eq!(value.exponent, -1126);
        assert_eq!(value.significand, 1 << 52);

        let mut one = unpack64(0x3ff0_0000_0000_0000);
        normalize64(&mut one);
        assert_eq!(one.exponent, -52);
        assert_eq!(one.significand, 1 << 52);

        let mut zero = unpack64(0x8000_0000_0000_0000);
        normalize64(&mut zero);
        assert_eq!(zero.kind, Kind::Zero);
        assert_eq!(zero.exponent, 0);
    }

    #[test]
    fn round_f32_exact_values_do_not_raise_flags() {
        assert_eq!(
            round32(false, -23, 1 << 23, RoundingMode::Rne),
            (0x3f80_0000, NO_FLAGS)
        );
        assert_eq!(
            round32(false, -149, 1, RoundingMode::Rne),
            (0x0000_0001, NO_FLAGS)
        );
        assert_eq!(
            round32(false, -149, 0x007f_ffff, RoundingMode::Rne),
            (0x007f_ffff, NO_FLAGS)
        );
        assert_eq!(
            round32(false, 104, 0x00ff_ffff, RoundingMode::Rne),
            (0x7f7f_ffff, NO_FLAGS)
        );
        assert_eq!(
            round32(true, -149, 1, RoundingMode::Rne),
            (0x8000_0001, NO_FLAGS)
        );
    }

    #[test]
    fn round_f32_ties_follow_each_mode() {
        let tie = (1u128 << 24) | 1;
        assert_eq!(
            round32(false, -24, tie, RoundingMode::Rne),
            (0x3f80_0000, Flags::INEXACT)
        );
        assert_eq!(
            round32(false, -24, tie, RoundingMode::Rmm),
            (0x3f80_0001, Flags::INEXACT)
        );
        assert_eq!(
            round32(false, -24, tie, RoundingMode::Rtz),
            (0x3f80_0000, Flags::INEXACT)
        );
        assert_eq!(
            round32(false, -24, tie, RoundingMode::Rdn),
            (0x3f80_0000, Flags::INEXACT)
        );
        assert_eq!(
            round32(false, -24, tie, RoundingMode::Rup),
            (0x3f80_0001, Flags::INEXACT)
        );
    }

    #[test]
    fn round_f32_directed_modes_respect_sign() {
        let tie = (1u128 << 24) | 1;
        assert_eq!(
            round32(true, -24, tie, RoundingMode::Rdn),
            (0xbf80_0001, Flags::INEXACT)
        );
        assert_eq!(
            round32(true, -24, tie, RoundingMode::Rup),
            (0xbf80_0000, Flags::INEXACT)
        );
        assert_eq!(
            round32(true, -24, tie, RoundingMode::Rtz),
            (0xbf80_0000, Flags::INEXACT)
        );
    }

    #[test]
    fn round_f32_above_tie_rounds_up_for_nearest() {
        let above_tie = (1u128 << 24) | 3;
        assert_eq!(
            round32(false, -24, above_tie, RoundingMode::Rne),
            (0x3f80_0002, Flags::INEXACT)
        );
        assert_eq!(
            round32(true, -24, above_tie, RoundingMode::Rne),
            (0xbf80_0002, Flags::INEXACT)
        );
    }

    #[test]
    fn round_f32_overflow_uses_mode_dependent_result() {
        let overflow_flags = Flags::OVERFLOW | Flags::INEXACT;
        let sig = 1u128 << 23;
        assert_eq!(
            round32(false, 105, sig, RoundingMode::Rne),
            (0x7f80_0000, overflow_flags)
        );
        assert_eq!(
            round32(false, 105, sig, RoundingMode::Rtz),
            (0x7f7f_ffff, overflow_flags)
        );
        assert_eq!(
            round32(false, 105, sig, RoundingMode::Rdn),
            (0x7f7f_ffff, overflow_flags)
        );
        assert_eq!(
            round32(false, 105, sig, RoundingMode::Rup),
            (0x7f80_0000, overflow_flags)
        );
        assert_eq!(
            round32(true, 105, sig, RoundingMode::Rdn),
            (0xff80_0000, overflow_flags)
        );
        assert_eq!(
            round32(true, 105, sig, RoundingMode::Rup),
            (0xff7f_ffff, overflow_flags)
        );
    }

    #[test]
    fn round_f32_underflow_to_zero_or_min_subnormal() {
        let underflow_flags = Flags::UNDERFLOW | Flags::INEXACT;
        let half_min = 1u128;
        assert_eq!(
            round32(false, -150, half_min, RoundingMode::Rne),
            (0x0000_0000, underflow_flags)
        );
        assert_eq!(
            round32(false, -150, half_min, RoundingMode::Rmm),
            (0x0000_0001, underflow_flags)
        );
        assert_eq!(
            round32(false, -150, half_min, RoundingMode::Rtz),
            (0x0000_0000, underflow_flags)
        );
        assert_eq!(
            round32(false, -150, half_min, RoundingMode::Rdn),
            (0x0000_0000, underflow_flags)
        );
        assert_eq!(
            round32(false, -150, half_min, RoundingMode::Rup),
            (0x0000_0001, underflow_flags)
        );
        assert_eq!(
            round32(true, -150, half_min, RoundingMode::Rdn),
            (0x8000_0001, underflow_flags)
        );
        assert_eq!(
            round32(true, -150, half_min, RoundingMode::Rup),
            (0x8000_0000, underflow_flags)
        );

        assert_eq!(
            round32(false, -151, 3, RoundingMode::Rne),
            (0x0000_0001, underflow_flags)
        );
    }

    #[test]
    fn round_f32_rounding_up_to_smallest_normal_is_not_underflow() {
        assert_eq!(
            round32(false, -150, (1u128 << 24) - 1, RoundingMode::Rne),
            (0x0080_0000, Flags::INEXACT)
        );
    }

    #[test]
    fn round_f32_left_shift_into_subnormal_range() {
        // 2^-140 = 2^9 * 2^-149, an exact subnormal.
        assert_eq!(
            round32(false, -140, 1, RoundingMode::Rne),
            (0x0000_0200, NO_FLAGS)
        );
    }

    #[test]
    fn round_f32_handles_discarded_bits_beyond_u128_width() {
        let underflow_flags = Flags::UNDERFLOW | Flags::INEXACT;
        let sig = 1u128 << 127;

        // Shift == 128: guard is bit 127, sticky is empty.
        assert_eq!(
            round32(false, -277, sig, RoundingMode::Rne),
            (0x0000_0000, underflow_flags)
        );
        assert_eq!(
            round32(false, -277, sig, RoundingMode::Rmm),
            (0x0000_0001, underflow_flags)
        );

        // Shift > 128: guard is zero and all discarded bits form sticky.
        assert_eq!(
            round32(false, -300, sig, RoundingMode::Rne),
            (0x0000_0000, underflow_flags)
        );
        assert_eq!(
            round32(false, -300, sig, RoundingMode::Rup),
            (0x0000_0001, underflow_flags)
        );
        assert_eq!(
            round32(true, -300, sig, RoundingMode::Rdn),
            (0x8000_0001, underflow_flags)
        );
    }

    #[test]
    fn round_f32_overflow_after_rounding() {
        // Max finite plus half an ULP: the unbounded-exponent rounded result
        // overflows only for modes that round the tie upward.
        let sig = (1u128 << 25) - 1;
        let overflow_flags = Flags::OVERFLOW | Flags::INEXACT;
        assert_eq!(
            round32(false, 103, sig, RoundingMode::Rne),
            (0x7f80_0000, overflow_flags)
        );
        assert_eq!(
            round32(false, 103, sig, RoundingMode::Rmm),
            (0x7f80_0000, overflow_flags)
        );
        assert_eq!(
            round32(false, 103, sig, RoundingMode::Rtz),
            (0x7f7f_ffff, Flags::INEXACT)
        );
        assert_eq!(
            round32(false, 103, sig, RoundingMode::Rdn),
            (0x7f7f_ffff, Flags::INEXACT)
        );
        assert_eq!(
            round32(false, 103, sig, RoundingMode::Rup),
            (0x7f80_0000, overflow_flags)
        );

        // The same tie with a negative sign: RDN rounds away from zero.
        assert_eq!(
            round32(true, 103, sig, RoundingMode::Rdn),
            (0xff80_0000, overflow_flags)
        );
        assert_eq!(
            round32(true, 103, sig, RoundingMode::Rup),
            (0xff7f_ffff, Flags::INEXACT)
        );
    }

    #[test]
    fn round_f64_exact_and_tie_values() {
        assert_eq!(
            round64(false, -52, 1 << 52, RoundingMode::Rne),
            (0x3ff0_0000_0000_0000, NO_FLAGS)
        );
        assert_eq!(
            round64(false, -1074, 1, RoundingMode::Rne),
            (0x0000_0000_0000_0001, NO_FLAGS)
        );

        let tie = (1u128 << 53) | 1;
        assert_eq!(
            round64(false, -53, tie, RoundingMode::Rne),
            (0x3ff0_0000_0000_0000, Flags::INEXACT)
        );
        assert_eq!(
            round64(false, -53, tie, RoundingMode::Rmm),
            (0x3ff0_0000_0000_0001, Flags::INEXACT)
        );
    }

    #[test]
    fn round_f64_overflow_and_underflow() {
        let overflow_flags = Flags::OVERFLOW | Flags::INEXACT;
        assert_eq!(
            round64(false, 972, 1 << 52, RoundingMode::Rne),
            (0x7ff0_0000_0000_0000, overflow_flags)
        );
        assert_eq!(
            round64(false, 972, 1 << 52, RoundingMode::Rtz),
            (0x7fef_ffff_ffff_ffff, overflow_flags)
        );

        let underflow_flags = Flags::UNDERFLOW | Flags::INEXACT;
        assert_eq!(
            round64(false, -1075, 1, RoundingMode::Rne),
            (0x0000_0000_0000_0000, underflow_flags)
        );
        assert_eq!(
            round64(false, -1075, 1, RoundingMode::Rup),
            (0x0000_0000_0000_0001, underflow_flags)
        );
    }

    #[test]
    fn normalize_then_round_pack_round_trips() {
        let mut f32_value = unpack32(0x0000_0001);
        normalize32(&mut f32_value);
        let (f32_result, f32_flags) = round_pack32(
            f32_value.sign,
            f32_value.exponent,
            f32_value.significand as u128,
            RoundingMode::Rne,
        );
        assert_eq!(f32_result.to_bits(), 0x0000_0001);
        assert_eq!(f32_flags, NO_FLAGS);

        let mut f64_value = unpack64(0x0000_0000_0000_0001);
        normalize64(&mut f64_value);
        let (f64_result, f64_flags) = round_pack64(
            f64_value.sign,
            f64_value.exponent,
            f64_value.significand as u128,
            RoundingMode::Rne,
        );
        assert_eq!(f64_result.to_bits(), 0x0000_0000_0000_0001);
        assert_eq!(f64_flags, NO_FLAGS);
    }

    #[test]
    fn round_f64_rounding_up_to_smallest_normal_is_not_underflow() {
        assert_eq!(
            round64(false, -1075, (1u128 << 53) - 1, RoundingMode::Rne),
            (0x0010_0000_0000_0000, Flags::INEXACT)
        );
    }

    #[test]
    fn round_zero_preserves_sign() {
        assert_eq!(
            round32(true, 0, 0, RoundingMode::Rne),
            (0x8000_0000, NO_FLAGS)
        );
        assert_eq!(
            round64(true, 0, 0, RoundingMode::Rne),
            (0x8000_0000_0000_0000, NO_FLAGS)
        );
    }
}
