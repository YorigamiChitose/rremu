//! UART 设备：NS16550A 子集（默认基址 0x1000_0000，8 字节区间）。
//!
//! | offset | 寄存器      | 行为 |
//! |--------|-------------|------|
//! | 0      | THR(写)/RDR(读) | 写 → `UartOutput` 事件；读 → 弹出输入 FIFO |
//! | 1      | IER         | RX / TX 中断使能 |
//! | 2      | FCR(写)/IIR(读) | FIFO 控制 / 中断标识 |
//! | 3      | LCR         | 可写可读 |
//! | 5      | LSR         | 恒 `0x60`；输入 FIFO 非空置 bit0 |
//! | 其余   | —           | 读 0，写忽略 |

use rremu_bus::{AccessSize, BusDevice, BusError, EventSink, EmuEvent, InputSink, Resetable};

const REG_THR_RDR: u32 = 0;
const REG_IER: u32 = 1;
const REG_FCR_ISR: u32 = 2;
const REG_LCR: u32 = 3;
const REG_LSR: u32 = 5;
const LSR_THR_EMPTY: u32 = 0x60;
const LSR_DATA_READY: u32 = 0x01;

/// IER bit0：接收数据可中断。
const IER_RX_ENABLE: u32 = 0x01;
/// IER bit1：发送保持寄存器空（THRE）中断。
const IER_TX_ENABLE: u32 = 0x02;
/// PLIC 源号（qemu-virt 约定：UART0 = 10）。
pub const UART_IRQ: u32 = 10;

pub struct Uart {
    base: u32,
    ier: u32,
    fcr: u32,
    lcr: u32,
    /// THRE 中断 pending；读 IIR 或写 THR 后更新。
    thre_pending: bool,
    input: std::collections::VecDeque<u8>,
}

impl Uart {
    pub fn new(base: u32) -> Self {
        Self {
            base,
            ier: 0,
            fcr: 0,
            lcr: 0,
            thre_pending: false,
            input: std::collections::VecDeque::new(),
        }
    }

    /// 注入输入字节（machine 转发；进入 FIFO 等待固件读走）。
    pub fn push_input(&mut self, byte: u8) {
        self.input.push_back(byte);
    }

    fn offset(&self, addr: u32) -> u32 {
        addr - self.base
    }

    /// 8250 IIR：RX 优先，其次 THRE；0x01 表示无 pending。
    fn iir(&self) -> u32 {
        if self.ier & IER_RX_ENABLE != 0 && !self.input.is_empty() {
            0x04
        } else if self.ier & IER_TX_ENABLE != 0 && self.thre_pending {
            0x02
        } else {
            0x01
        }
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
            REG_FCR_ISR => {
                let iir = self.iir();
                if iir == 0x02 {
                    // THRE 中断由读 IIR 清除；下一次 THR 写会重新置位。
                    self.thre_pending = false;
                }
                Ok(iir)
            }
            REG_LCR => Ok(self.lcr),
            REG_LSR => Ok(LSR_THR_EMPTY | if self.input.is_empty() { 0 } else { LSR_DATA_READY }),
            _ => Ok(0),
        }
    }

    fn write(&mut self, addr: u32, _size: AccessSize, data: u32, sink: &mut dyn EventSink) -> Result<(), BusError> {
        match self.offset(addr) {
            REG_THR_RDR => {
                sink.emit(EmuEvent::UartOutput { byte: (data & 0xff) as u8 });
                // THR 再次为空，若 TX 中断使能则触发下一次 THRE。
                self.thre_pending = true;
            }
            REG_IER => {
                self.ier = data & 0xff;
                if self.ier & IER_TX_ENABLE != 0 {
                    self.thre_pending = true;
                }
            }
            REG_FCR_ISR => self.fcr = data & 0xff,
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
            REG_FCR_ISR => {
                let iir = self.iir();
                if iir == 0x02 {
                    // THRE 中断由读 IIR 清除；下一次 THR 写会重新置位。
                    self.thre_pending = false;
                }
                Ok(iir)
            }
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

    fn as_input_sink(&mut self) -> Option<&mut dyn InputSink> {
        Some(self)
    }

    /// RX 有数据或 THRE pending 且对应 IER 位使能时断言 UART0 IRQ（level）。
    fn irq_line(&self) -> Option<u32> {
        if self.iir() != 0x01 {
            Some(UART_IRQ)
        } else {
            None
        }
    }

    fn as_resetable(&mut self) -> Option<&mut dyn Resetable> {
        Some(self)
    }
}

impl Resetable for Uart {
    fn reset(&mut self) {
        self.input.clear();
        self.ier = 0;
        self.fcr = 0;
        self.lcr = 0;
        self.thre_pending = false;
    }
}

impl InputSink for Uart {
    fn push_input(&mut self, byte: u8) {
        Uart::push_input(self, byte);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
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
        uart.write(BASE, AccessSize::Byte, b'h' as u32, &mut sink).unwrap();
        uart.write(BASE, AccessSize::Word, b'i' as u32, &mut sink).unwrap();
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
        assert_eq!(uart.read(BASE, AccessSize::Byte, &mut sink).unwrap(), 0);
        assert!(sink.0.is_empty());
        // 注入输入：LSR bit0 置位
        uart.push_input(b'A');
        uart.push_input(b'B');
        assert_eq!(uart.read(BASE + 5, AccessSize::Byte, &mut sink).unwrap(), 0x61);
        // 读 RDR 按序弹出
        assert_eq!(uart.read(BASE, AccessSize::Byte, &mut sink).unwrap(), b'A' as u32);
        assert_eq!(uart.read(BASE, AccessSize::Byte, &mut sink).unwrap(), b'B' as u32);
        // 弹空后 LSR 回 0x60
        assert_eq!(uart.read(BASE + 5, AccessSize::Byte, &mut sink).unwrap(), 0x60);
    }

    #[test]
    fn rdr_peek_does_not_pop() {
        let mut uart = Uart::new(BASE);
        let mut sink = Collector::default();
        uart.push_input(b'X');
        // peek 不消耗
        assert_eq!(uart.peek_read(BASE, AccessSize::Byte).unwrap(), b'X' as u32);
        assert_eq!(uart.peek_read(BASE, AccessSize::Byte).unwrap(), b'X' as u32);
        // 真读弹出一个
        assert_eq!(uart.read(BASE, AccessSize::Byte, &mut sink).unwrap(), b'X' as u32);
        assert_eq!(uart.peek_read(BASE, AccessSize::Byte).unwrap(), 0);
    }

    #[test]
    fn thre_interrupt_drives_tx_and_clears_on_iir_read() {
        let mut uart = Uart::new(BASE);
        let mut sink = Collector::default();
        uart.write(BASE + 1, AccessSize::Byte, IER_TX_ENABLE, &mut sink).unwrap();
        assert_eq!(uart.irq_line(), Some(UART_IRQ));
        assert_eq!(uart.read(BASE + 2, AccessSize::Byte, &mut sink).unwrap(), 0x02);
        assert_eq!(uart.irq_line(), None);
        uart.write(BASE, AccessSize::Byte, b'X' as u32, &mut sink).unwrap();
        assert_eq!(uart.irq_line(), Some(UART_IRQ));
        assert!(matches!(sink.0.last(), Some(EmuEvent::UartOutput { byte: b'X' })));
    }

    #[test]
    fn reset_clears_fifo_and_registers() {
        let mut uart = Uart::new(BASE);
        let mut sink = Collector::default();
        uart.push_input(b'A');
        uart.write(BASE + 1, AccessSize::Byte, 0x07, &mut sink).unwrap(); // IER
        uart.write(BASE + 3, AccessSize::Byte, 0x03, &mut sink).unwrap(); // LCR
        Resetable::reset(&mut uart);
        // FIFO 空 → LSR 回 0x60
        assert_eq!(uart.read(BASE + 5, AccessSize::Byte, &mut sink).unwrap(), 0x60);
        assert_eq!(uart.peek_read(BASE + 1, AccessSize::Byte).unwrap(), 0); // IER 清零
        assert_eq!(uart.peek_read(BASE + 3, AccessSize::Byte).unwrap(), 0); // LCR 清零
    }

    #[test]
    fn config_registers_store_read() {
        let mut uart = Uart::new(BASE);
        let mut sink = Collector::default();
        uart.write(BASE + 1, AccessSize::Byte, 0x07, &mut sink).unwrap(); // IER
        uart.write(BASE + 2, AccessSize::Byte, 0xC1, &mut sink).unwrap(); // FCR
        uart.write(BASE + 3, AccessSize::Byte, 0x03, &mut sink).unwrap(); // LCR
        assert_eq!(uart.peek_read(BASE + 1, AccessSize::Byte).unwrap(), 0x07);
        assert_eq!(uart.peek_read(BASE + 2, AccessSize::Byte).unwrap(), 0x02); // THRE pending
        assert_eq!(uart.peek_read(BASE + 3, AccessSize::Byte).unwrap(), 0x03);
        // 未知寄存器读 0 写忽略
        uart.write(BASE + 4, AccessSize::Byte, 0xff, &mut sink).unwrap();
        assert_eq!(uart.read(BASE + 4, AccessSize::Byte, &mut sink).unwrap(), 0);
        assert_eq!(uart.read(BASE + 6, AccessSize::Byte, &mut sink).unwrap(), 0);
    }
}
