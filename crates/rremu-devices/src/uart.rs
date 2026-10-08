//! UART 设备：NS16550A 子集（默认基址 0x1000_0000，8 字节区间）。
//!
//! | offset | 寄存器      | 行为 |
//! |--------|-------------|------|
//! | 0      | THR(写)/RDR(读) | 写 → `UartOutput` 事件；读 → 弹出输入 FIFO |
//! | 1      | IER         | 可写可读，暂不产生中断 |
//! | 2      | FCR(写)/ISR(读) | 可写可读 |
//! | 3      | LCR         | 可写可读 |
//! | 5      | LSR         | 恒 `0x60`；输入 FIFO 非空置 bit0 |
//! | 其余   | —           | 读 0，写忽略 |

use core::any::Any;
use rremu_bus::{AccessSize, BusDevice, BusError, EventSink, EmuEvent};

const REG_THR_RDR: u32 = 0;
const REG_IER: u32 = 1;
const REG_FCR_ISR: u32 = 2;
const REG_LCR: u32 = 3;
const REG_LSR: u32 = 5;
const LSR_THR_EMPTY: u32 = 0x60;
const LSR_DATA_READY: u32 = 0x01;

pub struct Uart {
    base: u32,
    ier: u32,
    isr: u32,
    lcr: u32,
    input: std::collections::VecDeque<u8>,
}

impl Uart {
    pub fn new(base: u32) -> Self {
        Self { base, ier: 0, isr: 0, lcr: 0, input: std::collections::VecDeque::new() }
    }

    /// 注入输入字节（machine 转发；进入 FIFO 等待固件读走）。
    pub fn push_input(&mut self, byte: u8) {
        self.input.push_back(byte);
    }

    fn offset(&self, addr: u32) -> u32 {
        addr - self.base
    }
}

impl BusDevice for Uart {
    fn read(&mut self, addr: u32, _size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
        match self.offset(addr) {
            REG_THR_RDR => {
                // 读 RDR：弹出输入 FIFO（副作用读）
                let byte = self.input.pop_front().unwrap_or(0);
                Ok(u32::from(byte))
            }
            REG_IER => Ok(self.ier),
            REG_FCR_ISR => Ok(self.isr),
            REG_LCR => Ok(self.lcr),
            REG_LSR => Ok(LSR_THR_EMPTY | if self.input.is_empty() { 0 } else { LSR_DATA_READY }),
            _ => Ok(0),
        }
    }

    fn write(&mut self, addr: u32, _size: AccessSize, data: u32, sink: &mut dyn EventSink) -> Result<(), BusError> {
        match self.offset(addr) {
            REG_THR_RDR => sink.emit(EmuEvent::UartOutput { byte: (data & 0xff) as u8 }),
            REG_IER => self.ier = data & 0xff,
            REG_FCR_ISR => self.isr = data & 0xff,
            REG_LCR => self.lcr = data & 0xff,
            _ => {}
        }
        Ok(())
    }

    fn peek_read(&mut self, addr: u32, _size: AccessSize) -> Result<u32, BusError> {
        match self.offset(addr) {
            // RDR peek 返回队首但不弹出（D12）
            REG_THR_RDR => Ok(u32::from(self.input.front().copied().unwrap_or(0))),
            REG_IER => Ok(self.ier),
            REG_FCR_ISR => Ok(self.isr),
            REG_LCR => Ok(self.lcr),
            REG_LSR => Ok(LSR_THR_EMPTY | if self.input.is_empty() { 0 } else { LSR_DATA_READY }),
            _ => Ok(0),
        }
    }

    fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
        Err(BusError::Device { addr: self.base, msg: "cannot bulk-write to uart".to_owned() })
    }

    fn device_name(&self) -> &str {
        "uart"
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Collector(Vec<EmuEvent>);
    impl EventSink for Collector {
        fn emit(&mut self, ev: EmuEvent) {
            self.0.push(ev);
        }
    }

    const BASE: u32 = 0x1000_0000;

    #[test]
    fn thr_write_emits_uart_output() {
        let mut uart = Uart::new(BASE);
        let mut sink = Collector::default();
        uart.write(BASE + 0, AccessSize::Byte, b'h' as u32, &mut sink).unwrap();
        uart.write(BASE + 0, AccessSize::Word, b'i' as u32, &mut sink).unwrap();
        assert_eq!(sink.0.len(), 2);
        assert!(matches!(sink.0[0], EmuEvent::UartOutput { byte: b'h' }));
        assert!(matches!(sink.0[1], EmuEvent::UartOutput { byte: b'i' }));
    }

    #[test]
    fn lsr_and_rdr_side_effects() {
        let mut uart = Uart::new(BASE);
        let mut sink = Collector::default();
        // 空闲：LSR = 0x60
        assert_eq!(uart.read(BASE + 5, AccessSize::Byte, &mut sink).unwrap(), 0x60);
        // 空读 RDR 返回 0，且无事件
        assert_eq!(uart.read(BASE + 0, AccessSize::Byte, &mut sink).unwrap(), 0);
        assert!(sink.0.is_empty());
        // 注入输入：LSR bit0 置位
        uart.push_input(b'A');
        uart.push_input(b'B');
        assert_eq!(uart.read(BASE + 5, AccessSize::Byte, &mut sink).unwrap(), 0x61);
        // 读 RDR 按序弹出
        assert_eq!(uart.read(BASE + 0, AccessSize::Byte, &mut sink).unwrap(), b'A' as u32);
        assert_eq!(uart.read(BASE + 0, AccessSize::Byte, &mut sink).unwrap(), b'B' as u32);
        // 弹空后 LSR 回 0x60
        assert_eq!(uart.read(BASE + 5, AccessSize::Byte, &mut sink).unwrap(), 0x60);
    }

    #[test]
    fn rdr_peek_does_not_pop() {
        let mut uart = Uart::new(BASE);
        let mut sink = Collector::default();
        uart.push_input(b'X');
        // peek 不消耗
        assert_eq!(uart.peek_read(BASE + 0, AccessSize::Byte).unwrap(), b'X' as u32);
        assert_eq!(uart.peek_read(BASE + 0, AccessSize::Byte).unwrap(), b'X' as u32);
        // 真读弹出一个
        assert_eq!(uart.read(BASE + 0, AccessSize::Byte, &mut sink).unwrap(), b'X' as u32);
        assert_eq!(uart.peek_read(BASE + 0, AccessSize::Byte).unwrap(), 0);
    }

    #[test]
    fn config_registers_store_read() {
        let mut uart = Uart::new(BASE);
        let mut sink = Collector::default();
        uart.write(BASE + 1, AccessSize::Byte, 0x07, &mut sink).unwrap(); // IER
        uart.write(BASE + 2, AccessSize::Byte, 0xC1, &mut sink).unwrap(); // FCR
        uart.write(BASE + 3, AccessSize::Byte, 0x03, &mut sink).unwrap(); // LCR
        assert_eq!(uart.peek_read(BASE + 1, AccessSize::Byte).unwrap(), 0x07);
        assert_eq!(uart.peek_read(BASE + 2, AccessSize::Byte).unwrap(), 0xC1);
        assert_eq!(uart.peek_read(BASE + 3, AccessSize::Byte).unwrap(), 0x03);
        // 未知寄存器读 0 写忽略
        uart.write(BASE + 4, AccessSize::Byte, 0xff, &mut sink).unwrap();
        assert_eq!(uart.read(BASE + 4, AccessSize::Byte, &mut sink).unwrap(), 0);
        assert_eq!(uart.read(BASE + 6, AccessSize::Byte, &mut sink).unwrap(), 0);
    }
}
