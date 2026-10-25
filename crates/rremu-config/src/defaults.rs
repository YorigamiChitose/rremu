//! 默认配置常量。

// ---------------------------------------------------------------------------
// 硬编码默认值（集中于此）
// ---------------------------------------------------------------------------

/// 默认 RAM 基址。
pub const DEFAULT_RAM_BASE: u32 = 0x8000_0000;
/// 默认 RAM 大小（16 MiB）。
pub const DEFAULT_RAM_SIZE: u32 = 16 * 1024 * 1024;
/// 默认 UART 基址。
pub const DEFAULT_UART_BASE: u32 = 0x1000_0000;
/// 默认 CLINT 基址。
pub const DEFAULT_CLINT_BASE: u32 = 0x0200_0000;
/// 默认 PLIC 基址（qemu-virt）。
pub const DEFAULT_PLIC_BASE: u32 = 0x0c00_0000;
/// 默认 syscon/test 基址（qemu-virt poweroff/reboot）。
pub const DEFAULT_SYSCON_BASE: u32 = 0x0010_0000;
/// 默认 Goldfish RTC 基址（qemu-virt）。
pub const DEFAULT_RTC_BASE: u32 = 0x0010_1000;
/// 默认 Framebuffer 基址。
pub const DEFAULT_FB_BASE: u32 = 0x2000_0000;
/// 默认 Framebuffer 宽度。
pub const DEFAULT_FB_WIDTH: u32 = 640;
/// 默认 Framebuffer 高度。
pub const DEFAULT_FB_HEIGHT: u32 = 480;

/// UART 设备尺寸（字节）。
pub const UART_SIZE: u32 = 8;
/// CLINT 设备尺寸（覆盖 msip/mtimecmp/mtime）。
pub const CLINT_SIZE: u32 = 0xC000;
/// PLIC 映射尺寸（qemu-virt）。
pub const PLIC_SIZE: u32 = 0x40_0000;
/// syscon/test 映射尺寸。
pub const SYSCON_SIZE: u32 = 0x1000;
/// Goldfish RTC 映射尺寸。
pub const RTC_SIZE: u32 = 0x1000;
/// Framebuffer 每像素字节数（RGB565）。
pub const FB_BPP: u32 = 2;
/// DTB 放置在 RAM 顶部下方保留的字节数。
pub const DTB_RESERVE: u32 = 0x10_0000;

/// 默认指令 trace 深度。
pub const DEFAULT_TRACE_CAPACITY: usize = 32;
/// 默认事件环容量。
pub const DEFAULT_EVENT_CAPACITY: usize = 64;
/// 默认 UART 事件环容量（串口 scrollback，同时容纳输入/输出事件）。
/// 需足够大，否则交互时最旧字节被挤出会导致控制台开头跳动。
pub const DEFAULT_UART_EVENT_CAPACITY: usize = 32768;
