//! 事件收集器与 UART 直出 sink。

use std::collections::VecDeque;

use rremu_bus::{EmuEvent, EventSink};

use crate::term::Terminal;

/// 事件环形收集器：machine 持有，前端经快照读取。
/// mem 环默认 16 条 MemAccess，uart 环默认 1024 字节。
pub struct RingCollector {
    mem: VecDeque<EmuEvent>,
    uart: VecDeque<EmuEvent>,
    halt: Option<i32>,
    mem_cap: usize,
    uart_cap: usize,
}

impl RingCollector {
    pub fn new() -> Self {
        Self {
            mem: VecDeque::new(),
            uart: VecDeque::new(),
            halt: None,
            mem_cap: 16,
            uart_cap: 1024,
        }
    }

    pub fn set_capacity(&mut self, mem: usize, uart: usize) {
        self.mem_cap = mem;
        self.uart_cap = uart;
        self.mem.truncate(mem);
        self.uart.truncate(uart);
    }

    pub fn mem_ring(&self) -> &VecDeque<EmuEvent> {
        &self.mem
    }

    pub fn uart_ring(&self) -> &VecDeque<EmuEvent> {
        &self.uart
    }

    pub fn take_halt(&mut self) -> Option<i32> {
        self.halt.take()
    }

    /// 取走全部 UART 输出字节（CLI 打印用）。
    pub fn drain_uart(&mut self) -> Vec<u8> {
        self.uart
            .drain(..)
            .filter_map(|ev| Self::as_uart_output(&ev))
            .collect()
    }

    /// 从事件提取 UART 输出字节（输入事件与其他事件不算输出）。
    fn as_uart_output(ev: &EmuEvent) -> Option<u8> {
        if let EmuEvent::UartOutput { byte } = ev {
            Some(*byte)
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.mem.clear();
        self.uart.clear();
        self.halt = None;
    }
}

impl Default for RingCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for RingCollector {
    fn emit(&mut self, ev: EmuEvent) {
        match ev {
            EmuEvent::MemAccess { .. } | EmuEvent::Trap { .. } => {
                // Trap 与存储访问同在 mem 环展示（受同一容量约束）
                if self.mem.len() == self.mem_cap {
                    self.mem.pop_front();
                }
                self.mem.push_back(ev);
            }
            EmuEvent::UartOutput { .. } | EmuEvent::UartInput { .. } => {
                if self.uart.len() == self.uart_cap {
                    self.uart.pop_front();
                }
                self.uart.push_back(ev);
            }
            EmuEvent::Halt { code } => self.halt = Some(code),
            EmuEvent::PowerOff => self.halt = Some(0),
            // 重启不是关机：用非零退出码区分，避免把 reboot 当成 poweroff。
            EmuEvent::Reboot => self.halt = Some(2),
        }
    }
}

/// 事件 sink：所有事件入 collector；UART 输出字节**同时**喂给终端状态机。
pub(crate) struct MachineSink<'a> {
    collector: &'a mut RingCollector,
    terminal: &'a mut Terminal,
    /// CLI 直出回调：UART 输出字节在进入事件环前同步转发。
    uart_sink: &'a mut Option<Box<dyn FnMut(u8)>>,
}

impl<'a> MachineSink<'a> {
    pub(crate) fn new(
        collector: &'a mut RingCollector,
        terminal: &'a mut Terminal,
        uart_sink: &'a mut Option<Box<dyn FnMut(u8)>>,
    ) -> Self {
        Self {
            collector,
            terminal,
            uart_sink,
        }
    }
}

impl EventSink for MachineSink<'_> {
    fn emit(&mut self, ev: EmuEvent) {
        if let EmuEvent::UartOutput { byte } = ev {
            self.terminal.feed(byte);
            if let Some(sink) = self.uart_sink.as_mut() {
                sink(byte);
            }
        }
        self.collector.emit(ev);
    }
}
