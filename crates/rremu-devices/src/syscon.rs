//! QEMU virt syscon/test 设备：poweroff / reboot 写入触发模拟器停机。
//!
//! QEMU virt 的 `sifive,test0` 位于 `0x0010_0000`：
//! - 写 `0x5555` → poweroff
//! - 写 `0x7777` → reboot（本模拟器统一按停机处理）

use rremu_bus::{AccessSize, BusDevice, BusError, EmuEvent, EventSink};

/// 映射尺寸。
pub const SYSCON_SIZE: u32 = 0x1000;
const POWEROFF: u32 = 0x5555;
const REBOOT: u32 = 0x7777;

pub struct Syscon {
    base: u32,
}

impl Syscon {
    pub fn new(base: u32) -> Self {
        Self { base }
    }
}

impl BusDevice for Syscon {
    fn read(&mut self, _addr: u32, _size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
        Ok(0)
    }

    fn write(&mut self, addr: u32, _size: AccessSize, data: u32, sink: &mut dyn EventSink) -> Result<(), BusError> {
        if addr - self.base != 0 {
            return Ok(());
        }
        if data == POWEROFF || data == REBOOT {
            sink.emit(EmuEvent::Halt { code: 0 });
        }
        Ok(())
    }

    fn peek_read(&mut self, _addr: u32, _size: AccessSize) -> Result<u32, BusError> {
        Ok(0)
    }

    fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
        Err(BusError::Device { addr: self.base, msg: "cannot bulk-write to syscon".to_owned() })
    }

    fn device_name(&self) -> &str {
        "syscon"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use rremu_bus::NullSink;

    const BASE: u32 = 0x0010_0000;

    #[test]
    fn poweroff_emits_halt() {
        struct Sink(Vec<EmuEvent>);
        impl EventSink for Sink {
            fn emit(&mut self, ev: EmuEvent) {
                self.0.push(ev);
            }
        }
        let mut syscon = Syscon::new(BASE);
        let mut sink = Sink(Vec::new());
        syscon.write(BASE, AccessSize::Word, POWEROFF, &mut sink).unwrap();
        assert_eq!(sink.0, vec![EmuEvent::Halt { code: 0 }]);
        let mut null = NullSink;
        syscon.write(BASE, AccessSize::Word, 0x1234, &mut null).unwrap();
        assert_eq!(sink.0.len(), 1);
    }
}
