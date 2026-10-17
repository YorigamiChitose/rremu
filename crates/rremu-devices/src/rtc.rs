//! Goldfish RTC：Linux `rtc-goldfish` 兼容的最小子集。
//!
//! | offset | 寄存器 |
//! |--------|--------|
//! | 0x00 | TIME_LOW |
//! | 0x04 | TIME_HIGH |
//! | 0x08 | ALARM_LOW |
//! | 0x0c | ALARM_HIGH |
//! | 0x10 | IRQ_ENABLED |
//! | 0x14 | CLEAR_ALARM |
//! | 0x18 | ALARM_STATUS |
//! | 0x1c | CLEAR_INTERRUPT |
//!
//! 时间来自 host 墙钟；alarm 中断暂未注入 PLIC（Linux 启动不依赖 alarm）。

use std::time::{SystemTime, UNIX_EPOCH};

use rremu_bus::{AccessSize, BusDevice, BusError, EventSink, Resetable};

const TIME_LOW: u32 = 0x00;
const TIME_HIGH: u32 = 0x04;
const ALARM_LOW: u32 = 0x08;
const ALARM_HIGH: u32 = 0x0c;
const IRQ_ENABLED: u32 = 0x10;
const CLEAR_ALARM: u32 = 0x14;
const ALARM_STATUS: u32 = 0x18;
const CLEAR_INTERRUPT: u32 = 0x1c;

/// 映射尺寸（qemu-virt）。
pub const RTC_SIZE: u32 = 0x1000;

pub struct Rtc {
    base: u32,
    alarm_low: u32,
    alarm_high: u32,
    irq_enabled: bool,
}

impl Rtc {
    pub fn new(base: u32) -> Self {
        Self { base, alarm_low: 0, alarm_high: 0, irq_enabled: false }
    }

    fn offset(&self, addr: u32) -> u32 {
        addr - self.base
    }

    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

impl BusDevice for Rtc {
    fn read(&mut self, addr: u32, size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
        let off = self.offset(addr);
        let now = self.now();
        let word = match off & !3 {
            TIME_LOW => now as u32,
            TIME_HIGH => (now >> 32) as u32,
            ALARM_LOW => self.alarm_low,
            ALARM_HIGH => self.alarm_high,
            IRQ_ENABLED => u32::from(self.irq_enabled),
            ALARM_STATUS => 0,
            _ => 0,
        };
        let shift = (off & 3) * 8;
        Ok(match size {
            AccessSize::Byte => (word >> shift) & 0xff,
            AccessSize::Half => (word >> shift) & 0xffff,
            AccessSize::Word => word,
        })
    }

    fn write(&mut self, addr: u32, size: AccessSize, data: u32, _sink: &mut dyn EventSink) -> Result<(), BusError> {
        let off = self.offset(addr);
        // 仅支持对齐的 32 位寄存器写入；Linux 驱动使用 writel。
        if size != AccessSize::Word || off & 3 != 0 {
            return Ok(());
        }
        match off {
            ALARM_LOW => self.alarm_low = data,
            ALARM_HIGH => self.alarm_high = data,
            IRQ_ENABLED => self.irq_enabled = data & 1 != 0,
            CLEAR_ALARM => {
                self.alarm_low = 0;
                self.alarm_high = 0;
            }
            CLEAR_INTERRUPT => {}
            _ => {}
        }
        Ok(())
    }

    fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
        let mut sink = rremu_bus::NullSink;
        self.read(addr, size, &mut sink)
    }

    fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
        Err(BusError::Device { addr: self.base, msg: "cannot bulk-write to rtc".to_owned() })
    }

    fn device_name(&self) -> &str {
        "rtc"
    }

    fn as_resetable(&mut self) -> Option<&mut dyn Resetable> {
        Some(self)
    }
}

impl Resetable for Rtc {
    fn reset(&mut self) {
        self.alarm_low = 0;
        self.alarm_high = 0;
        self.irq_enabled = false;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const BASE: u32 = 0x0010_1000;

    #[test]
    fn time_registers_are_readable() {
        let mut rtc = Rtc::new(BASE);
        let mut sink = rremu_bus::NullSink;
        let lo = rtc.read(BASE + TIME_LOW, AccessSize::Word, &mut sink).unwrap();
        let hi = rtc.read(BASE + TIME_HIGH, AccessSize::Word, &mut sink).unwrap();
        assert_ne!((u64::from(hi) << 32) | u64::from(lo), 0);
    }

    #[test]
    fn alarm_roundtrip_and_clear() {
        let mut rtc = Rtc::new(BASE);
        let mut sink = rremu_bus::NullSink;
        rtc.write(BASE + ALARM_LOW, AccessSize::Word, 0x1234, &mut sink).unwrap();
        rtc.write(BASE + ALARM_HIGH, AccessSize::Word, 0x56, &mut sink).unwrap();
        assert_eq!(rtc.read(BASE + ALARM_LOW, AccessSize::Word, &mut sink).unwrap(), 0x1234);
        assert_eq!(rtc.read(BASE + ALARM_HIGH, AccessSize::Word, &mut sink).unwrap(), 0x56);
        rtc.write(BASE + CLEAR_ALARM, AccessSize::Word, 0, &mut sink).unwrap();
        assert_eq!(rtc.read(BASE + ALARM_LOW, AccessSize::Word, &mut sink).unwrap(), 0);
    }
}
