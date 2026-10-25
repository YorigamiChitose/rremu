//! RAM 设备：Vec<u8> 小端存储，全宽度读写，peek 等价 read。

use rremu_bus::{AccessSize, BusDevice, BusError, EventSink, Resetable};

pub struct Ram {
    base: u32,
    mem: Vec<u8>,
}

impl Ram {
    pub fn new(base: u32, size: u32) -> Self {
        Self {
            base,
            mem: vec![0; size as usize],
        }
    }

    /// 复位：清零全部存储（machine reset 用）。
    pub fn reset(&mut self) {
        self.mem.fill(0);
    }

    fn offset(&self, addr: u32) -> usize {
        (addr - self.base) as usize
    }
}

impl Resetable for Ram {
    fn reset(&mut self) {
        Ram::reset(self);
    }
}

impl BusDevice for Ram {
    fn read(
        &mut self,
        addr: u32,
        size: AccessSize,
        _sink: &mut dyn EventSink,
    ) -> Result<u32, BusError> {
        let o = self.offset(addr);
        let mut v: u32 = 0;
        for i in 0..size.bytes() {
            v |= u32::from(self.mem[o + i as usize]) << (8 * i);
        }
        Ok(v)
    }

    fn write(
        &mut self,
        addr: u32,
        size: AccessSize,
        data: u32,
        _sink: &mut dyn EventSink,
    ) -> Result<(), BusError> {
        let o = self.offset(addr);
        for i in 0..size.bytes() {
            self.mem[o + i as usize] = (data >> (8 * i)) as u8;
        }
        Ok(())
    }

    fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
        // RAM 无副作用：peek 等价 read
        let mut sink = rremu_bus::NullSink;
        BusDevice::read(self, addr, size, &mut sink)
    }

    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError> {
        let o = self.offset(addr);
        self.mem[o..o + data.len()].copy_from_slice(data);
        Ok(())
    }

    fn device_name(&self) -> &str {
        "ram"
    }

    fn as_resetable(&mut self) -> Option<&mut dyn Resetable> {
        Some(self)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    fn w(
        addr: u32,
        size: AccessSize,
        data: u32,
    ) -> impl FnOnce(&mut Ram, &mut dyn EventSink) -> Result<(), BusError> {
        move |ram, sink| ram.write(addr, size, data, sink)
    }

    #[test]
    fn width_read_write_little_endian() {
        let mut ram = Ram::new(0x8000_0000, 0x1000);
        let mut sink = rremu_bus::NullSink;
        w(0x8000_0000, AccessSize::Word, 0x1122_3344)(&mut ram, &mut sink).unwrap();
        // 字节视图：小端
        assert_eq!(
            ram.read(0x8000_0000, AccessSize::Byte, &mut sink).unwrap(),
            0x44
        );
        assert_eq!(
            ram.read(0x8000_0001, AccessSize::Byte, &mut sink).unwrap(),
            0x33
        );
        assert_eq!(
            ram.read(0x8000_0000, AccessSize::Half, &mut sink).unwrap(),
            0x3344
        );
        // 子宽度写只影响低位
        w(0x8000_0000, AccessSize::Byte, 0xAA)(&mut ram, &mut sink).unwrap();
        assert_eq!(
            ram.read(0x8000_0000, AccessSize::Word, &mut sink).unwrap(),
            0x1122_33AA
        );
        w(0x8000_0000, AccessSize::Half, 0xBEEF)(&mut ram, &mut sink).unwrap();
        assert_eq!(
            ram.read(0x8000_0000, AccessSize::Word, &mut sink).unwrap(),
            0x1122_BEEF
        );
    }

    #[test]
    fn peek_equivalent_to_read() {
        let mut ram = Ram::new(0x8000_0000, 0x1000);
        let mut sink = rremu_bus::NullSink;
        w(0x8000_0010, AccessSize::Word, 0xdead_beef)(&mut ram, &mut sink).unwrap();
        assert_eq!(
            ram.peek_read(0x8000_0010, AccessSize::Word).unwrap(),
            0xdead_beef
        );
    }

    #[test]
    fn write_bytes_and_reset() {
        let mut ram = Ram::new(0x8000_0000, 0x1000);
        ram.write_bytes(0x8000_0004, b"hello").unwrap();
        let mut sink = rremu_bus::NullSink;
        assert_eq!(
            ram.read(0x8000_0004, AccessSize::Word, &mut sink).unwrap(),
            u32::from_le_bytes(*b"hell")
        );
        ram.reset();
        assert_eq!(ram.peek_read(0x8000_0004, AccessSize::Word).unwrap(), 0);
    }
}
