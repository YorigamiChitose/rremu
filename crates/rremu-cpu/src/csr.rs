//! CSR 状态、存在性与访问权限。

use rremu_isa::{cause, csr, Priv};

use super::{Cpu, TrapErr};

/// M 状态 CSR（mcycle/minstret 由 cycle/instret 计数器承载）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsrState {
    pub mstatus: u32,
    pub misa: u32,
    pub mie: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mip: u32,
    pub mhartid: u32,
    pub medeleg: u32,
    pub mideleg: u32,
    pub stvec: u32,
    pub sscratch: u32,
    pub sepc: u32,
    pub scause: u32,
    pub stval: u32,
    pub satp: u32,
    pub pmpcfg: [u32; 4],
    pub pmpaddr: [u32; 16],
}

impl CsrState {
    pub(crate) fn new() -> Self {
        Self {
            mstatus: csr::MSTATUS_MPP_M, // 复位进入 M-mode
            // RV32 + I/M/A/C/S/U
            misa: csr::MISA_MXL_32
                | csr::MISA_I
                | csr::MISA_M
                | csr::MISA_A
                | csr::MISA_C
                | csr::MISA_S
                | csr::MISA_U,
            mie: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mip: 0,
            mhartid: 0,
            medeleg: 0,
            mideleg: 0,
            stvec: 0,
            sscratch: 0,
            sepc: 0,
            scause: 0,
            stval: 0,
            satp: 0,
            pmpcfg: [0; 4],
            pmpaddr: [0; 16],
        }
    }
}

impl Cpu {
    /// CSR 视图读取（快照 + CSR 指令共用）。未知 CSR 返回 0。
    /// `sstatus`/`sie`/`sip` 是 `mstatus`/`mie`/`mip` 的 S 位掩码视图。
    pub fn csr_read(&self, addr: u16) -> u32 {
        match addr {
            csr::MSTATUS => self.csr.mstatus,
            csr::MSTATUSH | csr::SSTATUSH => 0,
            csr::MISA => self.csr.misa,
            csr::MIE => self.csr.mie,
            csr::MTVEC => self.csr.mtvec,
            csr::MSCRATCH => self.csr.mscratch,
            csr::MEPC => self.csr.mepc,
            csr::MCAUSE => self.csr.mcause,
            csr::MTVAL => self.csr.mtval,
            csr::MIP => self.csr.mip,
            csr::MEDELEG => self.csr.medeleg,
            csr::MIDELEG => self.csr.mideleg,
            csr::SSTATUS => self.csr.mstatus & csr::SSTATUS_MASK,
            csr::SIE => self.csr.mie & csr::SIE_S_MASK,
            csr::SIP => self.csr.mip & csr::SIE_S_MASK,
            csr::STVEC => self.csr.stvec,
            csr::SSCRATCH => self.csr.sscratch,
            csr::SEPC => self.csr.sepc,
            csr::SCAUSE => self.csr.scause,
            csr::STVAL => self.csr.stval,
            csr::SATP => self.csr.satp,
            csr::MCYCLE | csr::CYCLE | csr::TIME => self.cycle as u32,
            csr::MINSTRET | csr::INSTRET => self.instret as u32,
            csr::TIMEH => (self.cycle >> 32) as u32,
            csr::MHARTID => self.csr.mhartid,
            a if (csr::PMPCFG0..csr::PMPCFG0 + 4).contains(&a) => {
                self.csr.pmpcfg[(a - csr::PMPCFG0) as usize]
            }
            a if (csr::PMPADDR0..csr::PMPADDR0 + 16).contains(&a) => {
                self.csr.pmpaddr[(a - csr::PMPADDR0) as usize]
            }
            _ => 0,
        }
    }

    /// CSR 写入（CSR 指令执行用）。只读/未知 CSR 写入忽略。
    pub(crate) fn csr_write(&mut self, addr: u16, val: u32) {
        match addr {
            csr::MSTATUS => self.csr.mstatus = val & csr::MSTATUS_WRITE_MASK,
            // mstatush/sstatush 的高位当前无状态，忽略写入。
            csr::MSTATUSH | csr::SSTATUSH => {}
            // misa 为只读/WARL：忽略写入。
            csr::MIE => self.csr.mie = val,
            csr::MTVEC => self.csr.mtvec = val,
            csr::MSCRATCH => self.csr.mscratch = val,
            csr::MEPC => self.csr.mepc = val,
            csr::MCAUSE => self.csr.mcause = val,
            csr::MTVAL => self.csr.mtval = val,
            csr::MIP => self.csr.mip = val,
            csr::MEDELEG => self.csr.medeleg = val,
            csr::MIDELEG => self.csr.mideleg = val,
            csr::SSTATUS => {
                self.csr.mstatus =
                    (self.csr.mstatus & !csr::SSTATUS_MASK) | (val & csr::SSTATUS_MASK)
            }
            csr::SIE => self.csr.mie = (self.csr.mie & !csr::SIE_S_MASK) | (val & csr::SIE_S_MASK),
            csr::SIP => self.csr.mip = (self.csr.mip & !csr::SIE_S_MASK) | (val & csr::SIE_S_MASK),
            csr::STVEC => self.csr.stvec = val,
            csr::SSCRATCH => self.csr.sscratch = val,
            csr::SEPC => self.csr.sepc = val,
            csr::SCAUSE => self.csr.scause = val,
            csr::STVAL => self.csr.stval = val,
            csr::SATP => self.csr.satp = val,
            a if (csr::PMPCFG0..csr::PMPCFG0 + 4).contains(&a) => {
                self.csr.pmpcfg[(a - csr::PMPCFG0) as usize] = val;
            }
            a if (csr::PMPADDR0..csr::PMPADDR0 + 16).contains(&a) => {
                self.csr.pmpaddr[(a - csr::PMPADDR0) as usize] = val;
            }
            _ => {}
        }
    }

    /// 当前特权级对应的 CSR 地址 bit[9:8] 数值（U=0, S=1, M=3）。
    fn priv_csr_level(&self) -> u32 {
        match self.priv_ {
            Priv::User => 0,
            Priv::Supervisor => 1,
            Priv::Machine => 3,
        }
    }

    /// 当前实现存在的 CSR。未列出的 CSR 读/写必须触发 illegal instruction，
    /// 不能返回 0：OpenSBI 等固件会通过“读取是否成功”探测可选扩展。
    fn csr_implemented(addr: u16) -> bool {
        matches!(
            addr,
            csr::MSTATUS
                | csr::MSTATUSH
                | csr::MISA
                | csr::MIE
                | csr::MTVEC
                | csr::MSCRATCH
                | csr::MEPC
                | csr::MCAUSE
                | csr::MTVAL
                | csr::MIP
                | csr::MEDELEG
                | csr::MIDELEG
                | csr::SSTATUS
                | csr::SSTATUSH
                | csr::SIE
                | csr::SIP
                | csr::STVEC
                | csr::SSCRATCH
                | csr::SEPC
                | csr::SCAUSE
                | csr::STVAL
                | csr::SATP
                | csr::MCYCLE
                | csr::MINSTRET
                | csr::CYCLE
                | csr::TIME
                | csr::INSTRET
                | csr::TIMEH
                | csr::MVENDORID
                | csr::MARCHID
                | csr::MIMPID
                | csr::MHARTID
        ) || (csr::PMPCFG0..csr::PMPCFG0 + 4).contains(&addr)
            || (csr::PMPADDR0..csr::PMPADDR0 + 16).contains(&addr)
    }

    /// CSR 访问权限检查：CSR 不存在、特权级不足或写入只读 CSR 时触发
    /// illegal instruction。`write` 由具体指令决定（CSRRS/CSRRC 的 rs1=x0
    /// 不构成写）。
    pub(crate) fn check_csr_access(&self, addr: u16, write: bool, raw: u32) -> Result<(), TrapErr> {
        if !Self::csr_implemented(addr) {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: raw });
        }
        let required = u32::from((addr >> 8) & 0x3);
        let read_only = (addr >> 10) & 0x3 == 0x3;
        if required > self.priv_csr_level() || (write && read_only) {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: raw });
        }
        Ok(())
    }

    /// 更新 mstatus 三域：先清 MPP/MPIE/MIE，再写 MPP、MIE、MPIE。
    pub(crate) fn update_mstatus(&mut self, mpp: u32, mie: u32, mpie: u32) {
        let mut mstatus =
            self.csr.mstatus & !(csr::MSTATUS_MPP_MASK | csr::MSTATUS_MPIE | csr::MSTATUS_MIE);
        mstatus |= (mpp << csr::MSTATUS_MPP_SHIFT)
            | (mie << csr::MSTATUS_MIE.trailing_zeros())
            | (mpie << csr::MSTATUS_MPIE.trailing_zeros());
        self.csr.mstatus = mstatus;
    }

}
