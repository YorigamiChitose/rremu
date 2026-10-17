//! rremu-devices：可插拔设备模型（ram / uart / clint / plic / rtc / syscon / framebuffer）。不依赖 cpu。

pub mod clint;
pub mod framebuffer;
pub mod plic;
pub mod ram;
pub mod rtc;
pub mod syscon;
pub mod uart;

pub use clint::Clint;
pub use framebuffer::Framebuffer;
pub use plic::Plic;
pub use ram::Ram;
pub use rtc::Rtc;
pub use syscon::Syscon;
pub use uart::Uart;
