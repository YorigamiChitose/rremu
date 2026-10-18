//! 数据访存与原子操作。

use rremu_bus::{AccessSize, Bus, EventSink};
use rremu_isa::cause;

use crate::AccessType;
use super::{Cpu, TrapErr};

impl Cpu {
    #[inline]
    pub(crate) fn load<B: Bus>(
        &mut self,
        bus: &mut B,
        sink: &mut dyn EventSink,
        addr: u32,
        size: AccessSize,
    ) -> Result<u32, TrapErr> {
        let pa = self.translate(bus, sink, addr, AccessType::Load)?;
        if !self.pmp_check(pa, AccessType::Load) {
            return Err(TrapErr { cause: cause::LOAD_ACCESS_FAULT, mtval: addr });
        }
        bus.data_read(pa, size, sink).map_err(|e| TrapErr {
            cause: Self::cause_of(&e, cause::LOAD_MISALIGNED, cause::LOAD_ACCESS_FAULT),
            mtval: addr,
        })
    }

    #[inline]
    pub(crate) fn store<B: Bus>(
        &mut self,
        bus: &mut B,
        sink: &mut dyn EventSink,
        addr: u32,
        size: AccessSize,
        data: u32,
    ) -> Result<(), TrapErr> {
        self.reservation = None; // 任何 store 都使 lr.w 保留失效
        let pa = self.translate(bus, sink, addr, AccessType::Store)?;
        if !self.pmp_check(pa, AccessType::Store) {
            return Err(TrapErr { cause: cause::STORE_ACCESS_FAULT, mtval: addr });
        }
        bus.data_write(pa, size, data, sink).map_err(|e| TrapErr {
            cause: Self::cause_of(&e, cause::STORE_MISALIGNED, cause::STORE_ACCESS_FAULT),
            mtval: addr,
        })
    }

    /// AMO 原子读-改-写：读旧值 → 计算新值 → 写回 → rd = 旧值。
    /// 单核单线程下无需总线锁，直接 load + store 即可保持原子性。
    pub(crate) fn amo<B: Bus, E: EventSink, F: Fn(u32, u32) -> u32>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        rd: u8,
        rs1: u8,
        rs2: u8,
        f: F,
    ) -> Result<(), TrapErr> {
        let addr = self.x[usize::from(rs1)];
        let val = self.x[usize::from(rs2)];
        let old = self.load(bus, sink, addr, AccessSize::Word)?;
        let new = f(old, val);
        self.store(bus, sink, addr, AccessSize::Word, new)?;
        if rd != 0 {
            self.x[usize::from(rd)] = old;
        }
        Ok(())
    }}
