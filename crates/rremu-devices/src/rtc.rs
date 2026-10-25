//! Goldfish RTC：Linux `rtc-goldfish` 兼容子集。
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
//! 时间与 alarm 均使用纳秒。alarm 到期后通过 PLIC source 11 断言中断；
//! 时间来源为 host 墙钟。

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
/// RTC alarm 使用的 PLIC source。
pub const RTC_IRQ: u32 = 11;

/// 每 1000 个 cycle 检查一次 host 时间，避免每条指令都读取墙钟。
const CHECK_INTERVAL: u64 = 1000;

pub struct Rtc {
    base: u32,
    alarm_low: u32,
    alarm_high: u32,
    irq_enabled: bool,
    /// alarm 已装载且 IRQ 已启用；`ALARM_STATUS` 的可见状态。
    alarm_armed: bool,
    /// alarm 时间已到，等待驱动清除；驱动 `irq_line` 的依据。
    alarm_pending: bool,
    last_check_cycle: u64,
}

impl Rtc {
    pub fn new(base: u32) -> Self {
        Self {
            base,
            alarm_low: 0,
            alarm_high: 0,
            irq_enabled: false,
            alarm_armed: false,
            alarm_pending: false,
            last_check_cycle: 0,
        }
    }

    fn offset(&self, addr: u32) -> u32 {
        addr - self.base
    }

    fn now_ns(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }

    fn alarm_ns(&self) -> u64 {
        (u64::from(self.alarm_high) << 32) | u64::from(self.alarm_low)
    }

    fn disarm_alarm(&mut self) {
        self.alarm_armed = false;
        self.alarm_pending = false;
    }

    fn refresh_alarm(&mut self) {
        if !self.irq_enabled || !self.alarm_armed {
            self.alarm_pending = false;
            return;
        }
        let alarm = self.alarm_ns();
        if alarm != 0 && self.now_ns() >= alarm {
            self.alarm_pending = true;
        }
    }

    fn decode(&self, off: u32, size: AccessSize, now: u64) -> u32 {
        let word = match off & !3 {
            TIME_LOW => now as u32,
            TIME_HIGH => (now >> 32) as u32,
            ALARM_LOW => self.alarm_low,
            ALARM_HIGH => self.alarm_high,
            IRQ_ENABLED => u32::from(self.irq_enabled),
            ALARM_STATUS => u32::from(self.alarm_armed),
            _ => 0,
        };
        let shift = (off & 3) * 8;
        match size {
            AccessSize::Byte => (word >> shift) & 0xff,
            AccessSize::Half => (word >> shift) & 0xffff,
            AccessSize::Word => word,
        }
    }
}

impl BusDevice for Rtc {
    fn read(
        &mut self,
        addr: u32,
        size: AccessSize,
        _sink: &mut dyn EventSink,
    ) -> Result<u32, BusError> {
        self.refresh_alarm();
        let off = self.offset(addr);
        Ok(self.decode(off, size, self.now_ns()))
    }

    fn write(
        &mut self,
        addr: u32,
        size: AccessSize,
        data: u32,
        _sink: &mut dyn EventSink,
    ) -> Result<(), BusError> {
        let off = self.offset(addr);
        // 仅支持对齐的 32 位寄存器写入；Linux 驱动使用 writel。
        if size != AccessSize::Word || off & 3 != 0 {
            return Ok(());
        }
        match off {
            ALARM_LOW => {
                self.alarm_low = data;
                self.disarm_alarm();
            }
            ALARM_HIGH => {
                self.alarm_high = data;
                self.disarm_alarm();
            }
            IRQ_ENABLED => {
                self.irq_enabled = data & 1 != 0;
                if self.irq_enabled {
                    self.alarm_armed = self.alarm_ns() != 0;
                    self.alarm_pending = false;
                    self.refresh_alarm();
                } else {
                    self.disarm_alarm();
                }
            }
            CLEAR_ALARM => {
                self.alarm_low = 0;
                self.alarm_high = 0;
                self.disarm_alarm();
            }
            CLEAR_INTERRUPT => self.disarm_alarm(),
            _ => {}
        }
        Ok(())
    }

    fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
        // peek 不改变 alarm 状态；时间读取仍无副作用。
        let off = self.offset(addr);
        Ok(self.decode(off, size, self.now_ns()))
    }

    fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
        Err(BusError::Device {
            addr: self.base,
            msg: "cannot bulk-write to rtc".to_owned(),
        })
    }

    fn device_name(&self) -> &str {
        "rtc"
    }

    fn as_resetable(&mut self) -> Option<&mut dyn Resetable> {
        Some(self)
    }

    fn irq_line(&self) -> Option<u32> {
        (self.irq_enabled && self.alarm_pending).then_some(RTC_IRQ)
    }

    fn tick(&mut self, cycle: u64) {
        if cycle.wrapping_sub(self.last_check_cycle) < CHECK_INTERVAL {
            return;
        }
        self.last_check_cycle = cycle;
        self.refresh_alarm();
    }
}

impl Resetable for Rtc {
    fn reset(&mut self) {
        self.alarm_low = 0;
        self.alarm_high = 0;
        self.irq_enabled = false;
        self.alarm_armed = false;
        self.alarm_pending = false;
        self.last_check_cycle = 0;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const BASE: u32 = 0x0010_1000;

    fn write_alarm(rtc: &mut Rtc, alarm_ns: u64) {
        let mut sink = rremu_bus::NullSink;
        rtc.write(
            BASE + ALARM_LOW,
            AccessSize::Word,
            alarm_ns as u32,
            &mut sink,
        )
        .unwrap();
        rtc.write(
            BASE + ALARM_HIGH,
            AccessSize::Word,
            (alarm_ns >> 32) as u32,
            &mut sink,
        )
        .unwrap();
        rtc.write(BASE + IRQ_ENABLED, AccessSize::Word, 1, &mut sink)
            .unwrap();
    }

    #[test]
    fn time_registers_are_nanoseconds() {
        let mut rtc = Rtc::new(BASE);
        let mut sink = rremu_bus::NullSink;
        let lo = rtc
            .read(BASE + TIME_LOW, AccessSize::Word, &mut sink)
            .unwrap();
        let hi = rtc
            .read(BASE + TIME_HIGH, AccessSize::Word, &mut sink)
            .unwrap();
        let now = (u64::from(hi) << 32) | u64::from(lo);
        assert!(now > 1_000_000_000);
    }

    #[test]
    fn alarm_roundtrip_and_clear() {
        let mut rtc = Rtc::new(BASE);
        let mut sink = rremu_bus::NullSink;
        rtc.write(BASE + ALARM_LOW, AccessSize::Word, 0x1234, &mut sink)
            .unwrap();
        rtc.write(BASE + ALARM_HIGH, AccessSize::Word, 0x56, &mut sink)
            .unwrap();
        assert_eq!(
            rtc.read(BASE + ALARM_LOW, AccessSize::Word, &mut sink)
                .unwrap(),
            0x1234
        );
        assert_eq!(
            rtc.read(BASE + ALARM_HIGH, AccessSize::Word, &mut sink)
                .unwrap(),
            0x56
        );
        rtc.write(BASE + CLEAR_ALARM, AccessSize::Word, 0, &mut sink)
            .unwrap();
        assert_eq!(
            rtc.read(BASE + ALARM_LOW, AccessSize::Word, &mut sink)
                .unwrap(),
            0
        );
    }

    #[test]
    fn elapsed_alarm_asserts_irq_until_cleared() {
        let mut rtc = Rtc::new(BASE);
        let alarm = rtc.now_ns().saturating_sub(1);
        write_alarm(&mut rtc, alarm);

        assert_eq!(rtc.irq_line(), Some(RTC_IRQ));
        let mut sink = rremu_bus::NullSink;
        assert_eq!(
            rtc.read(BASE + ALARM_STATUS, AccessSize::Word, &mut sink)
                .unwrap(),
            1
        );

        rtc.write(BASE + CLEAR_INTERRUPT, AccessSize::Word, 1, &mut sink)
            .unwrap();
        assert_eq!(rtc.irq_line(), None);
        assert_eq!(
            rtc.read(BASE + ALARM_STATUS, AccessSize::Word, &mut sink)
                .unwrap(),
            0
        );
        rtc.tick(CHECK_INTERVAL * 2);
        assert_eq!(rtc.irq_line(), None);
    }

    #[test]
    fn future_alarm_stays_quiet_until_elapsed() {
        let mut rtc = Rtc::new(BASE);
        let alarm = rtc.now_ns().saturating_add(10_000_000_000);
        write_alarm(&mut rtc, alarm);

        assert_eq!(rtc.irq_line(), None);
        let mut sink = rremu_bus::NullSink;
        assert_eq!(
            rtc.read(BASE + ALARM_STATUS, AccessSize::Word, &mut sink)
                .unwrap(),
            1
        );
    }
}
