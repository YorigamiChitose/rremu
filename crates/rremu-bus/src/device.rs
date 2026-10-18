//! 总线设备接口与能力下取。

use crate::access::{AccessSize, BusError};
use crate::event::EventSink;

pub trait Resetable {
    fn reset(&mut self);
}

/// 输入接收设备：注入外部输入字节（如 UART）。
pub trait InputSink {
    fn push_input(&mut self, byte: u8);
}

/// 帧源设备：提供只读帧缓冲访问与 dirty 管理（如 Framebuffer）。
pub trait FrameSource {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn frame(&self) -> &[u32];
    fn is_dirty(&self) -> bool;
    fn clear_dirty(&mut self);
}

/// 总线设备 trait。地址为**绝对地址**，设备内部自行换算偏移。
pub trait BusDevice {
    /// 有副作用读（如弹 UART FIFO）。
    ///
    /// # Arguments
    /// * `sink` - 设备可在此发出自身事件（如 UartOutput）
    fn read(&mut self, addr: u32, size: AccessSize, sink: &mut dyn EventSink) -> Result<u32, BusError>;
    /// 写（可有副作用）。
    fn write(&mut self, addr: u32, size: AccessSize, data: u32, sink: &mut dyn EventSink) -> Result<(), BusError>;
    /// 无副作用读（peek_read）。**强制每个设备显式实现**，不得有默认实现。
    /// 对监视点/表达式求值/x 命令开放，不得消耗设备状态。
    fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError>;
    /// 批量写入（镜像加载用，无事件上报）。不支持时返回 `Device` 错误。
    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError>;
    fn device_name(&self) -> &str;

    /// 能力下取：返回可复位视图（仅 RAM 等覆写）。
    fn as_resetable(&mut self) -> Option<&mut dyn Resetable> { None }
    /// 能力下取：返回输入接收视图（仅 UART 等覆写）。
    fn as_input_sink(&mut self) -> Option<&mut dyn InputSink> { None }
    /// 能力下取：返回帧源视图（仅 Framebuffer 等覆写）。
    fn as_frame_source(&mut self) -> Option<&mut dyn FrameSource> { None }

    /// 检查设备的全部 pending 中断（返回 cause 码列表，每个含 bit31）。
    /// 默认空；CLINT 等中断源设备覆写此方法。
    fn check_interrupt(&self) -> Vec<u32> { Vec::new() }

    /// 本设备当前向中断控制器断言的 IRQ 源（PLIC source 号）。
    /// 默认 `None`；外设（如 UART）在有事时返回源号，由 `BusRouter`
    /// 在中断检查时喂给中断控制器。
    fn irq_line(&self) -> Option<u32> { None }

    /// 中断控制器接收设备 IRQ 断言（默认空操作；PLIC 覆写为置 pending）。
    fn irq_assert(&mut self, _source: u32) {}

    /// 每步周期同步：设备按 cycle 更新内部时序状态（如 CLINT mtime）。
    /// 默认空操作。由 `BusRouter::set_cycle` 遍历调用。
    fn tick(&mut self, _cycle: u64) {}
}

