//! rremu-devices：可插拔设备模型（ram / uart / clint / plic / framebuffer）。不依赖 cpu。

pub mod clint;
pub mod framebuffer;
pub mod plic;
pub mod ram;
pub mod uart;

pub use clint::Clint;
pub use framebuffer::Framebuffer;
pub use plic::Plic;
pub use ram::Ram;
pub use uart::Uart;
