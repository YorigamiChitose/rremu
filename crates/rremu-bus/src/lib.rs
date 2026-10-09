//! rremu-bus：总线抽象 —— BusDevice/Bus trait、地址路由、IrqLine、模拟事件。

use core::any::Any;
use core::fmt;
use core::ops::Range;
use thiserror::Error;

/// 访问宽度。
///
/// RV64 扩展点：未来新增 `DoubleWord`（8 字节），届时总线返回值/事件 value
/// 需由 u32 扩到 u64，TUI value 增加 16 位 hex 档位（见 PLAN §5.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessSize {
    Byte,
    Half,
    Word,
}

impl AccessSize {
    /// 字节数。
    #[inline]
    pub fn bytes(self) -> u32 {
        match self {
            AccessSize::Byte => 1,
            AccessSize::Half => 2,
            AccessSize::Word => 4,
        }
    }
    /// 对齐检查所需掩码。
    #[inline]
    pub fn align_mask(self) -> u32 {
        self.bytes() - 1
    }
}

impl fmt::Display for AccessSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessSize::Byte => write!(f, "b"),
            AccessSize::Half => write!(f, "h"),
            AccessSize::Word => write!(f, "w"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BusError {
    #[error("unmapped address {addr:#x}")]
    Unmapped { addr: u32 },
    #[error("misaligned {size} access at {addr:#x}")]
    Misaligned { addr: u32, size: AccessSize },
    #[error("device error at {addr:#x}: {msg}")]
    Device { addr: u32, msg: String },
    #[error("peek unsupported at {addr:#x}")]
    PeekUnsupported { addr: u32 },
    /// 注册地址区间与既有设备重叠（builder 阶段）。
    #[error("device range {start:#x}..{end:#x} overlaps existing mapping")]
    Overlap { start: u32, end: u32 },
}

/// 设备侧事件类型（存储访问的类别）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemKind {
    /// 指令取（instruction fetch）。
    Inst,
    /// 数据读（load）。
    DataRead,
    /// 数据写（store）。
    DataWrite,
}

impl fmt::Display for MemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemKind::Inst => write!(f, "IF"),
            MemKind::DataRead => write!(f, "DR"),
            MemKind::DataWrite => write!(f, "DW"),
        }
    }
}

/// 模拟器事件：设备行为与系统状态变化统一经由事件流上报。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmuEvent {
    MemAccess { cycle: u64, kind: MemKind, size: AccessSize, addr: u32, value: u32, device: String },
    UartOutput { byte: u8 },
    UartInput { byte: u8 },
    Trap { cause: u32, pc: u32 },
    Halt { code: i32 },
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

/// 总线设备 trait。地址为**绝对地址**，设备内部自行换算偏移。
pub trait BusDevice {
    /// 有副作用读（如弹 UART FIFO）。
    ///
    /// # Arguments
    /// * `sink` - 设备可在此发出自身事件（如 UartOutput）
    fn read(&mut self, addr: u32, size: AccessSize, sink: &mut dyn EventSink) -> Result<u32, BusError>;
    /// 写（可有副作用）。
    fn write(&mut self, addr: u32, size: AccessSize, data: u32, sink: &mut dyn EventSink) -> Result<(), BusError>;
    /// 无副作用读（peek_read，D12）。**强制每个设备显式实现**，不得有默认实现。
    /// 对监视点/表达式求值/x 命令开放，不得消耗设备状态。
    fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError>;
    /// 批量写入（镜像加载用，无事件上报）。不支持时返回 `Device` 错误。
    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError>;
    fn device_name(&self) -> &str;
    /// 供 machine 层安全下取具体设备（如 uart 注入输入、ram 复位）。
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// 检查设备是否有 pending 中断（返回中断 cause 码，含 bit31）。
    /// 默认无中断；CLINT 等中断源设备覆写此方法。
    fn check_interrupt(&self) -> Option<u32> { None }

    /// 每步周期同步：设备按 cycle 更新内部时序状态（如 CLINT mtime）。
    /// 默认空操作。由 `BusRouter::set_cycle` 遍历调用。
    fn tick(&mut self, _cycle: u64) {}
}

/// CPU/前端编程面对的总线接口（依赖倒置：cpu 只认此 trait）。
///
/// 对外只暴露三个语义方法：`inst_read`（取指）、`data_read`（load）、
/// `data_write`（store），分别对应 `MemKind::Inst/DataRead/DataWrite`，
/// 使访存事件能直接区分指令取与数据访问。原 `fetch/read/write` 由
/// `BusRouter` 内部私有方法承担（见 `route_read`/`route_write`）。
pub trait Bus {
    /// 取指读原语（事件 kind = `MemKind::Inst`）：**一次**对齐物理读，宽度严格由
    /// 调用方 `size` 决定，发一条对应位宽的 `IF` 事件（B/H/W）。
    ///
    /// 本方法只做"路由 + 按 `size` 对齐校验 + 设备读 + 发事件"，**不含任何取指
    /// 策略**：不判定指令长度、不处理跨 XLEN、不做拼合。跨 XLEN（`pc&3==2`）
    /// 时如何拆成两次对齐 Half 读并拼合，由 CPU 取指单元（IFU）决定并连续调用
    /// 本原语两次完成。这样总线保持与 ISA 解耦的纯净传输层，且 RV64 扩展时
    /// 拆分策略随 XLEN 留在 CPU 侧（见 PLAN §5.3/§5.4）。
    ///
    /// 对齐违例返 `BusError::Misaligned { addr, size }`，CPU 映射为相应 trap。
    fn inst_read(&mut self, addr: u32, size: AccessSize, sink: &mut dyn EventSink) -> Result<u32, BusError>;
    /// 数据读（事件 kind = `MemKind::DataRead`）。
    fn data_read(&mut self, addr: u32, size: AccessSize, sink: &mut dyn EventSink) -> Result<u32, BusError>;
    /// 数据写（事件 kind = `MemKind::DataWrite`；命中 watch 且 bit0==1 时发 `EmuEvent::Halt`）。
    fn data_write(&mut self, addr: u32, size: AccessSize, data: u32, sink: &mut dyn EventSink) -> Result<(), BusError>;
    /// 无副作用、无事件读（调试 peek）。
    fn peek(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError>;
    /// 批量写入（镜像加载，无事件）。
    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError>;
    /// 定位地址所在设备（machine 层用）。
    fn device_mut(&mut self, addr: u32) -> Option<&mut dyn BusDevice>;
    /// 设置 tohost 监视地址（None 取消）。
    fn set_watch(&mut self, addr: Option<u32>);
    /// CPU 每步开始前同步当前 cycle，供 MemAccess 事件携带，
    /// 并下发 tick 到所有设备（CLINT 等用此更新内部时序状态）。
    fn set_cycle(&mut self, cycle: u64);
    /// 聚合所有设备的 pending 中断，返回最高优先级 cause（cause 码降序：MEI>MTI>MSI）。
    fn pending_interrupt(&mut self) -> Option<u32>;
}

struct Region {
    range: Range<u32>,
    device: Box<dyn BusDevice>,
}

impl Region {
    fn contains(&self, addr: u32) -> bool {
        self.range.contains(&addr)
    }
}

/// 路由实现：按注册顺序线性查找（设备数量少，单周期模型足够）。
pub struct BusRouter {
    regions: Vec<Region>,
    watch: Option<u32>,
    cycle: u64,
}

impl BusRouter {
    pub fn new() -> Self {
        Self { regions: Vec::new(), watch: None, cycle: 0 }
    }

    /// 注册设备区间（start..end，end 独占）。重叠返回 `BusError::Overlap`。
    pub fn map(&mut self, range: Range<u32>, device: Box<dyn BusDevice>) -> Result<(), BusError> {
        if range.start >= range.end {
            return Err(BusError::Overlap { start: range.start, end: range.end });
        }
        if self.regions.iter().any(|r| range.start < r.range.end && r.range.start < range.end) {
            return Err(BusError::Overlap { start: range.start, end: range.end });
        }
        self.regions.push(Region { range, device });
        Ok(())
    }

    fn check_align(addr: u32, size: AccessSize) -> Result<(), BusError> {
        if addr & size.align_mask() != 0 {
            return Err(BusError::Misaligned { addr, size });
        }
        Ok(())
    }

    /// 路由 + 设备读 + 发 `MemAccess{kind}` 事件（公开方法 inst_read/data_read 共用）。
    fn route_read(
        &mut self,
        addr: u32,
        size: AccessSize,
        kind: MemKind,
        sink: &mut dyn EventSink,
    ) -> Result<u32, BusError> {
        Self::check_align(addr, size)?;
        let cycle = self.cycle;
        let idx = self
            .regions
            .iter()
            .position(|r| r.contains(addr))
            .ok_or(BusError::Unmapped { addr })?;
        let region = &mut self.regions[idx];
        let value = region.device.read(addr, size, sink)?;
        sink.emit(EmuEvent::MemAccess {
            cycle,
            kind,
            size,
            addr,
            value,
            device: region.device.device_name().to_owned(),
        });
        Ok(value)
    }

    /// 路由 + 设备写 + 发 `MemAccess{kind}` 事件 + tohost 监视（data_write 用）。
    fn route_write(
        &mut self,
        addr: u32,
        size: AccessSize,
        data: u32,
        kind: MemKind,
        sink: &mut dyn EventSink,
    ) -> Result<(), BusError> {
        Self::check_align(addr, size)?;
        let cycle = self.cycle;
        let idx = self
            .regions
            .iter()
            .position(|r| r.contains(addr))
            .ok_or(BusError::Unmapped { addr })?;
        let region = &mut self.regions[idx];
        region.device.write(addr, size, data, sink)?;
        sink.emit(EmuEvent::MemAccess {
            cycle,
            kind,
            size,
            addr,
            value: data,
            device: region.device.device_name().to_owned(),
        });
        // tohost 监视：写入值 bit0==1 表示进程退出（riscv-tests htif 约定）
        if self.watch == Some(addr) && data & 1 == 1 {
            sink.emit(EmuEvent::Halt { code: (data >> 1) as i32 });
        }
        Ok(())
    }
}

impl Default for BusRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for BusRouter {
    fn inst_read(&mut self, addr: u32, size: AccessSize, sink: &mut dyn EventSink) -> Result<u32, BusError> {
        // 单次对齐物理读原语：跨 XLEN 拆分由 CPU(IFU) 决定，总线不感知指令长度。
        self.route_read(addr, size, MemKind::Inst, sink)
    }

    fn data_read(&mut self, addr: u32, size: AccessSize, sink: &mut dyn EventSink) -> Result<u32, BusError> {
        self.route_read(addr, size, MemKind::DataRead, sink)
    }

    fn data_write(&mut self, addr: u32, size: AccessSize, data: u32, sink: &mut dyn EventSink) -> Result<(), BusError> {
        self.route_write(addr, size, data, MemKind::DataWrite, sink)
    }

    fn peek(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
        Self::check_align(addr, size)?;
        let idx = self
            .regions
            .iter()
            .position(|r| r.contains(addr))
            .ok_or(BusError::PeekUnsupported { addr })?;
        self.regions[idx].device.peek_read(addr, size)
    }

    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError> {
        let idx = self
            .regions
            .iter()
            .position(|r| r.contains(addr))
            .ok_or(BusError::Unmapped { addr })?;
        let region = &mut self.regions[idx];
        // 区间截断检查：不允许跨越设备边界写入
        let off = addr - region.range.start;
        let avail = region.range.end - region.range.start - off;
        if u64::from(data.len() as u32) > u64::from(avail) {
            return Err(BusError::Unmapped { addr: region.range.end });
        }
        region.device.write_bytes(addr, data)
    }

    fn device_mut(&mut self, addr: u32) -> Option<&mut dyn BusDevice> {
        let idx = self.regions.iter().position(|r| r.contains(addr))?;
        Some(self.regions[idx].device.as_mut())
    }

    fn set_watch(&mut self, addr: Option<u32>) {
        self.watch = addr;
    }

    fn set_cycle(&mut self, cycle: u64) {
        self.cycle = cycle;
        for r in &mut self.regions {
            r.device.tick(cycle);
        }
    }

    fn pending_interrupt(&mut self) -> Option<u32> {
        self.regions
            .iter()
            .filter_map(|r| r.device.check_interrupt())
            .max()
    }
}

/// 中断线（M3 CLINT 使用；M1 先提供基础设施）。
#[derive(Debug, Default)]
pub struct IrqLine {
    pending: bool,
}

impl IrqLine {
    pub fn new() -> Self {
        Self { pending: false }
    }
    pub fn raise(&mut self) {
        self.pending = true;
    }
    pub fn clear(&mut self) {
        self.pending = false;
    }
    pub fn is_pending(&self) -> bool {
        self.pending
    }
}

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
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
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

    #[test]
    fn watch_halt_on_tohost_write() {
        let mut bus = BusRouter::new();
        bus.map(0x8000_0000..0x8000_1000, Box::new(MockRam::new(0x8000_0000, 0x1000, "ram"))).unwrap();
        bus.set_watch(Some(0x8000_0100));
        let mut sink = Collector::default();
        // bit0=0：非退出写
        bus.data_write(0x8000_0100, AccessSize::Word, 2, &mut sink).unwrap();
        assert!(!sink.0.iter().any(|e| matches!(e, EmuEvent::Halt { .. })));
        // bit0=1：退出，code = 3 >> 1 = 1
        bus.data_write(0x8000_0100, AccessSize::Word, 3, &mut sink).unwrap();
        assert!(sink.0.iter().any(|e| matches!(e, EmuEvent::Halt { code: 1 })));
        // watch 地址外的写不触发
        bus.data_write(0x8000_0200, AccessSize::Word, 1, &mut sink).unwrap();
        assert_eq!(sink.0.iter().filter(|e| matches!(e, EmuEvent::Halt { .. })).count(), 1);
    }

    #[test]
    fn irq_line() {
        let mut irq = IrqLine::new();
        assert!(!irq.is_pending());
        irq.raise();
        assert!(irq.is_pending());
        irq.clear();
        assert!(!irq.is_pending());
    }

    /// 测试设备：可配置返回一个中断 cause，并记录 tick 收到的 cycle。
    struct MockIrq {
        cause: Option<u32>,
        ticked_cycle: u64,
    }

    impl MockIrq {
        fn new(cause: u32) -> Self {
            Self { cause: Some(cause), ticked_cycle: 0 }
        }
        fn none() -> Self {
            Self { cause: None, ticked_cycle: 0 }
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
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn check_interrupt(&self) -> Option<u32> {
            self.cause
        }
        fn tick(&mut self, cycle: u64) {
            self.ticked_cycle = cycle;
        }
    }

    #[test]
    fn pending_interrupt_aggregates_max() {
        let mut bus = BusRouter::new();
        // MSI=0x8000_0003 < MTI=0x8000_0007
        bus.map(0x1000_0000..0x1000_0010, Box::new(MockIrq::new(0x8000_0003))).unwrap();
        bus.map(0x2000_0000..0x2000_0010, Box::new(MockIrq::new(0x8000_0007))).unwrap();
        assert_eq!(bus.pending_interrupt(), Some(0x8000_0007));
    }

    #[test]
    fn pending_interrupt_none_when_no_device_has_irq() {
        let mut bus = BusRouter::new();
        bus.map(0x1000_0000..0x1000_0010, Box::new(MockIrq::none())).unwrap();
        bus.map(0x2000_0000..0x2000_0010, Box::new(MockRam::new(0x2000_0000, 0x10, "ram"))).unwrap();
        assert_eq!(bus.pending_interrupt(), None);
    }

    #[test]
    fn set_cycle_dispatches_tick_to_all_devices() {
        let mut bus = BusRouter::new();
        let dev1 = MockIrq::new(0x8000_0007);
        let dev2 = MockIrq::new(0x8000_0003);
        bus.map(0x1000_0000..0x1000_0010, Box::new(dev1)).unwrap();
        bus.map(0x2000_0000..0x2000_0010, Box::new(dev2)).unwrap();
        bus.set_cycle(42);
        // tick 应被 dispatch 到所有设备
        // 用 pending_interrupt 间接验证 tick 生效（MockIrq tick 只记录 cycle）
        // 直接 downcast 检查 ticked_cycle
        let d1 = bus.device_mut(0x1000_0000).unwrap().as_any_mut().downcast_ref::<MockIrq>().unwrap();
        assert_eq!(d1.ticked_cycle, 42);
        let d2 = bus.device_mut(0x2000_0000).unwrap().as_any_mut().downcast_ref::<MockIrq>().unwrap();
        assert_eq!(d2.ticked_cycle, 42);
    }
}
