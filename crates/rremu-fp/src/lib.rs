//! rremu-fp：纯 Rust 的 IEEE-754 浮点基础层。
//!
//! 本 crate 只处理原始位模式、舍入模式、异常标志、浮点分类，
//! 以及不依赖宿主浮点环境的解码/规范化/舍入/编码原语。
//! 单精度运算在 [`add`]、[`muldiv`]、[`sqrt`] 中实现，
//! 双精度运算在 [`add64`]、[`muldiv64`]、[`sqrt64`] 中实现；
//! 融合乘加在后续模块中实现。
//!
//! 所有原语都使用整数和位操作，禁止先转换为宿主 `f32`/`f64`。

#![no_std]

pub mod add;
pub mod add64;
pub mod flags;
pub mod format;
pub mod muldiv;
pub mod muldiv64;
pub mod round;
pub mod softfloat;
pub mod sqrt;
pub mod sqrt64;

pub use add::{add32, sub32};
pub use add64::{add64, sub64};
pub use flags::Flags;
pub use format::{Fp32, Fp64, FpClass};
pub use muldiv::{div32, mul32};
pub use muldiv64::{div64, mul64};
pub use round::RoundingMode;
pub use softfloat::{
    Kind, Unpacked32, Unpacked64, normalize32, normalize64, round_pack32, round_pack64, unpack32,
    unpack64,
};
pub use sqrt::sqrt32;
pub use sqrt64::sqrt64;
