//! 单精度和双精度融合乘加。
//!
//! 乘积使用整数 significand 的精确结果；加数先与乘积对齐到同一个最低
//! 有效位指数，再由 [`round_pack32`] / [`round_pack64`] 做唯一一次舍入。
//! 对齐使用 256 位无符号整数，只有在两个操作数相距极远、低位不可能影响
//! 舍入时才把较小的项压缩成 sticky 位。

use crate::flags::Flags;
use crate::format::{Fp32, Fp64};
use crate::round::RoundingMode;
use crate::softfloat::{round_pack32, round_pack64, unpack32, unpack64};

/// 精确对齐时允许的最大位宽。
///
/// 超过这个宽度时，两个操作数的量级已经相差至少 150 位，较小项的
/// 精确低位不可能影响最终舍入，可以安全地合并为 sticky。
const EXACT_MAX_WIDTH: i32 = 255;

/// 把较小项压缩为 sticky 时，在大项最低有效位以下保留的距离。
///
/// 大项的 significand 最多 106 位，因此这个距离远低于任何单次舍入的
/// guard 位，同时又能保证正负尾数通过借位正确地影响不精确结果。
const FALLBACK_GUARD: u32 = 128;

/// 单精度融合乘加：`a * b + c`。
pub fn fmadd32(a: Fp32, b: Fp32, c: Fp32, mode: RoundingMode) -> (Fp32, Flags) {
    mul_add32(a, b, c, false, false, mode)
}

/// 单精度融合乘减：`a * b - c`。
pub fn fmsub32(a: Fp32, b: Fp32, c: Fp32, mode: RoundingMode) -> (Fp32, Flags) {
    mul_add32(a, b, c, false, true, mode)
}

/// 单精度负融合乘加：`-(a * b) - c`。
pub fn fnmadd32(a: Fp32, b: Fp32, c: Fp32, mode: RoundingMode) -> (Fp32, Flags) {
    mul_add32(a, b, c, true, true, mode)
}

/// 单精度负融合乘减：`-(a * b) + c`。
pub fn fnmsub32(a: Fp32, b: Fp32, c: Fp32, mode: RoundingMode) -> (Fp32, Flags) {
    mul_add32(a, b, c, true, false, mode)
}

/// 双精度融合乘加：`a * b + c`。
pub fn fmadd64(a: Fp64, b: Fp64, c: Fp64, mode: RoundingMode) -> (Fp64, Flags) {
    mul_add64(a, b, c, false, false, mode)
}

/// 双精度融合乘减：`a * b - c`。
pub fn fmsub64(a: Fp64, b: Fp64, c: Fp64, mode: RoundingMode) -> (Fp64, Flags) {
    mul_add64(a, b, c, false, true, mode)
}

/// 双精度负融合乘加：`-(a * b) - c`。
pub fn fnmadd64(a: Fp64, b: Fp64, c: Fp64, mode: RoundingMode) -> (Fp64, Flags) {
    mul_add64(a, b, c, true, true, mode)
}

/// 双精度负融合乘减：`-(a * b) + c`。
pub fn fnmsub64(a: Fp64, b: Fp64, c: Fp64, mode: RoundingMode) -> (Fp64, Flags) {
    mul_add64(a, b, c, true, false, mode)
}

fn mul_add32(
    a: Fp32,
    b: Fp32,
    c: Fp32,
    negate_product: bool,
    negate_addend: bool,
    mode: RoundingMode,
) -> (Fp32, Flags) {
    let product_sign = a.sign() ^ b.sign() ^ negate_product;
    let addend_sign = c.sign() ^ negate_addend;
    let zero_inf = (a.is_inf() && b.is_zero()) || (b.is_inf() && a.is_zero());

    if a.is_nan() || b.is_nan() || c.is_nan() {
        let mut flags = Flags::empty();
        if a.is_signaling_nan() || b.is_signaling_nan() || c.is_signaling_nan() {
            flags |= Flags::INVALID;
        }
        if zero_inf {
            flags |= Flags::INVALID;
        }
        return (Fp32::canonical_nan(), flags);
    }

    if a.is_inf() || b.is_inf() {
        if zero_inf {
            return (Fp32::canonical_nan(), Flags::INVALID);
        }
        if c.is_inf() && c.sign() ^ negate_addend != product_sign {
            return (Fp32::canonical_nan(), Flags::INVALID);
        }
        return (inf32(product_sign), Flags::empty());
    }

    if c.is_inf() {
        return (inf32(addend_sign), Flags::empty());
    }

    if a.is_zero() || b.is_zero() {
        if c.is_zero() {
            return (
                zero32(zero_sign(product_sign, addend_sign, mode)),
                Flags::empty(),
            );
        }
        return (c.with_sign(addend_sign), Flags::empty());
    }

    let ua = unpack32(a.to_bits());
    let ub = unpack32(b.to_bits());
    let uc = unpack32(c.to_bits());
    let product = (ua.significand as u128) * (ub.significand as u128);
    let product_exp = ua.exponent + ub.exponent;

    if c.is_zero() {
        return round_pack32(product_sign, product_exp, product, mode);
    }

    let product_lead = product_exp + (127 - product.leading_zeros() as i32);
    let addend_lead = uc.exponent + (31 - uc.significand.leading_zeros() as i32);
    let min_exp = product_exp.min(uc.exponent);
    let max_lead = product_lead.max(addend_lead);
    let width = max_lead - min_exp + 1;

    if width <= EXACT_MAX_WIDTH {
        let product_wide = Wide::from_u128(product).shl((product_exp - min_exp) as u32);
        let addend_wide =
            Wide::from_u128(uc.significand as u128).shl((uc.exponent - min_exp) as u32);
        let (sign, exponent, value) = combine(
            product_sign,
            product_wide,
            addend_sign,
            addend_wide,
            min_exp,
            mode,
        );
        return finish32(sign, exponent, value, mode);
    }

    let (big_sign, big_exp, big_sig, small_sign, small_nonzero) = if product_lead >= addend_lead {
        (
            product_sign,
            product_exp,
            product,
            addend_sign,
            uc.significand != 0,
        )
    } else {
        (
            addend_sign,
            uc.exponent,
            uc.significand as u128,
            product_sign,
            product != 0,
        )
    };
    let mut value = Wide::from_u128(big_sig).shl(FALLBACK_GUARD);
    if small_nonzero {
        let sticky = Wide::from_u128(1);
        value = if big_sign == small_sign {
            value.add(sticky)
        } else {
            value.sub(sticky)
        };
    }
    finish32(big_sign, big_exp - FALLBACK_GUARD as i32, value, mode)
}

fn mul_add64(
    a: Fp64,
    b: Fp64,
    c: Fp64,
    negate_product: bool,
    negate_addend: bool,
    mode: RoundingMode,
) -> (Fp64, Flags) {
    let product_sign = a.sign() ^ b.sign() ^ negate_product;
    let addend_sign = c.sign() ^ negate_addend;
    let zero_inf = (a.is_inf() && b.is_zero()) || (b.is_inf() && a.is_zero());

    if a.is_nan() || b.is_nan() || c.is_nan() {
        let mut flags = Flags::empty();
        if a.is_signaling_nan() || b.is_signaling_nan() || c.is_signaling_nan() {
            flags |= Flags::INVALID;
        }
        if zero_inf {
            flags |= Flags::INVALID;
        }
        return (Fp64::canonical_nan(), flags);
    }

    if a.is_inf() || b.is_inf() {
        if zero_inf {
            return (Fp64::canonical_nan(), Flags::INVALID);
        }
        if c.is_inf() && c.sign() ^ negate_addend != product_sign {
            return (Fp64::canonical_nan(), Flags::INVALID);
        }
        return (inf64(product_sign), Flags::empty());
    }

    if c.is_inf() {
        return (inf64(addend_sign), Flags::empty());
    }

    if a.is_zero() || b.is_zero() {
        if c.is_zero() {
            return (
                zero64(zero_sign(product_sign, addend_sign, mode)),
                Flags::empty(),
            );
        }
        return (c.with_sign(addend_sign), Flags::empty());
    }

    let ua = unpack64(a.to_bits());
    let ub = unpack64(b.to_bits());
    let uc = unpack64(c.to_bits());
    let product = (ua.significand as u128) * (ub.significand as u128);
    let product_exp = ua.exponent + ub.exponent;

    if c.is_zero() {
        return round_pack64(product_sign, product_exp, product, mode);
    }

    let product_lead = product_exp + (127 - product.leading_zeros() as i32);
    let addend_lead = uc.exponent + (63 - uc.significand.leading_zeros() as i32);
    let min_exp = product_exp.min(uc.exponent);
    let max_lead = product_lead.max(addend_lead);
    let width = max_lead - min_exp + 1;

    if width <= EXACT_MAX_WIDTH {
        let product_wide = Wide::from_u128(product).shl((product_exp - min_exp) as u32);
        let addend_wide =
            Wide::from_u128(uc.significand as u128).shl((uc.exponent - min_exp) as u32);
        let (sign, exponent, value) = combine(
            product_sign,
            product_wide,
            addend_sign,
            addend_wide,
            min_exp,
            mode,
        );
        return finish64(sign, exponent, value, mode);
    }

    let (big_sign, big_exp, big_sig, small_sign, small_nonzero) = if product_lead >= addend_lead {
        (
            product_sign,
            product_exp,
            product,
            addend_sign,
            uc.significand != 0,
        )
    } else {
        (
            addend_sign,
            uc.exponent,
            uc.significand as u128,
            product_sign,
            product != 0,
        )
    };
    let mut value = Wide::from_u128(big_sig).shl(FALLBACK_GUARD);
    if small_nonzero {
        let sticky = Wide::from_u128(1);
        value = if big_sign == small_sign {
            value.add(sticky)
        } else {
            value.sub(sticky)
        };
    }
    finish64(big_sign, big_exp - FALLBACK_GUARD as i32, value, mode)
}

fn combine(
    product_sign: bool,
    product: Wide,
    addend_sign: bool,
    addend: Wide,
    exponent: i32,
    mode: RoundingMode,
) -> (bool, i32, Wide) {
    if product_sign == addend_sign {
        debug_assert!(!product.is_zero() || !addend.is_zero());
        (product_sign, exponent, product.add(addend))
    } else {
        match product.cmp(addend) {
            core::cmp::Ordering::Greater => (product_sign, exponent, product.sub(addend)),
            core::cmp::Ordering::Less => (addend_sign, exponent, addend.sub(product)),
            core::cmp::Ordering::Equal => (mode == RoundingMode::Rdn, exponent, Wide::zero()),
        }
    }
}

fn finish32(sign: bool, exponent: i32, value: Wide, mode: RoundingMode) -> (Fp32, Flags) {
    if value.is_zero() {
        return (zero32(mode == RoundingMode::Rdn), Flags::empty());
    }
    let msb = value.bit_len() as i32 - 1;
    if msb <= 127 {
        return round_pack32(sign, exponent, value.low_u128(), mode);
    }
    let shift = (msb - 127) as u32;
    let (top, _) = value.shr_jam(shift);
    round_pack32(sign, exponent + shift as i32, top.low_u128(), mode)
}

fn finish64(sign: bool, exponent: i32, value: Wide, mode: RoundingMode) -> (Fp64, Flags) {
    if value.is_zero() {
        return (zero64(mode == RoundingMode::Rdn), Flags::empty());
    }
    let msb = value.bit_len() as i32 - 1;
    if msb <= 127 {
        return round_pack64(sign, exponent, value.low_u128(), mode);
    }
    let shift = (msb - 127) as u32;
    let (top, _) = value.shr_jam(shift);
    round_pack64(sign, exponent + shift as i32, top.low_u128(), mode)
}

fn zero_sign(product_sign: bool, addend_sign: bool, mode: RoundingMode) -> bool {
    if product_sign == addend_sign {
        product_sign
    } else {
        mode == RoundingMode::Rdn
    }
}

const fn zero32(sign: bool) -> Fp32 {
    Fp32::from_bits((sign as u32) << 31)
}

const fn inf32(sign: bool) -> Fp32 {
    Fp32::from_bits(((sign as u32) << 31) | 0x7f80_0000)
}

const fn zero64(sign: bool) -> Fp64 {
    Fp64::from_bits((sign as u64) << 63)
}

const fn inf64(sign: bool) -> Fp64 {
    Fp64::from_bits(((sign as u64) << 63) | 0x7ff0_0000_0000_0000)
}

/// 256 位无符号整数，使用小端 limb 顺序。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Wide {
    limbs: [u64; 4],
}

impl Wide {
    const fn zero() -> Self {
        Self { limbs: [0; 4] }
    }

    fn from_u128(value: u128) -> Self {
        Self {
            limbs: [value as u64, (value >> 64) as u64, 0, 0],
        }
    }

    fn is_zero(self) -> bool {
        self.limbs == [0; 4]
    }

    fn bit_len(self) -> u32 {
        for index in (0..4).rev() {
            let limb = self.limbs[index];
            if limb != 0 {
                return index as u32 * 64 + (64 - limb.leading_zeros());
            }
        }
        0
    }

    fn low_u128(self) -> u128 {
        (self.limbs[0] as u128) | ((self.limbs[1] as u128) << 64)
    }

    fn shl(self, shift: u32) -> Self {
        if shift == 0 {
            return self;
        }
        debug_assert!(shift < 256);
        debug_assert!(self.bit_len() + shift <= 256);
        let limb_shift = (shift / 64) as usize;
        let bit_shift = shift % 64;
        let mut limbs = [0u64; 4];
        for index in 0..4 {
            let target = index + limb_shift;
            if target >= 4 {
                break;
            }
            limbs[target] |= self.limbs[index] << bit_shift;
            if bit_shift != 0 && target + 1 < 4 {
                limbs[target + 1] |= self.limbs[index] >> (64 - bit_shift);
            }
        }
        Self { limbs }
    }

    fn shr_jam(self, shift: u32) -> (Self, bool) {
        if shift == 0 {
            return (self, false);
        }
        if shift >= 256 {
            return (Self::zero(), !self.is_zero());
        }
        let limb_shift = (shift / 64) as usize;
        let bit_shift = shift % 64;
        let mut limbs = [0u64; 4];
        for index in limb_shift..4 {
            let target = index - limb_shift;
            limbs[target] |= self.limbs[index] >> bit_shift;
            if bit_shift != 0 && index + 1 < 4 {
                limbs[target] |= self.limbs[index + 1] << (64 - bit_shift);
            }
        }
        let mut discarded = self.limbs[..limb_shift].iter().any(|&limb| limb != 0);
        if bit_shift != 0 {
            let mask = (1u64 << bit_shift) - 1;
            discarded |= self.limbs[limb_shift] & mask != 0;
        }
        if discarded {
            limbs[0] |= 1;
        }
        (Self { limbs }, discarded)
    }

    fn add(self, other: Self) -> Self {
        let mut limbs = [0u64; 4];
        let mut carry = false;
        for (index, limb) in limbs.iter_mut().enumerate() {
            let (sum, carry_a) = self.limbs[index].overflowing_add(other.limbs[index]);
            let (sum, carry_b) = sum.overflowing_add(carry as u64);
            *limb = sum;
            carry = carry_a || carry_b;
        }
        debug_assert!(!carry, "wide addition overflow");
        Self { limbs }
    }

    fn sub(self, other: Self) -> Self {
        let mut limbs = [0u64; 4];
        let mut borrow = false;
        for (index, limb) in limbs.iter_mut().enumerate() {
            let (difference, borrow_a) = self.limbs[index].overflowing_sub(other.limbs[index]);
            let (difference, borrow_b) = difference.overflowing_sub(borrow as u64);
            *limb = difference;
            borrow = borrow_a || borrow_b;
        }
        debug_assert!(!borrow, "wide subtraction underflow");
        Self { limbs }
    }

    fn cmp(self, other: Self) -> core::cmp::Ordering {
        for index in (0..4).rev() {
            match self.limbs[index].cmp(&other.limbs[index]) {
                core::cmp::Ordering::Equal => {}
                ordering => return ordering,
            }
        }
        core::cmp::Ordering::Equal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: u32 = 0x3f80_0000;
    const TWO: u32 = 0x4000_0000;
    const THREE: u32 = 0x4040_0000;
    const FIVE: u32 = 0x40a0_0000;
    const HALF: u32 = 0x3f00_0000;
    const ONE_PLUS_ULP: u32 = 0x3f80_0001;
    const ONE_PLUS_TWO_ULP: u32 = 0x3f80_0002;
    const MIN_SUBNORMAL: u32 = 0x0000_0001;
    const MAX_FINITE: u32 = 0x7f7f_ffff;
    const INF: u32 = 0x7f80_0000;
    const NEG_INF: u32 = 0xff80_0000;
    const QUIET_NAN: u32 = 0x7fc0_0000;
    const SIGNALING_NAN: u32 = 0x7f80_0001;
    const NEG_ZERO: u32 = 0x8000_0000;
    const NEG_ONE: u32 = 0xbf80_0000;
    const TWO_POW_MINUS_24: u32 = 0x3380_0000;
    const TWO_POW_MINUS_46: u32 = 0x2880_0000;

    fn f32(bits: u32) -> Fp32 {
        Fp32::from_bits(bits)
    }

    fn fmadd(a: u32, b: u32, c: u32, mode: RoundingMode) -> (u32, Flags) {
        let (value, flags) = fmadd32(f32(a), f32(b), f32(c), mode);
        (value.to_bits(), flags)
    }

    #[test]
    fn fused_operations_use_full_product() {
        assert_eq!(
            fmadd(ONE, TWO, THREE, RoundingMode::Rne),
            (FIVE, Flags::empty())
        );
        assert_eq!(
            fmadd(
                ONE_PLUS_ULP,
                ONE_PLUS_ULP,
                ONE_PLUS_TWO_ULP ^ 0x8000_0000,
                RoundingMode::Rne
            ),
            (TWO_POW_MINUS_46, Flags::empty())
        );

        let (value, flags) = fmsub32(f32(ONE), f32(TWO), f32(THREE), RoundingMode::Rne);
        assert_eq!(value.to_bits(), NEG_ONE);
        assert_eq!(flags, Flags::empty());

        let (value, flags) = fnmadd32(f32(ONE), f32(TWO), f32(THREE), RoundingMode::Rne);
        assert!(value.to_bits() & 0x8000_0000 != 0);
        assert_eq!(flags, Flags::empty());

        let (value, flags) = fnmsub32(f32(ONE), f32(TWO), f32(THREE), RoundingMode::Rne);
        assert_eq!(value.to_bits(), ONE);
        assert_eq!(flags, Flags::empty());
    }

    #[test]
    fn ties_are_rounded_once_by_each_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, ONE),
            (RoundingMode::Rtz, ONE),
            (RoundingMode::Rdn, ONE),
            (RoundingMode::Rup, ONE_PLUS_ULP),
            (RoundingMode::Rmm, ONE_PLUS_ULP),
        ] {
            assert_eq!(
                fmadd(ONE, ONE, TWO_POW_MINUS_24, mode),
                (expected, Flags::INEXACT)
            );
        }
    }

    #[test]
    fn exact_cancellation_zero_sign_follows_mode() {
        assert_eq!(
            fmadd(ONE, ONE, NEG_ONE, RoundingMode::Rne),
            (0, Flags::empty())
        );
        assert_eq!(
            fmadd(ONE, ONE, NEG_ONE, RoundingMode::Rdn),
            (NEG_ZERO, Flags::empty())
        );

        for (mode, expected) in [
            (RoundingMode::Rne, 0),
            (RoundingMode::Rtz, 0),
            (RoundingMode::Rdn, 0),
            (RoundingMode::Rup, MIN_SUBNORMAL),
            (RoundingMode::Rmm, MIN_SUBNORMAL),
        ] {
            assert_eq!(
                fmadd(MIN_SUBNORMAL, HALF, 0, mode),
                (expected, Flags::UNDERFLOW | Flags::INEXACT)
            );
        }
    }

    #[test]
    fn overflow_respects_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, INF),
            (RoundingMode::Rtz, MAX_FINITE),
            (RoundingMode::Rdn, MAX_FINITE),
            (RoundingMode::Rup, INF),
            (RoundingMode::Rmm, INF),
        ] {
            assert_eq!(
                fmadd(MAX_FINITE, TWO, 0, mode),
                (expected, Flags::OVERFLOW | Flags::INEXACT)
            );
        }
    }

    #[test]
    fn far_apart_terms_keep_sticky_and_direction() {
        assert_eq!(
            fmadd(MAX_FINITE, ONE, MIN_SUBNORMAL, RoundingMode::Rne),
            (MAX_FINITE, Flags::INEXACT)
        );
        assert_eq!(
            fmadd(
                MAX_FINITE,
                ONE,
                MIN_SUBNORMAL ^ 0x8000_0000,
                RoundingMode::Rne
            ),
            (MAX_FINITE, Flags::INEXACT)
        );
        assert_eq!(
            fmadd(
                MAX_FINITE,
                ONE,
                MIN_SUBNORMAL ^ 0x8000_0000,
                RoundingMode::Rdn
            ),
            (0x7f7f_fffe, Flags::INEXACT)
        );
    }

    #[test]
    fn special_values_and_flags() {
        assert_eq!(
            fmadd(INF, ONE, NEG_INF, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            fmadd(INF, ONE, INF, RoundingMode::Rne),
            (INF, Flags::empty())
        );
        assert_eq!(
            fmadd(INF, 0, INF, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            fmadd(INF, 0, QUIET_NAN, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            fmadd(QUIET_NAN, ONE, ONE, RoundingMode::Rne),
            (QUIET_NAN, Flags::empty())
        );
        assert_eq!(
            fmadd(SIGNALING_NAN, ONE, ONE, RoundingMode::Rne),
            (QUIET_NAN, Flags::INVALID)
        );
        assert_eq!(
            fmadd(INF, ONE, QUIET_NAN, RoundingMode::Rne),
            (QUIET_NAN, Flags::empty())
        );
    }

    #[test]
    fn zero_signs_use_effective_operands() {
        assert_eq!(fmadd(0, ONE, 0, RoundingMode::Rne), (0, Flags::empty()));
        assert_eq!(
            fmadd(NEG_ZERO, ONE, NEG_ZERO, RoundingMode::Rne),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            fmadd(0, ONE, NEG_ZERO, RoundingMode::Rne),
            (0, Flags::empty())
        );
        assert_eq!(
            fmadd(0, ONE, NEG_ZERO, RoundingMode::Rdn),
            (NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            fmsub32(f32(0), f32(ONE), f32(0), RoundingMode::Rne)
                .0
                .to_bits(),
            0
        );
        assert_eq!(
            fmsub32(f32(0), f32(ONE), f32(0), RoundingMode::Rdn)
                .0
                .to_bits(),
            NEG_ZERO
        );
    }

    const D_ONE: u64 = 0x3ff0_0000_0000_0000;
    const D_TWO: u64 = 0x4000_0000_0000_0000;
    const D_THREE: u64 = 0x4008_0000_0000_0000;
    const D_ONE_PLUS_ULP: u64 = 0x3ff0_0000_0000_0001;
    const D_ONE_PLUS_TWO_ULP: u64 = 0x3ff0_0000_0000_0002;
    const D_TWO_POW_MINUS_53: u64 = 0x3ca0_0000_0000_0000;
    const D_TWO_POW_MINUS_104: u64 = 0x3970_0000_0000_0000;
    const D_MAX_FINITE: u64 = 0x7fef_ffff_ffff_ffff;
    const D_NEG_INF: u64 = 0xfff0_0000_0000_0000;
    const D_NEG_ZERO: u64 = 0x8000_0000_0000_0000;
    const D_QUIET_NAN: u64 = 0x7ff8_0000_0000_0000;

    fn f64(bits: u64) -> Fp64 {
        Fp64::from_bits(bits)
    }

    fn fmadd64_bits(a: u64, b: u64, c: u64, mode: RoundingMode) -> (u64, Flags) {
        let (value, flags) = fmadd64(f64(a), f64(b), f64(c), mode);
        (value.to_bits(), flags)
    }

    #[test]
    fn double_fused_product_is_not_rounded_early() {
        assert_eq!(
            fmadd64_bits(
                D_ONE_PLUS_ULP,
                D_ONE_PLUS_ULP,
                D_ONE_PLUS_TWO_ULP ^ (1 << 63),
                RoundingMode::Rne
            ),
            (D_TWO_POW_MINUS_104, Flags::empty())
        );
        assert_eq!(
            fmadd64_bits(D_ONE, D_TWO, D_THREE, RoundingMode::Rne),
            (0x4014_0000_0000_0000, Flags::empty())
        );
    }

    #[test]
    fn double_ties_are_rounded_once_by_each_mode() {
        for (mode, expected) in [
            (RoundingMode::Rne, D_ONE),
            (RoundingMode::Rtz, D_ONE),
            (RoundingMode::Rdn, D_ONE),
            (RoundingMode::Rup, D_ONE_PLUS_ULP),
            (RoundingMode::Rmm, D_ONE_PLUS_ULP),
        ] {
            assert_eq!(
                fmadd64_bits(D_ONE, D_ONE, D_TWO_POW_MINUS_53, mode),
                (expected, Flags::INEXACT)
            );
        }
    }

    #[test]
    fn double_special_values_and_overflow() {
        assert_eq!(
            fmadd64_bits(D_ONE, D_ONE, D_NEG_ZERO, RoundingMode::Rdn),
            (D_ONE, Flags::empty())
        );
        assert_eq!(
            fmadd64_bits(D_ONE, D_ONE, D_ONE ^ (1 << 63), RoundingMode::Rdn),
            (D_NEG_ZERO, Flags::empty())
        );
        assert_eq!(
            fmadd64_bits(D_MAX_FINITE, D_TWO, 0, RoundingMode::Rne),
            (0x7ff0_0000_0000_0000, Flags::OVERFLOW | Flags::INEXACT)
        );
        assert_eq!(
            fmadd64_bits(0x7ff0_0000_0000_0000, D_ONE, D_NEG_INF, RoundingMode::Rne),
            (D_QUIET_NAN, Flags::INVALID)
        );
    }
}
