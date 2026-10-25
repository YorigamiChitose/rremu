//! 模拟事件与事件接收接口。

use crate::access::{AccessSize, MemKind};

/// 模拟器事件：设备行为与系统状态变化统一经由事件流上报。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmuEvent {
    MemAccess {
        cycle: u64,
        kind: MemKind,
        size: AccessSize,
        addr: u32,
        value: u32,
        device: String,
    },
    UartOutput {
        byte: u8,
    },
    UartInput {
        byte: u8,
    },
    Trap {
        cause: u32,
        pc: u32,
    },
    Halt {
        code: i32,
    },
    /// syscon 写 `0x5555`：请求关机。
    PowerOff,
    /// syscon 写 `0x7777`：请求重启。
    Reboot,
}

/// 事件接收器。machine 层提供收集实现（环形缓冲），设备/总线只管发出。
pub trait EventSink {
    fn emit(&mut self, ev: EmuEvent);
}

/// 空实现（测试/无事件场景兜底）。
pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&mut self, _ev: EmuEvent) {}
}
