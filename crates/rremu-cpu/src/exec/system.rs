//! System instruction helpers: SRET/MRET/ECALL/EBREAK.

use rremu_isa::{Priv, cause, csr as isa_csr};

use super::{Cpu, TrapErr};

impl Cpu {
    pub(crate) fn exec_sret(&mut self) -> u32 {
        let sstatus = self.csr.mstatus;
        let spp = (sstatus >> isa_csr::MSTATUS_SPP.trailing_zeros()) & 1;
        let spie = (sstatus >> isa_csr::MSTATUS_SPIE.trailing_zeros()) & 1;
        self.csr.mstatus &= !(isa_csr::MSTATUS_SPP
            | isa_csr::MSTATUS_SIE
            | isa_csr::MSTATUS_SPIE
            | isa_csr::MSTATUS_MPRV);
        self.csr.mstatus |= (spie << isa_csr::MSTATUS_SIE.trailing_zeros())
            | (1 << isa_csr::MSTATUS_SPIE.trailing_zeros());
        self.priv_ = if spp == 1 {
            Priv::Supervisor
        } else {
            Priv::User
        };
        self.csr.sepc
    }

    pub(crate) fn exec_mret(&mut self) -> u32 {
        let mstatus = self.csr.mstatus;
        let mpp = (mstatus >> isa_csr::MSTATUS_MPP_SHIFT) & 0b11;
        let mpie = (mstatus >> isa_csr::MSTATUS_MPIE.trailing_zeros()) & 1;
        self.update_mstatus(0, mpie, 1);
        if mpp != 0b11 {
            self.csr.mstatus &= !isa_csr::MSTATUS_MPRV;
        }
        self.priv_ = match mpp {
            0b11 => Priv::Machine,
            0b01 => Priv::Supervisor,
            _ => Priv::User,
        };
        self.csr.mepc
    }

    pub(crate) fn ecall_trap(&self) -> TrapErr {
        let c = match self.priv_ {
            Priv::Machine => cause::ECALL_FROM_M,
            Priv::Supervisor => cause::ECALL_FROM_S,
            Priv::User => cause::ECALL_FROM_U,
        };
        TrapErr { cause: c, mtval: 0 }
    }

    pub(crate) fn breakpoint_trap() -> TrapErr {
        TrapErr {
            cause: cause::BREAKPOINT,
            mtval: 0,
        }
    }
}
