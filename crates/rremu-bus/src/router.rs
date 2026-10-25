//! 总线路由与访问原语实现。

use core::ops::Range;

use crate::access::{AccessSize, BusError, MemKind};
use crate::device::BusDevice;
use crate::event::{EmuEvent, EventSink};

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
    /// 拆分策略随 XLEN 留在 CPU 侧。
    ///
    /// 对齐违例返 `BusError::Misaligned { addr, size }`，CPU 映射为相应 trap。
    fn inst_read(
        &mut self,
        addr: u32,
        size: AccessSize,
        sink: &mut dyn EventSink,
    ) -> Result<u32, BusError>;
    /// 数据读（事件 kind = `MemKind::DataRead`）。
    fn data_read(
        &mut self,
        addr: u32,
        size: AccessSize,
        sink: &mut dyn EventSink,
    ) -> Result<u32, BusError>;
    /// 数据写（事件 kind = `MemKind::DataWrite`）。
    fn data_write(
        &mut self,
        addr: u32,
        size: AccessSize,
        data: u32,
        sink: &mut dyn EventSink,
    ) -> Result<(), BusError>;
    /// 无副作用、无事件读（调试 peek）。
    fn peek(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError>;
    /// 批量写入（镜像加载，无事件）。
    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError>;
    /// 定位地址所在设备（machine 层用）。
    fn device_mut(&mut self, addr: u32) -> Option<&mut dyn BusDevice>;
    /// CPU 每步开始前同步当前 cycle，供 MemAccess 事件携带，
    /// 并下发 tick 到所有设备（CLINT 等用此更新内部时序状态）。
    fn set_cycle(&mut self, cycle: u64);
    /// 聚合所有设备的全部 pending 中断 cause，由 CPU 侧按 spec 序裁决。
    fn service_interrupts(&mut self) -> Vec<u32>;
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
    cycle: u64,
}

impl BusRouter {
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
            cycle: 0,
        }
    }

    /// 注册设备区间（start..end，end 独占）。重叠返回 `BusError::Overlap`。
    pub fn map(&mut self, range: Range<u32>, device: Box<dyn BusDevice>) -> Result<(), BusError> {
        if range.start >= range.end {
            return Err(BusError::Overlap {
                start: range.start,
                end: range.end,
            });
        }
        if self
            .regions
            .iter()
            .any(|r| range.start < r.range.end && r.range.start < range.end)
        {
            return Err(BusError::Overlap {
                start: range.start,
                end: range.end,
            });
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

    /// 定位地址所在区域；未映射统一返 `Unmapped`。
    fn region_index(&self, addr: u32) -> Result<usize, BusError> {
        self.regions
            .iter()
            .position(|r| r.contains(addr))
            .ok_or(BusError::Unmapped { addr })
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
        let idx = self.region_index(addr)?;
        // 访问不得跨出设备区间末尾（否则设备内部可能越界 panic）
        let access_end = addr
            .checked_add(size.bytes())
            .ok_or(BusError::Unmapped { addr })?;
        if access_end > self.regions[idx].range.end {
            return Err(BusError::Unmapped { addr: access_end });
        }
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

    /// 路由 + 设备写 + 发 `MemAccess{kind}` 事件（data_write 用）。
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
        let idx = self.region_index(addr)?;
        // 访问不得跨出设备区间末尾（否则设备内部可能越界 panic）
        let access_end = addr
            .checked_add(size.bytes())
            .ok_or(BusError::Unmapped { addr })?;
        if access_end > self.regions[idx].range.end {
            return Err(BusError::Unmapped { addr: access_end });
        }
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
        Ok(())
    }
}

impl Default for BusRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for BusRouter {
    fn inst_read(
        &mut self,
        addr: u32,
        size: AccessSize,
        sink: &mut dyn EventSink,
    ) -> Result<u32, BusError> {
        // 单次对齐物理读原语：跨 XLEN 拆分由 CPU(IFU) 决定，总线不感知指令长度。
        self.route_read(addr, size, MemKind::Inst, sink)
    }

    fn data_read(
        &mut self,
        addr: u32,
        size: AccessSize,
        sink: &mut dyn EventSink,
    ) -> Result<u32, BusError> {
        self.route_read(addr, size, MemKind::DataRead, sink)
    }

    fn data_write(
        &mut self,
        addr: u32,
        size: AccessSize,
        data: u32,
        sink: &mut dyn EventSink,
    ) -> Result<(), BusError> {
        self.route_write(addr, size, data, MemKind::DataWrite, sink)
    }

    fn peek(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
        Self::check_align(addr, size)?;
        let idx = self.region_index(addr)?;
        self.regions[idx].device.peek_read(addr, size)
    }

    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError> {
        let idx = self.region_index(addr)?;
        let region = &mut self.regions[idx];
        // 区间截断检查：不允许跨越设备边界写入
        let off = addr - region.range.start;
        let avail = region.range.end - region.range.start - off;
        if u64::from(data.len() as u32) > u64::from(avail) {
            return Err(BusError::Unmapped {
                addr: region.range.end,
            });
        }
        region.device.write_bytes(addr, data)
    }

    fn device_mut(&mut self, addr: u32) -> Option<&mut dyn BusDevice> {
        let idx = self.regions.iter().position(|r| r.contains(addr))?;
        Some(self.regions[idx].device.as_mut())
    }

    fn set_cycle(&mut self, cycle: u64) {
        self.cycle = cycle;
        for r in &mut self.regions {
            r.device.tick(cycle);
        }
    }

    fn service_interrupts(&mut self) -> Vec<u32> {
        // 1) 收集各设备断言的 IRQ 源（level）；喂给中断控制器（PLIC）
        let asserted: Vec<u32> = self
            .regions
            .iter()
            .filter_map(|r| r.device.irq_line())
            .collect();
        if !asserted.is_empty() {
            for r in &mut self.regions {
                for &src in &asserted {
                    r.device.irq_assert(src);
                }
            }
        }
        // 2) 聚合各设备 pending 中断 cause
        let mut causes = Vec::new();
        for r in &self.regions {
            causes.extend(r.device.check_interrupt());
        }
        causes
    }
}
