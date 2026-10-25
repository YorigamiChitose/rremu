//! Trap 入口、委派与 TrapErr。

use rremu_bus::{EmuEvent, EventSink};
use rremu_isa::{Priv, csr};

use super::{Cpu, CpuError, StepOutcome};

/// 执行期内部 trap 信号（bus 错误 → cause 映射已完成）。
#[derive(Debug)]
pub(crate) struct TrapErr {
    pub(crate) cause: u32,
    pub(crate) mtval: u32,
}

impl Cpu {
    /// 陷入流程：mepc ← pc；mcause ← cause；mtval ← mtval；priv ← M；pc ← mtvec（仅 DIRECT）。
    pub(crate) fn trap<E: EventSink>(
        &mut self,
        sink: &mut E,
        cause: u32,
        mtval: u32,
    ) -> Result<StepOutcome, CpuError> {
        sink.emit(EmuEvent::Trap { cause, pc: self.pc });

        // 陷阱委派：非 M 特权下，已委派（medeleg/mideleg）的异常/中断交 S-mode
        let is_interrupt = cause & 0x8000_0000 != 0;
        let code = cause & 0x7fff_ffff;
        let delegated = self.priv_ != Priv::Machine
            && if is_interrupt {
                (self.csr.mideleg >> code) & 1 == 1
            } else {
                (self.csr.medeleg >> code) & 1 == 1
            };
        if delegated {
            return self.trap_supervisor(cause, mtval);
        }

        self.csr.mepc = self.pc;
        self.csr.mcause = cause;
        self.csr.mtval = mtval;

        // 更新 mstatus: MPP = 当前特权级, MPIE = 旧 MIE, MIE = 0
        let mpp = match self.priv_ {
            Priv::Machine => 0b11,
            Priv::Supervisor => 0b01,
            Priv::User => 0b00,
        };
        let old_mie = (self.csr.mstatus >> csr::MSTATUS_MIE.trailing_zeros()) & 1;
        self.update_mstatus(mpp, 0, old_mie);

        self.priv_ = Priv::Machine;
        self.cycle += 1;
        if self.csr.mtvec & 1 == 1 {
            return Err(CpuError::Unimplemented("vectored mtvec"));
        }
        self.pc = self.csr.mtvec & !1;
        Ok(StepOutcome::Trapped { cause })
    }

    /// 陷入到 S-mode：sepc/scause/stval ← 当前；sstatus.SPP/SPIE/SIE；pc ← stvec（仅 DIRECT）。
    pub(crate) fn trap_supervisor(
        &mut self,
        cause: u32,
        mtval: u32,
    ) -> Result<StepOutcome, CpuError> {
        self.csr.sepc = self.pc;
        self.csr.scause = cause;
        self.csr.stval = mtval;
        let spp = u32::from(self.priv_ == Priv::Supervisor);
        let old_sie = (self.csr.mstatus >> csr::MSTATUS_SIE.trailing_zeros()) & 1;
        self.csr.mstatus &= !(csr::MSTATUS_SPP | csr::MSTATUS_SPIE | csr::MSTATUS_SIE);
        self.csr.mstatus |= (spp << csr::MSTATUS_SPP.trailing_zeros())
            | (old_sie << csr::MSTATUS_SPIE.trailing_zeros());
        self.priv_ = Priv::Supervisor;
        self.cycle += 1;
        if self.csr.stvec & 1 == 1 {
            return Err(CpuError::Unimplemented("vectored stvec"));
        }
        self.pc = self.csr.stvec & !1;
        Ok(StepOutcome::Trapped { cause })
    }
}
