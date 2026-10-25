//! 硬件布局与设备区间。

use crate::defaults::*;

// ---------------------------------------------------------------------------
// 结构化配置
// ---------------------------------------------------------------------------

/// 一段设备区间（基址 + 尺寸）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Device {
    pub base: u32,
    pub size: u32,
}

/// Framebuffer 布局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbLayout {
    pub base: u32,
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
}

/// 硬件布局（guest 可见的地址空间）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub ram_base: u32,
    pub ram_size: u32,
    /// `None` 表示不映射 UART。
    pub uart: Option<Device>,
    /// `None` 表示不映射 CLINT。
    pub clint: Option<Device>,
    /// `None` 表示不映射 PLIC。
    pub plic: Option<Device>,
    /// `None` 表示不映射 syscon/test。
    pub syscon: Option<Device>,
    /// `None` 表示不映射 Goldfish RTC。
    pub rtc: Option<Device>,
    /// `None` 表示不映射 Framebuffer。
    pub fb: Option<FbLayout>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            ram_base: DEFAULT_RAM_BASE,
            ram_size: DEFAULT_RAM_SIZE,
            uart: Some(Device {
                base: DEFAULT_UART_BASE,
                size: UART_SIZE,
            }),
            clint: Some(Device {
                base: DEFAULT_CLINT_BASE,
                size: CLINT_SIZE,
            }),
            plic: Some(Device {
                base: DEFAULT_PLIC_BASE,
                size: PLIC_SIZE,
            }),
            syscon: Some(Device {
                base: DEFAULT_SYSCON_BASE,
                size: SYSCON_SIZE,
            }),
            rtc: Some(Device {
                base: DEFAULT_RTC_BASE,
                size: RTC_SIZE,
            }),
            fb: Some(FbLayout {
                base: DEFAULT_FB_BASE,
                width: DEFAULT_FB_WIDTH,
                height: DEFAULT_FB_HEIGHT,
                bpp: FB_BPP,
            }),
        }
    }
}
