//! 帧缓冲访问器与零拷贝帧引用。

use rremu_bus::{Bus, BusRouter};

/// 帧缓冲访问器：持有帧缓冲配置（基址/尺寸），封装对总线帧源设备的访问。
pub(crate) struct FramebufferAccessor {
    base: Option<u32>,
    width: u32,
    height: u32,
}

impl FramebufferAccessor {
    pub(crate) fn new(base: Option<u32>, width: u32, height: u32) -> Self {
        Self {
            base,
            width,
            height,
        }
    }

    pub(crate) fn dimensions(&self) -> Option<(u32, u32)> {
        self.base.map(|_| (self.width, self.height))
    }

    pub(crate) fn base(&self) -> Option<u32> {
        self.base
    }

    /// 全帧只读访问（不清 dirty）。
    pub(crate) fn full_frame<'a>(&self, bus: &'a mut BusRouter) -> Option<FramebufferFrame<'a>> {
        let fb = bus.device_mut(self.base?)?.as_frame_source()?;
        Some(FramebufferFrame {
            width: fb.width(),
            height: fb.height(),
            pixels: fb.frame(),
        })
    }

    /// dirty 驱动零拷贝访问：dirty 时清标志并返回帧，否则 None。
    pub(crate) fn dirty_frame<'a>(&self, bus: &'a mut BusRouter) -> Option<FramebufferFrame<'a>> {
        let fb = bus.device_mut(self.base?)?.as_frame_source()?;
        if !fb.is_dirty() {
            return None;
        }
        fb.clear_dirty();
        Some(FramebufferFrame {
            width: fb.width(),
            height: fb.height(),
            pixels: fb.frame(),
        })
    }
}

/// 帧缓冲零拷贝引用（窗口/TUI 刷新用）。
///
/// `pixels` 生命周期绑定到 `&mut Machine`，调用方需在再次修改 machine 前
/// 用完（如完成像素拷贝/上屏）。dirty 驱动的获取会消费一次 dirty 标志；
/// 全量获取则不改变 dirty。
#[derive(Debug)]
pub struct FramebufferFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u32],
}
