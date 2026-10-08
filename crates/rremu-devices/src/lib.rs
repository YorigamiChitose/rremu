//! rremu-devices：可插拔设备模型（ram / uart / clint / framebuffer）。不依赖 cpu。

pub mod ram;
pub mod uart;

pub use ram::Ram;
pub use uart::Uart;
