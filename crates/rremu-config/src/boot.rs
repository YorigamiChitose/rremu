//! 启动、诊断与 host 路径配置。

use crate::defaults::*;

/// 诊断/调试配置（与硬件无关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diag {
    pub trace_capacity: usize,
    pub event_capacity: usize,
    pub uart_event_capacity: usize,
}

/// 启动上下文（复位后注入到 a0/a1）。
///
/// qemu-virt 约定：`a0 = hartid`、`a1 = dtb 物理地址`。
/// RT-Thread 可能不用；OpenSBI/Linux 必需。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Boot {
    pub hartid: u32,
    pub dtb: Option<u32>,
}

/// host 侧路径（dump/ELF/UART 输入）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paths {
    pub elf: Option<String>,
    pub dump_on_exit: Option<String>,
    /// UART 输入源（文件/FIFO）；`None` 表示不自动读取输入。
    pub uart_in: Option<String>,
}

impl Default for Diag {
    fn default() -> Self {
        Self {
            trace_capacity: DEFAULT_TRACE_CAPACITY,
            event_capacity: DEFAULT_EVENT_CAPACITY,
            uart_event_capacity: DEFAULT_UART_EVENT_CAPACITY,
        }
    }
}

