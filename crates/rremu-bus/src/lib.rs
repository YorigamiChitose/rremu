//! rremu-bus：总线抽象 —— BusDevice/Bus trait、地址路由、模拟事件。

pub mod access;
pub mod device;
pub mod event;
pub mod router;

pub use access::{AccessSize, BusError, MemKind};
pub use device::{BusDevice, FrameSource, InputSink, Resetable};
pub use event::{EmuEvent, EventSink, NullSink};
pub use router::{Bus, BusRouter};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    /// 测试设备：回显自身存储。
    struct MockRam {
        base: u32,
        mem: Vec<u8>,
        name: String,
    }

    impl MockRam {
        fn new(base: u32, size: usize, name: &str) -> Self {
            Self { base, mem: vec![0; size], name: name.to_owned() }
        }
        fn offset(&self, addr: u32) -> usize {
            (addr - self.base) as usize
        }
    }

    impl BusDevice for MockRam {
        fn read(&mut self, addr: u32, size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
            let o = self.offset(addr);
            let mut v: u32 = 0;
            for i in 0..size.bytes() {
                v |= u32::from(self.mem[o + i as usize]) << (8 * i);
            }
            Ok(v)
        }
        fn write(&mut self, addr: u32, size: AccessSize, data: u32, _sink: &mut dyn EventSink) -> Result<(), BusError> {
            let o = self.offset(addr);
            for i in 0..size.bytes() {
                self.mem[o + i as usize] = (data >> (8 * i)) as u8;
            }
            Ok(())
        }
        fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
            // 与 read 等价（RAM 无副作用）
            let mut sink = NullSink;
            self.read(addr, size, &mut sink)
        }
        fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError> {
            let o = self.offset(addr);
            self.mem[o..o + data.len()].copy_from_slice(data);
            Ok(())
        }
        fn device_name(&self) -> &str {
            &self.name
        }
    }

    #[derive(Default)]
    struct Collector(Vec<EmuEvent>);
    impl EventSink for Collector {
        fn emit(&mut self, ev: EmuEvent) {
            self.0.push(ev);
        }
    }

    #[test]
    fn route_read_write() {
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_1000, Box::new(MockRam::new(0x8000_0000, 0x1000, "ram"))).unwrap();
        let mut sink = Collector::default();
        bus.data_write(0x8000_0100, AccessSize::Word, 0xdead_beef, &mut sink).unwrap();
        let v = bus.data_read(0x8000_0100, AccessSize::Word, &mut sink).unwrap();
        assert_eq!(v, 0xdead_beef);
        // 子宽度写入低位
        bus.data_write(0x8000_0100, AccessSize::Byte, 0x12, &mut sink).unwrap();
        assert_eq!(bus.peek(0x8000_0100, AccessSize::Word).unwrap(), 0xdead_bef2 & 0xffff_ff00 | 0x12);
    }

    #[test]
    fn unmapped() {
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_1000, Box::new(MockRam::new(0x8000_0000, 0x1000, "ram"))).unwrap();
        let mut sink = Collector::default();
        assert_eq!(bus.data_read(0x9000_0000, AccessSize::Word, &mut sink), Err(BusError::Unmapped { addr: 0x9000_0000 }));
        assert_eq!(bus.data_write(0x1000_0000, AccessSize::Byte, 1, &mut sink), Err(BusError::Unmapped { addr: 0x1000_0000 }));
        assert!(bus.peek(0x0, AccessSize::Word).is_err());
    }

    #[test]
    fn access_crossing_region_end_returns_unmapped() {
        // RAM 大小非 4 的倍数时，跨末尾的 Word 访问应返回 Unmapped 而非 panic
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_000A, Box::new(MockRam::new(0x8000_0000, 10, "ram"))).unwrap();
        let mut sink = Collector::default();
        // Word 读 0x8000_0008 → 8..12 跨出 0x8000_000A
        assert_eq!(
            bus.data_read(0x8000_0008, AccessSize::Word, &mut sink),
            Err(BusError::Unmapped { addr: 0x8000_000C })
        );
        assert_eq!(
            bus.data_write(0x8000_0008, AccessSize::Word, 0, &mut sink),
            Err(BusError::Unmapped { addr: 0x8000_000C })
        );
        // 区间内访问仍正常
        bus.data_write(0x8000_0000, AccessSize::Word, 0xDEAD_BEEF, &mut sink).unwrap();
        assert_eq!(bus.peek(0x8000_0000, AccessSize::Word).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn misaligned() {
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_1000, Box::new(MockRam::new(0x8000_0000, 0x1000, "ram"))).unwrap();
        let mut sink = Collector::default();
        assert_eq!(
            bus.data_read(0x8000_0002, AccessSize::Word, &mut sink),
            Err(BusError::Misaligned { addr: 0x8000_0002, size: AccessSize::Word })
        );
        assert_eq!(
            bus.data_write(0x8000_0001, AccessSize::Half, 0, &mut sink),
            Err(BusError::Misaligned { addr: 0x8000_0001, size: AccessSize::Half })
        );
        // 字节访问任意对齐皆可
        bus.data_write(0x8000_0003, AccessSize::Byte, 0x55, &mut sink).unwrap();
    }

    #[test]
    fn inst_read_word_single_event() {
        // 取指原语：一次对齐 Word 读，只发一条 IF W，不做任何跨字处理
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_1000, Box::new(MockRam::new(0x8000_0000, 0x1000, "ram"))).unwrap();
        bus.set_cycle(99);
        bus.write_bytes(0x8000_0000, &[0x13, 0x00, 0x00, 0x00]).unwrap(); // addi x0, x0, 0 (nop)
        let mut sink = Collector::default();
        let v = bus.inst_read(0x8000_0000, AccessSize::Word, &mut sink).unwrap();
        assert_eq!(v, 0x0000_0013);
        assert_eq!(sink.0.len(), 1);
        match sink.0[0] {
            EmuEvent::MemAccess { cycle, kind, size, addr, value, ref device } => {
                assert_eq!((cycle, kind, size, addr, value), (99, MemKind::Inst, AccessSize::Word, 0x8000_0000, 0x0000_0013));
                assert_eq!(device, "ram");
            }
            ref ev => panic!("expected MemAccess, got {ev:?}"),
        }
    }

    #[test]
    fn inst_read_half_single_event() {
        // 取指原语：2 字节对齐 Half 读只发一条 IF H（跨 XLEN 拆分由 CPU 连续调两次，
        // 总线本身不拼合）。内存小端：@...02 = 78 56，@...04 = 34 12。
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_1000, Box::new(MockRam::new(0x8000_0000, 0x1000, "ram"))).unwrap();
        bus.set_cycle(7);
        bus.write_bytes(0x8000_0000, &[0xAA, 0xBB, 0x78, 0x56, 0x34, 0x12, 0xCC, 0xDD]).unwrap();
        let mut sink = Collector::default();
        let lo = bus.inst_read(0x8000_0002, AccessSize::Half, &mut sink).unwrap();
        let hi = bus.inst_read(0x8000_0004, AccessSize::Half, &mut sink).unwrap();
        assert_eq!((lo, hi), (0x5678, 0x1234));
        assert_eq!(sink.0.len(), 2, "two primitive IF H calls emit two events");
        match sink.0[0] {
            EmuEvent::MemAccess { cycle, kind, size, addr, value, ref device } => {
                assert_eq!((cycle, kind, size, addr, value), (7, MemKind::Inst, AccessSize::Half, 0x8000_0002, 0x5678));
                assert_eq!(device, "ram");
            }
            ref ev => panic!("expected MemAccess, got {ev:?}"),
        }
        match sink.0[1] {
            EmuEvent::MemAccess { cycle, kind, size, addr, value, ref device } => {
                assert_eq!((cycle, kind, size, addr, value), (7, MemKind::Inst, AccessSize::Half, 0x8000_0004, 0x1234));
                assert_eq!(device, "ram");
            }
            ref ev => panic!("expected MemAccess, got {ev:?}"),
        }
    }

    #[test]
    fn inst_read_enforces_alignment_per_size() {
        // 原语严格按传入 size 校验对齐，不替调用方拆字：
        // Half @奇数 → Misaligned{Half}；Word @2字节对齐 → Misaligned{Word}（拼合是 CPU 的事）。
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_1000, Box::new(MockRam::new(0x8000_0000, 0x1000, "ram"))).unwrap();
        let mut sink = Collector::default();
        assert_eq!(
            bus.inst_read(0x8000_0001, AccessSize::Half, &mut sink),
            Err(BusError::Misaligned { addr: 0x8000_0001, size: AccessSize::Half })
        );
        assert_eq!(
            bus.inst_read(0x8000_0002, AccessSize::Word, &mut sink),
            Err(BusError::Misaligned { addr: 0x8000_0002, size: AccessSize::Word })
        );
    }

    #[test]
    fn overlap_rejected() {
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_1000, Box::new(MockRam::new(0x8000_0000, 0x1000, "ram"))).unwrap();
        // 完全重叠
        assert!(bus.map(0x8000_0000..0x8000_0100, Box::new(MockRam::new(0x8000_0000, 0x100, "a"))).is_err());
        // 尾部相交
        assert!(bus.map(0x8000_0f00..0x8000_1100, Box::new(MockRam::new(0x8000_0f00, 0x200, "b"))).is_err());
        // 相邻不重叠（合法）
        bus.map(0x8000_1000..0x8000_2000, Box::new(MockRam::new(0x8000_1000, 0x1000, "c"))).unwrap();
    }

    #[test]
    fn event_sink_collects_mem_access() {
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_1000, Box::new(MockRam::new(0x8000_0000, 0x1000, "ram"))).unwrap();
        bus.set_cycle(42);
        let mut sink = Collector::default();
        bus.data_write(0x8000_0000, AccessSize::Word, 7, &mut sink).unwrap();
        bus.data_read(0x8000_0000, AccessSize::Half, &mut sink).unwrap();
        bus.inst_read(0x8000_0000, AccessSize::Word, &mut sink).unwrap();
        assert_eq!(sink.0.len(), 3);
        match sink.0[0] {
            EmuEvent::MemAccess { cycle, kind, size, addr, value, ref device } => {
                assert_eq!((cycle, kind, size, addr, value), (42, MemKind::DataWrite, AccessSize::Word, 0x8000_0000, 7));
                assert_eq!(device, "ram");
            }
            ref ev => panic!("expected MemAccess, got {ev:?}"),
        }
        match sink.0[2] {
            EmuEvent::MemAccess { kind, size, .. } => {
                assert_eq!(kind, MemKind::Inst);
                assert_eq!(size, AccessSize::Word);
            }
            ref ev => panic!("expected MemAccess, got {ev:?}"),
        }
        // peek 不产生事件
        bus.peek(0x8000_0000, AccessSize::Word).unwrap();
        assert_eq!(sink.0.len(), 3);
    }

    /// 测试设备：可配置返回一个中断 cause，并记录 tick 收到的 cycle。
    struct MockIrq {
        causes: Vec<u32>,
        ticked_cycle: std::rc::Rc<std::cell::Cell<u64>>,
    }

    impl MockIrq {
        fn new(cause: u32) -> Self {
            Self { causes: vec![cause], ticked_cycle: std::rc::Rc::new(std::cell::Cell::new(0)) }
        }
        fn none() -> Self {
            Self { causes: Vec::new(), ticked_cycle: std::rc::Rc::new(std::cell::Cell::new(0)) }
        }
    }

    impl BusDevice for MockIrq {
        fn read(&mut self, _addr: u32, _size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
            Ok(0)
        }
        fn write(&mut self, _addr: u32, _size: AccessSize, _data: u32, _sink: &mut dyn EventSink) -> Result<(), BusError> {
            Ok(())
        }
        fn peek_read(&mut self, _addr: u32, _size: AccessSize) -> Result<u32, BusError> {
            Ok(0)
        }
        fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
            Ok(())
        }
        fn device_name(&self) -> &str {
            "mock-irq"
        }
        fn check_interrupt(&self) -> Vec<u32> {
            self.causes.clone()
        }
        fn tick(&mut self, cycle: u64) {
            self.ticked_cycle.set(cycle);
        }
    }

    #[test]
    fn pending_interrupts_collects_all_causes() {
        // 设备侧不裁剪：所有 pending cause 原样返回，CPU 侧按 spec 序裁决
        let mut bus = BusRouter::new();
        // MSI=0x8000_0003, MTI=0x8000_0007
        bus.map(0x1000_0000..0x1000_0010, Box::new(MockIrq::new(0x8000_0003))).unwrap();
        bus.map(0x2000_0000..0x2000_0010, Box::new(MockIrq::new(0x8000_0007))).unwrap();
        let causes = bus.pending_interrupts();
        assert!(causes.contains(&0x8000_0003));
        assert!(causes.contains(&0x8000_0007));
    }

    #[test]
    fn pending_interrupts_empty_when_no_device_has_irq() {
        let mut bus = BusRouter::new();
        bus.map(0x1000_0000..0x1000_0010, Box::new(MockIrq::none())).unwrap();
        bus.map(0x2000_0000..0x2000_0010, Box::new(MockRam::new(0x2000_0000, 0x10, "ram"))).unwrap();
        assert!(bus.pending_interrupts().is_empty());
    }

    #[test]
    fn set_cycle_dispatches_tick_to_all_devices() {
        let mut bus = BusRouter::new();
        let dev1 = MockIrq::new(0x8000_0007);
        let dev2 = MockIrq::new(0x8000_0003);
        let tick1 = dev1.ticked_cycle.clone();
        let tick2 = dev2.ticked_cycle.clone();
        bus.map(0x1000_0000..0x1000_0010, Box::new(dev1)).unwrap();
        bus.map(0x2000_0000..0x2000_0010, Box::new(dev2)).unwrap();
        bus.set_cycle(42);
        // tick 应被 dispatch 到所有设备
        assert_eq!(tick1.get(), 42);
        assert_eq!(tick2.get(), 42);
    }
}
