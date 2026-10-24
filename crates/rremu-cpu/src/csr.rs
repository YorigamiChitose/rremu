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
    /// `fcsr`：低 5 位 fflags，bits 5..7 为 frm。
    pub fcsr: u32,
    pub mcounteren: u32,
    pub scounteren: u32,
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
            fcsr: 0,
            mcounteren: 0,
            scounteren: 0,
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
    /// 调试/快照读取：未实现 CSR 返回 None，不触发架构 trap。
    /// `sstatus`/`sie`/`sip` 是 `mstatus`/`mie`/`mip` 的 S 位掩码视图。
    pub fn debug_csr_read(&self, addr: u16) -> Option<u32> {
        Some(match addr {
            csr::MSTATUS => self.mstatus_value(),
            csr::MSTATUSH | csr::SSTATUSH => 0,
            csr::MISA => self.csr.misa,
            csr::MIE => self.csr.mie,
            csr::MTVEC => self.csr.mtvec,
            csr::MSCRATCH => self.csr.mscratch,
            csr::MEPC => self.csr.mepc,
            csr::MCAUSE => self.csr.mcause,
            csr::MTVAL => self.csr.mtval,
            csr::MIP => self.csr.mip,
            csr::FFLAGS => self.csr.fcsr & csr::FCSR_FFLAGS_MASK,
            csr::FRM => (self.csr.fcsr & csr::FCSR_FRM_MASK) >> csr::FCSR_FRM_SHIFT,
            csr::FCSR => self.csr.fcsr,
            csr::MCOUNTEREN => self.csr.mcounteren,
            csr::SCOUNTEREN => self.csr.scounteren,
            csr::MEDELEG => self.csr.medeleg,
            csr::MIDELEG => self.csr.mideleg,
            csr::SSTATUS => self.sstatus_value(),
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
            _ => return None,
        })
    }

    /// 架构读取：未实现 CSR 必须 illegal。仅用于 CSR 指令路径。
    pub(crate) fn architectural_csr_read(&self, addr: u16) -> Result<u32, TrapErr> {
        if !Self::csr_implemented(addr) {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: 0 });
        }
        Ok(self.debug_csr_read(addr).unwrap_or(0))
    }

    /// FS != Off 时浮点状态可用。
    pub(crate) fn fp_enabled(&self) -> bool {
        self.csr.mstatus & csr::MSTATUS_FS_MASK != csr::MSTATUS_FS_OFF
    }

    /// 写浮点状态后置 FS=Dirty。调用方必须先检查 `fp_enabled()`。
    pub(crate) fn mark_fp_dirty(&mut self) {
        self.csr.mstatus = (self.csr.mstatus & !csr::MSTATUS_FS_MASK) | csr::MSTATUS_FS_DIRTY;
    }

    /// 返回带只读 SD 汇总位的 mstatus。
    fn mstatus_value(&self) -> u32 {
        let mut value = self.csr.mstatus & !csr::MSTATUS_SD;
        if self.csr.mstatus & csr::MSTATUS_FS_MASK == csr::MSTATUS_FS_DIRTY {
            value |= csr::MSTATUS_SD;
        }
        value
    }

    /// 返回 sstatus 视图：仅 S 相关位、FS 与只读 SD。
    fn sstatus_value(&self) -> u32 {
        self.mstatus_value() & (csr::SSTATUS_MASK | csr::MSTATUS_SD)
    }

    fn is_fp_csr(addr: u16) -> bool {
        matches!(addr, csr::FFLAGS | csr::FRM | csr::FCSR)
    }

    fn pmp_entry_locked(&self, i: usize) -> bool {
        let cfg = (self.csr.pmpcfg[i / 4] >> ((i % 4) * 8)) & 0xff;
        cfg & 0x80 != 0
    }

    fn pmp_entry_mode(&self, i: usize) -> u32 {
        let cfg = (self.csr.pmpcfg[i / 4] >> ((i % 4) * 8)) & 0xff;
        (cfg >> 3) & 0x3
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
            csr::FFLAGS => {
                self.csr.fcsr =
                    (self.csr.fcsr & !csr::FCSR_FFLAGS_MASK) | (val & csr::FCSR_FFLAGS_MASK);
                self.mark_fp_dirty();
            }
            csr::FRM => {
                self.csr.fcsr = (self.csr.fcsr & !csr::FCSR_FRM_MASK)
                    | ((val << csr::FCSR_FRM_SHIFT) & csr::FCSR_FRM_MASK);
                self.mark_fp_dirty();
            }
            csr::FCSR => {
                self.csr.fcsr = val & csr::FCSR_MASK;
                self.mark_fp_dirty();
            }
            csr::MCYCLE => self.cycle = (self.cycle & !0xffff_ffff) | u64::from(val),
            csr::MINSTRET => self.instret = (self.instret & !0xffff_ffff) | u64::from(val),
            csr::MCOUNTEREN => self.csr.mcounteren = val,
            csr::SCOUNTEREN => self.csr.scounteren = val,
            csr::MEDELEG => self.csr.medeleg = val,
            csr::MIDELEG => self.csr.mideleg = val,
            csr::SSTATUS => {
                self.csr.mstatus =
                    (self.csr.mstatus & !csr::SSTATUS_MASK) | (val & csr::SSTATUS_MASK)
            }
            csr::SIE => self.csr.mie = (self.csr.mie & !csr::SIE_S_MASK) | (val & csr::SIE_S_MASK),
            csr::SIP => self.csr.mip = (self.csr.mip & !csr::MIP_SSIP) | (val & csr::MIP_SSIP),
            csr::STVEC => self.csr.stvec = val,
            csr::SSCRATCH => self.csr.sscratch = val,
            csr::SEPC => self.csr.sepc = val,
            csr::SCAUSE => self.csr.scause = val,
            csr::STVAL => self.csr.stval = val,
            csr::SATP => self.csr.satp = val,
            a if (csr::PMPCFG0..csr::PMPCFG0 + 4).contains(&a) => {
                let idx = (a - csr::PMPCFG0) as usize;
                let old = self.csr.pmpcfg[idx];
                let mut new = val;
                // 锁定 entry 的整字节配置保持不变。
                for byte in 0..4 {
                    let shift = byte * 8;
                    if (old >> shift) & 0x80 != 0 {
                        let mask = 0xffu32 << shift;
                        new = (new & !mask) | (old & mask);
                    }
                }
                self.csr.pmpcfg[idx] = new;
            }
            a if (csr::PMPADDR0..csr::PMPADDR0 + 16).contains(&a) => {
                let i = (a - csr::PMPADDR0) as usize;
                if self.pmp_entry_locked(i) {
                    return;
                }
                // entry i+1 为锁定 TOR 时，pmpaddr[i] 是其下界，忽略写入。
                if i + 1 < 16
                    && self.pmp_entry_locked(i + 1)
                    && self.pmp_entry_mode(i + 1) == 1
                {
                    return;
                }
                self.csr.pmpaddr[i] = val;
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
                | csr::FFLAGS
                | csr::FRM
                | csr::FCSR
                | csr::MCOUNTEREN
                | csr::MEDELEG
                | csr::MIDELEG
                | csr::SSTATUS
                | csr::SSTATUSH
                | csr::SIE
                | csr::SIP
                | csr::SCOUNTEREN
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

    /// 计数器对应的 mcounteren/scounteren 位。
    fn counter_enable_bit(addr: u16) -> Option<u32> {
        match addr {
            csr::CYCLE => Some(csr::COUNTEREN_CY),
            csr::TIME | csr::TIMEH => Some(csr::COUNTEREN_TM),
            csr::INSTRET => Some(csr::COUNTEREN_IR),
            _ => None,
        }
    }

    /// S/U 读取计数器时检查 mcounteren/scounteren；M-mode 不受限制。
    fn counter_access_allowed(&self, addr: u16) -> bool {
        let Some(bit) = Self::counter_enable_bit(addr) else { return true };
        match self.priv_ {
            Priv::Machine => true,
            Priv::Supervisor => self.csr.mcounteren & bit != 0,
            Priv::User => {
                self.csr.mcounteren & bit != 0 && self.csr.scounteren & bit != 0
            }
        }
    }

    /// CSR 访问权限检查：CSR 不存在、特权级不足或写入只读 CSR 时触发
    /// illegal instruction。`write` 由具体指令决定（CSRRS/CSRRC 的 rs1=x0
    /// 不构成写）。
    pub(crate) fn check_csr_access(&self, addr: u16, write: bool, raw: u32) -> Result<(), TrapErr> {
        if !Self::csr_implemented(addr) {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: raw });
        }
        if Self::is_fp_csr(addr) && !self.fp_enabled() {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: raw });
        }
        let required = u32::from((addr >> 8) & 0x3);
        let read_only = (addr >> 10) & 0x3 == 0x3;
        if required > self.priv_csr_level() || (write && read_only) {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: raw });
        }
        if !write && !self.counter_access_allowed(addr) {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: raw });
        }
        Ok(())
    }

    /// 更新 mstatus 三域：先清 MPP/MPIE/MIE，再写 MPP、MIE、MPIE。
    pub(crate) fn update_mstatus(&mut self, mpp: u32, mie: u32, mpie: u32) {
        let mut mstatus = self.csr.mstatus
            & !(csr::MSTATUS_SD | csr::MSTATUS_MPP_MASK | csr::MSTATUS_MPIE | csr::MSTATUS_MIE);
        mstatus |= (mpp << csr::MSTATUS_MPP_SHIFT)
            | (mie << csr::MSTATUS_MIE.trailing_zeros())
            | (mpie << csr::MSTATUS_MPIE.trailing_zeros());
        self.csr.mstatus = mstatus;
    }
}
