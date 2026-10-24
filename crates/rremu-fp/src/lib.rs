//! rremu-fp：纯 Rust 的 IEEE-754 浮点基础层。
//!
//! 本 crate 只处理原始位模式、舍入模式、异常标志和浮点分类，
//! 不依赖宿主 `f32`/`f64` 的运行时语义，也不依赖 CPU 或总线。
//! 具体的加、减、乘、除、开方和融合乘加在后续模块中实现。

#![no_std]

pub mod flags;
pub mod format;
pub mod round;

pub use flags::Flags;
pub use format::{Fp32, Fp64, FpClass};
pub use round::RoundingMode;
