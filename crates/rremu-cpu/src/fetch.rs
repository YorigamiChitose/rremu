//! 取指单元（IFU）与跨页取指规则。

use rremu_bus::{AccessSize, Bus, EventSink};
use rremu_decode::instr_len;
use rremu_isa::cause;

use super::{AccessType, Cpu};

impl Cpu {
    /// 取指单元(IFU)：取回 `pc` 处的指令字，高位在需要时补入。
    ///
    /// 跨页策略：4 字节对齐地址的 Word 读不会跨 4KiB 页；`pc & 3 == 2` 时
    /// 先读低半字判断长度，只有 32 位指令才读取 `pc+2`。这样页尾的 16 位
    /// 指令不会误触发下一页的 page fault。
    ///
    /// 调用前 `pc` 已保证偶数；返回 Err 为取指异常 cause。
    pub(crate) fn fetch_word<B: Bus, E: EventSink>(
        &self,
        bus: &mut B,
        pc: u32,
        sink: &mut E,
    ) -> Result<u32, u32> {
        // 取指总线错误 → 异常 cause：对齐违例 INSTR_MISALIGNED，其余 INSTR_ACCESS_FAULT
        fn map(e: rremu_bus::BusError) -> u32 {
            Cpu::cause_of(&e, cause::INSTR_MISALIGNED, cause::INSTR_ACCESS_FAULT)
        }
        let pa = match self.translate(bus, sink, pc, AccessType::Fetch) {
            Ok(pa) => pa,
            Err(t) => return Err(t.cause),
        };
        if pc & 3 == 0 {
            if !self.pmp_check(pa, AccessType::Fetch) {
                return Err(cause::INSTR_ACCESS_FAULT);
            }
            return bus.inst_read(pa, AccessSize::Word, sink).map_err(map);
        }

        // pc & 3 == 2（仅 RVC 允许 2 字节对齐）：先读低半字。
        if !self.pmp_check(pa, AccessType::Fetch) {
            return Err(cause::INSTR_ACCESS_FAULT);
        }
        let lo = bus.inst_read(pa, AccessSize::Half, sink).map_err(map)?;
        if instr_len(lo as u16) == 2 {
            return Ok(lo);
        }
        // 32 位指令：仅 pc 位于页尾 0xffe 时需要翻译下一页。
        let hi_pa = if pc & 0xfff == 0xffe {
            match self.translate(bus, sink, pc.wrapping_add(2), AccessType::Fetch) {
                Ok(pa) => pa,
                Err(t) => return Err(t.cause),
            }
        } else {
            pa.wrapping_add(2)
        };
        if !self.pmp_check(hi_pa, AccessType::Fetch) {
            return Err(cause::INSTR_ACCESS_FAULT);
        }
        let hi = bus.inst_read(hi_pa, AccessSize::Half, sink).map_err(map)?;
        Ok(lo | (hi << 16))
    }
}
