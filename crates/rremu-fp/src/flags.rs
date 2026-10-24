//! IEEE-754 异常标志。
//!
//! 位布局与 RISC-V `fflags` 一致：NX=bit0、UF=bit1、OF=bit2、DZ=bit3、NV=bit4。

use core::ops::{BitOr, BitOrAssign};

/// 一组 IEEE-754 异常标志。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(u8);

impl Flags {
    /// Inexact.
    pub const INEXACT: Self = Self(1 << 0);
    /// Underflow.
    pub const UNDERFLOW: Self = Self(1 << 1);
    /// Overflow.
    pub const OVERFLOW: Self = Self(1 << 2);
    /// Divide by zero.
    pub const DIVIDE_BY_ZERO: Self = Self(1 << 3);
    /// Invalid operation.
    pub const INVALID: Self = Self(1 << 4);
    /// 所有已定义标志的并集。
    pub const ALL: Self = Self((1 << 5) - 1);

    /// 空集合。
    pub const fn empty() -> Self {
        Self(0)
    }

    /// 从 CSR 原始值构造，忽略保留高位。
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & Self::ALL.0)
    }

    /// 返回 CSR 原始值。
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// 是否没有任何标志。
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// 是否包含 `other` 的全部标志。
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// 集合并集。
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// 集合交集。
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// 从集合中移除 `other`。
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// 插入标志。
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// 清除标志。
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

impl BitOr for Flags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl BitOrAssign for Flags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csr_bit_layout() {
        assert_eq!(Flags::INEXACT.bits(), 0b00001);
        assert_eq!(Flags::UNDERFLOW.bits(), 0b00010);
        assert_eq!(Flags::OVERFLOW.bits(), 0b00100);
        assert_eq!(Flags::DIVIDE_BY_ZERO.bits(), 0b01000);
        assert_eq!(Flags::INVALID.bits(), 0b10000);
        assert_eq!(Flags::ALL.bits(), 0b11111);
    }

    #[test]
    fn from_bits_masks_reserved_bits() {
        assert_eq!(Flags::from_bits(0xff), Flags::ALL);
        assert_eq!(Flags::from_bits(0b0010_0001), Flags::INEXACT);
    }

    #[test]
    fn set_operations() {
        let mut flags = Flags::empty();
        assert!(flags.is_empty());
        assert!(!flags.contains(Flags::INEXACT));

        flags.insert(Flags::INEXACT);
        flags |= Flags::OVERFLOW;
        assert_eq!(flags.bits(), 0b00101);
        assert!(flags.contains(Flags::INEXACT));
        assert!(flags.contains(Flags::INEXACT | Flags::OVERFLOW));
        assert_eq!(flags.intersection(Flags::INEXACT).bits(), 0b00001);

        flags.remove(Flags::INEXACT);
        assert_eq!(flags, Flags::OVERFLOW);
        assert_eq!(flags.difference(Flags::OVERFLOW), Flags::empty());
    }
}
